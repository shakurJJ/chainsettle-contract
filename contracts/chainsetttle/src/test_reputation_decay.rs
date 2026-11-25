#![cfg(test)]

//! #790 – Time-decayed supplier reputation weighting.

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{testutils::Ledger as _, vec, String, Symbol};

fn single_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Delivery"),
            token: None,
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
            logistics: None,
        },
    ]
}

fn complete_one_shipment(client: &ChainSettleContractClient, t: &TestSetup, id: &str) {
    let shipment_id = String::from_str(&t.env, id);
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

fn tier_config() -> SupplierTierConfig {
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

fn advance(t: &TestSetup, ledgers: u32) {
    let seq = t.env.ledger().sequence();
    t.env.ledger().set_sequence_number(seq + ledgers);
}

#[test]
fn decay_disabled_by_default() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(client.get_reputation_half_life(), 0);

    complete_one_shipment(&client, &t, "DECAY-0");
    advance(&t, 10_000);
    assert_eq!(client.get_reputation(&t.supplier).completed, 1);
}

#[test]
fn old_outcomes_contribute_less_over_time() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_reputation_half_life(&t.buyer, &1_000);

    for i in 0..4 {
        let id = std::format!("DECAY-A-{}", i);
        complete_one_shipment(&client, &t, &id);
    }
    assert_eq!(client.get_reputation(&t.supplier).completed, 4);

    // One half-life later the score has halved; lifetime counters are untouched.
    advance(&t, 1_000);
    assert_eq!(client.get_reputation(&t.supplier).completed, 2);
    assert_eq!(client.get_lifetime_reputation(&t.supplier).completed, 4);

    // A fresh outcome counts in full on top of the decayed history.
    complete_one_shipment(&client, &t, "DECAY-A-NEW");
    assert_eq!(client.get_reputation(&t.supplier).completed, 3);

    // Two more half-lives: (2 + 1) / 4 rounds to 1.
    advance(&t, 2_000);
    assert_eq!(client.get_reputation(&t.supplier).completed, 1);
    assert_eq!(client.get_lifetime_reputation(&t.supplier).completed, 5);
}

#[test]
fn tier_boundaries_follow_decayed_score() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &tier_config());
    client.set_reputation_half_life(&t.buyer, &1_000);

    for i in 0..5 {
        let id = std::format!("DECAY-T-{}", i);
        complete_one_shipment(&client, &t, &id);
    }
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Gold);

    // 5 -> 3.75 (rounds to 4) after half a half-life: Silver.
    advance(&t, 500);
    assert_eq!(client.get_reputation(&t.supplier).completed, 4);
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);

    // Long inactivity drops the supplier back to Bronze.
    advance(&t, 10_000);
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);

    // Disabling decay restores the lifetime-based tier.
    client.set_reputation_half_life(&t.buyer, &0);
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Gold);
}
