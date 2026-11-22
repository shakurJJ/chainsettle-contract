#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, String, Symbol};

use crate::test_common::{default_options, setup, single_buyer_vec};

fn milestones(env: &Env, customs: bool) -> Vec<Milestone> {
    let name = if customs { "customs" } else { "goods" };
    vec![
        env,
        Milestone {
            name: String::from_str(env, name),
            token: None,
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "goods"),
            token: None,
            payment_percent: 50,
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

fn confirm(client: &ChainSettleContractClient<'_>, s: &crate::test_common::TestSetup, id: &String, i: u32) {
    client.submit_proof(
        &s.supplier,
        id,
        &i,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    client.confirm_milestone(&s.buyer, id, &i);
}

#[test]
fn broker_paid_exactly_once_on_customs_clearance() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);
    let broker = Address::generate(&s.env);
    client.set_duties_config(&broker, &1_000i128);
    let id = String::from_str(&s.env, "DUTY-1");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &milestones(&s.env, true),
        &default_options(&s.env),
    );
    let before = token.balance(&broker);
    confirm(&client, &s, &id, 0);
    assert_eq!(token.balance(&broker) - before, 1_000);
    confirm(&client, &s, &id, 1);
    client.release_duties(&id);
    assert_eq!(token.balance(&broker) - before, 1_000);
    assert_eq!(client.get_duties_escrow(&id).unwrap().1, 0);
}

#[test]
fn unused_duties_refunded_on_completion() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let broker = Address::generate(&s.env);
    client.set_duties_config(&broker, &1_000i128);
    let id = String::from_str(&s.env, "DUTY-2");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &milestones(&s.env, false),
        &default_options(&s.env),
    );
    confirm(&client, &s, &id, 0);
    confirm(&client, &s, &id, 1);
    assert_eq!(client.get_duties_escrow(&id).unwrap().1, 0);
}
