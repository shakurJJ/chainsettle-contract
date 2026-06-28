#![cfg(test)]

//! Tests for issues #126 and #127.
//!
//! #126 — maximum-milestone (10) shipment full lifecycle completion test
//! #127 — duplicate shipment ID rejected on second create_shipment call

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

fn default_options(_env: &Env) -> ShipmentOptions {
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

fn single_buyer(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

fn one_hundred_percent_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Only"),
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ]
}

// ============================================================
// ISSUE #126 — 10-milestone (maximum) full lifecycle test
// ============================================================

/// Build exactly 10 milestones at 10% each.
fn build_ten_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    let mut ms = soroban_sdk::Vec::new(env);
    for i in 0u32..10u32 {
        let name = if i == 0 { String::from_str(env, "M0") }
            else if i == 1 { String::from_str(env, "M1") }
            else if i == 2 { String::from_str(env, "M2") }
            else if i == 3 { String::from_str(env, "M3") }
            else if i == 4 { String::from_str(env, "M4") }
            else if i == 5 { String::from_str(env, "M5") }
            else if i == 6 { String::from_str(env, "M6") }
            else if i == 7 { String::from_str(env, "M7") }
            else if i == 8 { String::from_str(env, "M8") }
            else { String::from_str(env, "M9") };
        ms.push_back(Milestone {
            name,
            payment_percent: 10,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        });
    }
    ms
}

/// Issue #126 — Full end-to-end lifecycle with exactly 10 milestones at 10% each.
///
/// Acceptance criteria (from spec):
/// - All 10 milestones confirmed successfully
/// - released_amount correct after each confirmation (10%, 20%, …, 100%)
/// - Final status is Completed
/// - Contract escrow balance is 0 after completion
#[test]
fn test_ten_milestone_full_lifecycle() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token_client = token::Client::new(&s.env, &s.token_id);

    // Lower min percent so 10% milestones are valid (default is 5, but be explicit).
    client.set_min_milestone_percent(&s.buyer, &10u32);

    let shipment_id = String::from_str(&s.env, "SHIP-10M");
    let total_amount: i128 = 1_000_000_000;

    client.create_shipment(
        &shipment_id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total_amount,
        &build_ten_milestones(&s.env),
        &default_options(&s.env),
    );

    let per_milestone = total_amount * 10 / 100; // 100_000_000

    for idx in 0u32..10u32 {
        client.submit_proof(
            &s.supplier,
            &shipment_id,
            &idx,
            &String::from_str(&s.env, "ipfs://proof"),
            &Symbol::new(&s.env, "ipfs"),
        );
        client.confirm_milestone(&s.buyer, &shipment_id, &idx);

        let shipment = client.get_shipment(&shipment_id);
        let expected_released = per_milestone * (idx as i128 + 1);

        // Verify cumulative released_amount after each confirmation.
        assert_eq!(
            shipment.released_amount,
            expected_released,
            "after milestone {}: released should be {}% of total",
            idx, (idx + 1) * 10
        );

        // Verify escrow balance decreases accordingly.
        assert_eq!(
            client.get_escrow_balance(&shipment_id),
            total_amount - expected_released,
            "escrow balance after milestone {}",
            idx
        );

        if idx < 9 {
            assert_eq!(shipment.status, ShipmentStatus::Active);
        }
    }

    let final_shipment = client.get_shipment(&shipment_id);
    assert_eq!(final_shipment.status, ShipmentStatus::Completed);
    assert_eq!(final_shipment.released_amount, total_amount);
    assert_eq!(client.get_escrow_balance(&shipment_id), 0);
    assert_eq!(token_client.balance(&s.supplier), total_amount);
}

/// Issue #126 — Verify intermediate released_amount after each individual confirmation.
#[test]
fn test_ten_milestone_intermediate_released_amounts() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);

    client.set_min_milestone_percent(&s.buyer, &10u32);

    let shipment_id = String::from_str(&s.env, "SHIP-10M-STEP");
    let total_amount: i128 = 1_000_000_000;

    client.create_shipment(
        &shipment_id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &total_amount,
        &build_ten_milestones(&s.env),
        &default_options(&s.env),
    );

    let per = total_amount * 10 / 100;

    for idx in 0u32..10u32 {
        client.submit_proof(
            &s.supplier,
            &shipment_id,
            &idx,
            &String::from_str(&s.env, "ipfs://step"),
            &Symbol::new(&s.env, "ipfs"),
        );
        client.confirm_milestone(&s.buyer, &shipment_id, &idx);

        assert_eq!(
            client.get_shipment(&shipment_id).released_amount,
            per * (idx as i128 + 1),
            "step {}: released_amount mismatch", idx
        );
    }
}

// ============================================================
// ISSUE #127 — Duplicate shipment ID rejected
// ============================================================

/// Issue #127 — Same caller creating a second shipment with the same ID panics.
#[test]
#[should_panic(expected = "shipment already exists")]
fn test_duplicate_shipment_id_same_caller() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let id = String::from_str(&s.env, "SHIP-DUP-001");

    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
        &500_000_000,
        &one_hundred_percent_milestone(&s.env),
        &default_options(&s.env),
    );

    // Second create with same ID — must panic.
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
        &999_999_999,
        &one_hundred_percent_milestone(&s.env),
        &default_options(&s.env),
    );
}

/// Issue #127 — Different caller using the same shipment ID is also rejected.
#[test]
#[should_panic(expected = "shipment already exists")]
fn test_duplicate_shipment_id_different_caller() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);

    let buyer2 = Address::generate(&s.env);
    token::StellarAssetClient::new(&s.env, &s.token_id).mint(&buyer2, &10_000_000_000i128);

    let id = String::from_str(&s.env, "SHIP-DUP-002");

    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
        &1_000_000_000,
        &one_hundred_percent_milestone(&s.env),
        &default_options(&s.env),
    );

    // Different buyer, same ID — must also panic.
    client.create_shipment(
        &id,
        &single_buyer(&s.env, &buyer2),
        &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
        &1_000_000_000,
        &one_hundred_percent_milestone(&s.env),
        &default_options(&s.env),
    );
}

/// Issue #127 — Original shipment state is unchanged after a failed duplicate attempt.
#[test]
fn test_duplicate_shipment_id_original_state_unchanged() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let id = String::from_str(&s.env, "SHIP-DUP-003");
    let original_amount: i128 = 750_000_000;

    client.create_shipment(
        &id,
        &single_buyer(&s.env, &s.buyer),
        &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
        &original_amount,
        &one_hundred_percent_milestone(&s.env),
        &default_options(&s.env),
    );

    let before = client.get_shipment(&id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.create_shipment(
            &id,
            &single_buyer(&s.env, &s.buyer),
            &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
            &999_000_000,
            &one_hundred_percent_milestone(&s.env),
            &default_options(&s.env),
        );
    }));

    assert!(result.is_err(), "duplicate create must panic");

    let after = client.get_shipment(&id);
    assert_eq!(after.total_amount, before.total_amount, "total_amount must be unchanged");
    assert_eq!(after.status, before.status, "status must be unchanged");
    assert_eq!(after.released_amount, before.released_amount, "released_amount must be unchanged");
}

/// Issue #127 — Unique IDs are always accepted (positive counterpart).
#[test]
fn test_unique_shipment_ids_always_accepted() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);

    let ids = [
        "SHIP-UNIQ-A", "SHIP-UNIQ-B", "SHIP-UNIQ-C", "SHIP-UNIQ-D", "SHIP-UNIQ-E",
    ];

    for id_str in ids.iter() {
        let id = String::from_str(&s.env, id_str);
        client.create_shipment(
            &id,
            &single_buyer(&s.env, &s.buyer),
            &s.supplier, &s.logistics, &s.arbiter, &s.token_id,
            &100_000_000,
            &one_hundred_percent_milestone(&s.env),
            &default_options(&s.env),
        );
        assert_eq!(client.get_shipment(&id).status, ShipmentStatus::Active);
    }
}
