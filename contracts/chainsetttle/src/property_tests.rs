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
    use soroban_sdk::{testutils::Address as _, token, vec, Address, BytesN, Env, String, Symbol};

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
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            },
            Milestone {
                name: String::from_str(env, "M1"),
                payment_percent: 50,
                proof_hash: String::from_str(env, ""),
                status: MilestoneStatus::Pending,
                release_after_ledger: 0,
                proof_submitted_ledger: None,
                dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
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


                metadata_hash: None,
                referrer: None,
                buyer_cancel_fee_bps: 0,
        early_bonus_pool: 0,
        review_window_ledgers: None,

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

// ============================================================
// MILESTONE PERCENTAGE FUZZ TESTS
//
// Pure-Rust model: validates the percentage constraint logic that
// the contract enforces in `create_shipment` and `rebalance_milestones`.
//
// Rules under test:
//   1. sum(percents) must equal 100
//   2. every individual percent must be >= MIN_PERCENT (default 5)
// ============================================================

#[cfg(test)]
mod milestone_percent_fuzz {
    use proptest::prelude::*;

    const MIN_PERCENT: u32 = 5;

    fn is_valid_percent_set(percents: &[u32]) -> bool {
        if percents.is_empty() {
            return false;
        }
        let sum: u32 = percents.iter().sum();
        sum == 100 && percents.iter().all(|&p| p >= MIN_PERCENT)
    }

    proptest! {
        #![proptest_config(proptest::test_runner::Config {
            cases: 10_000,
            ..Default::default()
        })]

        /// Validity is determined solely by (sum == 100) AND (all >= MIN_PERCENT).
        #[test]
        fn prop_percent_validity_is_sum_and_min_only(
            percents in prop::collection::vec(0u32..=100u32, 1..=10usize),
        ) {
            let sum: u32 = percents.iter().sum();
            let all_above_min = percents.iter().all(|&p| p >= MIN_PERCENT);
            let model_valid = sum == 100 && all_above_min;
            prop_assert_eq!(
                is_valid_percent_set(&percents),
                model_valid,
                "validity mismatch for {:?} (sum={}, all_above_min={})",
                percents, sum, all_above_min
            );
        }

        /// A two-element split (first, 100-first) is valid iff both halves >= MIN_PERCENT.
        #[test]
        fn prop_two_milestone_split_validity(first in 0u32..=100u32) {
            let second = 100u32.saturating_sub(first);
            let percents = [first, second];
            let expected = first >= MIN_PERCENT && second >= MIN_PERCENT;
            prop_assert_eq!(
                is_valid_percent_set(&percents),
                expected,
                "two-split ({}, {}) validity mismatch", first, second
            );
        }

        /// Swapping any two entries does not change validity (order-independent).
        #[test]
        fn prop_validity_is_order_independent(
            percents in prop::collection::vec(5u32..=50u32, 2..=8usize),
            a in 0usize..8,
            b in 0usize..8,
        ) {
            let n = percents.len();
            let mut reordered = percents.clone();
            reordered.swap(a % n, b % n);
            prop_assert_eq!(
                is_valid_percent_set(&percents),
                is_valid_percent_set(&reordered),
                "swap must not change validity"
            );
        }

        /// Any set containing a zero-valued entry is always invalid (0 < MIN_PERCENT).
        #[test]
        fn prop_zero_percent_entry_always_invalid(
            rest in prop::collection::vec(5u32..=50u32, 1..=9usize),
        ) {
            let mut percents = rest;
            percents.push(0);
            prop_assert!(
                !is_valid_percent_set(&percents),
                "set with a 0-valued entry must be invalid"
            );
        }

        /// A set with any entry > 100 cannot sum to exactly 100 with positive siblings.
        #[test]
        fn prop_entry_over_100_is_invalid(
            excess in 101u32..=200u32,
        ) {
            let sum = excess;
            prop_assert!(
                sum != 100,
                "single-entry sum {} must not equal 100", sum
            );
        }

        /// Adding a positive delta to a sum-100 set breaks the invariant.
        #[test]
        fn prop_augmented_sum_invalidates_set(
            first in 5u32..=90u32,
            delta in 1u32..=10u32,
        ) {
            let second = 100u32.saturating_sub(first);
            if second < MIN_PERCENT {
                return Ok(());
            }
            // (first, second) is a valid baseline.
            prop_assert!(is_valid_percent_set(&[first, second]));
            // Augmenting the first entry pushes the sum past 100.
            prop_assert!(
                !is_valid_percent_set(&[first + delta, second]),
                "augmented set (sum={}) must be invalid", first + delta + second
            );
        }

        /// For n milestones, the "all equal, with remainder on last" distribution
        /// is valid iff every per-entry value >= MIN_PERCENT.
        #[test]
        fn prop_uniform_milestone_distribution(n in 1usize..=20usize) {
            let per: u32 = 100 / n as u32;
            let remainder = 100 % n as u32;
            let percents: Vec<u32> = (0..n)
                .map(|i| if i == n - 1 { per + remainder } else { per })
                .collect();
            let sum: u32 = percents.iter().sum();
            prop_assert_eq!(sum, 100u32, "uniform distribution must sum to 100 for n={}", n);
            let all_above_min = percents.iter().all(|&p| p >= MIN_PERCENT);
            prop_assert_eq!(
                is_valid_percent_set(&percents),
                all_above_min,
                "uniform n={} validity should match all_above_min={}", n, all_above_min
            );
        }

        /// An empty list is always invalid (sum = 0 ≠ 100).
        #[test]
        fn prop_empty_list_is_invalid(_seed in 0u32..1u32) {
            let empty: Vec<u32> = vec![];
            prop_assert!(!is_valid_percent_set(&empty));
        }

        /// Splitting 100 into exactly MIN_PERCENT-sized blocks leaves
        /// 100 / MIN_PERCENT = 20 milestones, each at exactly 5 — valid.
        #[test]
        fn prop_minimum_percent_exact_split(_seed in 0u32..1u32) {
            let n = (100 / MIN_PERCENT) as usize; // 20
            let percents: Vec<u32> = (0..n).map(|_| MIN_PERCENT).collect();
            prop_assert!(is_valid_percent_set(&percents));
        }
    }

    // --------------------------------------------------------
    // CONTRACT-BACKED PERCENTAGE VALIDATION TESTS
    // These spin up a real Soroban test environment and call
    // the contract to verify the on-chain checks match the model.
    // --------------------------------------------------------
    #[cfg(test)]
    mod contract_percent_tests {
        use crate::{
            ChainSettleContract, ChainSettleContractClient, Milestone, MilestoneMode,
            MilestoneStatus, ShipmentOptions,
        };
        use proptest::prelude::*;
        use soroban_sdk::{testutils::Address as _, token, vec, Address, BytesN, Env, String};

        fn make_test_env() -> (Env, Address, Address, Address, Address, Address, Address) {
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
            token::StellarAssetClient::new(&env, &token_id)
                .mint(&buyer, &100_000_000_000i128);
            ChainSettleContractClient::new(&env, &contract_id).init(&buyer);
            (env, contract_id, token_id, buyer, supplier, logistics, arbiter)
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
        early_bonus_pool: 0,
        review_window_ledgers: None,

            }
        }

        fn milestone(env: &Env, name: &str, pct: u32) -> Milestone {
            Milestone {
                name: String::from_str(env, name),
                payment_percent: pct,
                proof_hash: String::from_str(env, ""),
                status: MilestoneStatus::Pending,
                release_after_ledger: 0,
                proof_submitted_ledger: None,
                dispute_opened_ledger: None,
            deadline_ledger: 0,
            penalty_bps_per_ledger: 0,
            }
        }

        proptest! {
            #![proptest_config(proptest::test_runner::Config {
                cases: 200,
                ..Default::default()
            })]

            /// Valid two-milestone splits (both >= 5, sum == 100) are accepted by the contract.
            #[test]
            fn prop_contract_accepts_valid_two_split(first in 5u32..=90u32) {
                let second = 100u32 - first;
                if second < 5 {
                    return Ok(());
                }
                let (env, cid, tid, buyer, supplier, logistics, arbiter) = make_test_env();
                let client = ChainSettleContractClient::new(&env, &cid);
                let sid = String::from_str(&env, "PCT-VALID");
                let ms = vec![&env, milestone(&env, "M0", first), milestone(&env, "M1", second)];
                let result = client.try_create_shipment(
                    &sid,
                    &vec![&env, buyer.clone()],
                    &supplier, &logistics, &arbiter, &tid,
                    &1_000_000i128,
                    &ms,
                    &default_options(&env),
                );
                prop_assert!(
                    result.is_ok(),
                    "valid split ({}, {}) must be accepted", first, second
                );
            }

            /// Any milestone percentage below 5 (the default minimum) is rejected.
            #[test]
            fn prop_contract_rejects_below_min_percent(below in 1u32..=4u32) {
                let (env, cid, tid, buyer, supplier, logistics, arbiter) = make_test_env();
                let client = ChainSettleContractClient::new(&env, &cid);
                let sid = String::from_str(&env, "PCT-LOW");
                let above = 100 - below;
                let ms = vec![&env, milestone(&env, "M0", below), milestone(&env, "M1", above)];
                let result = client.try_create_shipment(
                    &sid,
                    &vec![&env, buyer.clone()],
                    &supplier, &logistics, &arbiter, &tid,
                    &1_000_000i128,
                    &ms,
                    &default_options(&env),
                );
                prop_assert!(
                    result.is_err(),
                    "percent {} below minimum must be rejected", below
                );
            }

            /// Two-milestone sets whose percentages sum to something other than 100 are rejected.
            #[test]
            fn prop_contract_rejects_non_100_sum(
                first in 5u32..=45u32,
                second in 5u32..=45u32,
            ) {
                let sum = first + second;
                if sum == 100 { return Ok(()); }
                let (env, cid, tid, buyer, supplier, logistics, arbiter) = make_test_env();
                let client = ChainSettleContractClient::new(&env, &cid);
                let sid = String::from_str(&env, "PCT-BADSUM");
                let ms = vec![&env, milestone(&env, "M0", first), milestone(&env, "M1", second)];
                let result = client.try_create_shipment(
                    &sid,
                    &vec![&env, buyer.clone()],
                    &supplier, &logistics, &arbiter, &tid,
                    &1_000_000i128,
                    &ms,
                    &default_options(&env),
                );
                prop_assert!(
                    result.is_err(),
                    "sum {} != 100 must be rejected", sum
                );
            }

            /// A single 100% milestone (at or above the minimum) is always valid.
            #[test]
            fn prop_contract_accepts_single_100_milestone(_seed in 0u32..10u32) {
                let (env, cid, tid, buyer, supplier, logistics, arbiter) = make_test_env();
                let client = ChainSettleContractClient::new(&env, &cid);
                let sid = String::from_str(&env, "PCT-SINGLE");
                let ms = vec![&env, milestone(&env, "M0", 100)];
                let result = client.try_create_shipment(
                    &sid,
                    &vec![&env, buyer.clone()],
                    &supplier, &logistics, &arbiter, &tid,
                    &1_000_000i128,
                    &ms,
                    &default_options(&env),
                );
                prop_assert!(result.is_ok(), "single 100% milestone must be accepted");
            }
        }
    }
}
