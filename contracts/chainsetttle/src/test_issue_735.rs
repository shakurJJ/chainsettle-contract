#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, BytesN, String, Symbol,
};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[test]
fn refund_releases_only_after_supplier_confirms_return_receipt() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);

    let id = String::from_str(&s.env, "RMA-1");
    let mut opts = default_options(&s.env);
    opts.review_window_ledgers = Some(100);
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &opts,
    );
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );

    let reason = BytesN::<32>::from_array(&s.env, &[1u8; 32]);
    let proof = BytesN::<32>::from_array(&s.env, &[2u8; 32]);
    client.request_return(&s.buyer, &id, &0, &reason);

    let before = token.balance(&s.buyer);
    client.accept_return(&s.supplier, &id, &0);
    client.submit_return_proof(&s.logistics, &id, &0, &proof);
    assert_eq!(token.balance(&s.buyer), before, "no refund before receipt");

    client.confirm_return_receipt(&s.supplier, &id, &0);
    assert_eq!(token.balance(&s.buyer) - before, 2_500);

    let shipment = client.get_shipment(&id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Resolved);
}

#[test]
fn supplier_can_escalate_a_return_to_the_existing_dispute_flow() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let id = String::from_str(&s.env, "RMA-2");
    let mut opts = default_options(&s.env);
    opts.review_window_ledgers = Some(100);
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &opts,
    );
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    client.request_return(
        &s.buyer,
        &id,
        &0,
        &BytesN::<32>::from_array(&s.env, &[3u8; 32]),
    );
    client.escalate_return(&s.supplier, &id, &0);

    assert_eq!(
        client.get_shipment(&id).milestones.get(0).unwrap().status,
        MilestoneStatus::Disputed
    );
}
