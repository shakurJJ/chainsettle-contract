#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec};
use soroban_sdk::{testutils::Ledger, vec, String, Symbol};

fn proof_hash(env: &soroban_sdk::Env) -> String {
    String::from_str(env, "ipfs://proof")
}

fn proof_type(env: &soroban_sdk::Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

fn single_milestone(env: &soroban_sdk::Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "All"),
            payment_percent: 100,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
        },
    ]
}

fn create_ship(
    client: &ChainSettleContractClient,
    t: &crate::test_common::TestSetup,
    id: &str,
) -> String {
    let ship_id = String::from_str(&t.env, id);
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
    ship_id
}

fn seed_reputation(t: &crate::test_common::TestSetup, completed: u32, disputed: u32) {
    t.env.as_contract(&t.contract_id, || {
        t.env.storage().persistent().set(
            &DataKey::SupplierRep(t.supplier.clone()),
            &ReputationScore {
                completed,
                disputed,
                cancelled: 0,
            },
        );
    });
}

// ============================================================
// TASK 1 — Reputation fast-track
// ============================================================

#[test]
fn test_fast_track_disabled_by_default() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    seed_reputation(&t, 100, 0);
    assert!(!client.is_fast_track_eligible(&t.supplier));
}

#[test]
#[should_panic(expected = "confirmation cooldown not elapsed")]
fn test_fast_track_non_qualifying_still_gated() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_confirmation_cooldown(&t.buyer, &50u32);
    client.set_reputation_fast_track(&t.buyer, &5u32, &1000u32);
    seed_reputation(&t, 2, 0); // below min_completed
    assert!(!client.is_fast_track_eligible(&t.supplier));

    t.env.ledger().set_sequence_number(100);
    let ship_id = create_ship(&client, &t, "ft-no");
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.confirm_milestone(&t.buyer, &ship_id, &0u32);
}

#[test]
fn test_fast_track_high_dispute_ratio_ineligible() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_reputation_fast_track(&t.buyer, &5u32, &1000u32); // 10%
    seed_reputation(&t, 10, 2); // 20% disputed
    assert!(!client.is_fast_track_eligible(&t.supplier));
}

// ============================================================
// TASK 2 — Per-shipment mutual-consent pause
// ============================================================

#[test]
fn test_pause_one_sided_request_no_effect() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = create_ship(&client, &t, "pause-req");
    client.request_shipment_pause(&t.buyer, &ship_id);
    assert!(!client.is_shipment_paused(&ship_id));
    // Still mutable.
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
}

#[test]
#[should_panic(expected = "shipment is paused")]
fn test_pause_blocks_proof_after_approval() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = create_ship(&client, &t, "pause-block");
    client.request_shipment_pause(&t.buyer, &ship_id);
    client.approve_shipment_pause(&t.supplier, &ship_id);
    assert!(client.is_shipment_paused(&ship_id));
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
}

#[test]
fn test_pause_isolation_other_shipment_unaffected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let a = create_ship(&client, &t, "pause-a");
    let b = create_ship(&client, &t, "pause-b");
    client.request_shipment_pause(&t.buyer, &a);
    client.approve_shipment_pause(&t.supplier, &a);
    assert!(client.is_shipment_paused(&a));
    assert!(!client.is_shipment_paused(&b));
    client.submit_proof(
        &t.supplier,
        &b,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.confirm_milestone(&t.buyer, &b, &0u32);
}

// ============================================================
// TASK 3 — Milestone notes
// ============================================================

#[test]
fn test_milestone_notes_multiparty_and_cap() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = create_ship(&client, &t, "notes-1");

    client.add_milestone_note(
        &t.buyer,
        &ship_id,
        &0u32,
        &String::from_str(&t.env, "buyer note"),
    );
    client.add_milestone_note(
        &t.supplier,
        &ship_id,
        &0u32,
        &String::from_str(&t.env, "supplier note"),
    );
    client.add_milestone_note(
        &t.logistics,
        &ship_id,
        &0u32,
        &String::from_str(&t.env, "logistics note"),
    );

    let notes = client.get_milestone_notes(&ship_id, &0u32);
    assert_eq!(notes.len(), 3);
    assert_eq!(notes.get(0).unwrap().author, t.buyer);
    assert_eq!(notes.get(1).unwrap().author, t.supplier);
    assert_eq!(notes.get(2).unwrap().author, t.logistics);

    // Overflow cap (10): oldest dropped.
    for i in 0..10u32 {
        let s = std::format!("n{}", i);
        client.add_milestone_note(&t.buyer, &ship_id, &0u32, &String::from_str(&t.env, &s));
    }
    let notes = client.get_milestone_notes(&ship_id, &0u32);
    assert_eq!(notes.len(), 10);
    // First three original notes dropped; first remaining is from overflow loop.
    assert_eq!(notes.get(0).unwrap().author, t.buyer);

    // Notes must not alter milestone status.
    assert_eq!(
        client.get_milestone(&ship_id, &0u32).status,
        MilestoneStatus::Pending
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_milestone_notes_outsider_denied() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = create_ship(&client, &t, "notes-deny");
    client.add_milestone_note(
        &t.arbiter,
        &ship_id,
        &0u32,
        &String::from_str(&t.env, "nope"),
    );
}

// ============================================================
// TASK 4 — Shipment archival
// ============================================================

#[test]
#[should_panic(expected = "shipment not old enough to archive")]
fn test_archive_too_young_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_archive_threshold(&t.buyer, &1000u32);
    t.env.ledger().set_sequence_number(10);
    let ship_id = create_ship(&client, &t, "arch-young");
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.confirm_milestone(&t.buyer, &ship_id, &0u32);
    assert_eq!(
        client.get_shipment(&ship_id).status,
        ShipmentStatus::Completed
    );
    // Only advanced a little past creation.
    t.env.ledger().set_sequence_number(50);
    client.archive_shipment(&t.buyer, &ship_id);
}

#[test]
#[should_panic(expected = "only completed or cancelled shipments can be archived")]
fn test_archive_active_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_archive_threshold(&t.buyer, &1u32);
    let ship_id = create_ship(&client, &t, "arch-active");
    client.archive_shipment(&t.buyer, &ship_id);
}

#[test]
fn test_archive_success_and_query() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_archive_threshold(&t.buyer, &100u32);
    t.env.ledger().set_sequence_number(10);
    let ship_id = create_ship(&client, &t, "arch-ok");
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &proof_hash(&t.env),
        &proof_type(&t.env),
    );
    client.confirm_milestone(&t.buyer, &ship_id, &0u32);

    t.env.ledger().set_sequence_number(10 + 100);
    client.archive_shipment(&t.buyer, &ship_id);

    let archived = client.get_archived_shipment(&ship_id);
    assert_eq!(archived.id, ship_id);
    assert_eq!(archived.buyer, t.buyer);
    assert_eq!(archived.supplier, t.supplier);
    assert_eq!(archived.status, ShipmentStatus::Completed);
    assert_eq!(archived.total_amount, 1_000_000);
    assert_eq!(archived.released_amount, 1_000_000);
    assert_eq!(archived.completed_at, 10);
}
