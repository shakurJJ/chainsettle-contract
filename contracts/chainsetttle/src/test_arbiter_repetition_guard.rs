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
    assert!(panel2.contains(&pool[3]) && panel2.contains(&pool[4]) && panel2.contains(&pool[5]));
}

#[test]
fn test_panel_falls_back_to_least_recent_when_pool_too_small() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_history_size(&t.buyer, &3u32);
    let pool = add_pool(&t, &client, 4);

    let s1 = open_dispute(&t, &client, "f1", &t.supplier, &t.arbiter);
    client.assign_dispute_panel(&t.buyer, &s1, &0u32, &3u32); // pool[0..3]
    let s2 = open_dispute(&t, &client, "f2", &t.supplier, &t.arbiter);
    let panel2 = client.assign_dispute_panel(&t.buyer, &s2, &0u32, &3u32);

    // Only pool[3] is fresh; the other two seats reuse the oldest recent arbiters.
    assert_eq!(panel2.len(), 3);
    assert!(panel2.contains(&pool[3]));
    assert!(panel2.contains(&pool[0]));
    assert!(panel2.contains(&pool[1]));
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
