#![cfg(test)]

//! #740 consolidated disputes, #742 arbiter-declared fee rates and
//! #752 payout reference memos.

extern crate std;

use super::*;
use crate::test_common::{
    build_milestones, capture_events, default_options, setup, single_buyer_vec, TestSetup,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env, String, Symbol, TryFromVal};

const TOTAL: i128 = 1_000_000;
/// Milestone 0 is 25% of TOTAL.
const M0: i128 = 250_000;

fn sid(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn create(t: &TestSetup, id: &String, supplier: &Address, arbiter: &Address, fee_bps: u32) {
    let mut options = default_options(&t.env);
    options.arbiter_fee_bps = fee_bps;
    client(t).create_shipment(
        id,
        &single_buyer_vec(&t.env, &t.buyer),
        supplier,
        &t.logistics,
        arbiter,
        &t.token_id,
        &TOTAL,
        &build_milestones(&t.env),
        &options,
    );
}

fn open_dispute(t: &TestSetup, id: &String, supplier: &Address) {
    let c = client(t);
    c.submit_proof(
        supplier,
        id,
        &0u32,
        &String::from_str(&t.env, "QmProof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    c.raise_dispute(&t.buyer, id, &0u32);
}

fn balance(t: &TestSetup, who: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(who)
}

fn refs(env: &Env, ids: &[&String]) -> Vec<DisputeEntry> {
    let mut v = Vec::new(env);
    for id in ids {
        v.push_back(DisputeEntry {
            shipment_id: (*id).clone(),
            milestone_index: 0,
        });
    }
    v
}

/// Two disputed shipments between the same buyer and supplier.
fn two_disputes(t: &TestSetup, fee_bps: u32) -> (String, String) {
    let s1 = sid(&t.env, "c1");
    let s2 = sid(&t.env, "c2");
    create(t, &s1, &t.supplier, &t.arbiter, fee_bps);
    create(t, &s2, &t.supplier, &t.arbiter, fee_bps);
    open_dispute(t, &s1, &t.supplier);
    open_dispute(t, &s2, &t.supplier);
    (s1, s2)
}

fn approved_consolidation(t: &TestSetup, s1: &String, s2: &String) -> u64 {
    let c = client(t);
    let id = c.propose_dispute_consolidation(&t.arbiter, &refs(&t.env, &[s1, s2]));
    c.approve_dispute_consolidation(&t.buyer, &id);
    c.approve_dispute_consolidation(&t.supplier, &id);
    id
}

// ── #740 Consolidated disputes ───────────────────────────────────────────

#[test]
fn test_consolidated_ruling_updates_every_member() {
    let t = setup();
    let c = client(&t);
    let (s1, s2) = two_disputes(&t, 100);
    let id = approved_consolidation(&t, &s1, &s2);

    let supplier_before = balance(&t, &t.supplier);
    c.resolve_consolidated_dispute(&t.arbiter, &id, &true);

    for s in [&s1, &s2] {
        let shipment = c.get_shipment(s);
        assert_ne!(
            shipment.milestones.get(0).unwrap().status,
            MilestoneStatus::Disputed
        );
        assert_eq!(shipment.open_dispute_count, 0);
    }
    assert!(balance(&t, &t.supplier) > supplier_before);
    assert!(c.get_dispute_consolidation(&id).unwrap().resolved);
}

#[test]
fn test_consolidated_buyer_ruling_resolves_every_member() {
    let t = setup();
    let c = client(&t);
    let (s1, s2) = two_disputes(&t, 0);
    let id = approved_consolidation(&t, &s1, &s2);

    let supplier_before = balance(&t, &t.supplier);
    c.resolve_consolidated_dispute(&t.arbiter, &id, &false);

    // A buyer ruling on a full dispute rejects the proof: each member milestone
    // returns to Pending and nothing is paid to the supplier.
    for s in [&s1, &s2] {
        let shipment = c.get_shipment(s);
        assert_eq!(
            shipment.milestones.get(0).unwrap().status,
            MilestoneStatus::Pending
        );
        assert_eq!(shipment.open_dispute_count, 0);
    }
    assert_eq!(balance(&t, &t.supplier), supplier_before);
}

#[test]
fn test_consolidated_fee_priced_on_combined_contested_amount() {
    let t = setup();
    let c = client(&t);
    // Tier: 200 bps once the contested amount reaches 400_000. Each member alone
    // (250_000) stays on the 100 bps base rate; combined (500_000) crosses it.
    t.env.as_contract(&t.contract_id, || {
        let tiers: Vec<(i128, u32)> = vec![&t.env, (400_000i128, 200u32)];
        t.env
            .storage()
            .persistent()
            .set(&DataKeyExt::ArbiterFeeTiers, &tiers);
    });
    let (s1, s2) = two_disputes(&t, 100);
    let id = approved_consolidation(&t, &s1, &s2);

    let arbiter_before = balance(&t, &t.arbiter);
    c.resolve_consolidated_dispute(&t.arbiter, &id, &true);
    // 2% of the combined 500_000, charged once at the combined-amount rate.
    assert_eq!(balance(&t, &t.arbiter) - arbiter_before, 10_000);
}

#[test]
#[should_panic(expected = "disputes must be between the same buyer and supplier")]
fn test_cannot_consolidate_different_suppliers() {
    let t = setup();
    let other_supplier = Address::generate(&t.env);
    let s1 = sid(&t.env, "d1");
    let s2 = sid(&t.env, "d2");
    create(&t, &s1, &t.supplier, &t.arbiter, 0);
    create(&t, &s2, &other_supplier, &t.arbiter, 0);
    open_dispute(&t, &s1, &t.supplier);
    open_dispute(&t, &s2, &other_supplier);
    client(&t).propose_dispute_consolidation(&t.arbiter, &refs(&t.env, &[&s1, &s2]));
}

#[test]
#[should_panic(expected = "milestone is not in disputed status")]
fn test_cannot_consolidate_undisputed_milestone() {
    let t = setup();
    let s1 = sid(&t.env, "u1");
    let s2 = sid(&t.env, "u2");
    create(&t, &s1, &t.supplier, &t.arbiter, 0);
    create(&t, &s2, &t.supplier, &t.arbiter, 0);
    open_dispute(&t, &s1, &t.supplier);
    client(&t).propose_dispute_consolidation(&t.arbiter, &refs(&t.env, &[&s1, &s2]));
}

#[test]
#[should_panic(expected = "consolidation not approved by both parties")]
fn test_resolve_requires_both_approvals() {
    let t = setup();
    let c = client(&t);
    let (s1, s2) = two_disputes(&t, 0);
    let id = c.propose_dispute_consolidation(&t.arbiter, &refs(&t.env, &[&s1, &s2]));
    c.approve_dispute_consolidation(&t.buyer, &id);
    c.resolve_consolidated_dispute(&t.arbiter, &id, &true);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_third_party_cannot_approve() {
    let t = setup();
    let c = client(&t);
    let (s1, s2) = two_disputes(&t, 0);
    let id = c.propose_dispute_consolidation(&t.arbiter, &refs(&t.env, &[&s1, &s2]));
    c.approve_dispute_consolidation(&Address::generate(&t.env), &id);
}

#[test]
#[should_panic(expected = "dispute is consolidated")]
fn test_member_cannot_be_resolved_individually_once_approved() {
    let t = setup();
    let c = client(&t);
    let (s1, s2) = two_disputes(&t, 0);
    approved_consolidation(&t, &s1, &s2);
    c.resolve_dispute(&t.arbiter, &s1, &0u32, &true, &None);
}

#[test]
fn test_unapproved_proposal_does_not_block_individual_resolution() {
    let t = setup();
    let c = client(&t);
    let (s1, s2) = two_disputes(&t, 0);
    let id = c.propose_dispute_consolidation(&t.arbiter, &refs(&t.env, &[&s1, &s2]));
    c.resolve_dispute(&t.arbiter, &s1, &0u32, &true, &None);
    // The proposal can no longer be approved: a member is closed.
    assert!(c
        .try_approve_dispute_consolidation(&t.buyer, &id)
        .is_err());
}

// ── #742 Arbiter-declared fee rate ───────────────────────────────────────

#[test]
#[should_panic(expected = "arbiter fee rate exceeds cap")]
fn test_arbiter_rate_above_cap_rejected() {
    let t = setup();
    let c = client(&t);
    c.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    c.set_max_arbiter_fee_bps(&t.buyer, &500u32);
    c.set_arbiter_fee_rate(&t.arbiter, &600u32);
}

#[test]
#[should_panic(expected = "arbiter is not in the pool")]
fn test_non_pool_arbiter_cannot_declare_rate() {
    let t = setup();
    let c = client(&t);
    c.set_max_arbiter_fee_bps(&t.buyer, &500u32);
    c.set_arbiter_fee_rate(&t.arbiter, &100u32);
}

#[test]
fn test_rate_reported_in_arbiter_stats() {
    let t = setup();
    let c = client(&t);
    c.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    c.set_max_arbiter_fee_bps(&t.buyer, &500u32);
    assert_eq!(c.get_arbiter_stats(&t.arbiter).fee_rate_bps, 0);
    c.set_arbiter_fee_rate(&t.arbiter, &300u32);
    assert_eq!(c.get_arbiter_stats(&t.arbiter).fee_rate_bps, 300);
    assert_eq!(c.get_arbiter_fee_rate(&t.arbiter), 300);
}

#[test]
fn test_rate_snapshotted_at_assignment() {
    let t = setup();
    let c = client(&t);
    c.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    c.set_max_arbiter_fee_bps(&t.buyer, &500u32);
    c.set_arbiter_fee_rate(&t.arbiter, &300u32);

    let s = sid(&t.env, "pool1");
    // Pool sentinel arbiter: the contract address.
    create(&t, &s, &t.supplier, &t.contract_id, 0);
    open_dispute(&t, &s, &t.supplier);
    let shipment = c.get_shipment(&s);
    assert_eq!(shipment.arbiter, t.arbiter);
    assert_eq!(shipment.arbiter_fee_bps, 300);

    // A later change does not apply to the in-flight dispute.
    c.set_arbiter_fee_rate(&t.arbiter, &500u32);
    assert_eq!(c.get_shipment(&s).arbiter_fee_bps, 300);

    let before = balance(&t, &t.arbiter);
    c.resolve_dispute(&t.arbiter, &s, &0u32, &true, &None);
    assert_eq!(balance(&t, &t.arbiter) - before, M0 * 300 / 10_000);
}

#[test]
fn test_undeclared_rate_keeps_shipment_rate() {
    let t = setup();
    let c = client(&t);
    c.add_arbiter_to_pool(&t.buyer, &t.arbiter);
    let mut options = default_options(&t.env);
    options.arbiter_fee_bps = 150;
    let s = sid(&t.env, "pool2");
    c.create_shipment(
        &s,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.contract_id,
        &t.token_id,
        &TOTAL,
        &build_milestones(&t.env),
        &options,
    );
    open_dispute(&t, &s, &t.supplier);
    assert_eq!(c.get_shipment(&s).arbiter_fee_bps, 150);
}

// ── #752 Payout reference memo ───────────────────────────────────────────

fn payout_reference_event(
    env: &Env,
    events: &soroban_sdk::Vec<crate::test_common::CapturedEvent>,
) -> Option<(u32, i128, String)> {
    let topic = Symbol::new(env, "payout_reference");
    for (_, topics, data) in events.iter() {
        if let Some(first) = topics.get(0) {
            if Symbol::try_from_val(env, &first).map_or(false, |s| s == topic) {
                return Some(<(u32, i128, String)>::try_from_val(env, &data).unwrap());
            }
        }
    }
    None
}

#[test]
fn test_release_event_carries_reference() {
    let t = setup();
    let c = client(&t);
    let s = sid(&t.env, "ref1");
    create(&t, &s, &t.supplier, &t.arbiter, 0);
    let reference = String::from_str(&t.env, "INV-2026-0042");
    c.set_payout_reference(&t.supplier, &s, &reference);
    assert_eq!(c.get_payout_reference(&s), Some(reference.clone()));

    c.submit_proof(
        &t.supplier,
        &s,
        &0u32,
        &String::from_str(&t.env, "QmProof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    let events = capture_events(&t.env, || c.confirm_milestone(&t.buyer, &s, &0u32));
    let (idx, amount, got) = payout_reference_event(&t.env, &events).expect("payout_reference event");
    assert_eq!(idx, 0);
    assert!(amount > 0);
    assert_eq!(got, reference);
}

#[test]
fn test_no_reference_event_when_unset() {
    let t = setup();
    let c = client(&t);
    let s = sid(&t.env, "ref2");
    create(&t, &s, &t.supplier, &t.arbiter, 0);
    c.submit_proof(
        &t.supplier,
        &s,
        &0u32,
        &String::from_str(&t.env, "QmProof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    let events = capture_events(&t.env, || c.confirm_milestone(&t.buyer, &s, &0u32));
    assert!(payout_reference_event(&t.env, &events).is_none());
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_only_supplier_sets_reference() {
    let t = setup();
    let s = sid(&t.env, "ref3");
    create(&t, &s, &t.supplier, &t.arbiter, 0);
    client(&t).set_payout_reference(&t.buyer, &s, &String::from_str(&t.env, "INV-1"));
}

#[test]
#[should_panic(expected = "payout reference too long")]
fn test_reference_length_bounded() {
    let t = setup();
    let s = sid(&t.env, "ref4");
    create(&t, &s, &t.supplier, &t.arbiter, 0);
    let long = String::from_str(
        &t.env,
        "0123456789012345678901234567890123456789012345678901234567890123456789",
    );
    client(&t).set_payout_reference(&t.supplier, &s, &long);
}

#[test]
fn test_empty_reference_clears() {
    let t = setup();
    let c = client(&t);
    let s = sid(&t.env, "ref5");
    create(&t, &s, &t.supplier, &t.arbiter, 0);
    c.set_payout_reference(&t.supplier, &s, &String::from_str(&t.env, "INV-1"));
    c.set_payout_reference(&t.supplier, &s, &String::from_str(&t.env, ""));
    assert_eq!(c.get_payout_reference(&s), None);
}
