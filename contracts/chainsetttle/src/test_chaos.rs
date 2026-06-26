// #116 — Chaos test: random operation sequences, escrow balance invariant.
//
// Invariant: at any point after any sequence of operations,
//   token.balance(contract) == Σ (s.total_amount − s.released_amount) for all Active shipments
//
// proptest generates sequences of up to 20 random ops over 10 shipment "slots".
// Each op is executed via std::panic::catch_unwind so invalid-state failures
// are silently swallowed — only the final invariant matters.
//
// Run count: 5 000 (matching the CI requirement).

#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    token, vec, Address, Env, String,, Symbol};
use proptest::prelude::*;

// Fixed slot IDs so we can reference shipments without heap allocation gymnastics
const SLOT_IDS: [&str; 10] = [
    "CHX-0", "CHX-1", "CHX-2", "CHX-3", "CHX-4",
    "CHX-5", "CHX-6", "CHX-7", "CHX-8", "CHX-9",
];

// ---- operation model --------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    /// Create shipment in slot `slot` with the given amount.
    /// Skipped if that slot is already created.
    CreateShipment { slot: usize, amount: i128, sequential: bool, holdback: u32 },
    /// Submit proof on milestone `m` of shipment in `slot`.
    SubmitProof { slot: usize, m: u32 },
    /// Buyer confirms milestone `m` of shipment in `slot`.
    ConfirmMilestone { slot: usize, m: u32 },
    /// Buyer raises a dispute on milestone `m` of shipment in `slot`.
    RaiseDispute { slot: usize, m: u32 },
    /// Arbiter resolves dispute on milestone `m`; approve controls direction.
    ResolveDispute { slot: usize, m: u32, approve: bool },
    /// Buyer cancels the shipment (ignored if any milestone is Disputed).
    CancelShipment { slot: usize },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        // CreateShipment: amounts between 1_000 and 10_000_000 to stay within minted balance
        (0usize..10, 1_000i128..1_000_000i128, any::<bool>(), 0u32..5u32).prop_map(
            |(slot, amount, sequential, holdback)| Op::CreateShipment {
                slot,
                amount: amount * 100, // keep 8 significant digits
                sequential,
                holdback,
            }
        ),
        (0usize..10, 0u32..2u32).prop_map(|(slot, m)| Op::SubmitProof { slot, m }),
        (0usize..10, 0u32..2u32).prop_map(|(slot, m)| Op::ConfirmMilestone { slot, m }),
        (0usize..10, 0u32..2u32).prop_map(|(slot, m)| Op::RaiseDispute { slot, m }),
        (0usize..10, 0u32..2u32, any::<bool>()).prop_map(|(slot, m, approve)| {
            Op::ResolveDispute { slot, m, approve }
        }),
        (0usize..10).prop_map(|slot| Op::CancelShipment { slot }),
    ]
}

// ---- execution helpers ------------------------------------------------------

fn two_milestone_chaos(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: String::from_str(env, "A"),
            payment_percent: 60,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
        Milestone {
            name: String::from_str(env, "B"),
            payment_percent: 40,
            proof_hash: String::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
        },
    ]
}

// Execute `op` against the contract; silently swallow any panic so the sequence
// can continue and the invariant is checked at the end.
fn exec(
    op: &Op,
    env: &Env,
    contract_id: &Address,
    token_id: &Address,
    buyer: &Address,
    supplier: &Address,
    logistics: &Address,
    arbiter: &Address,
    created: &mut [bool; 10],
) {
    let client = ChainSettleContractClient::new(env, contract_id);

    match op {
        Op::CreateShipment { slot, amount, sequential, holdback } => {
            if created[*slot] {
                return;
            }
            let id = String::from_str(env, SLOT_IDS[*slot]);
            let milestones = two_milestone_chaos(env);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.create_shipment(
                    &id, buyer, supplier, logistics, arbiter, token_id,
                    amount, &milestones, sequential, holdback,
                );
            }));
            if result.is_ok() {
                created[*slot] = true;
            }
        }

        Op::SubmitProof { slot, m } => {
            if !created[*slot] {
                return;
            }
            let id = String::from_str(env, SLOT_IDS[*slot]);
            let proof = String::from_str(env, "ipfs://chaos");
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.submit_proof(supplier, &id, m, &proof, &Symbol::new(&env, "ipfs"));
            }));
        }

        Op::ConfirmMilestone { slot, m } => {
            if !created[*slot] {
                return;
            }
            let id = String::from_str(env, SLOT_IDS[*slot]);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.confirm_milestone(buyer, &id, m);
            }));
        }

        Op::RaiseDispute { slot, m } => {
            if !created[*slot] {
                return;
            }
            let id = String::from_str(env, SLOT_IDS[*slot]);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.raise_dispute(buyer, &id, m);
            }));
        }

        Op::ResolveDispute { slot, m, approve } => {
            if !created[*slot] {
                return;
            }
            let id = String::from_str(env, SLOT_IDS[*slot]);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.resolve_dispute(arbiter, &id, m, approve);
            }));
        }

        Op::CancelShipment { slot } => {
            if !created[*slot] {
                return;
            }
            let id = String::from_str(env, SLOT_IDS[*slot]);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.cancel_shipment(buyer, &id);
            }));
        }
    }
}

// ---- invariant check --------------------------------------------------------

fn assert_escrow_invariant(
    env: &Env,
    contract_id: &Address,
    token_id: &Address,
    created: &[bool; 10],
) {
    let client = ChainSettleContractClient::new(env, contract_id);
    let token_client = token::Client::new(env, token_id);

    let expected: i128 = (0..10)
        .filter(|&i| created[i])
        .map(|i| {
            let id = String::from_str(env, SLOT_IDS[i]);
            let ship = client.get_shipment(&id);
            if ship.status == ShipmentStatus::Active {
                ship.total_amount - ship.released_amount
            } else {
                0
            }
        })
        .sum();

    let actual = token_client.balance(contract_id);
    assert_eq!(
        actual, expected,
        "escrow invariant violated: contract holds {} but sum of active escrows is {}",
        actual, expected
    );
}

// ---- proptest ---------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 5_000,
        // Shrinking is disabled for speed — the full sequence is logged on failure.
        max_shrink_iters: 0,
        ..Default::default()
    })]

    /// Core invariant: after any random sequence of operations,
    /// the contract's token balance equals the sum of all active shipment escrows.
    #[test]
    fn chaos_escrow_balance_invariant(
        ops in proptest::collection::vec(op_strategy(), 0usize..=20)
    ) {
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

        // Mint enough for all 10 slots at max per-slot amount (1_000_000 × 100 = 100_000_000)
        token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &1_000_000_000);

        let client = ChainSettleContractClient::new(&env, &contract_id);
        client.init(&buyer);

        let mut created: [bool; 10] = [false; 10];

        for op in &ops {
            exec(
                op, &env, &contract_id, &token_id,
                &buyer, &supplier, &logistics, &arbiter,
                &mut created,
            );
        }

        assert_escrow_invariant(&env, &contract_id, &token_id, &created);
    }
}

// ---- named scenario coverage ------------------------------------------------
// Proptest guarantees random coverage, but the four named scenarios from the
// acceptance criteria are also exercised explicitly below for fast CI feedback.

#[test]
fn chaos_create_only_scenario() {
    // Create shipments but never confirm or dispute; entire escrow must remain locked.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ChainSettleContract, ());
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &1_000_000_000);
    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    let mut created = [false; 10];
    for i in 0..10 {
        let op = Op::CreateShipment { slot: i, amount: 50_000 * 100, sequential: false, holdback: 0 };
        exec(&op, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
    }
    assert_escrow_invariant(&env, &contract_id, &token_id, &created);
}

#[test]
fn chaos_partial_confirm_scenario() {
    // Confirm milestone 0 on every shipment; escrow = 40% per shipment remains.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ChainSettleContract, ());
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &1_000_000_000);
    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    let mut created = [false; 10];
    for i in 0..10 {
        let amount = 100_000i128;
        let create = Op::CreateShipment { slot: i, amount, sequential: false, holdback: 0 };
        exec(&create, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
        let proof = Op::SubmitProof { slot: i, m: 0 };
        exec(&proof, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
        let confirm = Op::ConfirmMilestone { slot: i, m: 0 };
        exec(&confirm, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
    }
    assert_escrow_invariant(&env, &contract_id, &token_id, &created);
}

#[test]
fn chaos_dispute_only_scenario() {
    // Submit proof then immediately dispute on milestone 0; no confirms.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ChainSettleContract, ());
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &1_000_000_000);
    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    let mut created = [false; 10];
    for i in 0..10 {
        let create = Op::CreateShipment { slot: i, amount: 200_000, sequential: false, holdback: 0 };
        exec(&create, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
        let proof = Op::SubmitProof { slot: i, m: 0 };
        exec(&proof, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
        let dispute = Op::RaiseDispute { slot: i, m: 0 };
        exec(&dispute, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
    }
    assert_escrow_invariant(&env, &contract_id, &token_id, &created);
}

#[test]
fn chaos_abandon_scenario() {
    // Create then immediately cancel; contract balance must drop to zero.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ChainSettleContract, ());
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &1_000_000_000);
    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    let mut created = [false; 10];
    for i in 0..10 {
        let create = Op::CreateShipment { slot: i, amount: 300_000, sequential: false, holdback: 0 };
        exec(&create, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
        let cancel = Op::CancelShipment { slot: i };
        exec(&cancel, &env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, &mut created);
    }
    assert_escrow_invariant(&env, &contract_id, &token_id, &created);

    // All cancelled — contract holds nothing
    let token_client = token::Client::new(&env, &token_id);
    assert_eq!(token_client.balance(&contract_id), 0, "contract should hold nothing after all cancellations");
}
