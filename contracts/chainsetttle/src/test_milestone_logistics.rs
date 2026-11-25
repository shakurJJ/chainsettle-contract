#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, String, Symbol};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[test]
fn per_milestone_logistics_routes_fees_and_index() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let tok = token::Client::new(&s.env, &s.token_id);
    let alt_logistics = Address::generate(&s.env);

    let mut milestones = build_milestones(&s.env);
    let mut m0 = milestones.get(0).unwrap();
    m0.logistics = Some(alt_logistics.clone());
    milestones.set(0, m0);

    let mut opts = default_options(&s.env);
    opts.logistics_fee_bps = 1000; // 10%

    let id = String::from_str(&s.env, "ML-756");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &milestones,
        &opts,
    );

    // per-milestone carrier is indexed
    let by_alt = client.get_shipments_by_logistics(&alt_logistics, &None, &10);
    assert_eq!(by_alt.len(), 1);

    // milestone 0 fee goes to alt_logistics
    let before_alt = tok.balance(&alt_logistics);
    client.submit_proof(&s.supplier, &id, &0, &String::from_str(&s.env, "p"), &Symbol::new(&s.env, "ipfs"));
    client.confirm_milestone(&s.buyer, &id, &0);
    let after_alt = tok.balance(&alt_logistics);
    assert!(after_alt > before_alt);

    // milestone 1 uses default shipment logistics
    let before_default = tok.balance(&s.logistics);
    client.submit_proof(&s.supplier, &id, &1, &String::from_str(&s.env, "p2"), &Symbol::new(&s.env, "ipfs"));
    client.confirm_milestone(&s.buyer, &id, &1);
    assert!(tok.balance(&s.logistics) > before_default);
}
