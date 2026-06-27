#![cfg(test)]

use soroban_sdk::{token, Address, BytesN, Env, String, Symbol, Vec};

use crate::{
    ChainSettleContract, Milestone, MilestoneMode, MilestoneStatus, ShipmentOptions, ShipmentStatus,
};

// ============================================================
// TEST SETUP & HELPERS
// ============================================================

struct OracleTestSetup {
    env: Env,
    chainsettle: ChainSettleContract,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
    admin: Address,
    token_id: Address,
}

fn oracle_setup() -> OracleTestSetup {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();

    let buyer = Address::random(&env);
    let supplier = Address::random(&env);
    let logistics = Address::random(&env);
    let arbiter = Address::random(&env);
    let admin = Address::random(&env);
    let token_admin = Address::random(&env);

    // Deploy mock token
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_client = token::Client::new(&env, &token_id);

    // Mint tokens
    token_client.mint(&buyer, &(100_000 * 10i128.pow(18)));
    token_client.mint(&supplier, &(10_000 * 10i128.pow(18)));

    // Deploy ChainSettle contract
    let client = ChainSettleContract;
    env.register_contract(None, ChainSettleContract);

    // Initialize ChainSettle
    ChainSettleContract::init(env.clone(), admin.clone());

    OracleTestSetup {
        env,
        chainsettle: client,
        buyer,
        supplier,
        logistics,
        arbiter,
        admin,
        token_id,
    }
}

fn build_single_milestone(env: &Env) -> Vec<Milestone> {
    let mut v = Vec::new(&env);
    v.push_back(Milestone {
        name: String::from_str(&env, "Delivery"),
        payment_percent: 100,
        proof_hash: String::from_str(&env, ""),
        status: MilestoneStatus::Pending,
        release_after_ledger: 0,
        proof_submitted_ledger: None,
        dispute_opened_ledger: None,
    });
    v
}

fn build_three_milestones(env: &Env) -> Vec<Milestone> {
    let mut v = Vec::new(&env);
    v.push_back(Milestone {
        name: String::from_str(&env, "Dispatch"),
        payment_percent: 33,
        proof_hash: String::from_str(&env, ""),
        status: MilestoneStatus::Pending,
        release_after_ledger: 0,
        proof_submitted_ledger: None,
        dispute_opened_ledger: None,
    });
    v.push_back(Milestone {
        name: String::from_str(&env, "Transit"),
        payment_percent: 33,
        proof_hash: String::from_str(&env, ""),
        status: MilestoneStatus::Pending,
        release_after_ledger: 0,
        proof_submitted_ledger: None,
        dispute_opened_ledger: None,
    });
    v.push_back(Milestone {
        name: String::from_str(&env, "Delivery"),
        payment_percent: 34,
        proof_hash: String::from_str(&env, ""),
        status: MilestoneStatus::Pending,
        release_after_ledger: 0,
        proof_submitted_ledger: None,
        dispute_opened_ledger: None,
    });
    v
}

fn default_options(env: &Env) -> ShipmentOptions {
    ShipmentOptions {
        response_deadline: 0,
        penalty_bps: 0,
        milestone_mode: MilestoneMode::Parallel,
        holdback_ledgers: 0,
        dispute_cooldown_ledgers: 0,
        late_penalty_bps_per_ledger: 0,
        auto_confirm_ledgers: 0,
        dispute_bond_amount: 0,
        arbiter_fee_bps: 0,
        logistics_fee_bps: 0,
        supplier_collateral: 0,
        expires_at_ledger: None,

        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,

    }
}

// ============================================================
// TESTS: ORACLE INTEGRATION PATTERNS
// ============================================================

#[test]
fn test_oracle_pattern_proof_verified_auto_confirm() {
    let setup = oracle_setup();

    let milestones = build_single_milestone(&setup.env);
    let shipment_id = String::from_str(&setup.env, "shipment-oracle-001");
    let total_amount = 1_000 * 10i128.pow(18);

    let buyers = {
        let mut v = Vec::new(&setup.env);
        v.push_back(setup.buyer.clone());
        v
    };

    let options = default_options(&setup.env);

    // Create shipment
    ChainSettleContract::create_shipment(
        setup.env.clone(),
        shipment_id.clone(),
        buyers.clone(),
        setup.supplier.clone(),
        setup.logistics.clone(),
        setup.arbiter.clone(),
        setup.token_id.clone(),
        total_amount,
        milestones.clone(),
        options,
    );

    // Verify shipment created
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.total_amount, total_amount);
    assert_eq!(shipment.milestones.len(), 1);

    // Simulate oracle verification: submit proof
    let proof_hash = String::from_str(&setup.env, "QmVerifiedProof123");
    ChainSettleContract::submit_proof(
        setup.env.clone(),
        setup.supplier.clone(),
        shipment_id.clone(),
        0u32,
        proof_hash.clone(),
        Symbol::new(&setup.env, "ipfs"),
    );

    // Verify milestone in ProofSubmitted state
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    let milestone = shipment.milestones.get(0).unwrap();
    assert_eq!(milestone.status, MilestoneStatus::ProofSubmitted);
    assert_eq!(milestone.proof_hash, proof_hash);

    // Buyer confirms (oracle pre-verified, buyer trusts oracle)
    let mut indices = Vec::new(&setup.env);
    indices.push_back(0u32);
    ChainSettleContract::confirm_milestones(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        indices,
    );

    // Verify milestone confirmed and payment released
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    let milestone = shipment.milestones.get(0).unwrap();
    assert_eq!(milestone.status, MilestoneStatus::Confirmed);

    let token_client = token::Client::new(&setup.env, &setup.token_id);
    let supplier_final = token_client.balance(&setup.supplier);
    assert!(supplier_final > 0, "Supplier received payment");
}

#[test]
fn test_oracle_pattern_proof_rejected_raises_dispute() {
    let setup = oracle_setup();

    let milestones = build_single_milestone(&setup.env);
    let shipment_id = String::from_str(&setup.env, "shipment-oracle-rejected");
    let total_amount = 1_000 * 10i128.pow(18);

    let buyers = {
        let mut v = Vec::new(&setup.env);
        v.push_back(setup.buyer.clone());
        v
    };

    let options = default_options(&setup.env);

    // Create shipment
    ChainSettleContract::create_shipment(
        setup.env.clone(),
        shipment_id.clone(),
        buyers.clone(),
        setup.supplier.clone(),
        setup.logistics.clone(),
        setup.arbiter.clone(),
        setup.token_id.clone(),
        total_amount,
        milestones.clone(),
        options,
    );

    // Submit proof (not verified by oracle)
    let proof_hash = String::from_str(&setup.env, "QmRejectedProof");
    ChainSettleContract::submit_proof(
        setup.env.clone(),
        setup.supplier.clone(),
        shipment_id.clone(),
        0u32,
        proof_hash,
        Symbol::new(&setup.env, "ipfs"),
    );

    // Milestone is now ProofSubmitted
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::ProofSubmitted
    );

    // Buyer disputes (oracle rejected the proof)
    ChainSettleContract::raise_dispute(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        0u32,
    );

    // Verify milestone is disputed
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Disputed
    );
}

#[test]
fn test_oracle_pattern_multiple_proofs_cross_contract() {
    let setup = oracle_setup();

    let milestones = build_three_milestones(&setup.env);
    let shipment_id = String::from_str(&setup.env, "shipment-oracle-multi");
    let total_amount = 3_000 * 10i128.pow(18);

    let buyers = {
        let mut v = Vec::new(&setup.env);
        v.push_back(setup.buyer.clone());
        v
    };

    let options = default_options(&setup.env);

    // Create shipment
    ChainSettleContract::create_shipment(
        setup.env.clone(),
        shipment_id.clone(),
        buyers.clone(),
        setup.supplier.clone(),
        setup.logistics.clone(),
        setup.arbiter.clone(),
        setup.token_id.clone(),
        total_amount,
        milestones.clone(),
        options,
    );

    // Submit all proofs sequentially
    for i in 0..3 {
        let proof_hash = String::from_str(&setup.env, &format!("QmProof{}", i));
        ChainSettleContract::submit_proof(
            setup.env.clone(),
            setup.supplier.clone(),
            shipment_id.clone(),
            i as u32,
            proof_hash,
            Symbol::new(&setup.env, "ipfs"),
        );
    }

    // Verify all milestones are ProofSubmitted
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    for i in 0..3 {
        assert_eq!(
            shipment.milestones.get(i).unwrap().status,
            MilestoneStatus::ProofSubmitted
        );
    }

    // Buyer confirms all milestones (all oracle-verified)
    let mut indices = Vec::new(&setup.env);
    indices.push_back(0u32);
    indices.push_back(1u32);
    indices.push_back(2u32);

    ChainSettleContract::confirm_milestones(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        indices,
    );

    // Verify shipment completed
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
}

#[test]
fn test_oracle_pattern_partial_approval_mixed_proofs() {
    let setup = oracle_setup();

    let milestones = build_three_milestones(&setup.env);
    let shipment_id = String::from_str(&setup.env, "shipment-oracle-partial");
    let total_amount = 3_000 * 10i128.pow(18);

    let buyers = {
        let mut v = Vec::new(&setup.env);
        v.push_back(setup.buyer.clone());
        v
    };

    let options = default_options(&setup.env);

    // Create shipment
    ChainSettleContract::create_shipment(
        setup.env.clone(),
        shipment_id.clone(),
        buyers.clone(),
        setup.supplier.clone(),
        setup.logistics.clone(),
        setup.arbiter.clone(),
        setup.token_id.clone(),
        total_amount,
        milestones.clone(),
        options,
    );

    // Submit proofs for all 3 milestones
    for i in 0..3 {
        let proof_hash = String::from_str(&setup.env, &format!("QmProof{}", i));
        ChainSettleContract::submit_proof(
            setup.env.clone(),
            setup.supplier.clone(),
            shipment_id.clone(),
            i as u32,
            proof_hash,
            Symbol::new(&setup.env, "ipfs"),
        );
    }

    // Buyer confirms only milestones 0 and 2 (oracle rejected milestone 1)
    let mut indices = Vec::new(&setup.env);
    indices.push_back(0u32);
    indices.push_back(2u32);

    ChainSettleContract::confirm_milestones(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        indices,
    );

    // Verify partial confirmation
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(
        shipment.milestones.get(1).unwrap().status,
        MilestoneStatus::ProofSubmitted
    );
    assert_eq!(
        shipment.milestones.get(2).unwrap().status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(shipment.status, ShipmentStatus::Active);
}

#[test]
fn test_oracle_pattern_dispute_after_rejection() {
    let setup = oracle_setup();

    let milestones = build_three_milestones(&setup.env);
    let shipment_id = String::from_str(&setup.env, "shipment-oracle-dispute");
    let total_amount = 3_000 * 10i128.pow(18);

    let buyers = {
        let mut v = Vec::new(&setup.env);
        v.push_back(setup.buyer.clone());
        v
    };

    let options = ShipmentOptions {
        response_deadline: 0,
        penalty_bps: 0,
        milestone_mode: MilestoneMode::Parallel,
        holdback_ledgers: 0,
        dispute_cooldown_ledgers: 0,
        late_penalty_bps_per_ledger: 0,
        auto_confirm_ledgers: 0,
        dispute_bond_amount: 0,
        arbiter_fee_bps: 0,
        logistics_fee_bps: 0,
        supplier_collateral: 0,
        expires_at_ledger: None,

        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,

    };

    // Create shipment
    ChainSettleContract::create_shipment(
        setup.env.clone(),
        shipment_id.clone(),
        buyers.clone(),
        setup.supplier.clone(),
        setup.logistics.clone(),
        setup.arbiter.clone(),
        setup.token_id.clone(),
        total_amount,
        milestones.clone(),
        options,
    );

    // Submit proof for milestone 0
    ChainSettleContract::submit_proof(
        setup.env.clone(),
        setup.supplier.clone(),
        shipment_id.clone(),
        0u32,
        String::from_str(&setup.env, "QmProof0"),
        Symbol::new(&setup.env, "ipfs"),
    );

    // Buyer raises dispute (oracle rejected)
    ChainSettleContract::raise_dispute(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        0u32,
    );

    // Verify dispute status
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Disputed
    );

    // Arbiter resolves: approves the proof despite oracle rejection
    ChainSettleContract::resolve_dispute(
        setup.env.clone(),
        setup.arbiter.clone(),
        shipment_id.clone(),
        0u32,
        true, // approve
        String::from_str(&setup.env, "Proof valid despite oracle"),
    );

    // Verify dispute resolved and milestone confirmed
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id.clone());
    assert_eq!(
        shipment.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed
    );
}

#[test]
fn test_oracle_pattern_shipment_lifecycle_oracle_verified() {
    let setup = oracle_setup();

    let milestones = build_three_milestones(&setup.env);
    let shipment_id = String::from_str(&setup.env, "shipment-oracle-full-lifecycle");
    let total_amount = 3_000 * 10i128.pow(18);

    let buyers = {
        let mut v = Vec::new(&setup.env);
        v.push_back(setup.buyer.clone());
        v
    };

    let options = default_options(&setup.env);

    // Create shipment
    ChainSettleContract::create_shipment(
        setup.env.clone(),
        shipment_id.clone(),
        buyers.clone(),
        setup.supplier.clone(),
        setup.logistics.clone(),
        setup.arbiter.clone(),
        setup.token_id.clone(),
        total_amount,
        milestones.clone(),
        options,
    );

    let token_client = token::Client::new(&setup.env, &setup.token_id);

    // Milestone 0: Dispatch
    ChainSettleContract::submit_proof(
        setup.env.clone(),
        setup.supplier.clone(),
        shipment_id.clone(),
        0u32,
        String::from_str(&setup.env, "QmDispatch"),
        Symbol::new(&setup.env, "ipfs"),
    );

    let mut idx = Vec::new(&setup.env);
    idx.push_back(0u32);
    ChainSettleContract::confirm_milestones(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        idx,
    );

    let mut balance = token_client.balance(&setup.supplier);
    let expected = (total_amount * 33) / 100;
    assert!(balance > 0, "Supplier should receive payment for milestone 0");

    // Milestone 1: Transit
    ChainSettleContract::submit_proof(
        setup.env.clone(),
        setup.logistics.clone(),
        shipment_id.clone(),
        1u32,
        String::from_str(&setup.env, "QmTransit"),
        Symbol::new(&setup.env, "ipfs"),
    );

    let mut idx = Vec::new(&setup.env);
    idx.push_back(1u32);
    ChainSettleContract::confirm_milestones(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        idx,
    );

    // Milestone 2: Delivery
    ChainSettleContract::submit_proof(
        setup.env.clone(),
        setup.supplier.clone(),
        shipment_id.clone(),
        2u32,
        String::from_str(&setup.env, "QmDelivery"),
        Symbol::new(&setup.env, "ipfs"),
    );

    let mut idx = Vec::new(&setup.env);
    idx.push_back(2u32);
    ChainSettleContract::confirm_milestones(
        setup.env.clone(),
        setup.buyer.clone(),
        shipment_id.clone(),
        idx,
    );

    // Verify complete lifecycle
    let shipment = ChainSettleContract::get_shipment(setup.env.clone(), shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    for i in 0..3 {
        assert_eq!(
            shipment.milestones.get(i).unwrap().status,
            MilestoneStatus::Confirmed
        );
    }
}
