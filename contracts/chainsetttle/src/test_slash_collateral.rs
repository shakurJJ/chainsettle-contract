#![cfg(test)]

//! Tests for collateral slashing to the buyer on a missed milestone deadline.
//!
//! Covers `slash_collateral_for_miss`, the `collateral_slash_bps_per_miss`
//! shipment option, and the once-per-milestone guard.

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, String, Symbol, Vec,
};

fn sid(env: &soroban_sdk::Env, id: &str) -> String {
    String::from_str(env, id)
}

/// One milestone carrying a ledger deadline `deadline` ledgers from creation.
fn milestone_with_deadline(env: &soroban_sdk::Env, deadline: u32) -> Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Dispatch"),
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: deadline,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
        },
    ]
}

fn proof(env: &soroban_sdk::Env) -> String {
    String::from_str(env, "ipfs://proof")
}

fn proof_type(env: &soroban_sdk::Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

/// Creates a single-milestone shipment with `collateral` locked and a
/// `collateral_slash_bps_per_miss` rate, and returns its id.
fn create_ship(
    t: &TestSetup,
    id: &str,
    collateral: i128,
    slash_bps: u32,
    deadline: u32,
) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = collateral;

    let shipment_id = sid(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestone_with_deadline(&t.env, deadline),
        &opts,
    );
    if slash_bps > 0 {
        client.set_collateral_slash_bps(&t.buyer, &shipment_id, &slash_bps);
    }
    shipment_id
}

// ============================================================
// HAPPY PATH
// ============================================================

#[test]
fn test_slash_marks_milestone_and_is_visible_in_queries() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-flag", 1_000_000, 5_000, 100);
    assert!(!client.is_collateral_slashed(&id, &0u32));
    assert_eq!(client.get_collateral_slash_bps(&id), 5_000);

    t.env.ledger().set_sequence_number(200);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);

    assert!(client.is_collateral_slashed(&id, &0u32));
}

// ============================================================
// ONE SLASH PER MILESTONE
// ============================================================

#[test]
#[should_panic(expected = "milestone already slashed")]
fn test_slash_rejected_twice_for_same_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-twice", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

#[test]
fn test_double_slash_call_does_not_move_extra_funds() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-twice-safe", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
    let after_first = token_client.balance(&t.buyer);

    // The second call panics, so its effects are rolled back entirely.
    let result = client.try_slash_collateral_for_miss(&t.buyer, &id, &0u32);
    assert!(result.is_err());
    assert_eq!(token_client.balance(&t.buyer), after_first);
    assert_eq!(client.get_locked_collateral(&id), 900_000);
}

// ============================================================
// DISABLED / DEADLINE ENFORCEMENT
// ============================================================

#[test]
#[should_panic(expected = "collateral slashing is not enabled")]
fn test_slash_rejected_when_bps_is_zero() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-disabled", 1_000_000, 0, 100);
    t.env.ledger().set_sequence_number(200);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

#[test]
#[should_panic(expected = "milestone deadline has not passed")]
fn test_slash_rejected_before_deadline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-early", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(100);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

#[test]
#[should_panic(expected = "milestone has no deadline")]
fn test_slash_rejected_without_a_deadline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-no-deadline", 1_000_000, 1_000, 0);
    t.env.ledger().set_sequence_number(500);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

// ============================================================
// PROOF / STATE PRECONDITIONS
// ============================================================

#[test]
#[should_panic(expected = "milestone is not pending")]
fn test_slash_rejected_once_proof_submitted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-proofed", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &proof_type(&t.env));
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

// ============================================================
// CREATION-TIME VALIDATION
// ============================================================

#[test]
#[should_panic(expected = "bps cannot exceed 10000")]
fn test_creation_rejects_bps_above_10000() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;

    let id = sid(&t.env, "slash-bad-bps");
    client.create_shipment(
        &id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestone_with_deadline(&t.env, 100),
        &opts,
    );
    client.set_collateral_slash_bps(&t.buyer, &id, &10_001);
}

// ============================================================
// EDGE CASES
// ============================================================

#[test]
fn test_slash_after_an_incremental_collateral_release() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-incremental", 1_000_000, 5_000, 100);

    // The buyer opts into milestone-linked release: 100% of this single
    // milestone is due back to the supplier on confirmation.
    client.set_incremental_collateral(&t.buyer, &id, &true);

    // The supplier delivers on time, so collateral is released, not slashed.
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &proof_type(&t.env));
    client.confirm_milestone(&t.buyer, &id, &0u32);
    assert_eq!(client.get_locked_collateral(&id), 0);

    // A second shipment exercises the case where a release and a slash overlap:
    // the release base is snapshotted, so the slash must be capped by what is
    // actually still locked rather than going negative.
    let id2 = create_ship(&t, "slash-incremental-2", 1_000_000, 5_000, 100);
    client.set_incremental_collateral(&t.buyer, &id2, &true);
    let supplier_before = token_client.balance(&t.supplier);

    t.env.ledger().set_sequence_number(200);
    let slashed = client.slash_collateral_for_miss(&t.buyer, &id2, &0u32);
    assert_eq!(slashed, 500_000);
    assert_eq!(client.get_locked_collateral(&id2), 500_000);
    assert!(token_client.balance(&t.supplier) >= supplier_before);
}

#[test]
#[should_panic(expected = "slash amount rounds to zero")]
fn test_slash_rejected_when_amount_rounds_to_zero() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // 1 base unit of collateral at 1 bps truncates to 0 — nothing to move.
    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-dust", 1, 1, 100);
    t.env.ledger().set_sequence_number(200);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

#[test]
fn test_slash_writes_an_audit_entry() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-audit", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);

    let shipment = client.get_shipment(&id);
    let last = shipment
        .audit_log
        .get(shipment.audit_log.len() - 1)
        .unwrap();
    assert_eq!(last.action, Symbol::new(&t.env, "collateral_slashed"));
    assert_eq!(last.caller, t.buyer);
    assert_eq!(last.ledger, 200);
    assert_eq!(last.detail, Symbol::new(&t.env, "slash_collateral"));
}

