#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{BytesN, String};

fn create(client: &ChainSettleContractClient, t: &TestSetup, id: &str, supplier: &Address) {
    client.create_shipment(
        &String::from_str(&t.env, id),
        &single_buyer_vec(&t.env, &t.buyer),
        supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn test_admin_manages_exemption_list() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    assert!(!client.is_rate_limit_exempt(&t.supplier));
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);
    assert!(client.is_rate_limit_exempt(&t.supplier));
    client.remove_rate_limit_exemption(&t.buyer, &t.supplier);
    assert!(!client.is_rate_limit_exempt(&t.supplier));
}

#[test]
#[should_panic]
fn test_non_admin_cannot_add_exemption() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.add_rate_limit_exemption(&t.supplier, &t.supplier);
}

#[test]
fn test_rate_limit_config_set_get_and_clear() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    assert_eq!(client.get_shipment_creation_rate_limit(), None);
    client.set_shipment_creation_rate_limit(&t.buyer, &2u32, &100u32);
    assert_eq!(client.get_shipment_creation_rate_limit(), Some((2, 100)));
    client.set_shipment_creation_rate_limit(&t.buyer, &0u32, &0u32);
    assert_eq!(client.get_shipment_creation_rate_limit(), None);
}

#[test]
#[should_panic(expected = "shipment creation rate limit exceeded")]
fn test_non_exempt_supplier_is_rate_limited() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_shipment_creation_rate_limit(&t.buyer, &2u32, &100u32);

    create(&client, &t, "ne-1", &t.supplier);
    create(&client, &t, "ne-2", &t.supplier);
    create(&client, &t, "ne-3", &t.supplier);
}

#[test]
fn test_rate_limit_window_resets() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_shipment_creation_rate_limit(&t.buyer, &1u32, &100u32);

    create(&client, &t, "w-1", &t.supplier);
    t.env.ledger().with_mut(|l| l.sequence_number += 100);
    create(&client, &t, "w-2", &t.supplier);
}

#[test]
fn test_exempt_supplier_bypasses_rate_limit() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_shipment_creation_rate_limit(&t.buyer, &1u32, &100u32);
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);

    for i in 0..5 {
        create(&client, &t, &std::format!("ex-{}", i), &t.supplier);
    }
    assert_eq!(client.get_shipment_count(&t.supplier), 5);
}

#[test]
fn test_exemption_does_not_affect_other_suppliers() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_shipment_creation_rate_limit(&t.buyer, &1u32, &100u32);
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);

    let other = Address::generate(&t.env);
    create(&client, &t, "o-1", &other);
    let result = client.try_create_shipment(
        &String::from_str(&t.env, "o-2"),
        &single_buyer_vec(&t.env, &t.buyer),
        &other,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    assert!(
        result.is_err(),
        "non-exempt supplier must still be rate limited"
    );
}

#[test]
fn test_removed_exemption_restores_rate_limit() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_shipment_creation_rate_limit(&t.buyer, &1u32, &100u32);
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);
    create(&client, &t, "r-1", &t.supplier);
    client.remove_rate_limit_exemption(&t.buyer, &t.supplier);

    create(&client, &t, "r-2", &t.supplier);
    let result = client.try_create_shipment(
        &String::from_str(&t.env, "r-3"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_exempt_supplier_still_subject_to_blacklist() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_shipment_creation_rate_limit(&t.buyer, &1u32, &100u32);
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);
    client.blacklist_address(
        &t.buyer,
        &t.supplier,
        &BytesN::from_array(&t.env, &[1u8; 32]),
    );

    create(&client, &t, "bl-1", &t.supplier);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_exempt_supplier_still_subject_to_whitelist() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);
    let other = Address::generate(&t.env);
    client.add_to_whitelist(&t.buyer, &other);

    create(&client, &t, "wl-1", &t.supplier);
}

#[test]
#[should_panic(expected = "total amount exceeds maximum shipment value")]
fn test_exempt_supplier_still_subject_to_value_bounds() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.add_rate_limit_exemption(&t.buyer, &t.supplier);
    client.set_max_shipment_value(&t.buyer, &100i128);

    create(&client, &t, "vb-1", &t.supplier);
}
