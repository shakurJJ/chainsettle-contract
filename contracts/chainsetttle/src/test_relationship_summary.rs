// #751 — Bilateral trading relationship summary query.
//
// `get_relationship_summary(buyer, supplier)` returns pair-level shipment
// counts, volume per token, dispute rate and average confirmation time,
// maintained in O(1) on completion, cancellation, dispute resolution and
// buyer confirmation.

#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, String, Symbol,
};

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn create(
    t: &TestSetup,
    id: &str,
    buyer: &Address,
    supplier: &Address,
    token: &Address,
    total: i128,
) -> String {
    let shipment_id = String::from_str(&t.env, id);
    client(t).create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, buyer),
        supplier,
        &t.logistics,
        &t.arbiter,
        token,
        &total,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    shipment_id
}

fn advance(t: &TestSetup, ledgers: u32) {
    let now = t.env.ledger().sequence();
    t.env.ledger().with_mut(|l| l.sequence_number = now + ledgers);
}

fn submit(t: &TestSetup, supplier: &Address, shipment_id: &String, index: u32, proof: &str) {
    client(t).submit_proof(
        supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, proof),
        &Symbol::new(&t.env, "ipfs"),
    );
}

/// Submit proof, wait `latency` ledgers, then confirm as `buyer`.
fn deliver(
    t: &TestSetup,
    buyer: &Address,
    supplier: &Address,
    shipment_id: &String,
    index: u32,
    latency: u32,
) {
    submit(t, supplier, shipment_id, index, "proof");
    advance(t, latency);
    client(t).confirm_milestone(buyer, shipment_id, &index);
}

fn second_token(t: &TestSetup) -> Address {
    let admin = Address::generate(&t.env);
    let token_id = t.env.register_stellar_asset_contract_v2(admin).address();
    token::StellarAssetClient::new(&t.env, &token_id).mint(&t.buyer, &10_000_000_000);
    token_id
}

fn assert_empty(summary: &RelationshipSummary) {
    assert_eq!(summary.shipment_count, 0);
    assert_eq!(summary.completed_count, 0);
    assert_eq!(summary.cancelled_count, 0);
    assert_eq!(summary.disputed_shipment_count, 0);
    assert_eq!(summary.disputes_resolved, 0);
    assert_eq!(summary.dispute_rate_bps, 0);
    assert_eq!(summary.confirmation_count, 0);
    assert_eq!(summary.avg_confirmation_ledgers, 0);
    assert!(summary.volume_by_token.is_empty());
}

#[test]
fn test_unknown_pair_returns_zeroed_summary() {
    let t = setup();
    assert_empty(&client(&t).get_relationship_summary(&t.buyer, &t.supplier));
}

#[test]
fn test_query_is_read_only() {
    let t = setup();
    let key = DataKeyExt5::RelationshipSummary(t.buyer.clone(), t.supplier.clone());

    // Querying a pair with no history must not create storage.
    client(&t).get_relationship_summary(&t.buyer, &t.supplier);
    let stored = t
        .env
        .as_contract(&t.contract_id, || t.env.storage().persistent().has(&key));
    assert!(!stored);

    // Querying an existing pair must not change it.
    let id = create(&t, "RS-RO", &t.buyer, &t.supplier, &t.token_id, 1_000_000);
    for i in 0..3u32 {
        deliver(&t, &t.buyer, &t.supplier, &id, i, 1);
    }
    let first = client(&t).get_relationship_summary(&t.buyer, &t.supplier);
    let second = client(&t).get_relationship_summary(&t.buyer, &t.supplier);
    assert_eq!(first, second);
    let raw: RelationshipSummary = t
        .env
        .as_contract(&t.contract_id, || t.env.storage().persistent().get(&key).unwrap());
    assert_eq!(raw, first);
}

#[test]
fn test_counters_match_scripted_multi_shipment_history() {
    let t = setup();
    let token2 = second_token(&t);
    let (b, s) = (t.buyer.clone(), t.supplier.clone());

    // S1: clean completion, three confirmations 4 ledgers after proof.
    let s1 = create(&t, "RS-S1", &b, &s, &t.token_id, 1_000_000);
    for i in 0..3u32 {
        deliver(&t, &b, &s, &s1, i, 4);
    }
    let sum = client(&t).get_relationship_summary(&b, &s);
    assert_eq!(sum.shipment_count, 1);
    assert_eq!(sum.completed_count, 1);
    assert_eq!(sum.confirmation_count, 3);
    assert_eq!(sum.avg_confirmation_ledgers, 4);

    // S2: dispute on milestone 0 won by the buyer (proof rejected), then the
    // supplier resubmits and every milestone is confirmed 2 ledgers after proof.
    let s2 = create(&t, "RS-S2", &b, &s, &t.token_id, 1_000_000);
    submit(&t, &s, &s2, 0, "bad-proof");
    client(&t).raise_dispute(&b, &s2, &0u32);
    client(&t).resolve_dispute(&t.arbiter, &s2, &0u32, &false, &None);
    submit(&t, &s, &s2, 0, "good-proof");
    advance(&t, 2);
    client(&t).confirm_milestone(&b, &s2, &0u32);
    deliver(&t, &b, &s, &s2, 1, 2);
    deliver(&t, &b, &s, &s2, 2, 2);

    // S3: one milestone delivered (1 ledger latency), then cancelled.
    let s3 = create(&t, "RS-S3", &b, &s, &t.token_id, 1_000_000);
    deliver(&t, &b, &s, &s3, 0, 1);
    let mid = client(&t).get_relationship_summary(&b, &s);
    assert_eq!(mid.shipment_count, 2, "active shipments are not counted yet");
    client(&t).cancel_shipment(&b, &s3);

    // S4 (second token): dispute on milestone 0 won by the supplier, the rest
    // confirmed 6 ledgers after proof.
    let s4 = create(&t, "RS-S4", &b, &s, &token2, 500_000);
    submit(&t, &s, &s4, 0, "proof");
    client(&t).raise_dispute(&b, &s4, &0u32);
    client(&t).resolve_dispute(&t.arbiter, &s4, &0u32, &true, &None);
    deliver(&t, &b, &s, &s4, 1, 6);
    deliver(&t, &b, &s, &s4, 2, 6);

    let sum = client(&t).get_relationship_summary(&b, &s);
    assert_eq!(sum.shipment_count, 4);
    assert_eq!(sum.completed_count, 3);
    assert_eq!(sum.cancelled_count, 1);
    assert_eq!(sum.disputed_shipment_count, 2);
    assert_eq!(sum.disputes_resolved, 2);
    assert_eq!(sum.buyer_dispute_wins, 1);
    assert_eq!(sum.supplier_dispute_wins, 1);
    assert_eq!(sum.dispute_rate_bps, 5_000);
    // Confirmations: S1 3x4, S2 3x2, S3 1x1, S4 2x6 (milestone 0 was resolved,
    // not confirmed).
    assert_eq!(sum.confirmation_count, 9);
    assert_eq!(sum.total_confirmation_ledgers, 12 + 6 + 1 + 12);
    assert_eq!(sum.avg_confirmation_ledgers, 31 / 9);
    // Volume: S1 + S2 in full, S3's delivered milestone; S4 in token2.
    assert_eq!(sum.volume_by_token.len(), 2);
    assert_eq!(sum.volume_by_token.get(t.token_id.clone()), Some(2_250_000));
    assert_eq!(sum.volume_by_token.get(token2.clone()), Some(500_000));
}

#[test]
fn test_pairs_are_tracked_independently() {
    let t = setup();
    let other_supplier = Address::generate(&t.env);

    let a = create(&t, "RS-A", &t.buyer, &t.supplier, &t.token_id, 1_000_000);
    for i in 0..3u32 {
        deliver(&t, &t.buyer, &t.supplier, &a, i, 3);
    }
    let b = create(&t, "RS-B", &t.buyer2, &t.supplier, &t.token_id, 400_000);
    client(&t).cancel_shipment(&t.buyer2, &b);
    let c = create(&t, "RS-C", &t.buyer, &other_supplier, &t.token_id, 200_000);
    for i in 0..3u32 {
        deliver(&t, &t.buyer, &other_supplier, &c, i, 1);
    }

    let ab = client(&t).get_relationship_summary(&t.buyer, &t.supplier);
    assert_eq!(ab.shipment_count, 1);
    assert_eq!(ab.completed_count, 1);
    assert_eq!(ab.avg_confirmation_ledgers, 3);
    assert_eq!(ab.volume_by_token.get(t.token_id.clone()), Some(1_000_000));

    let b2s = client(&t).get_relationship_summary(&t.buyer2, &t.supplier);
    assert_eq!(b2s.shipment_count, 1);
    assert_eq!(b2s.cancelled_count, 1);
    assert_eq!(b2s.confirmation_count, 0);
    assert!(b2s.volume_by_token.is_empty());

    let bo = client(&t).get_relationship_summary(&t.buyer, &other_supplier);
    assert_eq!(bo.shipment_count, 1);
    assert_eq!(bo.volume_by_token.get(t.token_id.clone()), Some(200_000));

    // The pair key is directional: (supplier, buyer) is not the same pair.
    assert_empty(&client(&t).get_relationship_summary(&t.supplier, &t.buyer));
}

#[test]
fn test_open_dispute_counts_shipment_as_disputed_only_once_closed() {
    let t = setup();
    let id = create(&t, "RS-DISP", &t.buyer, &t.supplier, &t.token_id, 1_000_000);
    submit(&t, &t.supplier, &id, 0, "proof");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    // Two resolutions on the same shipment count as one disputed shipment.
    client(&t).resolve_dispute(&t.arbiter, &id, &0u32, &false, &None);
    submit(&t, &t.supplier, &id, 0, "proof-2");
    client(&t).raise_dispute(&t.buyer, &id, &0u32);
    client(&t).resolve_dispute(&t.arbiter, &id, &0u32, &true, &None);

    let open = client(&t).get_relationship_summary(&t.buyer, &t.supplier);
    assert_eq!(open.disputes_resolved, 2);
    assert_eq!(open.shipment_count, 0);
    assert_eq!(open.dispute_rate_bps, 0);

    deliver(&t, &t.buyer, &t.supplier, &id, 1, 1);
    deliver(&t, &t.buyer, &t.supplier, &id, 2, 1);

    let closed = client(&t).get_relationship_summary(&t.buyer, &t.supplier);
    assert_eq!(closed.shipment_count, 1);
    assert_eq!(closed.disputed_shipment_count, 1);
    assert_eq!(closed.dispute_rate_bps, 10_000);
    assert_eq!(closed.buyer_dispute_wins, 1);
    assert_eq!(closed.supplier_dispute_wins, 1);
}
