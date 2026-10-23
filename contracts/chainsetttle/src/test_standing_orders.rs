#![cfg(test)]

//! #562 – Standing orders that auto-create recurring shipments.

extern crate std;

use super::*;
use crate::test_common::{setup, single_buyer_vec};
use soroban_sdk::{testutils::Ledger as _, vec, String};

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

fn order_params(t: &crate::test_common::TestSetup, amount: i128) -> StandingOrderParams {
    StandingOrderParams {
        template_name: String::from_str(&t.env, "weekly"),
        supplier: t.supplier.clone(),
        logistics: t.logistics.clone(),
        arbiter: t.arbiter.clone(),
        token: t.token_id.clone(),
        amount,
        milestones: single_milestone(&t.env),
        interval_ledgers: 10,
        max_occurrences: 3,
    }
}

#[test]
fn standing_order_happy_path_deterministic_ids() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let amount = 1_000_000i128;
    client.vault_deposit(&t.buyer, &t.token_id, &(amount * 3));

    let order_id = client.create_standing_order(&t.buyer, &order_params(&t, amount));
    let id1 = client.execute_standing_order(&order_id);
    assert_eq!(id1, String::from_str(&t.env, "so-1-0"));

    // Not due yet.
    let early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.execute_standing_order(&order_id);
    }));
    assert!(early.is_err());

    t.env.ledger().with_mut(|l| l.sequence_number += 10);
    let id2 = client.execute_standing_order(&order_id);
    assert_eq!(id2, String::from_str(&t.env, "so-1-1"));

    let order = client.get_standing_order(&order_id);
    assert_eq!(order.occurrences, 2);
    assert_eq!(order.buyer, t.buyer);
}

#[test]
fn stops_after_max_occurrences() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let amount = 500_000i128;
    client.vault_deposit(&t.buyer, &t.token_id, &(amount * 5));

    let mut params = order_params(&t, amount);
    params.max_occurrences = 2;
    params.interval_ledgers = 1;
    let order_id = client.create_standing_order(&t.buyer, &params);

    client.execute_standing_order(&order_id);
    t.env.ledger().with_mut(|l| l.sequence_number += 1);
    client.execute_standing_order(&order_id);
    t.env.ledger().with_mut(|l| l.sequence_number += 1);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.execute_standing_order(&order_id);
    }));
    assert!(result.is_err());
}

#[test]
fn cancel_blocks_further_execution() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.vault_deposit(&t.buyer, &t.token_id, &5_000_000i128);
    let order_id = client.create_standing_order(&t.buyer, &order_params(&t, 1_000_000));
    client.cancel_standing_order(&t.buyer, &order_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.execute_standing_order(&order_id);
    }));
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "insufficient vault balance")]
fn fails_cleanly_when_vault_insufficient() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // No vault deposit.
    let order_id = client.create_standing_order(&t.buyer, &order_params(&t, 1_000_000));
    client.execute_standing_order(&order_id);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn cancel_requires_owner() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.vault_deposit(&t.buyer, &t.token_id, &5_000_000i128);
    let order_id = client.create_standing_order(&t.buyer, &order_params(&t, 1_000_000));
    client.cancel_standing_order(&t.buyer2, &order_id);
}

#[test]
fn shipment_created_only_when_due() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.vault_deposit(&t.buyer, &t.token_id, &10_000_000i128);
    let order_id = client.create_standing_order(&t.buyer, &order_params(&t, 1_000_000));

    // First execute is allowed at creation ledger.
    let _ = client.execute_standing_order(&order_id);
    // Immediate re-execute must fail (interval not elapsed).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.execute_standing_order(&order_id);
    }));
    assert!(result.is_err());
}

// Keep single_buyer_vec import used for compile-time linkage with test_common patterns.
#[allow(dead_code)]
fn _touch(t: &crate::test_common::TestSetup) {
    let _ = single_buyer_vec(&t.env, &t.buyer);
}
