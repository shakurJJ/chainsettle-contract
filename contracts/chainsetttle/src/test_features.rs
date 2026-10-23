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

    Setup {
        env,
        contract_id,
        token_id,
        buyer,
        supplier,
        logistics,
        arbiter,
    }
}

fn single_buyer(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

/// Build a single 100% milestone with an optional deadline_ledger and penalty_bps.
fn single_milestone(
    env: &Env,
    deadline_ledger: u32,
    penalty_bps_per_ledger: u32,
) -> soroban_sdk::Vec<Milestone> {
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
            not_before_ledger: 0,
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
        dispute_bond_bps: 0,
        arbiter_fee_bps: 0,
        logistics_fee_bps: 0,
        supplier_collateral: 0,
        expires_at_ledger: None,
        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,
        early_bonus_pool: 0,
        review_window_ledgers: None,
        milestone_splits: Vec::new(env),
        deadlines: Vec::new(env),
        dispute_timeout_seconds: 0,
        default_resolution: Resolution::Buyer,
        backup_arbiter: None,
        confirmation_cooldown_ledgers: None,
        arbiter_panel: Vec::new(env),
        jurisdiction: None,
        grace_period_ledgers: 0,
        quality_grades: Vec::new(env),
        milestone_quantities: Vec::new(env),
        retainage_bps: 0,
        warranty_bps: 0,
        warranty_ledgers: 0,
        inspector: None,
        inspected_milestones: Vec::new(env),
        proof_submitters: Vec::new(env),
        require_dual_attestation: false,
        fund_from_vault: false,
        milestone_suppliers: Vec::new(env),
        refund_recipient: None,
        require_encrypted_evidence: false,
    }
}

// ============================================================
// #99 — LATE-DELIVERY PENALTY
// ============================================================

// ============================================================
// #98 — EARLY COMPLETION BONUS
// ============================================================

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
    assert_eq!(
        m.status,
        MilestoneStatus::Confirmed,
        "auto-confirm should succeed after per-shipment window"
    );
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
    assert_eq!(
        m.status,
        MilestoneStatus::Confirmed,
        "auto-confirm should use global window as fallback"
    );
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
