// Tests for `estimate_shipment_costs` query.
//
// Verifies `estimate_shipment_costs` accurately simulates shipment-level
// fees, payouts, dispute bond, and supplier collateral before creation,
// and respects active waivers, fee holidays, tiers, and custom splits.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    vec, Address, Env, String, Vec,
};

#[test]
fn test_estimate_basic_no_fees() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let milestones = build_milestones(&t.env);
    let opts = default_options(&t.env);

    let est = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );

    assert_eq!(est.total_amount, 1_000_000);
    assert_eq!(est.total_platform_fee, 0);
    assert_eq!(est.total_logistics_fee, 0);
    assert_eq!(est.total_net_amount, 1_000_000);
    assert_eq!(est.collateral_required, 0);
    assert_eq!(est.dispute_bond, 0);
    assert_eq!(est.applied_fee_bps, 0);
    assert_eq!(est.milestones.len(), 3);

    // Milestone 0 (25%): 250_000
    let m0 = est.milestones.get(0).unwrap();
    assert_eq!(m0.milestone_index, 0);
    assert_eq!(m0.gross_amount, 250_000);
    assert_eq!(m0.platform_fee, 0);
    assert_eq!(m0.logistics_fee, 0);
    assert_eq!(m0.net_amount, 250_000);

    // Milestone 1 (50%): 500_000
    let m1 = est.milestones.get(1).unwrap();
    assert_eq!(m1.milestone_index, 1);
    assert_eq!(m1.gross_amount, 500_000);
    assert_eq!(m1.platform_fee, 0);
    assert_eq!(m1.logistics_fee, 0);
    assert_eq!(m1.net_amount, 500_000);

    // Milestone 2 (25%): 250_000
    let m2 = est.milestones.get(2).unwrap();
    assert_eq!(m2.milestone_index, 2);
    assert_eq!(m2.gross_amount, 250_000);
    assert_eq!(m2.platform_fee, 0);
    assert_eq!(m2.logistics_fee, 0);
    assert_eq!(m2.net_amount, 250_000);
}

#[test]
fn test_estimate_matches_actual_e2e_payouts() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // 1. Configure platform fee (200 bps = 2%)
    client.set_fee_config(&t.buyer, &200u32, &t.treasury);

    let mut opts = default_options(&t.env);
    opts.logistics_fee_bps = 300; // 3%
    opts.dispute_bond_amount = 5_000;
    opts.dispute_bond_bps = 100; // 1% of 1_000_000 = 10_000 -> per dispute bond = 15_000
    opts.supplier_collateral = 50_000;

    let milestones = build_milestones(&t.env);
    let total_amount = 1_000_000i128;

    // 2. Query estimate before shipment creation
    let est = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &total_amount,
        &milestones,
        &opts,
    );

    assert_eq!(est.total_amount, 1_000_000);
    assert_eq!(est.applied_fee_bps, 200);
    assert_eq!(est.collateral_required, 50_000);
    // 3 milestones * 15_000 = 45_000
    assert_eq!(est.dispute_bond, 45_000);

    // Milestone 0 (25% = 250_000):
    // Platform fee: 2% of 250_000 = 5_000
    // Logistics fee: 3% of 250_000 = 7_500
    // Net: 250_000 - 5_000 - 7_500 = 237_500
    let m0 = est.milestones.get(0).unwrap();
    assert_eq!(m0.gross_amount, 250_000);
    assert_eq!(m0.platform_fee, 5_000);
    assert_eq!(m0.logistics_fee, 7_500);
    assert_eq!(m0.net_amount, 237_500);

    // Milestone 1 (50% = 500_000):
    // Platform fee: 2% of 500_000 = 10_000
    // Logistics fee: 3% of 500_000 = 15_000
    // Net: 500_000 - 10_000 - 15_000 = 475_000
    let m1 = est.milestones.get(1).unwrap();
    assert_eq!(m1.gross_amount, 500_000);
    assert_eq!(m1.platform_fee, 10_000);
    assert_eq!(m1.logistics_fee, 15_000);
    assert_eq!(m1.net_amount, 475_000);

    // Milestone 2 (25% = 250_000):
    // Platform fee: 2% of 250_000 = 5_000
    // Logistics fee: 3% of 250_000 = 7_500
    // Net: 250_000 - 5_000 - 7_500 = 237_500
    let m2 = est.milestones.get(2).unwrap();
    assert_eq!(m2.gross_amount, 250_000);
    assert_eq!(m2.platform_fee, 5_000);
    assert_eq!(m2.logistics_fee, 7_500);
    assert_eq!(m2.net_amount, 237_500);

    assert_eq!(est.total_platform_fee, 20_000);
    assert_eq!(est.total_logistics_fee, 30_000);
    assert_eq!(est.total_net_amount, 950_000);

    // 3. Create actual shipment and compare preview & confirmation payouts
    let shipment_id = String::from_str(&t.env, "EST-E2E-001");
    // Mint token for supplier to lock collateral
    let token_admin = Address::generate(&t.env);
    let token_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    token_client.mint(&t.supplier, &100_000);

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &milestones,
        &opts,
    );

    let ship = client.get_shipment(&shipment_id);
    assert_eq!(ship.dispute_bond_amount, 15_000);
    assert_eq!(client.get_supplier_collateral(&shipment_id), 50_000);

    // Verify milestone previews match the estimate
    for idx in 0..3u32 {
        let prev = client.preview_milestone_payout(&shipment_id, &idx);
        let m_est = est.milestones.get(idx).unwrap();
        assert_eq!(prev.gross_amount, m_est.gross_amount);
        assert_eq!(prev.platform_fee, m_est.platform_fee);
        assert_eq!(prev.logistics_fee, m_est.logistics_fee);
        assert_eq!(prev.supplier_net_amount, m_est.net_amount);
    }
}

#[test]
fn test_estimate_reflects_fee_holiday() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // 200 bps platform fee
    client.set_fee_config(&t.buyer, &200u32, &t.treasury);

    let milestones = build_milestones(&t.env);
    let mut opts = default_options(&t.env);
    opts.logistics_fee_bps = 100;

    // Before holiday: 200 bps fee
    let est_before = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );
    assert_eq!(est_before.applied_fee_bps, 200);
    assert_eq!(est_before.total_platform_fee, 20_000);
    assert_eq!(est_before.total_logistics_fee, 10_000);
    assert_eq!(est_before.total_net_amount, 970_000);

    // Schedule fee holiday covering current ledger (sequence 0..100)
    let cur_ledger = t.env.ledger().sequence();
    client.schedule_fee_holiday(&t.buyer, &cur_ledger, &(cur_ledger + 50));

    assert!(client.is_fee_holiday_active());

    // During holiday: platform fee waived (0 bps)
    let est_holiday = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );
    assert_eq!(est_holiday.applied_fee_bps, 0);
    assert_eq!(est_holiday.total_platform_fee, 0);
    assert_eq!(est_holiday.total_logistics_fee, 10_000);
    assert_eq!(est_holiday.total_net_amount, 990_000);

    // Cancel holiday -> platform fee restored
    client.cancel_fee_holiday(&t.buyer);
    assert!(!client.is_fee_holiday_active());

    let est_after = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );
    assert_eq!(est_after.applied_fee_bps, 200);
    assert_eq!(est_after.total_platform_fee, 20_000);
}

#[test]
fn test_estimate_reflects_vip_fee_waiver() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Set up multisig admin for governance fee waiver
    let admins = vec![&t.env, t.buyer.clone()];
    client.init_multisig_admin(&t.buyer, &admins, &1u32);

    client.set_fee_config(&t.buyer, &200u32, &t.treasury);

    // Propose 50% waiver (5000 bps) for the buyer
    client.propose_fee_waiver(&t.buyer, &t.buyer, &5000u32, &0u64);
    assert_eq!(client.get_fee_waiver(&t.buyer), Some((5000u32, 0u64)));

    let milestones = build_milestones(&t.env);
    let opts = default_options(&t.env);

    // Fee becomes 200 - (200 * 50%) = 100 bps
    let est = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );

    assert_eq!(est.applied_fee_bps, 100);
    assert_eq!(est.total_platform_fee, 10_000);
    assert_eq!(est.total_net_amount, 990_000);
}

#[test]
fn test_estimate_reflects_buyer_lifetime_volume_tier() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &200u32, &t.treasury);

    // Set fee tiers:
    // Tier 0: min volume 0 -> 200 bps
    // Tier 1: min volume 500_000 -> 120 bps
    // Tier 2: min volume 2_000_000 -> 80 bps
    let tiers = vec![
        &t.env,
        FeeTier {
            min_lifetime_volume: 0,
            fee_bps: 200,
        },
        FeeTier {
            min_lifetime_volume: 500_000,
            fee_bps: 120,
        },
        FeeTier {
            min_lifetime_volume: 2_000_000,
            fee_bps: 80,
        },
    ];
    client.set_fee_tiers(&t.buyer, &tiers);

    let milestones = build_milestones(&t.env);
    let opts = default_options(&t.env);

    // Buyer with 0 volume gets 200 bps
    let est0 = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );
    assert_eq!(est0.applied_fee_bps, 200);
    assert_eq!(est0.total_platform_fee, 20_000);

    // Set buyer volume to 600_000 -> Tier 1 (120 bps)
    client.set_lifetime_volume(&t.buyer, &t.buyer, &600_000i128);

    let est1 = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );
    assert_eq!(est1.applied_fee_bps, 120);
    assert_eq!(est1.total_platform_fee, 12_000);
    assert_eq!(est1.total_net_amount, 988_000);
}

#[test]
fn test_estimate_reflects_custom_milestone_splits() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &100u32, &t.treasury);

    let mut opts = default_options(&t.env);
    // Custom 2-milestone splits: 3000 bps (30%) and 7000 bps (70%)
    opts.milestone_splits = vec![&t.env, 3000u32, 7000u32];

    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Phase 1"),
            payment_percent: 50, // Ignored because milestone_splits is set
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "Phase 2"),
            payment_percent: 50, // Ignored
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ];

    let est = client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &opts,
    );

    assert_eq!(est.milestones.len(), 2);

    let m0 = est.milestones.get(0).unwrap();
    assert_eq!(m0.gross_amount, 300_000);
    assert_eq!(m0.platform_fee, 3_000);
    assert_eq!(m0.net_amount, 297_000);

    let m1 = est.milestones.get(1).unwrap();
    assert_eq!(m1.gross_amount, 700_000);
    assert_eq!(m1.platform_fee, 7_000);
    assert_eq!(m1.net_amount, 693_000);

    assert_eq!(est.total_platform_fee, 10_000);
    assert_eq!(est.total_net_amount, 990_000);
}

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_estimate_zero_amount_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let milestones = build_milestones(&t.env);
    let opts = default_options(&t.env);

    client.estimate_shipment_costs(&t.buyer, &t.token_id, &0i128, &milestones, &opts);
}

#[test]
#[should_panic(expected = "milestones cannot be empty")]
fn test_estimate_empty_milestones_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let empty_milestones: Vec<Milestone> = Vec::new(&t.env);
    let opts = default_options(&t.env);

    client.estimate_shipment_costs(
        &t.buyer,
        &t.token_id,
        &1_000_000i128,
        &empty_milestones,
        &opts,
    );
}

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_estimate_invalid_percentage_sum_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mut milestones = build_milestones(&t.env);
    let mut m0 = milestones.get(0).unwrap();
    m0.payment_percent = 90; // sum becomes 90 + 50 + 25 = 165
    milestones.set(0, m0);
    let opts = default_options(&t.env);

    client.estimate_shipment_costs(&t.buyer, &t.token_id, &1_000_000i128, &milestones, &opts);
}

#[test]
#[should_panic(expected = "InvalidSplitConfiguration")]
fn test_estimate_invalid_splits_sum_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let milestones = build_milestones(&t.env);
    let mut opts = default_options(&t.env);
    opts.milestone_splits = vec![&t.env, 3000u32, 3000u32, 3000u32]; // sum = 9000 != 10000

    client.estimate_shipment_costs(&t.buyer, &t.token_id, &1_000_000i128, &milestones, &opts);
}

#[test]
#[should_panic(expected = "dispute_bond_bps exceeds maximum allowed")]
fn test_estimate_dispute_bond_bps_cap_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let milestones = build_milestones(&t.env);
    let mut opts = default_options(&t.env);
    opts.dispute_bond_bps = 5001; // exceeds default max 5000 bps

    client.estimate_shipment_costs(&t.buyer, &t.token_id, &1_000_000i128, &milestones, &opts);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_estimate_blacklisted_buyer_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.blacklist_address(&t.buyer, &t.buyer);

    let milestones = build_milestones(&t.env);
    let opts = default_options(&t.env);

    client.estimate_shipment_costs(&t.buyer, &t.token_id, &1_000_000i128, &milestones, &opts);
}

#[test]
#[should_panic(expected = "token is not in the approved whitelist")]
fn test_estimate_unapproved_token_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let other_token = Address::generate(&t.env);
    client.add_allowed_token(&t.buyer, &other_token);

    let milestones = build_milestones(&t.env);
    let opts = default_options(&t.env);

    // t.token_id is not other_token
    client.estimate_shipment_costs(&t.buyer, &t.token_id, &1_000_000i128, &milestones, &opts);
}
