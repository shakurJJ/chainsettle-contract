#![cfg(test)]

use super::*;
use crate::test_common::{
    create_standard_shipment, setup, submit_proof_for_milestone, TestSetup,
};
use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{testutils::{Address as _, Ledger as _}, Bytes, BytesN, String};

const TOTAL: i128 = 1_000_000;

fn setup_shipment(t: &TestSetup, id: &str, expiry: u32) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = String::from_str(&t.env, id);
    create_standard_shipment(
        &client,
        &t.env,
        &id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    submit_proof_for_milestone(t, &id, 0);
    t.env.ledger().with_mut(|l| l.sequence_number = 100);
    let _ = expiry;
    id
}

fn signature(t: &TestSetup, id: &String, nonce: u64, expiry: u32) -> BytesN<64> {
    let sk = SigningKey::from_bytes(&[9u8; 32]);
    let payload: Bytes =
        ChainSettleContract::confirmation_payload(&t.env, id, 0, nonce, expiry);
    let sig = sk.sign(payload.to_buffer::<128>().as_slice());
    BytesN::from_array(&t.env, &sig.to_bytes())
}

#[test]
fn relayer_submits_signed_confirmation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = setup_shipment(&t, "signed-1", 200);
    let sk = SigningKey::from_bytes(&[9u8; 32]);
    client.register_confirmation_key(
        &t.buyer,
        &BytesN::from_array(&t.env, &sk.verifying_key().to_bytes()),
    );
    let relayer = Address::generate(&t.env);

    client.confirm_milestone_signed(
        &relayer,
        &t.buyer,
        &id,
        &0u32,
        &1u64,
        &200u32,
        &signature(&t, &id, 1, 200),
    );
    let shipment = client.get_shipment(&id);
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

#[test]
fn replayed_nonce_is_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = setup_shipment(&t, "signed-2", 200);
    let sk = SigningKey::from_bytes(&[8u8; 32]);
    client.register_confirmation_key(
        &t.buyer,
        &BytesN::from_array(&t.env, &sk.verifying_key().to_bytes()),
    );
    let sig = {
        let payload = ChainSettleContract::confirmation_payload(&t.env, &id, 0, 7, 200);
        BytesN::from_array(&t.env, &sk.sign(payload.to_buffer::<128>().as_slice()).to_bytes())
    };
    let relayer = Address::generate(&t.env);
    client.confirm_milestone_signed(&relayer, &t.buyer, &id, &0u32, &7u64, &200u32, &sig);
    assert!(client
        .try_confirm_milestone_signed(&relayer, &t.buyer, &id, &0u32, &7u64, &200u32, &sig)
        .is_err());
}

#[test]
fn expired_and_revoked_keys_are_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = setup_shipment(&t, "signed-3", 50);
    let sk = SigningKey::from_bytes(&[7u8; 32]);
    client.register_confirmation_key(
        &t.buyer,
        &BytesN::from_array(&t.env, &sk.verifying_key().to_bytes()),
    );
    let sig = {
        let payload = ChainSettleContract::confirmation_payload(&t.env, &id, 0, 3, 50);
        BytesN::from_array(&t.env, &sk.sign(payload.to_buffer::<128>().as_slice()).to_bytes())
    };
    let relayer = Address::generate(&t.env);
    assert!(client
        .try_confirm_milestone_signed(&relayer, &t.buyer, &id, &0u32, &3u64, &50u32, &sig)
        .is_err());

    client.revoke_confirmation_key(&t.buyer);
    t.env.ledger().with_mut(|l| l.sequence_number = 40);
    assert!(client
        .try_confirm_milestone_signed(&relayer, &t.buyer, &id, &0u32, &4u64, &200u32, &sig)
        .is_err());
}
