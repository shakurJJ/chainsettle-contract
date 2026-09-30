#![cfg(test)]

use super::*;
use crate::test_common::{build_milestones, default_options, setup, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger as _},
    Address, String, Symbol,
};

fn create_test_shipment(t: &TestSetup, id: &str) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, id);
    let milestones = build_milestones(&t.env);
    let options = default_options(&t.env);
    let buyers = soroban_sdk::vec![&t.env, t.buyer.clone()];

    client.create_shipment(
        &shipment_id,
        &buyers,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &milestones,
        &options,
    );

    shipment_id
}

#[test]
fn test_extend_shipment_ttl_happy_path() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = create_test_shipment(&t, "SHIP-TTL-1");

    // Add notes, advance request, and dispute to instantiate related persistent keys
    client.add_milestone_note(
        &t.buyer,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "Initial note"),
    );
    client.request_advance(&t.supplier, &shipment_id, &0, &20);
    client.raise_dispute(
        &t.buyer,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "Packaging damaged"),
    );

    // Snapshot shipment state before TTL extension
    let shipment_before = client.get_shipment(&shipment_id);

    // Fast-forward ledger so TTL decrements
    t.env.ledger().set_sequence_number(50_000);

    // Check that TTL is lower before extension
    t.env.as_contract(&t.contract_id, || {
        let shipment_key = DataKey::Shipment(shipment_id.clone());
        let ttl_before = t.env.storage().persistent().get_ttl(&shipment_key);
        assert!(ttl_before <= constants::TTL_MAX_LEDGERS - 50_000 + 10);
    });

    // Permissionless call: any arbitrary caller can extend TTL
    client.extend_shipment_ttl(&shipment_id);

    // Verify persistent TTLs were extended
    t.env.as_contract(&t.contract_id, || {
        let shipment_key = DataKey::Shipment(shipment_id.clone());
        let ttl_after = t.env.storage().persistent().get_ttl(&shipment_key);
        assert!(
            ttl_after >= constants::TTL_MAX_LEDGERS - 100,
            "Shipment TTL should be bumped to TTL_MAX_LEDGERS, got {}",
            ttl_after
        );

        let note_key = DataKeyExt::MilestoneNotes(shipment_id.clone(), 0);
        let note_ttl = t.env.storage().persistent().get_ttl(&note_key);
        assert!(
            note_ttl >= constants::TTL_MAX_LEDGERS - 100,
            "MilestoneNotes TTL should be bumped to TTL_MAX_LEDGERS, got {}",
            note_ttl
        );

        let advance_key = DataKey::AdvanceRequest(shipment_id.clone(), 0);
        let advance_ttl = t.env.storage().persistent().get_ttl(&advance_key);
        assert!(
            advance_ttl >= constants::TTL_MAX_LEDGERS - 100,
            "AdvanceRequest TTL should be bumped to TTL_MAX_LEDGERS, got {}",
            advance_ttl
        );

        let dispute_key = DataKeyExt::DisputeOpenedAt(shipment_id.clone(), 1);
        let dispute_ttl = t.env.storage().persistent().get_ttl(&dispute_key);
        assert!(
            dispute_ttl >= constants::TTL_MAX_LEDGERS - 100,
            "DisputeOpenedAt TTL should be bumped to TTL_MAX_LEDGERS, got {}",
            dispute_ttl
        );
    });

    // Verify NO side effects on shipment state
    let shipment_after = client.get_shipment(&shipment_id);
    assert_eq!(shipment_before.id, shipment_after.id);
    assert_eq!(shipment_before.status, shipment_after.status);
    assert_eq!(shipment_before.total_amount, shipment_after.total_amount);
    assert_eq!(
        shipment_before.released_amount,
        shipment_after.released_amount
    );
    assert_eq!(
        shipment_before.audit_log.len(),
        shipment_after.audit_log.len()
    );
    assert_eq!(
        shipment_before.milestones.len(),
        shipment_after.milestones.len()
    );
    for i in 0..shipment_before.milestones.len() {
        let mb = shipment_before.milestones.get(i).unwrap();
        let ma = shipment_after.milestones.get(i).unwrap();
        assert_eq!(mb.status, ma.status);
        assert_eq!(mb.payment_percent, ma.payment_percent);
        assert_eq!(mb.proof_hash, ma.proof_hash);
    }

    // Verify event was emitted
    let events = t.env.events().all();
    let found = events.iter().any(|e| {
        if e.0 == t.contract_id {
            // Check topic
            if e.1.len() >= 2 {
                let topic0: Symbol = e.1.get(0).unwrap().try_into().unwrap_or(Symbol::new(&t.env, ""));
                return topic0 == Symbol::new(&t.env, "shipment_ttl_extended");
            }
        }
        false
    });
    assert!(found, "shipment_ttl_extended event must be published");
}

#[test]
#[should_panic(expected = "shipment not found")]
fn test_extend_shipment_ttl_unknown_shipment_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let unknown_id = String::from_str(&t.env, "NON-EXISTENT-SHIPMENT");

    client.extend_shipment_ttl(&unknown_id);
}

#[test]
fn test_extend_shipment_ttl_minimal_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = create_test_shipment(&t, "SHIP-MINIMAL");

    // Fast-forward ledger
    t.env.ledger().set_sequence_number(80_000);

    // Call extend_shipment_ttl on shipment with no optional keys attached
    client.extend_shipment_ttl(&shipment_id);

    t.env.as_contract(&t.contract_id, || {
        let shipment_key = DataKey::Shipment(shipment_id.clone());
        let ttl_after = t.env.storage().persistent().get_ttl(&shipment_key);
        assert!(ttl_after >= constants::TTL_MAX_LEDGERS - 100);
    });
}

#[test]
fn test_extend_shipment_ttl_idempotent() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = create_test_shipment(&t, "SHIP-IDEMPOTENT");

    // Calling multiple times should succeed without error or state change
    client.extend_shipment_ttl(&shipment_id);
    client.extend_shipment_ttl(&shipment_id);
    client.extend_shipment_ttl(&shipment_id);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.id, shipment_id);
}

#[test]
fn test_extend_shipment_ttl_permissionless_caller() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = create_test_shipment(&t, "SHIP-CALLER");

    // Arbitrary unprivileged third party
    let _stranger = Address::generate(&t.env);

    // Should succeed with no authentication error
    client.extend_shipment_ttl(&shipment_id);
}
