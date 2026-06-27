// #120 — WASM upgrade migration test
// Verifies that persistent state (all shipment fields) survives a WASM upgrade.
//
// Pre-requisite: build the contract WASM before running this test:
//   stellar contract build   (from workspace root)
// This produces: target/wasm32v1-none/release/chainsetttle.wasm

#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    token, vec, Address, Env, String, Symbol};

// Real WASM bytes — the same binary is used for both v1 and v2 in this test,
// which isolates the state-persistence concern from any logic change.
const WASM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32v1-none/release/chainsetttle.wasm"
));

// ---- helpers ----------------------------------------------------------------

fn two_milestone_vec(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Phase 1"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: String::from_str(env, "Phase 2"),
            payment_percent: 50,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ]
}

fn make_shipment(
    client: &ChainSettleContractClient,
    env: &Env,
    id: &str,
    buyer: &Address,
    supplier: &Address,
    logistics: &Address,
    arbiter: &Address,
    token_id: &Address,
    amount: i128,
) {
    client.create_shipment(
        &String::from_str(env, id),
        buyer,
        supplier,
        logistics,
        arbiter,
        token_id,
        &amount,
        &two_milestone_vec(env),
        &false,
        &0,
    );
}

// ---- test -------------------------------------------------------------------

/// Deploy, create 3 shipments at varying lifecycle stages, upgrade WASM,
/// then assert all pre-upgrade state is intact and write operations still work.
///
/// Lifecycle stages covered:
///   S1 — freshly created, all milestones Pending
///   S2 — milestone 0 Confirmed, milestone 1 ProofSubmitted
///   S3 — milestone 0 Disputed (active dispute)
#[test]
fn test_wasm_upgrade_state_persists() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &100_000_000);

    let client = ChainSettleContractClient::new(&env, &contract_id);
    let token_client = token::Client::new(&env, &token_id);
    client.init(&buyer);

    let amount: i128 = 1_000_000;

    // --- S1: pure Pending
    make_shipment(&client, &env, "UPG-001", &buyer, &supplier, &logistics, &arbiter, &token_id, amount);

    // --- S2: milestone 0 Confirmed, milestone 1 ProofSubmitted
    make_shipment(&client, &env, "UPG-002", &buyer, &supplier, &logistics, &arbiter, &token_id, amount);
    let id2 = String::from_str(&env, "UPG-002");
    client.submit_proof(&supplier, &id2, &0, &String::from_str(&env, "ipfs://s2-m0"), &Symbol::new(&env, "ipfs"));
    client.confirm_milestone(&buyer, &id2, &0);
    client.submit_proof(&logistics, &id2, &1, &String::from_str(&env, "ipfs://s2-m1"), &Symbol::new(&env, "ipfs"));

    // --- S3: milestone 0 Disputed
    make_shipment(&client, &env, "UPG-003", &buyer, &supplier, &logistics, &arbiter, &token_id, amount);
    let id3 = String::from_str(&env, "UPG-003");
    client.submit_proof(&supplier, &id3, &0, &String::from_str(&env, "ipfs://s3-m0"), &Symbol::new(&env, "ipfs"));
    client.raise_dispute(&buyer, &id3, &0);

    // Snapshot pre-upgrade values
    let s2_released_pre = client.get_shipment(&id2).released_amount;

    // --- Upgrade ---
    // Upload the same binary as "v2" — tests storage key compatibility, not logic change.
    let new_wasm_hash = env.deployer().upload_contract_wasm(WASM);
    client.upgrade(&new_wasm_hash);

    // ---- Post-upgrade reads must succeed with identical data ----------------

    let id1 = String::from_str(&env, "UPG-001");
    let s1_post = client.get_shipment(&id1);
    assert_eq!(s1_post.status, ShipmentStatus::Active, "S1 status changed across upgrade");
    assert_eq!(s1_post.total_amount, amount, "S1 total_amount corrupted");
    assert_eq!(s1_post.released_amount, 0, "S1 released_amount corrupted");
    assert_eq!(s1_post.milestones.len(), 2, "S1 milestone count changed");
    assert_eq!(
        s1_post.milestones.get(0).unwrap().status,
        MilestoneStatus::Pending,
        "S1 milestone 0 status corrupted"
    );
    assert_eq!(
        client.get_escrow_balance(&id1),
        amount,
        "S1 escrow balance wrong post-upgrade"
    );

    let s2_post = client.get_shipment(&id2);
    assert_eq!(s2_post.status, ShipmentStatus::Active, "S2 status changed across upgrade");
    assert_eq!(s2_post.released_amount, s2_released_pre, "S2 released_amount corrupted");
    assert_eq!(
        s2_post.milestones.get(0).unwrap().status,
        MilestoneStatus::Confirmed,
        "S2 milestone 0 status corrupted"
    );
    assert_eq!(
        s2_post.milestones.get(1).unwrap().status,
        MilestoneStatus::ProofSubmitted,
        "S2 milestone 1 status corrupted"
    );
    assert_eq!(
        s2_post.milestones.get(1).unwrap().proof_hash,
        String::from_str(&env, "ipfs://s2-m1"),
        "S2 proof_hash corrupted"
    );

    let s3_post = client.get_shipment(&id3);
    assert_eq!(s3_post.status, ShipmentStatus::Active, "S3 status changed across upgrade");
    assert_eq!(
        s3_post.milestones.get(0).unwrap().status,
        MilestoneStatus::Disputed,
        "S3 active dispute not preserved across upgrade"
    );

    // ---- Post-upgrade write path: confirm remaining milestone on S2 ----------
    client.confirm_milestone(&buyer, &id2, &1);
    let s2_done = client.get_shipment(&id2);
    assert_eq!(
        s2_done.status,
        ShipmentStatus::Completed,
        "S2 should complete after confirming final milestone post-upgrade"
    );
    assert_eq!(s2_done.released_amount, amount, "S2 released_amount wrong post-upgrade");
    assert_eq!(
        client.get_escrow_balance(&id2),
        0,
        "S2 escrow should be zero when completed"
    );

    // ---- Active dispute on S3 must be resolvable post-upgrade ---------------
    client.resolve_dispute(&arbiter, &id3, &0, &true);
    assert_eq!(
        client.get_shipment(&id3).milestones.get(0).unwrap().status,
        MilestoneStatus::Resolved,
        "S3 dispute not resolvable post-upgrade"
    );

    // Storage key structure unchanged: token balances reflect correct transfers.
    // S2: 50% released pre-upgrade + 50% released post-upgrade = full amount.
    // S3: milestone 0 approved at 50%.
    let expected_supplier = amount + amount / 2;
    assert_eq!(
        token_client.balance(&supplier),
        expected_supplier,
        "supplier token balance wrong after post-upgrade operations"
    );
}
