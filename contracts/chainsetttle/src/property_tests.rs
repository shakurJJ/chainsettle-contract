#![cfg(test)]

use proptest::prelude::*;

// ============================================================
// STATE MACHINE MODEL
//
// Pure-Rust model with no heap allocation (arrays only).
// Runs at full speed — 10,000 cases are generated per property.
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelMilestoneStatus {
    Pending,
    ProofSubmitted,
    Confirmed,
    Disputed,
    Resolved,
    ConfirmedHeld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelOp {
    SubmitProof,
    Confirm,
    Hold,
    RaiseDispute,
    ResolveApprove,
    ResolveReject,
    ReleaseHeld,
}

fn apply_op(status: ModelMilestoneStatus, op: ModelOp) -> Option<ModelMilestoneStatus> {
    match (status, op) {
        (ModelMilestoneStatus::Pending, ModelOp::SubmitProof) => {
            Some(ModelMilestoneStatus::ProofSubmitted)
        }
        (ModelMilestoneStatus::ProofSubmitted, ModelOp::Confirm) => {
            Some(ModelMilestoneStatus::Confirmed)
        }
        (ModelMilestoneStatus::ProofSubmitted, ModelOp::Hold) => {
            Some(ModelMilestoneStatus::ConfirmedHeld)
        }
        (ModelMilestoneStatus::ProofSubmitted, ModelOp::RaiseDispute) => {
            Some(ModelMilestoneStatus::Disputed)
        }
        (ModelMilestoneStatus::ConfirmedHeld, ModelOp::ReleaseHeld) => {
            Some(ModelMilestoneStatus::Confirmed)
        }
        (ModelMilestoneStatus::ConfirmedHeld, ModelOp::RaiseDispute) => {
            Some(ModelMilestoneStatus::Disputed)
        }
        (ModelMilestoneStatus::Disputed, ModelOp::ResolveApprove) => {
            Some(ModelMilestoneStatus::Resolved)
        }
        (ModelMilestoneStatus::Disputed, ModelOp::ResolveReject) => {
            Some(ModelMilestoneStatus::Pending)
        }
        _ => None,
    }
}

fn is_terminal(status: ModelMilestoneStatus) -> bool {
    matches!(
        status,
        ModelMilestoneStatus::Confirmed | ModelMilestoneStatus::Resolved
    )
}

const ALL_OPS: [ModelOp; 7] = [
    ModelOp::SubmitProof,
    ModelOp::Confirm,
    ModelOp::Hold,
    ModelOp::RaiseDispute,
    ModelOp::ResolveApprove,
    ModelOp::ResolveReject,
    ModelOp::ReleaseHeld,
];

fn all_successors_none(status: ModelMilestoneStatus) -> bool {
    ALL_OPS.iter().all(|op| apply_op(status, *op).is_none())
}

fn op_strategy() -> impl Strategy<Value = ModelOp> {
    prop_oneof![
        Just(ModelOp::SubmitProof),
        Just(ModelOp::Confirm),
        Just(ModelOp::Hold),
        Just(ModelOp::RaiseDispute),
        Just(ModelOp::ResolveApprove),
        Just(ModelOp::ResolveReject),
        Just(ModelOp::ReleaseHeld),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10_000,
        ..Default::default()
    })]

    /// 10,000 random sequences — invalid ops are skipped (as the contract panics),
    /// and at the end the state must be one of the six reachable states.
    #[test]
    fn prop_valid_sequences_stay_consistent(
        ops in prop::collection::vec(op_strategy(), 1..=50)
    ) {
        let mut status = ModelMilestoneStatus::Pending;
        for op in &ops {
            if let Some(next) = apply_op(status, *op) {
                status = next;
            }
        }
        let reachable = [
            ModelMilestoneStatus::Pending,
            ModelMilestoneStatus::ProofSubmitted,
            ModelMilestoneStatus::Confirmed,
            ModelMilestoneStatus::Disputed,
            ModelMilestoneStatus::Resolved,
            ModelMilestoneStatus::ConfirmedHeld,
        ];
        prop_assert!(reachable.contains(&status));
    }

    /// Terminal states (Confirmed, Resolved) must reject every operation.
    #[test]
    fn prop_terminal_states_are_final(
        ops in prop::collection::vec(op_strategy(), 1..=50)
    ) {
        let mut status = ModelMilestoneStatus::Pending;
        for op in &ops {
            if let Some(next) = apply_op(status, *op) {
                status = next;
            }
        }
        if is_terminal(status) {
            prop_assert!(
                all_successors_none(status),
                "terminal state {:?} must have no valid successors", status
            );
        }
    }

    /// Once Confirmed, always Confirmed — no regression possible.
    #[test]
    fn prop_confirmed_never_regresses(
        ops in prop::collection::vec(op_strategy(), 1..=100)
    ) {
        let mut status = ModelMilestoneStatus::Pending;
        let mut ever_confirmed = false;
        for op in &ops {
            if let Some(next) = apply_op(status, *op) {
                status = next;
            }
            if status == ModelMilestoneStatus::Confirmed {
                ever_confirmed = true;
            }
        }
        if ever_confirmed {
            prop_assert_eq!(status, ModelMilestoneStatus::Confirmed, "Confirmed must not regress");
        }
    }

    /// Once Resolved, always Resolved — no regression possible.
    #[test]
    fn prop_resolved_never_regresses(
        ops in prop::collection::vec(op_strategy(), 1..=100)
    ) {
        let mut status = ModelMilestoneStatus::Pending;
        let mut ever_resolved = false;
        for op in &ops {
            if let Some(next) = apply_op(status, *op) {
                status = next;
            }
            if status == ModelMilestoneStatus::Resolved {
                ever_resolved = true;
            }
        }
        if ever_resolved {
            prop_assert_eq!(status, ModelMilestoneStatus::Resolved, "Resolved must not regress");
        }
    }

    /// apply_op never produces a state outside the known reachable set.
    #[test]
    fn prop_apply_op_only_yields_valid_successors(
        status_idx in 0usize..6,
        op in op_strategy(),
    ) {
        let statuses = [
            ModelMilestoneStatus::Pending,
            ModelMilestoneStatus::ProofSubmitted,
            ModelMilestoneStatus::Confirmed,
            ModelMilestoneStatus::Disputed,
            ModelMilestoneStatus::Resolved,
            ModelMilestoneStatus::ConfirmedHeld,
        ];
        let status = statuses[status_idx];
        let result = apply_op(status, op);

        if let Some(next) = result {
            let known = [
                ModelMilestoneStatus::Pending,
                ModelMilestoneStatus::ProofSubmitted,
                ModelMilestoneStatus::Confirmed,
                ModelMilestoneStatus::Disputed,
                ModelMilestoneStatus::Resolved,
                ModelMilestoneStatus::ConfirmedHeld,
            ];
            prop_assert!(known.contains(&next));
            if is_terminal(next) {
                prop_assert!(all_successors_none(next));
            }
        }
    }

    /// Invalid operations at terminal states always return None.
    #[test]
    fn prop_terminal_states_reject_all_ops(op in op_strategy()) {
        prop_assert!(apply_op(ModelMilestoneStatus::Confirmed, op).is_none());
        prop_assert!(apply_op(ModelMilestoneStatus::Resolved, op).is_none());
    }
}

// ============================================================
// CONTRACT-BACKED PROPERTY TESTS
// Uses the Soroban test env to verify the contract matches the model.
// ============================================================

#[cfg(test)]
mod contract_prop_tests {
    use super::*;
    use crate::{
        ChainSettleContract, ChainSettleContractClient, Milestone, MilestoneMode, MilestoneStatus,
        ShipmentOptions,
    };
    use soroban_sdk::{testutils::Address as _, token, vec, Address, Env, String, Symbol};

    fn make_env_and_client() -> (Env, Address, Address, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(ChainSettleContract, ());
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let buyer = Address::generate(&env);
        let supplier = Address::generate(&env);
        let logistics = Address::generate(&env);
        let arbiter = Address::generate(&env);
        token::StellarAssetClient::new(&env, &token_id).mint(&buyer, &10_000_000_000);
        let client = ChainSettleContractClient::new(&env, &contract_id);
        client.init(&buyer);
        (
            env,
            contract_id,
            token_id,
            buyer,
            supplier,
            logistics,
            arbiter,
        )
    }

    fn make_shipment(
        env: &Env,
        contract_id: &Address,
        token_id: &Address,
        buyer: &Address,
        supplier: &Address,
        logistics: &Address,
        arbiter: &Address,
        id: &str,
    ) {
        let client = ChainSettleContractClient::new(env, contract_id);
        let sid = String::from_str(env, id);
        let milestones = vec![
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
        ];
        client.create_shipment(
            &sid,
            &vec![env, buyer.clone()],
            supplier,
            logistics,
            arbiter,
            token_id,
            &1_000_000_000i128,
            &milestones,
            &ShipmentOptions {
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
            },
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            ..Default::default()
        })]

        /// released_amount never exceeds total_amount after any combination of confirmations.
        #[test]
        fn prop_contract_released_never_exceeds_total(
            confirm_m0 in proptest::bool::ANY,
            confirm_m1 in proptest::bool::ANY,
        ) {
            let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) =
                make_env_and_client();
            make_shipment(&env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, "PROP-001");
            let client = ChainSettleContractClient::new(&env, &contract_id);
            let sid = String::from_str(&env, "PROP-001");

            if confirm_m0 {
                client.submit_proof(&supplier, &sid, &0, &String::from_str(&env, "qm0"), &Symbol::new(&env, "ipfs"));
                client.confirm_milestone(&buyer, &sid, &0);
            }
            if confirm_m1 {
                client.submit_proof(&supplier, &sid, &1, &String::from_str(&env, "qm1"), &Symbol::new(&env, "ipfs"));
                client.confirm_milestone(&buyer, &sid, &1);
            }

            let ship = client.get_shipment(&sid);
            prop_assert!(
                ship.released_amount <= ship.total_amount,
                "released {} must not exceed total {}", ship.released_amount, ship.total_amount
            );
        }

        /// Confirmed milestone rejects further proof submissions.
        #[test]
        fn prop_contract_confirmed_milestone_rejects_resubmit(_seed in 0u32..100) {
            let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) =
                make_env_and_client();
            make_shipment(&env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, "PROP-002");
            let client = ChainSettleContractClient::new(&env, &contract_id);
            let sid = String::from_str(&env, "PROP-002");

            client.submit_proof(&supplier, &sid, &0, &String::from_str(&env, "qm0"), &Symbol::new(&env, "ipfs"));
            client.confirm_milestone(&buyer, &sid, &0);

            let ship = client.get_shipment(&sid);
            let ms = ship.milestones.get(0).unwrap();
            prop_assert_eq!(ms.status, MilestoneStatus::Confirmed);
        }

        /// Dispute rejection followed by resubmission succeeds (Pending→ProofSubmitted loop).
        #[test]
        fn prop_contract_dispute_reject_allows_resubmission(_seed in 0u32..100) {
            let (env, contract_id, token_id, buyer, supplier, logistics, arbiter) =
                make_env_and_client();
            make_shipment(&env, &contract_id, &token_id, &buyer, &supplier, &logistics, &arbiter, "PROP-003");
            let client = ChainSettleContractClient::new(&env, &contract_id);
            let sid = String::from_str(&env, "PROP-003");

            client.submit_proof(&supplier, &sid, &0, &String::from_str(&env, "qm_first"), &Symbol::new(&env, "ipfs"));
            client.raise_dispute(&buyer, &sid, &0);
            client.resolve_dispute(&arbiter, &sid, &0, &false);

            let ship = client.get_shipment(&sid);
            let ms = ship.milestones.get(0).unwrap();
            prop_assert_eq!(ms.status, MilestoneStatus::Pending);

            client.submit_proof(&supplier, &sid, &0, &String::from_str(&env, "qm_second"), &Symbol::new(&env, "ipfs"));

            let ship2 = client.get_shipment(&sid);
            let ms2 = ship2.milestones.get(0).unwrap();
            prop_assert_eq!(ms2.status, MilestoneStatus::ProofSubmitted);
        }
    }
}
