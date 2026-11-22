#![cfg(test)]

//! #744: Expedited dispute track for perishable shipments.

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    String, Symbol,
};

const AMOUNT: i128 = 1_000_000;

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn perishable() -> ShipmentOptionsExt {
    ShipmentOptionsExt {
        perishable: true,
        ..ShipmentOptionsExt::default()
    }
}

fn expedited_config() -> ExpeditedConfig {
    ExpeditedConfig {
        review_window_ledgers: 100,
        evidence_window_ledgers: 50,
        resolution_timeout_seconds: 3_600,
        fallback_resolution: Resolution::Buyer,
    }
}

fn create(
    t: &TestSetup,
    id: &str,
    arbiter: &Address,
    opts: ShipmentOptions,
    ext: ShipmentOptionsExt,
) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.create_shipment_ext(
        &sid(&t.env, id),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        arbiter,
        &t.token_id,
        &AMOUNT,
        &build_milestones(&t.env),
        &opts,
        &ext,
    )
}

fn submit_proof(t: &TestSetup, id: &String, idx: u32) {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.submit_proof(
        &t.supplier,
        id,
        &idx,
        &String::from_str(&t.env, "QmXyz123"),
        &Symbol::new(&t.env, "ipfs"),
    );
}

fn submit_evidence(t: &TestSetup, id: &String) -> bool {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client
        .try_submit_dispute_evidence(
            &t.buyer,
            id,
            &0,
            &String::from_str(&t.env, "QmEvidence"),
            &Symbol::new(&t.env, "photo"),
        )
        .is_ok()
}

// ── Configuration ───────────────────────────────────────────────────────────

#[test]
fn test_expedited_config_roundtrip_and_flags() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert_eq!(client.get_expedited_config(), None);

    client.set_expedited_config(&t.buyer, &expedited_config());
    assert_eq!(client.get_expedited_config(), Some(expedited_config()));

    let p = create(
        &t,
        "PER-1",
        &t.arbiter,
        default_options(&t.env),
        perishable(),
    );
    let n = create(
        &t,
        "STD-1",
        &t.arbiter,
        default_options(&t.env),
        ShipmentOptionsExt::default(),
    );
    assert!(client.is_shipment_perishable(&p));
    assert!(!client.is_shipment_perishable(&n));

    client.clear_expedited_config(&t.buyer);
    assert_eq!(client.get_expedited_config(), None);
}

#[test]
#[should_panic(expected = "expedited windows must be greater than zero")]
fn test_expedited_config_rejects_zero_window() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mut cfg = expedited_config();
    cfg.evidence_window_ledgers = 0;
    client.set_expedited_config(&t.buyer, &cfg);
}

#[test]
fn test_expedited_config_is_admin_only() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let outsider = Address::generate(&t.env);
    assert!(client
        .try_set_expedited_config(&outsider, &expedited_config())
        .is_err());
}

// ── Review window ───────────────────────────────────────────────────────────

#[test]
fn test_perishable_uses_shortened_review_window() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_expedited_config(&t.buyer, &expedited_config());

    let mut opts = default_options(&t.env);
    opts.auto_confirm_ledgers = 1_000;
    let p = create(&t, "PER-RV", &t.arbiter, opts.clone(), perishable());
    let n = create(
        &t,
        "STD-RV",
        &t.arbiter,
        opts,
        ShipmentOptionsExt::default(),
    );

    t.env.ledger().set_sequence_number(10);
    submit_proof(&t, &p, 0);
    submit_proof(&t, &n, 0);

    t.env.ledger().set_sequence_number(110);
    // Perishable: 100-ledger expedited window has elapsed.
    client.claim_auto_confirmation(&p, &0);
    assert_eq!(
        client.get_milestone(&p, &0).status,
        MilestoneStatus::Confirmed
    );
    // Standard: still inside its 1 000-ledger window.
    assert!(client.try_claim_auto_confirmation(&n, &0).is_err());
    assert_eq!(
        client.get_milestone(&n, &0).status,
        MilestoneStatus::ProofSubmitted
    );
}

#[test]
fn test_perishable_review_window_applies_when_standard_disabled() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_expedited_config(&t.buyer, &expedited_config());
    let p = create(
        &t,
        "PER-RV0",
        &t.arbiter,
        default_options(&t.env),
        perishable(),
    );
    let n = create(
        &t,
        "STD-RV0",
        &t.arbiter,
        default_options(&t.env),
        ShipmentOptionsExt::default(),
    );

    t.env.ledger().set_sequence_number(10);
    submit_proof(&t, &p, 0);
    submit_proof(&t, &n, 0);
    t.env.ledger().set_sequence_number(110);

    client.claim_auto_confirmation(&p, &0);
    assert_eq!(
        client.get_milestone(&p, &0).status,
        MilestoneStatus::Confirmed
    );
    // Standard shipment has no auto-confirmation at all.
    assert!(client.try_claim_auto_confirmation(&n, &0).is_err());
}

#[test]
fn test_perishable_respects_explicit_review_opt_out() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_expedited_config(&t.buyer, &expedited_config());
    let mut opts = default_options(&t.env);
    opts.review_window_ledgers = Some(0);
    let p = create(&t, "PER-OPT", &t.arbiter, opts, perishable());

    t.env.ledger().set_sequence_number(10);
    submit_proof(&t, &p, 0);
    t.env.ledger().set_sequence_number(500);
    assert!(client.try_claim_auto_confirmation(&p, &0).is_err());
}

#[test]
fn test_perishable_without_config_uses_standard_windows() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mut opts = default_options(&t.env);
    opts.auto_confirm_ledgers = 1_000;
    let p = create(&t, "PER-NOCFG", &t.arbiter, opts, perishable());

    t.env.ledger().set_sequence_number(10);
    submit_proof(&t, &p, 0);
    t.env.ledger().set_sequence_number(110);
    assert!(client.try_claim_auto_confirmation(&p, &0).is_err());
}

// ── Evidence window ─────────────────────────────────────────────────────────

#[test]
fn test_perishable_evidence_window_closes() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_expedited_config(&t.buyer, &expedited_config());
    let p = create(
        &t,
        "PER-EV",
        &t.arbiter,
        default_options(&t.env),
        perishable(),
    );
    let n = create(
        &t,
        "STD-EV",
        &t.arbiter,
        default_options(&t.env),
        ShipmentOptionsExt::default(),
    );

    t.env.ledger().set_sequence_number(10);
    submit_proof(&t, &p, 0);
    submit_proof(&t, &n, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    client.raise_dispute(&t.buyer, &n, &0);

    t.env.ledger().set_sequence_number(59);
    assert!(submit_evidence(&t, &p), "inside the window");

    t.env.ledger().set_sequence_number(60);
    assert!(
        !submit_evidence(&t, &p),
        "window is 50 ledgers from dispute open"
    );
    // Non-perishable disputes keep accepting evidence.
    assert!(submit_evidence(&t, &n));
    assert_eq!(client.get_dispute_evidence(&p, &0).len(), 1);
}

#[test]
#[should_panic(expected = "evidence window closed")]
fn test_perishable_late_evidence_panics_with_reason() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_expedited_config(&t.buyer, &expedited_config());
    let p = create(
        &t,
        "PER-EV2",
        &t.arbiter,
        default_options(&t.env),
        perishable(),
    );
    submit_proof(&t, &p, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    t.env.ledger().set_sequence_number(50);
    client.submit_dispute_evidence(
        &t.supplier,
        &p,
        &0,
        &String::from_str(&t.env, "QmLate"),
        &Symbol::new(&t.env, "photo"),
    );
}

// ── Resolution window & timeout fallback ────────────────────────────────────

#[test]
fn test_perishable_timeout_fallback_when_shipment_has_none() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = soroban_sdk::token::Client::new(&t.env, &t.token_id);
    client.set_expedited_config(&t.buyer, &expedited_config());
    let p = create(
        &t,
        "PER-TO",
        &t.arbiter,
        default_options(&t.env),
        perishable(),
    );
    let n = create(
        &t,
        "STD-TO",
        &t.arbiter,
        default_options(&t.env),
        ShipmentOptionsExt::default(),
    );

    t.env.ledger().set_timestamp(1_000);
    submit_proof(&t, &p, 0);
    submit_proof(&t, &n, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    client.raise_dispute(&t.buyer, &n, &0);

    t.env.ledger().set_timestamp(1_000 + 3_599);
    assert!(client.try_resolve_dispute_timeout(&p, &0).is_err());

    t.env.ledger().set_timestamp(1_000 + 3_600);
    let before = token.balance(&t.buyer);
    client.resolve_dispute_timeout(&p, &0);
    assert_eq!(
        client.get_milestone(&p, &0).status,
        MilestoneStatus::Resolved
    );
    // Fallback resolution is Buyer: the 25% milestone is refunded.
    assert_eq!(token.balance(&t.buyer) - before, AMOUNT / 4);

    // Non-perishable without its own timeout is unchanged: no auto-resolution.
    assert!(client.try_resolve_dispute_timeout(&n, &0).is_err());
    assert_eq!(
        client.get_milestone(&n, &0).status,
        MilestoneStatus::Disputed
    );
}

#[test]
fn test_perishable_shortens_shipment_timeout_keeping_its_resolution() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let token = soroban_sdk::token::Client::new(&t.env, &t.token_id);
    client.set_expedited_config(&t.buyer, &expedited_config());

    let mut opts = default_options(&t.env);
    opts.dispute_timeout_seconds = 86_400;
    opts.default_resolution = Resolution::Supplier;
    let p = create(&t, "PER-TO2", &t.arbiter, opts.clone(), perishable());
    let n = create(
        &t,
        "STD-TO2",
        &t.arbiter,
        opts,
        ShipmentOptionsExt::default(),
    );

    t.env.ledger().set_timestamp(1_000);
    submit_proof(&t, &p, 0);
    submit_proof(&t, &n, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    client.raise_dispute(&t.buyer, &n, &0);

    t.env.ledger().set_timestamp(1_000 + 3_600);
    client.resolve_dispute_timeout(&p, &0);
    // The shipment's own default (Supplier) still decides the outcome.
    assert_eq!(token.balance(&t.supplier), AMOUNT / 4);
    // The standard shipment still waits for its full day.
    assert!(client.try_resolve_dispute_timeout(&n, &0).is_err());

    t.env.ledger().set_timestamp(1_000 + 86_400);
    client.resolve_dispute_timeout(&n, &0);
    assert_eq!(
        client.get_milestone(&n, &0).status,
        MilestoneStatus::Resolved
    );
}

// ── Expedited arbiter routing ───────────────────────────────────────────────

#[test]
fn test_set_arbiter_expedited_flag() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    assert!(!client.is_arbiter_expedited(&t.arbiter));
    client.set_arbiter_expedited(&t.arbiter, &true);
    assert!(client.is_arbiter_expedited(&t.arbiter));
    client.set_arbiter_expedited(&t.arbiter, &false);
    assert!(!client.is_arbiter_expedited(&t.arbiter));
}

#[test]
fn test_perishable_dispute_routes_to_expedited_arbiter() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let standard_arb = Address::generate(&t.env);
    let expedited_arb = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &standard_arb);
    client.add_arbiter_to_pool(&t.buyer, &expedited_arb);
    client.set_arbiter_expedited(&expedited_arb, &true);

    // Passing the contract address as arbiter selects the pool.
    let p = create(
        &t,
        "PER-POOL",
        &t.contract_id,
        default_options(&t.env),
        perishable(),
    );
    submit_proof(&t, &p, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    assert_eq!(client.get_shipment(&p).arbiter, expedited_arb);
}

#[test]
fn test_standard_dispute_pool_assignment_unchanged() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let standard_arb = Address::generate(&t.env);
    let expedited_arb = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &standard_arb);
    client.add_arbiter_to_pool(&t.buyer, &expedited_arb);
    client.set_arbiter_expedited(&expedited_arb, &true);

    let n = create(
        &t,
        "STD-POOL",
        &t.contract_id,
        default_options(&t.env),
        ShipmentOptionsExt::default(),
    );
    submit_proof(&t, &n, 0);
    client.raise_dispute(&t.buyer, &n, &0);
    // Round-robin starts at pool index 0, regardless of the expedited flag.
    assert_eq!(client.get_shipment(&n).arbiter, standard_arb);
}

#[test]
fn test_perishable_falls_back_to_pool_without_expedited_arbiters() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let standard_arb = Address::generate(&t.env);
    client.add_arbiter_to_pool(&t.buyer, &standard_arb);

    let p = create(
        &t,
        "PER-FB",
        &t.contract_id,
        default_options(&t.env),
        perishable(),
    );
    submit_proof(&t, &p, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    assert_eq!(client.get_shipment(&p).arbiter, standard_arb);
}

#[test]
fn test_perishable_panel_prefers_expedited_arbiters() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let mut expedited = soroban_sdk::Vec::new(&t.env);
    for _ in 0..3 {
        let a = Address::generate(&t.env);
        client.add_arbiter_to_pool(&t.buyer, &a);
        client.set_arbiter_expedited(&a, &true);
        expedited.push_back(a);
    }
    for _ in 0..3 {
        client.add_arbiter_to_pool(&t.buyer, &Address::generate(&t.env));
    }

    let p = create(
        &t,
        "PER-PANEL",
        &t.arbiter,
        default_options(&t.env),
        perishable(),
    );
    submit_proof(&t, &p, 0);
    client.raise_dispute(&t.buyer, &p, &0);
    let panel = client.assign_dispute_panel(&t.buyer, &p, &0, &3);
    assert_eq!(panel.len(), 3);
    for member in panel.iter() {
        assert!(expedited.contains(&member));
    }
}
