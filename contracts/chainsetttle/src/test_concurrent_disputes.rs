// #117 — Concurrent dispute simulation across 10 simultaneous active shipments.
// Opens disputes on 10 shipments, resolves them in a shuffled order, and asserts
// that each shipment's state and escrow balance are independent.

#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    token, vec, Address, Env, String,, Symbol};

// Resolution order: deliberately non-sequential to catch cross-shipment contamination.
const RESOLUTION_ORDER: [usize; 10] = [5, 2, 8, 1, 6, 0, 9, 3, 7, 4];

fn three_milestone_vec(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "Dispatch"),
            payment_percent: 40,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "Transit"),
            payment_percent: 40,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "Delivery"),
            payment_percent: 20,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
    ]
}

fn ship_id(env: &Env, i: usize) -> String {
    // Fixed IDs so we can reference them by index without allocation trickery.
    let ids = [
        "CD-000", "CD-001", "CD-002", "CD-003", "CD-004",
        "CD-005", "CD-006", "CD-007", "CD-008", "CD-009",
    ];
    String::from_str(env, ids[i])
}

/// Create shipment i, submit proof on milestone 0, then immediately raise a dispute.
fn create_and_dispute(
    client: &ChainSettleContractClient,
    env: &Env,
    i: usize,
    buyer: &Address,
    supplier: &Address,
    logistics: &Address,
    arbiter: &Address,
    token_id: &Address,
    amount: i128,
) {
    let id = ship_id(env, i);
    client.create_shipment(
        &id, buyer, supplier, logistics, arbiter, token_id,
        &amount, &three_milestone_vec(env), &false, &0,
    );
    let proof = String::from_str(env, "ipfs://concurrent-proof");
    client.submit_proof(supplier, &id, &0, &proof, &Symbol::new(&env, "ipfs"));
    client.raise_dispute(buyer, &id, &0);
}

// ---- tests ------------------------------------------------------------------

/// All 10 disputes opened simultaneously, resolved in shuffled order.
/// Each shipment's escrow and token balances must settle independently.
#[test]
fn test_10_concurrent_disputes_resolved_independently() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
    let token_client = token::Client::new(&env, &token_id);

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    let amount: i128 = 1_000_000;
    // Mint enough for 10 shipments
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &(amount * 10));

    client.init(&buyer);

    // Step 1: Create all 10 shipments and raise disputes in rapid succession
    for i in 0..10 {
        create_and_dispute(&client, &env, i, &buyer, &supplier, &logistics, &arbiter, &token_id, amount);
    }

    // Confirm all 10 are Disputed before any resolution
    for i in 0..10 {
        let id = ship_id(&env, i);
        assert_eq!(
            client.get_milestone(&id, &0).status,
            MilestoneStatus::Disputed,
            "shipment {} should be disputed before resolution pass", i
        );
    }

    // Step 2: Resolve in shuffled order — alternating approve/reject for variety
    for (pass, &i) in RESOLUTION_ORDER.iter().enumerate() {
        let id = ship_id(&env, i);
        let approve = pass % 2 == 0; // even passes approve, odd reject
        client.resolve_dispute(&arbiter, &id, &0, &approve);
    }

    // Step 3: Verify each shipment settled independently
    let mut total_supplier_expected: i128 = 0;

    for (pass, &i) in RESOLUTION_ORDER.iter().enumerate() {
        let id = ship_id(&env, i);
        let approve = pass % 2 == 0;
        let shipment = client.get_shipment(&id);

        // Milestone 0 carries 40% of amount
        let milestone_payment = amount * 40 / 100;

        if approve {
            // Approved: milestone 0 Resolved, released_amount incremented
            assert_eq!(
                shipment.milestones.get(0).unwrap().status,
                MilestoneStatus::Resolved,
                "shipment {} milestone 0 should be Resolved", i
            );
            assert_eq!(
                shipment.released_amount, milestone_payment,
                "shipment {} released_amount wrong after approval", i
            );
            // Escrow = total - released
            assert_eq!(
                client.get_escrow_balance(&id),
                amount - milestone_payment,
                "shipment {} escrow balance wrong after approval", i
            );
            total_supplier_expected += milestone_payment;
        } else {
            // Rejected: milestone 0 back to Pending, released_amount unchanged
            assert_eq!(
                shipment.milestones.get(0).unwrap().status,
                MilestoneStatus::Pending,
                "shipment {} milestone 0 should be Pending after rejection", i
            );
            assert_eq!(
                shipment.released_amount, 0,
                "shipment {} released_amount should be 0 after rejection", i
            );
            assert_eq!(
                client.get_escrow_balance(&id),
                amount,
                "shipment {} escrow balance wrong after rejection", i
            );
        }

        // Shipment must still be Active (one milestone done doesn't complete it)
        assert_eq!(
            shipment.status, ShipmentStatus::Active,
            "shipment {} should still be Active", i
        );
    }

    // Step 4: Supplier's total token balance equals only the approved payments —
    // no cross-contamination from other shipments.
    assert_eq!(
        token_client.balance(&supplier),
        total_supplier_expected,
        "supplier total balance does not match sum of individually approved payments"
    );

    // Step 5: Contract holds the correct remaining escrow for every shipment
    let expected_contract_balance: i128 = (0..10).map(|i| {
        let id = ship_id(&env, i);
        client.get_shipment(&id).total_amount - client.get_shipment(&id).released_amount
    }).sum();
    assert_eq!(
        token_client.balance(&contract_id),
        expected_contract_balance,
        "contract balance does not match sum of all remaining escrows"
    );
}

/// Interleaved resolution order must not affect any other shipment's milestone statuses.
#[test]
fn test_dispute_resolution_does_not_cross_contaminate_milestones() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());
    let client = ChainSettleContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    let amount: i128 = 500_000;
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &(amount * 10));

    client.init(&buyer);

    for i in 0..10 {
        create_and_dispute(&client, &env, i, &buyer, &supplier, &logistics, &arbiter, &token_id, amount);
    }

    // Resolve all in shuffled order (all approved this time)
    for &i in &RESOLUTION_ORDER {
        let id = ship_id(&env, i);
        client.resolve_dispute(&arbiter, &id, &0, &true);

        // Immediately after resolving shipment i, all OTHER shipments in the
        // pending-resolution set must still be in Disputed state.
        for j in 0..10 {
            if j == i { continue; }
            let other_id = ship_id(&env, j);
            let other_ship = client.get_shipment(&other_id);
            // Milestones that haven't been resolved yet remain Disputed
            let m0_status = other_ship.milestones.get(0).unwrap().status;
            let already_resolved = RESOLUTION_ORDER
                .iter()
                .position(|&x| x == j)
                .map(|pos| RESOLUTION_ORDER.iter().position(|&x| x == i).unwrap() >= pos)
                .unwrap_or(false);
            if !already_resolved {
                assert_eq!(
                    m0_status,
                    MilestoneStatus::Disputed,
                    "shipment {} milestone 0 should still be Disputed while shipment {} was resolved", j, i
                );
            }
        }
    }
}
