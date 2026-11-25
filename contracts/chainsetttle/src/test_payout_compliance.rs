#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, testutils::Address as _, token, Address, Env, String, Symbol};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[contract]
struct MockComplianceRegistry;

#[contractimpl]
impl MockComplianceRegistry {
    pub fn set_flagged(env: Env, party: Address, flagged: bool) {
        env.storage().persistent().set(&party, &flagged);
    }

    pub fn is_flagged(env: Env, party: Address) -> bool {
        env.storage().persistent().get(&party).unwrap_or(false)
    }
}

#[test]
fn flagged_party_blocks_payout_and_sets_hold() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let registry_id = s.env.register(MockComplianceRegistry, ());
    let registry = MockComplianceRegistryClient::new(&s.env, &registry_id);

    client.set_payout_compliance_registry(&s.admin, &registry_id);
    client.set_payout_compliance_enabled(&s.admin, &true);

    let id = String::from_str(&s.env, "COMP-757");
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

    client.submit_proof(&s.supplier, &id, &0, &String::from_str(&s.env, "proof"), &Symbol::new(&s.env, "ipfs"));
    registry.set_flagged(&s.supplier, &true);

    let supplier_before = token::Client::new(&s.env, &s.token_id).balance(&s.supplier);
    let res = client.try_confirm_milestone(&s.buyer, &id, &0);
    assert!(res.is_err());
    let supplier_after = token::Client::new(&s.env, &s.token_id).balance(&s.supplier);
    assert_eq!(supplier_after, supplier_before);
    assert!(client.is_on_compliance_hold(&id));
}

#[test]
fn unflagged_payout_unaffected() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let registry_id = s.env.register(MockComplianceRegistry, ());

    client.set_payout_compliance_registry(&s.admin, &registry_id);
    client.set_payout_compliance_enabled(&s.admin, &true);

    let id = String::from_str(&s.env, "COMP-757-OK");
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

    client.submit_proof(&s.supplier, &id, &0, &String::from_str(&s.env, "proof"), &Symbol::new(&s.env, "ipfs"));
    let before = token::Client::new(&s.env, &s.token_id).balance(&s.supplier);
    client.confirm_milestone(&s.buyer, &id, &0);
    assert!(token::Client::new(&s.env, &s.token_id).balance(&s.supplier) > before);
}
