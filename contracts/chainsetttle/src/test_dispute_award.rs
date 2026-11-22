#![cfg(test)]

use super::*;
use crate::test_common::{
    create_standard_shipment, setup, submit_proof_for_milestone, TestSetup,
};
use soroban_sdk::{testutils::Address as _, BytesN, String};

const TOTAL: i128 = 1_000_000;

fn open_dispute(t: &TestSetup, id: &str) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, id);
    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    submit_proof_for_milestone(t, &shipment_id, 0);
    client.raise_dispute(&t.buyer, &shipment_id, &0u32);
    shipment_id
}

fn hash(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

#[test]
fn award_hash_is_stored_and_immutable() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = open_dispute(&t, "award-1");
    let h = hash(&t.env, 11);
    client.set_award_hash_threshold(&t.buyer, &200_000i128);
    client.resolve_dispute_with_award(&t.arbiter, &id, &0u32, &true, &None, &Some(h.clone()));

    assert_eq!(client.get_dispute_award(&id, &0u32), Some(h.clone()));
    client.attach_dispute_award_hash(&t.arbiter, &id, &0u32, &h);
    assert!(client
        .try_attach_dispute_award_hash(&t.arbiter, &id, &0u32, &hash(&t.env, 99))
        .is_err());
    assert_eq!(client.get_dispute_award(&id, &0u32), Some(h));
}

#[test]
fn threshold_requires_award_hash() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = open_dispute(&t, "award-2");
    client.set_award_hash_threshold(&t.buyer, &200_000i128);

    assert!(client
        .try_resolve_dispute(&t.arbiter, &id, &0u32, &true, &None)
        .is_err());
    client.resolve_dispute_with_award(
        &t.arbiter,
        &id,
        &0u32,
        &true,
        &None,
        &Some(hash(&t.env, 22)),
    );
    assert_eq!(client.get_dispute_award(&id, &0u32), Some(hash(&t.env, 22)));
}
