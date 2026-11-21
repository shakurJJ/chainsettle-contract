#![cfg(test)]

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, TestSetup};
use soroban_sdk::{
    testutils::Address as _, token, vec, Address, String, Symbol,
};

const TOTAL: i128 = 1_000_000_000;

fn client(t: &TestSetup) -> ChainSettleContractClient<'_> {
    ChainSettleContractClient::new(&t.env, &t.contract_id)
}

fn setup_org_with_member(role: OrgRole, spend_cap: i128) -> (TestSetup, Address, Address) {
    let t = setup();
    let c = client(&t);
    let org = Address::generate(&t.env);
    let member = Address::generate(&t.env);
    token::StellarAssetClient::new(&t.env, &t.token_id).mint(&org, &10_000_000_000);
    c.create_organization(&org);
    c.add_org_member(&org, &member, &role, &spend_cap);
    (t, org, member)
}

fn create_org_shipment(
    t: &TestSetup,
    org: &Address,
    member: &Address,
    id: &str,
    amount: i128,
) {
    client(t).create_organization_shipment(
        org,
        member,
        &String::from_str(&t.env, id),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &amount,
        &build_milestones(&t.env),
        &default_options(&t.env),
    );
}

#[test]
fn organization_shipment_records_org_buyer_and_acting_member() {
    let (t, org, member) = setup_org_with_member(OrgRole::Creator, 2_000_000_000);
    let c = client(&t);
    let shipment_id = String::from_str(&t.env, "ORG-SHIP-1");

    create_org_shipment(&t, &org, &member, "ORG-SHIP-1", TOTAL);

    assert!(c.is_organization(&org));
    assert_eq!(c.get_org_member(&org, &member).unwrap().role, OrgRole::Creator);
    assert_eq!(c.get_org_shipment(&shipment_id), Some(org.clone()));
    assert_eq!(c.get_org_shipment_member(&shipment_id), Some(member.clone()));

    let shipment = c.get_shipment(&shipment_id);
    assert_eq!(shipment.buyers, vec![&t.env, org.clone()]);
    let audit = shipment.audit_log.get(shipment.audit_log.len() - 1).unwrap();
    assert_eq!(audit.caller, member);
    assert_eq!(
        audit.action,
        Symbol::new(&t.env, "organization_shipment_created")
    );
}

#[test]
#[should_panic(expected = "not an organization member")]
fn removed_member_loses_access_immediately() {
    let (t, org, member) = setup_org_with_member(OrgRole::Creator, 2_000_000_000);
    client(&t).remove_org_member(&org, &member);
    assert!(client(&t).get_org_member(&org, &member).is_none());
    create_org_shipment(&t, &org, &member, "ORG-SHIP-REMOVED", TOTAL);
}

#[test]
#[should_panic(expected = "spend cap exceeded")]
fn creator_cannot_exceed_spend_cap() {
    let (t, org, member) = setup_org_with_member(OrgRole::Creator, TOTAL - 1);
    create_org_shipment(&t, &org, &member, "ORG-SHIP-CAP", TOTAL);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn confirmer_cannot_create_shipments() {
    let (t, org, member) = setup_org_with_member(OrgRole::Confirmer, 2_000_000_000);
    create_org_shipment(&t, &org, &member, "ORG-SHIP-ROLE", TOTAL);
}
