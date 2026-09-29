#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec};
use soroban_sdk::{testutils::Address as _, token, vec, String, Symbol};

fn single_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "M1"),
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ]
}

fn two_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "M1"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "M2"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ]
}

// ── #556 top_up_collateral ─────────────────────────────────────────────────

#[test]
fn test_top_up_collateral_happy_path() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000);

    let shipment_id = String::from_str(&t.env, "s-topup");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 10_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &single_milestone(&t.env),
        &opts,
    );

    let before = client.get_locked_collateral(&shipment_id);
    client.top_up_collateral(&t.supplier, &shipment_id, &5_000i128);
    let after = client.get_locked_collateral(&shipment_id);
    assert_eq!(after, before + 5_000);
}

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn test_top_up_collateral_zero_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000);

    let shipment_id = String::from_str(&t.env, "s-topup-zero");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 10_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &single_milestone(&t.env),
        &opts,
    );
    client.top_up_collateral(&t.supplier, &shipment_id, &0i128);
}

#[test]
#[should_panic(expected = "unauthorized: only the supplier")]
fn test_top_up_collateral_wrong_caller() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000);

    let shipment_id = String::from_str(&t.env, "s-topup-unauth");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 10_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &single_milestone(&t.env),
        &opts,
    );
    let rando = Address::generate(&t.env);
    client.top_up_collateral(&rando, &shipment_id, &1_000i128);
}

#[test]
#[should_panic(expected = "top-up only allowed on active shipments")]
fn test_top_up_collateral_on_cancelled_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000);

    let shipment_id = String::from_str(&t.env, "s-topup-done");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 10_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &single_milestone(&t.env),
        &opts,
    );
    client.cancel_shipment(&t.buyer, &shipment_id);
    client.top_up_collateral(&t.supplier, &shipment_id, &1_000i128);
}

// ── #553 set_refund_recipient ──────────────────────────────────────────────

#[test]
fn test_set_and_get_refund_recipient() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "s-recip");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );

    assert_eq!(client.get_refund_recipient(&shipment_id), None);

    let recipient = Address::generate(&t.env);
    client.set_refund_recipient(&t.buyer, &shipment_id, &Some(recipient.clone()));
    assert_eq!(client.get_refund_recipient(&shipment_id), Some(recipient));
}

#[test]
fn test_clear_refund_recipient() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "s-recip-clear");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );

    let recipient = Address::generate(&t.env);
    client.set_refund_recipient(&t.buyer, &shipment_id, &Some(recipient));
    client.set_refund_recipient(&t.buyer, &shipment_id, &None);
    assert_eq!(client.get_refund_recipient(&shipment_id), None);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_set_refund_recipient_non_buyer_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "s-recip-unauth");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );

    let rando = Address::generate(&t.env);
    client.set_refund_recipient(&rando, &shipment_id, &Some(Address::generate(&t.env)));
}

// ── #554 cancel_for_blacklisted_supplier ──────────────────────────────────

#[test]
fn test_cancel_blacklisted_supplier_happy() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mint_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    mint_client.mint(&t.supplier, &100_000);

    let shipment_id = String::from_str(&t.env, "s-bl-cancel");
    let mut opts = default_options(&t.env);
    opts.supplier_collateral = 10_000;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &single_milestone(&t.env),
        &opts,
    );

    let reason_hash = soroban_sdk::BytesN::from_array(&t.env, &[1u8; 32]);
    client.blacklist_address(&t.buyer, &t.supplier, &reason_hash);

    client.cancel_for_blacklisted_supplier(&t.buyer, &shipment_id);

    let s = client.get_shipment(&shipment_id);
    assert_eq!(s.status, ShipmentStatus::Cancelled);
    assert_eq!(s.cancellation_reason.len(), 1);
}

#[test]
#[should_panic(expected = "supplier is not blacklisted")]
fn test_cancel_blacklisted_supplier_not_blacklisted() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "s-not-bl");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );

    client.cancel_for_blacklisted_supplier(&t.buyer, &shipment_id);
}

// ── #551 consortium milestone suppliers ───────────────────────────────────

#[test]
fn test_consortium_creation_and_listing() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let supplier_a = Address::generate(&t.env);
    let supplier_b = Address::generate(&t.env);

    let shipment_id = String::from_str(&t.env, "s-consortium");
    let mut opts = default_options(&t.env);
    opts.milestone_suppliers = vec![&t.env, supplier_a.clone(), supplier_b.clone()];
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &two_milestones(&t.env),
        &opts,
    );

    assert!(client.is_consortium_shipment(&shipment_id));

    let a_shipments = client.get_shipments_by_supplier(&supplier_a);
    assert!(a_shipments.contains(&shipment_id), "supplier_a should be indexed");

    let b_shipments = client.get_shipments_by_supplier(&supplier_b);
    assert!(b_shipments.contains(&shipment_id), "supplier_b should be indexed");
}

#[test]
fn test_non_consortium_shipment_flag() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "s-single");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );

    assert!(!client.is_consortium_shipment(&shipment_id));
}

#[test]
fn test_consortium_proof_by_milestone_supplier() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let supplier_a = Address::generate(&t.env);
    let supplier_b = Address::generate(&t.env);

    let shipment_id = String::from_str(&t.env, "s-cons-proof");
    let mut opts = default_options(&t.env);
    opts.milestone_suppliers = vec![&t.env, supplier_a.clone(), supplier_b.clone()];
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &200_000,
        &two_milestones(&t.env),
        &opts,
    );

    // supplier_a submits proof for milestone 0 — authorized (mock_all_auths).
    client.submit_proof(
        &supplier_a,
        &shipment_id,
        &0u32,
        &String::from_str(&t.env, "proof1"),
        &Symbol::new(&t.env, "hash"),
    );
}

#[test]
#[should_panic(expected = "milestone_suppliers length must match milestone count")]
fn test_consortium_length_mismatch_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "s-cons-bad");
    let mut opts = default_options(&t.env);
    opts.milestone_suppliers = vec![
        &t.env,
        Address::generate(&t.env),
        Address::generate(&t.env),
    ];
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &100_000,
        &single_milestone(&t.env),
        &opts,
    );
}
