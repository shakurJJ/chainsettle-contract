#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{Address, Env, String, Symbol, TryFromVal};

fn sid(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

fn ipfs(env: &Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

fn proof_hash(env: &Env) -> String {
    String::from_str(env, "QmProof")
}

fn create_standard(
    t: &TestSetup,
    client: &ChainSettleContractClient,
    shipment_id: &String,
    arbiter: &Address,
) {
    client.create_shipment(
        shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
}

/// Runs a full dispute -> resolve -> appeal -> (overturning) resolve cycle on
/// a fresh shipment, so the original `arbiter`'s first resolution is
/// overturned by the second (appeal) arbiter. The final `resolve_dispute`
/// call (the one that may trigger a slash) is the last contract invocation,
/// so its events are what `env.events().all()` reflects afterward.
fn open_resolve_appeal_overturn(
    t: &TestSetup,
    client: &ChainSettleContractClient,
    ship_id: &String,
    original_arbiter: &Address,
) {
    client.submit_proof(
        &t.supplier,
        ship_id,
        &0u32,
        &proof_hash(&t.env),
        &ipfs(&t.env),
    );
    client.raise_dispute(&t.buyer, ship_id, &0u32);
    // Original arbiter approves (releases payment to supplier)...
    client.resolve_dispute(original_arbiter, ship_id, &0u32, &true, &None);
    // ...then the appeal arbiter rejects instead: an overturn.
    client.appeal_dispute(&t.buyer, ship_id, &0u32);
    let shipment = client.get_shipment(ship_id);
    let appeal_arbiter = shipment.arbiter;
    client.resolve_dispute(&appeal_arbiter, ship_id, &0u32, &false, &None);
}

#[test]
fn test_overturn_increments_count_but_does_not_slash_below_threshold() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &3u32);

    let ship_id = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);

    let stats = client.get_arbiter_stats(&t.arbiter);
    assert_eq!(stats.overturned_count, 1);
    assert!(!client.is_arbiter_slashed(&t.arbiter));

    let pool = client.get_arbiter_pool();
    assert!(pool.contains(&t.arbiter));
}

#[test]
fn test_arbiter_crossing_threshold_is_slashed_and_removed_from_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &2u32);

    let ship_id_1 = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id_1, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_1, &t.arbiter);
    assert!(!client.is_arbiter_slashed(&t.arbiter));

    // Re-add arbiter2 as the current shipment.arbiter for the second cycle so the
    // pool still contains t.arbiter to draw from during appeal reassignment.
    let ship_id_2 = sid(&t.env, "s2");
    create_standard(&t, &client, &ship_id_2, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_2, &t.arbiter);

    // Threshold of 2 reached — arbiter should now be slashed.
    assert!(client.is_arbiter_slashed(&t.arbiter));
    let stats = client.get_arbiter_stats(&t.arbiter);
    assert_eq!(stats.overturned_count, 2);

    let pool = client.get_arbiter_pool();
    assert!(!pool.contains(&t.arbiter));
}

#[test]
fn test_arbiter_slashed_event_fires_with_address_and_count() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &1u32);

    let ship_id = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id, &t.arbiter);
    // The last call inside this helper is the overturning resolve_dispute,
    // which is what triggers the slash — so events() reflects it below.
    open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);

    let events = t.env.events().all();
    let mut found = false;
    for i in 0..events.len() {
        let (_id, topics, data) = events.get(i).unwrap();
        let topic: Symbol = Symbol::try_from_val(&t.env, &topics.get(0).unwrap()).unwrap();
        if topic == Symbol::new(&t.env, "arbiter_slashed") {
            let (slashed_arbiter, count): (Address, u32) =
                <(Address, u32)>::try_from_val(&t.env, &data).unwrap();
            assert_eq!(slashed_arbiter, t.arbiter);
            assert_eq!(count, 1);
            found = true;
        }
    }
    assert!(found, "expected arbiter_slashed event to fire");
    assert!(client.is_arbiter_slashed(&t.arbiter));
}

#[test]
#[should_panic(expected = "NoArbitersAvailable")]
fn test_slashed_arbiter_cannot_be_assigned_to_new_disputes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Only one arbiter in the pool; once slashed, the pool is empty.
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &1u32);

    let ship_id = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id, &t.arbiter);

    // Manually add a second arbiter only to allow the appeal step to find a
    // distinct arbiter, then remove it so the pool is empty for the next check.
    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);
    assert!(client.is_arbiter_slashed(&t.arbiter));

    client.remove_arbiter_from_pool(&t.buyer, &arbiter2);
    assert_eq!(client.get_arbiter_pool().len(), 0);

    // New shipment relying on the pool-sentinel arbiter has nobody to assign.
    let ship_id_2 = sid(&t.env, "s2");
    client.create_shipment(
        &ship_id_2,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.contract_id,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        &t.supplier,
        &ship_id_2,
        &0u32,
        &proof_hash(&t.env),
        &ipfs(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship_id_2, &0u32);
}

#[test]
fn test_reinstated_arbiter_can_be_re_added_to_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &1u32);

    let ship_id = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);
    assert!(client.is_arbiter_slashed(&t.arbiter));

    client.reinstate_arbiter(&t.buyer, &t.arbiter);
    assert!(!client.is_arbiter_slashed(&t.arbiter));

    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    assert!(client.get_arbiter_pool().contains(&t.arbiter));
}

#[test]
#[should_panic(expected = "arbiter is slashed and must be reinstated before re-adding")]
fn test_add_arbiter_to_pool_rejects_slashed_arbiter_without_reinstatement() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &1u32);

    let ship_id = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);
    assert!(client.is_arbiter_slashed(&t.arbiter));

    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
}

#[test]
fn test_multiple_arbiters_slashed_independently() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    let arbiter3 = Address::generate(&t.env);
    
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.add_arbiter_to_pool(&t.buyer, &arbiter3);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &2u32);

    // First arbiter gets overturned twice
    let ship_id_1 = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id_1, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_1, &t.arbiter);
    
    let ship_id_2 = sid(&t.env, "s2");
    create_standard(&t, &client, &ship_id_2, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_2, &t.arbiter);
    
    assert!(client.is_arbiter_slashed(&t.arbiter));
    assert_eq!(client.get_arbiter_stats(&t.arbiter).overturned_count, 2);

    // Second arbiter gets overturned twice
    let ship_id_3 = sid(&t.env, "s3");
    create_standard(&t, &client, &ship_id_3, &arbiter2);
    open_resolve_appeal_overturn(&t, &client, &ship_id_3, &arbiter2);
    
    let ship_id_4 = sid(&t.env, "s4");
    create_standard(&t, &client, &ship_id_4, &arbiter2);
    open_resolve_appeal_overturn(&t, &client, &ship_id_4, &arbiter2);
    
    assert!(client.is_arbiter_slashed(&arbiter2));
    assert_eq!(client.get_arbiter_stats(&arbiter2).overturned_count, 2);

    // Third arbiter should still be in good standing
    assert!(!client.is_arbiter_slashed(&arbiter3));
    assert_eq!(client.get_arbiter_stats(&arbiter3).overturned_count, 0);

    let pool = client.get_arbiter_pool();
    assert!(!pool.contains(&t.arbiter));
    assert!(!pool.contains(&arbiter2));
    assert!(pool.contains(&arbiter3));
}

#[test]
fn test_reinstate_arbiter_resets_overturned_count() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &2u32);

    // Get one overturn
    let ship_id_1 = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id_1, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_1, &t.arbiter);
    
    let stats_before = client.get_arbiter_stats(&t.arbiter);
    assert_eq!(stats_before.overturned_count, 1);
    assert!(!client.is_arbiter_slashed(&t.arbiter));

    // Get second overturn to trigger slash
    let ship_id_2 = sid(&t.env, "s2");
    create_standard(&t, &client, &ship_id_2, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_2, &t.arbiter);
    
    assert!(client.is_arbiter_slashed(&t.arbiter));
    assert_eq!(client.get_arbiter_stats(&t.arbiter).overturned_count, 2);

    // Reinstate and verify count is reset
    client.reinstate_arbiter(&t.buyer, &t.arbiter);
    
    let stats_after = client.get_arbiter_stats(&t.arbiter);
    assert_eq!(stats_after.overturned_count, 0);
    assert!(!client.is_arbiter_slashed(&t.arbiter));
}

#[test]
fn test_changing_threshold_does_not_retroactively_slash() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &5u32);

    // Accumulate 2 overturns under threshold of 5
    let ship_id_1 = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id_1, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_1, &t.arbiter);
    
    let ship_id_2 = sid(&t.env, "s2");
    create_standard(&t, &client, &ship_id_2, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_2, &t.arbiter);
    
    assert_eq!(client.get_arbiter_stats(&t.arbiter).overturned_count, 2);
    assert!(!client.is_arbiter_slashed(&t.arbiter));

    // Lower threshold to 1 - should not retroactively slash
    client.set_max_overturned_before_slash(&t.buyer, &1u32);
    assert!(!client.is_arbiter_slashed(&t.arbiter));
    
    // But the next overturn should trigger slash at the new threshold
    let ship_id_3 = sid(&t.env, "s3");
    create_standard(&t, &client, &ship_id_3, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id_3, &t.arbiter);
    
    // With 3 overturns and threshold of 1, should now be slashed
    assert!(client.is_arbiter_slashed(&t.arbiter));
    assert_eq!(client.get_arbiter_stats(&t.arbiter).overturned_count, 3);
}

#[test]
fn test_arbiter_stats_persist_across_multiple_disputes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &10u32);

    // Track incrementing overturns
    for i in 1..=5 {
        let ship_id = sid(&t.env, &std::format!("ship_{}", i));
        create_standard(&t, &client, &ship_id, &t.arbiter);
        open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);
        
        let stats = client.get_arbiter_stats(&t.arbiter);
        assert_eq!(stats.overturned_count, i);
        assert!(!client.is_arbiter_slashed(&t.arbiter), "Should not be slashed at count {}", i);
    }

    // Verify final count
    let final_stats = client.get_arbiter_stats(&t.arbiter);
    assert_eq!(final_stats.overturned_count, 5);
    assert!(!client.is_arbiter_slashed(&t.arbiter));
}

#[test]
#[should_panic(expected = "arbiter already slashed")]
fn test_cannot_reinstate_arbiter_twice_consecutively() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let arbiter2 = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &arbiter2);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    client.set_max_overturned_before_slash(&t.buyer, &1u32);

    let ship_id = sid(&t.env, "s1");
    create_standard(&t, &client, &ship_id, &t.arbiter);
    open_resolve_appeal_overturn(&t, &client, &ship_id, &t.arbiter);
    
    assert!(client.is_arbiter_slashed(&t.arbiter));

    // First reinstatement should succeed
    client.reinstate_arbiter(&t.buyer, &t.arbiter);
    assert!(!client.is_arbiter_slashed(&t.arbiter));

    // Second reinstatement should panic - arbiter is not slashed
    client.reinstate_arbiter(&t.buyer, &t.arbiter);
}
