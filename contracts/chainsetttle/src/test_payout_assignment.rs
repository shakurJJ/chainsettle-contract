// #800 — Payout assignment (invoice factoring) lifecycle.
//
// Covers `assign_payout`, `transfer_payout_assignment`,
// `release_payout_assignment` and `get_payout_assignment`: an assigned
// milestone pays the financier, a transfer moves the receivable, a release
// restores the supplier, and a dispute resolved against the supplier pays the
// assignee nothing.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, String, Symbol,
};

const TOTAL: i128 = 1_000_000;
// build_milestones splits 25% / 50% / 25%.
const M0: i128 = 250_000;
const M1: i128 = 500_000;
const M2: i128 = 250_000;

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn create_with(t: &TestSetup, id: &str, opts: &ShipmentOptions) -> String {
    let shipment_id = String::from_str(&t.env, id);
    client(t).create_shipment(
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

fn create(t: &TestSetup, id: &str) -> String {
    create_with(t, id, &default_options(&t.env))
}

fn submit(t: &TestSetup, shipment_id: &String, index: u32, proof: &str) {
    client(t).submit_proof(
        &t.supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, proof),
        &Symbol::new(&t.env, "ipfs"),
    );
}

fn confirm(t: &TestSetup, shipment_id: &String, index: u32) {
    submit(t, shipment_id, index, "proof");
    client(t).confirm_milestone(&t.buyer, shipment_id, &index);
}

fn balance(t: &TestSetup, addr: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(addr)
}

// ── get_payout_assignment / assign_payout ───────────────────────────────────

#[test]
fn test_no_assignment_by_default() {
    let t = setup();
    let id = create(&t, "PA-NONE");
    assert_eq!(client(&t).get_payout_assignment(&id), None);
}

#[test]
fn test_assign_records_assignment() {
    let t = setup();
    let id = create(&t, "PA-REC");
    let financier = Address::generate(&t.env);

    client(&t).assign_payout(&t.supplier, &id, &financier);

    let a = client(&t).get_payout_assignment(&id).unwrap();
    assert_eq!(a.supplier, t.supplier);
    assert_eq!(a.financier, financier);
    assert_eq!(a.assigned_at_ledger, t.env.ledger().sequence());
}

#[test]
fn test_assigned_milestone_pays_financier_on_confirmation() {
    let t = setup();
    let id = create(&t, "PA-PAY");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    confirm(&t, &id, 0);

    assert_eq!(balance(&t, &financier), M0);
    assert_eq!(balance(&t, &t.supplier), 0);
}

#[test]
fn test_assigned_payout_covers_every_milestone_until_completion() {
    let t = setup();
    let id = create(&t, "PA-ALL");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    confirm(&t, &id, 0);
    confirm(&t, &id, 1);
    confirm(&t, &id, 2);

    assert_eq!(balance(&t, &financier), TOTAL);
    assert_eq!(balance(&t, &t.supplier), 0);
    assert_eq!(client(&t).get_shipment(&id).status, ShipmentStatus::Completed);
}

#[test]
fn test_approved_advance_pays_financier() {
    let t = setup();
    let id = create(&t, "PA-ADV");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    client(&t).request_advance(&t.supplier, &id, &1u32, &20u32);
    client(&t).approve_advance(&t.buyer, &id, &1u32);
    assert_eq!(balance(&t, &financier), M1 * 20 / 100);

    // The advance is deducted from the milestone payout, so the financier
    // ends up with exactly the milestone value.
    confirm(&t, &id, 1);
    assert_eq!(balance(&t, &financier), M1);
    assert_eq!(balance(&t, &t.supplier), 0);
}

#[test]
fn test_assignment_made_after_a_payout_only_redirects_later_payouts() {
    let t = setup();
    let id = create(&t, "PA-LATE");
    confirm(&t, &id, 0);
    assert_eq!(balance(&t, &t.supplier), M0);

    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);
    confirm(&t, &id, 1);

    assert_eq!(balance(&t, &t.supplier), M0);
    assert_eq!(balance(&t, &financier), M1);
}

#[test]
#[should_panic(expected = "only supplier can assign payout")]
fn test_only_supplier_can_assign() {
    let t = setup();
    let id = create(&t, "PA-AUTH");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.buyer, &id, &financier);
}

#[test]
fn test_supplier_cannot_assign_to_itself() {
    let t = setup();
    let id = create(&t, "PA-SELF");
    // Rejected either by the host (the same address authorising twice in one
    // frame) or by the contract's "financier must differ from supplier" check.
    assert!(client(&t).try_assign_payout(&t.supplier, &id, &t.supplier).is_err());
    assert_eq!(client(&t).get_payout_assignment(&id), None);
}

#[test]
#[should_panic(expected = "payout already assigned")]
fn test_cannot_assign_twice() {
    let t = setup();
    let id = create(&t, "PA-TWICE");
    client(&t).assign_payout(&t.supplier, &id, &Address::generate(&t.env));
    client(&t).assign_payout(&t.supplier, &id, &Address::generate(&t.env));
}

#[test]
#[should_panic(expected = "cannot assign payout during open dispute")]
fn test_cannot_assign_during_open_dispute() {
    let t = setup();
    let id = create(&t, "PA-DISP");
    submit(&t, &id, 0, "proof");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    client(&t).assign_payout(&t.supplier, &id, &Address::generate(&t.env));
}

#[test]
#[should_panic(expected = "shipment is not active")]
fn test_cannot_assign_on_completed_shipment() {
    let t = setup();
    let id = create(&t, "PA-DONE");
    confirm(&t, &id, 0);
    confirm(&t, &id, 1);
    confirm(&t, &id, 2);
    client(&t).assign_payout(&t.supplier, &id, &Address::generate(&t.env));
}

// ── transfer_payout_assignment ──────────────────────────────────────────────

#[test]
fn test_transfer_moves_assignment_to_new_financier() {
    let t = setup();
    let id = create(&t, "PA-XFER");
    let first = Address::generate(&t.env);
    let second = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &first);

    confirm(&t, &id, 0);
    assert_eq!(balance(&t, &first), M0);

    client(&t).transfer_payout_assignment(&first, &id, &second);
    let a = client(&t).get_payout_assignment(&id).unwrap();
    assert_eq!(a.financier, second);
    assert_eq!(a.supplier, t.supplier);

    confirm(&t, &id, 1);
    assert_eq!(balance(&t, &first), M0, "old financier receives nothing more");
    assert_eq!(balance(&t, &second), M1);
    assert_eq!(balance(&t, &t.supplier), 0);
}

#[test]
#[should_panic(expected = "only current financier can transfer assignment")]
fn test_only_current_financier_can_transfer() {
    let t = setup();
    let id = create(&t, "PA-XAUTH");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);
    let stranger = Address::generate(&t.env);
    client(&t).transfer_payout_assignment(&stranger, &id, &Address::generate(&t.env));
}

#[test]
#[should_panic(expected = "invalid new financier")]
fn test_cannot_transfer_back_to_supplier() {
    let t = setup();
    let id = create(&t, "PA-XSUP");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);
    client(&t).transfer_payout_assignment(&financier, &id, &t.supplier);
}

#[test]
#[should_panic(expected = "payout not assigned")]
fn test_cannot_transfer_missing_assignment() {
    let t = setup();
    let id = create(&t, "PA-XNONE");
    let financier = Address::generate(&t.env);
    client(&t).transfer_payout_assignment(&financier, &id, &Address::generate(&t.env));
}

// ── release_payout_assignment ───────────────────────────────────────────────

#[test]
fn test_release_restores_supplier_payouts() {
    let t = setup();
    let id = create(&t, "PA-REL");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    confirm(&t, &id, 0);
    client(&t).release_payout_assignment(&financier, &id);
    assert_eq!(client(&t).get_payout_assignment(&id), None);

    confirm(&t, &id, 1);
    confirm(&t, &id, 2);
    assert_eq!(balance(&t, &financier), M0);
    assert_eq!(balance(&t, &t.supplier), M1 + M2);
}

#[test]
fn test_release_after_transfer_restores_supplier() {
    let t = setup();
    let id = create(&t, "PA-XREL");
    let first = Address::generate(&t.env);
    let second = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &first);
    client(&t).transfer_payout_assignment(&first, &id, &second);
    client(&t).release_payout_assignment(&second, &id);

    confirm(&t, &id, 0);
    assert_eq!(balance(&t, &t.supplier), M0);
    assert_eq!(balance(&t, &first), 0);
    assert_eq!(balance(&t, &second), 0);
}

#[test]
fn test_supplier_can_reassign_after_release() {
    let t = setup();
    let id = create(&t, "PA-REASSIGN");
    let first = Address::generate(&t.env);
    let second = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &first);
    client(&t).release_payout_assignment(&first, &id);
    client(&t).assign_payout(&t.supplier, &id, &second);

    confirm(&t, &id, 0);
    assert_eq!(balance(&t, &second), M0);
    assert_eq!(balance(&t, &first), 0);
}

#[test]
#[should_panic(expected = "only current financier can release assignment")]
fn test_supplier_cannot_release_assignment() {
    let t = setup();
    let id = create(&t, "PA-RAUTH");
    client(&t).assign_payout(&t.supplier, &id, &Address::generate(&t.env));
    client(&t).release_payout_assignment(&t.supplier, &id);
}

#[test]
#[should_panic(expected = "only current financier can release assignment")]
fn test_previous_financier_cannot_release_after_transfer() {
    let t = setup();
    let id = create(&t, "PA-ROLD");
    let first = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &first);
    client(&t).transfer_payout_assignment(&first, &id, &Address::generate(&t.env));
    client(&t).release_payout_assignment(&first, &id);
}

#[test]
#[should_panic(expected = "payout not assigned")]
fn test_cannot_release_missing_assignment() {
    let t = setup();
    let id = create(&t, "PA-RNONE");
    client(&t).release_payout_assignment(&Address::generate(&t.env), &id);
}

// ── Disputes ────────────────────────────────────────────────────────────────

#[test]
fn test_dispute_resolved_against_supplier_pays_assignee_nothing() {
    let t = setup();
    let id = create(&t, "PA-DLOSE");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    submit(&t, &id, 0, "proof");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    client(&t).resolve_dispute(&t.arbiter, &id, &0u32, &false, &None);

    assert_eq!(balance(&t, &financier), 0);
    assert_eq!(balance(&t, &t.supplier), 0);
    let shipment = client(&t).get_shipment(&id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Pending);
    assert_eq!(shipment.released_amount, 0);
    // The assignment survives the dispute.
    assert_eq!(client(&t).get_payout_assignment(&id).unwrap().financier, financier);
}

#[test]
fn test_partial_dispute_lost_by_supplier_refunds_buyer_not_assignee() {
    let t = setup();
    let id = create(&t, "PA-PLOSE");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    submit(&t, &id, 1, "proof");
    let buyer_before = balance(&t, &t.buyer);
    client(&t).raise_partial_dispute(&t.buyer, &id, &1u32, &40u32);
    // The uncontested 60% belongs to the receivable and goes to the financier.
    let uncontested = M1 * 60 / 100;
    assert_eq!(balance(&t, &financier), uncontested);

    client(&t).resolve_dispute(&t.arbiter, &id, &1u32, &false, &None);

    let contested = M1 * 40 / 100;
    assert_eq!(balance(&t, &financier), uncontested, "assignee gets nothing from the lost portion");
    assert_eq!(balance(&t, &t.buyer) - buyer_before, contested);
    assert_eq!(balance(&t, &t.supplier), 0);
}

#[test]
fn test_dispute_timeout_for_buyer_pays_assignee_nothing() {
    let t = setup();
    let mut opts = default_options(&t.env);
    opts.dispute_timeout_seconds = 100;
    opts.default_resolution = Resolution::Buyer;
    let id = create_with(&t, "PA-TLOSE", &opts);
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    submit(&t, &id, 0, "proof");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    let buyer_before = balance(&t, &t.buyer);
    let now = t.env.ledger().timestamp();
    t.env.ledger().with_mut(|l| l.timestamp = now + 101);
    client(&t).resolve_dispute_timeout(&id, &0u32);

    assert_eq!(balance(&t, &financier), 0);
    assert_eq!(balance(&t, &t.supplier), 0);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, M0);
}

#[test]
fn test_dispute_timeout_for_supplier_pays_assignee() {
    let t = setup();
    let mut opts = default_options(&t.env);
    opts.dispute_timeout_seconds = 100;
    opts.default_resolution = Resolution::Supplier;
    let id = create_with(&t, "PA-TWIN", &opts);
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    submit(&t, &id, 0, "proof");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    let now = t.env.ledger().timestamp();
    t.env.ledger().with_mut(|l| l.timestamp = now + 101);
    client(&t).resolve_dispute_timeout(&id, &0u32);

    assert_eq!(balance(&t, &financier), M0);
    assert_eq!(balance(&t, &t.supplier), 0);
}

#[test]
fn test_dispute_resolved_for_supplier_pays_assignee() {
    let t = setup();
    let id = create(&t, "PA-DWIN");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    submit(&t, &id, 0, "proof");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    client(&t).resolve_dispute(&t.arbiter, &id, &0u32, &true, &None);

    assert_eq!(balance(&t, &financier), M0);
    assert_eq!(balance(&t, &t.supplier), 0);
}

// ── Refunds never reach the assignee ────────────────────────────────────────

#[test]
fn test_buyer_cancellation_refunds_buyer_not_assignee() {
    let t = setup();
    let id = create(&t, "PA-CANCEL");
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);
    confirm(&t, &id, 0);

    let buyer_before = balance(&t, &t.buyer);
    client(&t).cancel_shipment(&t.buyer, &id);

    assert_eq!(balance(&t, &financier), M0);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, TOTAL - M0);
    assert_eq!(balance(&t, &t.supplier), 0);
}
