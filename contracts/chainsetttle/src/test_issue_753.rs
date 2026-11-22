#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, String, Symbol};

use crate::test_common::{default_options, setup, single_buyer_vec};

fn two_milestones(env: &Env) -> Vec<Milestone> {
    vec![
        env,
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
        Milestone {
            name: String::from_str(env, "delivery"),
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

#[test]
fn withholding_matches_preview_and_only_applies_to_jurisdiction() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);
    let authority = Address::generate(&s.env);
    let us = Symbol::new(&s.env, "US");
    client.set_timelock_duration(&s.buyer, &0u32);
    client.set_withholding_rule(&s.buyer, &us, &1_000u32, &authority);
    client.execute_withholding_rule(&s.buyer, &us);
    assert_eq!(client.get_withholding_rule(&us).unwrap().0, 1_000);

    let id = String::from_str(&s.env, "WH-1");
    let mut opts = default_options(&s.env);
    opts.jurisdiction = Some(us.clone());
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &two_milestones(&s.env),
        &opts,
    );
    let before = token.balance(&s.supplier);
    let auth_before = token.balance(&authority);
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    let preview = client.preview_milestone_payout(&id, &0);
    assert_eq!(preview.supplier_net_amount, 4_500);
    client.confirm_milestone(&s.buyer, &id, &0);
    assert_eq!(token.balance(&s.supplier) - before, 4_500);
    assert_eq!(token.balance(&authority) - auth_before, 500);

    // Other jurisdiction: no withholding.
    let id2 = String::from_str(&s.env, "WH-2");
    let mut opts2 = default_options(&s.env);
    opts2.jurisdiction = Some(Symbol::new(&s.env, "EU"));
    client.create_shipment(
        &id2,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &two_milestones(&s.env),
        &opts2,
    );
    let before2 = token.balance(&s.supplier);
    client.submit_proof(
        &s.supplier,
        &id2,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    assert_eq!(client.preview_milestone_payout(&id2, &0).supplier_net_amount, 5_000);
    client.confirm_milestone(&s.buyer, &id2, &0);
    assert_eq!(token.balance(&s.supplier) - before2, 5_000);
}
