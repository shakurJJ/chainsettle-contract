// #520 — Retainage held until final milestone completion.
//
// `retainage_bps` of every net milestone payment is withheld into a per-shipment
// balance, released to the supplier when the shipment completes, and refunded to
// the buyer if the shipment is cancelled.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{testutils::Events as _, token, String, Symbol, TryFromVal};

const TOTAL: i128 = 1_000_000;

fn retainage_options(t: &TestSetup, bps: u32) -> ShipmentOptions {
    let mut opts = default_options(&t.env);
    opts.retainage_bps = bps;
    opts
}

fn create(t: &TestSetup, id: &str, opts: &ShipmentOptions) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &build_milestones(&t.env),
        opts,
    );
    shipment_id
}

fn confirm(t: &TestSetup, shipment_id: &String, index: u32) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.submit_proof(
        &t.supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, shipment_id, &index);
}

fn balance(t: &TestSetup, addr: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(addr)
}

fn has_event(t: &TestSetup, topic: &str) -> bool {
    let wanted = Symbol::new(&t.env, topic);
    t.env.events().all().iter().any(|e| {
        e.1.get(0)
            .and_then(|v| Symbol::try_from_val(&t.env, &v).ok())
            .map(|s| s == wanted)
            .unwrap_or(false)
    })
}

#[test]
fn test_each_confirmation_withholds_exactly_retainage_bps() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "RET-EACH", &retainage_options(&t, 1_000)); // 10%

    let before = balance(&t, &t.supplier);
    confirm(&t, &id, 0); // 250_000 gross
    assert!(has_event(&t, "holdback_withheld"));
    assert_eq!(balance(&t, &t.supplier) - before, 225_000);
    assert_eq!(client.get_retainage_balance(&id), 25_000);

    let before = balance(&t, &t.supplier);
    confirm(&t, &id, 1); // 500_000 gross
    assert_eq!(balance(&t, &t.supplier) - before, 450_000);
    assert_eq!(client.get_retainage_balance(&id), 75_000);
}

#[test]
fn test_withholding_applies_to_net_payment_after_fees() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &1_000u32, &t.treasury); // 10% platform fee
    let id = create(&t, "RET-NET", &retainage_options(&t, 1_000));

    let before = balance(&t, &t.supplier);
    confirm(&t, &id, 0);
    // 250_000 gross → 225_000 net → 22_500 retained.
    assert_eq!(client.get_retainage_balance(&id), 22_500);
    assert_eq!(balance(&t, &t.supplier) - before, 202_500);
}

#[test]
fn test_advance_combines_with_platform_fee_and_retainage() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &1_000u32, &t.treasury);
    let id = create(&t, "RET-ADVANCE", &retainage_options(&t, 1_000));

    let supplier_before = balance(&t, &t.supplier);
    client.request_advance(&t.supplier, &id, &0u32, &10u32);
    client.approve_advance(&t.buyer, &id, &0u32);
    assert_eq!(balance(&t, &t.supplier) - supplier_before, 25_000);

    confirm(&t, &id, 0);

    // 250,000 gross - 25,000 fee - 25,000 advance = 200,000;
    // 10% of that net transfer is retained, leaving 180,000 for the supplier.
    assert_eq!(balance(&t, &t.supplier) - supplier_before, 205_000);
    assert_eq!(client.get_retainage_balance(&id), 20_000);
}

#[test]
fn test_final_confirmation_releases_all_retainage() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "RET-FINAL", &retainage_options(&t, 2_000));

    let before = balance(&t, &t.supplier);
    confirm(&t, &id, 0);
    confirm(&t, &id, 1);
    assert_eq!(client.get_retainage_balance(&id), 150_000);

    confirm(&t, &id, 2);
    assert!(has_event(&t, "retainage_released"));
    assert_eq!(client.get_retainage_balance(&id), 0);
    assert_eq!(balance(&t, &t.supplier) - before, TOTAL);

    let s = client.get_shipment(&id);
    assert_eq!(s.status, ShipmentStatus::Completed);
    let released = s
        .audit_log
        .iter()
        .any(|e| e.action == Symbol::new(&t.env, "retainage_released"));
    assert!(released);
    assert_eq!(client.get_total_escrowed_value(&t.token_id), 0);
}

#[test]
fn test_cancellation_refunds_retainage_to_buyer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "RET-CANCEL", &retainage_options(&t, 1_000));
    confirm(&t, &id, 0);
    assert_eq!(client.get_retainage_balance(&id), 25_000);

    let buyer_before = balance(&t, &t.buyer);
    client.cancel_shipment(&t.buyer, &id);
    assert!(has_event(&t, "holdback_refunded"));

    // Unreleased 750_000 plus the 25_000 retainage.
    assert_eq!(balance(&t, &t.buyer) - buyer_before, 775_000);
    assert_eq!(client.get_retainage_balance(&id), 0);
    assert_eq!(client.get_total_escrowed_value(&t.token_id), 0);
}

#[test]
fn test_completion_and_escrow_balance_stay_consistent() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "RET-CONSIST", &retainage_options(&t, 1_000));

    confirm(&t, &id, 0);
    assert_eq!(client.get_completion_percentage(&id), 25);
    assert_eq!(client.get_escrow_balance(&id), 750_000);
    // Retainage stays in the contract and is still tracked as escrowed.
    assert_eq!(client.get_total_escrowed_value(&t.token_id), 775_000);

    confirm(&t, &id, 1);
    assert_eq!(client.get_completion_percentage(&id), 75);
    assert_eq!(client.get_escrow_balance(&id), 250_000);
}

#[test]
fn test_zero_retainage_is_backward_compatible() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "RET-OFF", &default_options(&t.env));
    let before = balance(&t, &t.supplier);
    confirm(&t, &id, 0);
    assert_eq!(balance(&t, &t.supplier) - before, 250_000);
    assert_eq!(client.get_retainage_balance(&id), 0);
}

#[test]
#[should_panic(expected = "retainage_bps exceeds maximum")]
fn test_creation_rejects_retainage_above_max() {
    let t = setup();
    create(&t, "RET-MAX", &retainage_options(&t, 2_001));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_buyer_cannot_cancel_to_reclaim_retainage() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "RET-AUTH", &retainage_options(&t, 1_000));
    confirm(&t, &id, 0);
    client.cancel_shipment(&t.supplier, &id);
}
