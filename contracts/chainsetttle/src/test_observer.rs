#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, Address, String, vec};
use crate::test_common::*;

// ============================================================
// SHIPMENT OBSERVER TESTS
// ============================================================

#[test]
fn test_add_observer_appears_in_list() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-1");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.buyer, &shipment_id, &observer);

    let observers = client.get_shipment_observers(&shipment_id);
    assert_eq!(observers.len(), 1);
    assert_eq!(observers.get(0).unwrap(), &observer);
}

#[test]
fn test_add_multiple_observers() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-2");
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

    let obs1 = Address::generate(&t.env);
    let obs2 = Address::generate(&t.env);
    let obs3 = Address::generate(&t.env);

    client.add_shipment_observer(&t.buyer, &shipment_id, &obs1);
    client.add_shipment_observer(&t.buyer, &shipment_id, &obs2);
    client.add_shipment_observer(&t.buyer, &shipment_id, &obs3);

    let observers = client.get_shipment_observers(&shipment_id);
    assert_eq!(observers.len(), 3);
    assert_eq!(observers.get(0).unwrap(), &obs1);
    assert_eq!(observers.get(1).unwrap(), &obs2);
    assert_eq!(observers.get(2).unwrap(), &obs3);
}

#[test]
fn test_add_duplicate_observer_is_idempotent() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-3");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.buyer, &shipment_id, &observer);
    client.add_shipment_observer(&t.buyer, &shipment_id, &observer);

    let observers = client.get_shipment_observers(&shipment_id);
    assert_eq!(observers.len(), 1);
}

#[test]
fn test_remove_observer_removes_from_list() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-4");
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

    let obs1 = Address::generate(&t.env);
    let obs2 = Address::generate(&t.env);

    client.add_shipment_observer(&t.buyer, &shipment_id, &obs1);
    client.add_shipment_observer(&t.buyer, &shipment_id, &obs2);

    client.remove_shipment_observer(&t.buyer, &shipment_id, &obs1);

    let observers = client.get_shipment_observers(&shipment_id);
    assert_eq!(observers.len(), 1);
    assert_eq!(observers.get(0).unwrap(), &obs2);
}

#[test]
fn test_remove_last_observer_clears_list() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-5");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.buyer, &shipment_id, &observer);

    client.remove_shipment_observer(&t.buyer, &shipment_id, &observer);

    let observers = client.get_shipment_observers(&shipment_id);
    assert_eq!(observers.len(), 0);
}

#[test]
fn test_get_observers_returns_empty_for_no_observers() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-6");
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

    let observers = client.get_shipment_observers(&shipment_id);
    assert_eq!(observers.len(), 0);
}

#[test]
fn test_observer_has_no_confirm_authority() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-7");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.buyer, &shipment_id, &observer);

    // Submit proof for first milestone.
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://proof"),
        &Symbol::new(&t.env, "ipfs"),
    );

    // Observer should NOT be able to confirm the milestone.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.confirm_milestone(&observer, &shipment_id, &0);
    }));
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_buyer_cannot_add_observer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-8");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.supplier, &shipment_id, &observer);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_buyer_cannot_remove_observer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-9");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.buyer, &shipment_id, &observer);

    client.remove_shipment_observer(&t.supplier, &shipment_id, &observer);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_buyer2_cannot_add_observer_to_buyer1_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-OBS-10");
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

    let observer = Address::generate(&t.env);
    client.add_shipment_observer(&t.buyer2, &shipment_id, &observer);
}
