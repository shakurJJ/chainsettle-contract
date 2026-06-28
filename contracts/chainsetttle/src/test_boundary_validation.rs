#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String,
};
use std::format;

// ============================================================
// TEST SETUP
// ============================================================

struct TestSetup {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
}

fn setup() -> TestSetup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = token::StellarAssetClient::new(&env, &token_id);

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    // Mint sufficient tokens for all test scenarios
    token_client.mint(&buyer, &1_000_000_000_000);

    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    TestSetup {
        env,
        contract_id,
        token_id,
        buyer,
        supplier,
        logistics,
        arbiter,
    }
}

fn build_valid_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Milestone 1"),
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

fn build_zero_percent_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Zero Percent Milestone"),
            payment_percent: 0,
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

fn build_low_percent_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Low Percent Milestone"),
            payment_percent: 1,
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

fn build_multi_milestone_with_zero(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Milestone 1"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "Milestone 2 (Zero Percent)"),
            payment_percent: 0,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "Milestone 3"),
            payment_percent: 50,
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

fn single_buyer_vec(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
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

// ============================================================
// TEST 1: Zero total_amount Rejected
// ============================================================

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_create_shipment_zero_total_amount_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-ZERO-AMT");
    let total_amount: i128 = 0;

    // Attempt to create shipment with zero total_amount
    // Should panic: "amount must be greater than zero"
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_valid_milestone(&t.env),
        &default_options(&t.env),
    );
}

// ============================================================
// TEST 2: Negative total_amount (-1) Rejected
// ============================================================

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_create_shipment_negative_total_amount_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-NEG-AMT");
    let total_amount: i128 = -1;

    // Attempt to create shipment with negative total_amount
    // Should panic: "amount must be greater than zero"
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_valid_milestone(&t.env),
        &default_options(&t.env),
    );
}

// ============================================================
// TEST 3: Large Negative total_amount Rejected
// ============================================================

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_create_shipment_large_negative_total_amount_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-LARGE-NEG");
    let total_amount: i128 = -1_000_000_000;

    // Attempt to create shipment with large negative total_amount
    // Should panic: "amount must be greater than zero"
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_valid_milestone(&t.env),
        &default_options(&t.env),
    );
}

// ============================================================
// TEST 4: Single Milestone with payment_percent = 0 Rejected
// ============================================================

#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_create_shipment_single_milestone_zero_percent_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-ZERO-PCT");
    let total_amount: i128 = 1_000_000_000;

    // Attempt to create shipment with single milestone at 0%
    // Should panic: "InvalidPercentages"
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_zero_percent_milestone(&t.env),
        &default_options(&t.env),
    );
}

// ============================================================
// TEST 5: Multiple Milestones with One at 0% Rejected
// ============================================================

#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_create_shipment_multi_milestone_with_zero_percent_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MULTI-ZERO");
    let total_amount: i128 = 1_000_000_000;

    // Attempt to create shipment with 3 milestones, middle one at 0%
    // Should panic: "InvalidPercentages"
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_multi_milestone_with_zero(&t.env),
        &default_options(&t.env),
    );
}

// ============================================================
// TEST 6: Minimum Valid Amount (1) Accepted
// ============================================================

#[test]
fn test_create_shipment_minimum_valid_amount_accepted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MIN-VALID");
    let total_amount: i128 = 1; // Minimum valid amount

    // Create shipment with minimum valid amount
    // Should succeed (no panic)
    let result = client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_valid_milestone(&t.env),
        &default_options(&t.env),
    );

    // Verify shipment was created successfully
    assert_eq!(result, shipment_id);
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 1);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

// ============================================================
// TEST 7: Small Valid Amount (100) Accepted
// ============================================================

#[test]
fn test_create_shipment_small_valid_amount_accepted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-SMALL-VALID");
    let total_amount: i128 = 100;

    // Create shipment with small valid amount
    // Should succeed (no panic)
    let result = client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_valid_milestone(&t.env),
        &default_options(&t.env),
    );

    // Verify shipment was created successfully
    assert_eq!(result, shipment_id);
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 100);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

// ============================================================
// TEST 8: Milestone with Min Percent (5%) Accepted
// ============================================================

#[test]
fn test_create_shipment_milestone_min_percent_accepted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MIN-PCT");
    let total_amount: i128 = 1_000_000_000;

    // Create milestone with minimum valid percentage (default min_pct = 5%)
    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Milestone 1"),
            payment_percent: 5, // Minimum valid percentage
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "Milestone 2"),
            payment_percent: 95,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ];

    // Should succeed with minimum valid percentages
    let result = client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &milestones,
        &default_options(&t.env),
    );

    assert_eq!(result, shipment_id);
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().payment_percent, 5);
    assert_eq!(shipment.milestones.get(1).unwrap().payment_percent, 95);
}

// ============================================================
// TEST 9: Below Minimum Percentage (4%) Rejected
// ============================================================

#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_create_shipment_below_min_percent_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-BELOW-MIN-PCT");
    let total_amount: i128 = 1_000_000_000;

    // Create milestone with percentage below minimum (< 5%)
    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Milestone 1"),
            payment_percent: 4, // Below minimum (< 5%)
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "Milestone 2"),
            payment_percent: 96,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ];

    // Should panic: "InvalidPercentages"
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &milestones,
        &default_options(&t.env),
    );
}

// ============================================================
// TEST 10: Large Valid Amount Accepted
// ============================================================

#[test]
fn test_create_shipment_large_valid_amount_accepted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-LARGE-VALID");
    let total_amount: i128 = 999_999_999_999; // Large but valid amount

    // Create shipment with large valid amount
    let result = client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_valid_milestone(&t.env),
        &default_options(&t.env),
    );

    assert_eq!(result, shipment_id);
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 999_999_999_999);
}

// ============================================================
// TEST 11: All Zero and Negative Amount Cases Summary
// ============================================================

#[test]
fn test_boundary_validation_coverage_summary() {
    // This test documents the comprehensive coverage of boundary validation:
    
    // Amount Validation:
    // ✓ Test 1: Zero amount (0) rejected
    // ✓ Test 2: Negative amount (-1) rejected
    // ✓ Test 3: Large negative amount rejected
    // ✓ Test 6: Minimum valid amount (1) accepted
    // ✓ Test 7: Small valid amount (100) accepted
    // ✓ Test 10: Large valid amount accepted
    
    // Milestone Percentage Validation:
    // ✓ Test 4: Single milestone with 0% rejected
    // ✓ Test 5: Multiple milestones with one at 0% rejected
    // ✓ Test 8: Milestone at minimum valid % (5%) accepted
    // ✓ Test 9: Milestone below minimum % (4%) rejected
    
    // Error Messages Verified:
    // ✓ "amount must be greater than zero" for invalid amounts
    // ✓ "InvalidPercentages" for invalid milestone percentages
    
    // Boundary Test Matrix:
    // Zero/Negative: ✓ Covered
    // Minimum Valid: ✓ Covered
    // Valid Range: ✓ Covered
    // Large Values: ✓ Covered
    
    assert_eq!(1, 1); // Trivial assertion; this test documents coverage
}
