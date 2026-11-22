#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{
    build_milestones, default_options, setup, single_buyer_vec, TestSetup,
};
use soroban_sdk::{testutils::Address as _, vec, Address, String, Symbol};

const TOTAL: i128 = 1_000_000_000;

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn split_parts(env: &Env, entries: &[(&str, u32)]) -> Vec<MilestoneSplitPart> {
    let mut parts = Vec::new(env);
    for (name, payment_percent) in entries {
        parts.push_back(MilestoneSplitPart {
            name: String::from_str(env, name),
            token: None,
            payment_percent: *payment_percent,
        });
    }
    parts
}

fn create_with_options(
    t: &TestSetup,
    id: &str,
    options: &ShipmentOptions,
    milestones: &Vec<Milestone>,
) -> String {
    let shipment_id = String::from_str(&t.env, id);
    client(t).create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        milestones,
        options,
    );
    shipment_id
}

fn create_basic(t: &TestSetup, id: &str) -> String {
    let milestones = build_milestones(&t.env);
    create_with_options(t, id, &default_options(&t.env), &milestones)
}

fn submit(t: &TestSetup, shipment_id: &String, index: u32) {
    client(t).submit_proof(
        &t.supplier,
        shipment_id,
        &index,
        &String::from_str(&t.env, "proof_hash"),
        &Symbol::new(&t.env, "ipfs"),
    );
}

#[test]
fn test_split_waits_for_approval_and_preserves_escrow_and_later_state() {
    let t = setup();
    let c = client(&t);
    let mut milestones = build_milestones(&t.env);
    let mut later = milestones.get(2).unwrap();
    later.deadline_ledger = 4_321;
    milestones.set(2, later);
    let mut options = default_options(&t.env);
    options.milestone_splits = vec![&t.env, 2_500, 5_000, 2_500];
    let shipment_id = create_with_options(&t, "SPLIT-OK", &options, &milestones);

    t.env.as_contract(&t.contract_id, || {
        t.env.storage().persistent().set(
            &DataKeyExt::MilestoneDeadline(shipment_id.clone(), 2),
            &7_777u32,
        );
    });
    submit(&t, &shipment_id, 2);
    let escrow_before = c.get_escrow_balance(&shipment_id);

    let parts = split_parts(&t.env, &[("Build", 20), ("Inspect", 30)]);
    c.propose_milestone_split(&t.supplier, &shipment_id, &1, &parts);
    assert_eq!(c.get_shipment(&shipment_id).milestones.len(), 3);
    assert_eq!(
        c.get_milestone_split_proposal(&shipment_id),
        Some(MilestoneSplitProposal {
            milestone_index: 1,
            proposer: t.supplier.clone(),
            parts,
        })
    );

    c.approve_milestone_split(&t.buyer, &shipment_id, &1);
    let shipment = c.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.len(), 4);
    assert_eq!(shipment.milestones.get(0).unwrap().payment_percent, 25);
    assert_eq!(shipment.milestones.get(1).unwrap().name, String::from_str(&t.env, "Build"));
    assert_eq!(shipment.milestones.get(1).unwrap().payment_percent, 20);
    assert_eq!(shipment.milestones.get(2).unwrap().name, String::from_str(&t.env, "Inspect"));
    assert_eq!(shipment.milestones.get(2).unwrap().payment_percent, 30);
    let shifted = shipment.milestones.get(3).unwrap();
    assert_eq!(shifted.name, String::from_str(&t.env, "Delivered"));
    assert_eq!(shifted.payment_percent, 25);
    assert_eq!(shifted.deadline_ledger, 4_321);
    assert_eq!(shifted.status, MilestoneStatus::ProofSubmitted);
    assert_eq!(shifted.proof_hash, String::from_str(&t.env, "proof_hash"));
    let shifted_deadline = t.env.as_contract(&t.contract_id, || {
        t.env
            .storage()
            .persistent()
            .get::<DataKeyExt, u32>(&DataKeyExt::MilestoneDeadline(shipment_id.clone(), 3))
    });
    assert_eq!(shifted_deadline, Some(7_777));
    assert_eq!(c.get_escrow_balance(&shipment_id), escrow_before);
    assert_eq!(c.get_milestone_split_proposal(&shipment_id), None);

    let splits: Vec<u32> = t.env.as_contract(&t.contract_id, || {
        t.env
            .storage()
            .persistent()
            .get(&DataKeyExt::MilestoneSplits(shipment_id.clone()))
            .unwrap()
    });
    assert_eq!(splits, vec![&t.env, 2_500, 2_000, 3_000, 2_500]);
    assert_eq!(splits.iter().sum::<u32>(), 10_000);
    assert_eq!(shipment.milestones.iter().map(|m| m.payment_percent).sum::<u32>(), 100);
    assert_eq!(
        shipment
            .audit_log
            .get(shipment.audit_log.len() - 1)
            .unwrap()
            .action,
        Symbol::new(&t.env, "milestones_split")
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_split_stranger_cannot_propose() {
    let t = setup();
    let shipment_id = create_basic(&t, "SPLIT-STRANGER-PROPOSE");
    let stranger = Address::generate(&t.env);
    client(&t).propose_milestone_split(
        &stranger,
        &shipment_id,
        &0,
        &split_parts(&t.env, &[("A", 10), ("B", 15)]),
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_split_stranger_cannot_approve() {
    let t = setup();
    let c = client(&t);
    let shipment_id = create_basic(&t, "SPLIT-STRANGER-APPROVE");
    c.propose_milestone_split(
        &t.buyer,
        &shipment_id,
        &0,
        &split_parts(&t.env, &[("A", 10), ("B", 15)]),
    );
    let stranger = Address::generate(&t.env);
    c.approve_milestone_split(&stranger, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "split must be approved by the counterparty")]
fn test_split_proposer_cannot_self_approve() {
    let t = setup();
    let c = client(&t);
    let shipment_id = create_basic(&t, "SPLIT-SELF-APPROVE");
    c.propose_milestone_split(
        &t.buyer,
        &shipment_id,
        &0,
        &split_parts(&t.env, &[("A", 10), ("B", 15)]),
    );
    c.approve_milestone_split(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "only pending milestones without proof can be split")]
fn test_split_rejected_if_milestone_becomes_non_pending_before_approval() {
    let t = setup();
    let c = client(&t);
    let shipment_id = create_basic(&t, "SPLIT-REVALIDATE");
    c.propose_milestone_split(
        &t.buyer,
        &shipment_id,
        &0,
        &split_parts(&t.env, &[("A", 10), ("B", 15)]),
    );
    submit(&t, &shipment_id, 0);
    c.approve_milestone_split(&t.supplier, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "split percentages must equal the original milestone percentage")]
fn test_split_rejects_percentages_that_do_not_match() {
    let t = setup();
    let shipment_id = create_basic(&t, "SPLIT-BAD-SUM");
    client(&t).propose_milestone_split(
        &t.buyer,
        &shipment_id,
        &0,
        &split_parts(&t.env, &[("A", 10), ("B", 10)]),
    );
}

#[test]
#[should_panic(expected = "TooManyMilestones")]
fn test_split_respects_maximum_milestone_count() {
    let t = setup();
    let c = client(&t);
    c.set_max_milestone_count(&t.buyer, &3);
    let shipment_id = create_basic(&t, "SPLIT-MAX");
    c.propose_milestone_split(
        &t.buyer,
        &shipment_id,
        &0,
        &split_parts(&t.env, &[("A", 10), ("B", 15)]),
    );
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn test_split_rejects_out_of_range_index() {
    let t = setup();
    let shipment_id = create_basic(&t, "SPLIT-INDEX");
    client(&t).propose_milestone_split(
        &t.buyer,
        &shipment_id,
        &3,
        &split_parts(&t.env, &[("A", 10), ("B", 15)]),
    );
}