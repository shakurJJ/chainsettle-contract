#![cfg(test)]

use super::*;
use crate::test_common::{build_milestones, default_options, setup, submit_proof_for_milestone, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, String, Symbol,
};

fn create_test_shipment(t: &TestSetup, id: &str) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, id);
    let milestones = build_milestones(&t.env);
    let options = default_options(&t.env);
    let buyers = soroban_sdk::vec![&t.env, t.buyer.clone()];

    client.create_shipment(
        &shipment_id,
        &buyers,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestones,
        &options,
    );

    shipment_id
}

#[test]
fn test_operation_pause_create_blocks_only_creation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone(); // In setup(), client.init(&buyer) sets buyer as admin

    // Create an initial shipment before pausing creation
    let shipment_id = create_test_shipment(&t, "SHIP-OP-1");

    // Initially not paused
    let op_create = Symbol::new(&t.env, "create");
    let op_confirm = Symbol::new(&t.env, "confirm");
    assert!(!client.is_operation_paused(&op_create));
    assert!(!client.is_operation_paused(&op_confirm));

    // Admin pauses create operation
    client.set_operation_paused(&admin, &op_create, &true);
    assert!(client.is_operation_paused(&op_create));
    assert!(!client.is_operation_paused(&op_confirm));

    // Existing shipment can still proceed with proof submission and milestone confirmation!
    submit_proof_for_milestone(&t, &shipment_id, 0);
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
    let s = client.get_shipment(&shipment_id);
    assert_eq!(s.milestones.get(0).unwrap().status, MilestoneStatus::Confirmed);

    // Unpause create and verify new shipments can be created
    client.set_operation_paused(&admin, &op_create, &false);
    assert!(!client.is_operation_paused(&op_create));
    let shipment_id_2 = create_test_shipment(&t, "SHIP-OP-2");
    let s2 = client.get_shipment(&shipment_id_2);
    assert_eq!(s2.status, ShipmentStatus::Active);
}

#[test]
#[should_panic(expected = "operation is paused")]
fn test_create_shipment_panics_when_create_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    client.set_operation_paused(&admin, &Symbol::new(&t.env, "create"), &true);
    create_test_shipment(&t, "SHIP-BLOCKED");
}

#[test]
#[should_panic(expected = "operation is paused")]
fn test_confirm_milestone_panics_when_confirm_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    let shipment_id = create_test_shipment(&t, "SHIP-CONFIRM-PAUSED");
    submit_proof_for_milestone(&t, &shipment_id, 0);

    client.set_operation_paused(&admin, &Symbol::new(&t.env, "confirm"), &true);
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "operation is paused")]
fn test_advance_panics_when_advance_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    let shipment_id = create_test_shipment(&t, "SHIP-ADV-PAUSED");

    client.set_operation_paused(&admin, &Symbol::new(&t.env, "advance"), &true);
    client.request_advance(&t.supplier, &shipment_id, &0, &20);
}

#[test]
#[should_panic(expected = "operation is paused")]
fn test_dispute_panics_when_dispute_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    let shipment_id = create_test_shipment(&t, "SHIP-DISPUTE-PAUSED");
    submit_proof_for_milestone(&t, &shipment_id, 0);

    client.set_operation_paused(&admin, &Symbol::new(&t.env, "dispute"), &true);
    client.raise_dispute(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "operation is paused")]
fn test_claim_payout_panics_when_claim_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    client.set_payout_mode(&t.supplier, &true);
    client.set_operation_paused(&admin, &Symbol::new(&t.env, "claim"), &true);
    client.claim_payout(&t.supplier, &t.token_id);
}

#[test]
fn test_global_pause_overrides_operation_pause() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    let op_create = Symbol::new(&t.env, "create");
    let op_confirm = Symbol::new(&t.env, "confirm");
    let op_dispute = Symbol::new(&t.env, "dispute");

    assert!(!client.is_operation_paused(&op_create));

    // Globally pause contract
    client.pause(&admin);

    // is_operation_paused returns true for all operations when global pause is active
    assert!(client.is_operation_paused(&op_create));
    assert!(client.is_operation_paused(&op_confirm));
    assert!(client.is_operation_paused(&op_dispute));

    client.unpause(&admin);
    assert!(!client.is_operation_paused(&op_create));
}

#[test]
#[should_panic(expected = "unknown operation")]
fn test_set_operation_paused_unknown_operation_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    let invalid_op = Symbol::new(&t.env, "invalid_op_name");
    client.set_operation_paused(&admin, &invalid_op, &true);
}

#[test]
#[should_panic(expected = "unknown operation")]
fn test_is_operation_paused_unknown_operation_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let invalid_op = Symbol::new(&t.env, "not_an_op");
    client.is_operation_paused(&invalid_op);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_set_operation_paused_unauthorized() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let stranger = Address::generate(&t.env);

    client.set_operation_paused(&stranger, &Symbol::new(&t.env, "create"), &true);
}

#[test]
fn test_operation_paused_event_emission() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let admin = t.buyer.clone();

    let op = Symbol::new(&t.env, "dispute");
    client.set_operation_paused(&admin, &op, &true);

    let events = t.env.events().all();
    let found = events.iter().any(|e| {
        if e.0 == t.contract_id && e.1.len() >= 2 {
            let topic0: Symbol = e.1.get(0).unwrap().try_into().unwrap_or(Symbol::new(&t.env, ""));
            let topic1: Symbol = e.1.get(1).unwrap().try_into().unwrap_or(Symbol::new(&t.env, ""));
            return topic0 == Symbol::new(&t.env, "operation_paused_set") && topic1 == op;
        }
        false
    });
    assert!(found, "operation_paused_set event must be published");
}
