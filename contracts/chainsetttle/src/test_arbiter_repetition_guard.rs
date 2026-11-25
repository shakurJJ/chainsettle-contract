#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String, Symbol};

fn sid(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

/// Creates a shipment for `supplier` with `arbiter`, submits proof on
/// milestone 0 and opens a dispute on it.
fn open_dispute(
    t: &TestSetup,
    client: &ChainSettleContractClient,
    id: &str,
    supplier: &Address,
    arbiter: &Address,
) -> String {
    let shipment_id = sid(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        supplier,
        &t.logistics,
        arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        supplier,
        &shipment_id,
        &0u32,
        &String::from_str(&t.env, "QmProof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0u32);
    shipment_id
}

fn add_pool(t: &TestSetup, client: &ChainSettleContractClient, n: u32) -> std::vec::Vec<Address> {
    let mut out = std::vec::Vec::new();
    for _ in 0..n {
        let a = Address::generate(&t.env);
        client.add_arbiter_to_pool(&t.buyer, &a);
        out.push(a);
    }
    out
}

// ============================================================
// Config + history tracking
// ============================================================

#[test]
fn test_history_size_defaults_to_disabled_and_is_settable() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(client.get_arbiter_history_size(), 0);
    client.set_arbiter_history_size(&t.buyer, &3u32);
    assert_eq!(client.get_arbiter_history_size(), 3);
}

#[test]
fn test_no_history_recorded_while_guard_disabled() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    add_pool(&t, &client, 2);
    open_dispute(&t, &client, "d1", &t.supplier, &t.contract_id);
    assert_eq!(client.get_supplier_recent_arbiters(&t.supplier).len(), 0);
}

#[test]
fn test_history_tracked_per_supplier_and_trimmed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &2u32);
    let pool = add_pool(&t, &client, 3);
    let other_supplier = Address::generate(&t.env);

    open_dispute(&t, &client, "d1", &t.supplier, &t.contract_id);
    open_dispute(&t, &client, "d2", &t.supplier, &t.contract_id);
    open_dispute(&t, &client, "d3", &t.supplier, &t.contract_id);
    open_dispute(&t, &client, "d4", &other_supplier, &t.contract_id);

    // Only the last 2 of the supplier's 3 assignments are retained, oldest first.
    let recent = client.get_supplier_recent_arbiters(&t.supplier);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent.get(0).unwrap(), pool[1]);
    assert_eq!(recent.get(1).unwrap(), pool[2]);
    // The other supplier's history is independent.
    assert_eq!(
        client.get_supplier_recent_arbiters(&other_supplier).len(),
        1
    );
}

// ============================================================
// Pool-arbiter assignment on raise_dispute
// ============================================================

#[test]
fn test_raise_dispute_skips_recent_arbiter_when_pool_allows() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &1u32);
    let pool = add_pool(&t, &client, 2);
    let other_supplier = Address::generate(&t.env);

    let s1 = open_dispute(&t, &client, "s1", &t.supplier, &t.contract_id);
    assert_eq!(client.get_shipment(&s1).arbiter, pool[0]);
    // Another supplier's dispute advances round-robin back to index 0.
    let s2 = open_dispute(&t, &client, "s2", &other_supplier, &t.contract_id);
    assert_eq!(client.get_shipment(&s2).arbiter, pool[1]);

    // Round-robin would hand pool[0] to the supplier again; the guard skips it.
    let s3 = open_dispute(&t, &client, "s3", &t.supplier, &t.contract_id);
    assert_eq!(client.get_shipment(&s3).arbiter, pool[1]);
}

#[test]
fn test_raise_dispute_repeats_without_guard() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pool = add_pool(&t, &client, 2);
    let other_supplier = Address::generate(&t.env);

    open_dispute(&t, &client, "s1", &t.supplier, &t.contract_id);
    open_dispute(&t, &client, "s2", &other_supplier, &t.contract_id);
    let s3 = open_dispute(&t, &client, "s3", &t.supplier, &t.contract_id);
    assert_eq!(client.get_shipment(&s3).arbiter, pool[0]);
}

#[test]
fn test_raise_dispute_falls_back_when_pool_too_small() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &2u32);
    let pool = add_pool(&t, &client, 1);

    let s1 = open_dispute(&t, &client, "s1", &t.supplier, &t.contract_id);
    let s2 = open_dispute(&t, &client, "s2", &t.supplier, &t.contract_id);
    assert_eq!(client.get_shipment(&s1).arbiter, pool[0]);
    assert_eq!(client.get_shipment(&s2).arbiter, pool[0]);
}

// ============================================================
// Appeal arbiter draw
// ============================================================

#[test]
fn test_appeal_skips_recent_arbiter() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &1u32);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    let pool = add_pool(&t, &client, 2);

    let s1 = open_dispute(&t, &client, "a1", &t.supplier, &t.arbiter);
    client.resolve_dispute(&t.arbiter, &s1, &0u32, &true, &None);
    client.appeal_dispute(&t.buyer, &s1, &0u32);
    assert_eq!(client.get_shipment(&s1).arbiter, pool[0]);

    // Without the guard the appeal would draw pool[0] again.
    let s2 = open_dispute(&t, &client, "a2", &t.supplier, &t.arbiter);
    client.resolve_dispute(&t.arbiter, &s2, &0u32, &true, &None);
    client.appeal_dispute(&t.buyer, &s2, &0u32);
    assert_eq!(client.get_shipment(&s2).arbiter, pool[1]);
}

// ============================================================
// Pool-drawn dispute panel
// ============================================================

#[test]
fn test_panel_avoids_recent_arbiters_when_pool_allows() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &3u32);
    let pool = add_pool(&t, &client, 6);

    let s1 = open_dispute(&t, &client, "p1", &t.supplier, &t.arbiter);
    let panel1 = client.assign_dispute_panel(&t.buyer, &s1, &0u32, &3u32);
    let s2 = open_dispute(&t, &client, "p2", &t.supplier, &t.arbiter);
    let panel2 = client.assign_dispute_panel(&t.buyer, &s2, &0u32, &3u32);

    assert_eq!(panel1.len(), 3);
    assert_eq!(panel2.len(), 3);
    for i in 0..panel2.len() {
        assert!(!panel1.contains(panel2.get(i).unwrap()));
    }
    assert_eq!(client.get_arbiter_panel(&s2), panel2);
    // Panels are drawn from a shuffled pool, so panel2 must be exactly the
    // three pool members panel1 did not use.
    for a in pool.iter() {
        assert!(panel1.contains(a) != panel2.contains(a));
    }
}

#[test]
fn test_panel_falls_back_to_least_recent_when_pool_too_small() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &3u32);
    let pool = add_pool(&t, &client, 4);

    let s1 = open_dispute(&t, &client, "f1", &t.supplier, &t.arbiter);
    let panel1 = client.assign_dispute_panel(&t.buyer, &s1, &0u32, &3u32);
    let s2 = open_dispute(&t, &client, "f2", &t.supplier, &t.arbiter);
    let panel2 = client.assign_dispute_panel(&t.buyer, &s2, &0u32, &3u32);

    // Only one pool member is fresh; the other two seats reuse the oldest
    // recent arbiters (panel1 is recorded in order, so its first two).
    let fresh = pool.iter().find(|a| !panel1.contains(*a)).unwrap();
    assert_eq!(panel2.len(), 3);
    assert!(panel2.contains(fresh));
    assert!(panel2.contains(panel1.get(0).unwrap()));
    assert!(panel2.contains(panel1.get(1).unwrap()));
}

#[test]
fn test_assigned_panel_can_resolve_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    add_pool(&t, &client, 3);

    let s1 = open_dispute(&t, &client, "v1", &t.supplier, &t.arbiter);
    let panel = client.assign_dispute_panel(&t.buyer, &s1, &0u32, &3u32);
    client.cast_dispute_vote(&panel.get(0).unwrap(), &s1, &0u32, &true);
    client.cast_dispute_vote(&panel.get(1).unwrap(), &s1, &0u32, &true);

    let status = client.get_shipment(&s1).milestones.get(0).unwrap().status;
    assert!(status != MilestoneStatus::Disputed);
}

#[test]
#[should_panic(expected = "not enough arbiters in pool for panel")]
fn test_panel_requires_enough_pool_members() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    add_pool(&t, &client, 2);
    let s1 = open_dispute(&t, &client, "e1", &t.supplier, &t.arbiter);
    client.assign_dispute_panel(&t.buyer, &s1, &0u32, &3u32);
}

#[test]
#[should_panic(expected = "milestone is not disputed")]
fn test_panel_requires_open_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    add_pool(&t, &client, 3);
    let shipment_id = sid(&t.env, "nd");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.assign_dispute_panel(&t.buyer, &shipment_id, &0u32, &3u32);
}

// ============================================================
// Additional repetition guard tests
// ============================================================

#[test]
fn test_history_size_zero_disables_guard_completely() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &0u32);
    let pool = add_pool(&t, &client, 2);

    // Even with explicit zero, guard should remain disabled
    let s1 = open_dispute(&t, &client, "z1", &t.supplier, &t.contract_id);
    let s2 = open_dispute(&t, &client, "z2", &t.supplier, &t.contract_id);
    
    // Should cycle back to first arbiter without skipping
    assert_eq!(client.get_shipment(&s1).arbiter, pool[0]);
    assert_eq!(client.get_shipment(&s2).arbiter, pool[1]);
    
    let s3 = open_dispute(&t, &client, "z3", &t.supplier, &t.contract_id);
    assert_eq!(client.get_shipment(&s3).arbiter, pool[0]);
}

#[test]
fn test_multiple_suppliers_maintain_independent_histories() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &2u32);
    let pool = add_pool(&t, &client, 4);
    
    let supplier2 = Address::generate(&t.env);
    let supplier3 = Address::generate(&t.env);

    // Each supplier gets different arbiters
    open_dispute(&t, &client, "m1", &t.supplier, &t.contract_id);
    open_dispute(&t, &client, "m2", &supplier2, &t.contract_id);
    open_dispute(&t, &client, "m3", &t.supplier, &t.contract_id);
    open_dispute(&t, &client, "m4", &supplier3, &t.contract_id);

    let hist1 = client.get_supplier_recent_arbiters(&t.supplier);
    let hist2 = client.get_supplier_recent_arbiters(&supplier2);
    let hist3 = client.get_supplier_recent_arbiters(&supplier3);

    // All histories should be independent
    assert_eq!(hist1.len(), 2);
    assert_eq!(hist2.len(), 1);
    assert_eq!(hist3.len(), 1);
    
    // Verify they're tracking different arbiters
    assert_eq!(hist1.get(0).unwrap(), pool[0]);
    assert_eq!(hist1.get(1).unwrap(), pool[2]);
    assert_eq!(hist2.get(0).unwrap(), pool[1]);
    assert_eq!(hist3.get(0).unwrap(), pool[3]);
}

#[test]
fn test_history_size_can_be_reduced_and_truncates_existing() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &5u32);
    let pool = add_pool(&t, &client, 5);

    // Create 5 disputes to fill history
    for i in 0..5 {
        open_dispute(&t, &client, &format!("h{}", i), &t.supplier, &t.contract_id);
    }
    
    assert_eq!(client.get_supplier_recent_arbiters(&t.supplier).len(), 5);
    
    // Reduce history size to 2
    client.set_arbiter_history_size(&t.buyer, &2u32);
    
    // Create one more dispute to trigger trimming
    open_dispute(&t, &client, "h5", &t.supplier, &t.contract_id);
    
    // History should now only keep the most recent 2
    let recent = client.get_supplier_recent_arbiters(&t.supplier);
    assert_eq!(recent.len(), 2);
    // Should have the last two arbiters from the sequence
    assert_eq!(recent.get(0).unwrap(), pool[4]);
    assert_eq!(recent.get(1).unwrap(), pool[0]);
}

#[test]
fn test_panel_with_max_history_cycles_through_all_pool_members() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pool_size = 9u32;
    let panel_size = 3u32;
    
    client.set_arbiter_history_size(&t.buyer, &6u32);
    let pool = add_pool(&t, &client, pool_size);

    // First panel
    let s1 = open_dispute(&t, &client, "c1", &t.supplier, &t.arbiter);
    let panel1 = client.assign_dispute_panel(&t.buyer, &s1, &0u32, &panel_size);
    
    // Second panel should avoid all three from panel1
    let s2 = open_dispute(&t, &client, "c2", &t.supplier, &t.arbiter);
    let panel2 = client.assign_dispute_panel(&t.buyer, &s2, &0u32, &panel_size);
    
    // Third panel should avoid all six recent arbiters
    let s3 = open_dispute(&t, &client, "c3", &t.supplier, &t.arbiter);
    let panel3 = client.assign_dispute_panel(&t.buyer, &s3, &0u32, &panel_size);
    
    // Verify no overlap between consecutive panels
    for a in panel2.iter() {
        assert!(!panel1.contains(a));
    }
    for a in panel3.iter() {
        assert!(!panel1.contains(a));
        assert!(!panel2.contains(a));
    }
    
    // All three panels together should cover all pool members
    let mut all_used = std::collections::HashSet::new();
    for a in panel1.iter() {
        all_used.insert(a.clone());
    }
    for a in panel2.iter() {
        all_used.insert(a.clone());
    }
    for a in panel3.iter() {
        all_used.insert(a.clone());
    }
    assert_eq!(all_used.len(), pool_size as usize);
}

#[test]
fn test_appeal_with_full_history_still_draws_from_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &3u32);
    client.set_appeal_window_ledgers(&t.buyer, &50u32);
    let pool = add_pool(&t, &client, 4);

    // Fill up the supplier's history with 3 arbiters
    for i in 0..3 {
        let s = open_dispute(&t, &client, &format!("af{}", i), &t.supplier, &t.contract_id);
        // Resolve immediately so we can keep opening disputes
        client.resolve_dispute(&client.get_shipment(&s).arbiter, &s, &0u32, &true, &None);
    }
    
    // Now create a dispute with a non-pool arbiter and appeal it
    let s_appeal = open_dispute(&t, &client, "af_appeal", &t.supplier, &t.arbiter);
    client.resolve_dispute(&t.arbiter, &s_appeal, &0u32, &true, &None);
    client.appeal_dispute(&t.buyer, &s_appeal, &0u32);
    
    // The appeal should draw from the pool, skipping the 3 recent arbiters
    let appeal_arbiter = client.get_shipment(&s_appeal).arbiter;
    assert!(pool.contains(&appeal_arbiter));
    
    // Should be the one pool member not in recent history
    let recent = client.get_supplier_recent_arbiters(&t.supplier);
    assert!(!recent.contains(&appeal_arbiter));
}
