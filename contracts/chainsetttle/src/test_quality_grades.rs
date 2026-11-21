// #519 — Quality-graded milestone confirmation.
//
// Buyers confirm a milestone with a pre-agreed quality grade. The grade's basis
// points decide the supplier's share of the milestone; the remainder is refunded
// to the buyer. The supplier may contest a grade with `raise_dispute` during the
// grade review window.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Events as _, Ledger as _},
    token, vec, String, Symbol, TryFromVal,
};

const TOTAL: i128 = 1_000_000;
// Milestone 0 is 25% of TOTAL.
const M0_GROSS: i128 = 250_000;
const REVIEW_WINDOW: u32 = 100;

fn graded_options(t: &TestSetup) -> ShipmentOptions {
    let mut opts = default_options(&t.env);
    opts.quality_grades = vec![&t.env, 10_000u32, 9_000u32, 7_500u32];
    // Short review window keeps ledger jumps within the test host's storage TTL.
    opts.review_window_ledgers = Some(REVIEW_WINDOW);
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

fn submit(t: &TestSetup, shipment_id: &String, index: u32) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.submit_proof(
        &t.supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
}

fn balance(t: &TestSetup, addr: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(addr)
}

fn advance_past_review_window(t: &TestSetup, shipment_id: &String, index: u32) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let release_after = client.get_milestone(shipment_id, &index).release_after_ledger;
    t.env.ledger().with_mut(|l| l.sequence_number = release_after);
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
fn test_grade_recorded_in_audit_log_and_events() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-AUDIT", &graded_options(&t));
    submit(&t, &id, 0);
    let supplier_before = balance(&t, &t.supplier);
    let buyer_before = balance(&t, &t.buyer);

    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &1u32);
    assert!(has_event(&t, "milestone_graded"));

    let log = client.get_shipment(&id).audit_log;
    let last = log.get(log.len() - 1).unwrap();
    assert_eq!(last.action, Symbol::new(&t.env, "milestone_graded"));
    assert_eq!(last.caller, t.buyer);
    assert_eq!(client.get_milestone_grade(&id, &0u32), Some(1u32));

    advance_past_review_window(&t, &id, 0);
    client.release_held_payment(&id, &0u32);
    assert!(has_event(&t, "grade_settled"));
    assert_eq!(balance(&t, &t.supplier) - supplier_before, 225_000);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, 25_000);
}

#[test]
#[should_panic(expected = "invalid grade index")]
fn test_invalid_grade_index_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-BAD", &graded_options(&t));
    submit(&t, &id, 0);
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &3u32);
}

#[test]
#[should_panic(expected = "quality grades not configured")]
fn test_graded_confirmation_requires_configured_grades() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-NONE", &default_options(&t.env));
    submit(&t, &id, 0);
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &0u32);
}

#[test]
#[should_panic(expected = "milestone proof not yet submitted")]
fn test_graded_confirmation_requires_submitted_proof() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-NOPROOF", &graded_options(&t));
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &1u32);
}

#[test]
fn test_grading_requires_buyer_signature() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-SIG", &graded_options(&t));
    submit(&t, &id, 0);

    t.env.set_auths(&[]);
    assert!(client
        .try_confirm_milestone_graded(&t.buyer, &id, &0u32, &1u32)
        .is_err());
}

#[test]
#[should_panic(expected = "grade 0 must be 10000 bps")]
fn test_creation_rejects_grade_zero_below_full() {
    let t = setup();
    let mut opts = graded_options(&t);
    opts.quality_grades = vec![&t.env, 9_000u32, 7_500u32];
    create(&t, "GRADE-CFG1", &opts);
}

#[test]
#[should_panic(expected = "quality grades must be non-increasing")]
fn test_creation_rejects_increasing_grades() {
    let t = setup();
    let mut opts = graded_options(&t);
    opts.quality_grades = vec![&t.env, 10_000u32, 7_500u32, 9_000u32];
    create(&t, "GRADE-CFG2", &opts);
}

#[test]
#[should_panic(expected = "grade review window has elapsed")]
fn test_supplier_cannot_contest_after_review_window() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-LATE", &graded_options(&t));
    submit(&t, &id, 0);
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &2u32);
    advance_past_review_window(&t, &id, 0);
    client.raise_dispute(&t.supplier, &id, &0u32);
}

#[test]
#[should_panic(expected = "graded milestone can only be disputed by the supplier")]
fn test_buyer_cannot_dispute_own_grade() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-BUYDISP", &graded_options(&t));
    submit(&t, &id, 0);
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &2u32);
    client.raise_dispute(&t.buyer, &id, &0u32);
}

#[test]
#[should_panic(expected = "grade dispute can only be resolved by the arbiter")]
fn test_buyer_cannot_withdraw_supplier_grade_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-WITHDRAW", &graded_options(&t));
    submit(&t, &id, 0);
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &2u32);
    client.raise_dispute(&t.supplier, &id, &0u32);
    client.withdraw_dispute(&t.buyer, &id, &0u32);
}

#[test]
#[should_panic(expected = "holdback period not yet expired")]
fn test_graded_payment_cannot_be_released_early() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "GRADE-EARLY", &graded_options(&t));
    submit(&t, &id, 0);
    client.confirm_milestone_graded(&t.buyer, &id, &0u32, &1u32);
    client.release_held_payment(&id, &0u32);
}
