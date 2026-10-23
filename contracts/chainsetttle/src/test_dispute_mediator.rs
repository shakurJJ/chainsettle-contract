#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{testutils::Address as _, token, vec, String, Symbol};

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn single_milestone(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Delivery"),
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

/// Creates a single-milestone shipment, submits proof and raises a dispute on it,
/// leaving milestone 0 in `Disputed` status ready for mediation.
fn setup_disputed_shipment(
    client: &ChainSettleContractClient,
    t: &crate::test_common::TestSetup,
    id: &str,
    total_amount: i128,
) -> String {
    let shipment_id = sid(&t.env, id);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://x"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    shipment_id
}

// ============================================================
// AUTHORIZATION
// ============================================================

#[test]
fn test_assigned_mediator_can_propose() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-assign", 1_000_000);

    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Supplier);

    let proposal = client.get_mediation_proposal(&shipment_id, &0).unwrap();
    assert_eq!(proposal.mediator, mediator);
    assert_eq!(proposal.suggested_outcome, Resolution::Supplier);
    assert!(!proposal.buyer_accepted);
    assert!(!proposal.supplier_accepted);
}

#[test]
fn test_pool_mediator_can_propose_without_specific_assignment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-pool", 1_000_000);

    client.set_mediator_pool(&t.buyer, &vec![&t.env, mediator.clone()]);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Buyer);

    assert!(client.get_mediation_proposal(&shipment_id, &0).is_some());
}

#[test]
#[should_panic(expected = "unauthorized mediator")]
fn test_unauthorized_mediator_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let not_a_mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-unauth", 1_000_000);

    client.propose_mediation(&not_a_mediator, &shipment_id, &0, &Resolution::Supplier);
}

#[test]
#[should_panic(expected = "milestone is not in disputed status")]
fn test_cannot_propose_mediation_on_non_disputed_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);

    let shipment_id = sid(&t.env, "med-not-disputed");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000i128,
        &single_milestone(&t.env),
        &default_options(&t.env),
    );
    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Supplier);
}

// ============================================================
// ACCEPTED MEDIATION
// ============================================================

#[test]
fn test_accepted_mediation_pays_supplier_and_bypasses_arbiter() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-accept-supplier", 1_000_000);

    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Supplier);

    let token_client = token::Client::new(&t.env, &t.token_id);
    let supplier_before = token_client.balance(&t.supplier);

    // Only buyer accepts so far — no payout yet.
    client.accept_mediation(&t.buyer, &shipment_id, &0);
    assert_eq!(token_client.balance(&t.supplier), supplier_before);
    let mid_proposal = client.get_mediation_proposal(&shipment_id, &0).unwrap();
    assert!(mid_proposal.buyer_accepted);
    assert!(!mid_proposal.supplier_accepted);

    // Supplier accepts too — outcome applies now.
    client.accept_mediation(&t.supplier, &shipment_id, &0);

    assert!(client.get_mediation_proposal(&shipment_id, &0).is_none());
    assert_eq!(token_client.balance(&t.supplier), supplier_before + 1_000_000);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Resolved);
    assert_eq!(shipment.open_dispute_count, 0);

    // Reputation credited as a completion, exactly like a normal resolve_dispute(approve=true).
    assert_eq!(client.get_reputation(&t.supplier).completed, 1);
}

#[test]
fn test_accepted_mediation_refunds_buyer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-accept-buyer", 1_000_000);

    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Buyer);

    let token_client = token::Client::new(&t.env, &t.token_id);
    let buyer_before = token_client.balance(&t.buyer);

    client.accept_mediation(&t.supplier, &shipment_id, &0);
    client.accept_mediation(&t.buyer, &shipment_id, &0);

    assert_eq!(token_client.balance(&t.buyer), buyer_before + 1_000_000);
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Resolved);
}

// ============================================================
// DECLINED MEDIATION → STANDARD DISPUTE FLOW UNAFFECTED
// ============================================================

#[test]
fn test_declined_mediation_falls_through_to_standard_dispute_flow() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-decline", 1_000_000);

    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Supplier);

    client.decline_mediation(&t.buyer, &shipment_id, &0);
    assert!(client.get_mediation_proposal(&shipment_id, &0).is_none());

    // Standard arbiter resolution still works normally on the still-open dispute.
    let token_client = token::Client::new(&t.env, &t.token_id);
    let supplier_before = token_client.balance(&t.supplier);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &true, &None);
    assert_eq!(token_client.balance(&t.supplier), supplier_before + 1_000_000);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
}

#[test]
fn test_declined_mediation_does_not_block_new_proposal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-decline-reprop", 1_000_000);

    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Supplier);
    client.decline_mediation(&t.supplier, &shipment_id, &0);

    // A fresh proposal can still be made on the still-open dispute.
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Buyer);
    assert!(client.get_mediation_proposal(&shipment_id, &0).is_some());
}

#[test]
#[should_panic(expected = "no pending mediation proposal")]
fn test_accept_without_proposal_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = setup_disputed_shipment(&client, &t, "med-no-proposal", 1_000_000);
    client.accept_mediation(&t.buyer, &shipment_id, &0);
}

#[test]
fn test_mediation_leaves_shipment_active_when_more_milestones_remain() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mediator = Address::generate(&t.env);

    let shipment_id = sid(&t.env, "med-partial-shipment");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000i128,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://x"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.assign_mediator(&t.buyer, &shipment_id, &mediator);
    client.propose_mediation(&mediator, &shipment_id, &0, &Resolution::Supplier);
    client.accept_mediation(&t.buyer, &shipment_id, &0);
    client.accept_mediation(&t.supplier, &shipment_id, &0);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Resolved);
}
