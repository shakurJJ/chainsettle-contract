#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{
    build_milestones, capture_events, default_options, has_event_topic, setup,
    single_buyer_vec, submit_proof_for_milestone, TestSetup,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    String,
};

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

#[test]
fn auto_confirm_warning_fires_once_inside_lead_window() {
    let t = setup();
    let c = client(&t);
    t.env.ledger().set_sequence_number(1);
    c.set_warning_lead_ledgers(&t.buyer, &5);

    let shipment_id = String::from_str(&t.env, "AUTO-WARN-1");
    let mut options = default_options(&t.env);
    options.auto_confirm_ledgers = 10;
    c.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    submit_proof_for_milestone(&t, &shipment_id, 0);

    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));

    t.env.ledger().set_sequence_number(6);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(has_event_topic(&t.env, &events, "auto_conf_warn"));

    t.env.ledger().set_sequence_number(7);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));
}

#[test]
fn auto_confirm_warning_does_not_fire_when_disabled() {
    let t = setup();
    let c = client(&t);
    t.env.ledger().set_sequence_number(1);
    // No warning lead ledgers set, so warnings are disabled

    let shipment_id = String::from_str(&t.env, "AUTO-WARN-DISABLED");
    let mut options = default_options(&t.env);
    options.auto_confirm_ledgers = 10;
    c.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    submit_proof_for_milestone(&t, &shipment_id, 0);

    // Even if we're in what would be the warning window, no warning should fire
    t.env.ledger().set_sequence_number(6);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));

    t.env.ledger().set_sequence_number(10);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));
}

#[test]
fn auto_confirm_warning_does_not_fire_outside_window() {
    let t = setup();
    let c = client(&t);
    t.env.ledger().set_sequence_number(1);
    c.set_warning_lead_ledgers(&t.buyer, &5);

    let shipment_id = String::from_str(&t.env, "AUTO-WARN-OUTSIDE");
    let mut options = default_options(&t.env);
    options.auto_confirm_ledgers = 20;
    c.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    submit_proof_for_milestone(&t, &shipment_id, 0);

    // Too early - before warning window
    t.env.ledger().set_sequence_number(10);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));

    // After warning window (past auto-confirm)
    t.env.ledger().set_sequence_number(22);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));
}

#[test]
fn auto_confirm_warning_fires_at_exact_boundary() {
    let t = setup();
    let c = client(&t);
    t.env.ledger().set_sequence_number(1);
    c.set_warning_lead_ledgers(&t.buyer, &3);

    let shipment_id = String::from_str(&t.env, "AUTO-WARN-BOUNDARY");
    let mut options = default_options(&t.env);
    options.auto_confirm_ledgers = 10;
    c.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    submit_proof_for_milestone(&t, &shipment_id, 0);

    // Test at exact start of warning window (auto_confirm - warning_lead)
    // proof_submitted at ledger 1, auto_confirm at 11, warning_lead = 3
    // Warning window should start at ledger 8 (11 - 3)
    t.env.ledger().set_sequence_number(8);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(has_event_topic(&t.env, &events, "auto_conf_warn"));

    // Should not fire again at same ledger
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));
}

#[test]
fn auto_confirm_warning_works_for_multiple_milestones() {
    let t = setup();
    let c = client(&t);
    t.env.ledger().set_sequence_number(1);
    c.set_warning_lead_ledgers(&t.buyer, &4);

    let shipment_id = String::from_str(&t.env, "AUTO-WARN-MULTI");
    let mut options = default_options(&t.env);
    options.auto_confirm_ledgers = 10;
    c.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    
    // Submit proof for first milestone
    submit_proof_for_milestone(&t, &shipment_id, 0);

    // Check warning for milestone 0
    t.env.ledger().set_sequence_number(7); // 1 + 10 - 4 = 7
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(has_event_topic(&t.env, &events, "auto_conf_warn"));

    // Submit proof for second milestone at a different time
    t.env.ledger().set_sequence_number(5);
    submit_proof_for_milestone(&t, &shipment_id, 1);

    // Check warning for milestone 1
    t.env.ledger().set_sequence_number(11); // 5 + 10 - 4 = 11
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &1);
    });
    assert!(has_event_topic(&t.env, &events, "auto_conf_warn"));

    // Verify milestone 0 doesn't fire again
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));
}

#[test]
fn auto_confirm_warning_respects_different_warning_lead_values() {
    let t = setup();
    let c = client(&t);
    t.env.ledger().set_sequence_number(1);
    
    // Test with a large warning lead window
    c.set_warning_lead_ledgers(&t.buyer, &8);

    let shipment_id = String::from_str(&t.env, "AUTO-WARN-LARGE-LEAD");
    let mut options = default_options(&t.env);
    options.auto_confirm_ledgers = 15;
    c.create_shipment(
        &shipment_id,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    submit_proof_for_milestone(&t, &shipment_id, 0);

    // Warning window should start at ledger 8 (1 + 15 - 8)
    t.env.ledger().set_sequence_number(7);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(!has_event_topic(&t.env, &events, "auto_conf_warn"));

    t.env.ledger().set_sequence_number(8);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id, &0);
    });
    assert!(has_event_topic(&t.env, &events, "auto_conf_warn"));

    // Update warning lead to smaller value for new shipment
    c.set_warning_lead_ledgers(&t.buyer, &2);
    
    let shipment_id2 = String::from_str(&t.env, "AUTO-WARN-SMALL-LEAD");
    t.env.ledger().set_sequence_number(10);
    c.create_shipment(
        &shipment_id2,
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &1_000_000_000,
        &build_milestones(&t.env),
        &options,
    );
    submit_proof_for_milestone(&t, &shipment_id2, 0);

    // Warning window should start at ledger 23 (10 + 15 - 2)
    t.env.ledger().set_sequence_number(23);
    let events = capture_events(&t.env, || {
        c.check_auto_confirm_warning(&shipment_id2, &0);
    });
    assert!(has_event_topic(&t.env, &events, "auto_conf_warn"));
}
