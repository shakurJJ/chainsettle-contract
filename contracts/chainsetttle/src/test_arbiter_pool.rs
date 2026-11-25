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
// New tests added
// ============================================================

#[test]
fn test_multiple_arbiters_round_robin_distribution() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, t.contract_id);

    // Add three arbiters to pool
    let arb1 = t.arbiter.clone();
    let arb2 = t.buyer2.clone();
    let arb3 = t.logistics.clone();
    
    client.add_arbiter_to_pool(&t.buyer, &arb1);
    client.add_arbiter_to_pool(&t.buyer, &arb2);
    client.add_arbiter_to_pool(&t.buyer, &arb3);

    // Create three shipments and verify round-robin assignment
    for i in 0..3u32 {
        let ship_id = String::from_str(&t.env, &std::format!("ship_{}", i));
        client.create_shipment(
            &ship_id,
            &single_buyer_vec(&t.env, &t.buyer),
            &t.supplier,
            &t.supplier, // using supplier as logistics to free up logistics as arbiter
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
        
        let shipment = client.get_shipment(&ship_id);
        match i {
            0 => assert_eq!(shipment.arbiter, arb1),
            1 => assert_eq!(shipment.arbiter, arb2),
            2 => assert_eq!(shipment.arbiter, arb3),
            _ => panic!("unexpected index"),
        }
    }
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_non_pool_arbiter_cannot_resolve_pool_dispute() {
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

    // Try to resolve with a different arbiter (not assigned from pool)
    // Should panic with Unauthorized
    client.resolve_dispute(&t.buyer2, &ship_id, &0u32, &true, &None);
}

#[test]
fn test_remove_arbiter_does_not_affect_existing_assignments() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add two arbiters
    client.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    client.add_arbiter_to_pool(&t.buyer, &t.buyer2);

    // Create shipment and raise dispute (gets t.arbiter)
    let ship_id = sid(&t.env, "pre_removal");
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
    
    let assigned_arbiter = client.get_shipment(&ship_id).arbiter.clone();
    assert_eq!(assigned_arbiter, t.arbiter);

    // Remove the assigned arbiter from pool
    client.remove_arbiter_from_pool(&t.buyer, &t.arbiter);

    // The previously assigned arbiter should still be able to resolve
    client.resolve_dispute(&assigned_arbiter, &ship_id, &0u32, &true, &None);
    
    let milestone = client.get_milestone(&ship_id, &0u32);
    assert_eq!(milestone.status, MilestoneStatus::Resolved);
}

#[test]
fn test_pool_index_advances_only_on_dispute_raised() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add two arbiters
    let arb1 = t.arbiter.clone();
    let arb2 = t.buyer2.clone();
    client.add_arbiter_to_pool(&t.buyer, &arb1);
    client.add_arbiter_to_pool(&t.buyer, &arb2);

    // Create multiple shipments but only raise disputes on some
    let ship1 = sid(&t.env, "ship_dispute_1");
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
    assert_eq!(client.get_shipment(&ship1).arbiter, arb1); // First arbiter

    // Create another shipment without dispute
    let ship2 = sid(&t.env, "ship_no_dispute");
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
    // No dispute raised here

    // Create third shipment and raise dispute - should get second arbiter
    let ship3 = sid(&t.env, "ship_dispute_2");
    client.create_shipment(
        &ship3,
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
        &ship3,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.raise_dispute(&t.buyer, &ship3, &0u32);
    assert_eq!(client.get_shipment(&ship3).arbiter, arb2); // Second arbiter
}

#[test]
fn test_add_multiple_arbiters_then_remove_one_and_verify_pool_state() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Add three arbiters
    let arb1 = t.arbiter.clone();
    let arb2 = t.buyer2.clone();
    let arb3 = t.logistics.clone();
    
    client.add_arbiter_to_pool(&t.buyer, &arb1);
    client.add_arbiter_to_pool(&t.buyer, &arb2);
    client.add_arbiter_to_pool(&t.buyer, &arb3);
    
    assert_eq!(client.get_arbiter_pool().len(), 3);

    // Remove middle arbiter
    client.remove_arbiter_from_pool(&t.buyer, &arb2);
    
    let pool = client.get_arbiter_pool();
    assert_eq!(pool.len(), 2);
    
    // Verify remaining arbiters
    assert!(pool.contains(&arb1));
    assert!(!pool.contains(&arb2));
    assert!(pool.contains(&arb3));

    // Create shipment and verify assignment works with updated pool
    let ship_id = sid(&t.env, "post_removal_test");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.supplier, // using supplier as logistics
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
    
    // Should assign one of the remaining arbiters
    let assigned = client.get_shipment(&ship_id).arbiter;
    assert!(assigned == arb1 || assigned == arb3);
    assert!(assigned != arb2);
}
