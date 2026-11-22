#![cfg(test)]

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{testutils::Events as _, token, vec, String};

const TOTAL: i128 = 1_000_000;
const M0_GROSS: i128 = 250_000;

fn create(t: &TestSetup, id: &str, tolerance: u32) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = String::from_str(&t.env, id);
    let mut opts = default_options(&t.env);
    opts.milestone_quantities = vec![&t.env, 100u32, 0u32, 0u32];
    client.create_shipment(
        &id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &build_milestones(&t.env),
        &opts,
    );
    if tolerance > 0 {
        client.set_quantity_tolerance_bps(&t.buyer, &id, &tolerance);
    }
    client.submit_proof(
        &t.supplier,
        &id,
        &0u32,
        &String::from_str(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    id
}

fn supplier_balance(t: &TestSetup) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(&t.supplier)
}

#[test]
fn near_complete_delivery_inside_band_pays_full() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "tol-inside", 500);
    let before = supplier_balance(&t);

    client.confirm_partial_quantity(&t.buyer, &id, &0u32, &96u32);

    assert_eq!(supplier_balance(&t) - before, M0_GROSS);
    let shipment = client.get_shipment(&id);
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

#[test]
fn delivery_outside_band_stays_pro_rata() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "tol-outside", 500);
    let before = supplier_balance(&t);

    client.confirm_partial_quantity(&t.buyer, &id, &0u32, &90u32);

    assert_eq!(supplier_balance(&t) - before, (M0_GROSS * 90) / 100);
    let shipment = client.get_shipment(&id);
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::ProofSubmitted
    );
}

#[test]
fn over_delivery_inside_band_pays_full() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "tol-over", 500);
    let before = supplier_balance(&t);

    client.confirm_partial_quantity(&t.buyer, &id, &0u32, &104u32);

    assert!(supplier_balance(&t) - before >= M0_GROSS);
}

#[test]
fn zero_tolerance_preserves_exact_behaviour() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, "tol-off", 0);
    let before = supplier_balance(&t);
    client.confirm_partial_quantity(&t.buyer, &id, &0u32, &96u32);
    assert_eq!(supplier_balance(&t) - before, (M0_GROSS * 96) / 100);
    assert!(client
        .try_confirm_partial_quantity(&t.buyer, &id, &0u32, &10u32)
        .is_err());
}
