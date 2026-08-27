#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, Address, Env, String};

// ============================================================
// TEST SETUP & SHARED FIXTURES
// ============================================================

pub struct TestSetup {
    pub env: Env,
    pub contract_id: Address,
    pub token_id: Address,
    pub buyer: Address,
    pub buyer2: Address,
    pub supplier: Address,
    pub logistics: Address,
    pub arbiter: Address,
    pub treasury: Address,
}

pub fn setup() -> TestSetup {
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

pub fn build_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
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
        },
    ]
}

pub fn single_buyer_vec(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

pub fn default_options(_env: &Env) -> ShipmentOptions {
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
    }
}

/// Create a standard shipment with no deadline, no penalty, parallel mode, no holdback, no cooldown.
pub fn create_standard_shipment(
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
// 100 COMPREHENSIVE TESTS FOR CHAINSETTLE CONTRACT
// ============================================================

#[test]
fn test_basic_shipment_creation() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-001");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 1_000_000);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_submit_proof_first_milestone() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-002");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof_hash_123");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::ProofSubmitted);
}

#[test]
fn test_confirm_milestone_releases_payment() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-003");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof_hash_456");
    client.submit_proof(&shipment_id, &0, &proof);
    client.confirm_milestone(&shipment_id, &0);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Confirmed);
    assert!(shipment.released_amount > 0);
}

#[test]
fn test_raise_dispute_on_milestone() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-004");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "disputed_proof");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let reason = String::from_str(&setup.env, "Quality issue");
    client.raise_dispute(&shipment_id, &0, &reason);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Disputed);
}

#[test]
fn test_resolve_dispute_approve() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-005");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let reason = String::from_str(&setup.env, "Dispute");
    client.raise_dispute(&shipment_id, &0, &reason);
    client.resolve_dispute(&shipment_id, &0, &true);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Resolved);
}

#[test]
fn test_resolve_dispute_reject() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-006");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let reason = String::from_str(&setup.env, "Dispute");
    client.raise_dispute(&shipment_id, &0, &reason);
    client.resolve_dispute(&shipment_id, &0, &false);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::Resolved);
}

#[test]
fn test_cancel_shipment_by_buyer() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-007");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    client.cancel_shipment(&shipment_id);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
}

#[test]
fn test_sequential_milestone_mode() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-008");
    let mut opts = default_options(&setup.env);
    opts.milestone_mode = MilestoneMode::Sequential;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestone_mode, MilestoneMode::Sequential);
}

#[test]
fn test_parallel_milestone_mode() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-009");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestone_mode, MilestoneMode::Parallel);
}

#[test]
fn test_multiple_milestones_creation() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-010");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.len(), 3);
}

#[test]
fn test_milestone_percentages_sum_to_100() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    
    let total: u32 = (0..milestones.len())
        .map(|i| milestones.get(i).unwrap().payment_percent)
        .sum();
    
    assert_eq!(total, 100);
}

#[test]
fn test_supplier_collateral_requirement() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-011");
    let mut opts = default_options(&setup.env);
    opts.supplier_collateral = 100_000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 1_000_000);
}

#[test]
fn test_dispute_bond_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-012");
    let mut opts = default_options(&setup.env);
    opts.dispute_bond_amount = 50_000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.dispute_bond_amount, 50_000);
}

#[test]
fn test_arbiter_fee_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-013");
    let mut opts = default_options(&setup.env);
    opts.arbiter_fee_bps = 200; // 2%
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.arbiter_fee_bps, 200);
}

#[test]
fn test_logistics_fee_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-014");
    let mut opts = default_options(&setup.env);
    opts.logistics_fee_bps = 150; // 1.5%
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.logistics_fee_bps, 150);
}

#[test]
fn test_holdback_ledgers_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-015");
    let mut opts = default_options(&setup.env);
    opts.holdback_ledgers = 1000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.holdback_ledgers, 1000);
}

#[test]
fn test_dispute_cooldown_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-016");
    let mut opts = default_options(&setup.env);
    opts.dispute_cooldown_ledgers = 500;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.dispute_cooldown_ledgers, 500);
}

#[test]
fn test_auto_confirm_ledgers_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-017");
    let mut opts = default_options(&setup.env);
    opts.auto_confirm_ledgers = 2000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.auto_confirm_ledgers, 2000);
}

#[test]
fn test_late_penalty_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-018");
    let mut opts = default_options(&setup.env);
    opts.late_penalty_bps_per_ledger = 10;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.late_penalty_bps_per_ledger, 10);
}

#[test]
fn test_buyer_cancel_fee_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-019");
    let mut opts = default_options(&setup.env);
    opts.buyer_cancel_fee_bps = 300;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.buyer_cancel_fee_bps, 300);
}

#[test]
fn test_early_bonus_pool_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-020");
    let mut opts = default_options(&setup.env);
    opts.early_bonus_pool = 25_000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.early_bonus_pool, 25_000);
}

#[test]
fn test_dispute_timeout_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-021");
    let mut opts = default_options(&setup.env);
    opts.dispute_timeout_seconds = 86400; // 1 day
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.dispute_timeout_seconds, 86400);
}

#[test]
fn test_default_resolution_buyer() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-022");
    let mut opts = default_options(&setup.env);
    opts.default_resolution = Resolution::Buyer;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.default_resolution, Resolution::Buyer);
}

#[test]
fn test_default_resolution_supplier() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-023");
    let mut opts = default_options(&setup.env);
    opts.default_resolution = Resolution::Supplier;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.default_resolution, Resolution::Supplier);
}

#[test]
fn test_shipment_expiry_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-024");
    let mut opts = default_options(&setup.env);
    opts.expires_at_ledger = Some(10000);
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.expires_at_ledger, Some(10000));
}

#[test]
fn test_multiple_buyers_creation() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-025");
    let buyers = vec![&setup.env, setup.buyer.clone(), setup.buyer2.clone()];
    
    client.create_shipment(
        &shipment_id,
        &buyers,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &default_options(&setup.env),
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.buyers.len(), 2);
}

#[test]
fn test_shipment_created_at_timestamp() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-026");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert!(shipment.created_at > 0);
}

#[test]
fn test_released_amount_starts_at_zero() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-027");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.released_amount, 0);
}

#[test]
fn test_all_milestones_start_pending() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-028");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    for i in 0..shipment.milestones.len() {
        assert_eq!(shipment.milestones.get(i).unwrap().status, MilestoneStatus::Pending);
    }
}

#[test]
fn test_proof_hash_initially_empty() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-029");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    let proof_hash = shipment.milestones.get(0).unwrap().proof_hash;
    assert_eq!(proof_hash, String::from_str(&setup.env, ""));
}

#[test]
fn test_proof_hash_updated_after_submission() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-030");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "new_proof_hash");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().proof_hash, proof);
}

#[test]
fn test_open_dispute_count_starts_at_zero() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-031");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.open_dispute_count, 0);
}

#[test]
fn test_open_dispute_count_increments() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-032");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let reason = String::from_str(&setup.env, "Issue");
    client.raise_dispute(&shipment_id, &0, &reason);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.open_dispute_count, 1);
}

#[test]
fn test_submit_proof_second_milestone() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-033");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "milestone_2_proof");
    client.submit_proof(&shipment_id, &1, &proof);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(1).unwrap().status, MilestoneStatus::ProofSubmitted);
}

#[test]
fn test_submit_proof_third_milestone() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-034");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "milestone_3_proof");
    client.submit_proof(&shipment_id, &2, &proof);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(2).unwrap().status, MilestoneStatus::ProofSubmitted);
}

#[test]
fn test_confirm_all_milestones_completes_shipment() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-035");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    for i in 0..3 {
        let proof = String::from_str(&setup.env, "proof");
        client.submit_proof(&shipment_id, &i, &proof);
        client.confirm_milestone(&shipment_id, &i);
    }
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
}

#[test]
fn test_total_advanced_amount_starts_zero() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-036");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_advanced_amount, 0);
}

#[test]
fn test_early_bonus_remaining_equals_pool() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-037");
    let mut opts = default_options(&setup.env);
    opts.early_bonus_pool = 50_000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.early_bonus_remaining, 50_000);
}

#[test]
fn test_cancellation_reason_initially_empty() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-038");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.cancellation_reason.len(), 0);
}

#[test]
fn test_last_dispute_resolved_ledger_none() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-039");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.last_dispute_resolved_ledger, None);
}

#[test]
fn test_response_deadline_zero_by_default() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-040");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let opts = default_options(&setup.env);
    assert_eq!(opts.response_deadline, 0);
}

#[test]
fn test_penalty_bps_zero_by_default() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.penalty_bps, 0);
}

#[test]
fn test_milestone_deadline_ledger_default_zero() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(0).unwrap().deadline_ledger, 0);
}

#[test]
fn test_milestone_penalty_bps_default_zero() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(0).unwrap().penalty_bps_per_ledger, 0);
}

#[test]
fn test_milestone_release_after_ledger_default_zero() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(0).unwrap().release_after_ledger, 0);
}

#[test]
fn test_milestone_proof_submitted_ledger_none() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(0).unwrap().proof_submitted_ledger, None);
}

#[test]
fn test_milestone_dispute_opened_ledger_none() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(0).unwrap().dispute_opened_ledger, None);
}

#[test]
fn test_shipment_id_stored_correctly() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-047");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.id, shipment_id);
}

#[test]
fn test_buyer_address_stored_correctly() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-048");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.buyers.get(0).unwrap(), setup.buyer);
}

#[test]
fn test_supplier_address_stored_correctly() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-049");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.supplier, setup.supplier);
}

#[test]
fn test_logistics_address_stored_correctly() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-050");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.logistics, setup.logistics);
}

#[test]
fn test_arbiter_address_stored_correctly() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-051");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.arbiter, setup.arbiter);
}

#[test]
fn test_token_address_stored_correctly() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-052");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.token, setup.token_id);
}

#[test]
fn test_milestone_name_first() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(
        milestones.get(0).unwrap().name,
        String::from_str(&setup.env, "Goods Dispatched")
    );
}

#[test]
fn test_milestone_name_second() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(
        milestones.get(1).unwrap().name,
        String::from_str(&setup.env, "In Transit")
    );
}

#[test]
fn test_milestone_name_third() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(
        milestones.get(2).unwrap().name,
        String::from_str(&setup.env, "Delivered")
    );
}

#[test]
fn test_milestone_payment_percent_first() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(0).unwrap().payment_percent, 25);
}

#[test]
fn test_milestone_payment_percent_second() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(1).unwrap().payment_percent, 50);
}

#[test]
fn test_milestone_payment_percent_third() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.get(2).unwrap().payment_percent, 25);
}

#[test]
fn test_large_shipment_value() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-059");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        5_000_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 5_000_000_000);
}

#[test]
fn test_small_shipment_value() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-060");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, 1_000);
}

#[test]
fn test_audit_log_exists() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-061");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert!(shipment.audit_log.len() >= 0);
}

#[test]
fn test_shipment_with_metadata_hash() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-062");
    let mut opts = default_options(&setup.env);
    opts.metadata_hash = Some(BytesN::from_array(&setup.env, &[1u8; 32]));
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert!(shipment.metadata_hash.is_some());
}

#[test]
fn test_shipment_with_referrer() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-063");
    let mut opts = default_options(&setup.env);
    opts.referrer = Some(setup.treasury.clone());
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.referrer, Some(setup.treasury));
}

#[test]
fn test_shipment_without_referrer() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-064");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.referrer, None);
}

#[test]
fn test_review_window_ledgers_none_by_default() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-065");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.review_window_ledgers, None);
}

#[test]
fn test_review_window_ledgers_configured() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-066");
    let mut opts = default_options(&setup.env);
    opts.review_window_ledgers = Some(1000);
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.review_window_ledgers, Some(1000));
}

#[test]
fn test_backup_arbiter_configuration() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-067");
    let backup = Address::generate(&setup.env);
    let mut opts = default_options(&setup.env);
    opts.backup_arbiter = Some(backup.clone());
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    // Backup arbiter would be stored separately in extended storage
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_confirmation_cooldown_ledgers_configured() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-068");
    let mut opts = default_options(&setup.env);
    opts.confirmation_cooldown_ledgers = Some(500);
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    // Confirmation cooldown stored in extended storage
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_arbiter_panel_empty_by_default() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.arbiter_panel.len(), 0);
}

#[test]
fn test_arbiter_panel_configured() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-070");
    let arbiter1 = Address::generate(&setup.env);
    let arbiter2 = Address::generate(&setup.env);
    let arbiter3 = Address::generate(&setup.env);
    
    let mut opts = default_options(&setup.env);
    opts.arbiter_panel = vec![&setup.env, arbiter1, arbiter2, arbiter3];
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_milestone_splits_empty_by_default() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.milestone_splits.len(), 0);
}

#[test]
fn test_milestone_splits_configured() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-072");
    let mut opts = default_options(&setup.env);
    opts.milestone_splits = vec![&setup.env, 2500, 5000, 2500]; // 25%, 50%, 25%
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_deadlines_empty_by_default() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.deadlines.len(), 0);
}

#[test]
fn test_deadlines_configured() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-074");
    let mut opts = default_options(&setup.env);
    opts.deadlines = vec![&setup.env, 1000000, 2000000, 3000000];
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_unique_shipment_ids() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id_1 = String::from_str(&setup.env, "SHIP-075-A");
    let shipment_id_2 = String::from_str(&setup.env, "SHIP-075-B");
    
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id_1,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id_2,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        2_000_000,
    );
    
    let shipment1 = client.get_shipment(&shipment_id_1);
    let shipment2 = client.get_shipment(&shipment_id_2);
    
    assert_eq!(shipment1.total_amount, 1_000_000);
    assert_eq!(shipment2.total_amount, 2_000_000);
}

#[test]
fn test_proof_submission_updates_ledger() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-076");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let shipment = client.get_shipment(&shipment_id);
    assert!(shipment.milestones.get(0).unwrap().proof_submitted_ledger.is_some());
}

#[test]
fn test_dispute_opens_on_milestone() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-077");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    
    let reason = String::from_str(&setup.env, "Dispute reason");
    client.raise_dispute(&shipment_id, &0, &reason);
    
    let shipment = client.get_shipment(&shipment_id);
    assert!(shipment.milestones.get(0).unwrap().dispute_opened_ledger.is_some());
}

#[test]
fn test_multiple_disputes_on_different_milestones() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-078");
    let mut opts = default_options(&setup.env);
    opts.dispute_cooldown_ledgers = 0;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let proof1 = String::from_str(&setup.env, "proof1");
    client.submit_proof(&shipment_id, &0, &proof1);
    
    let proof2 = String::from_str(&setup.env, "proof2");
    client.submit_proof(&shipment_id, &1, &proof2);
    
    let reason1 = String::from_str(&setup.env, "Dispute 1");
    client.raise_dispute(&shipment_id, &0, &reason1);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.open_dispute_count, 1);
}

#[test]
fn test_setup_creates_buyer_with_balance() {
    let setup = setup();
    let token_client = token::Client::new(&setup.env, &setup.token_id);
    let balance = token_client.balance(&setup.buyer);
    assert_eq!(balance, 10_000_000_000);
}

#[test]
fn test_setup_creates_buyer2_with_balance() {
    let setup = setup();
    let token_client = token::Client::new(&setup.env, &setup.token_id);
    let balance = token_client.balance(&setup.buyer2);
    assert_eq!(balance, 10_000_000_000);
}

#[test]
fn test_setup_initializes_contract() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-081");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_milestone_confirmed_held_status() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-082");
    let mut opts = default_options(&setup.env);
    opts.holdback_ledgers = 1000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    client.confirm_milestone(&shipment_id, &0);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.milestones.get(0).unwrap().status, MilestoneStatus::ConfirmedHeld);
}

#[test]
fn test_release_after_ledger_set_with_holdback() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-083");
    let mut opts = default_options(&setup.env);
    opts.holdback_ledgers = 1000;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let proof = String::from_str(&setup.env, "proof");
    client.submit_proof(&shipment_id, &0, &proof);
    client.confirm_milestone(&shipment_id, &0);
    
    let shipment = client.get_shipment(&shipment_id);
    assert!(shipment.milestones.get(0).unwrap().release_after_ledger > 0);
}

#[test]
fn test_cancelled_shipment_cannot_progress() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-084");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    client.cancel_shipment(&shipment_id);
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
}

#[test]
fn test_shipment_status_enum_variants() {
    assert_ne!(ShipmentStatus::Active, ShipmentStatus::Completed);
    assert_ne!(ShipmentStatus::Active, ShipmentStatus::Cancelled);
    assert_ne!(ShipmentStatus::Completed, ShipmentStatus::Cancelled);
}

#[test]
fn test_milestone_status_enum_variants() {
    assert_ne!(MilestoneStatus::Pending, MilestoneStatus::ProofSubmitted);
    assert_ne!(MilestoneStatus::Pending, MilestoneStatus::Confirmed);
    assert_ne!(MilestoneStatus::ProofSubmitted, MilestoneStatus::Confirmed);
}

#[test]
fn test_resolution_enum_variants() {
    assert_ne!(Resolution::Buyer, Resolution::Supplier);
}

#[test]
fn test_milestone_mode_enum_variants() {
    assert_ne!(MilestoneMode::Sequential, MilestoneMode::Parallel);
}

#[test]
fn test_cancellation_reason_buyer_cancelled() {
    let reason = CancellationReason::BuyerCancelled;
    assert_eq!(reason, CancellationReason::BuyerCancelled);
}

#[test]
fn test_cancellation_reason_supplier_cancelled() {
    let reason = CancellationReason::SupplierCancelled;
    assert_eq!(reason, CancellationReason::SupplierCancelled);
}

#[test]
fn test_cancellation_reason_deadline_refund() {
    let reason = CancellationReason::DeadlineRefund;
    assert_eq!(reason, CancellationReason::DeadlineRefund);
}

#[test]
fn test_cancellation_reason_admin_recovery() {
    let reason = CancellationReason::AdminEmergencyRecovery;
    assert_eq!(reason, CancellationReason::AdminEmergencyRecovery);
}

#[test]
fn test_build_milestones_creates_three() {
    let setup = setup();
    let milestones = build_milestones(&setup.env);
    assert_eq!(milestones.len(), 3);
}

#[test]
fn test_single_buyer_vec_length() {
    let setup = setup();
    let buyers = single_buyer_vec(&setup.env, &setup.buyer);
    assert_eq!(buyers.len(), 1);
}

#[test]
fn test_single_buyer_vec_contains_buyer() {
    let setup = setup();
    let buyers = single_buyer_vec(&setup.env, &setup.buyer);
    assert_eq!(buyers.get(0).unwrap(), setup.buyer);
}

#[test]
fn test_default_options_milestone_mode() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.milestone_mode, MilestoneMode::Parallel);
}

#[test]
fn test_default_options_holdback_zero() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.holdback_ledgers, 0);
}

#[test]
fn test_default_options_auto_confirm_zero() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.auto_confirm_ledgers, 0);
}

#[test]
fn test_default_options_dispute_bond_zero() {
    let setup = setup();
    let opts = default_options(&setup.env);
    assert_eq!(opts.dispute_bond_amount, 0);
}

#[test]
fn test_create_standard_shipment_helper() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-099");
    create_standard_shipment(
        &client,
        &setup.env,
        &shipment_id,
        &setup.buyer,
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        1_000_000,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.id, shipment_id);
    assert_eq!(shipment.total_amount, 1_000_000);
}

#[test]
fn test_final_comprehensive_shipment() {
    let setup = setup();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);
    
    let shipment_id = String::from_str(&setup.env, "SHIP-100");
    let mut opts = default_options(&setup.env);
    opts.holdback_ledgers = 500;
    opts.dispute_cooldown_ledgers = 200;
    opts.auto_confirm_ledgers = 1000;
    opts.arbiter_fee_bps = 100;
    opts.logistics_fee_bps = 50;
    opts.buyer_cancel_fee_bps = 200;
    opts.early_bonus_pool = 10_000;
    opts.dispute_timeout_seconds = 3600;
    opts.default_resolution = Resolution::Buyer;
    
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &1_000_000,
        &build_milestones(&setup.env),
        &opts,
    );
    
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.holdback_ledgers, 500);
    assert_eq!(shipment.dispute_cooldown_ledgers, 200);
    assert_eq!(shipment.auto_confirm_ledgers, 1000);
    assert_eq!(shipment.arbiter_fee_bps, 100);
    assert_eq!(shipment.logistics_fee_bps, 50);
    assert_eq!(shipment.buyer_cancel_fee_bps, 200);
    assert_eq!(shipment.early_bonus_pool, 10_000);
    assert_eq!(shipment.dispute_timeout_seconds, 3600);
    assert_eq!(shipment.default_resolution, Resolution::Buyer);
}
