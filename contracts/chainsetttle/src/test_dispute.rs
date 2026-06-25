#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, Symbol},
    token, Address, String, vec,
};
use crate::test_common::*;

// ============================================================
// DISPUTE TESTS
// ============================================================

#[test]
#[should_panic(expected = "DisputeAlreadyOpen")]
fn test_dispute_limit_blocks_second() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, "SHIP-DISP-LIMIT");
    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "ipfs://t"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    assert_eq!(client.get_shipment(&shipment_id).open_dispute_count, 1);
    client.raise_dispute(&t.buyer, &shipment_id, &1);
}

#[test]
fn test_dispute_limit_frees_slot_on_resolution() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, "SHIP-DISP-RESOLVE");
    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    assert_eq!(client.get_shipment(&shipment_id).open_dispute_count, 1);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);
    assert_eq!(client.get_shipment(&shipment_id).open_dispute_count, 0);
}

#[test]
fn test_dispute_limit_two_allows_two_concurrent_disputes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_max_concurrent_disputes(&t.buyer, &2);

    let shipment_id = String::from_str(&t.env, "SHIP-DISP-2");
    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &1,
        &String::from_str(&t.env, "ipfs://t"),
        &Symbol::new(&t.env, "ipfs"),
    );

    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.raise_dispute(&t.buyer, &shipment_id, &1);
    assert_eq!(client.get_shipment(&shipment_id).open_dispute_count, 2);
}

#[test]
fn test_dispute_cooldown_enforced() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COOLDOWN");
    let cooldown: u32 = 100;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions {
            response_deadline: 0,
            penalty_bps: 0,
            milestone_mode: MilestoneMode::Parallel,
            holdback_ledgers: 0,
            dispute_cooldown_ledgers: cooldown,
            late_penalty_bps_per_ledger: 0,
            auto_confirm_ledgers: 0,
            dispute_bond_amount: 0,
        },
    );

    // First dispute on milestone 0.
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    // Arbiter rejects — milestone goes back to Pending, cooldown starts.
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    // Resubmit proof for milestone 0.
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d2"),
        &Symbol::new(&t.env, "ipfs"),
    );

    // Advance ledger past cooldown.
    t.env.ledger().set_sequence_number(cooldown + 1);

    // Now dispute should succeed.
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Disputed
    );
}

#[test]
#[should_panic(expected = "dispute cooldown period has not elapsed")]
fn test_dispute_cooldown_blocks_early_redispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COOLDOWN-BLOCK");
    let cooldown: u32 = 500;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions {
            response_deadline: 0,
            penalty_bps: 0,
            milestone_mode: MilestoneMode::Parallel,
            holdback_ledgers: 0,
            dispute_cooldown_ledgers: cooldown,
            late_penalty_bps_per_ledger: 0,
            auto_confirm_ledgers: 0,
            dispute_bond_amount: 0,
        },
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    // Resubmit and immediately try to dispute again — must panic.
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d2"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
}

#[test]
fn test_no_cooldown_allows_immediate_redispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-NO-COOLDOWN");

    // cooldown = 0 means no restriction.
    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d2"),
        &Symbol::new(&t.env, "ipfs"),
    );
    // Should succeed immediately with no cooldown.
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Disputed
    );
}

#[test]
fn test_cooldown_updated_on_resolve() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COOLDOWN-UPD");
    let cooldown: u32 = 50;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions {
            response_deadline: 0,
            penalty_bps: 0,
            milestone_mode: MilestoneMode::Parallel,
            holdback_ledgers: 0,
            dispute_cooldown_ledgers: cooldown,
            late_penalty_bps_per_ledger: 0,
            auto_confirm_ledgers: 0,
            dispute_bond_amount: 0,
        },
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);

    t.env.ledger().set_sequence_number(10);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    // last_dispute_resolved_ledger should now be 10.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.last_dispute_resolved_ledger, Some(10u32));
}

#[test]
fn test_fee_deducted_on_dispute_resolve_approve() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let fee_bps: u32 = 200; // 2%
    client.set_fee_config(&t.buyer, &fee_bps, &t.treasury);

    let shipment_id = String::from_str(&t.env, "SHIP-FEE-DISP");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &true);

    let gross = total_amount * 25 / 100;
    let fee = gross * fee_bps as i128 / 10_000;
    let net = gross - fee;

    assert_eq!(token_client.balance(&t.supplier), net);
    assert_eq!(token_client.balance(&t.treasury), fee);
}
