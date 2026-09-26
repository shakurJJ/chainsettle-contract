#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{vec, String};

fn milestone(env: &Env, pct: u32, deadline_ledger: u32) -> Milestone {
    Milestone {
        name: String::from_str(env, "M"),
        payment_percent: pct,
        proof_hash: String::from_str(env, ""),
        status: MilestoneStatus::Pending,
        release_after_ledger: 0,
        proof_submitted_ledger: None,
        dispute_opened_ledger: None,
        deadline_ledger,
        penalty_bps_per_ledger: 0,
    }
}

/// Creates a two-milestone shipment whose milestones are due `first_in` and
/// `second_in` ledgers from now (0 = no deadline).
fn create_with_deadlines(
    t: &TestSetup,
    client: &ChainSettleContractClient,
    id: &str,
    first_in: u32,
    second_in: u32,
) -> String {
    let now = t.env.ledger().sequence();
    let at = |d: u32| if d == 0 { 0 } else { now + d };
    let shipment_id = String::from_str(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &vec![
            &t.env,
            milestone(&t.env, 50, at(first_in)),
            milestone(&t.env, 50, at(second_in)),
        ],
        &default_options(&t.env),
    );
    shipment_id
}

fn advance(t: &TestSetup, ledgers: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += ledgers);
}

#[test]
fn test_notice_period_defaults_to_zero_and_is_settable() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(client.get_pause_min_notice_ledgers(), 0);
    client.set_pause_min_notice_ledgers(&t.buyer, &100u32);
    assert_eq!(client.get_pause_min_notice_ledgers(), 100);
}

#[test]
#[should_panic]
fn test_non_admin_cannot_set_notice_period() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_pause_min_notice_ledgers(&t.supplier, &100u32);
}

#[test]
fn test_default_allows_pause_right_before_deadline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    advance(&t, 1_000);
    let shipment_id = create_with_deadlines(&t, &client, "def", 1, 0);
    client.request_shipment_pause(&t.supplier, &shipment_id);
    client.approve_shipment_pause(&t.buyer, &shipment_id);
    assert!(client.is_shipment_paused(&shipment_id));
}

#[test]
#[should_panic(expected = "pause request within minimum notice period of next milestone deadline")]
fn test_pause_within_notice_window_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    advance(&t, 1_000);
    client.set_pause_min_notice_ledgers(&t.buyer, &100u32);
    let shipment_id = create_with_deadlines(&t, &client, "near", 50, 500);
    client.request_shipment_pause(&t.supplier, &shipment_id);
}

#[test]
fn test_pause_outside_notice_window_succeeds() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    advance(&t, 1_000);
    client.set_pause_min_notice_ledgers(&t.buyer, &100u32);
    let shipment_id = create_with_deadlines(&t, &client, "far", 100, 500);
    client.request_shipment_pause(&t.supplier, &shipment_id);
    client.approve_shipment_pause(&t.buyer, &shipment_id);
    assert!(client.is_shipment_paused(&shipment_id));
}

#[test]
fn test_notice_window_applies_to_buyer_requests_too() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    advance(&t, 1_000);
    client.set_pause_min_notice_ledgers(&t.buyer, &100u32);
    let shipment_id = create_with_deadlines(&t, &client, "buyer", 10, 0);
    assert!(client
        .try_request_shipment_pause(&t.buyer, &shipment_id)
        .is_err());
}

#[test]
fn test_shipment_without_deadlines_is_unaffected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_pause_min_notice_ledgers(&t.buyer, &100u32);
    let shipment_id = create_with_deadlines(&t, &client, "none", 0, 0);
    client.request_shipment_pause(&t.supplier, &shipment_id);
}

#[test]
fn test_passed_deadline_is_not_upcoming() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    advance(&t, 1_000);
    client.set_pause_min_notice_ledgers(&t.buyer, &100u32);
    // First deadline already passed; next upcoming one is far away.
    let shipment_id = create_with_deadlines(&t, &client, "passed", 10, 500);
    advance(&t, 20);
    client.request_shipment_pause(&t.supplier, &shipment_id);
}
