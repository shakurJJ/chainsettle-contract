#![cfg(test)]

//! #745: Buyer purchase-order reference with per-buyer uniqueness and lookup.

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::String;

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn with_po(env: &Env, po: &str) -> ShipmentOptionsExt {
    ShipmentOptionsExt {
        po_reference: Some(String::from_str(env, po)),
        ..ShipmentOptionsExt::default()
    }
}

fn try_create(t: &TestSetup, id: &str, buyer: &Address, ext: ShipmentOptionsExt) -> bool {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client
        .try_create_shipment_ext(
            &sid(&t.env, id),
            &single_buyer_vec(&t.env, buyer),
            &t.supplier,
            &t.logistics,
            &t.arbiter,
            &t.token_id,
            &1_000_000,
            &build_milestones(&t.env),
            &default_options(&t.env),
            &ext,
        )
        .is_ok()
}

#[test]
fn test_lookup_by_po_returns_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(try_create(
        &t,
        "SHP-1",
        &t.buyer,
        with_po(&t.env, "PO-2026-0001")
    ));

    let found = client
        .get_shipment_by_po(&t.buyer, &String::from_str(&t.env, "PO-2026-0001"))
        .expect("PO should resolve");
    assert_eq!(found.id, sid(&t.env, "SHP-1"));
    assert_eq!(
        client.get_shipment_po_reference(&sid(&t.env, "SHP-1")),
        Some(String::from_str(&t.env, "PO-2026-0001"))
    );
}

#[test]
fn test_unknown_po_returns_none() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(try_create(&t, "SHP-1", &t.buyer, with_po(&t.env, "PO-A")));

    assert!(client
        .get_shipment_by_po(&t.buyer, &String::from_str(&t.env, "PO-UNKNOWN"))
        .is_none());
    // A known PO under a different buyer is also unknown.
    assert!(client
        .get_shipment_by_po(&t.buyer2, &String::from_str(&t.env, "PO-A"))
        .is_none());
}

#[test]
fn test_duplicate_po_for_same_buyer_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(try_create(&t, "SHP-1", &t.buyer, with_po(&t.env, "PO-DUP")));
    assert!(!try_create(
        &t,
        "SHP-2",
        &t.buyer,
        with_po(&t.env, "PO-DUP")
    ));

    // The rejected creation is rolled back entirely.
    assert!(client.try_get_shipment(&sid(&t.env, "SHP-2")).is_err());
    let found = client
        .get_shipment_by_po(&t.buyer, &String::from_str(&t.env, "PO-DUP"))
        .unwrap();
    assert_eq!(found.id, sid(&t.env, "SHP-1"));
}

#[test]
#[should_panic(expected = "duplicate po_reference for buyer")]
fn test_duplicate_po_panics_with_reason() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(try_create(&t, "SHP-1", &t.buyer, with_po(&t.env, "PO-DUP")));
    client.create_shipment_ext(
        &sid(&t.env, "SHP-2"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
        &with_po(&t.env, "PO-DUP"),
    );
}

#[test]
fn test_different_buyers_may_reuse_po() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(try_create(
        &t,
        "SHP-B1",
        &t.buyer,
        with_po(&t.env, "PO-SHARED")
    ));
    assert!(try_create(
        &t,
        "SHP-B2",
        &t.buyer2,
        with_po(&t.env, "PO-SHARED")
    ));

    let po = String::from_str(&t.env, "PO-SHARED");
    assert_eq!(
        client.get_shipment_by_po(&t.buyer, &po).unwrap().id,
        sid(&t.env, "SHP-B1")
    );
    assert_eq!(
        client.get_shipment_by_po(&t.buyer2, &po).unwrap().id,
        sid(&t.env, "SHP-B2")
    );
}

#[test]
fn test_po_length_bounds() {
    let t = setup();
    assert!(!try_create(&t, "SHP-EMPTY", &t.buyer, with_po(&t.env, "")));

    let max = "P".repeat(constants::MAX_PO_REFERENCE_LEN as usize);
    assert!(try_create(&t, "SHP-MAX", &t.buyer, with_po(&t.env, &max)));

    let too_long = "P".repeat(constants::MAX_PO_REFERENCE_LEN as usize + 1);
    assert!(!try_create(
        &t,
        "SHP-LONG",
        &t.buyer,
        with_po(&t.env, &too_long)
    ));
}

#[test]
#[should_panic(expected = "po_reference too long")]
fn test_po_too_long_panics_with_reason() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let too_long = "P".repeat(constants::MAX_PO_REFERENCE_LEN as usize + 1);
    client.create_shipment_ext(
        &sid(&t.env, "SHP-LONG"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
        &with_po(&t.env, &too_long),
    );
}

#[test]
fn test_shipment_without_po_has_none() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(try_create(
        &t,
        "SHP-NOPO",
        &t.buyer,
        ShipmentOptionsExt::default()
    ));
    assert_eq!(
        client.get_shipment_po_reference(&sid(&t.env, "SHP-NOPO")),
        None
    );
}
