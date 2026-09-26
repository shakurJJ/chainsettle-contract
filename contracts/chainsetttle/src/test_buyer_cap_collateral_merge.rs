// Tests for:
//   #477 – per-buyer cap on concurrent open disputes
//   #475 – milestone-linked partial collateral release
//   #517 – merge adjacent pending milestones by mutual consent

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{testutils::Address as _, token, Address, String, Symbol};

const TOTAL: i128 = 1_000_000_000;
const COLLATERAL: i128 = 100_000_000;

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn create(t: &TestSetup, id: &str, buyer: &Address, opts: &ShipmentOptions) -> String {
    let shipment_id = sid(&t.env, id);
    client(t).create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, buyer),
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

fn create_basic(t: &TestSetup, id: &str, buyer: &Address) -> String {
    create(t, id, buyer, &default_options(&t.env))
}

fn create_with_collateral(t: &TestSetup, id: &str) -> String {
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&t.supplier, &COLLATERAL);
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = COLLATERAL;
    create(t, id, &t.buyer, &opts)
}

fn submit(t: &TestSetup, shipment_id: &String, idx: u32) {
    client(t).submit_proof(
        &t.supplier,
        shipment_id,
        &idx,
        &String::from_str(&t.env, "proof_hash"),
        &Symbol::new(&t.env, "ipfs"),
    );
}

fn dispute(t: &TestSetup, buyer: &Address, shipment_id: &String, idx: u32) {
    submit(t, shipment_id, idx);
    client(t).raise_dispute(buyer, shipment_id, &idx);
}

// ============================================================
// #477 – PER-BUYER CONCURRENT DISPUTE CAP
// ============================================================

#[test]
fn test_buyer_dispute_cap_set_get_clear() {
    let t = setup();
    let c = client(&t);
    assert_eq!(c.get_buyer_dispute_cap(&t.buyer2), None);

    c.set_buyer_dispute_cap(&t.buyer, &t.buyer2, &2);
    assert_eq!(c.get_buyer_dispute_cap(&t.buyer2), Some(2));

    c.clear_buyer_dispute_cap(&t.buyer, &t.buyer2);
    assert_eq!(c.get_buyer_dispute_cap(&t.buyer2), None);
}

#[test]
#[should_panic(expected = "buyer dispute cap must be at least 1")]
fn test_buyer_dispute_cap_rejects_zero() {
    let t = setup();
    client(&t).set_buyer_dispute_cap(&t.buyer, &t.buyer2, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_buyer_dispute_cap_non_admin_rejected() {
    let t = setup();
    client(&t).set_buyer_dispute_cap(&t.buyer2, &t.buyer2, &1);
}

#[test]
fn test_uncapped_buyer_unaffected() {
    let t = setup();
    let a = create_basic(&t, "UNCAP-A", &t.buyer2);
    let b = create_basic(&t, "UNCAP-B", &t.buyer2);
    let c = create_basic(&t, "UNCAP-C", &t.buyer2);
    dispute(&t, &t.buyer2, &a, 0);
    dispute(&t, &t.buyer2, &b, 0);
    dispute(&t, &t.buyer2, &c, 0);
    assert_eq!(client(&t).get_buyer_open_disputes(&t.buyer2), 3);
}

#[test]
#[should_panic(expected = "BuyerDisputeCapReached")]
fn test_buyer_at_cap_blocked_even_with_global_room() {
    let t = setup();
    let c = client(&t);
    // Plenty of per-shipment room; the buyer cap is the binding constraint.
    c.set_max_concurrent_disputes(&t.buyer, &5);
    c.set_buyer_dispute_cap(&t.buyer, &t.buyer2, &1);

    let a = create_basic(&t, "CAP-A", &t.buyer2);
    let b = create_basic(&t, "CAP-B", &t.buyer2);
    dispute(&t, &t.buyer2, &a, 0);
    dispute(&t, &t.buyer2, &b, 0);
}

#[test]
#[should_panic(expected = "BuyerDisputeCapReached")]
fn test_buyer_cap_applies_to_partial_disputes() {
    let t = setup();
    let c = client(&t);
    c.set_buyer_dispute_cap(&t.buyer, &t.buyer2, &1);

    let a = create_basic(&t, "CAP-PA", &t.buyer2);
    let b = create_basic(&t, "CAP-PB", &t.buyer2);
    dispute(&t, &t.buyer2, &a, 0);
    submit(&t, &b, 0);
    c.raise_partial_dispute(&t.buyer2, &b, &0, &50);
}

#[test]
#[should_panic(expected = "DisputeAlreadyOpen")]
fn test_global_cap_still_binds_under_generous_buyer_cap() {
    let t = setup();
    let c = client(&t);
    // Default per-shipment cap is 1; buyer cap of 5 does not loosen it.
    c.set_buyer_dispute_cap(&t.buyer, &t.buyer2, &5);
    let a = create_basic(&t, "GLOB-A", &t.buyer2);
    dispute(&t, &t.buyer2, &a, 0);
    dispute(&t, &t.buyer2, &a, 1);
}

#[test]
fn test_buyer_cap_frees_up_after_resolution_and_is_per_buyer() {
    let t = setup();
    let c = client(&t);
    c.set_buyer_dispute_cap(&t.buyer, &t.buyer2, &1);

    let a = create_basic(&t, "FREE-A", &t.buyer2);
    let b = create_basic(&t, "FREE-B", &t.buyer2);
    dispute(&t, &t.buyer2, &a, 0);
    assert_eq!(c.get_buyer_open_disputes(&t.buyer2), 1);

    // A different, uncapped buyer is not affected by buyer2's cap.
    let other = create_basic(&t, "FREE-OTHER", &t.buyer);
    dispute(&t, &t.buyer, &other, 0);

    c.resolve_dispute(&t.arbiter, &a, &0, &true, &None);
    assert_eq!(c.get_buyer_open_disputes(&t.buyer2), 0);

    dispute(&t, &t.buyer2, &b, 0);
    assert_eq!(c.get_buyer_open_disputes(&t.buyer2), 1);
}

// ============================================================
// #475 – MILESTONE-LINKED PARTIAL COLLATERAL RELEASE
// ============================================================

#[test]
fn test_incremental_collateral_disabled_by_default() {
    let t = setup();
    let c = client(&t);
    let tok = token::Client::new(&t.env, &t.token_id);
    let s = create_with_collateral(&t, "COL-DEF");
    assert!(!c.get_incremental_collateral(&s));

    submit(&t, &s, 0);
    c.confirm_milestone(&t.buyer, &s, &0);
    // Nothing released early — historic behaviour.
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL);

    submit(&t, &s, 1);
    c.confirm_milestone(&t.buyer, &s, &1);
    let before = tok.balance(&t.supplier);
    submit(&t, &s, 2);
    c.confirm_milestone(&t.buyer, &s, &2);
    // Final milestone payment (25% net of no fee) + full collateral at completion.
    assert_eq!(tok.balance(&t.supplier) - before, TOTAL / 4 + COLLATERAL);
}

#[test]
fn test_incremental_collateral_released_proportionally() {
    let t = setup();
    let c = client(&t);
    let tok = token::Client::new(&t.env, &t.token_id);
    let s = create_with_collateral(&t, "COL-INC");
    c.set_incremental_collateral(&t.buyer, &s, &true);
    assert!(c.get_incremental_collateral(&s));

    // Milestone 0 = 25%.
    let before = tok.balance(&t.supplier);
    submit(&t, &s, 0);
    c.confirm_milestone(&t.buyer, &s, &0);
    assert_eq!(
        tok.balance(&t.supplier) - before,
        TOTAL / 4 + COLLATERAL / 4
    );
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL * 3 / 4);

    // Milestone 1 = 50%.
    submit(&t, &s, 1);
    c.confirm_milestone(&t.buyer, &s, &1);
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL / 4);

    // Milestone 2 = 25%: shipment completes, total collateral returned exactly once.
    let before = tok.balance(&t.supplier);
    submit(&t, &s, 2);
    c.confirm_milestone(&t.buyer, &s, &2);
    assert_eq!(
        tok.balance(&t.supplier) - before,
        TOTAL / 4 + COLLATERAL / 4
    );
    assert_eq!(c.get_locked_collateral(&s), 0);
    assert_eq!(c.get_shipment(&s).status, ShipmentStatus::Completed);
}

#[test]
fn test_disputed_milestone_does_not_release_collateral() {
    let t = setup();
    let c = client(&t);
    let s = create_with_collateral(&t, "COL-DISP");
    c.set_incremental_collateral(&t.buyer, &s, &true);

    dispute(&t, &t.buyer, &s, 1);
    c.resolve_dispute(&t.arbiter, &s, &1, &true, &None);
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL);

    submit(&t, &s, 0);
    c.confirm_milestone(&t.buyer, &s, &0);
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL * 3 / 4);
}

#[test]
fn test_remaining_collateral_forfeited_on_buyer_cancel() {
    let t = setup();
    let c = client(&t);
    let tok = token::Client::new(&t.env, &t.token_id);
    let s = create_with_collateral(&t, "COL-CANCEL");
    c.set_incremental_collateral(&t.buyer, &s, &true);

    submit(&t, &s, 0);
    c.confirm_milestone(&t.buyer, &s, &0);
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL * 3 / 4);

    let supplier_before = tok.balance(&t.supplier);
    c.cancel_shipment(&t.buyer, &s);
    // Supplier gets nothing more; the remaining 75% follows the existing forfeit rule.
    assert_eq!(tok.balance(&t.supplier), supplier_before);
}

#[test]
fn test_incremental_collateral_can_be_disabled_before_payout() {
    let t = setup();
    let c = client(&t);
    let s = create_with_collateral(&t, "COL-OFF");
    c.set_incremental_collateral(&t.buyer, &s, &true);
    c.set_incremental_collateral(&t.buyer, &s, &false);
    assert!(!c.get_incremental_collateral(&s));

    submit(&t, &s, 0);
    c.confirm_milestone(&t.buyer, &s, &0);
    assert_eq!(c.get_locked_collateral(&s), COLLATERAL);
}

#[test]
#[should_panic(expected = "collateral mode can only change before any payout")]
fn test_incremental_collateral_locked_after_payout() {
    let t = setup();
    let c = client(&t);
    let s = create_with_collateral(&t, "COL-LATE");
    submit(&t, &s, 0);
    c.confirm_milestone(&t.buyer, &s, &0);
    c.set_incremental_collateral(&t.buyer, &s, &true);
}

#[test]
#[should_panic(expected = "shipment has no supplier collateral")]
fn test_incremental_collateral_requires_collateral() {
    let t = setup();
    let s = create_basic(&t, "COL-NONE", &t.buyer);
    client(&t).set_incremental_collateral(&t.buyer, &s, &true);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_incremental_collateral_supplier_cannot_enable() {
    let t = setup();
    let s = create_with_collateral(&t, "COL-SUP");
    client(&t).set_incremental_collateral(&t.supplier, &s, &true);
}

// ============================================================
// #517 – MERGE ADJACENT PENDING MILESTONES
// ============================================================

#[test]
fn test_merge_happy_path() {
    let t = setup();
    let c = client(&t);
    let s = create_basic(&t, "MERGE-OK", &t.buyer);

    c.propose_milestone_merge(&t.supplier, &s, &0);
    // Nothing changes until the counterparty approves.
    assert_eq!(c.get_shipment(&s).milestones.len(), 3);
    assert_eq!(
        c.get_milestone_merge_proposal(&s),
        Some(MilestoneMergeProposal {
            first_index: 0,
            proposer: t.supplier.clone(),
        })
    );

    c.approve_milestone_merge(&t.buyer, &s, &0);
    let shipment = c.get_shipment(&s);
    assert_eq!(shipment.milestones.len(), 2);
    let m0 = shipment.milestones.get(0).unwrap();
    let m1 = shipment.milestones.get(1).unwrap();
    assert_eq!(m0.payment_percent, 75);
    assert_eq!(m0.name, String::from_str(&t.env, "Goods Dispatched"));
    assert_eq!(m1.payment_percent, 25);
    assert_eq!(m1.name, String::from_str(&t.env, "Delivered"));
    assert_eq!(m0.payment_percent + m1.payment_percent, 100);
    assert_eq!(c.get_milestone_merge_proposal(&s), None);

    // Recorded in the amendment log.
    let log = c.get_amendment_log(&s, &0);
    assert_eq!(log.len(), 1);
    let entry = log.get(0).unwrap();
    assert_eq!(entry.old_payment_percent, 25);
    assert_eq!(entry.new_payment_percent, 75);
    assert_eq!(entry.proposer, t.supplier);

    // Audit log records the merge.
    let last = shipment
        .audit_log
        .get(shipment.audit_log.len() - 1)
        .unwrap();
    assert_eq!(last.action, Symbol::new(&t.env, "milestones_merged"));
}

#[test]
fn test_merge_uses_later_deadline_and_shifts_later_milestones() {
    let t = setup();
    let c = client(&t);
    let mut milestones = build_milestones(&t.env);
    let mut m0 = milestones.get(0).unwrap();
    m0.deadline_ledger = 1_000;
    milestones.set(0, m0);
    let mut m1 = milestones.get(1).unwrap();
    m1.deadline_ledger = 2_000;
    milestones.set(1, m1);
    let s = sid(&t.env, "MERGE-DL");
    c.create_shipment(
        &s,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &milestones,
        &default_options(&t.env),
    );

    // Later milestone already has proof; it must shift down intact.
    submit(&t, &s, 2);

    c.propose_milestone_merge(&t.buyer, &s, &0);
    c.approve_milestone_merge(&t.supplier, &s, &0);

    let shipment = c.get_shipment(&s);
    assert_eq!(shipment.milestones.get(0).unwrap().deadline_ledger, 2_000);
    let shifted = shipment.milestones.get(1).unwrap();
    assert_eq!(shifted.status, MilestoneStatus::ProofSubmitted);

    // The shifted milestone can still be confirmed at its new index.
    let tok = token::Client::new(&t.env, &t.token_id);
    let before = tok.balance(&t.supplier);
    c.confirm_milestone(&t.buyer, &s, &1);
    assert_eq!(tok.balance(&t.supplier) - before, TOTAL / 4);
}

#[test]
fn test_merge_middle_pair() {
    let t = setup();
    let c = client(&t);
    let s = create_basic(&t, "MERGE-MID", &t.buyer);
    c.propose_milestone_merge(&t.buyer, &s, &1);
    c.approve_milestone_merge(&t.supplier, &s, &1);
    let shipment = c.get_shipment(&s);
    assert_eq!(shipment.milestones.len(), 2);
    assert_eq!(shipment.milestones.get(0).unwrap().payment_percent, 25);
    assert_eq!(shipment.milestones.get(1).unwrap().payment_percent, 75);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_merge_last_milestone_rejected() {
    let t = setup();
    let s = create_basic(&t, "MERGE-LAST", &t.buyer);
    client(&t).propose_milestone_merge(&t.buyer, &s, &2);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_merge_out_of_range_rejected() {
    let t = setup();
    let s = create_basic(&t, "MERGE-OOR", &t.buyer);
    client(&t).propose_milestone_merge(&t.buyer, &s, &u32::MAX);
}

#[test]
#[should_panic(expected = "only pending milestones without proof can be merged")]
fn test_merge_rejects_milestone_with_proof() {
    let t = setup();
    let s = create_basic(&t, "MERGE-PROOF", &t.buyer);
    submit(&t, &s, 1);
    client(&t).propose_milestone_merge(&t.buyer, &s, &0);
}

#[test]
#[should_panic(expected = "cannot merge a milestone with an advance")]
fn test_merge_rejects_milestone_with_advance() {
    let t = setup();
    let c = client(&t);
    let s = create_basic(&t, "MERGE-ADV", &t.buyer);
    c.request_advance(&t.supplier, &s, &1, &10);
    c.propose_milestone_merge(&t.buyer, &s, &0);
}

#[test]
#[should_panic(expected = "only pending milestones without proof can be merged")]
fn test_merge_rejects_disputed_milestone() {
    let t = setup();
    let s = create_basic(&t, "MERGE-DISP", &t.buyer);
    dispute(&t, &t.buyer, &s, 0);
    client(&t).propose_milestone_merge(&t.supplier, &s, &0);
}

#[test]
#[should_panic(expected = "merge must be approved by the counterparty")]
fn test_merge_proposer_cannot_self_approve() {
    let t = setup();
    let c = client(&t);
    let s = create_basic(&t, "MERGE-SELF", &t.buyer);
    c.propose_milestone_merge(&t.buyer, &s, &0);
    c.approve_milestone_merge(&t.buyer, &s, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_merge_stranger_cannot_propose() {
    let t = setup();
    let stranger = Address::generate(&t.env);
    let s = create_basic(&t, "MERGE-STR", &t.buyer);
    client(&t).propose_milestone_merge(&stranger, &s, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_merge_stranger_cannot_approve() {
    let t = setup();
    let c = client(&t);
    let stranger = Address::generate(&t.env);
    let s = create_basic(&t, "MERGE-STR2", &t.buyer);
    c.propose_milestone_merge(&t.buyer, &s, &0);
    c.approve_milestone_merge(&stranger, &s, &0);
}

#[test]
#[should_panic(expected = "merge index does not match proposal")]
fn test_merge_approval_index_mismatch() {
    let t = setup();
    let c = client(&t);
    let s = create_basic(&t, "MERGE-MIS", &t.buyer);
    c.propose_milestone_merge(&t.buyer, &s, &0);
    c.approve_milestone_merge(&t.supplier, &s, &1);
}

#[test]
#[should_panic(expected = "no pending milestone merge proposal")]
fn test_merge_approve_without_proposal() {
    let t = setup();
    let s = create_basic(&t, "MERGE-NONE", &t.buyer);
    client(&t).approve_milestone_merge(&t.supplier, &s, &0);
}

#[test]
#[should_panic(expected = "only pending milestones without proof can be merged")]
fn test_merge_revalidated_at_approval() {
    let t = setup();
    let c = client(&t);
    let s = create_basic(&t, "MERGE-REVAL", &t.buyer);
    c.propose_milestone_merge(&t.buyer, &s, &0);
    // State changes between proposal and approval.
    submit(&t, &s, 0);
    c.approve_milestone_merge(&t.supplier, &s, &0);
}
