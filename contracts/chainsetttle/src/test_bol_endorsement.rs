#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, BytesN, String, Symbol};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[test]
fn bol_endorsement_tracks_holder_and_history() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let id = String::from_str(&s.env, "BOL-758");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &default_options(&s.env),
    );

    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    let bol_hash = BytesN::from_array(&s.env, &[7u8; 32]);
    client.attach_bol_hash(&s.supplier, &id, &0, &bol_hash);

    let holder2 = Address::generate(&s.env);
    let holder3 = Address::generate(&s.env);
    client.endorse_bol(&s.supplier, &id, &holder2);
    client.endorse_bol(&holder2, &id, &holder3);

    let history = client.get_bol_endorsements(&id);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0).unwrap().from, s.supplier);
    assert_eq!(history.get(0).unwrap().to, holder2);
    assert_eq!(history.get(1).unwrap().from, holder2);
    assert_eq!(history.get(1).unwrap().to, holder3);
}

#[test]
#[should_panic(expected = "only current BOL holder can endorse")]
fn only_current_holder_can_endorse_bol() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let id = String::from_str(&s.env, "BOL-758-AUTH");
    client.create_shipment(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &default_options(&s.env),
    );
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    client.attach_bol_hash(&s.supplier, &id, &0, &BytesN::from_array(&s.env, &[8u8; 32]));
    let holder2 = Address::generate(&s.env);
    let stranger = Address::generate(&s.env);
    client.endorse_bol(&s.supplier, &id, &holder2);
    client.endorse_bol(&stranger, &id, &Address::generate(&s.env));
}
