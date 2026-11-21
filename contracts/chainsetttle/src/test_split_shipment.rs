#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{
    create_standard_shipment, setup, submit_proof_for_milestone, TestSetup,
};
use soroban_sdk::{testutils::Address as _, vec, Address, String};

const TOTAL: i128 = 1_000_000_000;

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

#[test]
fn split_shipment_happy_path_preserves_milestones_and_escrow() {
    let t = setup();
    let c = client(&t);
    let shipment_id = String::from_str(&t.env, "SPLIT-SHIP-1");
    let new_shipment_id = String::from_str(&t.env, "SPLIT-SHIP-2");

    create_standard_shipment(
        &c,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    let escrow_before = c.get_escrow_balance(&shipment_id);

    c.propose_split_shipment(
        &t.buyer,
        &shipment_id,
        &new_shipment_id,
        &vec![&t.env, 1u32],
    );
    c.agree_split_shipment(&t.supplier, &shipment_id);

    let original = c.get_shipment(&shipment_id);
    let split = c.get_shipment(&new_shipment_id);

    assert_eq!(split.id, new_shipment_id);
    assert_eq!(split.milestones.len(), 1);
    assert_eq!(
        split.milestones.get(0).unwrap().name,
        String::from_str(&t.env, "In Transit")
    );
    assert_eq!(split.total_amount, TOTAL * 50 / 100);
    assert_eq!(original.milestones.len(), 2);
    assert_eq!(original.total_amount, TOTAL - split.total_amount);
    assert_eq!(c.get_escrow_balance(&new_shipment_id), split.total_amount);
    assert_eq!(
        c.get_escrow_balance(&shipment_id) + c.get_escrow_balance(&new_shipment_id),
        escrow_before
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn stranger_cannot_agree_to_split() {
    let t = setup();
    let c = client(&t);
    let shipment_id = String::from_str(&t.env, "SPLIT-UNAUTH-1");
    create_standard_shipment(
        &c,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    c.propose_split_shipment(
        &t.buyer,
        &shipment_id,
        &String::from_str(&t.env, "SPLIT-UNAUTH-2"),
        &vec![&t.env, 1u32],
    );
    c.agree_split_shipment(&Address::generate(&t.env), &shipment_id);
}

#[test]
#[should_panic(expected = "only proposer can cancel")]
fn non_proposer_cannot_cancel_split() {
    let t = setup();
    let c = client(&t);
    let shipment_id = String::from_str(&t.env, "SPLIT-CANCEL-1");
    create_standard_shipment(
        &c,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    c.propose_split_shipment(
        &t.buyer,
        &shipment_id,
        &String::from_str(&t.env, "SPLIT-CANCEL-2"),
        &vec![&t.env, 1u32],
    );
    c.cancel_split_shipment(&t.supplier, &shipment_id);
}

#[test]
#[should_panic(expected = "only pending milestones can be split")]
fn cannot_split_non_pending_milestone() {
    let t = setup();
    let c = client(&t);
    let shipment_id = String::from_str(&t.env, "SPLIT-NON-PENDING");
    create_standard_shipment(
        &c,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    submit_proof_for_milestone(&t, &shipment_id, 1);
    c.propose_split_shipment(
        &t.buyer,
        &shipment_id,
        &String::from_str(&t.env, "SPLIT-NON-PENDING-2"),
        &vec![&t.env, 1u32],
    );
}
