//! #631 supplier on-time milestone rate metric.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{
    capture_events, default_options, has_event_topic, setup, single_buyer_vec, TestSetup,
};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{vec, String, Symbol};

fn milestone_with_deadline(env: &Env, pct: u32, deadline_ledger: u32) -> Milestone {
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
        not_before_ledger: 0,
    }
}

fn ipfs(env: &Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

fn proof(env: &Env) -> String {
    String::from_str(env, "QmProof")
}

fn create_two_milestones(
    t: &TestSetup,
    client: &ChainSettleContractClient,
    id: &str,
    first_deadline: u32,
    second_deadline: u32,
) -> String {
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
            milestone_with_deadline(&t.env, 50, first_deadline),
            milestone_with_deadline(&t.env, 50, second_deadline),
        ],
        &default_options(&t.env),
    );
    shipment_id
}

#[test]
fn test_timeliness_starts_at_zero() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let timeliness = client.get_supplier_timeliness(&t.supplier);
    assert_eq!(timeliness.on_time, 0);
    assert_eq!(timeliness.late, 0);
}

#[test]
fn test_on_time_confirmation_increments_on_time() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "ontime", now + 100, 0);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &ipfs(&t.env));
    client.confirm_milestone(&t.buyer, &id, &0u32);
    let timeliness = client.get_supplier_timeliness(&t.supplier);
    assert_eq!(timeliness.on_time, 1);
    assert_eq!(timeliness.late, 0);
}

#[test]
fn test_late_confirmation_increments_late() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "late", now + 5, 0);
    t.env.ledger().with_mut(|l| l.sequence_number = now + 10);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &ipfs(&t.env));
    client.confirm_milestone(&t.buyer, &id, &0u32);
    let timeliness = client.get_supplier_timeliness(&t.supplier);
    assert_eq!(timeliness.on_time, 0);
    assert_eq!(timeliness.late, 1);
}

#[test]
fn test_proof_exactly_at_deadline_counts_on_time() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "exact", now + 5, 0);
    t.env.ledger().with_mut(|l| l.sequence_number = now + 5);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &ipfs(&t.env));
    client.confirm_milestone(&t.buyer, &id, &0u32);
    let timeliness = client.get_supplier_timeliness(&t.supplier);
    assert_eq!(timeliness.on_time, 1);
    assert_eq!(timeliness.late, 0);
}

#[test]
fn test_milestone_without_deadline_is_ignored() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "nodeadline", now + 100, 0);
    // Second milestone has no deadline: confirming it must not move counters.
    client.submit_proof(&t.supplier, &id, &1u32, &proof(&t.env), &ipfs(&t.env));
    client.confirm_milestone(&t.buyer, &id, &1u32);
    let timeliness = client.get_supplier_timeliness(&t.supplier);
    assert_eq!(timeliness.on_time, 0);
    assert_eq!(timeliness.late, 0);
}

#[test]
fn test_confirmation_emits_timeliness_event() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "event", now + 100, 0);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &ipfs(&t.env));
    let events = capture_events(&t.env, || {
        client.confirm_milestone(&t.buyer, &id, &0u32);
    });
    assert!(has_event_topic(&t.env, &events, "timeliness_recorded"));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_buyer_cannot_confirm_for_timeliness() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "auth", now + 100, 0);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &ipfs(&t.env));
    let outsider = Address::generate(&t.env);
    client.confirm_milestone(&outsider, &id, &0u32);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_confirm_invalid_index_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let now = t.env.ledger().sequence();
    let id = create_two_milestones(&t, &client, "badidx", now + 100, 0);
    client.submit_proof(&t.supplier, &id, &0u32, &proof(&t.env), &ipfs(&t.env));
    client.confirm_milestone(&t.buyer, &id, &9u32);
}
