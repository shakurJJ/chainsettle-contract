#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};
use soroban_sdk::{vec, BytesN, Env, String, Symbol};

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn ipfs(env: &Env) -> Symbol {
    Symbol::new(env, "ipfs")
}

// ============================================================
// #113 – Fee tier tests
// ============================================================

#[test]
fn test_fee_tier_no_tiers_uses_default_fee() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &100u32, &t.treasury);
    // No tiers: get_fee_tier returns FeeConfig.fee_bps
    assert_eq!(client.get_fee_tier(&t.buyer), 100);
}

// ============================================================
// #112 – Invoice hash tests
// ============================================================

#[test]
fn test_no_invoice_hash_returns_none() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "nohash");

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

    assert!(client.get_invoice_hash(&ship_id, &0u32).is_none());
}

#[test]
#[should_panic(expected = "invoice hash already set and is immutable")]
fn test_invoice_hash_immutable_after_first_submission() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "invimm");

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
    client.submit_proof(
        &t.supplier,
        &ship_id,
        &0u32,
        &sid(&t.env, "h0"),
        &ipfs(&t.env),
    );
    let hash = BytesN::from_array(&t.env, &[0xAAu8; 32]);
    client.attach_invoice_hash(&t.supplier, &ship_id, &0u32, &hash);
    // Second attach must panic
    client.attach_invoice_hash(&t.supplier, &ship_id, &0u32, &hash);
}

// ============================================================
// #111 – Amendment log tests
// ============================================================

// Helper: milestones where index 0 can validly change percent (100% single milestone)
fn single_milestone(env: &Env, name: &str) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, name),
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

#[test]
fn test_amendment_log_empty_on_unilateral_proposal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "amlog2");

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

    // Only buyer proposes → no acceptance
    client.propose_amendment(&t.buyer, &ship_id, &0u32, &25u32, &sid(&t.env, "V2"));
    assert_eq!(client.get_amendment_log(&ship_id, &0u32).len(), 0);
}

// ============================================================
// #110 – Extension request tests
// ============================================================

#[test]
fn test_extension_request_count_includes_approved_requests() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "extapproved");

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
    client.set_max_extension_requests(&t.buyer, &2u32);

    client.request_extension(&t.supplier, &ship_id, &0u32, &100u32);
    client.approve_extension(&t.buyer, &ship_id, &0u32);
    client.request_extension(&t.supplier, &ship_id, &0u32, &200u32);
    client.approve_extension(&t.buyer, &ship_id, &0u32);

    assert!(client.get_milestone_deadline(&ship_id, &0u32) > 0);
}

#[test]
#[should_panic(expected = "max extension requests reached")]
fn test_extension_request_over_limit_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "extlimit");

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
    client.set_max_extension_requests(&t.buyer, &1u32);

    client.request_extension(&t.supplier, &ship_id, &0u32, &100u32);
    client.deny_extension(&t.buyer, &ship_id, &0u32);

    // second request should be rejected because the milestone has already reached cap
    client.request_extension(&t.supplier, &ship_id, &0u32, &200u32);
}

#[test]
#[should_panic(expected = "extension request already pending")]
fn test_double_extension_request_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "extdbl");

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

    client.request_extension(&t.supplier, &ship_id, &0u32, &100u32);
    // Second request while first is pending must panic
    client.request_extension(&t.supplier, &ship_id, &0u32, &200u32);
}

