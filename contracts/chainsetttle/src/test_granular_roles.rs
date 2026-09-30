#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, String, Vec,
};
use crate::test_common::*;

#[test]
fn test_grant_and_has_role_basic() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);
    let fee_mgr = Address::generate(&t.env);
    let compliance = Address::generate(&t.env);
    let arbiter_mgr = Address::generate(&t.env);

    // Initial state: no explicit roles
    assert!(!client.has_role(&Role::Pauser, &pauser));
    assert!(!client.has_role(&Role::FeeManager, &fee_mgr));
    assert!(!client.has_role(&Role::ComplianceOfficer, &compliance));
    assert!(!client.has_role(&Role::ArbiterManager, &arbiter_mgr));

    // Admin implicitly has all roles
    assert!(client.has_role(&Role::Pauser, &t.buyer));
    assert!(client.has_role(&Role::FeeManager, &t.buyer));
    assert!(client.has_role(&Role::ComplianceOfficer, &t.buyer));
    assert!(client.has_role(&Role::ArbiterManager, &t.buyer));

    // Grant roles
    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    client.grant_role(&t.buyer, &Role::FeeManager, &fee_mgr);
    client.grant_role(&t.buyer, &Role::ComplianceOfficer, &compliance);
    client.grant_role(&t.buyer, &Role::ArbiterManager, &arbiter_mgr);

    // Verify roles
    assert!(client.has_role(&Role::Pauser, &pauser));
    assert!(client.has_role(&Role::FeeManager, &fee_mgr));
    assert!(client.has_role(&Role::ComplianceOfficer, &compliance));
    assert!(client.has_role(&Role::ArbiterManager, &arbiter_mgr));

    // Scoped boundaries: pauser doesn't have other roles
    assert!(!client.has_role(&Role::FeeManager, &pauser));
    assert!(!client.has_role(&Role::ComplianceOfficer, &pauser));
    assert!(!client.has_role(&Role::ArbiterManager, &pauser));
}

#[test]
fn test_revoke_role_takes_effect_immediately() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    assert!(client.has_role(&Role::Pauser, &pauser));

    // Pauser can pause
    client.pause(&pauser);
    assert!(client.is_paused());
    client.unpause(&pauser);
    assert!(!client.is_paused());

    // Revoke
    client.revoke_role(&t.buyer, &Role::Pauser, &pauser);
    assert!(!client.has_role(&Role::Pauser, &pauser));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_revoked_role_cannot_call_scoped_function() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    client.revoke_role(&t.buyer, &Role::Pauser, &pauser);

    // Should fail with unauthorized
    client.pause(&pauser);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_grant_role() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let attacker = Address::generate(&t.env);
    let target = Address::generate(&t.env);

    client.grant_role(&attacker, &Role::Pauser, &target);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_revoke_role() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);
    let attacker = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    client.revoke_role(&attacker, &Role::Pauser, &pauser);
}

#[test]
fn test_pauser_can_pause_and_unpause() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);

    client.pause(&pauser);
    assert!(client.is_paused());

    client.unpause(&pauser);
    assert!(!client.is_paused());
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_pauser_cannot_set_fee_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    client.set_fee_config(&pauser, &100, &t.treasury);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_pauser_cannot_blacklist_address() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);
    let bad_actor = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    client.blacklist_address(&pauser, &bad_actor, &BytesN::from_array(&t.env, &[1u8; 32]));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_pauser_cannot_add_arbiter_to_pool() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let pauser = Address::generate(&t.env);
    let new_arbiter = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &pauser);
    client.add_arbiter_to_pool(&pauser, &new_arbiter);
}

#[test]
fn test_fee_manager_can_manage_fees() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let fee_mgr = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::FeeManager, &fee_mgr);

    // set_fee_config
    client.set_fee_config(&fee_mgr, &250, &t.treasury);
    assert_eq!(client.get_fee_config().fee_bps, 250);

    // schedule & cancel fee holiday
    client.schedule_fee_holiday(&fee_mgr, &100, &200);
    client.cancel_fee_holiday(&fee_mgr);

    // set_fee_recipients
    let recipients = vec![
        &t.env,
        FeeRecipient {
            recipient: t.treasury.clone(),
            share_bps: 10_000,
        },
    ];
    client.set_fee_recipients(&fee_mgr, &recipients);

    // set_referral_fee_bps
    client.set_referral_fee_bps(&fee_mgr, &300);
    assert_eq!(client.get_referral_fee_bps(), 300);

    // set_fee_tiers
    let tiers = vec![
        &t.env,
        FeeTier {
            min_lifetime_volume: 1_000_000,
            fee_bps: 50,
        },
    ];
    client.set_fee_tiers(&fee_mgr, &tiers);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_fee_manager_cannot_pause() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let fee_mgr = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::FeeManager, &fee_mgr);
    client.pause(&fee_mgr);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_fee_manager_cannot_blacklist_address() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let fee_mgr = Address::generate(&t.env);
    let bad_actor = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::FeeManager, &fee_mgr);
    client.blacklist_address(&fee_mgr, &bad_actor, &BytesN::from_array(&t.env, &[1u8; 32]));
}

#[test]
fn test_compliance_officer_can_manage_compliance() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let compliance = Address::generate(&t.env);
    let bad_actor = Address::generate(&t.env);
    let supplier = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ComplianceOfficer, &compliance);

    // Blacklist and unblacklist
    client.blacklist_address(&compliance, &bad_actor, &BytesN::from_array(&t.env, &[2u8; 32]));
    assert!(client.is_blacklisted(&bad_actor));
    client.remove_from_blacklist(&compliance, &bad_actor);
    assert!(!client.is_blacklisted(&bad_actor));

    // Whitelist and remove
    client.add_to_whitelist(&compliance, &supplier);
    client.remove_from_whitelist(&compliance, &supplier);

    // Mutual pre-approval
    client.set_require_mutual_preapproval(&compliance, &true);
    assert!(client.get_require_mutual_preapproval());
    client.set_require_mutual_preapproval(&compliance, &false);
    assert!(!client.get_require_mutual_preapproval());

    // Max allowed tokens
    client.set_max_allowed_tokens(&compliance, &10);
    assert_eq!(client.get_max_allowed_tokens(), 10);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_compliance_officer_cannot_pause() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let compliance = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ComplianceOfficer, &compliance);
    client.pause(&compliance);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_compliance_officer_cannot_set_fee_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let compliance = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ComplianceOfficer, &compliance);
    client.set_fee_config(&compliance, &200, &t.treasury);
}

#[test]
fn test_arbiter_manager_can_manage_arbiters() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let arbiter_mgr = Address::generate(&t.env);
    let new_arbiter = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ArbiterManager, &arbiter_mgr);

    client.add_arbiter_to_pool(&arbiter_mgr, &new_arbiter);
    client.remove_arbiter_from_pool(&arbiter_mgr, &new_arbiter);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_arbiter_manager_cannot_pause() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let arbiter_mgr = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ArbiterManager, &arbiter_mgr);
    client.pause(&arbiter_mgr);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_arbiter_manager_cannot_set_fee_config() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let arbiter_mgr = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ArbiterManager, &arbiter_mgr);
    client.set_fee_config(&arbiter_mgr, &300, &t.treasury);
}

#[test]
fn test_admin_can_call_all_scoped_functions_implicitly() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let arb = Address::generate(&t.env);
    let target = Address::generate(&t.env);

    // Admin can pause / unpause
    client.pause(&t.buyer);
    assert!(client.is_paused());
    client.unpause(&t.buyer);
    assert!(!client.is_paused());

    // Admin can set fee config
    client.set_fee_config(&t.buyer, &150, &t.treasury);
    assert_eq!(client.get_fee_config().fee_bps, 150);

    // Admin can blacklist
    client.blacklist_address(&t.buyer, &target, &BytesN::from_array(&t.env, &[9u8; 32]));
    assert!(client.is_blacklisted(&target));
    client.remove_from_blacklist(&t.buyer, &target);
    assert!(!client.is_blacklisted(&target));

    // Admin can add arbiter to pool
    client.add_arbiter_to_pool(&t.buyer, &arb);
    client.remove_arbiter_from_pool(&t.buyer, &arb);
}

#[test]
fn test_multi_role_assignment() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let dual_officer = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &dual_officer);
    client.grant_role(&t.buyer, &Role::FeeManager, &dual_officer);

    assert!(client.has_role(&Role::Pauser, &dual_officer));
    assert!(client.has_role(&Role::FeeManager, &dual_officer));
    assert!(!client.has_role(&Role::ComplianceOfficer, &dual_officer));
    assert!(!client.has_role(&Role::ArbiterManager, &dual_officer));

    // Can pause and set fees
    client.pause(&dual_officer);
    client.unpause(&dual_officer);
    client.set_fee_config(&dual_officer, &500, &t.treasury);
}

#[test]
fn test_grant_revoke_audit_log_and_events() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let user = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::Pauser, &user);
    client.revoke_role(&t.buyer, &Role::Pauser, &user);

    let log = client.get_admin_log();
    assert!(log.len() >= 2);
    let last_revoked = log.get(log.len() - 1).unwrap();
    let last_granted = log.get(log.len() - 2).unwrap();

    assert_eq!(last_granted.action, Symbol::new(&t.env, "grant_role"));
    assert_eq!(last_granted.result, Symbol::new(&t.env, "role_granted"));
    assert_eq!(last_revoked.action, Symbol::new(&t.env, "revoke_role"));
    assert_eq!(last_revoked.result, Symbol::new(&t.env, "role_revoked"));
}

#[test]
fn test_fee_manager_can_override_shipment_fee() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let fee_mgr = Address::generate(&t.env);
    let shipment_id = String::from_str(&t.env, "SHIP-OVERRIDE-01");

    client.grant_role(&t.buyer, &Role::FeeManager, &fee_mgr);

    // Fee manager can set and clear shipment fee override
    client.set_shipment_fee_override(&fee_mgr, &shipment_id, &150u32);
    assert_eq!(client.get_shipment_fee_override(&shipment_id), Some(150u32));

    client.clear_shipment_fee_override(&fee_mgr, &shipment_id);
    assert_eq!(client.get_shipment_fee_override(&shipment_id), None);
}

#[test]
fn test_compliance_officer_can_set_buyer_allowed_tokens_and_review_appeal() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let compliance = Address::generate(&t.env);
    let buyer = Address::generate(&t.env);
    let token = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::ComplianceOfficer, &compliance);

    // Set buyer allowed tokens
    client.set_buyer_allowed_tokens(&compliance, &buyer, &vec![&t.env, token.clone()]);
    let allowed = client.get_buyer_allowed_tokens(&buyer);
    assert_eq!(allowed.len(), 1);
    assert_eq!(allowed.get(0).unwrap(), token);

    // Blacklist, submit appeal, and review appeal
    let reason_hash = BytesN::from_array(&t.env, &[4u8; 32]);
    client.blacklist_address(&compliance, &buyer, &reason_hash);
    assert!(client.is_blacklisted(&buyer));

    client.submit_blacklist_appeal(&buyer, &String::from_str(&t.env, "ipfs://appeal-evidence"));
    client.review_blacklist_appeal(&compliance, &buyer, &true);
    assert!(!client.is_blacklisted(&buyer));
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_fee_manager_cannot_override_shipment_fee() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let compliance = Address::generate(&t.env);
    let shipment_id = String::from_str(&t.env, "SHIP-OVERRIDE-02");

    client.grant_role(&t.buyer, &Role::ComplianceOfficer, &compliance);
    client.set_shipment_fee_override(&compliance, &shipment_id, &150u32);
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_compliance_officer_cannot_set_buyer_allowed_tokens() {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let fee_mgr = Address::generate(&t.env);
    let buyer = Address::generate(&t.env);
    let token = Address::generate(&t.env);

    client.grant_role(&t.buyer, &Role::FeeManager, &fee_mgr);
    client.set_buyer_allowed_tokens(&fee_mgr, &buyer, &vec![&t.env, token]);
}

