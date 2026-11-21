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
