#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{testutils::Ledger as _, vec, Env, String, Symbol};

fn sid(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

fn ipfs(env: &Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

// ============================================================
// Issue #42 — Minimum shipment value
// ============================================================

#[test]
fn test_min_shipment_value_default_zero_allows_any_amount() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // Default min = 0 (disabled) — small shipment must succeed
    client.create_shipment(
        &sid(&t.env, "min-default"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1,
        &vec![
            &t.env,
            Milestone {
                name: sid(&t.env, "M"),
                payment_percent: 100,
                proof_hash: sid(&t.env, ""),
                status: MilestoneStatus::Pending,
                release_after_ledger: 0,
                proof_submitted_ledger: None,
                dispute_opened_ledger: None,
                deadline_ledger: 0,
                penalty_bps_per_ledger: 0,
                not_before_ledger: 0,
            },
        ],
        &default_options(&t.env),
    );
    assert_eq!(client.get_min_shipment_value(), 0);
}

#[test]
#[should_panic(expected = "MinShipmentValueNotMet")]
fn test_min_shipment_value_below_floor_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_min_shipment_value(&t.buyer, &1_000);
    // total_amount = 500 < 1_000 → must panic
    client.create_shipment(
        &sid(&t.env, "below-floor"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &500,
        &vec![
            &t.env,
            Milestone {
                name: sid(&t.env, "M"),
                payment_percent: 100,
                proof_hash: sid(&t.env, ""),
                status: MilestoneStatus::Pending,
                release_after_ledger: 0,
                proof_submitted_ledger: None,
                dispute_opened_ledger: None,
                deadline_ledger: 0,
                penalty_bps_per_ledger: 0,
                not_before_ledger: 0,
            },
        ],
        &default_options(&t.env),
    );
}

// ============================================================
// Issue #299 — Shipment-level fee override
// ============================================================

#[test]
fn test_shipment_fee_override_cleared_reverts_to_global() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "fee-clear");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.set_shipment_fee_override(&t.buyer, &ship_id, &50u32);
    client.clear_shipment_fee_override(&t.buyer, &ship_id);
    assert_eq!(client.get_shipment_fee_override(&ship_id), None);
}

// ============================================================
// Issue #300 — Long-hold escrow rebate
// ============================================================

#[test]
fn test_long_hold_rebate_getter_after_set() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_long_hold_rebate(&t.buyer, &1000u32, &500u32);
    assert_eq!(client.get_long_hold_rebate(), (1000u32, 500u32));
}

// ============================================================
// Issue #298 — Governance timelock
// ============================================================

#[test]
#[should_panic(expected = "TimelockNotExpired")]
fn test_execute_param_change_before_delay_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_timelock_duration(&t.buyer, &100u32);
    let param = Symbol::new(&t.env, "max_shipment_value");
    client.propose_param_change(&t.buyer, &param, &500_000);
    // Do NOT advance ledger — must panic
    client.execute_param_change(&t.buyer, &param);
}

#[test]
#[should_panic(expected = "NoPendingParamChange")]
fn test_cancel_param_change_removes_pending() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_timelock_duration(&t.buyer, &50u32);
    let param = Symbol::new(&t.env, "max_shipment_value");
    client.propose_param_change(&t.buyer, &param, &1_000_000);
    client.cancel_param_change(&t.buyer, &param);
    // Advance past timelock and try to execute — must panic (no pending change)
    t.env.ledger().with_mut(|l| l.sequence_number += 51);
    client.execute_param_change(&t.buyer, &param);
}

#[test]
fn test_timelock_zero_executes_immediately() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // Default timelock = 0 → effective_ledger = current → can execute immediately
    let param = Symbol::new(&t.env, "max_shipment_value");
    client.propose_param_change(&t.buyer, &param, &777_777);
    client.execute_param_change(&t.buyer, &param);
    assert_eq!(client.get_max_shipment_value(), 777_777);
}

#[test]
fn test_circuit_breaker_limit_param_change() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let param = Symbol::new(&t.env, "circuit_breaker_limit");
    client.propose_param_change(&t.buyer, &param, &5_000_000);
    client.execute_param_change(&t.buyer, &param);
    // No panic = success; circuit breaker limit updated
}
