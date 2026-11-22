#![cfg(test)]

use super::*;
use crate::test_common::setup;
use soroban_sdk::{testutils::Address as _, token, BytesN};

fn profile(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[7u8; 32])
}

#[test]
fn approval_adds_applicant_to_pool_and_keeps_stake() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let applicant = Address::generate(&t.env);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&applicant, &1_000);
    client.set_arbiter_application_stake(&t.buyer, &t.token_id, &100);

    client.apply_to_arbiter_pool(&applicant, &profile(&t.env));
    assert_eq!(client.get_arbiter_stake(&applicant), 100);
    assert_eq!(
        client.get_arbiter_application(&applicant).unwrap().status,
        ApplicationStatus::Pending
    );

    client.review_arbiter_application(&t.buyer, &applicant, &true);
    assert!(client.get_arbiter_pool().contains(&applicant));
    assert_eq!(client.get_arbiter_stake(&applicant), 100);
    assert_eq!(
        client.get_arbiter_application(&applicant).unwrap().status,
        ApplicationStatus::Approved
    );
}

#[test]
fn rejection_refunds_stake_and_stays_out_of_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let applicant = Address::generate(&t.env);
    let token_client = token::StellarAssetClient::new(&t.env, &t.token_id);
    token_client.mint(&applicant, &1_000);
    client.set_arbiter_application_stake(&t.buyer, &t.token_id, &250);

    client.apply_to_arbiter_pool(&applicant, &profile(&t.env));
    client.review_arbiter_application(&t.buyer, &applicant, &false);

    assert_eq!(client.get_arbiter_stake(&applicant), 0);
    assert!(!client.get_arbiter_pool().contains(&applicant));
    assert_eq!(
        client.get_arbiter_application(&applicant).unwrap().status,
        ApplicationStatus::Rejected
    );
}

#[test]
fn applications_are_paginated_in_submission_order() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_arbiter_application_stake(&t.buyer, &t.token_id, &0);
    let a = Address::generate(&t.env);
    let b = Address::generate(&t.env);
    client.apply_to_arbiter_pool(&a, &profile(&t.env));
    client.apply_to_arbiter_pool(&b, &profile(&t.env));

    let first = client.get_arbiter_applications(&None, &1);
    assert_eq!(first.len(), 1);
    assert_eq!(first.get(0).unwrap().applicant, a);
    let second = client.get_arbiter_applications(&Some(1), &10);
    assert_eq!(second.len(), 1);
    assert_eq!(second.get(0).unwrap().applicant, b);
}
