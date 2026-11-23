#![cfg(test)]

//! #551 consortium shipments and #552 substitute suppliers: proof rights and
//! payout routing.

extern crate std;

use super::*;
use crate::test_common::{default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, String, Symbol,
};

const TOTAL: i128 = 200_000;
const DEADLINE: u32 = 100;

fn milestone(env: &Env, name: &str, deadline_ledger: u32) -> Milestone {
    Milestone {
        name: String::from_str(env, name),
        token: None,
        payment_percent: 50,
        proof_hash: String::from_str(env, ""),
        status: MilestoneStatus::Pending,
        release_after_ledger: 0,
        proof_submitted_ledger: None,
        dispute_opened_ledger: None,
        deadline_ledger,
        penalty_bps_per_ledger: 0,
    }

        logistics: None,
}

/// Two 50% milestones; milestone 0 has a deadline so it can become overdue.
fn create(t: &TestSetup, id: &str, milestone_suppliers: Vec<Address>) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let shipment_id = String::from_str(&t.env, id);
    let mut opts = default_options(&t.env);
    opts.milestone_suppliers = milestone_suppliers;
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &TOTAL,
        &vec![
            &t.env,
            milestone(&t.env, "M1", DEADLINE),
            milestone(&t.env, "M2", 0),
        ],
        &opts,
    );
    shipment_id
}

fn submit_and_confirm(t: &TestSetup, shipment_id: &String, submitter: &Address, index: u32) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.submit_proof(
        submitter,
        shipment_id,
        &index,
        &String::from_str(&t.env, "proof_hash_123"),
        &Symbol::new(&t.env, "ipfs"),
    );
    client.confirm_milestone(&t.buyer, shipment_id, &index);
}

fn balance(t: &TestSetup, who: &Address) -> i128 {
    token::Client::new(&t.env, &t.token_id).balance(who)
}

/// Makes milestone 0 overdue and installs `substitute` for it via the
/// buyer-proposes / arbiter-approves flow.
fn substitute_milestone_0(t: &TestSetup, shipment_id: &String, substitute: &Address) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    t.env.ledger().set_sequence_number(DEADLINE + 1);
    client.propose_substitute_supplier(&t.buyer, shipment_id, &0, substitute);
    client.approve_substitute_supplier(&t.arbiter, shipment_id, &0);
    assert_eq!(
        client.get_milestone_supplier(shipment_id, &0),
        Some(substitute.clone())
    );
}

// ── #551 consortium ────────────────────────────────────────────────────────

#[test]
fn test_consortium_members_paid_for_own_milestones_only() {
    let t = setup();
    let a = Address::generate(&t.env);
    let b = Address::generate(&t.env);
    let id = create(&t, "cons-pay", vec![&t.env, a.clone(), b.clone()]);

    submit_and_confirm(&t, &id, &a, 0);
    let a_paid = balance(&t, &a);
    assert!(a_paid > 0, "member A must be paid for milestone 0");
    assert_eq!(balance(&t, &b), 0, "member B must not be paid for milestone 0");
    assert_eq!(balance(&t, &t.supplier), 0, "primary supplier must not be paid");

    submit_and_confirm(&t, &id, &b, 1);
    assert!(balance(&t, &b) > 0, "member B must be paid for milestone 1");
    assert_eq!(balance(&t, &a), a_paid, "member A must not be paid for milestone 1");
    assert_eq!(balance(&t, &t.supplier), 0, "primary supplier must not be paid");
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_consortium_member_cannot_submit_other_members_milestone() {
    let t = setup();
    let a = Address::generate(&t.env);
    let b = Address::generate(&t.env);
    let id = create(&t, "cons-cross", vec![&t.env, a, b.clone()]);

    submit_and_confirm(&t, &id, &b, 0);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_consortium_primary_supplier_cannot_submit_member_milestone() {
    let t = setup();
    let a = Address::generate(&t.env);
    let b = Address::generate(&t.env);
    let id = create(&t, "cons-primary", vec![&t.env, a, b]);
    let supplier = t.supplier.clone();

    submit_and_confirm(&t, &id, &supplier, 0);
}

// ── #552 substitute supplier ───────────────────────────────────────────────

#[test]
fn test_substitute_paid_and_original_supplier_not() {
    let t = setup();
    let substitute = Address::generate(&t.env);
    let id = create(&t, "sub-pay", Vec::new(&t.env));

    substitute_milestone_0(&t, &id, &substitute);
    submit_and_confirm(&t, &id, &substitute, 0);

    let sub_paid = balance(&t, &substitute);
    assert!(sub_paid > 0, "substitute must be paid for milestone 0");
    assert_eq!(balance(&t, &t.supplier), 0, "original supplier must not be paid for milestone 0");

    // The substitution is scoped to milestone 0: the original supplier keeps
    // milestone 1 and the substitute earns nothing more.
    let supplier = t.supplier.clone();
    submit_and_confirm(&t, &id, &supplier, 1);
    assert!(balance(&t, &t.supplier) > 0, "original supplier must be paid for milestone 1");
    assert_eq!(balance(&t, &substitute), sub_paid);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_original_supplier_cannot_submit_substituted_milestone() {
    let t = setup();
    let substitute = Address::generate(&t.env);
    let id = create(&t, "sub-orig", Vec::new(&t.env));
    let supplier = t.supplier.clone();

    substitute_milestone_0(&t, &id, &substitute);
    submit_and_confirm(&t, &id, &supplier, 0);
}

#[test]
fn test_substitute_replaces_consortium_member() {
    let t = setup();
    let a = Address::generate(&t.env);
    let b = Address::generate(&t.env);
    let substitute = Address::generate(&t.env);
    let id = create(&t, "cons-sub", vec![&t.env, a.clone(), b.clone()]);

    substitute_milestone_0(&t, &id, &substitute);
    submit_and_confirm(&t, &id, &substitute, 0);
    submit_and_confirm(&t, &id, &b, 1);

    assert!(balance(&t, &substitute) > 0, "substitute must be paid for milestone 0");
    assert!(balance(&t, &b) > 0, "member B must be paid for milestone 1");
    assert_eq!(balance(&t, &a), 0, "replaced member A must not be paid");
    assert_eq!(balance(&t, &t.supplier), 0, "primary supplier must not be paid");
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_replaced_consortium_member_cannot_submit() {
    let t = setup();
    let a = Address::generate(&t.env);
    let b = Address::generate(&t.env);
    let substitute = Address::generate(&t.env);
    let id = create(&t, "cons-sub-a", vec![&t.env, a.clone(), b]);

    substitute_milestone_0(&t, &id, &substitute);
    submit_and_confirm(&t, &id, &a, 0);
}
