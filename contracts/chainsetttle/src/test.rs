#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String,
};

// ============================================================
// TEST HELPERS
// ============================================================

fn setup() -> (Env, Address, Address, Address, Address, Address, Address) {
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

fn build_milestones(env: &Env) -> Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Goods Dispatched"),
            payment_percent: 25,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "In Transit"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "Delivered"),
            payment_percent: 25,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
    ]
}

/// Helper: create a standard shipment with no deadline / no penalty.
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
        buyer,
        supplier,
        logistics,
        arbiter,
        token_id,
        &total_amount,
        &build_milestones(env),
        &0,
        &0,
    );
}

// ============================================================
// EXISTING TESTS (updated for new create_shipment signature)
// ============================================================

#[test]
fn test_create_shipment_success() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let total_amount: i128 = 1_000_000_000;
    create(&client, &env, "SHIP-001", &buyer, &supplier, &logistics, &arbiter, &token_id, total_amount, false, 0);

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    assert_eq!(token_client.balance(&buyer), 10_000_000_000 - total_amount);
    assert_eq!(token_client.balance(&contract_id), total_amount);

    let shipment = client.get_shipment(&String::from_str(&env, "SHIP-001"));
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.total_amount, total_amount);
    assert_eq!(shipment.released_amount, 0);
    assert_eq!(shipment.milestones.len(), 3);
    assert!(!shipment.sequential);
    assert_eq!(shipment.holdback_ledgers, 0);
}

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_create_shipment_invalid_percentages() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let bad_milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Step 1"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "Step 2"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(&t.env, "Step 3"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
    ];

    client.create_shipment(
        &String::from_str(&t.env, "SHIP-BAD"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &bad_milestones,
        &0,
        &0,
    );
}

#[test]
fn test_full_shipment_lifecycle() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let total_amount: i128 = 1_000_000_000;
    create(&client, &env, "SHIP-FULL", &buyer, &supplier, &logistics, &arbiter, &token_id, total_amount, false, 0);

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    client.submit_proof(&logistics, &id, &1, &String::from_str(&env, "ipfs://transit"));
    client.confirm_milestone(&buyer, &id, &1);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::ProofSubmitted
    );

    let shipment = client.get_shipment(&id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    assert_eq!(token_client.balance(&supplier), total_amount);
    assert_eq!(client.get_escrow_balance(&id), 0);
}

    let expected_payment = total_amount * 25 / 100;
    assert_eq!(token_client.balance(&supplier), expected_payment);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.released_amount, expected_payment);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_raise_and_resolve_dispute_reject() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let total_amount: i128 = 1_000_000_000;
    create(&client, &env, "SHIP-REJECT", &buyer, &supplier, &logistics, &arbiter, &token_id, total_amount, false, 0);

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    assert_eq!(client.get_milestone(&id, &0).status, MilestoneStatus::Pending);
    assert_eq!(token_client.balance(&supplier), 0);
}

#[test]
fn test_cancel_shipment() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let total_amount: i128 = 1_000_000_000;
    let balance_before = token_client.balance(&buyer);
    create(&client, &env, "SHIP-CANCEL", &buyer, &supplier, &logistics, &arbiter, &token_id, total_amount, false, 0);

    let id = String::from_str(&env, "SHIP-CANCEL");
    client.cancel_shipment(&buyer, &id);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    assert_eq!(token_client.balance(&supplier), total_amount);
    assert_eq!(client.get_escrow_balance(&shipment_id), 0);
}

// ============================================================
// ISSUE #1: SEQUENTIAL MILESTONE TESTS
// ============================================================

#[test]
fn test_sequential_happy_path() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    create(&client, &env, "SEQ-OK", &buyer, &supplier, &logistics, &arbiter, &token_id, 1_000_000_000, true, 0);
    let id = String::from_str(&env, "SEQ-OK");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    client.submit_proof(&logistics, &id, &1, &String::from_str(&env, "ipfs://1"));
    client.confirm_milestone(&buyer, &id, &1);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Disputed
    );

    create(&client, &env, "SEQ-BAD", &buyer, &supplier, &logistics, &arbiter, &token_id, 1_000_000_000, true, 0);
    let id = String::from_str(&env, "SEQ-BAD");

    let expected = total_amount * 25 / 100;
    assert_eq!(token_client.balance(&supplier), expected);
}

#[test]
fn test_non_sequential_baseline() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    create(&client, &env, "NONSEQ", &buyer, &supplier, &logistics, &arbiter, &token_id, 1_000_000_000, false, 0);
    let id = String::from_str(&env, "NONSEQ");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

// ============================================================
// ISSUE #2: MULTI-TOKEN / WHITELIST TESTS
// ============================================================

#[test]
fn test_usdc_shipment_no_whitelist() {
    // Empty whitelist → any token accepted
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    create(&client, &env, "USDC-SHIP", &buyer, &supplier, &logistics, &arbiter, &token_id, 1_000_000_000, false, 0);
    assert_eq!(client.get_shipment(&String::from_str(&env, "USDC-SHIP")).status, ShipmentStatus::Active);
}

#[test]
fn test_xlm_shipment_whitelisted() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    // Register a second "XLM" token
    let xlm_admin = Address::generate(&env);
    let xlm_id = env.register_stellar_asset_contract_v2(xlm_admin.clone()).address();
    token::StellarAssetClient::new(&env, &xlm_id).mint(&buyer, &10_000_000_000);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Pending
    );
    assert_eq!(token_client.balance(&supplier), 0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_whitelisted_token_rejected() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    // Whitelist only token_id; try to use a different token
    client.add_allowed_token(&token_id);

    let other_admin = Address::generate(&env);
    let other_token = env.register_stellar_asset_contract_v2(other_admin.clone()).address();
    token::StellarAssetClient::new(&env, &other_token).mint(&buyer, &10_000_000_000);

    create(&client, &env, "BAD-TOKEN", &buyer, &supplier, &logistics, &arbiter, &other_token, 1_000_000_000, false, 0);
}

#[test]
fn test_whitelist_toggle() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    // Add then remove token_id — list becomes empty → permissionless again
    client.add_allowed_token(&token_id);
    client.remove_allowed_token(&token_id);

    // Should succeed because list is now empty
    create(&client, &env, "TOGGLE-SHIP", &buyer, &supplier, &logistics, &arbiter, &token_id, 1_000_000_000, false, 0);
    assert_eq!(client.get_shipment(&String::from_str(&env, "TOGGLE-SHIP")).status, ShipmentStatus::Active);
}

// ============================================================
// ISSUE #3: PARTIAL CANCELLATION TESTS
// ============================================================

#[test]
fn test_cancel_zero_confirmed() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let total: i128 = 1_000_000_000;
    let before = token_client.balance(&buyer);
    create(&client, &env, "CANCEL-ZERO", &buyer, &supplier, &logistics, &arbiter, &token_id, total, false, 0);

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    client.cancel_shipment(&buyer, &shipment_id);

    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Cancelled
    );
    assert_eq!(token_client.balance(&buyer), buyer_balance_before);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_unauthorized_confirm_milestone() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-AUTH");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        1_000_000_000,
    );

    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://proof"));
    // Supplier tries to confirm — should panic
    client.confirm_milestone(&supplier, &shipment_id, &0);
}

// ============================================================
// #4 — UPGRADE TESTS
// ============================================================

#[test]
#[should_panic(expected = "unauthorized")]
fn test_upgrade_non_admin_rejected() {
    let (env, contract_id, _token_id, _buyer, supplier, _logistics, _arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    // supplier is not admin — must panic
    let fake_hash = BytesN::from_array(&env, &[0u8; 32]);
    client.upgrade(&supplier, &fake_hash);
}

// Note: a successful upgrade test requires a second compiled WASM binary which is
// not available in unit-test context. The auth + event path is covered by the
// non-admin rejection test above and the contract logic is straightforward.

// ============================================================
// #8 — BATCH CONFIRM MILESTONES TESTS
// ============================================================

#[test]
fn test_batch_confirm_milestones_full() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let shipment_id = String::from_str(&env, "SHIP-BATCH");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    // Submit proof for all three milestones.
    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));
    client.submit_proof(&logistics, &shipment_id, &1, &String::from_str(&env, "ipfs://t"));
    client.submit_proof(&supplier, &shipment_id, &2, &String::from_str(&env, "ipfs://v"));

    // Batch confirm all three in one call.
    client.batch_confirm_milestones(&buyer, &shipment_id, &vec![&env, 0u32, 1u32, 2u32]);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    assert_eq!(token_client.balance(&supplier), total_amount);
}

#[test]
fn test_batch_confirm_single_element() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let shipment_id = String::from_str(&env, "SHIP-BATCH-1");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));
    client.batch_confirm_milestones(&buyer, &shipment_id, &vec![&env, 0u32]);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(token_client.balance(&supplier), total_amount * 25 / 100);
}

#[test]
fn test_batch_confirm_empty_is_noop() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-BATCH-EMPTY");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    // Empty batch — should succeed without changing anything.
    client.batch_confirm_milestones(&buyer, &shipment_id, &vec![&env]);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.released_amount, 0);
}

#[test]
#[should_panic(expected = "milestone proof not yet submitted")]
fn test_batch_confirm_partial_invalid_reverts() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-BATCH-FAIL");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        total_amount,
    );

    // Only submit proof for index 0; index 1 is still Pending.
    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));

    // Batch includes index 1 which has no proof — must revert entirely.
    client.batch_confirm_milestones(&buyer, &shipment_id, &vec![&env, 0u32, 1u32]);
}

// ============================================================
// #10 — SUPPLIER CANCEL TESTS
// ============================================================

#[test]
fn test_supplier_cancel_happy_path() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let shipment_id = String::from_str(&env, "SHIP-SUPCANCEL");
    let total_amount: i128 = 1_000_000_000;
    let deadline: u32 = 100;
    let penalty_bps: u32 = 500; // 5%

    client.create_shipment(
        &shipment_id,
        &buyer,
        &supplier,
        &logistics,
        &arbiter,
        &token_id,
        &total_amount,
        &build_milestones(&env),
        &deadline,
        &penalty_bps,
    );

    // Submit proof at ledger 0 (default in test env).
    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));

    // Advance ledger past deadline.
    env.ledger().set_sequence_number(deadline + 1);

    let buyer_balance_before = token_client.balance(&buyer);
    client.supplier_cancel(&supplier, &shipment_id);

    let penalty = total_amount * penalty_bps as i128 / 10_000;
    let refund = total_amount - penalty;

    assert_eq!(token_client.balance(&supplier), penalty);
    assert_eq!(token_client.balance(&buyer), buyer_balance_before + refund);
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Cancelled
    );
}

// ============================================================
// ISSUE #4: HOLDBACK TESTS
// ============================================================

#[test]
#[should_panic(expected = "buyer response deadline has not passed")]
fn test_supplier_cancel_premature() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let shipment_id = String::from_str(&env, "SHIP-PREMATURE");

    client.create_shipment(
        &shipment_id,
        &buyer,
        &supplier,
        &logistics,
        &arbiter,
        &token_id,
        &1_000_000_000,
        &build_milestones(&env),
        &1000,
        &500,
    );

    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));

    // Deadline not yet passed — must panic.
    client.supplier_cancel(&supplier, &shipment_id);
}

#[test]
#[should_panic(expected = "supplier cancellation not enabled for this shipment")]
fn test_supplier_cancel_zero_deadline_disabled() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-NODEADLINE");

    // deadline = 0 disables supplier cancellation.
    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        1_000_000_000,
    );

    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));
    client.supplier_cancel(&supplier, &shipment_id);
}

#[test]
fn test_supplier_cancel_penalty_calculation() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);

    let shipment_id = String::from_str(&env, "SHIP-PENALTY");
    let total_amount: i128 = 2_000_000_000;
    let penalty_bps: u32 = 1000; // 10%
    let deadline: u32 = 50;

    client.create_shipment(
        &shipment_id,
        &buyer,
        &supplier,
        &logistics,
        &arbiter,
        &token_id,
        &total_amount,
        &build_milestones(&env),
        &deadline,
        &penalty_bps,
    );

    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));
    env.ledger().set_sequence_number(deadline + 1);

    client.supplier_cancel(&supplier, &shipment_id);

    let expected_penalty = total_amount * penalty_bps as i128 / 10_000; // 200_000_000
    let expected_refund = total_amount - expected_penalty;               // 1_800_000_000

    assert_eq!(token_client.balance(&supplier), expected_penalty);
    // buyer started with 10_000_000_000, spent total_amount, got back refund
    assert_eq!(
        token_client.balance(&buyer),
        10_000_000_000 - total_amount + expected_refund
    );
}

// ============================================================
// #9 — PROPOSE AMENDMENT TESTS
// ============================================================

#[test]
fn test_amendment_full_mutual_consent() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-AMEND");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        1_000_000_000,
    );

    // Milestone 0 is 25%; amend to 30% (milestone 2 stays 25%, milestone 1 becomes 45% to keep sum=100).
    // For simplicity amend milestone 1 from 50% → 45% and milestone 2 from 25% → 30%.
    // Here we just amend milestone 0: 25% → 20%, and milestone 2: 25% → 30% separately.
    // Simplest: amend milestone 0 from 25 → 20, keeping others (50+20+30=100 requires milestone 2 = 30).
    // Let's just amend milestone 2 from 25 → 30 and milestone 1 from 50 → 45 in two separate calls.
    // For this test: amend milestone 0 only: 25 → 25 (same value, valid no-op amendment).
    // Actually let's do a real change: amend milestone 0: 25→20, but that breaks sum unless we also change others.
    // Easiest single-milestone amendment that keeps sum=100: change name only, keep percent same.
    let new_name = String::from_str(&env, "Goods Dispatched v2");

    // Buyer proposes.
    client.propose_amendment(&buyer, &shipment_id, &0, &25, &new_name);

    // Milestone not yet changed (only one party agreed).
    assert_eq!(
        client.get_milestone(&shipment_id, &0).name,
        String::from_str(&env, "Goods Dispatched")
    );

    // Supplier agrees with same terms.
    client.propose_amendment(&supplier, &shipment_id, &0, &25, &new_name);

    // Amendment applied.
    assert_eq!(client.get_milestone(&shipment_id, &0).name, new_name);
    assert_eq!(client.get_milestone(&shipment_id, &0).payment_percent, 25);
}

#[test]
fn test_amendment_mismatched_proposals_no_op() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-MISMATCH");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        1_000_000_000,
    );

    // Buyer proposes 25% with name "A".
    client.propose_amendment(
        &buyer,
        &shipment_id,
        &0,
        &25,
        &String::from_str(&env, "Name A"),
    );

    // Supplier proposes different terms (different name) — mismatch resets proposal.
    client.propose_amendment(
        &supplier,
        &shipment_id,
        &0,
        &25,
        &String::from_str(&env, "Name B"),
    );

    // Milestone unchanged because terms didn't match.
    assert_eq!(
        client.get_milestone(&shipment_id, &0).name,
        String::from_str(&env, "Goods Dispatched")
    );
}

#[test]
#[should_panic(expected = "can only amend a pending milestone")]
fn test_amendment_confirmed_milestone_rejected() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-AMEND-CONF");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        1_000_000_000,
    );

    // Confirm milestone 0.
    client.submit_proof(&supplier, &shipment_id, &0, &String::from_str(&env, "ipfs://d"));
    client.confirm_milestone(&buyer, &shipment_id, &0);

    // Attempt to amend a confirmed milestone — must panic.
    client.propose_amendment(
        &buyer,
        &shipment_id,
        &0,
        &25,
        &String::from_str(&env, "New Name"),
    );
}

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_amendment_invalid_percentage_sum() {
    let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) = setup();
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let shipment_id = String::from_str(&env, "SHIP-AMEND-PCT");

    create_standard_shipment(
        &client, &env, &shipment_id, &buyer, &supplier, &logistics, &arbiter, &token_id,
        1_000_000_000,
    );

    let new_name = String::from_str(&env, "Goods Dispatched");

    // Both parties agree to change milestone 0 from 25% → 50%, which makes total = 125.
    client.propose_amendment(&buyer, &shipment_id, &0, &50, &new_name);
    client.propose_amendment(&supplier, &shipment_id, &0, &50, &new_name);
}
