#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, String};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[test]
fn retry_with_same_key_returns_original_id_without_double_funding() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);
    client.set_idempotency_ttl(&s.buyer, 500);

    let key = String::from_str(&s.env, "wallet-timeout-retry-1");
    let opts = default_options(&s.env);
    let id1 = client.create_shipment_auto_id(
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &opts,
        &Some(key.clone()),
    );
    let id2 = client.create_shipment_auto_id(
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &opts,
        &Some(key),
    );
    assert_eq!(id1, id2);
    assert_eq!(token.balance(&s.contract_id), 10_000);
}

#[test]
fn key_reuse_with_different_params_is_rejected() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let key = String::from_str(&s.env, "wallet-timeout-retry-2");
    let opts = default_options(&s.env);
    client.create_shipment_auto_id(
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &opts,
        &Some(key.clone()),
    );
    assert!(client
        .try_create_shipment_auto_id(
            &single_buyer_vec(&s.env, &s.buyer),
            &s.supplier,
            &s.logistics,
            &s.arbiter,
            &s.token_id,
            &20_000i128,
            &build_milestones(&s.env),
            &opts,
            &Some(key),
        )
        .is_err());
}
