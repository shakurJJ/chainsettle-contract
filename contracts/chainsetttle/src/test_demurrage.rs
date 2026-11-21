// #734 — Demurrage charges payable by the buyer after free days at destination.
//
// The logistics provider marks arrival with `mark_arrived_at_destination`.
// After `demurrage_free_ledgers`, `demurrage_rate_per_ledger` accrues per
// ledger, drawn from a buyer-funded deposit and capped by it. Accrual stops on
// final confirmation; the charge goes to logistics and the unused deposit back
// to the buyer when the shipment closes.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{testutils::Ledger as _, token, String, Symbol};

const TOTAL: i128 = 1_000_000;
const RATE: i128 = 100;
const FREE: u32 = 10;
const DEPOSIT: i128 = 5_000;

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn demurrage_ext(free: u32, rate: i128, deposit: i128) -> ShipmentOptionsExt {
    ShipmentOptionsExt {
        demurrage_free_ledgers: free,
        demurrage_rate_per_ledger: rate,
        demurrage_deposit: deposit,
        ..Default::default()
    }
}

fn create_ext(t: &TestSetup, id: &str, opts: &ShipmentOptions, ext: &ShipmentOptionsExt) -> String {
    let shipment_id = String::from_str(&t.env, id);
    client(t).create_shipment_ext(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &build_milestones(&t.env),
        opts,
        ext,
    );
    shipment_id
}

fn create(t: &TestSetup, id: &str) -> String {
    create_ext(t, id, &default_options(&t.env), &demurrage_ext(FREE, RATE, DEPOSIT))
}

fn confirm(t: &TestSetup, shipment_id: &String, index: u32) {
    client(t).submit_proof(
        &t.supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client(t).confirm_milestone(&t.buyer, shipment_id, &index);
}

fn confirm_all(t: &TestSetup, shipment_id: &String) {
    for i in 0..3 {
        confirm(t, shipment_id, i);
    }
}

fn balance(t: &TestSetup, addr: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(addr)
}

fn ledger(t: &TestSetup) -> u32 {
    t.env.ledger().sequence()
}

fn set_ledger(t: &TestSetup, seq: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number = seq);
}

// ── Defaults / backwards compatibility ──────────────────────────────────────

#[test]
fn test_demurrage_disabled_by_default() {
    let t = setup();
    let id = String::from_str(&t.env, "DM-OFF");
    client(&t).create_shipment(
        &id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    assert_eq!(client(&t).get_shipment_options_ext(&id), ShipmentOptionsExt::default());
    assert_eq!(client(&t).get_demurrage_state(&id), None);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 0);
    assert!(client(&t).try_mark_arrived_at_destination(&t.logistics, &id).is_err());

    // Completing the shipment moves nothing extra.
    confirm_all(&t, &id);
    assert_eq!(balance(&t, &t.logistics), 0);
    assert_eq!(balance(&t, &t.supplier), TOTAL);
}

#[test]
fn test_create_shipment_ext_with_default_ext_matches_create_shipment() {
    let t = setup();
    let before = balance(&t, &t.buyer);
    let id = create_ext(&t, "DM-DEF", &default_options(&t.env), &ShipmentOptionsExt::default());

    assert_eq!(before - balance(&t, &t.buyer), TOTAL);
    assert_eq!(client(&t).get_demurrage_state(&id), None);
    assert_eq!(client(&t).get_shipment_options_ext(&id), ShipmentOptionsExt::default());
}

#[test]
fn test_buyer_funds_deposit_at_creation() {
    let t = setup();
    let before = balance(&t, &t.buyer);
    let id = create(&t, "DM-FUND");

    assert_eq!(before - balance(&t, &t.buyer), TOTAL + DEPOSIT);
    assert_eq!(balance(&t, &t.contract_id), TOTAL + DEPOSIT);
    let state = client(&t).get_demurrage_state(&id).unwrap();
    assert_eq!(state.deposit, DEPOSIT);
    assert_eq!(state.rate_per_ledger, RATE);
    assert_eq!(state.free_ledgers, FREE);
    assert_eq!(state.arrived_ledger, None);
    assert!(!state.settled);
    assert_eq!(client(&t).get_shipment_options_ext(&id), demurrage_ext(FREE, RATE, DEPOSIT));
}

// ── Accrual ─────────────────────────────────────────────────────────────────

#[test]
fn test_nothing_accrues_before_arrival() {
    let t = setup();
    let id = create(&t, "DM-TRANSIT");
    set_ledger(&t, ledger(&t) + 1_000);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 0);
}

#[test]
fn test_accrual_starts_only_after_free_period() {
    let t = setup();
    let id = create(&t, "DM-FREE");
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    assert_eq!(client(&t).get_demurrage_state(&id).unwrap().arrived_ledger, Some(arrived));

    set_ledger(&t, arrived + FREE - 1);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 0);
    set_ledger(&t, arrived + FREE);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 0, "last free ledger is free");
    set_ledger(&t, arrived + FREE + 1);
    assert_eq!(client(&t).get_demurrage_accrued(&id), RATE);
    set_ledger(&t, arrived + FREE + 7);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 7 * RATE);
}

#[test]
fn test_zero_free_period_accrues_from_next_ledger() {
    let t = setup();
    let id = create_ext(&t, "DM-NOFREE", &default_options(&t.env), &demurrage_ext(0, RATE, DEPOSIT));
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 0);
    set_ledger(&t, arrived + 3);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 3 * RATE);
}

#[test]
fn test_charge_never_exceeds_deposit() {
    let t = setup();
    let id = create(&t, "DM-CAP");
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);

    // 50 chargeable ledgers * 100 == deposit exactly.
    set_ledger(&t, arrived + FREE + 50);
    assert_eq!(client(&t).get_demurrage_accrued(&id), DEPOSIT);
    set_ledger(&t, arrived + FREE + 51);
    assert_eq!(client(&t).get_demurrage_accrued(&id), DEPOSIT);
    set_ledger(&t, arrived + 1_000);
    assert_eq!(client(&t).get_demurrage_accrued(&id), DEPOSIT);
}

#[test]
fn test_huge_rate_is_capped_without_overflow() {
    let t = setup();
    let id = create_ext(&t, "DM-HUGE", &default_options(&t.env), &demurrage_ext(0, i128::MAX, DEPOSIT));
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    set_ledger(&t, arrived + 1_000);
    assert_eq!(client(&t).get_demurrage_accrued(&id), DEPOSIT);
}

// ── Settlement ──────────────────────────────────────────────────────────────

#[test]
fn test_final_confirmation_stops_accrual_and_settles() {
    let t = setup();
    let id = create(&t, "DM-SETTLE");
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);

    set_ledger(&t, arrived + FREE + 12);
    let buyer_before = balance(&t, &t.buyer);
    confirm_all(&t, &id);

    let charge = 12 * RATE;
    assert_eq!(balance(&t, &t.logistics), charge);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, DEPOSIT - charge);
    assert_eq!(balance(&t, &t.supplier), TOTAL);
    assert_eq!(balance(&t, &t.contract_id), 0);

    let state = client(&t).get_demurrage_state(&id).unwrap();
    assert!(state.settled);
    assert_eq!(state.charged, charge);
    assert_eq!(state.stopped_ledger, Some(arrived + FREE + 12));

    // Accrual stays frozen afterwards.
    set_ledger(&t, arrived + FREE + 500);
    assert_eq!(client(&t).get_demurrage_accrued(&id), charge);
    assert_eq!(balance(&t, &t.logistics), charge);
}

#[test]
fn test_confirmation_before_free_period_ends_charges_nothing() {
    let t = setup();
    let id = create(&t, "DM-EARLY");
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    set_ledger(&t, arrived + FREE);

    let buyer_before = balance(&t, &t.buyer);
    confirm_all(&t, &id);

    assert_eq!(balance(&t, &t.logistics), 0);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, DEPOSIT);
}

#[test]
fn test_capped_charge_pays_whole_deposit_to_logistics() {
    let t = setup();
    let id = create(&t, "DM-CAPPAY");
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    set_ledger(&t, arrived + FREE + 100);

    let buyer_before = balance(&t, &t.buyer);
    confirm_all(&t, &id);

    assert_eq!(balance(&t, &t.logistics), DEPOSIT);
    assert_eq!(balance(&t, &t.buyer), buyer_before);
    assert_eq!(balance(&t, &t.contract_id), 0);
}

#[test]
fn test_completion_without_arrival_refunds_full_deposit() {
    let t = setup();
    let id = create(&t, "DM-NOARR");
    set_ledger(&t, ledger(&t) + 1_000);
    let buyer_before = balance(&t, &t.buyer);
    confirm_all(&t, &id);

    assert_eq!(balance(&t, &t.logistics), 0);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, DEPOSIT);
}

#[test]
fn test_accrual_stops_on_final_confirmation_even_when_payment_is_held() {
    let t = setup();
    let mut opts = default_options(&t.env);
    opts.holdback_ledgers = 100;
    let id = create_ext(&t, "DM-HELD", &opts, &demurrage_ext(FREE, RATE, DEPOSIT));
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);

    set_ledger(&t, arrived + FREE + 5);
    confirm(&t, &id, 0);
    confirm(&t, &id, 1);
    assert_eq!(client(&t).get_demurrage_state(&id).unwrap().stopped_ledger, None);
    confirm(&t, &id, 2);
    let stopped_at = arrived + FREE + 5;
    assert_eq!(client(&t).get_demurrage_state(&id).unwrap().stopped_ledger, Some(stopped_at));

    // Payment is released long after the final confirmation.
    set_ledger(&t, stopped_at + 200);
    assert_eq!(client(&t).get_demurrage_accrued(&id), 5 * RATE);
    for i in 0..3u32 {
        client(&t).release_held_payment(&id, &i);
    }
    assert_eq!(client(&t).get_shipment(&id).status, ShipmentStatus::Completed);
    assert_eq!(balance(&t, &t.logistics), 5 * RATE);
}

#[test]
fn test_buyer_cancellation_settles_accrued_demurrage() {
    let t = setup();
    let id = create(&t, "DM-CANCEL");
    let arrived = ledger(&t);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    set_ledger(&t, arrived + FREE + 3);

    let buyer_before = balance(&t, &t.buyer);
    client(&t).cancel_shipment(&t.buyer, &id);

    assert_eq!(balance(&t, &t.logistics), 3 * RATE);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, TOTAL + DEPOSIT - 3 * RATE);
    assert_eq!(balance(&t, &t.contract_id), 0);
    assert!(client(&t).get_demurrage_state(&id).unwrap().settled);
}

// ── Access control & validation ─────────────────────────────────────────────

#[test]
#[should_panic(expected = "only logistics can mark arrival")]
fn test_only_logistics_can_mark_arrival() {
    let t = setup();
    let id = create(&t, "DM-AUTH");
    client(&t).mark_arrived_at_destination(&t.supplier, &id);
}

#[test]
#[should_panic(expected = "arrival already marked")]
fn test_arrival_can_only_be_marked_once() {
    let t = setup();
    let id = create(&t, "DM-TWICE");
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
    set_ledger(&t, ledger(&t) + 50);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
}

#[test]
#[should_panic(expected = "shipment is not active")]
fn test_cannot_mark_arrival_after_completion() {
    let t = setup();
    let id = create(&t, "DM-LATE");
    confirm_all(&t, &id);
    client(&t).mark_arrived_at_destination(&t.logistics, &id);
}

#[test]
#[should_panic(expected = "demurrage requires a deposit")]
fn test_rate_without_deposit_rejected() {
    let t = setup();
    create_ext(&t, "DM-NODEP", &default_options(&t.env), &demurrage_ext(FREE, RATE, 0));
}

#[test]
#[should_panic(expected = "demurrage deposit requires a rate")]
fn test_deposit_without_rate_rejected() {
    let t = setup();
    create_ext(&t, "DM-NORATE", &default_options(&t.env), &demurrage_ext(FREE, 0, DEPOSIT));
}

#[test]
#[should_panic(expected = "invalid demurrage terms")]
fn test_negative_rate_rejected() {
    let t = setup();
    create_ext(&t, "DM-NEG", &default_options(&t.env), &demurrage_ext(FREE, -1, DEPOSIT));
}
