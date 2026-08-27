#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{vec, Address, Env, String, Symbol};

fn sid(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

fn advance_ledger(env: &Env, by: u32) {
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: env.ledger().timestamp(),
        protocol_version: 22,
        sequence_number: env.ledger().sequence() + by,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 4096,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_300_000,
    });
}

fn advance_timestamp(env: &Env, by: u64) {
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: env.ledger().timestamp() + by,
        protocol_version: 22,
        sequence_number: env.ledger().sequence(),
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_300_000,
    });
}

fn single_milestone_with_timestamp_deadline(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: sid(env, "M"),
            payment_percent: 100,
            proof_hash: sid(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
        },
    ]
}

// ============================================================
// #391 — Dispute bond scaling by shipment value (bps)
// ============================================================

#[test]
fn dispute_bond_bps_scales_with_shipment_value() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = sid(&t.env, "BOND-BPS-1");
    let mut opts = default_options(&t.env);
    opts.dispute_bond_bps = 500; // 5%

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &opts,
    );

    let shipment = client.get_shipment(&shipment_id);
    // 5% of 1_000_000 = 50_000, stacked onto the flat (0) bond.
    assert_eq!(shipment.dispute_bond_amount, 50_000);
}

#[test]
fn dispute_bond_bps_stacks_with_flat_amount() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = sid(&t.env, "BOND-BPS-2");
    let mut opts = default_options(&t.env);
    opts.dispute_bond_amount = 10_000;
    opts.dispute_bond_bps = 200; // 2% = 20_000

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &opts,
    );

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.dispute_bond_amount, 30_000);
}

#[test]
#[should_panic(expected = "dispute_bond_bps exceeds maximum allowed")]
fn dispute_bond_bps_rejected_above_admin_cap() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_max_dispute_bond_bps(&t.buyer, &1_000);

    let shipment_id = sid(&t.env, "BOND-BPS-3");
    let mut opts = default_options(&t.env);
    opts.dispute_bond_bps = 1_500;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &opts,
    );
}

#[test]
fn max_dispute_bond_bps_default_matches_constant() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(
        client.get_max_dispute_bond_bps(),
        constants::DEFAULT_MAX_DISPUTE_BOND_BPS
    );
}

// ============================================================
// #392 — Supplier cancellation cooldown
// ============================================================

fn create_cancellable(t: &TestSetup, client: &ChainSettleContractClient, shipment_id: &String) {
    let mut opts = default_options(&t.env);
    opts.response_deadline = 5;
    opts.penalty_bps = 0;
    client.create_shipment(
        shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &opts,
    );
}

fn make_cancellable(t: &TestSetup, client: &ChainSettleContractClient, shipment_id: &String) {
    create_cancellable(t, client, shipment_id);
    client.submit_proof(
        &t.supplier,
        shipment_id,
        &0,
        &sid(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    advance_ledger(&t.env, 10);
}

#[test]
fn supplier_cancel_allowed_within_cap() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_supplier_cancel_cooldown(&t.buyer, &t.supplier, &2, &1000, &500);

    let id1 = sid(&t.env, "CANCOOL-1");
    make_cancellable(&t, &client, &id1);
    client.supplier_cancel(&t.supplier, &id1);

    let shipment = client.get_shipment(&id1);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
}

#[test]
#[should_panic(expected = "supplier cancellation cooldown active")]
fn supplier_cancel_blocked_after_cap_exceeded() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_supplier_cancel_cooldown(&t.buyer, &t.supplier, &1, &1000, &500);

    let id1 = sid(&t.env, "CANCOOL-2A");
    make_cancellable(&t, &client, &id1);
    client.supplier_cancel(&t.supplier, &id1);

    let id2 = sid(&t.env, "CANCOOL-2B");
    make_cancellable(&t, &client, &id2);
    client.supplier_cancel(&t.supplier, &id2);
}

#[test]
fn supplier_cancel_resets_after_window_elapses() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_supplier_cancel_cooldown(&t.buyer, &t.supplier, &1, &50, &10);

    let id1 = sid(&t.env, "CANCOOL-3A");
    make_cancellable(&t, &client, &id1);
    client.supplier_cancel(&t.supplier, &id1);

    // Advance past both the rolling window and any cooldown penalty.
    advance_ledger(&t.env, 100);

    let id2 = sid(&t.env, "CANCOOL-3B");
    make_cancellable(&t, &client, &id2);
    client.supplier_cancel(&t.supplier, &id2);

    let shipment = client.get_shipment(&id2);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
}

#[test]
fn supplier_cancel_unlimited_when_not_configured() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let id1 = sid(&t.env, "CANCOOL-4A");
    make_cancellable(&t, &client, &id1);
    client.supplier_cancel(&t.supplier, &id1);

    let id2 = sid(&t.env, "CANCOOL-4B");
    make_cancellable(&t, &client, &id2);
    client.supplier_cancel(&t.supplier, &id2);

    assert_eq!(
        client.get_shipment(&id2).status,
        ShipmentStatus::Cancelled
    );
}

// ============================================================
// #390 — N-of-M oracle attestation requirement
// ============================================================

#[test]
fn confirm_milestone_blocked_until_oracle_threshold_met() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let purpose = Symbol::new(&t.env, "delivery");
    let oracle1 = Address::generate(&t.env);
    let oracle2 = Address::generate(&t.env);
    let oracle3 = Address::generate(&t.env);
    let oracles = vec![&t.env, oracle1.clone(), oracle2.clone(), oracle3.clone()];

    client.register_oracle_group(&t.buyer, &purpose, &oracles, &2);

    let shipment_id = sid(&t.env, "ORACLE-1");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.set_shipment_oracle_purpose(&t.buyer, &shipment_id, &purpose);

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &sid(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );

    // Only one of two required attestations submitted so far.
    client.submit_oracle_attestation(&oracle1, &shipment_id, &0);
    assert_eq!(client.get_oracle_attestation_count(&shipment_id, &0), 1);

    let result = client.try_confirm_milestone(&t.buyer, &shipment_id, &0);
    assert!(result.is_err());

    // Second attestation reaches the 2-of-3 threshold.
    client.submit_oracle_attestation(&oracle2, &shipment_id, &0);
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    assert_eq!(
        client.get_shipment(&shipment_id).milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

#[test]
fn confirm_milestone_unaffected_when_no_oracle_group_assigned() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = sid(&t.env, "ORACLE-2");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &sid(&t.env, "proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    assert_eq!(
        client.get_shipment(&shipment_id).milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

#[test]
#[should_panic(expected = "caller is not a member of the assigned oracle group")]
fn oracle_attestation_rejected_from_non_member() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let purpose = Symbol::new(&t.env, "delivery");
    let oracle1 = Address::generate(&t.env);
    let outsider = Address::generate(&t.env);
    client.register_oracle_group(&t.buyer, &purpose, &vec![&t.env, oracle1], &1);

    let shipment_id = sid(&t.env, "ORACLE-3");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
    client.set_shipment_oracle_purpose(&t.buyer, &shipment_id, &purpose);

    client.submit_oracle_attestation(&outsider, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "threshold must be between 1 and oracles.len()")]
fn register_oracle_group_rejects_threshold_above_member_count() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let purpose = Symbol::new(&t.env, "delivery");
    let oracle1 = Address::generate(&t.env);
    client.register_oracle_group(&t.buyer, &purpose, &vec![&t.env, oracle1], &2);
}

// ============================================================
// #389 — Escrow sweep of unclaimed refunds to treasury
// ============================================================

fn create_with_timestamp_deadline(
    t: &TestSetup,
    client: &ChainSettleContractClient,
    shipment_id: &String,
    deadline_offset: u64,
) {
    let mut opts = default_options(&t.env);
    opts.deadlines = vec![&t.env, t.env.ledger().timestamp() + deadline_offset];
    client.create_shipment(
        shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &single_milestone_with_timestamp_deadline(&t.env),
        &opts,
    );
}

#[test]
#[should_panic(expected = "unclaimed refund sweeping is not enabled")]
fn sweep_unclaimed_refund_disabled_by_default() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = sid(&t.env, "SWEEP-1");
    create_with_timestamp_deadline(&t, &client, &shipment_id, 100);
    advance_timestamp(&t.env, 200);

    client.sweep_unclaimed_refund(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "sweep window has not elapsed")]
fn sweep_unclaimed_refund_blocked_before_window_elapses() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &0, &t.treasury);
    client.set_refund_sweep_window(&t.buyer, &1_000);

    let shipment_id = sid(&t.env, "SWEEP-2");
    create_with_timestamp_deadline(&t, &client, &shipment_id, 100);
    advance_timestamp(&t.env, 200);

    client.mark_refund_claimable(&shipment_id, &0);
    client.sweep_unclaimed_refund(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "call mark_refund_claimable first")]
fn sweep_unclaimed_refund_requires_claimable_checkpoint() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_fee_config(&t.buyer, &0, &t.treasury);
    client.set_refund_sweep_window(&t.buyer, &100);

    let shipment_id = sid(&t.env, "SWEEP-3");
    create_with_timestamp_deadline(&t, &client, &shipment_id, 100);
    advance_timestamp(&t.env, 200);

    // Sweep attempted without ever calling mark_refund_claimable.
    client.sweep_unclaimed_refund(&t.buyer, &shipment_id, &0);
}

#[test]
fn sweep_unclaimed_refund_pays_treasury_after_window() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = soroban_sdk::token::Client::new(&t.env, &t.token_id);

    client.set_fee_config(&t.buyer, &0, &t.treasury);
    client.set_refund_sweep_window(&t.buyer, &100);

    let shipment_id = sid(&t.env, "SWEEP-4");
    create_with_timestamp_deadline(&t, &client, &shipment_id, 100);
    advance_timestamp(&t.env, 200);

    client.mark_refund_claimable(&shipment_id, &0);

    // Window hasn't elapsed yet.
    let too_early = client.try_sweep_unclaimed_refund(&t.buyer, &shipment_id, &0);
    assert!(too_early.is_err());

    advance_ledger(&t.env, 200);
    client.sweep_unclaimed_refund(&t.buyer, &shipment_id, &0);

    assert_eq!(token_client.balance(&t.treasury), 1_000_000);
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Expired
    );
}

#[test]
fn buyer_can_still_claim_refund_before_sweep() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = soroban_sdk::token::Client::new(&t.env, &t.token_id);

    client.set_fee_config(&t.buyer, &0, &t.treasury);
    client.set_refund_sweep_window(&t.buyer, &100);

    let shipment_id = sid(&t.env, "SWEEP-4");
    create_with_timestamp_deadline(&t, &client, &shipment_id, 100);
    advance_timestamp(&t.env, 200);

    let balance_before = token_client.balance(&t.buyer);
    client.claim_deadline_refund(&t.buyer, &shipment_id, &0);
    assert_eq!(token_client.balance(&t.buyer), balance_before + 1_000_000);
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Expired
    );
}

#[test]
fn refund_sweep_window_default_disabled() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(client.get_refund_sweep_window(), 0);
}
