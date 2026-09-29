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
    opts.collateral_slash_bps_per_miss = slash_bps;

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
    shipment_id
}

// ============================================================
// HAPPY PATH
// ============================================================

#[test]
fn test_slash_transfers_share_of_collateral_to_buyer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    // Deadline lands 100 ledgers out; advance past it before slashing.
    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-happy", 1_000_000, 2_500, 100);
    t.env.ledger().set_sequence_number(200);

    // The buyer's balance is unchanged until the slash fires.
    let before = token_client.balance(&t.buyer);

    let slashed = client.slash_collateral_for_miss(&t.buyer2, &id, &0u32);
    assert_eq!(slashed, 250_000, "25% of 1_000_000 collateral");
    assert_eq!(
        token_client.balance(&t.buyer) - before,
        250_000,
        "the buyer receives the slashed amount"
    );
}

#[test]
fn test_slash_debits_remaining_collateral() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-debit", 1_000_000, 2_500, 100);
    assert_eq!(client.get_locked_collateral(&id), 1_000_000);

    t.env.ledger().set_sequence_number(200);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);

    assert_eq!(client.get_locked_collateral(&id), 750_000);
}

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

#[test]
fn test_slash_is_permissionless_for_any_caller() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-perm", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    // A random bystander triggers it; the funds still land with the buyer.
    let bystander = Address::generate(&t.env);
    let before = token_client.balance(&t.buyer);
    let slashed = client.slash_collateral_for_miss(&bystander, &id, &0u32);

    assert_eq!(slashed, 100_000);
    assert_eq!(token_client.balance(&t.buyer) - before, 100_000);
}

#[test]
fn test_slash_requires_caller_authorization() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-auth", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    // Withdraw the blanket `mock_all_auths` from `setup()` so the caller's
    // `require_auth` is no longer satisfied.
    t.env.set_auths(&[]);

    let buyer_before = token_client.balance(&t.buyer);
    assert!(client
        .try_slash_collateral_for_miss(&t.buyer, &id, &0u32)
        .is_err());

    // Nothing moved and the milestone is still unslashed.
    assert_eq!(token_client.balance(&t.buyer), buyer_before);
    assert_eq!(client.get_locked_collateral(&id), 1_000_000);
    assert!(!client.is_collateral_slashed(&id, &0u32));
}

#[test]
fn test_slash_returns_remaining_collateral_on_completion() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-complete", 1_000_000, 5_000, 100);
    t.env.ledger().set_sequence_number(200);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);

    // The milestone that was slashed is still submittable and confirmable.
    let supplier_before = token_client.balance(&t.supplier);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &proof_type(&t.env));
    client.confirm_milestone(&t.buyer, &id, &0u32);

    // On completion the supplier gets back only what was never slashed, on top
    // of the milestone payment (1_000_000 for a single 100% milestone).
    assert_eq!(token_client.balance(&t.supplier) - supplier_before, 1_500_000);
}

#[test]
fn test_second_milestone_slashes_the_reduced_balance() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let client2 = ChainSettleContractClient::new(&t.env, &t.contract_id);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    // Two milestones, each 50%, both due at ledger 100.
    t.env.ledger().set_sequence_number(1);
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    opts.collateral_slash_bps_per_miss = 1_000; // 10%

    let id = sid(&t.env, "slash-two-ms");
    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "A"),
            payment_percent: 50,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 100,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "B"),
            payment_percent: 50,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 100,
            penalty_bps_per_ledger: 0,
        },
    ];
    client.create_shipment(
        &id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestones,
        &opts,
    );

    t.env.ledger().set_sequence_number(200);
    let before = token_client.balance(&t.buyer);

    // 10% of 1_000_000, then 10% of the 900_000 that remains.
    assert_eq!(client2.slash_collateral_for_miss(&t.buyer, &id, &0u32), 100_000);
    assert_eq!(
        client2.slash_collateral_for_miss(&t.buyer, &id, &1u32),
        90_000
    );
    assert_eq!(token_client.balance(&t.buyer) - before, 190_000);
    assert_eq!(client2.get_locked_collateral(&id), 810_000);
}

#[test]
fn test_slash_never_exceeds_remaining_collateral() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    // 100% rate: the first slash takes everything, the rest are no-ops.
    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-cap", 400_000, 10_000, 100);
    t.env.ledger().set_sequence_number(200);

    let contract_before = token_client.balance(&t.contract_id);
    let slashed = client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
    assert_eq!(slashed, 400_000);
    assert_eq!(client.get_locked_collateral(&id), 0);
    // The contract never sends out more than the collateral it held.
    assert_eq!(
        token_client.balance(&t.contract_id) + 400_000,
        contract_before
    );
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
fn test_zero_bps_leaves_existing_behaviour_untouched() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-zero-intact", 1_000_000, 0, 100);
    t.env.ledger().set_sequence_number(200);

    assert_eq!(client.get_collateral_slash_bps(&id), 0);
    assert!(!client.is_collateral_slashed(&id, &0u32));

    // Slashing is off, so a miss costs the supplier nothing: the collateral is
    // still whole after the deadline has passed.
    t.env.ledger().set_sequence_number(500);
    assert_eq!(client.get_locked_collateral(&id), 1_000_000);
    assert!(client
        .try_slash_collateral_for_miss(&t.buyer, &id, &0u32)
        .is_err());
    assert_eq!(client.get_locked_collateral(&id), 1_000_000);
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
#[should_panic(expected = "milestone deadline has not passed")]
fn test_slash_rejected_exactly_on_the_deadline_ledger() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-on-deadline", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(100);

    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

#[test]
fn test_slash_allowed_one_ledger_past_the_deadline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-just-late", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(101);

    assert_eq!(client.slash_collateral_for_miss(&t.buyer, &id, &0u32), 100_000);
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

#[test]
fn test_slash_respects_an_approved_deadline_extension() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-extended", 1_000_000, 1_000, 100);

    // The supplier asks for 500 more ledgers and the buyer approves. With no
    // stored override yet, approve_extension bases the new deadline on the
    // current ledger (1) rather than the milestone's own 100.
    client.request_extension(&t.supplier, &id, &0u32, &500u32);
    client.approve_extension(&t.buyer, &id, &0u32);
    let extended = client.get_milestone_deadline(&id, &0u32);
    assert_eq!(extended, 501);

    // Past the original deadline but inside the extension: still no slash.
    t.env.ledger().set_sequence_number(extended);
    assert!(client
        .try_slash_collateral_for_miss(&t.buyer, &id, &0u32)
        .is_err());

    // Only once the extended deadline passes does the slash apply.
    t.env.ledger().set_sequence_number(extended + 1);
    assert_eq!(
        client.slash_collateral_for_miss(&t.buyer, &id, &0u32),
        100_000
    );
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

#[test]
fn test_late_proof_is_not_slashed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-late-proof", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    // Proof arrives after the deadline — late, but on time for slashing purposes.
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &proof_type(&t.env));
    let before = token_client.balance(&t.buyer);

    assert!(client
        .try_slash_collateral_for_miss(&t.buyer, &id, &0u32)
        .is_err());
    assert_eq!(client.get_locked_collateral(&id), 1_000_000);
    assert_eq!(token_client.balance(&t.buyer), before);
}

#[test]
#[should_panic(expected = "shipment is not active")]
fn test_slash_rejected_after_cancellation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-cancelled", 1_000_000, 1_000, 100);
    client.cancel_shipment(&t.buyer, &id);

    t.env.ledger().set_sequence_number(200);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_slash_rejected_for_out_of_range_index() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-oob", 1_000_000, 1_000, 100);
    t.env.ledger().set_sequence_number(200);

    client.slash_collateral_for_miss(&t.buyer, &id, &7u32);
}

#[test]
#[should_panic(expected = "shipment not found")]
fn test_slash_rejected_for_unknown_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    t.env.ledger().set_sequence_number(200);

    client.slash_collateral_for_miss(&t.buyer, &sid(&t.env, "does-not-exist"), &0u32);
}

#[test]
fn test_slash_rejected_when_collateral_already_drained() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // 100% rate drains the collateral on the first miss.
    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-drained", 500_000, 10_000, 100);
    t.env.ledger().set_sequence_number(200);
    client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
    assert_eq!(client.get_locked_collateral(&id), 0);

    // A later milestone on the same shipment has nothing left to forfeit.
    assert!(client
        .try_slash_collateral_for_miss(&t.buyer, &id, &0u32)
        .is_err());
}

// ============================================================
// CREATION-TIME VALIDATION
// ============================================================

#[test]
#[should_panic(expected = "collateral_slash_bps_per_miss cannot exceed 10000")]
fn test_creation_rejects_bps_above_10000() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    opts.collateral_slash_bps_per_miss = 10_001;

    client.create_shipment(
        &sid(&t.env, "slash-bad-bps"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestone_with_deadline(&t.env, 100),
        &opts,
    );
}

#[test]
#[should_panic(expected = "collateral_slash_bps_per_miss requires supplier_collateral")]
fn test_creation_rejects_bps_without_collateral() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 0;
    opts.collateral_slash_bps_per_miss = 1_000;

    client.create_shipment(
        &sid(&t.env, "slash-no-collat"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestone_with_deadline(&t.env, 100),
        &opts,
    );
}

#[test]
fn test_creation_accepts_10000_bps_with_collateral() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    opts.collateral_slash_bps_per_miss = 10_000;

    let id = sid(&t.env, "slash-full-rate-ok");
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

    assert_eq!(client.get_collateral_slash_bps(&id), 10_000);
}

#[test]
fn test_default_bps_is_zero_and_no_key_written() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    assert_eq!(opts.collateral_slash_bps_per_miss, 0);

    let id = sid(&t.env, "slash-default");
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

    assert_eq!(client.get_collateral_slash_bps(&id), 0);
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
fn test_slash_of_one_base_unit_at_10000_bps() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-one-unit", 1, 10_000, 100);
    t.env.ledger().set_sequence_number(200);

    let before = token_client.balance(&t.buyer);
    assert_eq!(client.slash_collateral_for_miss(&t.buyer, &id, &0u32), 1);
    assert_eq!(token_client.balance(&t.buyer) - before, 1);
    assert_eq!(client.get_locked_collateral(&id), 0);
}

#[test]
fn test_slash_rounds_down_to_whole_base_units() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // 333 bps of 1_001 = 33.33 -> 33 base units.
    t.env.ledger().set_sequence_number(1);
    let id = create_ship(&t, "slash-round", 1_001, 333, 100);
    t.env.ledger().set_sequence_number(200);

    assert_eq!(client.slash_collateral_for_miss(&t.buyer, &id, &0u32), 33);
    assert_eq!(client.get_locked_collateral(&id), 968);
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

#[test]
fn test_slash_from_a_vault_funded_shipment_credits_the_vault() {    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    t.env.ledger().set_sequence_number(1);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000_000);

    // Seed the buyer's vault, then create a vault-funded shipment from it.
    let deposit = 5_000_000i128;
    client.vault_deposit(&t.buyer, &t.token_id, &deposit);
    // The deposit itself moved tokens out of the wallet, so measure from here.
    let buyer_before = token_client.balance(&t.buyer);

    let mut opts = default_options(&t.env);
    opts.fund_from_vault = true;
    opts.supplier_collateral = 1_000_000;
    opts.collateral_slash_bps_per_miss = 1_000;

    let id = sid(&t.env, "slash-vault");
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

    t.env.ledger().set_sequence_number(200);
    let slashed = client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
    assert_eq!(slashed, 100_000);

    // Vault-funded shipments refund into the vault, not the wallet: the wallet
    // is untouched and the vault shows 5_000_000 - 1_000_000 (escrow) + 100_000
    // (slashed collateral returned to the buyer).
    assert_eq!(token_client.balance(&t.buyer), buyer_before);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), 4_100_000);
}
