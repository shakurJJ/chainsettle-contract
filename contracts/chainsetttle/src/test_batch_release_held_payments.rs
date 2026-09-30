#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    vec, Address, Env, String, Vec,
};

#[test]
fn test_batch_release_held_payments_happy_path() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut opts1 = default_options(&t.env);
    opts1.holdback_ledgers = 100;

    let mut opts2 = default_options(&t.env);
    opts2.holdback_ledgers = 100;

    let shipment_id_1 = String::from_str(&t.env, "SHIP-HELD-001");
    let shipment_id_2 = String::from_str(&t.env, "SHIP-HELD-002");

    let milestones1 = build_milestones(&t.env);
    let milestones2 = build_milestones(&t.env);

    client.create_shipment(
        &shipment_id_1,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &milestones1,
        &opts1,
    );

    client.create_shipment(
        &shipment_id_2,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &milestones2,
        &opts2,
    );

    // Confirm milestone 0 on both shipments
    client.confirm_milestone(&t.buyer, &shipment_id_1, &0u32);
    client.confirm_milestone(&t.buyer, &shipment_id_2, &0u32);

    let ship1 = client.get_shipment(&shipment_id_1);
    let ship2 = client.get_shipment(&shipment_id_2);
    assert_eq!(ship1.milestones.get(0).unwrap().status, MilestoneStatus::ConfirmedHeld);
    assert_eq!(ship2.milestones.get(0).unwrap().status, MilestoneStatus::ConfirmedHeld);

    // Fast-forward ledger sequence past holdback
    t.env.ledger().with_mut(|li| {
        li.sequence_number += 150;
    });

    let items = vec![
        &t.env,
        (shipment_id_1.clone(), 0u32),
        (shipment_id_2.clone(), 0u32),
    ];

    let released = client.batch_release_held_payments(&items);
    assert_eq!(released.len(), 2);
    assert_eq!(released.get(0).unwrap(), (shipment_id_1.clone(), 0u32));
    assert_eq!(released.get(1).unwrap(), (shipment_id_2.clone(), 0u32));

    let ship1_after = client.get_shipment(&shipment_id_1);
    let ship2_after = client.get_shipment(&shipment_id_2);
    assert_eq!(ship1_after.milestones.get(0).unwrap().status, MilestoneStatus::Confirmed);
    assert_eq!(ship2_after.milestones.get(0).unwrap().status, MilestoneStatus::Confirmed);
}

#[test]
fn test_batch_release_held_payments_skips_unexpired() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut opts1 = default_options(&t.env);
    opts1.holdback_ledgers = 50; // short holdback

    let mut opts2 = default_options(&t.env);
    opts2.holdback_ledgers = 200; // long holdback

    let shipment_id_1 = String::from_str(&t.env, "SHIP-EXPIRED");
    let shipment_id_2 = String::from_str(&t.env, "SHIP-UNEXPIRED");

    client.create_shipment(
        &shipment_id_1,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &opts1,
    );

    client.create_shipment(
        &shipment_id_2,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &opts2,
    );

    client.confirm_milestone(&t.buyer, &shipment_id_1, &0u32);
    client.confirm_milestone(&t.buyer, &shipment_id_2, &0u32);

    // Fast-forward by 80 ledgers: ship1 expired (50), ship2 not expired (200)
    t.env.ledger().with_mut(|li| {
        li.sequence_number += 80;
    });

    let items = vec![
        &t.env,
        (shipment_id_1.clone(), 0u32),
        (shipment_id_2.clone(), 0u32),
    ];

    let released = client.batch_release_held_payments(&items);
    assert_eq!(released.len(), 1);
    assert_eq!(released.get(0).unwrap(), (shipment_id_1.clone(), 0u32));

    assert_eq!(
        client.get_shipment(&shipment_id_1).milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(
        client.get_shipment(&shipment_id_2).milestones.get(0).unwrap().status,
        MilestoneStatus::ConfirmedHeld
    );
}

#[test]
fn test_batch_release_skips_invalid_status_and_missing_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut opts = default_options(&t.env);
    opts.holdback_ledgers = 50;

    let shipment_id = String::from_str(&t.env, "SHIP-VALID");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &build_milestones(&t.env),
        &opts,
    );

    // Milestone 0 confirmed held
    client.confirm_milestone(&t.buyer, &shipment_id, &0u32);
    // Milestone 1 stays Pending

    t.env.ledger().with_mut(|li| {
        li.sequence_number += 100;
    });

    let items = vec![
        &t.env,
        (String::from_str(&t.env, "NON-EXISTENT"), 0u32),
        (shipment_id.clone(), 1u32), // Pending, not ConfirmedHeld
        (shipment_id.clone(), 99u32), // Out of bounds index
        (shipment_id.clone(), 0u32), // Valid ConfirmedHeld & expired
    ];

    let released = client.batch_release_held_payments(&items);
    assert_eq!(released.len(), 1);
    assert_eq!(released.get(0).unwrap(), (shipment_id.clone(), 0u32));
}

#[test]
#[should_panic(expected = "batch too large")]
fn test_batch_release_oversized_batch_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let mut items: Vec<(String, u32)> = Vec::new(&t.env);
    for i in 0..21u32 {
        items.push_back((String::from_str(&t.env, "SHIP-ID"), i));
    }

    client.batch_release_held_payments(&items);
}

#[test]
fn test_batch_release_empty_batch_returns_empty() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let items: Vec<(String, u32)> = Vec::new(&t.env);
    let released = client.batch_release_held_payments(&items);
    assert_eq!(released.len(), 0);
}

#[test]
#[should_panic(expected = "contract is paused")]
fn test_batch_release_paused_contract_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.pause(&t.buyer);

    let items = vec![&t.env, (String::from_str(&t.env, "SHIP-1"), 0u32)];
    client.batch_release_held_payments(&items);
}
