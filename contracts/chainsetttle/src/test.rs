#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String, Symbol,
};
use std::format;

// ============================================================
// TEST HELPERS
// ============================================================

struct TestSetup {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    buyer2: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
    treasury: Address,
}

fn setup() -> TestSetup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = token::StellarAssetClient::new(&env, &token_id);

    let buyer = Address::generate(&env);
    let buyer2 = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    let treasury = Address::generate(&env);

    token_client.mint(&buyer, &10_000_000_000);
    token_client.mint(&buyer2, &10_000_000_000);

    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    TestSetup {
        env,
        contract_id,
        token_id,
        buyer,
        buyer2,
        supplier,
        logistics,
        arbiter,
        treasury,
    }
}

fn build_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Goods Dispatched"),
            payment_percent: 25,
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
            name: String::from_str(env, "In Transit"),
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
            name: String::from_str(env, "Delivered"),
            payment_percent: 25,
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

fn single_buyer_vec(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

fn default_options(_env: &Env) -> ShipmentOptions {
    ShipmentOptions {
        response_deadline: 0,
        penalty_bps: 0,
        milestone_mode: MilestoneMode::Parallel,
        holdback_ledgers: 0,
        dispute_cooldown_ledgers: 0,
        late_penalty_bps_per_ledger: 0,
        auto_confirm_ledgers: 0,
        dispute_bond_amount: 0,
        dispute_bond_bps: 0,
        arbiter_fee_bps: 0,
        logistics_fee_bps: 0,
        supplier_collateral: 0,
        expires_at_ledger: None,
        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,
        early_bonus_pool: 0,
        review_window_ledgers: None,
        milestone_splits: vec![_env],
        deadlines: vec![_env],
        dispute_timeout_seconds: 0,
        default_resolution: Resolution::Buyer,
        backup_arbiter: None,
        confirmation_cooldown_ledgers: None,
        arbiter_panel: vec![_env],
        jurisdiction: None,
        grace_period_ledgers: 0,
        quality_grades: vec![_env],
        milestone_quantities: vec![_env],
        retainage_bps: 0,
        warranty_bps: 0,
        warranty_ledgers: 0,
        inspector: None,
        inspected_milestones: vec![_env],
        proof_submitters: vec![_env],
        require_dual_attestation: false,
        fund_from_vault: false,
        milestone_suppliers: soroban_sdk::vec![_env],
        refund_recipient: None,
        require_encrypted_evidence: false,
    }
}

/// Create a standard shipment with no deadline, no penalty, parallel mode, no holdback, no cooldown.
fn create_standard_shipment(
    client: &ChainSettleContractClient,
    env: &Env,
    shipment_id: &String,
    buyer: &Address,
    supplier: &Address,
    logistics: &Address,
    arbiter: &Address,
    token_id: &Address,
    total_amount: i128,
) {
    client.create_shipment(
        shipment_id,
        &single_buyer_vec(env, buyer),
        supplier,
        logistics,
        arbiter,
        token_id,
        &total_amount,
        &build_milestones(env),
        &default_options(env),
    );
}

// ============================================================
// CORE LIFECYCLE TESTS
// ============================================================

// ============================================================
// PAUSE / UNPAUSE TESTS
// ============================================================

// ============================================================
// ESCROW TOP-UP TESTS
// ============================================================

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

// ============================================================
// DISPUTE COOLDOWN TESTS
// ============================================================

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
            dispute_bond_bps: 0,
            arbiter_fee_bps: 0,
            logistics_fee_bps: 0,
            supplier_collateral: 0,
            expires_at_ledger: None,

            metadata_hash: None,
            referrer: None,
            buyer_cancel_fee_bps: 0,
            early_bonus_pool: 0,
            review_window_ledgers: None,
            milestone_splits: vec![&t.env],
            deadlines: vec![&t.env],
            dispute_timeout_seconds: 0,
            default_resolution: Resolution::Buyer,
            backup_arbiter: None,
            confirmation_cooldown_ledgers: None,
            arbiter_panel: vec![&t.env],
            jurisdiction: None,
            grace_period_ledgers: 0,
            quality_grades: vec![&t.env],
            milestone_quantities: vec![&t.env],
            retainage_bps: 0,
            warranty_bps: 0,
            warranty_ledgers: 0,
            inspector: None,
            inspected_milestones: vec![&t.env],
            proof_submitters: vec![&t.env],
            require_dual_attestation: false,
            fund_from_vault: false,
            milestone_suppliers: soroban_sdk::vec![&t.env],
            refund_recipient: None,
            require_encrypted_evidence: false,
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
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false, &None);

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

// ============================================================
// TRANSFER BUYER / SUPPLIER TESTS
// ============================================================

#[test]
#[should_panic(expected = "transfer disallowed: open dispute must be resolved first")]
fn test_transfer_buyer_blocked_by_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-XFER-DISP");

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

    // Transfer while dispute is open — must panic.
    client.transfer_buyer(&t.buyer, &shipment_id, &t.buyer2);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_transfer_supplier_wrong_caller_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let new_supplier = Address::generate(&t.env);
    let shipment_id = String::from_str(&t.env, "SHIP-XFER-SUP-AUTH");

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

    // Buyer tries to transfer supplier role — must be rejected.
    client.transfer_supplier(&t.buyer, &shipment_id, &new_supplier);
}

// ============================================================
// NON-SEQUENTIAL / PARALLEL MODE TESTS
// ============================================================

// ============================================================
// TOKEN WHITELIST TESTS
// ============================================================

// ============================================================
// FEE TESTS
// ============================================================

#[test]
#[should_panic(expected = "fee_bps exceeds maximum of 1000")]
fn test_fee_bps_exceeds_max_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &1001, &t.treasury);
}

// ============================================================
// HOLDBACK TESTS
// ============================================================

// ============================================================
// BATCH CONFIRM TESTS
// ============================================================

// ============================================================
// MULTISIG TESTS
// ============================================================

// ============================================================
// AMENDMENT TESTS
// ============================================================

#[test]
#[should_panic(expected = "can only amend a pending milestone")]
fn test_amendment_confirmed_milestone_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-AMEND-CONF");

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
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    client.propose_amendment(
        &t.buyer,
        &shipment_id,
        &0,
        &25,
        &String::from_str(&t.env, "New Name"),
    );
}

// ============================================================
// ARITHMETIC / OVERFLOW EDGE-CASE TESTS
// ============================================================

// ============================================================
// SUPPLIER CANCEL TESTS
// ============================================================

#[test]
#[should_panic(expected = "buyer response deadline has not passed")]
fn test_deadline_cancellation_too_early() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-PREMATURE");

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
            response_deadline: 1000,
            penalty_bps: 500,
            milestone_mode: MilestoneMode::Parallel,
            holdback_ledgers: 0,
            dispute_cooldown_ledgers: 0,
            late_penalty_bps_per_ledger: 0,
            auto_confirm_ledgers: 0,
            dispute_bond_amount: 0,
            dispute_bond_bps: 0,
            arbiter_fee_bps: 0,
            logistics_fee_bps: 0,
            supplier_collateral: 0,
            expires_at_ledger: None,

            metadata_hash: None,
            referrer: None,
            buyer_cancel_fee_bps: 0,
            early_bonus_pool: 0,
            review_window_ledgers: None,
            milestone_splits: vec![&t.env],
            deadlines: vec![&t.env],
            dispute_timeout_seconds: 0,
            default_resolution: Resolution::Buyer,
            backup_arbiter: None,
            confirmation_cooldown_ledgers: None,
            arbiter_panel: vec![&t.env],
            jurisdiction: None,
            grace_period_ledgers: 0,
            quality_grades: vec![&t.env],
            milestone_quantities: vec![&t.env],
            retainage_bps: 0,
            warranty_bps: 0,
            warranty_ledgers: 0,
            inspector: None,
            inspected_milestones: vec![&t.env],
            proof_submitters: vec![&t.env],
            require_dual_attestation: false,
            fund_from_vault: false,
            milestone_suppliers: soroban_sdk::vec![&t.env],
            refund_recipient: None,
            require_encrypted_evidence: false,
        },
    );

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.supplier_cancel(&t.supplier, &shipment_id);
}

// ============================================================
// UPGRADE TESTS
// ============================================================

#[test]
#[should_panic(expected = "unauthorized")]
fn test_upgrade_non_admin_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let fake_hash = BytesN::from_array(&t.env, &[0u8; 32]);
    client.upgrade(&t.supplier, &fake_hash);
}

// ============================================================
// DISPUTE RESOLVE — FEE ON APPROVE
// ============================================================

// ============================================================
// GET_COMPLETION_PERCENTAGE TESTS
// ============================================================

// ============================================================
// EVENT PAYLOAD CORRECTNESS TESTS  (Issue #50)
// ============================================================

// ============================================================
// #284 — SUPPLIER PAYOUT BATCHING TESTS
// ============================================================

#[test]
#[should_panic(expected = "no pending payout")]
fn test_claim_payout_zero_balance_panics() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // Supplier never accumulated anything.
    client.claim_payout(&t.supplier, &t.token_id);
}

// ============================================================
// #285 — PER-ADDRESS OUTFLOW RATE LIMITING TESTS
// ============================================================

#[test]
#[should_panic(expected = "address outflow limit exceeded")]
fn test_address_outflow_limit_independent_of_global_breaker() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let total: i128 = 1_000_000_000;
    let milestone_payment = total * 25 / 100;

    // Global circuit breaker allows plenty of headroom (10x).
    client.set_circuit_breaker(&t.buyer, &(milestone_payment * 10), &1000);
    // Per-address limit is tighter — less than one payment.
    client.set_address_outflow_limit(&t.buyer, &t.supplier, &(milestone_payment - 1), &1000);

    let id = String::from_str(&t.env, "OUTFLOW-INDEP");
    create_standard_shipment(
        &client,
        &t.env,
        &id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total,
    );
    client.submit_proof(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    // Per-address cap trips even though global breaker has headroom.
    client.confirm_milestone(&t.buyer, &id, &0);
}

#[test]
fn test_get_address_outflow_limit_returns_configured_values() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let limit: i128 = 500_000_000;
    let window: u32 = 200;
    client.set_address_outflow_limit(&t.buyer, &t.supplier, &limit, &window);

    let (ret_limit, ret_window, _ret_start, ret_outflow) =
        client.get_address_outflow_limit(&t.supplier);
    assert_eq!(ret_limit, limit);
    assert_eq!(ret_window, window);
    assert_eq!(ret_outflow, 0);
}

// ============================================================
// #286 — DISPUTE EVIDENCE VERSIONING TESTS
// ============================================================

#[test]
fn test_submit_dispute_evidence_multiple_parties_append_only() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let id = String::from_str(&t.env, "EVID-MULTI");
    create_standard_shipment(
        &client,
        &t.env,
        &id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );
    client.submit_proof(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &id, &0);

    client.submit_dispute_evidence(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://sup-ev"),
        &Symbol::new(&t.env, "invoice"),
    );
    client.submit_dispute_evidence(
        &t.logistics,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://log-ev"),
        &Symbol::new(&t.env, "photo"),
    );
    client.submit_dispute_evidence(
        &t.buyer,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://buy-ev"),
        &Symbol::new(&t.env, "affidavit"),
    );

    let entries = client.get_dispute_evidence(&id, &0);
    assert_eq!(entries.len(), 3);
    // Verify submission order.
    assert_eq!(entries.get(0).unwrap().submitter, t.supplier);
    assert_eq!(entries.get(1).unwrap().submitter, t.logistics);
    assert_eq!(entries.get(2).unwrap().submitter, t.buyer);
}

#[test]
#[should_panic(expected = "evidence can only be submitted while milestone is Disputed")]
fn test_submit_evidence_rejected_on_pending_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let id = String::from_str(&t.env, "EVID-PENDING");
    create_standard_shipment(
        &client,
        &t.env,
        &id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );
    // Milestone 0 is Pending — must reject.
    client.submit_dispute_evidence(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://ev"),
        &Symbol::new(&t.env, "invoice"),
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_submit_evidence_rejected_for_arbiter() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let id = String::from_str(&t.env, "EVID-ARBITER");
    create_standard_shipment(
        &client,
        &t.env,
        &id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );
    client.submit_proof(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &id, &0);
    // Arbiter is not allowed — only supplier, logistics, or buyer.
    client.submit_dispute_evidence(
        &t.arbiter,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://ev"),
        &Symbol::new(&t.env, "note"),
    );
}

// ============================================================
// #287 — BUYER-INITIATED DISPUTE WITHDRAWAL TESTS
// ============================================================

#[test]
fn test_withdraw_dispute_removes_from_active_disputes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let id = String::from_str(&t.env, "WD-ACTIVE");
    create_standard_shipment(
        &client,
        &t.env,
        &id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );
    client.submit_proof(
        &t.supplier,
        &id,
        &0,
        &String::from_str(&t.env, "ipfs://proof"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.raise_dispute(&t.buyer, &id, &0);
    assert_eq!(client.get_active_disputes().len(), 1);

    client.withdraw_dispute(&t.buyer, &id, &0);
    assert_eq!(client.get_active_disputes().len(), 0);
}

