#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, Symbol},
    token, Address, BytesN, String, vec,
};
use crate::test_common::*;

// ============================================================
// ADMIN CONTROL TESTS: PAUSE/UNPAUSE
// ============================================================

#[test]
fn test_pause_blocks_state_changes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Pause the contract (buyer is admin in setup).
    client.pause(&t.buyer);
    assert!(client.is_paused());
}

#[test]
#[should_panic(expected = "contract is paused")]
fn test_pause_blocks_create_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.pause(&t.buyer);

    create_standard_shipment(
        &client,
        &t.env,
        &String::from_str(&t.env, "SHIP-PAUSED"),
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        1_000_000_000,
    );
}

#[test]
fn test_unpause_restores_operations() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.pause(&t.buyer);
    assert!(client.is_paused());

    client.unpause(&t.buyer);
    assert!(!client.is_paused());

    // Should succeed after unpause.
    let shipment_id = String::from_str(&t.env, "SHIP-UNPAUSED");
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
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Active
    );
}

#[test]
#[should_panic(expected = "contract is paused")]
fn test_pause_blocks_confirm_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-PAUSE-CONF");
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

    client.pause(&t.buyer);
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
}

#[test]
fn test_read_only_accessible_while_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-READ-PAUSED");
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

    client.pause(&t.buyer);

    // Read-only calls must still work.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);

    let milestone = client.get_milestone(&shipment_id, &0);
    assert_eq!(milestone.status, MilestoneStatus::Pending);

    let balance = client.get_escrow_balance(&shipment_id);
    assert_eq!(balance, 1_000_000_000);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_pause() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    // supplier is not admin
    client.pause(&t.supplier);
}

// ============================================================
// ADMIN CONTROL TESTS: MIN MILESTONE PERCENT & SETTINGS
// ============================================================

#[test]
fn test_min_milestone_percent_accepts_threshold() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_min_milestone_percent(&t.buyer, &5);

    let shipment_id = String::from_str(&t.env, "SHIP-MIN-OK");
    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Goods Dispatched"),
            payment_percent: 5,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "In Transit"),
            payment_percent: 45,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Delivered"),
            payment_percent: 50,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ];

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &milestones,
        &default_options(&t.env),
    );

    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Active
    );
}

#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_min_milestone_percent_rejects_below_threshold() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MIN-BAD");
    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Goods Dispatched"),
            payment_percent: 4,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "In Transit"),
            payment_percent: 46,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Delivered"),
            payment_percent: 50,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ];

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &milestones,
        &default_options(&t.env),
    );
}

#[test]
fn test_min_milestone_percent_updates_via_admin() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.set_min_milestone_percent(&t.buyer, &10);
    assert_eq!(client.get_admin_log().len() >= 1, true);

    let shipment_id = String::from_str(&t.env, "SHIP-MIN-UPDATE");
    let milestones = vec![
        &t.env,
        Milestone {
            name: String::from_str(&t.env, "Goods Dispatched"),
            payment_percent: 10,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "In Transit"),
            payment_percent: 40,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Delivered"),
            payment_percent: 50,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ];

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &milestones,
        &default_options(&t.env),
    );
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Active
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_update_min_milestone_percent() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_min_milestone_percent(&t.supplier, &7);
}

#[test]
fn test_admin_action_log_capped_and_ordered() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    for i in 1..=51 {
        let pct = if i <= 100 { i as u32 } else { 100 };
        t.env.ledger().with_mut(|l| l.sequence_number += 1);
        client.set_min_milestone_percent(&t.buyer, &pct);
    }

    let log = client.get_admin_log();
    assert_eq!(log.len(), 50);
    for i in 1..log.len() {
        assert!(log.get(i - 1).unwrap().ledger <= log.get(i).unwrap().ledger);
    }
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_upgrade_non_admin_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let fake_hash = BytesN::from_array(&t.env, &[0u8; 32]);
    client.upgrade(&t.supplier, &fake_hash);
}

// ============================================================
// ADMIN CONTROL TESTS: BLACKLIST
// ============================================================

#[test]
#[should_panic(expected = "unauthorized")]
fn test_blacklisted_buyer_blocks_create_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let reason = BytesN::from_array(&t.env, &[0u8; 32]);
    client.blacklist_address(&t.buyer, &t.buyer, &reason);

    let shipment_id = String::from_str(&t.env, "SHIP-BLACK-BUYER");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn test_blacklist_removal_restores_participation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let reason = BytesN::from_array(&t.env, &[1u8; 32]);
    client.blacklist_address(&t.buyer, &t.supplier, &reason);
    assert!(client.is_blacklisted(&t.supplier));

    client.remove_from_blacklist(&t.buyer, &t.supplier);
    assert!(!client.is_blacklisted(&t.supplier));

    let shipment_id = String::from_str(&t.env, "SHIP-BLACK-RESTORED");
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
    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Active
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_blacklisted_arbiter_blocks_create_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let reason = BytesN::from_array(&t.env, &[2u8; 32]);
    client.blacklist_address(&t.buyer, &t.arbiter, &reason);

    let shipment_id = String::from_str(&t.env, "SHIP-BLACK-ARBITER");
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn test_existing_shipment_works_after_post_creation_blacklist() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, "SHIP-BLACK-EXISTING");
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
    let reason = BytesN::from_array(&t.env, &[3u8; 32]);
    client.blacklist_address(&t.buyer, &t.supplier, &reason);

    assert_eq!(
        client.get_shipment(&shipment_id).status,
        ShipmentStatus::Active
    );
    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &String::from_str(&t.env, "ipfs://d"),
        &Symbol::new(&t.env, "ipfs"),
    );
    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::ProofSubmitted
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_top_up_non_buyer_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-TOPUP-AUTH");

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

    // Supplier is not a buyer — must be rejected.
    client.top_up_escrow(&t.supplier, &shipment_id, &100_000);
}
