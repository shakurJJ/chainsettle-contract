#![cfg(test)]

extern crate std;

// Tests for two new proposed features:
// 1. Per-milestone insurance holdback (insurance_bps)
// 2. Per-milestone oracle price conditions (price_condition)

use crate::{
    test_common::*,
    ChainSettleContract, ChainSettleContractClient, Milestone, MilestoneStatus, Resolution,
    ShipmentOptions, ShipmentStatus,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, token, Address, Env, String,
};

// ============================================================
// MOCK ORACLE FOR PRICE CONDITIONS
// ============================================================

#[contracttype]
pub enum OracleKey {
    Price(Address), // asset -> price
}

/// Simple mock oracle that stores price per asset.
/// Used for testing price_condition feature.
#[contract]
pub struct MockPriceOracle;

#[contractimpl]
impl MockPriceOracle {
    /// Set the current price for an asset (in 7 decimals: 1.0 = 10_000_000)
    pub fn set_price(env: Env, asset: Address, price: i128) {
        env.storage()
            .instance()
            .set(&OracleKey::Price(asset), &price);
    }

    /// Get the latest price for an asset
    pub fn get_price(env: Env, asset: Address) -> i128 {
        env.storage()
            .instance()
            .get(&OracleKey::Price(asset))
            .unwrap_or(0)
    }
}

// ============================================================
// TEST 1: BASIC INSURANCE HOLDBACK - ON-TIME RELEASE
// ============================================================

#[test]
fn test_insurance_holdback_released_on_time_confirmation() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    let token_client = token::StellarAssetClient::new(&setup.env, &setup.token_id);

    let shipment_id = String::from_str(&setup.env, "INS-001");
    let total_amount: i128 = 1_000_000;
    let insurance_bps: u32 = 200; // 2% holdback

    // Create shipment with insurance holdback
    let mut opts = default_options(&setup.env);
    opts.holdback_ledgers = 0; // No time-based holdback for this test

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &opts,
    );

    // Submit proof and confirm milestone 0 (25% = 250,000)
    let proof = String::from_str(&setup.env, "delivery_proof");
    client.submit_proof(&shipment_id, &0, &proof);

    let supplier_balance_before = token_client.balance(&setup.supplier);

    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    // Expected payout calculation:
    // Gross payment: 250,000 (25% of 1,000,000)
    // Insurance holdback: 5,000 (2% of 250,000)
    // Net immediate payout: 245,000
    let supplier_balance_after = token_client.balance(&setup.supplier);
    let expected_immediate_payout = 245_000; // 250,000 - 5,000
    assert_eq!(
        supplier_balance_after - supplier_balance_before,
        expected_immediate_payout
    );

    // Insurance holdback should be queryable
    let holdback_amount = client.get_milestone_insurance_holdback(&shipment_id, &0);
    assert_eq!(holdback_amount, 5_000);

    // On-time completion releases the insurance holdback to supplier
    client.release_insurance_holdback(&setup.buyer, &shipment_id, &0);

    let supplier_final_balance = token_client.balance(&setup.supplier);
    assert_eq!(supplier_final_balance - supplier_balance_after, 5_000);

    // Holdback should now be zero
    assert_eq!(
        client.get_milestone_insurance_holdback(&shipment_id, &0),
        0
    );
}

// ============================================================
// TEST 2: INSURANCE HOLDBACK - FORFEITED ON DISPUTE LOSS
// ============================================================

#[test]
fn test_insurance_holdback_forfeited_on_dispute_loss() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    let token_client = token::StellarAssetClient::new(&setup.env, &setup.token_id);

    let shipment_id = String::from_str(&setup.env, "INS-002");
    let total_amount: i128 = 2_000_000;
    let insurance_bps: u32 = 300; // 3% holdback

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // Submit proof and confirm milestone 0
    let proof = String::from_str(&setup.env, "disputed_delivery");
    client.submit_proof(&shipment_id, &0, &proof);
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    // Gross payment: 500,000 (25% of 2,000,000)
    // Insurance holdback: 15,000 (3% of 500,000)
    let holdback = client.get_milestone_insurance_holdback(&shipment_id, &0);
    assert_eq!(holdback, 15_000);

    // Buyer raises dispute on milestone 1
    let proof2 = String::from_str(&setup.env, "faulty_proof");
    client.submit_proof(&shipment_id, &1, &proof2);

    let reason = String::from_str(&setup.env, "Quality issue");
    client.raise_dispute(&setup.buyer, &shipment_id, &1, &reason);

    // Arbiter resolves AGAINST supplier (approve=false)
    client.resolve_dispute(&setup.arbiter, &shipment_id, &1, &false);

    let buyer_balance_before = token_client.balance(&setup.buyer);

    // On dispute loss, the insurance holdback from milestone 0 goes to buyer
    client.forfeit_insurance_to_buyer(&shipment_id, &1);

    let buyer_balance_after = token_client.balance(&setup.buyer);
    // Buyer should receive the forfeited insurance from milestone 1
    // Expected: 3% of milestone 1's gross payment
    let milestone_1_gross = total_amount * 50 / 100; // 50% = 1,000,000
    let expected_forfeit = milestone_1_gross * insurance_bps as i128 / 10_000; // 30,000
    assert_eq!(buyer_balance_after - buyer_balance_before, expected_forfeit);
}

// ============================================================
// TEST 3: INSURANCE HOLDBACK - MULTIPLE MILESTONES
// ============================================================

#[test]
fn test_insurance_holdback_across_multiple_milestones() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    let token_client = token::StellarAssetClient::new(&setup.env, &setup.token_id);

    let shipment_id = String::from_str(&setup.env, "INS-003");
    let total_amount: i128 = 3_000_000;
    let insurance_bps: u32 = 250; // 2.5% holdback

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // Confirm all three milestones
    for i in 0u32..3 {
        let proof = String::from_str(&setup.env, "proof");
        client.submit_proof(&shipment_id, &i, &proof);
        client.confirm_milestone(&setup.buyer, &shipment_id, &i);

        // Verify holdback is recorded for each milestone
        let holdback = client.get_milestone_insurance_holdback(&shipment_id, &i);
        let milestone_pct = match i {
            0 => 25,
            1 => 50,
            2 => 25,
            _ => 0,
        };
        let expected_holdback = (total_amount * milestone_pct / 100) * insurance_bps as i128 / 10_000;
        assert_eq!(holdback, expected_holdback);
    }

    // Total insurance held: 2.5% of 3,000,000 = 75,000
    let total_holdback = client.get_total_insurance_holdback(&shipment_id);
    assert_eq!(total_holdback, 75_000);

    // Release all insurance holdbacks (on-time delivery)
    for i in 0u32..3 {
        client.release_insurance_holdback(&setup.buyer, &shipment_id, &i);
    }

    // All holdbacks should be released
    assert_eq!(client.get_total_insurance_holdback(&shipment_id), 0);
}

// ============================================================
// TEST 4: ORACLE PRICE CONDITION - BLOCKED WHEN PRICE TOO LOW
// ============================================================

#[test]
#[should_panic(expected = "price condition not met")]
fn test_oracle_price_condition_blocks_confirmation_when_price_low() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    // Register mock oracle
    let oracle_addr = setup.env.register(MockPriceOracle, ());
    let oracle_client = MockPriceOracleClient::new(&setup.env, &oracle_addr);

    // Admin approves this oracle
    client.approve_oracle(&setup.buyer, &oracle_addr);

    let shipment_id = String::from_str(&setup.env, "ORACLE-001");
    let total_amount: i128 = 1_000_000;
    let min_price: i128 = 15_000_000; // 1.5 USD (7 decimals)

    // Set current price BELOW threshold
    oracle_client.set_price(&setup.token_id, &12_000_000); // 1.2 USD

    // Create shipment with price condition on milestone 0
    let mut opts = default_options(&setup.env);
    // Assume we add a field to ShipmentOptions for per-milestone price conditions
    // For now, this is illustrative of the intended API

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &opts,
    );

    // Set price condition on milestone 0
    client.set_milestone_price_condition(&setup.buyer, &shipment_id, &0, &oracle_addr, &min_price);

    // Submit proof
    let proof = String::from_str(&setup.env, "delivery_complete");
    client.submit_proof(&shipment_id, &0, &proof);

    // Attempt to confirm should FAIL because price (1.2) < min_price (1.5)
    client.confirm_milestone(&setup.buyer, &shipment_id, &0); // Should panic
}

// ============================================================
// TEST 5: ORACLE PRICE CONDITION - ALLOWED WHEN PRICE MEETS THRESHOLD
// ============================================================

#[test]
fn test_oracle_price_condition_allows_confirmation_when_price_met() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    let token_client = token::StellarAssetClient::new(&setup.env, &setup.token_id);

    // Register mock oracle
    let oracle_addr = setup.env.register(MockPriceOracle, ());
    let oracle_client = MockPriceOracleClient::new(&setup.env, &oracle_addr);

    // Admin approves this oracle
    client.approve_oracle(&setup.buyer, &oracle_addr);

    let shipment_id = String::from_str(&setup.env, "ORACLE-002");
    let total_amount: i128 = 5_000_000;
    let min_price: i128 = 10_000_000; // 1.0 USD

    // Set current price ABOVE threshold
    oracle_client.set_price(&setup.token_id, &18_000_000); // 1.8 USD

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // Set price condition
    client.set_milestone_price_condition(&setup.buyer, &shipment_id, &0, &oracle_addr, &min_price);

    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);

    let supplier_balance_before = token_client.balance(&setup.supplier);

    // Confirm should succeed because price (1.8) >= min_price (1.0)
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    let supplier_balance_after = token_client.balance(&setup.supplier);
    let milestone_payment = total_amount * 25 / 100; // 25% = 1,250,000
    assert_eq!(
        supplier_balance_after - supplier_balance_before,
        milestone_payment
    );

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

// ============================================================
// TEST 6: ORACLE PRICE CONDITION - NO CONDITION BEHAVES NORMALLY
// ============================================================

#[test]
fn test_no_price_condition_allows_normal_confirmation() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = String::from_str(&setup.env, "ORACLE-003");
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // No price condition set - should work normally
    let proof = String::from_str(&setup.env, "normal_proof");
    client.submit_proof(&shipment_id, &0, &proof);
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

// ============================================================
// TEST 7: ORACLE NOT APPROVED - SHOULD FAIL
// ============================================================

#[test]
#[should_panic(expected = "oracle not approved by admin")]
fn test_unapproved_oracle_rejected() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let oracle_addr = setup.env.register(MockPriceOracle, ());
    // DO NOT approve this oracle

    let shipment_id = String::from_str(&setup.env, "ORACLE-004");

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // Attempt to set price condition with unapproved oracle should fail
    client.set_milestone_price_condition(
        &setup.buyer,
        &shipment_id,
        &0,
        &oracle_addr,
        &10_000_000,
    ); // Should panic
}

// ============================================================
// TEST 8: COMBINED - INSURANCE HOLDBACK + ORACLE PRICE CONDITION
// ============================================================

#[test]
fn test_combined_insurance_and_price_condition() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    let token_client = token::StellarAssetClient::new(&setup.env, &setup.token_id);

    // Setup oracle
    let oracle_addr = setup.env.register(MockPriceOracle, ());
    let oracle_client = MockPriceOracleClient::new(&setup.env, &oracle_addr);
    client.approve_oracle(&setup.buyer, &oracle_addr);

    let shipment_id = String::from_str(&setup.env, "COMBINED-001");
    let total_amount: i128 = 10_000_000;
    let insurance_bps: u32 = 150; // 1.5%
    let min_price: i128 = 20_000_000; // 2.0 USD

    // Set good price
    oracle_client.set_price(&setup.token_id, &25_000_000); // 2.5 USD

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // Set both insurance and price condition on milestone 0
    client.set_milestone_price_condition(&setup.buyer, &shipment_id, &0, &oracle_addr, &min_price);

    let proof = String::from_str(&setup.env, "combined_proof");
    client.submit_proof(&shipment_id, &0, &proof);

    let supplier_balance_before = token_client.balance(&setup.supplier);

    // Confirm milestone - should pass price check
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    // Expected payout:
    // Gross: 2,500,000 (25% of 10,000,000)
    // Insurance holdback: 37,500 (1.5% of 2,500,000)
    // Net immediate: 2,462,500
    let supplier_balance_after = token_client.balance(&setup.supplier);
    assert_eq!(
        supplier_balance_after - supplier_balance_before,
        2_462_500
    );

    // Verify insurance holdback
    let holdback = client.get_milestone_insurance_holdback(&shipment_id, &0);
    assert_eq!(holdback, 37_500);

    // Release insurance on good delivery
    client.release_insurance_holdback(&setup.buyer, &shipment_id, &0);

    let supplier_final = token_client.balance(&setup.supplier);
    assert_eq!(supplier_final - supplier_balance_after, 37_500);
}

// ============================================================
// TEST 9: INSURANCE QUERY - VISIBLE ALONGSIDE ESCROW BALANCE
// ============================================================

#[test]
fn test_insurance_holdback_visible_in_queries() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = String::from_str(&setup.env, "QUERY-001");
    let total_amount: i128 = 8_000_000;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    // Confirm first milestone
    let proof = String::from_str(&setup.env, "proof1");
    client.submit_proof(&shipment_id, &0, &proof);
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    // Query insurance holdback for milestone 0
    let holdback_0 = client.get_milestone_insurance_holdback(&shipment_id, &0);
    assert!(holdback_0 > 0);

    // Query total insurance across all milestones
    let total_insurance = client.get_total_insurance_holdback(&shipment_id);
    assert_eq!(total_insurance, holdback_0);

    // Query should also show remaining escrow balance
    let escrow_balance = client.get_shipment_escrow_balance(&shipment_id);
    // Should be: total_amount - released_amount - total_insurance_held
    let expected_escrow = total_amount
        - (total_amount * 25 / 100) // 25% released for milestone 0
        - holdback_0;
    assert_eq!(escrow_balance, expected_escrow);
}

// ============================================================
// TEST 10: EDGE CASE - ZERO INSURANCE BPS
// ============================================================

#[test]
fn test_zero_insurance_bps_no_holdback() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    let token_client = token::StellarAssetClient::new(&setup.env, &setup.token_id);

    let shipment_id = String::from_str(&setup.env, "ZERO-INS-001");
    let total_amount: i128 = 1_000_000;
    let insurance_bps: u32 = 0; // No insurance

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );

    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);

    let supplier_balance_before = token_client.balance(&setup.supplier);
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);
    let supplier_balance_after = token_client.balance(&setup.supplier);

    // Full payment should be released (25% = 250,000)
    assert_eq!(supplier_balance_after - supplier_balance_before, 250_000);

    // No insurance holdback
    let holdback = client.get_milestone_insurance_holdback(&shipment_id, &0);
    assert_eq!(holdback, 0);
}
