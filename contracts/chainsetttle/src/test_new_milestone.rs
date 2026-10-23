#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{create_standard_shipment, setup, submit_proof_for_milestone, TestSetup};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, vec, Address, String, Symbol, TryFromVal};

const TOTAL: i128 = 1_000_000;

fn create(t: &TestSetup, client: &ChainSettleContractClient, id: &str) -> String {
    let shipment_id = String::from_str(&t.env, id);
    create_standard_shipment(
        client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        TOTAL,
    );
    shipment_id
}

fn name(t: &TestSetup) -> String {
    String::from_str(&t.env, "Extra Inspection")
}

fn balance(t: &TestSetup, who: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(who)
}

fn has_event(t: &TestSetup, wanted: &str) -> bool {
    let wanted = Symbol::new(&t.env, wanted);
    t.env.events().all().iter().any(|(_, topics, _)| {
        topics
            .get(0)
            .and_then(|v| Symbol::try_from_val(&t.env, &v).ok())
            .map(|sym| sym == wanted)
            .unwrap_or(false)
    })
}

fn release_all(t: &TestSetup, client: &ChainSettleContractClient, id: &String, from: u32) {
    let count = client.get_shipment(id).milestones.len();
    for i in from..count {
        submit_proof_for_milestone(t, id, i);
        client.confirm_milestone(&t.buyer, id, &i);
    }
}

fn percent_sum(shipment: &Shipment) -> u32 {
    shipment.milestones.iter().map(|m| m.payment_percent).sum()
}

// ── Happy paths ─────────────────────────────────────────────────────────────

#[test]
fn test_supplier_proposes_buyer_accepts_and_funds() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-S2B");

    client.propose_new_milestone(&t.supplier, &id, &name(&t), &250_000);
    assert!(has_event(&t, "new_milestone_proposed"));
    let proposal = client.get_new_milestone_proposal(&id).unwrap();
    assert_eq!(proposal.proposer, t.supplier);
    assert_eq!(proposal.amount, 250_000);
    assert_eq!(proposal.funded_by, None);

    let buyer_before = balance(&t, &t.buyer);
    let contract_before = balance(&t, &t.contract_id);
    let tvl_before = client.get_total_escrowed_value(&t.token_id);

    client.accept_new_milestone(&t.buyer, &id);
    assert!(has_event(&t, "new_milestone_accepted"));

    assert_eq!(buyer_before - balance(&t, &t.buyer), 250_000);
    assert_eq!(balance(&t, &t.contract_id) - contract_before, 250_000);
    assert_eq!(client.get_total_escrowed_value(&t.token_id) - tvl_before, 250_000);

    let shipment = client.get_shipment(&id);
    assert_eq!(shipment.total_amount, TOTAL + 250_000);
    assert_eq!(shipment.milestones.len(), 4);
    assert_eq!(percent_sum(&shipment), 100);
    let new_m = shipment.milestones.get(3).unwrap();
    assert_eq!(new_m.name, name(&t));
    assert_eq!(new_m.status, MilestoneStatus::Pending);
    let percents: std::vec::Vec<u32> =
        shipment.milestones.iter().map(|m| m.payment_percent).collect();
    assert_eq!(percents, std::vec![20u32, 40, 20, 20]);
    assert!(client.get_new_milestone_proposal(&id).is_none());

    let last = shipment.audit_log.get(shipment.audit_log.len() - 1).unwrap();
    assert_eq!(last.action, Symbol::new(&t.env, "milestone_appended"));
    assert_eq!(last.caller, t.buyer);

    // Every milestone keeps its absolute value; supplier receives the full new total.
    let supplier_before = balance(&t, &t.supplier);
    release_all(&t, &client, &id, 0);
    assert_eq!(balance(&t, &t.supplier) - supplier_before, TOTAL + 250_000);
    assert_eq!(client.get_shipment(&id).status, ShipmentStatus::Completed);
}

#[test]
fn test_buyer_proposal_is_prefunded_and_supplier_accepts() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-B2S");

    let buyer_before = balance(&t, &t.buyer);
    client.propose_new_milestone(&t.buyer, &id, &name(&t), &100_000);
    assert_eq!(buyer_before - balance(&t, &t.buyer), 100_000);
    assert_eq!(
        client.get_new_milestone_proposal(&id).unwrap().funded_by,
        Some(t.buyer.clone())
    );
    // Escrow only grows on acceptance.
    assert_eq!(client.get_shipment(&id).total_amount, TOTAL);

    let contract_before = balance(&t, &t.contract_id);
    client.accept_new_milestone(&t.supplier, &id);
    assert_eq!(balance(&t, &t.contract_id), contract_before);
    assert_eq!(buyer_before - balance(&t, &t.buyer), 100_000);
    assert_eq!(client.get_shipment(&id).total_amount, TOTAL + 100_000);
}

#[test]
fn test_existing_milestone_values_preserved_with_uneven_amount() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-UNEVEN");

    client.propose_new_milestone(&t.supplier, &id, &name(&t), &333_333);
    client.accept_new_milestone(&t.buyer, &id);

    let shipment = client.get_shipment(&id);
    assert_eq!(shipment.total_amount, TOTAL + 333_333);
    assert_eq!(percent_sum(&shipment), 100);

    let originals = [250_000i128, 500_000, 250_000];
    let mut paid_total = 0i128;
    for i in 0..4u32 {
        let before = balance(&t, &t.supplier);
        submit_proof_for_milestone(&t, &id, i);
        client.confirm_milestone(&t.buyer, &id, &i);
        let paid = balance(&t, &t.supplier) - before;
        if (i as usize) < originals.len() {
            assert!(paid >= originals[i as usize], "existing milestone shrank");
        } else {
            assert!(paid <= 333_333 && paid > 0);
        }
        paid_total += paid;
    }
    assert!(paid_total <= TOTAL + 333_333);
}

#[test]
fn test_append_after_partial_release() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-PARTIAL");

    let supplier_before = balance(&t, &t.supplier);
    submit_proof_for_milestone(&t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    client.propose_new_milestone(&t.buyer, &id, &name(&t), &250_000);
    client.accept_new_milestone(&t.supplier, &id);

    release_all(&t, &client, &id, 1);
    assert_eq!(balance(&t, &t.supplier) - supplier_before, TOTAL + 250_000);
}

#[test]
fn test_cancel_by_proposer_refunds_deposit() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-CANCEL");

    let buyer_before = balance(&t, &t.buyer);
    client.propose_new_milestone(&t.buyer, &id, &name(&t), &100_000);
    client.cancel_new_milestone(&t.buyer, &id);
    assert!(has_event(&t, "new_milestone_cancelled"));

    assert_eq!(balance(&t, &t.buyer), buyer_before);
    assert!(client.get_new_milestone_proposal(&id).is_none());
    assert_eq!(client.get_shipment(&id).milestones.len(), 3);
}

#[test]
fn test_counterparty_can_reject_and_deposit_is_refunded() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-REJECT");

    let buyer_before = balance(&t, &t.buyer);
    client.propose_new_milestone(&t.buyer, &id, &name(&t), &100_000);
    client.cancel_new_milestone(&t.supplier, &id);
    assert_eq!(balance(&t, &t.buyer), buyer_before);

    let shipment = client.get_shipment(&id);
    let last = shipment.audit_log.get(shipment.audit_log.len() - 1).unwrap();
    assert_eq!(last.action, Symbol::new(&t.env, "new_ms_cancelled"));
}

#[test]
fn test_deposit_refundable_after_shipment_cancelled() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-SHIPCANCEL");

    client.propose_new_milestone(&t.buyer, &id, &name(&t), &100_000);
    client.cancel_shipment(&t.buyer, &id);

    let buyer_before = balance(&t, &t.buyer);
    client.cancel_new_milestone(&t.buyer, &id);
    assert_eq!(balance(&t, &t.buyer) - buyer_before, 100_000);
}

// ── Authorization failures ──────────────────────────────────────────────────

#[test]
#[should_panic(expected = "milestone must be accepted by the counterparty")]
fn test_proposer_cannot_accept_own_proposal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-SELF");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    client.accept_new_milestone(&t.supplier, &id);
}

#[test]
#[should_panic(expected = "milestone must be accepted by the counterparty")]
fn test_co_buyer_cannot_accept_buyer_proposal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = String::from_str(&t.env, "NM-COBUYER");
    client.create_shipment(
        &id,
        &vec![&t.env, t.buyer.clone(), t.buyer2.clone()],
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &crate::test_common::build_milestones(&t.env),
        &crate::test_common::default_options(&t.env),
    );
    client.propose_new_milestone(&t.buyer, &id, &name(&t), &100_000);
    client.accept_new_milestone(&t.buyer2, &id);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_stranger_cannot_propose() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-STRANGER");
    let stranger = Address::generate(&t.env);
    client.propose_new_milestone(&stranger, &id, &name(&t), &100_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_stranger_cannot_accept() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-STRANGER2");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    let stranger = Address::generate(&t.env);
    client.accept_new_milestone(&stranger, &id);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_stranger_cannot_cancel() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-STRANGER3");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    let stranger = Address::generate(&t.env);
    client.cancel_new_milestone(&stranger, &id);
}

#[test]
fn test_propose_requires_signature() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-SIG");
    t.env.set_auths(&[]);
    assert!(client
        .try_propose_new_milestone(&t.supplier, &id, &name(&t), &100_000)
        .is_err());
}

#[test]
fn test_accept_requires_signature() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-SIG2");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    t.env.set_auths(&[]);
    assert!(client.try_accept_new_milestone(&t.buyer, &id).is_err());
}

// ── Validation / edge cases ─────────────────────────────────────────────────

#[test]
#[should_panic(expected = "milestone amount must be positive")]
fn test_zero_amount_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-ZERO");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &0);
}

#[test]
#[should_panic(expected = "milestone name must not be empty")]
fn test_empty_name_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-NAME");
    client.propose_new_milestone(&t.supplier, &id, &String::from_str(&t.env, ""), &100);
}

#[test]
#[should_panic(expected = "milestone amount too small")]
fn test_dust_amount_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-DUST");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &1);
    client.accept_new_milestone(&t.buyer, &id);
}

#[test]
#[should_panic(expected = "new milestone proposal already pending")]
fn test_only_one_pending_proposal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-DOUBLE");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    client.propose_new_milestone(&t.buyer, &id, &name(&t), &100_000);
}

#[test]
#[should_panic(expected = "TooManyMilestones")]
fn test_milestone_cap_enforced_on_propose() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-CAP");
    client.set_max_milestone_count(&t.buyer, &3);
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
}

#[test]
#[should_panic(expected = "TooManyMilestones")]
fn test_milestone_cap_rechecked_on_accept() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-CAP2");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    client.set_max_milestone_count(&t.buyer, &3);
    client.accept_new_milestone(&t.buyer, &id);
}

#[test]
#[should_panic(expected = "no pending new milestone proposal")]
fn test_accept_without_proposal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-NONE");
    client.accept_new_milestone(&t.buyer, &id);
}

#[test]
#[should_panic(expected = "shipment is not active")]
fn test_cannot_propose_on_cancelled_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-INACTIVE");
    client.cancel_shipment(&t.buyer, &id);
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
}

#[test]
#[should_panic(expected = "shipment is not active")]
fn test_cannot_accept_on_cancelled_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let id = create(&t, &client, "NM-INACTIVE2");
    client.propose_new_milestone(&t.supplier, &id, &name(&t), &100_000);
    client.cancel_shipment(&t.buyer, &id);
    client.accept_new_milestone(&t.buyer, &id);
}
