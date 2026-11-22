#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, String, Symbol};

use crate::test_common::{default_options, setup, single_buyer_vec};

fn milestones(env: &Env) -> Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "goods"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
            token: None,
        },
        Milestone {
            name: String::from_str(env, "local freight"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            not_before_ledger: 0,
            token: None,
        },
    ]
}

fn register_alt_token(s: &crate::test_common::TestSetup) -> Address {
    let admin = Address::generate(&s.env);
    let id = s.env.register_stellar_asset_contract_v2(admin.clone()).address();
    token::StellarAssetClient::new(&s.env, &id).mint(&s.buyer, &10_000_000);
    id
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
fn each_milestone_pays_in_its_own_token() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let alt = register_alt_token(&s);
    let alt_client = token::Client::new(&s.env, &alt);
    client.set_milestone_tokens(&vec![&s.env, None, Some(alt.clone())]);
    let id = String::from_str(&s.env, "MIX-1");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &milestones(&s.env),
        &default_options(&s.env),
    );
    assert_eq!(client.get_milestone_escrow(&id, &alt), 5_000);
    assert_eq!(client.get_milestone_token(&id, &1), Some(alt.clone()));
    let supplier_alt_before = alt_client.balance(&s.supplier);
    confirm(&client, &s, &id, 1);
    assert_eq!(alt_client.balance(&s.supplier) - supplier_alt_before, 5_000);
    assert_eq!(client.get_milestone_escrow(&id, &alt), 0);
}

#[test]
fn cancellation_refunds_each_token() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let alt = register_alt_token(&s);
    let alt_client = token::Client::new(&s.env, &alt);
    client.set_milestone_tokens(&vec![&s.env, None, Some(alt.clone())]);
    let id = String::from_str(&s.env, "MIX-2");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &milestones(&s.env),
        &default_options(&s.env),
    );
    let before = alt_client.balance(&s.buyer);
    client.cancel_shipment(&s.buyer, &id);
    assert_eq!(alt_client.balance(&s.buyer) - before, 5_000);
    assert_eq!(client.get_milestone_escrow(&id, &alt), 0);
}
