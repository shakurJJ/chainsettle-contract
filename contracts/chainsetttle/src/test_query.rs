#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, String,
};
use crate::test_common::*;

// ============================================================
// QUERY & READ-ONLY TESTS: COMPLETION PERCENTAGE
// ============================================================

#[test]
fn test_get_completion_percentage_fresh_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-FRESH");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    // Freshly created shipment with no milestones confirmed should return 0%
    assert_eq!(client.get_completion_percentage(&shipment_id), 0);
}

#[test]
fn test_get_completion_percentage_partial_one_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-PARTIAL-1");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    // Confirm first milestone (25%)
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // Should return 25%
    assert_eq!(client.get_completion_percentage(&shipment_id), 25);
}

#[test]
fn test_get_completion_percentage_partial_two_milestones() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-PARTIAL-2");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    // Confirm first milestone (25%)
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // Confirm second milestone (50% cumulative = 75% total)
    client.submit_proof(
        &t.logistics,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "ipfs://t"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    // Should return 75%
    assert_eq!(client.get_completion_percentage(&shipment_id), 75);
}

#[test]
fn test_get_completion_percentage_full_completion() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-FULL");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    // Confirm all milestones
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    client.submit_proof(
        &t.logistics,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "ipfs://t"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &2,
        &String::from_str(&t.env, "ipfs://v"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &2);

    // Should return 100%
    assert_eq!(client.get_completion_percentage(&shipment_id), 100);
}

#[test]
fn test_get_completion_percentage_zero_released() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-ZERO");
    let total_amount: i128 = 100;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    // Before any confirmation, released_amount is 0, should return 0%
    assert_eq!(client.get_completion_percentage(&shipment_id), 0);

    // Confirm first milestone (25 out of 100 = 25%)
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // (25 * 100) / 100 = 25%
    assert_eq!(client.get_completion_percentage(&shipment_id), 25);
}
