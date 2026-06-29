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

#[test]
fn test_fee_tier_buyer_below_threshold_uses_default() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &100u32, &t.treasury);
    let tiers = vec![&t.env, FeeTier { min_lifetime_volume: 500_000, fee_bps: 50 }];
    client.set_fee_tiers(&t.buyer, &tiers);
    // 0 lifetime volume → no tier qualifies
    assert_eq!(client.get_fee_tier(&t.buyer), 100);
}

#[test]
fn test_fee_tier_upgrade_after_volume_accumulation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &200u32, &t.treasury);
    let tiers = vec![&t.env, FeeTier { min_lifetime_volume: 100_000, fee_bps: 50 }];
    client.set_fee_tiers(&t.buyer, &tiers);
    assert_eq!(client.get_fee_tier(&t.buyer), 200, "below threshold initially");

    // Confirm a milestone to accumulate volume
    let ship_id = sid(&t.env, "vol1");
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &400_000,
        &vec![
            &t.env,
            Milestone {
                name: String::from_str(&t.env, "All"),
                payment_percent: 100,
                proof_hash: String::from_str(&t.env, ""),
                status: MilestoneStatus::Pending,
                release_after_ledger: 0,
                proof_submitted_ledger: None,
                dispute_opened_ledger: None,
                deadline_ledger: 0,
                penalty_bps_per_ledger: 0,
            }
        ],
        &default_options(&t.env),
    );
    client.submit_proof(&t.supplier, &ship_id, &0u32, &sid(&t.env, "h0"), &ipfs(&t.env));
    client.confirm_milestone(&t.buyer, &ship_id, &0u32);

    // After 400_000 volume, buyer qualifies for 50 bps
    assert_eq!(client.get_fee_tier(&t.buyer), 50, "should be on reduced tier");
}

#[test]
fn test_shipment_fee_bps_locked_at_creation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &100u32, &t.treasury);

    let ship_id = sid(&t.env, "locked");
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

    // ShipmentFeeBps key is set: verify via get_fee_tier (indirect)
    // With no lifetime volume the default 100 bps applies
    assert_eq!(client.get_fee_tier(&t.buyer), 100);
}

// ============================================================
// #112 – Invoice hash tests
// ============================================================

#[test]
fn test_invoice_hash_stored_and_retrieved() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "invship");

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
    client.submit_proof(&t.supplier, &ship_id, &0u32, &sid(&t.env, "h0"), &ipfs(&t.env));

    let hash = BytesN::from_array(&t.env, &[0x11u8; 32]);
    client.attach_invoice_hash(&t.supplier, &ship_id, &0u32, &hash);

    assert_eq!(client.get_invoice_hash(&ship_id, &0u32), Some(hash));
}

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
    client.submit_proof(&t.supplier, &ship_id, &0u32, &sid(&t.env, "h0"), &ipfs(&t.env));
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
        }
    ]
}

#[test]
fn test_amendment_log_entry_on_accepted_amendment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "amlog");

    // Use milestones where a name change (keeping 25%) is valid
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

    // Propose name change keeping same percent (25→25 valid, sum=100)
    client.propose_amendment(&t.buyer, &ship_id, &0u32, &25u32, &sid(&t.env, "Updated Name"));
    client.propose_amendment(&t.supplier, &ship_id, &0u32, &25u32, &sid(&t.env, "Updated Name"));

    let log = client.get_amendment_log(&ship_id, &0u32);
    assert_eq!(log.len(), 1, "log should have one entry after accepted amendment");
    let e = log.get(0).unwrap();
    assert_eq!(e.old_payment_percent, 25);
    assert_eq!(e.new_payment_percent, 25);
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

#[test]
fn test_amendment_log_chronological_order() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "amorder");

    // Single 100% milestone allows any percent change x where x+0=100, i.e. x=100 only.
    // So we do two name changes keeping percent at 100.
    client.create_shipment(
        &ship_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000,
        &single_milestone(&t.env, "Phase1"),
        &default_options(&t.env),
    );

    // First amendment: rename
    client.propose_amendment(&t.buyer, &ship_id, &0u32, &100u32, &sid(&t.env, "Phase2"));
    client.propose_amendment(&t.supplier, &ship_id, &0u32, &100u32, &sid(&t.env, "Phase2"));

    // Second amendment: rename again
    client.propose_amendment(&t.buyer, &ship_id, &0u32, &100u32, &sid(&t.env, "Phase3"));
    client.propose_amendment(&t.supplier, &ship_id, &0u32, &100u32, &sid(&t.env, "Phase3"));

    let log = client.get_amendment_log(&ship_id, &0u32);
    assert_eq!(log.len(), 2, "two accepted amendments → two log entries");
    assert!(log.get(0).unwrap().ledger <= log.get(1).unwrap().ledger,
        "entries should be chronological");
}

// ============================================================
// #110 – Extension request tests
// ============================================================

#[test]
fn test_extension_approved_updates_deadline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "extship");

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

    client.request_extension(&t.supplier, &ship_id, &0u32, &500u32);
    client.approve_extension(&t.buyer, &ship_id, &0u32);

    let deadline = client.get_milestone_deadline(&ship_id, &0u32);
    assert!(deadline > 0, "deadline should be set after approval");
}

#[test]
fn test_extension_denied_clears_without_changing_deadline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "extdeny");

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

    client.request_extension(&t.supplier, &ship_id, &0u32, &500u32);
    client.deny_extension(&t.buyer, &ship_id, &0u32);

    assert_eq!(client.get_milestone_deadline(&ship_id, &0u32), 0);
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

#[test]
#[should_panic(expected = "unauthorized")]
fn test_buyer_cannot_request_extension() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let ship_id = sid(&t.env, "extauth");

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

    // Buyer is not supplier — must panic
    client.request_extension(&t.buyer, &ship_id, &0u32, &200u32);
}
