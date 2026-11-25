#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{String, Symbol};

fn sid(env: &soroban_sdk::Env, id: &str) -> String {
    String::from_str(env, id)
}

fn proof_hash(env: &soroban_sdk::Env) -> soroban_sdk::String {
    soroban_sdk::String::from_str(env, "QmXyz123")
}

fn proof_type(env: &soroban_sdk::Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

/// Sentinel: passing the contract address itself as arbiter signals "use pool".
fn pool_sentinel(t: &crate::test_common::TestSetup) -> soroban_sdk::Address {
    t.contract_id.clone()
}

// ============================================================
// Arbiter pool management
// ============================================================

#[test]
fn test_add_and_get_arbiter_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    assert_eq!(client.get_arbiter_pool().len(), 0);

    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    let pool = client.get_arbiter_pool();
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.get(0).unwrap(), t.arbiter);
}

#[test]
fn test_add_arbiter_deduplication() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter); // duplicate
    assert_eq!(client.get_arbiter_pool().len(), 1);
}

#[test]
fn test_remove_arbiter_from_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &t.logistics);
    client.remove_arbiter_from_pool(&t.buyer, &t.arbiter);

    let pool = client.get_arbiter_pool();
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.get(0).unwrap(), t.logistics);
}

// ============================================================
// Empty pool panics
// ============================================================

#[test]
#[should_panic(expected = "NoArbitersAvailable")]
fn test_raise_dispute_empty_pool_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Pool is empty — pool sentinel passed as arbiter
    let ship_id = sid(&t.env, "ship1");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    // Should panic: no arbiters in pool
    client.raise_dispute(&t.buyer, &ship_id, &0u32);
}

// ============================================================
// Explicit per-shipment arbiter still works (backward compat)
// ============================================================

#[test]
fn test_explicit_arbiter_override_still_works() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add something to the pool — but use an explicit arbiter, not the sentinel
    client.add_arbiter_to_pool(&t.buyer, &t.logistics);

    let ship_id = sid(&t.env, "ship_explicit");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter, // explicit, not sentinel
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship_id, &0u32);

    // Arbiter stored on shipment should still be the explicit one
    let shipment = client.get_shipment(&ship_id);
    assert_eq!(shipment.arbiter, t.arbiter);

    // The explicit arbiter can resolve
    client.resolve_dispute(&t.arbiter, &ship_id, &0u32, &true, &None);
}

// ============================================================
// Round-robin assignment
// ============================================================

#[test]
fn test_round_robin_assigns_in_order() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Two arbiters in pool
    let arb1 = t.arbiter.clone();
    let arb2 = t.buyer2.clone();
    client.add_arbiter_to_pool(&t.buyer, &arb1);
    client.add_arbiter_to_pool(&t.buyer, &arb2);

    // Shipment 1 → should get arb1 (index 0)
    let ship1 = sid(&t.env, "ship1");
    client.create_shipment(
        &ship1,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        &t.supplier,
        &ship1,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship1, &0u32);
    assert_eq!(client.get_shipment(&ship1).arbiter, arb1);

    // Shipment 2 → should get arb2 (index 1)
    let ship2 = sid(&t.env, "ship2");
    client.create_shipment(
        &ship2,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        &t.supplier,
        &ship2,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship2, &0u32);
    assert_eq!(client.get_shipment(&ship2).arbiter, arb2);
}

// ============================================================
// Pool wraps at end (round-robin wrap)
// ============================================================

#[test]
fn test_round_robin_wraps_at_end() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Only one arbiter in pool → both shipments should get it
    let arb1 = t.arbiter.clone();
    client.add_arbiter_to_pool(&t.buyer, &arb1);

    for i in 0..2u32 {
        let ship_id = String::from_str(&t.env, if i == 0 { "wrap1" } else { "wrap2" });
        client.create_shipment(
            &ship_id,
            &single_buyer_vec(&t.env, &t.buyer),
            &t.supplier,
            &t.logistics,
            &pool_sentinel(&t),
            &t.token_id,
            &1_000_000,
            &build_milestones(&t.env),
            &default_options(&t.env),
        );
        client.submit_proof(
            &t.supplier,
            &ship_id,
            &0u32,
            &proof_hash(&t.env),
            &proof_type(&t.env),
        );
        client.raise_dispute(&t.buyer, &ship_id, &0u32);
        assert_eq!(client.get_shipment(&ship_id).arbiter, arb1);

        // Resolve so the shipment doesn't block a second iteration
        client.resolve_dispute(&arb1, &ship_id, &0u32, &false, &None); // reject → back to Pending
        client.submit_proof(
            &t.supplier,
            &ship_id,
            &0u32,
            &proof_hash(&t.env),
            &proof_type(&t.env),
        );
        client.confirm_milestone(&t.buyer, &ship_id, &0u32);
    }
}

// ============================================================
// Pool-assigned arbiter is the one who can resolve
// ============================================================

#[test]
fn test_pool_assigned_arbiter_resolves_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);

    let ship_id = sid(&t.env, "resolve_test");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship_id, &0u32);

    // Pool-assigned arbiter (t.arbiter) should be able to resolve
    client.resolve_dispute(&t.arbiter, &ship_id, &0u32, &true, &None);

    let milestone = client.get_milestone(&ship_id, &0u32);
    assert_eq!(milestone.status, MilestoneStatus::Resolved);
}

// ============================================================
// New Tests: Extended arbiter pool scenarios
// ============================================================

#[test]
fn test_multiple_arbiters_different_disputes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add three arbiters to pool
    let arb1 = t.arbiter.clone();
    let arb2 = t.buyer2.clone();
    let arb3 = t.logistics.clone();
    
    client.add_arbiter_to_pool(&t.buyer, &arb1);
    client.add_arbiter_to_pool(&t.buyer, &arb2);
    client.add_arbiter_to_pool(&t.buyer, &arb3);

    // Create three shipments and raise disputes
    let ship1 = sid(&t.env, "multi_ship1");
    let ship2 = sid(&t.env, "multi_ship2");
    let ship3 = sid(&t.env, "multi_ship3");

    for (idx, ship_id) in [&ship1, &ship2, &ship3].iter().enumerate() {
        client.create_shipment(
            ship_id,
            &single_buyer_vec(&t.env, &t.buyer),
            &t.supplier,
            &t.logistics,
            &pool_sentinel(&t),
            &t.token_id,
            &1_000_000,
            &build_milestones(&t.env),
            &default_options(&t.env),
        );
        client.submit_proof(
            &t.supplier,
            ship_id,
            &0u32,
            &proof_hash(&t.env),
            &proof_type(&t.env),
        );
        client.raise_dispute(&t.buyer, ship_id, &0u32);

        // Verify each shipment gets a different arbiter in round-robin order
        let shipment = client.get_shipment(ship_id);
        match idx {
            0 => assert_eq!(shipment.arbiter, arb1),
            1 => assert_eq!(shipment.arbiter, arb2),
            2 => assert_eq!(shipment.arbiter, arb3),
            _ => panic!("Unexpected index"),
        }
    }
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_non_pool_arbiter_cannot_resolve_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add only one arbiter to pool
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);

    let ship_id = sid(&t.env, "unauthorized_resolve");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship_id, &0u32);

    // Try to resolve with someone NOT assigned from the pool
    // Should panic: buyer2 is not the assigned arbiter
    client.resolve_dispute(&t.buyer2, &ship_id, &0u32, &true, &None);
}

#[test]
fn test_remove_arbiter_affects_pool_size() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add three arbiters
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &t.buyer2);
    client.add_arbiter_to_pool(&t.buyer, &t.logistics);

    assert_eq!(client.get_arbiter_pool().len(), 3);

    // Remove one
    client.remove_arbiter_from_pool(&t.buyer, &t.buyer2);
    assert_eq!(client.get_arbiter_pool().len(), 2);

    // Verify the remaining arbiters are correct
    let pool = client.get_arbiter_pool();
    assert!(pool.contains(&t.arbiter));
    assert!(pool.contains(&t.logistics));
    assert!(!pool.contains(&t.buyer2));
}

#[test]
fn test_pool_assignment_with_multiple_milestones() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add two arbiters to pool
    let arb1 = t.arbiter.clone();
    let arb2 = t.buyer2.clone();
    client.add_arbiter_to_pool(&t.buyer, &arb1);
    client.add_arbiter_to_pool(&t.buyer, &arb2);

    let ship_id = sid(&t.env, "multi_milestone");
    
    // Create shipment with multiple milestones
    let mut milestones = build_milestones(&t.env);
    // Assuming build_milestones creates at least 2 milestones
    
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &milestones,
        &default_options(&t.env),
    );

    // Submit proof and raise dispute for first milestone
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship_id, &0u32);

    // Verify arbiter was assigned
    let shipment = client.get_shipment(&ship_id);
    assert_eq!(shipment.arbiter, arb1);

    // Resolve the dispute and confirm milestone
    client.resolve_dispute(&arb1, &ship_id, &0u32, &true, &None);

    // The same shipment should keep the same arbiter for consistency
    let shipment_after = client.get_shipment(&ship_id);
    assert_eq!(shipment_after.arbiter, arb1);
}

#[test]
fn test_pool_state_persists_across_operations() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Initial state: empty pool
    assert_eq!(client.get_arbiter_pool().len(), 0);

    // Add arbiters
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &t.buyer2);
    assert_eq!(client.get_arbiter_pool().len(), 2);

    // Create and complete a shipment
    let ship_id = sid(&t.env, "persist_test");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &pool_sentinel(&t),
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    // Pool should still be intact
    assert_eq!(client.get_arbiter_pool().len(), 2);
    let pool = client.get_arbiter_pool();
    assert!(pool.contains(&t.arbiter));
    assert!(pool.contains(&t.buyer2));

    // Add another arbiter
    client.add_arbiter_to_pool(&t.buyer, &t.logistics);
    assert_eq!(client.get_arbiter_pool().len(), 3);

    // Pool modifications should be independent of shipment operations
    let final_pool = client.get_arbiter_pool();
    assert_eq!(final_pool.len(), 3);
    assert!(final_pool.contains(&t.arbiter));
    assert!(final_pool.contains(&t.buyer2));
    assert!(final_pool.contains(&t.logistics));
}
