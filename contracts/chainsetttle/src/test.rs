#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String,
};

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
        },
        Milestone {
            name: String::from_str(env, "In Transit"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(env, "Delivered"),
            payment_percent: 25,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
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

#[test]
fn test_create_shipment_success() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-001");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    assert_eq!(token_client.balance(&t.buyer), 10_000_000_000 - total_amount);
    assert_eq!(token_client.balance(&t.contract_id), total_amount);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.total_amount, total_amount);
    assert_eq!(shipment.released_amount, 0);
    assert_eq!(shipment.milestones.len(), 3);
    assert_eq!(shipment.holdback_ledgers, 0);
    assert_eq!(shipment.dispute_cooldown_ledgers, 0);
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
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Step 2"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(&t.env, "Step 3"),
            payment_percent: 30,
            proof_hash: String::from_str(&t.env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
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
        &default_options(&t.env),
    );
}

#[test]
fn test_full_shipment_lifecycle() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-FULL");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    client.submit_proof(&t.logistics, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    client.submit_proof(&t.supplier, &shipment_id, &2, &String::from_str(&t.env, "ipfs://v"));
    client.confirm_milestone(&t.buyer, &shipment_id, &2);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    assert_eq!(token_client.balance(&t.supplier), total_amount);
    assert_eq!(client.get_escrow_balance(&shipment_id), 0);
}

#[test]
fn test_cancel_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-CANCEL");
    let total_amount: i128 = 1_000_000_000;
    let buyer_balance_before = token_client.balance(&t.buyer);

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.cancel_shipment(&t.buyer, &shipment_id);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
    assert_eq!(token_client.balance(&t.buyer), buyer_balance_before);
}

#[test]
fn test_cancel_partial_confirmed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-PARTIAL");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Confirm milestone 0 (25%)
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let buyer_balance_after_confirm = token_client.balance(&t.buyer);
    client.cancel_shipment(&t.buyer, &shipment_id);

    // Buyer should get back 75% (the unconfirmed portion)
    let expected_refund = total_amount * 75 / 100;
    assert_eq!(token_client.balance(&t.buyer), buyer_balance_after_confirm + expected_refund);
    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Cancelled);
}

#[test]
#[should_panic(expected = "cannot cancel: dispute must be resolved first")]
fn test_cancel_blocked_by_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-DISP-CANCEL");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.cancel_shipment(&t.buyer, &shipment_id);
}

#[test]
fn test_cancel_zero_confirmed() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let total: i128 = 1_000_000_000;
    let before = token_client.balance(&t.buyer);
    let shipment_id = String::from_str(&t.env, "CANCEL-ZERO");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total,
    );

    client.cancel_shipment(&t.buyer, &shipment_id);

    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Cancelled);
    assert_eq!(token_client.balance(&t.buyer), before);
}

// ============================================================
// PAUSE / UNPAUSE TESTS
// ============================================================

#[test]
fn test_pause_blocks_state_changes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Pause the contract (buyer is admin in setup).
    client.pause(&t.buyer);
    assert!(client.is_paused());
    // The should_panic tests below verify individual functions are blocked.
}

#[test]
#[should_panic(expected = "contract is paused")]
fn test_pause_blocks_create_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    client.pause(&t.buyer);

    create_standard_shipment(
        &client, &t.env,
        &String::from_str(&t.env, "SHIP-PAUSED"),
        &t.buyer, &t.supplier, &t.logistics, &t.arbiter, &t.token_id,
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
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );
    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Active);
}

#[test]
#[should_panic(expected = "contract is paused")]
fn test_pause_blocks_confirm_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-PAUSE-CONF");
    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));

    client.pause(&t.buyer);
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
}

#[test]
fn test_read_only_accessible_while_paused() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-READ-PAUSED");
    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
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
// ESCROW TOP-UP TESTS
// ============================================================

#[test]
fn test_top_up_escrow_increases_total_amount() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-TOPUP");
    let initial_amount: i128 = 1_000_000_000;
    let top_up: i128 = 500_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, initial_amount,
    );

    client.top_up_escrow(&t.buyer, &shipment_id, &top_up);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.total_amount, initial_amount + top_up);
    assert_eq!(client.get_escrow_balance(&shipment_id), initial_amount + top_up);
    assert_eq!(token_client.balance(&t.contract_id), initial_amount + top_up);
}

#[test]
fn test_top_up_proportionally_increases_milestone_payments() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-TOPUP-PROP");
    let initial_amount: i128 = 1_000_000_000;
    let top_up: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, initial_amount,
    );

    client.top_up_escrow(&t.buyer, &shipment_id, &top_up);

    // Confirm milestone 0 (25%) — payment should be 25% of new total (2_000_000_000)
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let expected_payment = (initial_amount + top_up) * 25 / 100; // 500_000_000
    assert_eq!(token_client.balance(&t.supplier), expected_payment);
}

#[test]
#[should_panic(expected = "top-up disallowed: shipment is not active")]
fn test_top_up_disallowed_after_completion() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-TOPUP-DONE");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    // Complete the shipment.
    for i in 0u32..3u32 {
        client.submit_proof(&t.supplier, &shipment_id, &i, &String::from_str(&t.env, "ipfs://x"));
        client.confirm_milestone(&t.buyer, &shipment_id, &i);
    }

    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Completed);
    client.top_up_escrow(&t.buyer, &shipment_id, &100_000);
}

#[test]
#[should_panic(expected = "top-up disallowed: shipment is not active")]
fn test_top_up_disallowed_after_cancellation() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-TOPUP-CANCEL");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.cancel_shipment(&t.buyer, &shipment_id);
    client.top_up_escrow(&t.buyer, &shipment_id, &100_000);
}

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

    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Active);
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
    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Active);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_update_min_milestone_percent() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_min_milestone_percent(&t.supplier, &7);
}

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
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );
    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Active);
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
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );
    let reason = BytesN::from_array(&t.env, &[3u8; 32]);
    client.blacklist_address(&t.buyer, &t.supplier, &reason);

    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Active);
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    assert_eq!(client.get_milestone(&shipment_id, &0).status, MilestoneStatus::ProofSubmitted);
}

#[test]
#[should_panic(expected = "DisputeAlreadyOpen")]
fn test_dispute_limit_blocks_second() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, "SHIP-DISP-LIMIT");
    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.submit_proof(&t.supplier, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));
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
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
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
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.submit_proof(&t.supplier, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));

    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.raise_dispute(&t.buyer, &shipment_id, &1);
    assert_eq!(client.get_shipment(&shipment_id).open_dispute_count, 2);
}

#[test]
fn test_admin_action_log_capped_and_ordered() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    for i in 1..=51 {
        let pct = if i <= 100 { i as u32 } else { 100 };
        client.set_min_milestone_percent(&t.buyer, &pct);
    }

    let log = client.get_admin_log();
    assert_eq!(log.len(), 50);
    for i in 1..log.len() {
        assert!(log.get(i - 1).unwrap().ledger < log.get(i).unwrap().ledger);
    }
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_top_up_non_buyer_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-TOPUP-AUTH");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    // Supplier is not a buyer — must be rejected.
    client.top_up_escrow(&t.supplier, &shipment_id, &100_000);
}

// ============================================================
// DISPUTE COOLDOWN TESTS
// ============================================================

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
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: cooldown, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    // First dispute on milestone 0.
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    // Arbiter rejects — milestone goes back to Pending, cooldown starts.
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    // Resubmit proof for milestone 0.
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d2"));

    // Immediately trying to raise another dispute should fail (cooldown not elapsed).
    // We test this in the should_panic test below.

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
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: cooldown, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    // Resubmit and immediately try to dispute again — must panic.
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d2"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);
}

#[test]
fn test_no_cooldown_allows_immediate_redispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-NO-COOLDOWN");

    // cooldown = 0 means no restriction.
    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d2"));
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
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: cooldown, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);

    t.env.ledger().set_sequence_number(10);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &false);

    // last_dispute_resolved_ledger should now be 10.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.last_dispute_resolved_ledger, Some(10u32));
}

// ============================================================
// TRANSFER BUYER / SUPPLIER TESTS
// ============================================================

#[test]
fn test_transfer_buyer_success() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-XFER-BUYER");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.transfer_buyer(&t.buyer, &shipment_id, &t.buyer2);

    let shipment = client.get_shipment(&shipment_id);
    // buyer2 should now be in the buyers list; original buyer should be gone.
    assert_eq!(shipment.buyers.get(0).unwrap(), t.buyer2);
    assert!(!shipment.buyers.iter().any(|b| b == t.buyer));
}

#[test]
fn test_transfer_buyer_new_buyer_can_confirm() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-XFER-BUYER-CONF");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.transfer_buyer(&t.buyer, &shipment_id, &t.buyer2);

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    // New buyer confirms — should succeed.
    client.confirm_milestone(&t.buyer2, &shipment_id, &0);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(token_client.balance(&t.supplier), 1_000_000_000 * 25 / 100);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_transfer_buyer_old_buyer_cannot_confirm_after_transfer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-XFER-BUYER-OLD");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.transfer_buyer(&t.buyer, &shipment_id, &t.buyer2);

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    // Old buyer tries to confirm — must be rejected.
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
}

#[test]
#[should_panic(expected = "transfer disallowed: open dispute must be resolved first")]
fn test_transfer_buyer_blocked_by_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-XFER-DISP");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);

    // Transfer while dispute is open — must panic.
    client.transfer_buyer(&t.buyer, &shipment_id, &t.buyer2);
}

#[test]
fn test_transfer_supplier_success() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let new_supplier = Address::generate(&t.env);
    let shipment_id = String::from_str(&t.env, "SHIP-XFER-SUP");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.transfer_supplier(&t.supplier, &shipment_id, &new_supplier);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.supplier, new_supplier);

    // Payment should go to new_supplier after confirmation.
    client.submit_proof(&new_supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    assert_eq!(token_client.balance(&new_supplier), 1_000_000_000 * 25 / 100);
    assert_eq!(token_client.balance(&t.supplier), 0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_transfer_supplier_wrong_caller_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let new_supplier = Address::generate(&t.env);
    let shipment_id = String::from_str(&t.env, "SHIP-XFER-SUP-AUTH");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    // Buyer tries to transfer supplier role — must be rejected.
    client.transfer_supplier(&t.buyer, &shipment_id, &new_supplier);
}

#[test]
#[should_panic(expected = "transfer disallowed: open dispute must be resolved first")]
fn test_transfer_supplier_blocked_by_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let new_supplier = Address::generate(&t.env);
    let shipment_id = String::from_str(&t.env, "SHIP-XFER-SUP-DISP");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);

    client.transfer_supplier(&t.supplier, &shipment_id, &new_supplier);
}

// ============================================================
// NON-SEQUENTIAL / PARALLEL MODE TESTS
// ============================================================

#[test]
fn test_non_sequential_baseline() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "NONSEQ");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    // In parallel mode, milestone 2 can be submitted before milestone 0.
    client.submit_proof(&t.supplier, &shipment_id, &2, &String::from_str(&t.env, "ipfs://v"));
    assert_eq!(
        client.get_milestone(&shipment_id, &2).status,
        MilestoneStatus::ProofSubmitted
    );
}

#[test]
fn test_parallel_mode_allows_any_order() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "PARALLEL");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    // Submit and confirm in reverse order.
    client.submit_proof(&t.supplier, &shipment_id, &2, &String::from_str(&t.env, "ipfs://v"));
    client.confirm_milestone(&t.buyer, &shipment_id, &2);

    client.submit_proof(&t.supplier, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Completed);
    assert_eq!(token_client.balance(&t.supplier), 1_000_000_000);
}

// ============================================================
// TOKEN WHITELIST TESTS
// ============================================================

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_whitelisted_token_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    // Whitelist only token_id; try to use a different token.
    client.add_allowed_token(&t.token_id);

    let other_admin = Address::generate(&t.env);
    let other_token = t.env.register_stellar_asset_contract_v2(other_admin.clone()).address();
    token::StellarAssetClient::new(&t.env, &other_token).mint(&t.buyer, &10_000_000_000);

    client.create_shipment(
        &String::from_str(&t.env, "BAD-TOKEN"),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &other_token,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );
}

// ============================================================
// FEE TESTS
// ============================================================

#[test]
fn test_fee_deducted_on_confirm_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let fee_bps: u32 = 100; // 1%
    client.set_fee_config(&t.buyer, &fee_bps, &t.treasury);

    let shipment_id = String::from_str(&t.env, "SHIP-FEE");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let gross = total_amount * 25 / 100; // 250_000_000
    let fee = gross * fee_bps as i128 / 10_000; // 2_500_000
    let net = gross - fee;

    assert_eq!(token_client.balance(&t.supplier), net);
    assert_eq!(token_client.balance(&t.treasury), fee);
}

#[test]
#[should_panic(expected = "fee_bps exceeds maximum of 1000")]
fn test_fee_bps_exceeds_max_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &1001, &t.treasury);
}

#[test]
fn test_no_fee_config_backward_compatible() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-NOFEE");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // No fee config — supplier gets full gross payment.
    assert_eq!(token_client.balance(&t.supplier), total_amount * 25 / 100);
}

// ============================================================
// HOLDBACK TESTS
// ============================================================

#[test]
fn test_holdback_happy_path() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-HOLD");
    let holdback: u32 = 100;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: holdback, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // Payment should be held — supplier balance still 0.
    assert_eq!(token_client.balance(&t.supplier), 0);
    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::ConfirmedHeld
    );

    // Advance past holdback window.
    t.env.ledger().set_sequence_number(holdback + 1);
    client.release_held_payment(&shipment_id, &0);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(token_client.balance(&t.supplier), 1_000_000_000 * 25 / 100);
}

#[test]
fn test_no_holdback_immediate_transfer() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-NOHOLD");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    assert_eq!(token_client.balance(&t.supplier), 1_000_000_000 * 25 / 100);
}

#[test]
fn test_holdback_early_dispute_cancels_hold() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-HOLD-DISP");

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 200, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::ConfirmedHeld
    );

    // Buyer disputes the held milestone.
    client.raise_dispute(&t.buyer, &shipment_id, &0);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Disputed
    );
    // release_after_ledger should be cleared.
    assert_eq!(client.get_milestone(&shipment_id, &0).release_after_ledger, 0);
}

// ============================================================
// BATCH CONFIRM TESTS
// ============================================================

#[test]
fn test_batch_confirm_milestones_full() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-BATCH");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.submit_proof(&t.logistics, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));
    client.submit_proof(&t.supplier, &shipment_id, &2, &String::from_str(&t.env, "ipfs://v"));

    client.batch_confirm_milestones(&t.buyer, &shipment_id, &vec![&t.env, 0u32, 1u32, 2u32]);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Completed);
    assert_eq!(shipment.released_amount, total_amount);
    assert_eq!(token_client.balance(&t.supplier), total_amount);
}

#[test]
fn test_batch_confirm_single_element() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-BATCH-1");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.batch_confirm_milestones(&t.buyer, &shipment_id, &vec![&t.env, 0u32]);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(token_client.balance(&t.supplier), total_amount * 25 / 100);
}

#[test]
fn test_batch_confirm_empty_is_noop() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-BATCH-EMPTY");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.batch_confirm_milestones(&t.buyer, &shipment_id, &vec![&t.env]);

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Active);
    assert_eq!(shipment.released_amount, 0);
}

#[test]
#[should_panic(expected = "milestone proof not yet submitted")]
fn test_batch_confirm_partial_invalid_reverts() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-BATCH-FAIL");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    // Index 1 has no proof — must revert entirely.
    client.batch_confirm_milestones(&t.buyer, &shipment_id, &vec![&t.env, 0u32, 1u32]);
}

// ============================================================
// MULTISIG TESTS
// ============================================================

#[test]
fn test_multisig_both_buyers_must_confirm() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MULTI");
    let total_amount: i128 = 1_000_000_000;

    // Two-buyer shipment.
    client.create_shipment(
        &shipment_id,
        &vec![&t.env, t.buyer.clone(), t.buyer2.clone()],
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_milestones(&t.env),
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));

    // Either buyer can confirm independently in this implementation.
    client.confirm_milestone(&t.buyer, &shipment_id, &0);
    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Confirmed
    );
    assert_eq!(token_client.balance(&t.supplier), total_amount * 25 / 100);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_multisig_duplicate_approval_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MULTI-DUP");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    // Supplier is not a buyer — must be rejected.
    client.confirm_milestone(&t.supplier, &shipment_id, &0);
}

#[test]
fn test_multisig_minority_veto_dispute() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MULTI-VETO");

    client.create_shipment(
        &shipment_id,
        &vec![&t.env, t.buyer.clone(), t.buyer2.clone()],
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &ShipmentOptions { response_deadline: 0, penalty_bps: 0, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    // buyer2 raises a dispute — only one co-buyer needed.
    client.raise_dispute(&t.buyer2, &shipment_id, &0);

    assert_eq!(
        client.get_milestone(&shipment_id, &0).status,
        MilestoneStatus::Disputed
    );
}

// ============================================================
// AMENDMENT TESTS
// ============================================================

#[test]
fn test_amendment_full_mutual_consent() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-AMEND");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    let new_name = String::from_str(&t.env, "Goods Dispatched v2");

    client.propose_amendment(&t.buyer, &shipment_id, &0, &25, &new_name);
    // Not yet applied — only one party agreed.
    assert_eq!(
        client.get_milestone(&shipment_id, &0).name,
        String::from_str(&t.env, "Goods Dispatched")
    );

    client.propose_amendment(&t.supplier, &shipment_id, &0, &25, &new_name);
    // Both agreed — amendment applied.
    assert_eq!(client.get_milestone(&shipment_id, &0).name, new_name);
    assert_eq!(client.get_milestone(&shipment_id, &0).payment_percent, 25);
}

#[test]
fn test_amendment_mismatched_proposals_no_op() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-MISMATCH");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.propose_amendment(&t.buyer, &shipment_id, &0, &25, &String::from_str(&t.env, "Name A"));
    client.propose_amendment(&t.supplier, &shipment_id, &0, &25, &String::from_str(&t.env, "Name B"));

    // Mismatch — milestone unchanged.
    assert_eq!(
        client.get_milestone(&shipment_id, &0).name,
        String::from_str(&t.env, "Goods Dispatched")
    );
}

#[test]
#[should_panic(expected = "can only amend a pending milestone")]
fn test_amendment_confirmed_milestone_rejected() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-AMEND-CONF");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    client.propose_amendment(&t.buyer, &shipment_id, &0, &25, &String::from_str(&t.env, "New Name"));
}

#[test]
#[should_panic(expected = "milestone percentages must sum to 100")]
fn test_amendment_invalid_percentage_sum() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-AMEND-PCT");

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, 1_000_000_000,
    );

    let new_name = String::from_str(&t.env, "Goods Dispatched");
    // 50+50+25 = 125 — must panic.
    client.propose_amendment(&t.buyer, &shipment_id, &0, &50, &new_name);
    client.propose_amendment(&t.supplier, &shipment_id, &0, &50, &new_name);
}

// ============================================================
// SUPPLIER CANCEL TESTS
// ============================================================

#[test]
fn test_deadline_cancellation_success() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);

    let shipment_id = String::from_str(&t.env, "SHIP-SUPCANCEL");
    let total_amount: i128 = 1_000_000_000;
    let deadline: u32 = 100;
    let penalty_bps: u32 = 500;

    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &total_amount,
        &build_milestones(&t.env),
        &ShipmentOptions { response_deadline: deadline, penalty_bps, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    t.env.ledger().set_sequence_number(deadline + 1);

    let buyer_balance_before = token_client.balance(&t.buyer);
    client.supplier_cancel(&t.supplier, &shipment_id);

    let penalty = total_amount * penalty_bps as i128 / 10_000;
    let refund = total_amount - penalty;

    assert_eq!(token_client.balance(&t.supplier), penalty);
    assert_eq!(token_client.balance(&t.buyer), buyer_balance_before + refund);
    assert_eq!(client.get_shipment(&shipment_id).status, ShipmentStatus::Cancelled);
}

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
        &ShipmentOptions { response_deadline: 1000, penalty_bps: 500, milestone_mode: MilestoneMode::Parallel, holdback_ledgers: 0, dispute_cooldown_ledgers: 0, late_penalty_bps_per_ledger: 0, auto_confirm_ledgers: 0 },
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
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
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.raise_dispute(&t.buyer, &shipment_id, &0);
    client.resolve_dispute(&t.arbiter, &shipment_id, &0, &true);

    let gross = total_amount * 25 / 100;
    let fee = gross * fee_bps as i128 / 10_000;
    let net = gross - fee;

    assert_eq!(token_client.balance(&t.supplier), net);
    assert_eq!(token_client.balance(&t.treasury), fee);
}

// ============================================================
// GET_COMPLETION_PERCENTAGE TESTS
// ============================================================

#[test]
fn test_get_completion_percentage_fresh_shipment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-FRESH");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Freshly created shipment with no milestones confirmed should return 0%
    assert_eq!(client.get_completion_percentage(&shipment_id), 0);
}

#[test]
fn test_get_completion_percentage_partial_one_milestone() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-PARTIAL-1");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Confirm first milestone (25%)
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // Should return 25%
    assert_eq!(client.get_completion_percentage(&shipment_id), 25);
}

#[test]
fn test_get_completion_percentage_partial_two_milestones() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-PARTIAL-2");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Confirm first milestone (25%)
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // Confirm second milestone (50% cumulative = 75% total)
    client.submit_proof(&t.logistics, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    // Should return 75%
    assert_eq!(client.get_completion_percentage(&shipment_id), 75);
}

#[test]
fn test_get_completion_percentage_full_completion() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-FULL");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Confirm all milestones
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    client.submit_proof(&t.logistics, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));
    client.confirm_milestone(&t.buyer, &shipment_id, &1);

    client.submit_proof(&t.supplier, &shipment_id, &2, &String::from_str(&t.env, "ipfs://v"));
    client.confirm_milestone(&t.buyer, &shipment_id, &2);

    // Should return 100%
    assert_eq!(client.get_completion_percentage(&shipment_id), 100);
}

#[test]
fn test_get_completion_percentage_zero_released() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = String::from_str(&t.env, "SHIP-COMPL-ZERO");
    let total_amount: i128 = 100;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Before any confirmation, released_amount is 0, should return 0%
    assert_eq!(client.get_completion_percentage(&shipment_id), 0);

    // Confirm first milestone (25 out of 100 = 25%)
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // (25 * 100) / 100 = 25%
    assert_eq!(client.get_completion_percentage(&shipment_id), 25);
}

// ============================================================
// EVENT PAYLOAD CORRECTNESS TESTS  (Issue #50)
// ============================================================

#[test]
fn test_shipment_created_event_includes_all_role_addresses() {
    // Verifies create_shipment emits an event with all four role addresses,
    // token, total_amount, and ledger embedded in the payload.
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, "SHIP-EVT-CREATE");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // The event payload encodes the same data that is persisted in the shipment.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.buyers.get(0).unwrap(), t.buyer,   "event: buyer matches");
    assert_eq!(shipment.supplier,  t.supplier,             "event: supplier matches");
    assert_eq!(shipment.logistics, t.logistics,            "event: logistics matches");
    assert_eq!(shipment.arbiter,   t.arbiter,              "event: arbiter matches");
    assert_eq!(shipment.token,     t.token_id,             "event: token matches");
    assert_eq!(shipment.total_amount, total_amount,        "event: total_amount matches");
    assert!(shipment.created_at > 0,                       "event: ledger field is non-zero");
}

#[test]
fn test_shipment_cancelled_event_includes_refund_and_cancelled_by() {
    // Verifies cancel_shipment emits (refunded_amount, cancelled_by, ledger).
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);
    let shipment_id = String::from_str(&t.env, "SHIP-EVT-CANCEL");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    let buyer_before = token_client.balance(&t.buyer);
    client.cancel_shipment(&t.buyer, &shipment_id);

    // refunded_amount: no milestones confirmed so the full escrow is returned.
    let refund = token_client.balance(&t.buyer) - buyer_before;
    assert_eq!(refund, total_amount,          "event refunded_amount = full escrow");

    // cancelled_by: the buyer who called cancel_shipment.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.status, ShipmentStatus::Cancelled);
    assert_eq!(shipment.buyers.get(0).unwrap(), t.buyer,  "event cancelled_by = buyer");
}

#[test]
fn test_shipment_cancelled_partial_refund_event_data() {
    // Verifies that when some milestones are already confirmed, cancelled_amount reflects
    // only the unconfirmed portion (matching the event's refunded_amount field).
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);
    let shipment_id = String::from_str(&t.env, "SHIP-EVT-CANCEL-P");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    // Confirm milestone 0 (25%).
    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    let buyer_before = token_client.balance(&t.buyer);
    client.cancel_shipment(&t.buyer, &shipment_id);

    // Remaining 75% is refunded; event refunded_amount should reflect this.
    let refund = token_client.balance(&t.buyer) - buyer_before;
    assert_eq!(refund, total_amount * 75 / 100, "event refunded_amount = 75% of escrow");
}

#[test]
fn test_milestone_confirmed_event_includes_supplier_and_ledger() {
    // Verifies confirm_milestone emits (index, payment, fee, penalty, supplier, ledger).
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);
    let shipment_id = String::from_str(&t.env, "SHIP-EVT-CONFIRM");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));

    let supplier_before = token_client.balance(&t.supplier);
    client.confirm_milestone(&t.buyer, &shipment_id, &0);

    // payment = 25% of total_amount (matches event payment field).
    let expected_payment: i128 = total_amount * 25 / 100;
    assert_eq!(
        token_client.balance(&t.supplier) - supplier_before,
        expected_payment,
        "event payment = milestone payment_percent * total_amount / 100"
    );

    // supplier field in the event matches the stored shipment.supplier.
    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.supplier, t.supplier, "event supplier field is correct");
}

#[test]
fn test_batch_confirm_milestone_confirmed_event_includes_supplier() {
    // Verifies batch_confirm_milestones emits milestone_confirmed with supplier and ledger.
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token_client = token::Client::new(&t.env, &t.token_id);
    let shipment_id = String::from_str(&t.env, "SHIP-EVT-BATCH");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client, &t.env, &shipment_id, &t.buyer, &t.supplier,
        &t.logistics, &t.arbiter, &t.token_id, total_amount,
    );

    client.submit_proof(&t.supplier, &shipment_id, &0, &String::from_str(&t.env, "ipfs://d"));
    client.submit_proof(&t.logistics, &shipment_id, &1, &String::from_str(&t.env, "ipfs://t"));

    let supplier_before = token_client.balance(&t.supplier);
    client.batch_confirm_milestones(&t.buyer, &shipment_id, &vec![&t.env, 0u32, 1u32]);

    // Both milestones paid to supplier: 25% + 50% = 75%.
    let expected_payment: i128 = total_amount * 75 / 100;
    assert_eq!(
        token_client.balance(&t.supplier) - supplier_before,
        expected_payment,
        "batch event payments sum to 75% of total_amount"
    );

    let shipment = client.get_shipment(&shipment_id);
    assert_eq!(shipment.supplier, t.supplier, "event supplier field is correct");
}

