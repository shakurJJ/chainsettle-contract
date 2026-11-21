// #386 — Escrow release schedule preview: simulate payout waterfall before confirmation.
//
// Verifies `preview_milestone_payout` reports exactly what `confirm_milestone`
// would pay out — gross amount, fees, and net supplier amount — without
// mutating any state or requiring the milestone to already be confirmed.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    vec, String,
};

#[test]
fn test_preview_matches_gross_with_no_fees_configured() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "PREV-001");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    // Milestone 0 is 25% of 1_000_000 = 250_000.
    let preview = client.preview_milestone_payout(&shipment_id, &0u32);
    assert_eq!(preview.gross_amount, 250_000);
    assert_eq!(preview.platform_fee, 0);
    assert_eq!(preview.logistics_fee, 0);
    assert_eq!(preview.advance_deducted, 0);
    assert_eq!(preview.late_penalty_deducted, 0);
    assert_eq!(preview.supplier_net_amount, 250_000);
    assert!(!preview.would_be_held);
    assert!(!preview.is_final_milestone);
}

#[test]
fn test_preview_does_not_mutate_state() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "PREV-002");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    let before = client.get_shipment(&shipment_id);
    let _ = client.preview_milestone_payout(&shipment_id, &0u32);
    let _ = client.preview_milestone_payout(&shipment_id, &1u32);
    let after = client.get_shipment(&shipment_id);

    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(
        before.milestones.get(0).unwrap().status,
        after.milestones.get(0).unwrap().status
    );
}

#[test]
fn test_preview_reflects_logistics_fee() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "PREV-003");
    let mut opts = default_options(&t.env);
    opts.logistics_fee_bps = 500; // 5%

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &opts,
    );

    let preview = client.preview_milestone_payout(&shipment_id, &0u32);
    // 250_000 gross, 5% logistics fee = 12_500.
    assert_eq!(preview.gross_amount, 250_000);
    assert_eq!(preview.logistics_fee, 12_500);
    assert_eq!(preview.supplier_net_amount, 237_500);
}

#[test]
fn test_preview_reflects_platform_fee_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &1000u32, &t.treasury); // 10%

    let shipment_id = String::from_str(&t.env, "PREV-004");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    let preview = client.preview_milestone_payout(&shipment_id, &0u32);
    assert_eq!(preview.gross_amount, 250_000);
    assert_eq!(preview.applied_fee_bps, 1000);
    assert_eq!(preview.platform_fee, 25_000);
    assert_eq!(preview.supplier_net_amount, 225_000);
}

#[test]
fn test_preview_matches_combined_fee_waterfall_example() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &1000u32, &t.treasury);
    client.set_fee_tiers(
        &t.buyer,
        &vec![
            &t.env,
            FeeTier {
                min_lifetime_volume: 0,
                fee_bps: 800,
            },
        ],
    );
    client.initialize_multisig_admin(&t.buyer, &vec![&t.env, t.buyer.clone()], &1u32);
    client.propose_fee_waiver(&t.buyer, &t.buyer, &2500u32, &0u64);

    let shipment_id = String::from_str(&t.env, "PREV-FEE-WATERFALL");
    let mut milestones = build_milestones(&t.env);
    let mut first_milestone = milestones.get(0).unwrap();
    first_milestone.deadline_ledger = 100;
    first_milestone.penalty_bps_per_ledger = 100;
    milestones.set(0, first_milestone);

    let mut options = default_options(&t.env);
    options.logistics_fee_bps = 500;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &milestones,
        &options,
    );
    client.set_shipment_fee_override(&t.buyer, &shipment_id, &700u32);
    client.request_advance(&t.supplier, &shipment_id, &0u32, &4u32);
    client.approve_advance(&t.buyer, &shipment_id, &0u32);

    t.env.ledger().set_sequence_number(102);
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0u32,
        &String::from_str(&t.env, "proof"),
        &soroban_sdk::Symbol::new(&t.env, "ipfs"),
    );

    let preview = client.preview_milestone_payout(&shipment_id, &0u32);
    assert_eq!(preview.gross_amount, 250_000);
    assert_eq!(preview.advance_deducted, 10_000);
    assert_eq!(preview.late_penalty_deducted, 5_000);
    assert_eq!(preview.applied_fee_bps, 525);
    assert_eq!(preview.platform_fee, 12_862);
    assert_eq!(preview.logistics_fee, 12_250);
    assert_eq!(preview.supplier_net_amount, 209_888);
    assert!(!preview.would_be_held);
    assert!(!preview.is_final_milestone);
}

#[test]
fn test_preview_marks_final_milestone_correctly() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "PREV-005");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    // Confirm the first two milestones for real.
    for i in 0..2u32 {
        client.submit_proof(
            &t.supplier,
            &shipment_id,
            &i,
            &String::from_str(&t.env, "proof"),
            &soroban_sdk::Symbol::new(&t.env, "ipfs"),
        );
        client.confirm_milestone(&t.buyer, &shipment_id, &i);
    }

    // Previewing the last remaining milestone should report is_final_milestone.
    let preview = client.preview_milestone_payout(&shipment_id, &2u32);
    assert!(preview.is_final_milestone);
}

#[test]
fn test_preview_reports_held_when_holdback_configured() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "PREV-006");
    let mut opts = default_options(&t.env);
    opts.holdback_ledgers = 1000;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &opts,
    );

    let preview = client.preview_milestone_payout(&shipment_id, &0u32);
    assert!(preview.would_be_held);
    assert_eq!(preview.platform_fee, 0);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_preview_rejects_invalid_milestone_index() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "PREV-007");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.preview_milestone_payout(&shipment_id, &99u32);
}
