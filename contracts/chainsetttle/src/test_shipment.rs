#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, String, vec,
};
use crate::test_common::*;

// ============================================================
// CORE LIFECYCLE TESTS
// ============================================================

#[test]
fn test_create_shipment_success() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-001");
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

    assert_eq!(
        token_client.balance(&t.buyer),
        10_000_000_000 - total_amount
    );
    assert_eq!(token_client.balance(&t.contract_id), total_amount);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.total_amount, total_amount);
    assert_eq!(shipment.released_amount, 0);
    assert_eq!(shipment.milestones.len(), 3);
    assert_eq!(shipment.holdback_ledgers, 0);
    assert_eq!(shipment.dispute_cooldown_ledgers, 0);
}

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_create_shipment_invalid_percentages() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let bad_milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Step 1"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Step 2"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Step 3"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ];

    client.create_shipment(
        &String::from_str(&t.env, "SHIP-BAD"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &bad_milestones,
        &default_options(&t.env),
    );
}

#[test]
fn test_full_shipment_lifecycle() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-FULL");
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

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    assert_eq!(token_client.balance(&t.supplier), total_amount);
    assert_eq!(client.get_escrow_balance(&shipment_id), 0);
}

#[test]
fn test_full_lifecycle_with_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-FULL-DISP");
    let total_amount: i128 = 1_000_000_000;

    let buyer_balance_before = token_client.balance(&t.buyer);

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

    // Milestone 0: supplier submits proof and buyer confirms -> payment released
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d0"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let m0_payment = total_amount * 25 / 100;
    assert_eq!(token_client.balance(&t.supplier), m0_payment);
    assert_eq!(
        client.get_escrow_balance(&shipment_id),
        total_amount - m0_payment
    );

    // Milestone 1: submit -> buyer disputes -> arbiter rejects -> supplier resubmits -> buyer confirms
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "ipfs://d1"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &1);
    client.resolve_dispute(&t.arbiter, &shipment_id, &1, &false);

    // After reject, supplier resubmits proof and buyer confirms
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "ipfs://d1-resub"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    let m1_payment = total_amount * 50 / 100;
    assert_eq!(token_client.balance(&t.supplier), m0_payment + m1_payment);
    assert_eq!(
        client.get_escrow_balance(&shipment_id),
        total_amount - m0_payment - m1_payment
    );

    // Milestone 2: submit and confirm -> final payment and completion
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &2,
        &String::from_str(&t.env, "ipfs://d2"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &2);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);

    // Final balances: supplier gets full amount, buyer reduced by total_amount, contract escrow zero
    assert_eq!(token_client.balance(&t.supplier), total_amount);
    assert_eq!(
        token_client.balance(&t.buyer),
        buyer_balance_before - total_amount
    );
    assert_eq!(client.get_escrow_balance(&shipment_id), 0);
}

#[test]
fn test_cancel_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-CANCEL");
    let total_amount: i128 = 1_000_000_000;
    let buyer_balance_before = token_client.balance(&t.buyer);

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

    client.cancel_shipment(&t.buyer, &shipment_id);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
    assert_eq!(token_client.balance(&t.buyer), buyer_balance_before);
}

#[test]
fn test_cancel_partial_confirmed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-PARTIAL");
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

    // Confirm milestone 0 (25%)
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let buyer_balance_after_confirm = token_client.balance(&t.buyer);
    client.cancel_shipment(&t.buyer, &shipment_id);

    // Buyer should get back 75% (the unconfirmed portion)
    let expected_refund = total_amount * 75 / 100;
    assert_eq!(
        token_client.balance(&t.buyer),
        buyer_balance_after_confirm + expected_refund
    );
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Cancelled
    );
}

#[test]
#[should_panic(expected = "cannot cancel: dispute must be resolved first")]
fn test_cancel_blocked_by_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-DISP-CANCEL");

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.cancel_shipment(&t.buyer, &shipment_id);
}

#[test]
fn test_cancel_zero_confirmed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let total: i128 = 1_000_000_000;
    let before = token_client.balance(&t.buyer);
    let shipment_id = String::from_str(&t.env, "CANCEL-ZERO");

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total,
    );

    client.cancel_shipment(&t.buyer, &shipment_id);

    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Cancelled
    );
    assert_eq!(token_client.balance(&t.buyer), before);
}
