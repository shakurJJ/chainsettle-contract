#![cfg(test)]

//! #560 – Tier-based maximum shipment value for suppliers.

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec};
use soroban_sdk::{vec, String, Symbol};

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn single_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
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
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
        },
    ]
}

fn tier_config_with_caps(bronze: i128, silver: i128, gold: i128) -> SupplierTierConfig {
    SupplierTierConfig {
        silver_min_completed: 2,
        silver_max_disputed_ratio_bps: 10_000,
        silver_multiplier_bps: 8_000,
        gold_min_completed: 4,
        gold_max_disputed_ratio_bps: 10_000,
        gold_multiplier_bps: 5_000,
        bronze_max_value: bronze,
        silver_max_value: silver,
        gold_max_value: gold,
    }
}

fn complete_one(client: &ChainSettleContractClient, t: &crate::test_common::TestSetup, id: &str) {
    let shipment_id = sid(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://x"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "SupplierTierMaxValueExceeded")]
fn bronze_over_cap_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &tier_config_with_caps(500_000, 2_000_000, 0));

    client.create_shipment(
        &sid(&t.env, "BRONZE-OVER"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &500_001i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn bronze_at_cap_ok() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &tier_config_with_caps(500_000, 2_000_000, 0));

    client.create_shipment(
        &sid(&t.env, "BRONZE-OK"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &500_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn upgrading_tier_raises_allowed_value() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // Bronze capped at 200k; Silver at 2M.
    client.set_supplier_tier_config(&t.buyer, &tier_config_with_caps(200_000, 2_000_000, 0));

    // Reach Silver (2 completions).
    complete_one(&client, &t, "tier-up-1");
    complete_one(&client, &t, "tier-up-2");
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);

    client.create_shipment(
        &sid(&t.env, "SILVER-OK"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_500_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn cap_of_zero_means_unlimited() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &tier_config_with_caps(0, 0, 0));

    client.create_shipment(
        &sid(&t.env, "UNLIMITED"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &9_000_000_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
}

#[test]
#[should_panic(expected = "total amount exceeds maximum shipment value")]
fn works_with_global_max_value() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // Tier allows anything; global max is 1_000_000.
    client.set_supplier_tier_config(&t.buyer, &tier_config_with_caps(0, 0, 0));
    client.set_max_shipment_value(&t.buyer, &1_000_000i128);

    client.create_shipment(
        &sid(&t.env, "GLOBAL-MAX"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_001i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
}
