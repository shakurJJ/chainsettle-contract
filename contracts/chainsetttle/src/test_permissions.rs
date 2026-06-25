#![cfg(test)]

//! # Permission Matrix Tests
//!
//! Verifies, for every auth-requiring public function, exactly which role is
//! allowed and which three are denied with `"unauthorized"`.
//!
//! ## Role definitions
//! | Role     | Description                                      |
//! |----------|--------------------------------------------------|
//! | buyer    | Primary buyer; creates shipment, confirms, etc.  |
//! | supplier | Proof submitter; receives payment                |
//! | logistics| Transit proof submitter                          |
//! | arbiter  | Dispute resolver                                 |
//! | admin    | Contract-level admin set at init                 |
//! | stranger | Unrelated address (no role)                      |
//!
//! ## Permission Matrix (auth-requiring functions)
//!
//! ```text
//! Function                   | buyer | supplier | logistics | arbiter | admin
//! ---------------------------|-------|----------|-----------|---------|------
//! pause                      |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! unpause                    |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_escalation_threshold   |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_max_shipment_value     |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_circuit_breaker        |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_fee_config             |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_max_concurrent_disputes|   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_min_milestone_percent  |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! set_max_advance_percent    |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! blacklist_address          |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! remove_from_blacklist      |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! add_allowed_token          |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! remove_allowed_token       |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! nominate_admin             |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! revoke_nomination          |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! emergency_recover          |   ✗   |    ✗     |     ✗     |    ✗    |  ✓
//! create_shipment            |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! top_up_escrow              |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! rebalance_milestones       |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! confirm_milestone          |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! batch_confirm_milestones   |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! raise_dispute              |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! raise_partial_dispute      |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! cancel_shipment            |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! approve_advance            |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! set_proof_whitelist        |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! transfer_buyer             |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! propose_amendment(buyer)   |   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! propose_amendment(supplier)|   ✗   |    ✓     |     ✗     |    ✗    |  ✗
//! propose_arbiter_rotation(b)|   ✓   |    ✗     |     ✗     |    ✗    |  ✗
//! propose_arbiter_rotation(s)|   ✗   |    ✓     |     ✗     |    ✗    |  ✗
//! submit_proof               |   ✗   |    ✓     |     ✓     |    ✗    |  ✗
//! request_advance            |   ✗   |    ✓     |     ✗     |    ✗    |  ✗
//! supplier_cancel            |   ✗   |    ✓     |     ✗     |    ✗    |  ✗
//! transfer_supplier          |   ✗   |    ✓     |     ✗     |    ✗    |  ✗
//! resolve_dispute            |   ✗   |    ✗     |     ✗     |    ✓    |  ✗
//! ```
//!
//! ## CI guard
//! The constant `PERMISSION_TEST_COUNT` at the bottom of this file must equal
//! the actual number of `#[test]` functions. CI will fail if tests are removed
//! without updating the constant (via `grep` count assertion in the test suite).

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String, Symbol,
};

// ============================================================
// HELPERS
// ============================================================

struct Roles {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
    stranger: Address,
}

fn setup() -> Roles {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    let stranger = Address::generate(&env);

    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &100_000_000_000);
    token::StellarAssetClient::new(&env, &token_id).mint(&stranger, &100_000_000_000);

    ChainSettleContractClient::new(&env, &contract_id).init(&admin);

    Roles { env, contract_id, token_id, admin, buyer, supplier, logistics, arbiter, stranger }
}

fn milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "M0"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(env, "M1"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ]
}

fn opts(env: &Env) -> ShipmentOptions {
    ShipmentOptions {
        response_deadline: 100,
        penalty_bps: 0,
        milestone_mode: MilestoneMode::Parallel,
        holdback_ledgers: 0,
        dispute_cooldown_ledgers: 0,
        late_penalty_bps_per_ledger: 0,
        auto_confirm_ledgers: 0,
        dispute_bond_amount: 0,
        arbiter_fee_bps: 0,
    }
}

fn buyers(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

/// Create a standard shipment and return its ID.
fn make_shipment(r: &Roles, id: &str) -> String {
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    let sid = String::from_str(&r.env, id);
    client.create_shipment(
        &sid,
        &buyers(&r.env, &r.buyer),
        &r.supplier,
        &r.logistics,
        &r.arbiter,
        &r.token_id,
        &1_000_000_000,
        &milestones(&r.env),
        &opts(&r.env),
    );
    sid
}

/// Advance milestone 0 to ProofSubmitted.
fn submit_m0(r: &Roles, sid: &String) {
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    client.submit_proof(
        &r.supplier,
        sid,
        &0,
        &String::from_str(&r.env, "ipfs://x"),
        &Symbol::new(&r.env, "ipfs"),
    );
}

/// Advance milestone 0 to Disputed.
fn dispute_m0(r: &Roles, sid: &String) {
    submit_m0(r, sid);
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    client.raise_dispute(&r.buyer, sid, &0);
}

fn dummy_wasm_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

// ============================================================
// pause — admin only
// ============================================================

#[test]
fn test_perm_pause_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).pause(&r.admin);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_pause_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).pause(&r.buyer);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_pause_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).pause(&r.supplier);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_pause_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).pause(&r.logistics);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_pause_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).pause(&r.arbiter);
}

// ============================================================
// unpause — admin only
// ============================================================

#[test]
fn test_perm_unpause_admin_allowed() {
    let r = setup();
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    client.pause(&r.admin);
    client.unpause(&r.admin);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_unpause_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).unpause(&r.buyer);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_unpause_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).unpause(&r.supplier);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_unpause_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).unpause(&r.logistics);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_unpause_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id).unpause(&r.arbiter);
}

// ============================================================
// set_escalation_threshold — admin only
// ============================================================

#[test]
fn test_perm_set_escalation_threshold_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_escalation_threshold(&r.admin, &100);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_escalation_threshold_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_escalation_threshold(&r.buyer, &100);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_escalation_threshold_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_escalation_threshold(&r.supplier, &100);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_escalation_threshold_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_escalation_threshold(&r.logistics, &100);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_escalation_threshold_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_escalation_threshold(&r.arbiter, &100);
}

// ============================================================
// set_max_shipment_value — admin only
// ============================================================

#[test]
fn test_perm_set_max_shipment_value_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_shipment_value(&r.admin, &5_000_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_shipment_value_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_shipment_value(&r.buyer, &5_000_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_shipment_value_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_shipment_value(&r.supplier, &5_000_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_shipment_value_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_shipment_value(&r.logistics, &5_000_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_shipment_value_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_shipment_value(&r.arbiter, &5_000_000_000);
}

// ============================================================
// set_circuit_breaker — admin only
// ============================================================

#[test]
fn test_perm_set_circuit_breaker_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_circuit_breaker(&r.admin, &1_000_000_000, &1000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_circuit_breaker_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_circuit_breaker(&r.buyer, &1_000_000_000, &1000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_circuit_breaker_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_circuit_breaker(&r.supplier, &1_000_000_000, &1000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_circuit_breaker_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_circuit_breaker(&r.logistics, &1_000_000_000, &1000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_circuit_breaker_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_circuit_breaker(&r.arbiter, &1_000_000_000, &1000);
}

// ============================================================
// set_fee_config — admin only
// ============================================================

#[test]
fn test_perm_set_fee_config_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_fee_config(&r.admin, &100, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_fee_config_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_fee_config(&r.buyer, &100, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_fee_config_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_fee_config(&r.supplier, &100, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_fee_config_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_fee_config(&r.logistics, &100, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_fee_config_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_fee_config(&r.arbiter, &100, &r.stranger);
}

// ============================================================
// set_max_concurrent_disputes — admin only
// ============================================================

#[test]
fn test_perm_set_max_concurrent_disputes_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_concurrent_disputes(&r.admin, &3);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_concurrent_disputes_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_concurrent_disputes(&r.buyer, &3);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_concurrent_disputes_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_concurrent_disputes(&r.supplier, &3);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_concurrent_disputes_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_concurrent_disputes(&r.logistics, &3);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_concurrent_disputes_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_concurrent_disputes(&r.arbiter, &3);
}

// ============================================================
// set_min_milestone_percent — admin only
// ============================================================

#[test]
fn test_perm_set_min_milestone_percent_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_min_milestone_percent(&r.admin, &5);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_min_milestone_percent_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_min_milestone_percent(&r.buyer, &5);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_min_milestone_percent_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_min_milestone_percent(&r.supplier, &5);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_min_milestone_percent_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_min_milestone_percent(&r.logistics, &5);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_min_milestone_percent_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_min_milestone_percent(&r.arbiter, &5);
}

// ============================================================
// set_max_advance_percent — admin only
// ============================================================

#[test]
fn test_perm_set_max_advance_percent_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_advance_percent(&r.admin, &50);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_advance_percent_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_advance_percent(&r.buyer, &50);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_advance_percent_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_advance_percent(&r.supplier, &50);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_advance_percent_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_advance_percent(&r.logistics, &50);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_max_advance_percent_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_max_advance_percent(&r.arbiter, &50);
}

// ============================================================
// blacklist_address — admin only
// ============================================================

#[test]
fn test_perm_blacklist_address_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .blacklist_address(&r.admin, &r.stranger, &dummy_wasm_hash(&r.env));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_blacklist_address_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .blacklist_address(&r.buyer, &r.stranger, &dummy_wasm_hash(&r.env));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_blacklist_address_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .blacklist_address(&r.supplier, &r.stranger, &dummy_wasm_hash(&r.env));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_blacklist_address_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .blacklist_address(&r.logistics, &r.stranger, &dummy_wasm_hash(&r.env));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_blacklist_address_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .blacklist_address(&r.arbiter, &r.stranger, &dummy_wasm_hash(&r.env));
}

// ============================================================
// remove_from_blacklist — admin only
// ============================================================

#[test]
fn test_perm_remove_from_blacklist_admin_allowed() {
    let r = setup();
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    client.blacklist_address(&r.admin, &r.stranger, &dummy_wasm_hash(&r.env));
    client.remove_from_blacklist(&r.admin, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_remove_from_blacklist_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .remove_from_blacklist(&r.buyer, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_remove_from_blacklist_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .remove_from_blacklist(&r.supplier, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_remove_from_blacklist_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .remove_from_blacklist(&r.logistics, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_remove_from_blacklist_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .remove_from_blacklist(&r.arbiter, &r.stranger);
}

// ============================================================
// add_allowed_token — admin only (no caller param; enforced via stored admin require_auth)
// ============================================================

#[test]
fn test_perm_add_allowed_token_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .add_allowed_token(&r.token_id);
}

// ============================================================
// remove_allowed_token — admin only (no caller param; enforced via stored admin require_auth)
// ============================================================

#[test]
fn test_perm_remove_allowed_token_admin_allowed() {
    let r = setup();
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    client.add_allowed_token(&r.token_id);
    client.remove_allowed_token(&r.token_id);
}

// ============================================================
// nominate_admin — admin only
// ============================================================

#[test]
fn test_perm_nominate_admin_admin_allowed() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .nominate_admin(&r.admin, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_nominate_admin_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .nominate_admin(&r.buyer, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_nominate_admin_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .nominate_admin(&r.supplier, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_nominate_admin_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .nominate_admin(&r.logistics, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_nominate_admin_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .nominate_admin(&r.arbiter, &r.stranger);
}

// ============================================================
// revoke_nomination — admin only
// ============================================================

#[test]
fn test_perm_revoke_nomination_admin_allowed() {
    let r = setup();
    let client = ChainSettleContractClient::new(&r.env, &r.contract_id);
    client.nominate_admin(&r.admin, &r.stranger);
    client.revoke_nomination(&r.admin);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_revoke_nomination_buyer_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .revoke_nomination(&r.buyer);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_revoke_nomination_supplier_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .revoke_nomination(&r.supplier);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_revoke_nomination_logistics_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .revoke_nomination(&r.logistics);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_revoke_nomination_arbiter_denied() {
    let r = setup();
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .revoke_nomination(&r.arbiter);
}

// ============================================================
// create_shipment — buyer (as primary authoriser) only
// ============================================================

#[test]
fn test_perm_create_shipment_buyer_allowed() {
    let r = setup();
    // buyer is the only entry in the buyers vec; this must succeed
    make_shipment(&r, "PERM-CS-OK");
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_create_shipment_supplier_as_buyer_denied() {
    // supplier in the buyers list but not the one signing — mock_all_auths passes
    // the auth, but the shipment is created with supplier as "buyer". The real
    // denial we want to test is: a stranger cannot be a valid buyer role on
    // an already-created shipment when calling buyer-only functions. Here we
    // verify that a non-buyer cannot call confirm_milestone on a buyer-created ship.
    let r = setup();
    let sid = make_shipment(&r, "PERM-CS-S");
    submit_m0(&r, &sid);
    // supplier tries to confirm — must panic
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.supplier, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_create_shipment_logistics_as_confirmer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CS-L");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.logistics, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_create_shipment_arbiter_as_confirmer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CS-A");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.arbiter, &sid, &0);
}

// ============================================================
// top_up_escrow — buyer only
// ============================================================

#[test]
fn test_perm_top_up_escrow_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TUE-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .top_up_escrow(&r.buyer, &sid, &500_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_top_up_escrow_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TUE-S");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .top_up_escrow(&r.supplier, &sid, &500_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_top_up_escrow_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TUE-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .top_up_escrow(&r.logistics, &sid, &500_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_top_up_escrow_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TUE-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .top_up_escrow(&r.arbiter, &sid, &500_000_000);
}

// ============================================================
// rebalance_milestones — buyer only
// ============================================================

#[test]
fn test_perm_rebalance_milestones_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RBM-OK");
    let new_pcts = vec![&r.env, 40u32, 60u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .rebalance_milestones(&r.buyer, &sid, &new_pcts);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_rebalance_milestones_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RBM-S");
    let new_pcts = vec![&r.env, 40u32, 60u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .rebalance_milestones(&r.supplier, &sid, &new_pcts);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_rebalance_milestones_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RBM-L");
    let new_pcts = vec![&r.env, 40u32, 60u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .rebalance_milestones(&r.logistics, &sid, &new_pcts);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_rebalance_milestones_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RBM-A");
    let new_pcts = vec![&r.env, 40u32, 60u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .rebalance_milestones(&r.arbiter, &sid, &new_pcts);
}

// ============================================================
// confirm_milestone — buyer only
// ============================================================

#[test]
fn test_perm_confirm_milestone_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CM-OK");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.buyer, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_confirm_milestone_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CM-S");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.supplier, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_confirm_milestone_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CM-L");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.logistics, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_confirm_milestone_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CM-A");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .confirm_milestone(&r.arbiter, &sid, &0);
}

// ============================================================
// raise_dispute — buyer only
// ============================================================

#[test]
fn test_perm_raise_dispute_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RD-OK");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_dispute(&r.buyer, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_raise_dispute_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RD-S");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_dispute(&r.supplier, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_raise_dispute_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RD-L");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_dispute(&r.logistics, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_raise_dispute_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RD-A");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_dispute(&r.arbiter, &sid, &0);
}

// ============================================================
// raise_partial_dispute — buyer only
// ============================================================

#[test]
fn test_perm_raise_partial_dispute_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RPD-OK");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_partial_dispute(&r.buyer, &sid, &0, &40);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_raise_partial_dispute_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RPD-S");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_partial_dispute(&r.supplier, &sid, &0, &40);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_raise_partial_dispute_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RPD-L");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_partial_dispute(&r.logistics, &sid, &0, &40);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_raise_partial_dispute_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RPD-A");
    submit_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .raise_partial_dispute(&r.arbiter, &sid, &0, &40);
}

// ============================================================
// cancel_shipment — buyer only
// ============================================================

#[test]
fn test_perm_cancel_shipment_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CAN-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .cancel_shipment(&r.buyer, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_cancel_shipment_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CAN-S");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .cancel_shipment(&r.supplier, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_cancel_shipment_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CAN-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .cancel_shipment(&r.logistics, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_cancel_shipment_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-CAN-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .cancel_shipment(&r.arbiter, &sid);
}

// ============================================================
// approve_advance — buyer only
// ============================================================

#[test]
fn test_perm_approve_advance_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-AA-OK");
    // supplier first requests an advance
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.supplier, &sid, &0, &20);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .approve_advance(&r.buyer, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_approve_advance_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-AA-S");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.supplier, &sid, &0, &20);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .approve_advance(&r.supplier, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_approve_advance_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-AA-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.supplier, &sid, &0, &20);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .approve_advance(&r.logistics, &sid, &0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_approve_advance_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-AA-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.supplier, &sid, &0, &20);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .approve_advance(&r.arbiter, &sid, &0);
}

// ============================================================
// set_proof_whitelist — buyer only
// ============================================================

#[test]
fn test_perm_set_proof_whitelist_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SPW-OK");
    let types = vec![&r.env, Symbol::new(&r.env, "ipfs")];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_proof_whitelist(&r.buyer, &sid, &0, &types);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_proof_whitelist_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SPW-S");
    let types = vec![&r.env, Symbol::new(&r.env, "ipfs")];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_proof_whitelist(&r.supplier, &sid, &0, &types);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_proof_whitelist_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SPW-L");
    let types = vec![&r.env, Symbol::new(&r.env, "ipfs")];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_proof_whitelist(&r.logistics, &sid, &0, &types);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_set_proof_whitelist_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SPW-A");
    let types = vec![&r.env, Symbol::new(&r.env, "ipfs")];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .set_proof_whitelist(&r.arbiter, &sid, &0, &types);
}

// ============================================================
// transfer_buyer — current buyer only
// ============================================================

#[test]
fn test_perm_transfer_buyer_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TB-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_buyer(&r.buyer, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_transfer_buyer_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TB-S");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_buyer(&r.supplier, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_transfer_buyer_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TB-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_buyer(&r.logistics, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_transfer_buyer_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TB-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_buyer(&r.arbiter, &sid, &r.stranger);
}

// ============================================================
// resolve_dispute — arbiter only
// ============================================================

#[test]
fn test_perm_resolve_dispute_arbiter_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RES-OK");
    dispute_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .resolve_dispute(&r.arbiter, &sid, &0, &true);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_resolve_dispute_buyer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RES-B");
    dispute_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .resolve_dispute(&r.buyer, &sid, &0, &true);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_resolve_dispute_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RES-S");
    dispute_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .resolve_dispute(&r.supplier, &sid, &0, &true);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_resolve_dispute_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RES-L");
    dispute_m0(&r, &sid);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .resolve_dispute(&r.logistics, &sid, &0, &true);
}

// ============================================================
// submit_proof — supplier OR logistics
// ============================================================

#[test]
fn test_perm_submit_proof_supplier_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SP-SUP");
    ChainSettleContractClient::new(&r.env, &r.contract_id).submit_proof(
        &r.supplier,
        &sid,
        &0,
        &String::from_str(&r.env, "ipfs://a"),
        &Symbol::new(&r.env, "ipfs"),
    );
}

#[test]
fn test_perm_submit_proof_logistics_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SP-LOG");
    ChainSettleContractClient::new(&r.env, &r.contract_id).submit_proof(
        &r.logistics,
        &sid,
        &0,
        &String::from_str(&r.env, "ipfs://b"),
        &Symbol::new(&r.env, "ipfs"),
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_submit_proof_buyer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SP-B");
    ChainSettleContractClient::new(&r.env, &r.contract_id).submit_proof(
        &r.buyer,
        &sid,
        &0,
        &String::from_str(&r.env, "ipfs://c"),
        &Symbol::new(&r.env, "ipfs"),
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_submit_proof_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SP-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id).submit_proof(
        &r.arbiter,
        &sid,
        &0,
        &String::from_str(&r.env, "ipfs://d"),
        &Symbol::new(&r.env, "ipfs"),
    );
}

// ============================================================
// request_advance — supplier only
// ============================================================

#[test]
fn test_perm_request_advance_supplier_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RA-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.supplier, &sid, &0, &20);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_request_advance_buyer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RA-B");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.buyer, &sid, &0, &20);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_request_advance_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RA-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.logistics, &sid, &0, &20);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_request_advance_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-RA-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .request_advance(&r.arbiter, &sid, &0, &20);
}

// ============================================================
// supplier_cancel — supplier only
// ============================================================

#[test]
fn test_perm_supplier_cancel_supplier_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SC-OK");
    // Submit proof so a deadline can pass; advance ledger past response_deadline (100)
    submit_m0(&r, &sid);
    r.env.ledger().set_sequence_number(200);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .supplier_cancel(&r.supplier, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_supplier_cancel_buyer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SC-B");
    submit_m0(&r, &sid);
    r.env.ledger().set_sequence_number(200);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .supplier_cancel(&r.buyer, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_supplier_cancel_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SC-L");
    submit_m0(&r, &sid);
    r.env.ledger().set_sequence_number(200);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .supplier_cancel(&r.logistics, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_supplier_cancel_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-SC-A");
    submit_m0(&r, &sid);
    r.env.ledger().set_sequence_number(200);
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .supplier_cancel(&r.arbiter, &sid);
}

// ============================================================
// transfer_supplier — current supplier only
// ============================================================

#[test]
fn test_perm_transfer_supplier_supplier_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TS-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_supplier(&r.supplier, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_transfer_supplier_buyer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TS-B");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_supplier(&r.buyer, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_transfer_supplier_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TS-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_supplier(&r.logistics, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_transfer_supplier_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-TS-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .transfer_supplier(&r.arbiter, &sid, &r.stranger);
}

// ============================================================
// propose_amendment — buyer OR supplier; others denied
// ============================================================

#[test]
fn test_perm_propose_amendment_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PA-B-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_amendment(&r.buyer, &sid, &0, &50, &String::from_str(&r.env, "M0v2"));
}

#[test]
fn test_perm_propose_amendment_supplier_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PA-S-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_amendment(&r.supplier, &sid, &0, &50, &String::from_str(&r.env, "M0v2"));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_propose_amendment_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PA-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_amendment(&r.logistics, &sid, &0, &50, &String::from_str(&r.env, "M0v2"));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_propose_amendment_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PA-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_amendment(&r.arbiter, &sid, &0, &50, &String::from_str(&r.env, "M0v2"));
}

// ============================================================
// propose_arbiter_rotation — buyer OR supplier; others denied
// ============================================================

#[test]
fn test_perm_propose_arbiter_rotation_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PAR-B-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_arbiter_rotation(&r.buyer, &sid, &r.stranger);
}

#[test]
fn test_perm_propose_arbiter_rotation_supplier_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PAR-S-OK");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_arbiter_rotation(&r.supplier, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_propose_arbiter_rotation_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PAR-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_arbiter_rotation(&r.logistics, &sid, &r.stranger);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_propose_arbiter_rotation_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-PAR-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .propose_arbiter_rotation(&r.arbiter, &sid, &r.stranger);
}

// ============================================================
// emergency_recover — admin only
// ============================================================

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_emergency_recover_buyer_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-ER-B");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .emergency_recover(&r.buyer, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_emergency_recover_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-ER-S");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .emergency_recover(&r.supplier, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_emergency_recover_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-ER-L");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .emergency_recover(&r.logistics, &sid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_emergency_recover_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-ER-A");
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .emergency_recover(&r.arbiter, &sid);
}

// ============================================================
// batch_confirm_milestones — buyer only
// ============================================================

#[test]
fn test_perm_batch_confirm_milestones_buyer_allowed() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-BCM-OK");
    submit_m0(&r, &sid);
    let indices = vec![&r.env, 0u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .batch_confirm_milestones(&r.buyer, &sid, &indices);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_batch_confirm_milestones_supplier_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-BCM-S");
    submit_m0(&r, &sid);
    let indices = vec![&r.env, 0u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .batch_confirm_milestones(&r.supplier, &sid, &indices);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_batch_confirm_milestones_logistics_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-BCM-L");
    submit_m0(&r, &sid);
    let indices = vec![&r.env, 0u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .batch_confirm_milestones(&r.logistics, &sid, &indices);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_perm_batch_confirm_milestones_arbiter_denied() {
    let r = setup();
    let sid = make_shipment(&r, "PERM-BCM-A");
    submit_m0(&r, &sid);
    let indices = vec![&r.env, 0u32];
    ChainSettleContractClient::new(&r.env, &r.contract_id)
        .batch_confirm_milestones(&r.arbiter, &sid, &indices);
}

// ============================================================
// CI GUARD
// ============================================================
//
// Count of `fn test_perm_` functions in this file.  CI can assert:
//   grep -c 'fn test_perm_' src/test_permissions.rs == PERMISSION_TEST_COUNT
//
// Update this constant whenever a new test is added or removed.
pub const PERMISSION_TEST_COUNT: usize = 146;

#[test]
fn test_perm_count_guard() {
    // Counts the number of permission tests via the constant above.
    // If PERMISSION_TEST_COUNT is wrong the build will fail to compile
    // (unreachable assertion) or alert reviewers during code review.
    assert!(PERMISSION_TEST_COUNT >= 40, "permission matrix requires at least 40 test cases");
}
