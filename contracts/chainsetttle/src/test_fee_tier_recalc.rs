#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec};
use soroban_sdk::{token, vec, String, Symbol};

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn two_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "First"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "Second"),
            payment_percent: 50,
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

fn submit_and_confirm(
    client: &ChainSettleContractClient,
    t: &crate::test_common::TestSetup,
    shipment_id: &String,
    idx: u32,
) {
    client.submit_proof(
        &t.supplier,
        shipment_id,
        &idx,
        &String::from_str(&t.env, "ipfs://x"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, shipment_id, &idx);
}

#[test]
fn test_final_milestone_uses_tier_at_completion_not_creation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Base fee 10%; a volume tier drops it to 2% once lifetime volume >= 5_000_000.
    client.set_fee_config(&t.buyer, &1000u32, &t.treasury);
    client.set_fee_tiers(
        &t.buyer,
        &vec![
            &t.env,
            FeeTier {
                min_lifetime_volume: 5_000_000,
                fee_bps: 200,
            },
        ],
    );

    let token_client = token::Client::new(&t.env, &t.token_id);

    // Shipment A: two milestones, created while the buyer has zero lifetime volume,
    // so the 10% base tier is locked in at creation.
    let shipment_a = sid(&t.env, "fee-recalc-a");
    client.create_shipment(
        &shipment_a,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &2_000_000i128,
        &two_milestones(&t.env),
        &default_options(&t.env),
    );

    // Confirm the first (non-final) milestone — charged at the locked-in 10%.
    let treasury_before_m0 = token_client.balance(&t.treasury);
    submit_and_confirm(&client, &t, &shipment_a, 0);
    assert_eq!(
        token_client.balance(&t.treasury) - treasury_before_m0,
        100_000,
        "first milestone charged the 10% tier locked in at creation"
    );

    // Complete an unrelated shipment for the same buyer to push their lifetime volume
    // (1_000_000 already recorded + this) past the 5_000_000 tier threshold.
    let shipment_b = sid(&t.env, "fee-recalc-b");
    client.create_shipment(
        &shipment_b,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &4_500_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
    submit_and_confirm(&client, &t, &shipment_b, 0);

    // Now confirm the *final* milestone of shipment A: the buyer's lifetime volume
    // (1_000_000 + 4_500_000 = 5_500_000) now qualifies for the 2% tier, so this
    // payment should be charged at 2%, not the 10% locked in at creation.
    let treasury_before_final = token_client.balance(&t.treasury);
    submit_and_confirm(&client, &t, &shipment_a, 1);
    assert_eq!(
        token_client.balance(&t.treasury) - treasury_before_final,
        20_000,
        "final milestone recalculates the buyer's fee tier as of completion time"
    );

    let shipment = client.get_shipment(&shipment_a);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
}

#[test]
fn test_shipment_fee_override_still_takes_precedence_at_completion() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &1000u32, &t.treasury);
    client.set_fee_tiers(
        &t.buyer,
        &vec![
            &t.env,
            FeeTier {
                min_lifetime_volume: 0,
                fee_bps: 200,
            },
        ],
    );

    let shipment_id = sid(&t.env, "fee-override-final");
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

    // Override to 5% — should win over both the locked tier and any recalculated tier.
    client.set_shipment_fee_override(&t.buyer, &shipment_id, &500u32);

    let token_client = token::Client::new(&t.env, &t.token_id);
    let treasury_before = token_client.balance(&t.treasury);
    submit_and_confirm(&client, &t, &shipment_id, 0);

    assert_eq!(
        token_client.balance(&t.treasury) - treasury_before,
        50_000,
        "shipment-level override (5%) takes precedence over the recalculated tier"
    );
}

#[test]
fn test_single_milestone_shipment_is_always_final() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &1000u32, &t.treasury);
    // Tier available from the very first ledger of lifetime volume.
    client.set_fee_tiers(
        &t.buyer,
        &vec![
            &t.env,
            FeeTier {
                min_lifetime_volume: 0,
                fee_bps: 300,
            },
        ],
    );

    let shipment_id = sid(&t.env, "fee-single-final");
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

    let token_client = token::Client::new(&t.env, &t.token_id);
    let treasury_before = token_client.balance(&t.treasury);
    submit_and_confirm(&client, &t, &shipment_id, 0);
    assert_eq!(token_client.balance(&t.treasury) - treasury_before, 30_000);
}
