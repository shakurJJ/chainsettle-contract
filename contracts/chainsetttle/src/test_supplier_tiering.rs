#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec};
use soroban_sdk::{token, vec, String, Symbol};

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

/// Creates and fully completes a single-milestone shipment for `supplier`, bumping
/// their `completed` reputation count by 1.
fn complete_one_shipment(
    client: &ChainSettleContractClient,
    t: &crate::test_common::TestSetup,
    id: &str,
) {
    let shipment_id = sid(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
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

fn standard_tier_config(_env: &Env) -> SupplierTierConfig {
    SupplierTierConfig {
        silver_min_completed: 3,
        silver_max_disputed_ratio_bps: 10_000,
        silver_multiplier_bps: 8_000,
        gold_min_completed: 5,
        gold_max_disputed_ratio_bps: 10_000,
        gold_multiplier_bps: 5_000,
        bronze_max_value: 0,
        silver_max_value: 0,
        gold_max_value: 0,
    }
}

// ============================================================
// TIER DERIVATION
// ============================================================

#[test]
fn test_new_supplier_defaults_to_bronze_with_no_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);
}

#[test]
fn test_new_supplier_defaults_to_bronze_with_config_set() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &standard_tier_config(&t.env));
    // No completed shipments yet.
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);
}

#[test]
fn test_tier_boundaries_bronze_silver_gold() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &standard_tier_config(&t.env));

    // 1 and 2 completed → still Bronze (threshold is 3).
    complete_one_shipment(&client, &t, "tier-1");
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);
    complete_one_shipment(&client, &t, "tier-2");
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);

    // 3rd completion crosses the Silver boundary.
    complete_one_shipment(&client, &t, "tier-3");
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);
    complete_one_shipment(&client, &t, "tier-4");
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);

    // 5th completion crosses the Gold boundary.
    complete_one_shipment(&client, &t, "tier-5");
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Gold);
}

#[test]
fn test_disputed_ratio_blocks_tier_upgrade() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mut config = standard_tier_config(&t.env);
    // Silver requires a disputed ratio of 0 (i.e. zero tolerance).
    config.silver_max_disputed_ratio_bps = 0;
    client.set_supplier_tier_config(&t.buyer, &config);

    for i in 0..3 {
        complete_one_shipment(&client, &t, &std::format!("tier-dispute-{}", i));
    }
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);

    // Raise and reject a dispute against this supplier — bumps `disputed` count.
    let shipment_id = sid(&t.env, "tier-dispute-open");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
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
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false, &None);

    // Ratio is now > 0, so Silver's zero-tolerance threshold is no longer met.
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);
}

// ============================================================
// COLLATERAL SCALING
// ============================================================

#[test]
fn test_bronze_collateral_unchanged_from_base() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &standard_tier_config(&t.env));

    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000_000);
    let token_client = token::Client::new(&t.env, &t.token_id);
    let before = token_client.balance(&t.supplier);

    let shipment_id = sid(&t.env, "collat-bronze");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &10_000_000i128,
        &single_milestone(&t.env),
        &opts,
    );

    let after = token_client.balance(&t.supplier);
    assert_eq!(before - after, 1_000_000, "Bronze supplier pays the full base collateral");
}

#[test]
fn test_silver_and_gold_collateral_scaled_down() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &standard_tier_config(&t.env));

    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000_000);
    let token_client = token::Client::new(&t.env, &t.token_id);

    // Build the supplier up to Silver (3 completed).
    for i in 0..3 {
        complete_one_shipment(&client, &t, &std::format!("collat-build-{}", i));
    }
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);

    let before = token_client.balance(&t.supplier);
    let shipment_id = sid(&t.env, "collat-silver");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &10_000_000i128,
        &single_milestone(&t.env),
        &opts,
    );
    let after = token_client.balance(&t.supplier);
    assert_eq!(before - after, 800_000, "Silver supplier pays 80% of base collateral");

    // Build up to Gold (5 completed total).
    for i in 3..5 {
        complete_one_shipment(&client, &t, &std::format!("collat-build-{}", i));
    }
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Gold);

    let before_gold = token_client.balance(&t.supplier);
    let shipment_id_gold = sid(&t.env, "collat-gold");
    client.create_shipment(
        &shipment_id_gold,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &10_000_000i128,
        &single_milestone(&t.env),
        &opts,
    );
    let after_gold = token_client.balance(&t.supplier);
    assert_eq!(before_gold - after_gold, 500_000, "Gold supplier pays 50% of base collateral");
}

#[test]
fn test_zero_collateral_requirement_unaffected_by_tier() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &standard_tier_config(&t.env));

    let token_client = token::Client::new(&t.env, &t.token_id);
    let before = token_client.balance(&t.supplier);

    let shipment_id = sid(&t.env, "collat-zero");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &10_000_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );

    // No collateral requested, so tiering has nothing to scale — supplier balance untouched.
    assert_eq!(token_client.balance(&t.supplier), before);
}

#[test]
#[should_panic(expected = "tier multiplier cannot exceed 10000 bps")]
fn test_multiplier_over_10000_bps_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mut config = standard_tier_config(&t.env);
    config.gold_multiplier_bps = 10_001;
    client.set_supplier_tier_config(&t.buyer, &config);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_only_admin_can_set_tier_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.supplier, &standard_tier_config(&t.env));
}
