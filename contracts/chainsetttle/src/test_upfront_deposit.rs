// #738 — Upfront buyer deposit released when the supplier accepts terms.
//
// `upfront_deposit_bps` of the shipment value is released to the supplier on
// `accept_shipment_terms`, is capped by `max_advance_percent`, is deducted pro
// rata from milestone payouts, and is excluded from buyer cancellation refunds.

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
const GROSS: [i128; 3] = [250_000, 500_000, 250_000];

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn deposit_ext(bps: u32) -> ShipmentOptionsExt {
    ShipmentOptionsExt {
        upfront_deposit_bps: bps,
        ..Default::default()
    }
}

fn create_with(t: &TestSetup, id: &str, opts: &ShipmentOptions, bps: u32) -> String {
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
        &deposit_ext(bps),
    );
    shipment_id
}

fn create(t: &TestSetup, id: &str, bps: u32) -> String {
    create_with(t, id, &default_options(&t.env), bps)
}

fn submit(t: &TestSetup, shipment_id: &String, index: u32) {
    client(t).submit_proof(
        &t.supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
}

fn confirm(t: &TestSetup, shipment_id: &String, index: u32) {
    submit(t, shipment_id, index);
    client(t).confirm_milestone(&t.buyer, shipment_id, &index);
}

fn balance(t: &TestSetup, addr: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(addr)
}

// ── Defaults / backwards compatibility ──────────────────────────────────────

#[test]
fn test_accepting_terms_without_deposit_moves_nothing() {
    let t = setup();
    let id = create(&t, "UD-OFF", 0);
    assert_eq!(client(&t).get_terms_acceptance(&id), None);

    let released = client(&t).accept_shipment_terms(&t.supplier, &id);

    assert_eq!(released, 0);
    assert_eq!(balance(&t, &t.supplier), 0);
    assert_eq!(client(&t).get_shipment(&id).total_advanced_amount, 0);
    assert_eq!(
        client(&t).get_terms_acceptance(&id),
        Some((t.env.ledger().sequence(), 0))
    );
}

#[test]
fn test_plain_shipment_pays_full_milestones() {
    let t = setup();
    let id = create(&t, "UD-PLAIN", 0);
    client(&t).accept_shipment_terms(&t.supplier, &id);
    for i in 0..3u32 {
        confirm(&t, &id, i);
    }
    assert_eq!(balance(&t, &t.supplier), TOTAL);
}

// ── Release ─────────────────────────────────────────────────────────────────

#[test]
fn test_deposit_released_on_acceptance() {
    let t = setup();
    let id = create(&t, "UD-REL", 1_000); // 10%

    let released = client(&t).accept_shipment_terms(&t.supplier, &id);

    assert_eq!(released, 100_000);
    assert_eq!(balance(&t, &t.supplier), 100_000);
    assert_eq!(balance(&t, &t.contract_id), TOTAL - 100_000);
    assert_eq!(client(&t).get_shipment(&id).total_advanced_amount, 100_000);
    assert_eq!(
        client(&t).get_terms_acceptance(&id),
        Some((t.env.ledger().sequence(), 100_000))
    );
}

#[test]
fn test_deposit_releases_exactly_once() {
    let t = setup();
    let id = create(&t, "UD-ONCE", 1_000);
    client(&t).accept_shipment_terms(&t.supplier, &id);

    let second = client(&t).try_accept_shipment_terms(&t.supplier, &id);

    assert!(second.is_err());
    assert_eq!(balance(&t, &t.supplier), 100_000);
    assert_eq!(client(&t).get_shipment(&id).total_advanced_amount, 100_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_only_supplier_can_accept_terms() {
    let t = setup();
    let id = create(&t, "UD-AUTH", 1_000);
    client(&t).accept_shipment_terms(&t.buyer, &id);
}

#[test]
fn test_deposit_goes_to_financier_when_payout_assigned() {
    let t = setup();
    let id = create(&t, "UD-FIN", 1_000);
    let financier = Address::generate(&t.env);
    client(&t).assign_payout(&t.supplier, &id, &financier);

    client(&t).accept_shipment_terms(&t.supplier, &id);

    assert_eq!(balance(&t, &financier), 100_000);
    assert_eq!(balance(&t, &t.supplier), 0);
}

// ── Pro-rata deduction ──────────────────────────────────────────────────────

#[test]
fn test_deposit_deducted_pro_rata_and_payouts_sum_to_total() {
    let t = setup();
    let bps: u32 = 1_234;
    let id = create(&t, "UD-SUM", bps);
    let deposit = client(&t).accept_shipment_terms(&t.supplier, &id);

    let mut paid = deposit;
    for i in 0..3u32 {
        let before = balance(&t, &t.supplier);
        confirm(&t, &id, i);
        let payout = balance(&t, &t.supplier) - before;
        let gross = GROSS[i as usize];
        assert_eq!(payout, gross - gross * bps as i128 / 10_000, "milestone {}", i);
        paid += payout;
    }

    assert_eq!(paid, TOTAL);
    assert_eq!(balance(&t, &t.supplier), TOTAL);
    assert_eq!(balance(&t, &t.contract_id), 0);
    let shipment = client(&t).get_shipment(&id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.total_advanced_amount, 0);
    assert_eq!(shipment.released_amount, TOTAL);
}

#[test]
fn test_payout_preview_reflects_deposit_deduction() {
    let t = setup();
    let id = create(&t, "UD-PREV", 2_000);
    client(&t).accept_shipment_terms(&t.supplier, &id);
    let preview = client(&t).preview_milestone_payout(&id, &1u32);
    assert_eq!(preview.advance_deducted, GROSS[1] * 2_000 / 10_000);
}

#[test]
fn test_held_payout_still_deducts_deposit() {
    let t = setup();
    let mut opts = default_options(&t.env);
    opts.holdback_ledgers = 10;
    let id = create_with(&t, "UD-HELD", &opts, 1_000);
    client(&t).accept_shipment_terms(&t.supplier, &id);

    for i in 0..3u32 {
        confirm(&t, &id, i);
    }
    let now = t.env.ledger().sequence();
    t.env.ledger().with_mut(|l| l.sequence_number = now + 11);
    for i in 0..3u32 {
        client(&t).release_held_payment(&id, &i);
    }

    assert_eq!(balance(&t, &t.supplier), TOTAL);
    assert_eq!(balance(&t, &t.contract_id), 0);
}

// ── Cancellation ────────────────────────────────────────────────────────────

#[test]
fn test_deposit_excluded_from_buyer_cancellation_refund() {
    let t = setup();
    let buyer_start = balance(&t, &t.buyer);
    let id = create(&t, "UD-CANCEL", 1_000);
    client(&t).accept_shipment_terms(&t.supplier, &id);

    client(&t).cancel_shipment(&t.buyer, &id);

    assert_eq!(buyer_start - balance(&t, &t.buyer), 100_000);
    assert_eq!(balance(&t, &t.supplier), 100_000);
    assert_eq!(balance(&t, &t.contract_id), 0);
}

#[test]
fn test_cancellation_after_partial_delivery_keeps_unconsumed_deposit_with_supplier() {
    let t = setup();
    let buyer_start = balance(&t, &t.buyer);
    let id = create(&t, "UD-PCANCEL", 1_000);
    client(&t).accept_shipment_terms(&t.supplier, &id);
    confirm(&t, &id, 0); // pays 250_000 - 25_000

    client(&t).cancel_shipment(&t.buyer, &id);

    // Supplier keeps the whole deposit plus milestone 0's net payout; the
    // buyer gets back the rest of the escrow and nothing more.
    assert_eq!(balance(&t, &t.supplier), 100_000 + 225_000);
    assert_eq!(buyer_start - balance(&t, &t.buyer), 325_000);
    assert_eq!(balance(&t, &t.contract_id), 0);
}

#[test]
fn test_cancellation_before_acceptance_refunds_everything() {
    let t = setup();
    let buyer_start = balance(&t, &t.buyer);
    let id = create(&t, "UD-NOACC", 1_000);
    client(&t).cancel_shipment(&t.buyer, &id);
    assert_eq!(balance(&t, &t.buyer), buyer_start);
    assert_eq!(balance(&t, &t.supplier), 0);
}

// ── Caps & preconditions ────────────────────────────────────────────────────

#[test]
fn test_deposit_at_max_advance_percent_allowed() {
    let t = setup();
    // Default max_advance_percent is 30% == 3_000 bps.
    let id = create(&t, "UD-MAX", 3_000);
    assert_eq!(client(&t).accept_shipment_terms(&t.supplier, &id), 300_000);
}

#[test]
#[should_panic(expected = "UpfrontDepositExceedsMax")]
fn test_deposit_above_max_advance_percent_rejected() {
    let t = setup();
    create(&t, "UD-OVER", 3_001);
}

#[test]
#[should_panic(expected = "UpfrontDepositExceedsMax")]
fn test_deposit_cap_follows_admin_max_advance_percent() {
    let t = setup();
    client(&t).set_max_advance_percent(&t.buyer, &10u32);
    create(&t, "UD-ADMIN", 1_001);
}

#[test]
#[should_panic(expected = "upfront deposit must be released before the shipment progresses")]
fn test_deposit_cannot_be_released_after_work_starts() {
    let t = setup();
    let id = create(&t, "UD-PROG", 1_000);
    submit(&t, &id, 0);
    client(&t).accept_shipment_terms(&t.supplier, &id);
}

#[test]
#[should_panic(expected = "upfront deposit conflicts with an existing advance request")]
fn test_deposit_conflicts_with_pending_advance_request() {
    let t = setup();
    let id = create(&t, "UD-ADVREQ", 1_000);
    client(&t).request_advance(&t.supplier, &id, &1u32, &10u32);
    client(&t).accept_shipment_terms(&t.supplier, &id);
}

#[test]
#[should_panic(expected = "upfront deposit not supported for consortium shipments")]
fn test_deposit_rejected_for_consortium_shipments() {
    let t = setup();
    let mut opts = default_options(&t.env);
    let other = Address::generate(&t.env);
    opts.milestone_suppliers =
        soroban_sdk::vec![&t.env, t.supplier.clone(), other.clone(), t.supplier.clone()];
    create_with(&t, "UD-CONS", &opts, 1_000);
}
