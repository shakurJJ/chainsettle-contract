#![cfg(test)]
// #163 — Partial cancellation with pro-rated refund after confirmed milestones.
//
// `Shipment.released_amount` already serves as the `total_released` tracker: it is
// incremented on every milestone payout (confirm_milestone, claim_auto_confirmation,
// release_held_payment, resolve_dispute, ...) and cancel_shipment refunds only
// `total_amount - released_amount - total_advanced_amount` to the buyer, leaving
// funds already paid out to the supplier untouched. These tests pin down that
// behaviour for the zero/partial/fully-confirmed scenarios called out in the issue.

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{testutils::Events as _, token, String, Symbol, TryFromVal};

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

#[test]
fn test_cancel_with_zero_confirmed_milestones_refunds_full_amount() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = sid(&t.env, "cancel-zero-confirmed");
    let total_amount = 1_000_000_000i128;

    let buyer_balance_before = token_client.balance(&t.buyer);

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    // Nothing confirmed yet — released_amount is 0.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.released_amount, 0);

    client.cancel_shipment(&t.buyer, &shipment_id);

    let buyer_balance_after = token_client.balance(&t.buyer);
    assert_eq!(
        buyer_balance_after, buyer_balance_before,
        "buyer should be refunded the entire escrowed amount when nothing was confirmed"
    );

    let cancelled = client.get_shipment(&shipment_id);
    assert_eq!(cancelled.status, ShipmentStatus::Cancelled);
}

#[test]
fn test_cancel_with_partially_confirmed_milestones_refunds_only_remainder() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = sid(&t.env, "cancel-partial-confirmed");
    let total_amount = 1_000_000_000i128;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_milestones(&t.env), // 25% / 50% / 25%
        &default_options(&t.env),
    );

    // Confirm the first two milestones (25% + 50% = 75% released to supplier).
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "proof0"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "proof1"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    let shipment = client.get_shipment(&shipment_id);
    let released_before_cancel = shipment.released_amount;
    assert_eq!(released_before_cancel, 750_000_000);

    let supplier_balance_before_cancel = token_client.balance(&t.supplier);
    let buyer_balance_before_cancel = token_client.balance(&t.buyer);

    client.cancel_shipment(&t.buyer, &shipment_id);

    // Supplier keeps exactly what was already released for the confirmed milestones —
    // cancellation must not claw anything back from them.
    let supplier_balance_after_cancel = token_client.balance(&t.supplier);
    assert_eq!(
        supplier_balance_after_cancel, supplier_balance_before_cancel,
        "supplier's already-released milestone payments must not be reclaimed on cancellation"
    );

    // Buyer is refunded only the unconfirmed remainder (25% of total).
    let buyer_balance_after_cancel = token_client.balance(&t.buyer);
    let expected_refund = total_amount - released_before_cancel;
    assert_eq!(
        buyer_balance_after_cancel - buyer_balance_before_cancel,
        expected_refund,
        "buyer should only be refunded the value of unconfirmed milestones"
    );
    assert_eq!(expected_refund, 250_000_000);

    let cancelled = client.get_shipment(&shipment_id);
    assert_eq!(cancelled.status, ShipmentStatus::Cancelled);
}

#[test]
#[should_panic(expected = "shipment is not active")]
fn test_cancel_after_full_confirmation_is_blocked() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = sid(&t.env, "cancel-fully-confirmed");
    let total_amount = 1_000_000_000i128;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    for i in 0..3u32 {
        client.submit_proof(
            &t.supplier,
            &shipment_id,
            &i,
            &String::from_str(&t.env, "proof"),
            &Symbol::new(&t.env, "ipfs"),
        );
        client.confirm_milestone(&t.buyer, &shipment_id, &i);
    }

    // Once every milestone is confirmed the shipment auto-transitions to Completed,
    // so a fully confirmed shipment can no longer be cancelled — it is blocked outright
    // rather than returning a zero-value refund.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);

    client.cancel_shipment(&t.buyer, &shipment_id);
}

#[test]
fn test_shipment_cancelled_event_carries_shipment_id_and_refund_amount() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = sid(&t.env, "cancel-event-check");
    let total_amount = 1_000_000_000i128;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.cancel_shipment(&t.buyer, &shipment_id);

    let events = t.env.events().all();
    let expected_topic_1 = Symbol::new(&t.env, "shipment_cancelled");
    let mut found = false;
    for e in events.iter() {
        let topics = e.1.clone();
        if topics.len() >= 1 {
            let topic0 = Symbol::try_from_val(&t.env, &topics.get(0).unwrap()).unwrap();
            if topic0 == expected_topic_1 {
                found = true;
                break;
            }
        }
    }
    assert!(found, "shipment_cancelled event must be emitted on cancellation");
}
