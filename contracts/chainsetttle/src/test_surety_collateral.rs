#![cfg(test)]

//! #789 – Third-party surety posting collateral for a supplier.

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, String, Symbol, Vec,
};

fn single_milestone(env: &Env, deadline: u32) -> Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Delivery"),
            token: None,
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: deadline,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
            logistics: None,
        },
    ]
}

/// Creates a shipment where the supplier locks `collateral`, then has an
/// approved surety post the same amount on the supplier's behalf.
fn create_surety_backed(t: &TestSetup, id: &str, collateral: i128, deadline: u32) -> (String, Address) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let sac = token::StellarAssetClient::new(&t.env, &t.token_id);
    sac.mint(&t.supplier, &collateral);

    let surety = Address::generate(&t.env);
    sac.mint(&surety, &collateral);
    client.set_approved_surety(&t.buyer, &surety, &true);

    let mut opts = default_options(&t.env);
    opts.supplier_collateral = collateral;
    let shipment_id = String::from_str(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &single_milestone(&t.env, deadline),
        &opts,
    );
    client.post_surety_collateral(&surety, &shipment_id, &collateral);
    (shipment_id, surety)
}

#[test]
fn surety_substitutes_supplier_collateral() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let tok = token::Client::new(&t.env, &t.token_id);

    let (id, surety) = create_surety_backed(&t, "SURETY-1", 10_000, 0);

    // Supplier's own capital is released; the surety now backs the collateral.
    assert_eq!(tok.balance(&t.supplier), 10_000);
    assert_eq!(tok.balance(&surety), 0);
    assert_eq!(client.get_locked_collateral(&id), 10_000);
    let deposit = client.get_surety_deposit(&id).unwrap();
    assert_eq!(deposit.surety, surety);
    assert_eq!(deposit.amount, 10_000);
}

#[test]
fn refund_on_completion_goes_to_surety() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let tok = token::Client::new(&t.env, &t.token_id);

    let (id, surety) = create_surety_backed(&t, "SURETY-2", 10_000, 0);
    let supplier_before = tok.balance(&t.supplier);

    client.submit_proof(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &id, &0);

    assert_eq!(tok.balance(&surety), 10_000);
    // Supplier receives the milestone payment but not the surety's collateral.
    let supplier_gain = tok.balance(&t.supplier) - supplier_before;
    assert!(supplier_gain <= 1_000_000);
    assert_eq!(client.get_surety_deposit(&id).unwrap().amount, 0);
}

#[test]
fn slash_reduces_surety_deposit() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    t.env.ledger().set_sequence_number(1);
    let (id, _surety) = create_surety_backed(&t, "SURETY-3", 10_000, 100);
    client.set_collateral_slash_bps(&t.buyer, &id, &5_000);

    t.env.ledger().set_sequence_number(200);
    let slashed = client.slash_collateral_for_miss(&t.buyer, &id, &0u32);
    assert_eq!(slashed, 5_000);
    assert_eq!(client.get_surety_deposit(&id).unwrap().amount, 5_000);
    assert_eq!(client.get_locked_collateral(&id), 5_000);
}

#[test]
fn buyer_cancel_forfeit_draws_surety() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let (id, _surety) = create_surety_backed(&t, "SURETY-4", 10_000, 0);
    client.cancel_shipment(&t.buyer, &id);
    assert_eq!(client.get_surety_deposit(&id).unwrap().amount, 0);
}

#[test]
#[should_panic(expected = "surety is not approved")]
fn unapproved_surety_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, "SURETY-5");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &single_milestone(&t.env, 0),
        &default_options(&t.env),
    );
    let stranger = Address::generate(&t.env);
    client.post_surety_collateral(&stranger, &shipment_id, &100);
}
