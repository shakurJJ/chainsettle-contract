#![cfg(test)]

//! Tests for #617: global uniqueness registry for bill-of-lading and
//! invoice hashes.

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{testutils::Address as _, vec, Address, BytesN, String, Symbol, Vec};

fn single_milestone(env: &soroban_sdk::Env) -> Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "All"),
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
        },
    ]
}

fn create_active(t: &TestSetup, id: &str) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = String::from_str(&t.env, id);
    let opts = default_options(&t.env);
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &single_milestone(&t.env),
        &opts,
    );
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &String::from_str(&t.env, "ipfs://proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    ship_id
}

fn hash32(env: &soroban_sdk::Env, fill: u8) -> BytesN<32> {
    BytesN::from_array(env, &[fill; 32])
}

#[test]
fn test_attach_hashes_happy_path() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create_active(&t, "doc-ok");

    let invoice = hash32(&t.env, 1);
    let bol = hash32(&t.env, 2);
    client.attach_invoice_hash(&t.supplier, &id, &0u32, &invoice);
    client.attach_bol_hash(&t.supplier, &id, &0u32, &bol);

    assert_eq!(client.get_invoice_hash(&id, &0u32).unwrap(), invoice);
    assert_eq!(client.get_bol_hash(&id, &0u32).unwrap(), bol);
    assert_eq!(client.get_document_hash_owner(&invoice).unwrap(), id);
    assert_eq!(client.get_document_hash_owner(&bol).unwrap(), id);
}

#[test]
#[should_panic(expected = "document hash already used")]
fn test_duplicate_invoice_hash_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let a = create_active(&t, "doc-a");
    let b = create_active(&t, "doc-b");

    let invoice = hash32(&t.env, 3);
    client.attach_invoice_hash(&t.supplier, &a, &0u32, &invoice);
    client.attach_invoice_hash(&t.supplier, &b, &0u32, &invoice);
}

#[test]
#[should_panic(expected = "document hash already used")]
fn test_duplicate_bol_hash_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let a = create_active(&t, "doc-c");
    let b = create_active(&t, "doc-d");

    let bol = hash32(&t.env, 4);
    client.attach_bol_hash(&t.supplier, &a, &0u32, &bol);
    client.attach_bol_hash(&t.supplier, &b, &0u32, &bol);
}

#[test]
#[should_panic(expected = "document hash already used")]
fn test_cross_type_collision_rejected() {
    // An invoice hash and a BOL hash share one registry namespace.
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let a = create_active(&t, "doc-e");
    let b = create_active(&t, "doc-f");

    let h = hash32(&t.env, 5);
    client.attach_invoice_hash(&t.supplier, &a, &0u32, &h);
    client.attach_bol_hash(&t.supplier, &b, &0u32, &h);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_attach_requires_supplier_auth() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create_active(&t, "doc-auth");
    let stranger = Address::generate(&t.env);

    client.attach_bol_hash(&stranger, &id, &0u32, &hash32(&t.env, 6));
}

#[test]
#[should_panic(expected = "bol hash already set and is immutable")]
fn test_bol_hash_immutable_once_set() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create_active(&t, "doc-imm");

    client.attach_bol_hash(&t.supplier, &id, &0u32, &hash32(&t.env, 7));
    client.attach_bol_hash(&t.supplier, &id, &0u32, &hash32(&t.env, 8));
}

#[test]
fn test_hash_reusable_after_cancel() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let a = create_active(&t, "doc-g");

    let invoice = hash32(&t.env, 9);
    let bol = hash32(&t.env, 10);
    client.attach_invoice_hash(&t.supplier, &a, &0u32, &invoice);
    client.attach_bol_hash(&t.supplier, &a, &0u32, &bol);

    client.cancel_shipment(&t.buyer, &a);
    assert!(client.get_document_hash_owner(&invoice).is_none());
    assert!(client.get_document_hash_owner(&bol).is_none());

    // Same documents finance a new shipment without a panic.
    let b = create_active(&t, "doc-h");
    client.attach_invoice_hash(&t.supplier, &b, &0u32, &invoice);
    client.attach_bol_hash(&t.supplier, &b, &0u32, &bol);
    assert_eq!(client.get_document_hash_owner(&invoice).unwrap(), b);
}
