#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    vec, Address, Env, String, Vec,
};

#[test]
fn test_set_get_clear_default_options() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Initial state: no default options
    assert_eq!(client.get_default_options(&t.buyer), None);

    let mut profile = default_options(&t.env);
    profile.holdback_ledgers = 200;
    profile.logistics_fee_bps = 150;
    profile.dispute_bond_amount = 5_000;
    profile.late_penalty_bps_per_ledger = 30;

    // Set default options
    client.set_default_options(&t.buyer, &profile);

    // Fetch and verify
    let saved = client.get_default_options(&t.buyer);
    assert!(saved.is_some());
    let saved_opts = saved.unwrap();
    assert_eq!(saved_opts.holdback_ledgers, 200);
    assert_eq!(saved_opts.logistics_fee_bps, 150);
    assert_eq!(saved_opts.dispute_bond_amount, 5_000);
    assert_eq!(saved_opts.late_penalty_bps_per_ledger, 30);

    // Clear default options
    client.clear_default_options(&t.buyer);
    assert_eq!(client.get_default_options(&t.buyer), None);
}

#[test]
fn test_create_shipment_with_defaults_uses_saved_profile() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.holdback_ledgers = 150;
    profile.late_penalty_bps_per_ledger = 25;
    profile.logistics_fee_bps = 200;
    profile.dispute_bond_amount = 8_000;
    profile.supplier_collateral = 40_000;

    // Save profile for buyer
    client.set_default_options(&t.buyer, &profile);

    // Mint token balance for supplier collateral
    let token_client = soroban_sdk::token::StellarAssetClient::new(&t.env, &t.token_id);
    token_client.mint(&t.supplier, &100_000);

    let shipment_id = String::from_str(&t.env, "SHIP-DEF-001");
    let milestones = build_milestones(&t.env);

    client.create_shipment_with_defaults(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &milestones,
    );

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.holdback_ledgers, 150);
    assert_eq!(shipment.late_penalty_bps_per_ledger, 25);
    assert_eq!(shipment.logistics_fee_bps, 200);
    assert_eq!(shipment.dispute_bond_amount, 8_000);
    assert_eq!(client.get_supplier_collateral(&shipment_id), 40_000);
}

#[test]
fn test_create_shipment_with_defaults_falls_back_to_contract_defaults() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-CONTRACT-DEF");
    let milestones = build_milestones(&t.env);

    // No profile saved -> contract defaults (holdback = 0, logistics_fee_bps = 0, etc.)
    client.create_shipment_with_defaults(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &milestones,
    );

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.holdback_ledgers, 0);
    assert_eq!(shipment.late_penalty_bps_per_ledger, 0);
    assert_eq!(shipment.logistics_fee_bps, 0);
    assert_eq!(shipment.dispute_bond_amount, 0);
    assert_eq!(client.get_supplier_collateral(&shipment_id), 0);
}

#[test]
fn test_create_shipment_with_defaults_falls_back_after_clear() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.holdback_ledgers = 500;
    client.set_default_options(&t.buyer, &profile);

    // Clear profile
    client.clear_default_options(&t.buyer);

    let shipment_id = String::from_str(&t.env, "SHIP-AFTER-CLEAR");
    let milestones = build_milestones(&t.env);

    client.create_shipment_with_defaults(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &milestones,
    );

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.holdback_ledgers, 0);
}

#[test]
#[should_panic(expected = "dispute bond cannot be negative")]
fn test_set_default_options_negative_dispute_bond_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.dispute_bond_amount = -1;

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "collateral cannot be negative")]
fn test_set_default_options_negative_collateral_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.supplier_collateral = -1;

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "buyer_cancel_fee_bps cannot exceed 1000 (10%)")]
fn test_set_default_options_excessive_cancel_fee_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.buyer_cancel_fee_bps = 1001;

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "dispute_bond_bps exceeds maximum allowed")]
fn test_set_default_options_excessive_dispute_bond_bps_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.dispute_bond_bps = 5001; // exceeds default 5000 bps

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "retainage_bps exceeds maximum allowed")]
fn test_set_default_options_excessive_retainage_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.retainage_bps = 2001; // max 2000

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "warranty_bps exceeds maximum allowed")]
fn test_set_default_options_excessive_warranty_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.warranty_bps = 2001; // max 2000

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "warranty_ledgers must be greater than zero when warranty_bps is set")]
fn test_set_default_options_warranty_without_ledgers_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    profile.warranty_bps = 500;
    profile.warranty_ledgers = 0;

    client.set_default_options(&t.buyer, &profile);
}

#[test]
#[should_panic(expected = "too many quality grades")]
fn test_set_default_options_too_many_grades_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut profile = default_options(&t.env);
    let mut grades = Vec::new(&t.env);
    for _ in 0..11 {
        grades.push_back(1000u32);
    }
    profile.quality_grades = grades;

    client.set_default_options(&t.buyer, &profile);
}
