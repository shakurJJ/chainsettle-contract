// #121 — Boundary parameter tests for all u32 and i128 fields.
//
// Naming convention:
//   accept_*  — expects the call to succeed
//   reject_*  — expects the call to panic (the specific message is checked where
//               the contract provides one; otherwise any panic is sufficient)
//
// Coverage:
//   create_shipment  — total_amount (i128), payment_percent (u32), holdback_ledgers (u32)
//   submit_proof     — milestone_index (u32)
//   confirm_milestone — milestone_index (u32)
//   raise_dispute    — milestone_index (u32)
//   resolve_dispute  — milestone_index (u32)
//   release_held_payment — milestone_index (u32)
//   cancel_shipment  — (no numeric params; boundary is shipment existence)
//   get_milestone    — milestone_index (u32)
//   get_escrow_balance — (no numeric params)
//   get_shipment     — (no numeric params)

#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, Env, String, Symbol,
};

// ---- shared env setup -------------------------------------------------------

struct Ctx {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
}

fn setup_ctx() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    // Mint 10B for boundary tests requiring small amounts
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &10_000_000_000);

    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    Ctx { env, contract_id, token_id, buyer, supplier, logistics, arbiter }
}

fn milestones_summing_to(env: &Env, pct: u32) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Only"),
            payment_percent: pct,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
    ]
}

fn standard_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "M0"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "M1"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
    ]
}

fn create_std(ctx: &Ctx, id: &str, amount: i128) {
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, id),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &amount, &standard_milestones(&ctx.env), &false, &0,
    );
}

fn create_holdback(ctx: &Ctx, id: &str, amount: i128, holdback: u32) {
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, id),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &amount, &standard_milestones(&ctx.env), &false, &holdback,
    );
}

// =============================================================================
// create_shipment — total_amount (i128)
// =============================================================================

#[test]
fn accept_create_shipment_amount_one() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    // Minimum valid positive amount
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-AMT-1"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &1_i128,
        &milestones_summing_to(&ctx.env, 100),
        &false, &0,
    );
    let s = client.get_shipment(&String::from_str(&ctx.env, "BNDRY-AMT-1"));
    assert_eq!(s.total_amount, 1);
    assert_eq!(s.released_amount, 0);
}

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn reject_create_shipment_amount_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-AMT-0"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &0_i128,
        &milestones_summing_to(&ctx.env, 100),
        &false, &0,
    );
}

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn reject_create_shipment_amount_i128_min() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-AMT-MIN"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &i128::MIN,
        &milestones_summing_to(&ctx.env, 100),
        &false, &0,
    );
}

#[test]
#[should_panic]
fn reject_create_shipment_amount_i128_max_insufficient_balance() {
    // i128::MAX exceeds any minted balance — token transfer panics
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-AMT-IMAX"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &i128::MAX,
        &milestones_summing_to(&ctx.env, 100),
        &false, &0,
    );
}

// =============================================================================
// create_shipment — payment_percent (u32) in milestone list
// =============================================================================

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn reject_create_shipment_payment_percent_zero_sum() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    // All milestones at 0% → sum = 0, not 100
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-PCT-0"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &1_000_000_i128,
        &milestones_summing_to(&ctx.env, 0),
        &false, &0,
    );
}

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn reject_create_shipment_payment_percent_u32_max_overflows_sum() {
    // u32::MAX as a single milestone's percentage — sum >> 100
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-PCT-MAX"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &1_000_000_i128,
        &milestones_summing_to(&ctx.env, u32::MAX),
        &false, &0,
    );
}

#[test]
fn accept_create_shipment_payment_percent_100_single_milestone() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-PCT-100"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &1_000_000_i128,
        &milestones_summing_to(&ctx.env, 100),
        &false, &0,
    );
    let s = client.get_shipment(&String::from_str(&ctx.env, "BNDRY-PCT-100"));
    assert_eq!(s.milestones.get(0).unwrap().payment_percent, 100);
}

// =============================================================================
// create_shipment — holdback_ledgers (u32)
// =============================================================================

#[test]
fn accept_create_shipment_holdback_u32_max() {
    // u32::MAX holdback is a valid (extremely long) hold window
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-HOLD-MAX"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &1_000_000_i128,
        &standard_milestones(&ctx.env),
        &false,
        &u32::MAX,
    );
    let s = client.get_shipment(&String::from_str(&ctx.env, "BNDRY-HOLD-MAX"));
    assert_eq!(s.holdback_ledgers, u32::MAX);
    assert_eq!(s.status, ShipmentStatus::Active);
}

#[test]
fn accept_create_shipment_holdback_zero_immediate_release() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    let token_client = token::Client::new(&ctx.env, &ctx.token_id);
    create_std(&ctx, "BNDRY-HOLD-0", 1_000_000);

    let id = String::from_str(&ctx.env, "BNDRY-HOLD-0");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.confirm_milestone(&ctx.buyer, &id, &0);
    // Payment transferred immediately — supplier has non-zero balance
    assert!(token_client.balance(&ctx.supplier) > 0);
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::Confirmed);
}

// =============================================================================
// submit_proof — milestone_index (u32)
// =============================================================================

#[test]
fn accept_submit_proof_milestone_index_min_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-SP-0", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-SP-0");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::ProofSubmitted);
}

#[test]
fn accept_submit_proof_milestone_index_last() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-SP-LAST", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-SP-LAST");
    // Standard milestones have 2 entries (index 0 and 1); 1 is the last valid index
    client.submit_proof(&ctx.supplier, &id, &1, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    assert_eq!(client.get_milestone(&id, &1).status, MilestoneStatus::ProofSubmitted);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn reject_submit_proof_milestone_index_out_of_bounds() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-SP-OOB", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-SP-OOB");
    client.submit_proof(&ctx.supplier, &id, &2, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs")); // 2-milestone ship: OOB
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn reject_submit_proof_milestone_index_u32_max() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-SP-UMAX", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-SP-UMAX");
    client.submit_proof(&ctx.supplier, &id, &u32::MAX, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
}

// =============================================================================
// confirm_milestone — milestone_index (u32)
// =============================================================================

#[test]
fn accept_confirm_milestone_index_min_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-CM-0", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-CM-0");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.confirm_milestone(&ctx.buyer, &id, &0);
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::Confirmed);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn reject_confirm_milestone_index_u32_max() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-CM-UMAX", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-CM-UMAX");
    client.confirm_milestone(&ctx.buyer, &id, &u32::MAX);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn reject_confirm_milestone_index_out_of_bounds() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-CM-OOB", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-CM-OOB");
    client.confirm_milestone(&ctx.buyer, &id, &2); // 2-milestone ship
}

// =============================================================================
// raise_dispute — milestone_index (u32)
// =============================================================================

#[test]
fn accept_raise_dispute_milestone_index_min_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-RD-0", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-RD-0");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.raise_dispute(&ctx.buyer, &id, &0);
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::Disputed);
}

#[test]
#[should_panic]
fn reject_raise_dispute_milestone_index_u32_max() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-RD-UMAX", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-RD-UMAX");
    client.raise_dispute(&ctx.buyer, &id, &u32::MAX);
}

// =============================================================================
// resolve_dispute — milestone_index (u32)
// =============================================================================

#[test]
fn accept_resolve_dispute_milestone_index_min_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-RESV-0", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-RESV-0");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.raise_dispute(&ctx.buyer, &id, &0);
    client.resolve_dispute(&ctx.arbiter, &id, &0, &true);
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::Resolved);
}

#[test]
#[should_panic]
fn reject_resolve_dispute_milestone_index_u32_max() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-RESV-UMAX", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-RESV-UMAX");
    client.resolve_dispute(&ctx.arbiter, &id, &u32::MAX, &true);
}

// =============================================================================
// release_held_payment — milestone_index (u32)
// =============================================================================

#[test]
fn accept_release_held_payment_milestone_index_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    let token_client = token::Client::new(&ctx.env, &ctx.token_id);
    create_holdback(&ctx, "BNDRY-RHP-0", 1_000_000, 10);

    let id = String::from_str(&ctx.env, "BNDRY-RHP-0");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.confirm_milestone(&ctx.buyer, &id, &0);
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::ConfirmedHeld);

    // Advance ledger past holdback
    ctx.env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 0,
        protocol_version: 22,
        sequence_number: ctx.env.ledger().sequence() + 11,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_300_000,
    });

    client.release_held_payment(&id, &0);
    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::Confirmed);
    assert!(token_client.balance(&ctx.supplier) > 0);
}

#[test]
#[should_panic]
fn reject_release_held_payment_milestone_index_u32_max() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_holdback(&ctx, "BNDRY-RHP-UMAX", 1_000_000, 10);
    let id = String::from_str(&ctx.env, "BNDRY-RHP-UMAX");
    client.release_held_payment(&id, &u32::MAX);
}

#[test]
#[should_panic(expected = "holdback period not yet expired")]
fn reject_release_held_payment_before_expiry_at_ledger_0() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_holdback(&ctx, "BNDRY-RHP-EARLY", 1_000_000, u32::MAX);

    let id = String::from_str(&ctx.env, "BNDRY-RHP-EARLY");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.confirm_milestone(&ctx.buyer, &id, &0);
    // Try to release immediately — holdback is u32::MAX ledgers in the future
    client.release_held_payment(&id, &0);
}

// =============================================================================
// get_milestone — milestone_index (u32)
// =============================================================================

#[test]
fn accept_get_milestone_index_min_zero() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-GM-0", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-GM-0");
    let m = client.get_milestone(&id, &0);
    assert_eq!(m.status, MilestoneStatus::Pending);
    assert_eq!(m.payment_percent, 50);
}

#[test]
fn accept_get_milestone_index_last() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-GM-LAST", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-GM-LAST");
    let m = client.get_milestone(&id, &1);
    assert_eq!(m.status, MilestoneStatus::Pending);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn reject_get_milestone_index_u32_max() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-GM-UMAX", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-GM-UMAX");
    client.get_milestone(&id, &u32::MAX);
}

#[test]
#[should_panic(expected = "invalid milestone index")]
fn reject_get_milestone_index_out_of_bounds() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    create_std(&ctx, "BNDRY-GM-OOB", 1_000_000);
    let id = String::from_str(&ctx.env, "BNDRY-GM-OOB");
    client.get_milestone(&id, &2);
}

// =============================================================================
// get_shipment / get_escrow_balance — non-existent shipment
// =============================================================================

#[test]
#[should_panic(expected = "shipment not found")]
fn reject_get_shipment_unknown_id() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.get_shipment(&String::from_str(&ctx.env, "DOES-NOT-EXIST"));
}

#[test]
#[should_panic(expected = "shipment not found")]
fn reject_get_escrow_balance_unknown_id() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    client.get_escrow_balance(&String::from_str(&ctx.env, "DOES-NOT-EXIST"));
}

// =============================================================================
// Numeric edge: released_amount can never exceed total_amount (i128 arithmetic)
// =============================================================================

#[test]
fn accept_escrow_balance_never_negative_after_full_release() {
    let ctx = setup_ctx();
    let client = ChainSettleContractClient::new(&ctx.env, &ctx.contract_id);
    // Single 100% milestone; confirming it releases everything
    let amount: i128 = 1;
    client.create_shipment(
        &String::from_str(&ctx.env, "BNDRY-ESCROW-MIN"),
        &ctx.buyer, &ctx.supplier, &ctx.logistics, &ctx.arbiter, &ctx.token_id,
        &amount,
        &milestones_summing_to(&ctx.env, 100),
        &false, &0,
    );
    let id = String::from_str(&ctx.env, "BNDRY-ESCROW-MIN");
    client.submit_proof(&ctx.supplier, &id, &0, &String::from_str(&ctx.env, "h"), &Symbol::new(&ctx.env, "ipfs"));
    client.confirm_milestone(&ctx.buyer, &id, &0);

    let balance = client.get_escrow_balance(&id);
    // 1 * 100 / 100 = 1 released; escrow = 1 - 1 = 0
    assert_eq!(balance, 0, "escrow balance should be zero, not negative");
}
