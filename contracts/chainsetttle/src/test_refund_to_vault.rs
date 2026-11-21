#![cfg(test)]

//! #746: Route cancellation and dispute refunds into the buyer vault.

extern crate std;

use super::*;
use crate::test_common::{
    build_milestones, capture_events, default_options, setup, single_buyer_vec, TestSetup,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Map, String, Symbol, TryFromVal, Val,
};

const AMOUNT: i128 = 1_000_000;

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn vault_ext() -> ShipmentOptionsExt {
    ShipmentOptionsExt {
        refund_to_vault: true,
        ..ShipmentOptionsExt::default()
    }
}

fn create(t: &TestSetup, id: &str, opts: ShipmentOptions, ext: ShipmentOptionsExt) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.create_shipment_ext(
        &sid(&t.env, id),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &AMOUNT,
        &build_milestones(&t.env),
        &opts,
        &ext,
    )
}

fn submit_proof(t: &TestSetup, id: &String, idx: u32) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.submit_proof(
        &t.supplier,
        id,
        &idx,
        &String::from_str(&t.env, "QmXyz123"),
        &Symbol::new(&t.env, "ipfs"),
    );
}

/// Data map of the last `chainsettle/<name>` event in `events`.
fn find_event(
    env: &Env,
    events: &soroban_sdk::Vec<crate::test_common::CapturedEvent>,
    name: &str,
) -> Option<Map<Symbol, Val>> {
    let ns = Symbol::new(env, "chainsettle");
    let expected = Symbol::new(env, name);
    let mut found = None;
    for e in events.iter() {
        if e.1.len() != 2 {
            continue;
        }
        let t0 = Symbol::try_from_val(env, &e.1.get(0).unwrap());
        let t1 = Symbol::try_from_val(env, &e.1.get(1).unwrap());
        if let (Ok(t0), Ok(t1)) = (t0, t1) {
            if t0 == ns && t1 == expected {
                found = Some(Map::<Symbol, Val>::try_from_val(env, &e.2).unwrap());
            }
        }
    }
    found
}

fn to_vault(env: &Env, data: &Map<Symbol, Val>) -> bool {
    bool::try_from_val(env, &data.get(Symbol::new(env, "to_vault")).unwrap()).unwrap()
}

/// Every token the contract holds is accounted for by escrow + vault balances.
fn assert_reconciled(t: &TestSetup) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let held = token::Client::new(&t.env, &t.token_id).balance(&t.contract_id);
    let escrowed = client.get_total_escrowed_value(&t.token_id);
    let vaulted = client.get_vault_balance(&t.buyer, &t.token_id)
        + client.get_vault_balance(&t.buyer2, &t.token_id);
    assert_eq!(held, escrowed + vaulted);
}

// ── Cancellation paths ──────────────────────────────────────────────────────

#[test]
fn test_buyer_cancel_refund_credited_to_vault() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let id = create(&t, "V-CANCEL", default_options(&t.env), vault_ext());
    assert!(client.is_refund_to_vault(&id));
    let wallet_before = token.balance(&t.buyer);

    let events = capture_events(&t.env, || client.cancel_shipment(&t.buyer, &id));
    let data = find_event(&t.env, &events, "shipment_cancelled").expect("event");
    assert!(to_vault(&t.env, &data));

    assert_eq!(token.balance(&t.buyer), wallet_before, "no token transfer");
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), AMOUNT);
    assert_reconciled(&t);

    // The credited balance is withdrawable.
    client.vault_withdraw(&t.buyer, &t.token_id, &AMOUNT);
    assert_eq!(token.balance(&t.buyer), wallet_before + AMOUNT);
    assert_eq!(token.balance(&t.contract_id), 0);
}

#[test]
fn test_refund_without_flag_is_a_transfer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let id = create(
        &t,
        "V-OFF",
        default_options(&t.env),
        ShipmentOptionsExt::default(),
    );
    assert!(!client.is_refund_to_vault(&id));
    let wallet_before = token.balance(&t.buyer);

    let events = capture_events(&t.env, || client.cancel_shipment(&t.buyer, &id));
    let data = find_event(&t.env, &events, "shipment_cancelled").expect("event");
    assert!(!to_vault(&t.env, &data));

    assert_eq!(token.balance(&t.buyer), wallet_before + AMOUNT);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), 0);
    assert_reconciled(&t);
}

#[test]
fn test_buyer_profile_routes_plain_shipments_to_vault() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(!client.get_buyer_profile(&t.buyer).refund_to_vault);
    client.set_buyer_profile(
        &t.buyer,
        &BuyerProfile {
            refund_to_vault: true,
        },
    );
    assert!(client.get_buyer_profile(&t.buyer).refund_to_vault);

    let id = sid(&t.env, "V-PROFILE");
    client.create_shipment(
        &id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &AMOUNT,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.cancel_shipment(&t.buyer, &id);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), AMOUNT);
    assert_reconciled(&t);
}

#[test]
fn test_supplier_cancel_refund_credited_to_vault() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let mut opts = default_options(&t.env);
    opts.response_deadline = 10;
    opts.penalty_bps = 500;
    let id = create(&t, "V-SUPCANCEL", opts, vault_ext());
    let wallet_before = token.balance(&t.buyer);

    t.env.ledger().set_sequence_number(5);
    submit_proof(&t, &id, 0);
    t.env.ledger().set_sequence_number(20);
    client.supplier_cancel(&t.supplier, &id);

    assert_eq!(token.balance(&t.buyer), wallet_before);
    let penalty = AMOUNT * 500 / 10_000;
    assert_eq!(
        client.get_vault_balance(&t.buyer, &t.token_id),
        AMOUNT - penalty
    );
    assert_reconciled(&t);
}

#[test]
fn test_deadline_refund_credited_to_vault() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let mut opts = default_options(&t.env);
    opts.deadlines = soroban_sdk::vec![&t.env, 1_000u64, 0u64, 0u64];
    let id = create(&t, "V-DEADLINE", opts, vault_ext());
    let wallet_before = token.balance(&t.buyer);

    t.env.ledger().set_timestamp(1_001);
    client.claim_deadline_refund(&t.buyer, &id, &0);

    assert_eq!(token.balance(&t.buyer), wallet_before);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), AMOUNT);
    assert_reconciled(&t);
}

#[test]
fn test_refund_recipient_vault_is_credited() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let recipient = Address::generate(&t.env);
    let mut opts = default_options(&t.env);
    opts.refund_recipient = Some(recipient.clone());
    let id = create(&t, "V-RECIP", opts, vault_ext());

    client.cancel_shipment(&t.buyer, &id);
    assert_eq!(client.get_vault_balance(&recipient, &t.token_id), AMOUNT);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), 0);
}

// ── Dispute paths ───────────────────────────────────────────────────────────

#[test]
fn test_dispute_timeout_refund_credited_to_vault() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let mut opts = default_options(&t.env);
    opts.dispute_timeout_seconds = 100;
    opts.default_resolution = Resolution::Buyer;
    let id = create(&t, "V-TIMEOUT", opts, vault_ext());
    let wallet_before = token.balance(&t.buyer);

    t.env.ledger().set_timestamp(1_000);
    submit_proof(&t, &id, 0);
    client.raise_dispute(&t.buyer, &id, &0);
    t.env.ledger().set_timestamp(1_100);

    let events = capture_events(&t.env, || client.resolve_dispute_timeout(&id, &0));
    let data = find_event(&t.env, &events, "dispute_resolved").expect("event");
    assert!(to_vault(&t.env, &data));

    assert_eq!(token.balance(&t.buyer), wallet_before);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), AMOUNT / 4);
    assert_reconciled(&t);
}

#[test]
fn test_split_resolution_buyer_share_credited_to_vault() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let id = create(&t, "V-SPLIT", default_options(&t.env), vault_ext());
    let wallet_before = token.balance(&t.buyer);

    submit_proof(&t, &id, 0);
    client.raise_dispute(&t.buyer, &id, &0);
    // Supplier gets 60%, buyer 40% of the 25% milestone.
    client.resolve_dispute_split(&t.arbiter, &id, &0, &6_000);

    let milestone = AMOUNT / 4;
    assert_eq!(token.balance(&t.buyer), wallet_before);
    assert_eq!(token.balance(&t.supplier), milestone * 6_000 / 10_000);
    assert_eq!(
        client.get_vault_balance(&t.buyer, &t.token_id),
        milestone - milestone * 6_000 / 10_000
    );
    assert_reconciled(&t);
}

#[test]
fn test_dispute_refund_without_flag_is_a_transfer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = token::Client::new(&t.env, &t.token_id);
    let mut opts = default_options(&t.env);
    opts.dispute_timeout_seconds = 100;
    opts.default_resolution = Resolution::Buyer;
    let id = create(&t, "V-TIMEOUT-OFF", opts, ShipmentOptionsExt::default());
    let wallet_before = token.balance(&t.buyer);

    t.env.ledger().set_timestamp(1_000);
    submit_proof(&t, &id, 0);
    client.raise_dispute(&t.buyer, &id, &0);
    t.env.ledger().set_timestamp(1_100);
    let events = capture_events(&t.env, || client.resolve_dispute_timeout(&id, &0));
    let data = find_event(&t.env, &events, "dispute_resolved").expect("event");
    assert!(!to_vault(&t.env, &data));

    assert_eq!(token.balance(&t.buyer), wallet_before + AMOUNT / 4);
    assert_eq!(client.get_vault_balance(&t.buyer, &t.token_id), 0);
}
