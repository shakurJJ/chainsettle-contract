#![cfg(test)]

//! Tests for issues #99, #98, and #96.
//!
//! #99 — per-ledger late-delivery penalty deducted from supplier payment
//! #98 — milestone early completion bonus from buyer-funded bonus pool
//! #96 — per-shipment configurable proof review window

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, Env, String, Symbol,
};

// ============================================================
// HELPERS
// ============================================================

struct Setup {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &10_000_000_000_000i128);

    ChainSettleContractClient::new(&env, &contract_id).init(&buyer);

    Setup { env, contract_id, token_id, buyer, supplier, logistics, arbiter }
}

fn single_buyer(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

/// Build a single 100% milestone with an optional deadline_ledger and penalty_bps.
fn single_milestone(env: &Env, deadline_ledger: u32, penalty_bps_per_ledger: u32) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Delivery"),
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger,
            penalty_bps_per_ledger,
        },
    ]
}

fn default_opts(env: &Env) -> ShipmentOptions {
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
        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,
        early_bonus_pool: 0,
        review_window_ledgers: None,
    }
}

// ============================================================
// #99 — LATE-DELIVERY PENALTY
// ============================================================

/// On-time proof submission (at or before deadline): no penalty.
#[test]
fn test_penalty_no_penalty_on_time_submission() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let deadline: u32 = 100;
    let penalty_bps: u32 = 500; // 5% per ledger

    let id = String::from_str(&s.env, "SHIP-ONTIME");

    // Set ledger to before deadline.
    s.env.ledger().set_sequence_number(50);

    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, penalty_bps),
        &default_opts(&s.env),
    );

    let supplier_before = token_client.balance(&s.supplier);

    // Submit proof exactly at deadline (no overdue ledgers).
    s.env.ledger().set_sequence_number(deadline);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    client.confirm_milestone(&s.buyer, &id, &0);

    // Supplier receives full payment — no penalty.
    let supplier_after = token_client.balance(&s.supplier);
    assert_eq!(supplier_after - supplier_before, total, "no penalty expected for on-time submission");
}

/// One ledger overdue: penalty = 1 * penalty_bps / 10_000 * payment.
#[test]
fn test_penalty_one_ledger_overdue() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let deadline: u32 = 100;
    let penalty_bps: u32 = 500; // 5% per ledger

    let id = String::from_str(&s.env, "SHIP-1LATE");

    s.env.ledger().set_sequence_number(50);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, penalty_bps),
        &default_opts(&s.env),
    );

    let buyer_before = token_client.balance(&s.buyer);
    let supplier_before = token_client.balance(&s.supplier);

    // Submit proof 1 ledger after deadline.
    s.env.ledger().set_sequence_number(deadline + 1);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    client.confirm_milestone(&s.buyer, &id, &0);

    // Expected penalty: 1 overdue ledger * 500 bps / 10_000 * 1_000_000 = 50_000
    let expected_penalty: i128 = (total * (penalty_bps as i128 * 1)) / 10_000;
    let supplier_after = token_client.balance(&s.supplier);
    let buyer_after = token_client.balance(&s.buyer);

    assert_eq!(supplier_after - supplier_before, total - expected_penalty, "supplier should receive total minus penalty");
    assert_eq!(buyer_after - buyer_before, expected_penalty, "penalty returned to buyer");
}

/// Penalty capped at 50% of milestone payment.
#[test]
fn test_penalty_capped_at_50_percent() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let deadline: u32 = 100;
    let penalty_bps: u32 = 1_000; // 10% per ledger — 6 overdue would be 60%, but capped at 50%

    let id = String::from_str(&s.env, "SHIP-CAPPED");

    s.env.ledger().set_sequence_number(50);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, penalty_bps),
        &default_opts(&s.env),
    );

    let buyer_before = token_client.balance(&s.buyer);
    let supplier_before = token_client.balance(&s.supplier);

    // Submit proof 6 ledgers late (10% * 6 = 60%, capped at 50%).
    s.env.ledger().set_sequence_number(deadline + 6);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    client.confirm_milestone(&s.buyer, &id, &0);

    let cap = total / 2; // 500_000
    let supplier_after = token_client.balance(&s.supplier);
    let buyer_after = token_client.balance(&s.buyer);

    assert_eq!(
        supplier_after - supplier_before,
        total - cap,
        "supplier should receive total minus capped penalty"
    );
    assert_eq!(buyer_after - buyer_before, cap, "buyer receives capped penalty refund");
}

/// Zero penalty_bps_per_ledger disables per-milestone penalty; no deduction even when overdue.
#[test]
fn test_penalty_zero_bps_baseline_no_penalty() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let deadline: u32 = 100;

    let id = String::from_str(&s.env, "SHIP-NOPEN");

    s.env.ledger().set_sequence_number(50);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, 0), // 0 = disabled
        &default_opts(&s.env),
    );

    let supplier_before = token_client.balance(&s.supplier);

    // Submit proof 50 ledgers late — but penalty_bps = 0, so no penalty.
    s.env.ledger().set_sequence_number(deadline + 50);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    client.confirm_milestone(&s.buyer, &id, &0);

    let supplier_after = token_client.balance(&s.supplier);
    assert_eq!(supplier_after - supplier_before, total, "zero penalty_bps should disable penalty");
}

// ============================================================
// #98 — EARLY COMPLETION BONUS
// ============================================================

/// Early confirmation (at or before deadline): bonus paid to supplier.
#[test]
fn test_bonus_early_confirmation_pays_bonus() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let bonus_pool: i128 = 100_000;
    let deadline: u32 = 200;

    let id = String::from_str(&s.env, "SHIP-BONUS-EARLY");

    s.env.ledger().set_sequence_number(10);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, 0),
        &ShipmentOptions {
            early_bonus_pool: bonus_pool,
            ..default_opts(&s.env)
        },
    );

    let supplier_before = token_client.balance(&s.supplier);

    s.env.ledger().set_sequence_number(50); // before deadline
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    // Confirm before deadline.
    s.env.ledger().set_sequence_number(100); // still before deadline of 200
    client.confirm_milestone(&s.buyer, &id, &0);

    let supplier_after = token_client.balance(&s.supplier);
    // Supplier should get total payment + bonus (1 milestone = 100% of bonus pool).
    assert_eq!(supplier_after - supplier_before, total + bonus_pool, "supplier should receive payment + bonus");
}

/// Late confirmation (after deadline): no bonus paid.
#[test]
fn test_bonus_late_confirmation_no_bonus() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let bonus_pool: i128 = 100_000;
    let deadline: u32 = 100;

    let id = String::from_str(&s.env, "SHIP-BONUS-LATE");

    s.env.ledger().set_sequence_number(10);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, 0),
        &ShipmentOptions {
            early_bonus_pool: bonus_pool,
            ..default_opts(&s.env)
        },
    );

    let supplier_before = token_client.balance(&s.supplier);
    let buyer_before = token_client.balance(&s.buyer);

    // Confirm after deadline — no bonus.
    s.env.ledger().set_sequence_number(50);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    s.env.ledger().set_sequence_number(deadline + 1); // after deadline
    client.confirm_milestone(&s.buyer, &id, &0);

    let supplier_after = token_client.balance(&s.supplier);
    let buyer_after = token_client.balance(&s.buyer);

    // Supplier gets only the milestone payment; unused pool returned to buyer on completion.
    assert_eq!(supplier_after - supplier_before, total, "no bonus for late confirmation");
    assert_eq!(buyer_after - buyer_before, bonus_pool, "unused bonus pool returned to buyer");
}

/// Unused bonus pool returned to buyer when shipment completes without earning bonuses.
#[test]
fn test_bonus_unused_pool_returned_to_buyer_on_completion() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let bonus_pool: i128 = 200_000;
    let deadline: u32 = 100;

    let id = String::from_str(&s.env, "SHIP-UNUSED-POOL");

    s.env.ledger().set_sequence_number(10);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, deadline, 0),
        &ShipmentOptions {
            early_bonus_pool: bonus_pool,
            ..default_opts(&s.env)
        },
    );

    let buyer_before = token_client.balance(&s.buyer);

    // Submit and confirm after deadline so bonus is not earned.
    s.env.ledger().set_sequence_number(50);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    s.env.ledger().set_sequence_number(deadline + 10);
    client.confirm_milestone(&s.buyer, &id, &0);

    // After completion, buyer should have received the unused bonus pool back.
    let buyer_after = token_client.balance(&s.buyer);
    assert_eq!(
        buyer_after - buyer_before,
        bonus_pool,
        "full unused bonus pool returned to buyer on completion"
    );

    let shipment = client.get_shipment(&id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.early_bonus_remaining, 0);
}

/// early_bonus_pool = 0 disables feature; no extra transfer from buyer.
#[test]
fn test_bonus_zero_pool_baseline() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    let total: i128 = 1_000_000;
    let id = String::from_str(&s.env, "SHIP-ZERO-POOL");

    s.env.ledger().set_sequence_number(10);
    let buyer_before = token_client.balance(&s.buyer);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total,
        &single_milestone(&s.env, 200, 0),
        &ShipmentOptions {
            early_bonus_pool: 0,
            ..default_opts(&s.env)
        },
    );
    let buyer_after_create = token_client.balance(&s.buyer);
    // Buyer only paid total_amount (no bonus pool).
    assert_eq!(buyer_before - buyer_after_create, total);

    let supplier_before = token_client.balance(&s.supplier);
    s.env.ledger().set_sequence_number(50);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    client.confirm_milestone(&s.buyer, &id, &0);
    let supplier_after = token_client.balance(&s.supplier);
    assert_eq!(supplier_after - supplier_before, total, "full payment, no bonus");
}

// ============================================================
// #96 — PER-SHIPMENT CONFIGURABLE PROOF REVIEW WINDOW
// ============================================================

/// Per-shipment review_window_ledgers overrides global auto_confirm_ledgers.
#[test]
fn test_review_window_per_shipment_respected() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);

    let per_shipment_window: u32 = 50;
    let id = String::from_str(&s.env, "SHIP-WINDOW");

    s.env.ledger().set_sequence_number(10);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &1_000_000,
        &single_milestone(&s.env, 0, 0),
        &ShipmentOptions {
            review_window_ledgers: Some(per_shipment_window),
            ..default_opts(&s.env)
        },
    );

    // Submit proof at ledger 10; window expires at 10 + 50 = 60.
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    // Advance past the per-shipment window.
    s.env.ledger().set_sequence_number(10 + per_shipment_window);

    // claim_auto_confirmation should succeed.
    client.claim_auto_confirmation(&id, &0);
    let m = client.get_milestone(&id, &0);
    assert_eq!(m.status, MilestoneStatus::Confirmed, "auto-confirm should succeed after per-shipment window");
}

/// Global default used when review_window_ledgers is None.
#[test]
fn test_review_window_global_fallback() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);

    let global_window: u32 = 75;
    // Set global admin default.
    client.set_auto_confirm_threshold(&s.buyer, &global_window);

    let id = String::from_str(&s.env, "SHIP-GLOBAL-WINDOW");

    s.env.ledger().set_sequence_number(10);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &1_000_000,
        &single_milestone(&s.env, 0, 0),
        &ShipmentOptions {
            review_window_ledgers: None, // should fall back to global
            auto_confirm_ledgers: 0,
            ..default_opts(&s.env)
        },
    );

    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    // Advance past the global window.
    s.env.ledger().set_sequence_number(10 + global_window);

    client.claim_auto_confirmation(&id, &0);
    let m = client.get_milestone(&id, &0);
    assert_eq!(m.status, MilestoneStatus::Confirmed, "auto-confirm should use global window as fallback");
}

/// review_window_ledgers = Some(0) opts out of auto-confirm even when global is set.
#[test]
#[should_panic(expected = "auto-confirmation not enabled for this shipment")]
fn test_review_window_per_shipment_opt_out() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);

    // Set global window so we can verify per-shipment opt-out takes precedence.
    client.set_auto_confirm_threshold(&s.buyer, &100);

    let id = String::from_str(&s.env, "SHIP-OPTOUT");

    s.env.ledger().set_sequence_number(10);
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &1_000_000,
        &single_milestone(&s.env, 0, 0),
        &ShipmentOptions {
            review_window_ledgers: Some(0), // opt-out
            ..default_opts(&s.env)
        },
    );

    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "ipfs://proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    // Even after 200 ledgers, auto-confirm should be disabled.
    s.env.ledger().set_sequence_number(210);
    client.claim_auto_confirmation(&id, &0); // should panic
}
