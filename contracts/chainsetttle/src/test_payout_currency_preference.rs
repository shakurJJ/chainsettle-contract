#![cfg(test)]
// #404 — Supplier payout currency preference, auto-convert at claim time.
//
// Since this contract has no swap-venue/price-oracle integration, conversion
// routes are modeled as admin-registered fixed rates (set_conversion_rate),
// standing in for a configured swap venue/oracle. claim_payout converts a
// batched payout into the supplier's preferred token only when both a route
// is registered AND the contract already holds enough of the preferred
// token to pay it out; otherwise it falls back to the original settlement
// token without reverting.

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{token, String, Symbol, TryFromVal};

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn ipfs(env: &Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

fn register_and_fund(env: &Env, to: &Address, amount: i128) -> Address {
    let token_admin = Address::generate(env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    token::StellarAssetClient::new(env, &token_id).mint(to, &amount);
    token_id
}

/// Creates a batched-payout shipment, submits proof and confirms the first
/// milestone so its payment accrues into the supplier's pending balance.
fn accrue_batched_payout(
    t: &crate::test_common::TestSetup,
    client: &ChainSettleContractClient,
    shipment_id: &String,
    token: &Address,
) {
    client.create_shipment(
        shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        token,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.set_payout_mode(&t.supplier, &true);
    client.submit_proof(
        &t.supplier,
        shipment_id,
        &0u32,
        &sid(&t.env, "QmProof"),
        &ipfs(&t.env),
    );
    client.confirm_milestone(&t.buyer, shipment_id, &0u32);
}

#[test]
fn test_claim_payout_converts_to_preferred_token_when_route_and_liquidity_exist() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let source_token = t.token_id.clone();
    let preferred_token = register_and_fund(&t.env, &t.buyer, 10_000_000_000);
    // Fund the contract itself with the preferred token so it can pay out
    // the converted amount.
    token::StellarAssetClient::new(&t.env, &preferred_token)
        .mint(&t.contract_id, &10_000_000_000i128);

    client.set_conversion_rate(&t.buyer, &source_token, &preferred_token, &20_000u32); // 2:1
    client.set_payout_currency_preference(&t.supplier, &preferred_token);

    let shipment_id = sid(&t.env, "s1");
    accrue_batched_payout(&t, &client, &shipment_id, &source_token);

    let pending = client.get_pending_payout(&t.supplier);
    assert_eq!(pending, 250_000);

    let preferred_client = token::Client::new(&t.env, &preferred_token);
    let balance_before = preferred_client.balance(&t.supplier);

    client.claim_payout(&t.supplier, &source_token);

    let balance_after = preferred_client.balance(&t.supplier);
    assert_eq!(balance_after - balance_before, pending * 2);
    assert_eq!(client.get_pending_payout(&t.supplier), 0);
}

#[test]
fn test_claim_payout_falls_back_to_source_token_when_no_route_configured() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let source_token = t.token_id.clone();
    let preferred_token = register_and_fund(&t.env, &t.buyer, 10_000_000_000);
    // No conversion rate registered for (source_token, preferred_token).
    client.set_payout_currency_preference(&t.supplier, &preferred_token);

    let shipment_id = sid(&t.env, "s1");
    accrue_batched_payout(&t, &client, &shipment_id, &source_token);
    let pending = client.get_pending_payout(&t.supplier);

    let source_client = token::Client::new(&t.env, &source_token);
    let balance_before = source_client.balance(&t.supplier);

    client.claim_payout(&t.supplier, &source_token);

    let balance_after = source_client.balance(&t.supplier);
    assert_eq!(balance_after - balance_before, pending);
    assert_eq!(client.get_pending_payout(&t.supplier), 0);
}

#[test]
fn test_claim_payout_falls_back_when_route_exists_but_contract_lacks_liquidity() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let source_token = t.token_id.clone();
    // Preferred token registered and a rate configured, but the contract is
    // never funded with it — insufficient liquidity to pay out the converted
    // amount.
    let preferred_token = register_and_fund(&t.env, &t.buyer, 10_000_000_000);
    client.set_conversion_rate(&t.buyer, &source_token, &preferred_token, &10_000u32);
    client.set_payout_currency_preference(&t.supplier, &preferred_token);

    let shipment_id = sid(&t.env, "s1");
    accrue_batched_payout(&t, &client, &shipment_id, &source_token);
    let pending = client.get_pending_payout(&t.supplier);

    let source_client = token::Client::new(&t.env, &source_token);
    let balance_before = source_client.balance(&t.supplier);

    client.claim_payout(&t.supplier, &source_token);

    let balance_after = source_client.balance(&t.supplier);
    assert_eq!(balance_after - balance_before, pending);
}

#[test]
fn test_claim_payout_event_records_conversion_details() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let source_token = t.token_id.clone();
    let preferred_token = register_and_fund(&t.env, &t.buyer, 10_000_000_000);
    token::StellarAssetClient::new(&t.env, &preferred_token)
        .mint(&t.contract_id, &10_000_000_000i128);

    client.set_conversion_rate(&t.buyer, &source_token, &preferred_token, &15_000u32); // 1.5:1
    client.set_payout_currency_preference(&t.supplier, &preferred_token);

    let shipment_id = sid(&t.env, "s1");
    accrue_batched_payout(&t, &client, &shipment_id, &source_token);

    client.claim_payout(&t.supplier, &source_token);

    let events = t.env.events().all();
    let mut found = false;
    for i in 0..events.len() {
        let (_id, topics, data) = events.get(i).unwrap();
        let topic: Symbol = Symbol::try_from_val(&t.env, &topics.get(0).unwrap()).unwrap();
        if topic == Symbol::new(&t.env, "payout_claimed") {
            let (amount, token_paid, rate_bps): (i128, Address, i128) =
                <(i128, Address, i128)>::try_from_val(&t.env, &data).unwrap();
            assert_eq!(token_paid, preferred_token);
            assert_eq!(rate_bps, 15_000);
            assert!(amount > 0);
            found = true;
        }
    }
    assert!(found, "expected payout_claimed event to fire");
}

#[test]
fn test_get_conversion_rate_reflects_configuration() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let token_a = t.token_id.clone();
    let token_b = register_and_fund(&t.env, &t.buyer, 1);

    assert!(client.get_conversion_rate(&token_a, &token_b).is_none());
    client.set_conversion_rate(&t.buyer, &token_a, &token_b, &12_500u32);
    assert_eq!(client.get_conversion_rate(&token_a, &token_b), Some(12_500u32));

    client.clear_conversion_rate(&t.buyer, &token_a, &token_b);
    assert!(client.get_conversion_rate(&token_a, &token_b).is_none());
}
