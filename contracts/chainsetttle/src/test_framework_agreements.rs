#![cfg(test)]

//! #786 – Framework agreements with call-off shipments.

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, TestSetup};
use soroban_sdk::{testutils::Ledger as _, String};

fn terms(t: &TestSetup) -> FrameworkTerms {
    FrameworkTerms {
        token: t.token_id.clone(),
        logistics: t.logistics.clone(),
        arbiter: t.arbiter.clone(),
        milestones: build_milestones(&t.env),
        options: default_options(&t.env),
    }
}

fn call_off(t: &TestSetup, id: &str, amount: i128) -> CallOffParams {
    CallOffParams {
        shipment_id: String::from_str(&t.env, id),
        amount,
    }
}

#[test]
fn call_offs_inherit_terms_and_track_cap() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    t.env.ledger().set_sequence_number(10);

    let fid = client.create_framework(&t.buyer, &t.supplier, &10_000, &terms(&t), &1_000);
    assert_eq!(client.get_framework_remaining_cap(&fid), 10_000);

    let sid = client.create_call_off(&fid, &call_off(&t, "CO-1", 4_000));
    let shipment = client.get_shipment(&sid);
    assert_eq!(shipment.supplier, t.supplier);
    assert_eq!(shipment.logistics, t.logistics);
    assert_eq!(shipment.arbiter, t.arbiter);
    assert_eq!(shipment.token, t.token_id);
    assert_eq!(shipment.total_amount, 4_000);
    assert_eq!(shipment.milestones.len(), 3);

    assert_eq!(client.get_framework_remaining_cap(&fid), 6_000);
    assert_eq!(client.get_framework(&fid).used, 4_000);
    assert_eq!(client.get_framework_call_offs(&fid).len(), 1);

    // Filling the cap exactly is allowed.
    client.create_call_off(&fid, &call_off(&t, "CO-2", 6_000));
    assert_eq!(client.get_framework_remaining_cap(&fid), 0);
}

#[test]
#[should_panic(expected = "call-off exceeds framework cap")]
fn call_off_cannot_exceed_cap() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    t.env.ledger().set_sequence_number(10);

    let fid = client.create_framework(&t.buyer, &t.supplier, &10_000, &terms(&t), &1_000);
    client.create_call_off(&fid, &call_off(&t, "CO-3", 7_000));
    client.create_call_off(&fid, &call_off(&t, "CO-4", 3_001));
}

#[test]
#[should_panic(expected = "framework expired")]
fn expired_framework_blocks_call_offs() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    t.env.ledger().set_sequence_number(10);

    let fid = client.create_framework(&t.buyer, &t.supplier, &10_000, &terms(&t), &100);
    t.env.ledger().set_sequence_number(100);
    assert_eq!(client.get_framework_remaining_cap(&fid), 0);
    client.create_call_off(&fid, &call_off(&t, "CO-5", 1_000));
}

#[test]
#[should_panic(expected = "expires_at must be in the future")]
fn framework_requires_future_expiry() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    t.env.ledger().set_sequence_number(10);
    client.create_framework(&t.buyer, &t.supplier, &10_000, &terms(&t), &10);
}
