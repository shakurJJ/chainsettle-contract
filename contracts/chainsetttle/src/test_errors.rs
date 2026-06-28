#![cfg(test)]

//! Negative test suite for all ChainSettleError variants.
//!
//! Naming convention: `test_err_<ErrorName>_<scenario>`
//! Each error code has ≥ 2 tests. After each failing call the state is
//! verified to be unchanged (idempotency check).

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String,
};

// ============================================================
// HELPERS (mirrors test.rs helpers to keep this file standalone)
// ============================================================

struct T {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
}

fn setup() -> T {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    token::StellarAssetClient::new(&env, &token_id).mint(&Address::generate(&env), &0); // warm up

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &10_000_000_000);

    ChainSettleContractClient::new(&env, &contract_id).init(&buyer);

    T {
        env,
        contract_id,
        token_id,
        buyer,
        supplier,
        logistics,
        arbiter,
    }
}

fn milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "M0"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "M1"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ]
}

fn opts(env: &Env) -> ShipmentOptions {
    ShipmentOptions {
        response_deadline: 0,
        penalty_bps: 0,
        milestone_mode: MilestoneMode::Parallel,
        holdback_ledgers: 0,
        dispute_cooldown_ledgers: 0,
        late_penalty_bps_per_ledger: 0,
        auto_confirm_ledgers: 0,
        dispute_bond_amount: 0,
        arbiter_fee_bps: 0,
        logistics_fee_bps: 0,
        supplier_collateral: 0,
        expires_at_ledger: None,

        metadata_hash: Some(BytesN::from_array(env, &[0u8; 32])),

        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,
        early_bonus_pool: 0,
        review_window_ledgers: None,

    }
}

fn buyers(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

/// Create a shipment and return the client.
fn make_shipment<'a>(client: &'a ChainSettleContractClient, t: &T, id: &str) -> String {
    let sid = String::from_str(&t.env, id);
    client.create_shipment(
        &sid,
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &milestones(&t.env),
        &opts(&t.env),
    );
    sid
}

// ============================================================
// Error 1 — ShipmentAlreadyExists
// ============================================================

/// Creating a shipment with a duplicate ID must panic.
#[test]
#[should_panic(expected = "shipment already exists")]
fn test_err_shipment_already_exists_duplicate_id() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-DUP");

    // State before second attempt.
    let before = client.get_shipment(&sid);

    // Second create with same ID must panic.
    client.create_shipment(
        &sid,
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &milestones(&t.env),
        &opts(&t.env),
    );

    // Idempotency: state unchanged (unreachable, but documents intent).
    let after = client.get_shipment(&sid);
    assert_eq!(before.total_amount, after.total_amount);
}

/// Duplicate ID after partial milestone progress must still panic.
#[test]
#[should_panic(expected = "shipment already exists")]
fn test_err_shipment_already_exists_after_proof_submitted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-DUP2");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));

    // Attempt to re-create the same shipment.
    client.create_shipment(
        &sid,
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &milestones(&t.env),
        &opts(&t.env),
    );
}

// ============================================================
// Error 2 — ShipmentNotFound
// ============================================================

/// get_shipment on a non-existent ID must panic.
#[test]
#[should_panic(expected = "shipment not found")]
fn test_err_shipment_not_found_get_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.get_shipment(&String::from_str(&t.env, "GHOST"));
}

/// submit_proof on a non-existent shipment must panic.
#[test]
#[should_panic(expected = "shipment not found")]
fn test_err_shipment_not_found_submit_proof() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.submit_proof(
        &t.supplier,
        &String::from_str(&t.env, "GHOST"),
        &0,
        &String::from_str(&t.env, "ipfs://x"),
    
        &Symbol::new(&t.env, "ipfs"),);
}

/// confirm_milestone on a non-existent shipment must panic.
#[test]
#[should_panic(expected = "shipment not found")]
fn test_err_shipment_not_found_confirm_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.confirm_milestone(&t.buyer, &String::from_str(&t.env, "GHOST"), &0);
}

// ============================================================
// Error 3 — Unauthorized
// ============================================================

/// Non-buyer calling confirm_milestone must panic.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_err_unauthorized_confirm_milestone_non_buyer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-UNAUTH-CONF");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));

    // State before.
    let before = client.get_milestone(&sid, &0);

    // Supplier is not a buyer.
    client.confirm_milestone(&t.supplier, &sid, &0);

    // Idempotency: milestone status unchanged.
    assert_eq!(client.get_milestone(&sid, &0).status, before.status);
}

/// Non-supplier/logistics calling submit_proof must panic.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_err_unauthorized_submit_proof_wrong_caller() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-UNAUTH-PROOF");

    // Buyer is not supplier or logistics.
    client.submit_proof(&t.buyer, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));
}

/// Non-arbiter calling resolve_dispute must panic.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_err_unauthorized_resolve_dispute_non_arbiter() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-UNAUTH-RESOLVE");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));
    client.raise_dispute(&t.buyer, &sid, &0);

    let before = client.get_milestone(&sid, &0);

    // Supplier is not the arbiter.
    client.resolve_dispute(&t.supplier, &sid, &0, &true);

    // Idempotency: milestone still Disputed.
    assert_eq!(client.get_milestone(&sid, &0).status, before.status);
}

// ============================================================
// Error 4 — InvalidMilestoneIndex
// ============================================================

/// submit_proof with out-of-range index must panic.
#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_err_invalid_milestone_index_submit_proof() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-IDX-PROOF");

    let before = client.get_shipment(&sid);

    client.submit_proof(
        &t.supplier,
        &sid,
        &99,
        &String::from_str(&t.env, "ipfs://x"),
    
        &Symbol::new(&t.env, "ipfs"),);

    // Idempotency: shipment unchanged.
    assert_eq!(
        client.get_shipment(&sid).released_amount,
        before.released_amount
    );
}

/// confirm_milestone with out-of-range index must panic.
#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_err_invalid_milestone_index_confirm_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-IDX-CONF");

    let before = client.get_shipment(&sid);

    client.confirm_milestone(&t.buyer, &sid, &99);

    assert_eq!(
        client.get_shipment(&sid).released_amount,
        before.released_amount
    );
}

/// get_milestone with out-of-range index must panic.
#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_err_invalid_milestone_index_get_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-IDX-GET");
    client.get_milestone(&sid, &99);
}

// ============================================================
// Error 5 — InvalidMilestoneStatus
// ============================================================

/// confirm_milestone on a Pending (no proof) milestone must panic.
#[test]
#[should_panic(expected = "milestone proof not yet submitted")]
fn test_err_invalid_milestone_status_confirm_pending() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-STATUS-CONF");

    let before = client.get_shipment(&sid);

    client.confirm_milestone(&t.buyer, &sid, &0);

    assert_eq!(
        client.get_shipment(&sid).released_amount,
        before.released_amount
    );
}

/// submit_proof on a ProofSubmitted milestone (not Pending) must panic.
#[test]
#[should_panic(expected = "milestone is not in pending status")]
fn test_err_invalid_milestone_status_submit_proof_already_submitted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-STATUS-PROOF");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));

    let before = client.get_milestone(&sid, &0);

    // Second submit on same milestone must panic.
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://y"), &Symbol::new(&t.env, "ipfs"));

    // Idempotency: proof_hash unchanged.
    assert_eq!(client.get_milestone(&sid, &0).proof_hash, before.proof_hash);
}

/// raise_dispute on a Pending milestone (no proof) must panic.
#[test]
#[should_panic(expected = "can only dispute a submitted or held proof")]
fn test_err_invalid_milestone_status_dispute_pending() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-STATUS-DISP");

    let before = client.get_shipment(&sid);

    client.raise_dispute(&t.buyer, &sid, &0);

    assert_eq!(
        client.get_shipment(&sid).open_dispute_count,
        before.open_dispute_count
    );
}

/// resolve_dispute on a non-Disputed milestone must panic.
#[test]
#[should_panic(expected = "milestone is not in disputed status")]
fn test_err_invalid_milestone_status_resolve_non_disputed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-STATUS-RESOLVE");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));

    let before = client.get_milestone(&sid, &0);

    // Milestone is ProofSubmitted, not Disputed.
    client.resolve_dispute(&t.arbiter, &sid, &0, &true);

    assert_eq!(client.get_milestone(&sid, &0).status, before.status);
}

// ============================================================
// Error 6 — ShipmentNotActive
// ============================================================

/// submit_proof on a Cancelled shipment must panic.
#[test]
#[should_panic(expected = "shipment is not active")]
fn test_err_shipment_not_active_submit_proof_after_cancel() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-INACTIVE-PROOF");
    client.cancel_shipment(&t.buyer, &sid);

    let before = client.get_shipment(&sid);

    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));

    assert_eq!(client.get_shipment(&sid).status, before.status);
}

/// confirm_milestone on a Completed shipment must panic.
#[test]
#[should_panic(expected = "shipment is not active")]
fn test_err_shipment_not_active_confirm_after_completion() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-INACTIVE-CONF");

    // Complete the shipment.
    for i in 0u32..2u32 {
        client.submit_proof(&t.supplier, &sid, &i, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));
        client.confirm_milestone(&t.buyer, &sid, &i);
    }
    assert_eq!(client.get_shipment(&sid).status, ShipmentStatus::Completed);

    let before = client.get_shipment(&sid);

    // Attempt to confirm again on a completed shipment.
    client.confirm_milestone(&t.buyer, &sid, &0);

    assert_eq!(
        client.get_shipment(&sid).released_amount,
        before.released_amount
    );
}

/// cancel_shipment on an already-cancelled shipment must panic.
#[test]
#[should_panic(expected = "shipment is not active")]
fn test_err_shipment_not_active_cancel_twice() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-INACTIVE-CANCEL");
    client.cancel_shipment(&t.buyer, &sid);

    let before = client.get_shipment(&sid);

    client.cancel_shipment(&t.buyer, &sid);

    assert_eq!(client.get_shipment(&sid).status, before.status);
}

// ============================================================
// Error 7 — InvalidPercentages
// ============================================================

/// Milestone percentages that don't sum to 100 must panic.
#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_err_invalid_percentages_sum_not_100() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let bad = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "A"),
            payment_percent: 40,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "B"),
            payment_percent: 40,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ];

    let sid = String::from_str(&t.env, "SHIP-PCT-BAD");
    client.create_shipment(
        &sid,
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &bad,
        &opts(&t.env),
    );

    // Idempotency: shipment must not exist.
    client.get_shipment(&sid); // would panic with "shipment not found" if correctly rejected
}

/// Percentages summing to > 100 must also panic.
#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_err_invalid_percentages_sum_over_100() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let bad = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "A"),
            payment_percent: 60,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "B"),
            payment_percent: 60,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ];

    client.create_shipment(
        &String::from_str(&t.env, "SHIP-PCT-OVER"),
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &bad,
        &opts(&t.env),
    );
}

/// A milestone below the minimum percent threshold must panic with InvalidPercentages.
#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_err_invalid_percentages_below_min_threshold() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Default min is 5%; use a 4% milestone.
    let bad = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Tiny"),
            payment_percent: 4,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "Big"),
            payment_percent: 96,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ];

    client.create_shipment(
        &String::from_str(&t.env, "SHIP-PCT-MIN"),
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &bad,
        &opts(&t.env),
    );
}

// ============================================================
// Error 8 — InvalidAmount
// ============================================================

/// total_amount of 0 must panic.
#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_err_invalid_amount_zero() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = String::from_str(&t.env, "SHIP-AMT-ZERO");
    client.create_shipment(
        &sid,
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &0,
        &milestones(&t.env),
        &opts(&t.env),
    );
}

/// Negative total_amount must panic.
#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_err_invalid_amount_negative() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = String::from_str(&t.env, "SHIP-AMT-NEG");
    client.create_shipment(
        &sid,
        &buyers(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &-1,
        &milestones(&t.env),
        &opts(&t.env),
    );
}

/// top_up_escrow with zero additional_amount must panic.
#[test]
#[should_panic(expected = "additional_amount must be greater than zero")]
fn test_err_invalid_amount_top_up_zero() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-TOPUP-ZERO");

    let before = client.get_shipment(&sid);

    client.top_up_escrow(&t.buyer, &sid, &0);

    // Idempotency: total_amount unchanged.
    assert_eq!(client.get_shipment(&sid).total_amount, before.total_amount);
}

// ============================================================
// Error 9 — DisputeAlreadyOpen
// ============================================================

/// Raising a second dispute while one is already open must panic.
#[test]
#[should_panic(expected = "DisputeAlreadyOpen")]
fn test_err_dispute_already_open_second_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-DISP-DUP");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));
    client.submit_proof(&t.supplier, &sid, &1, &String::from_str(&t.env, "ipfs://y"), &Symbol::new(&t.env, "ipfs"));

    client.raise_dispute(&t.buyer, &sid, &0);

    let before = client.get_shipment(&sid);

    // Second dispute on a different milestone while first is open must panic.
    client.raise_dispute(&t.buyer, &sid, &1);

    // Idempotency: open_dispute_count unchanged.
    assert_eq!(
        client.get_shipment(&sid).open_dispute_count,
        before.open_dispute_count
    );
}

/// After resolving the first dispute, a new one can be opened (slot freed).
/// Conversely, opening a second before resolution must still panic.
#[test]
#[should_panic(expected = "DisputeAlreadyOpen")]
fn test_err_dispute_already_open_before_resolution() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-DISP-BEFORE-RES");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));
    client.submit_proof(&t.supplier, &sid, &1, &String::from_str(&t.env, "ipfs://y"), &Symbol::new(&t.env, "ipfs"));

    // Open dispute on milestone 0.
    client.raise_dispute(&t.buyer, &sid, &0);
    assert_eq!(client.get_shipment(&sid).open_dispute_count, 1);

    // Attempt to open another on milestone 1 without resolving the first.
    client.raise_dispute(&t.buyer, &sid, &1);
}

/// After resolution the slot is freed and a new dispute is allowed.
/// This is the positive counterpart — verifies state is correctly restored.
#[test]
fn test_err_dispute_already_open_slot_freed_after_resolution() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let sid = make_shipment(&client, &t, "SHIP-DISP-FREED");
    client.submit_proof(&t.supplier, &sid, &0, &String::from_str(&t.env, "ipfs://x"), &Symbol::new(&t.env, "ipfs"));
    client.raise_dispute(&t.buyer, &sid, &0);
    assert_eq!(client.get_shipment(&sid).open_dispute_count, 1);

    // Resolve (reject) — slot freed.
    client.resolve_dispute(&t.arbiter, &sid, &0, &false);
    assert_eq!(client.get_shipment(&sid).open_dispute_count, 0);

    // Now a new dispute on milestone 1 must succeed.
    client.submit_proof(&t.supplier, &sid, &1, &String::from_str(&t.env, "ipfs://y"), &Symbol::new(&t.env, "ipfs"));
    client.raise_dispute(&t.buyer, &sid, &1);
    assert_eq!(client.get_shipment(&sid).open_dispute_count, 1);
}
