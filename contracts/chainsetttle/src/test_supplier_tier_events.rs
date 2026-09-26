#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::testutils::Events;
use soroban_sdk::{token, vec, String, Symbol, TryFromVal};

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
        },
    ]
}

/// Silver after 1 completion (zero dispute tolerance), Gold out of reach.
fn silver_at_one_config() -> SupplierTierConfig {
    SupplierTierConfig {
        silver_min_completed: 1,
        silver_max_disputed_ratio_bps: 0,
        silver_multiplier_bps: 8_000,
        gold_min_completed: 100,
        gold_max_disputed_ratio_bps: 0,
        gold_multiplier_bps: 5_000,
    }
}

fn create_and_submit(client: &ChainSettleContractClient, t: &TestSetup, id: &str) -> String {
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
    shipment_id
}

/// `supplier_tier_changed` payloads emitted by the most recent top-level call.
fn tier_change_events(env: &Env) -> std::vec::Vec<(Address, SupplierTier, SupplierTier)> {
    let mut out = std::vec::Vec::new();
    let events = env.events().all();
    for i in 0..events.len() {
        let (_id, topics, data) = events.get(i).unwrap();
        let topic: Symbol = match Symbol::try_from_val(env, &topics.get(0).unwrap()) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if topic == Symbol::new(env, "supplier_tier_changed") {
            out.push(<(Address, SupplierTier, SupplierTier)>::try_from_val(env, &data).unwrap());
        }
    }
    out
}

#[test]
fn test_tier_upgrade_emits_event() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &silver_at_one_config());

    let shipment_id = create_and_submit(&client, &t, "up-1");
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let events = tier_change_events(&t.env);
    assert_eq!(
        events,
        std::vec![(
            t.supplier.clone(),
            SupplierTier::Bronze,
            SupplierTier::Silver
        )]
    );
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);
}

#[test]
fn test_tier_downgrade_emits_event() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &silver_at_one_config());

    let first = create_and_submit(&client, &t, "down-1");
    client.confirm_milestone(&t.buyer, &first, &0);
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);

    // A rejected dispute bumps `disputed`, breaking Silver's zero tolerance.
    let second = create_and_submit(&client, &t, "down-2");
    client.raise_dispute(&t.buyer, &second, &0);
    let mut events = tier_change_events(&t.env);
    client.resolve_dispute(&t.arbiter, &second, &0, &false, &None);
    events.extend(tier_change_events(&t.env));

    assert_eq!(
        events,
        std::vec![(
            t.supplier.clone(),
            SupplierTier::Silver,
            SupplierTier::Bronze
        )]
    );
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Bronze);
}

#[test]
fn test_tier_change_detected_at_shipment_creation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &silver_at_one_config());
    let first = create_and_submit(&client, &t, "cfg-1");
    client.confirm_milestone(&t.buyer, &first, &0);

    // Tightening the config changes the supplier's computed tier; it is
    // picked up the next time creation evaluates the tier (collateral discount).
    let mut stricter = silver_at_one_config();
    stricter.silver_min_completed = 10;
    client.set_supplier_tier_config(&t.buyer, &stricter);

    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &10_000_000);
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 1_000_000;
    client.create_shipment(
        &sid(&t.env, "cfg-2"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &single_milestone(&t.env),
        &opts,
    );

    assert_eq!(
        tier_change_events(&t.env),
        std::vec![(
            t.supplier.clone(),
            SupplierTier::Silver,
            SupplierTier::Bronze
        )]
    );
}

#[test]
fn test_no_event_when_tier_unchanged() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_supplier_tier_config(&t.buyer, &silver_at_one_config());

    let first = create_and_submit(&client, &t, "same-1");
    client.confirm_milestone(&t.buyer, &first, &0);
    assert_eq!(tier_change_events(&t.env).len(), 1);

    // Second completion re-evaluates the tier (still Silver) — no event.
    let second = create_and_submit(&client, &t, "same-2");
    client.confirm_milestone(&t.buyer, &second, &0);
    assert!(tier_change_events(&t.env).is_empty());
    assert_eq!(client.get_supplier_tier(&t.supplier), SupplierTier::Silver);
}

#[test]
fn test_no_event_without_tier_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = create_and_submit(&client, &t, "nocfg-1");
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
    assert!(tier_change_events(&t.env).is_empty());
}
