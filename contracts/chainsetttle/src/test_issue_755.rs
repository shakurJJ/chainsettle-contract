#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, String, Symbol};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[test]
fn fixed_logistics_amount_paid_at_each_confirmation() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);
    client.set_fixed_logistics_amounts(&vec![&s.env, 100i128, 200, 300]);

    let id = String::from_str(&s.env, "LOG-FIXED-1");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &default_options(&s.env),
    );
    let before = token.balance(&s.logistics);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    client.confirm_milestone(&s.buyer, &id, &0);
    assert_eq!(token.balance(&s.logistics) - before, 100);
}

#[test]
fn rejects_overfunded_and_conflicting_schedules() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    client.set_fixed_logistics_amounts(&vec![&s.env, 20_000i128, 0, 0]);
    let id = String::from_str(&s.env, "LOG-FIXED-BAD");
    assert!(client
        .try_create_shipment(
            &id,
            &single_buyer_vec(&s.env, &s.buyer),
            &s.supplier,
            &s.logistics,
            &s.arbiter,
            &s.token_id,
            &10_000i128,
            &build_milestones(&s.env),
            &default_options(&s.env),
        )
        .is_err());

    let mut opts = default_options(&s.env);
    opts.logistics_fee_bps = 100;
    client.set_fixed_logistics_amounts(&vec![&s.env, 100i128, 200, 300]);
    let id2 = String::from_str(&s.env, "LOG-FIXED-CONFLICT");
    assert!(client
        .try_create_shipment(
            &id2,
            &single_buyer_vec(&s.env, &s.buyer),
            &s.supplier,
            &s.logistics,
            &s.arbiter,
            &s.token_id,
            &10_000i128,
            &build_milestones(&s.env),
            &opts,
        )
        .is_err());
}
