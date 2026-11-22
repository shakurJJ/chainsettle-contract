#![cfg(test)]

//! #750: Collect protocol fees in a separate fee token.

extern crate std;

use super::*;
use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec, TestSetup};
use soroban_sdk::{contract, contractimpl, testutils::Address as _, token, String, Symbol};

const AMOUNT: i128 = 1_000_000;
const FEE_BPS: u32 = 100; // 1%
/// First milestone is 25% of the shipment.
const MILESTONE: i128 = AMOUNT / 4;
const ESCROW_FEE: i128 = MILESTONE * FEE_BPS as i128 / 10_000;

#[contract]
pub struct MockFeeOracle;

#[contractimpl]
impl MockFeeOracle {
    pub fn set_price(env: Env, asset: Address, price: i128) {
        env.storage().instance().set(&asset, &price);
    }

    pub fn get_price(env: Env, asset: Address) -> i128 {
        env.storage().instance().get(&asset).unwrap_or(0)
    }
}

struct FeeSetup {
    t: TestSetup,
    fee_token: Address,
}

fn fee_setup() -> FeeSetup {
    let t = setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_config(&t.buyer, &FEE_BPS, &t.treasury);
    let fee_token = t
        .env
        .register_stellar_asset_contract_v2(Address::generate(&t.env))
        .address();
    let minter = token::StellarAssetClient::new(&t.env, &fee_token);
    minter.mint(&t.supplier, &1_000_000);
    minter.mint(&t.buyer, &1_000_000);
    FeeSetup { t, fee_token }
}

fn sid(env: &Env, id: &str) -> String {
    String::from_str(env, id)
}

fn create(t: &TestSetup, id: &str, ext: ShipmentOptionsExt) -> String {
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.create_shipment_ext(
        &sid(&t.env, id),
        &single_buyer_vec(&t.env, &t.buyer),
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        &AMOUNT,
        &build_milestones(&t.env),
        &default_options(&t.env),
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

fn approve(f: &FeeSetup, owner: &Address, amount: i128) {
    token::Client::new(&f.t.env, &f.fee_token).approve(owner, &f.t.contract_id, &amount, &10_000);
}

/// Escrow -> fee token at 2 fee-token units per escrow unit.
fn use_conversion_rate(f: &FeeSetup) {
    let client = ChainSettleContractClient::new(&f.t.env, &f.t.contract_id);
    client.set_conversion_rate(&f.t.buyer, &f.t.token_id, &f.fee_token, &20_000);
    client.set_fee_token(&f.t.buyer, &f.fee_token, &FeeRateSource::ConversionRate);
}

#[test]
fn test_fee_token_config_roundtrip() {
    let f = fee_setup();
    let client = ChainSettleContractClient::new(&f.t.env, &f.t.contract_id);
    assert_eq!(client.get_fee_token(), None);
    use_conversion_rate(&f);
    assert_eq!(
        client.get_fee_token(),
        Some(FeeTokenConfig {
            token: f.fee_token.clone(),
            rate_source: FeeRateSource::ConversionRate,
        })
    );
    client.clear_fee_token(&f.t.buyer);
    assert_eq!(client.get_fee_token(), None);
}

#[test]
fn test_set_fee_token_is_admin_only() {
    let f = fee_setup();
    let client = ChainSettleContractClient::new(&f.t.env, &f.t.contract_id);
    let outsider = Address::generate(&f.t.env);
    assert!(client
        .try_set_fee_token(&outsider, &f.fee_token, &FeeRateSource::ConversionRate)
        .is_err());
}

#[test]
fn test_supplier_pays_fee_in_fee_token_and_gets_gross_payout() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    let fee = token::Client::new(&t.env, &f.fee_token);
    use_conversion_rate(&f);

    let id = create(t, "FT-SUP", ShipmentOptionsExt::default());
    assert!(client.get_shipment_fee_token(&id).is_some());
    let fee_in_fee_token = ESCROW_FEE * 2;
    approve(&f, &t.supplier, fee_in_fee_token);

    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    // Gross payout in the escrow token; nothing taken from escrow for fees.
    assert_eq!(escrow.balance(&t.supplier), MILESTONE);
    assert_eq!(escrow.balance(&t.treasury), 0);
    // Fee equals the escrow-token fee converted at the registered rate.
    assert_eq!(fee.balance(&t.treasury), fee_in_fee_token);
    assert_eq!(fee.balance(&t.supplier), 1_000_000 - fee_in_fee_token);
    assert_eq!(client.get_treasury_revenue(&f.fee_token), fee_in_fee_token);
}

#[test]
fn test_buyer_bearer_pays_fee() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    let fee = token::Client::new(&t.env, &f.fee_token);
    use_conversion_rate(&f);

    let ext = ShipmentOptionsExt {
        fee_bearer: FeeBearer::Buyer,
        ..ShipmentOptionsExt::default()
    };
    let id = create(t, "FT-BUY", ext);
    assert_eq!(client.get_shipment_fee_bearer(&id), FeeBearer::Buyer);
    approve(&f, &t.buyer, ESCROW_FEE * 2);

    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    assert_eq!(escrow.balance(&t.supplier), MILESTONE);
    assert_eq!(fee.balance(&t.buyer), 1_000_000 - ESCROW_FEE * 2);
    assert_eq!(fee.balance(&t.supplier), 1_000_000);
    assert_eq!(fee.balance(&t.treasury), ESCROW_FEE * 2);
}

#[test]
fn test_missing_allowance_fails_before_payout() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    use_conversion_rate(&f);

    let id = create(t, "FT-NOALLOW", ShipmentOptionsExt::default());
    submit_proof(t, &id, 0);
    let escrow_before = escrow.balance(&t.contract_id);

    assert!(client.try_confirm_milestone(&t.buyer, &id, &0).is_err());
    assert_eq!(escrow.balance(&t.supplier), 0);
    assert_eq!(escrow.balance(&t.contract_id), escrow_before);
    assert_eq!(
        client.get_milestone(&id, &0).status,
        MilestoneStatus::ProofSubmitted
    );

    // Once the bearer approves, the same confirmation succeeds.
    approve(&f, &t.supplier, ESCROW_FEE * 2);
    client.confirm_milestone(&t.buyer, &id, &0);
    assert_eq!(escrow.balance(&t.supplier), MILESTONE);
}

#[test]
#[should_panic(expected = "insufficient fee token allowance")]
fn test_missing_allowance_panics_with_reason() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    use_conversion_rate(&f);
    let id = create(t, "FT-NOALLOW2", ShipmentOptionsExt::default());
    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);
}

#[test]
fn test_fallback_to_escrow_deduction_when_not_configured() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);

    let id = create(t, "FT-NONE", ShipmentOptionsExt::default());
    assert!(client.get_shipment_fee_token(&id).is_none());
    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    assert_eq!(escrow.balance(&t.supplier), MILESTONE - ESCROW_FEE);
    assert_eq!(escrow.balance(&t.treasury), ESCROW_FEE);
}

#[test]
fn test_shipments_created_before_fee_token_keep_escrow_deduction() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    let fee = token::Client::new(&t.env, &f.fee_token);

    let id = create(t, "FT-OLD", ShipmentOptionsExt::default());
    use_conversion_rate(&f);
    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    assert_eq!(escrow.balance(&t.supplier), MILESTONE - ESCROW_FEE);
    assert_eq!(fee.balance(&t.treasury), 0);
}

#[test]
fn test_locked_fee_token_survives_clear() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let fee = token::Client::new(&t.env, &f.fee_token);
    use_conversion_rate(&f);
    let id = create(t, "FT-LOCK", ShipmentOptionsExt::default());
    client.clear_fee_token(&t.buyer);

    approve(&f, &t.supplier, ESCROW_FEE * 2);
    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);
    assert_eq!(fee.balance(&t.treasury), ESCROW_FEE * 2);
}

#[test]
#[should_panic(expected = "fee token conversion rate not configured")]
fn test_creation_requires_conversion_route() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    client.set_fee_token(&t.buyer, &f.fee_token, &FeeRateSource::ConversionRate);
    create(t, "FT-NOROUTE", ShipmentOptionsExt::default());
}

#[test]
fn test_oracle_rate_source() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    let fee = token::Client::new(&t.env, &f.fee_token);

    let oracle = t.env.register(MockFeeOracle, ());
    let oracle_client = MockFeeOracleClient::new(&t.env, &oracle);
    // One escrow unit is worth three fee-token units.
    oracle_client.set_price(&t.token_id, &3_000_000);
    oracle_client.set_price(&f.fee_token, &1_000_000);
    client.set_fee_token(
        &t.buyer,
        &f.fee_token,
        &FeeRateSource::Oracle(oracle.clone()),
    );

    let id = create(t, "FT-ORACLE", ShipmentOptionsExt::default());
    approve(&f, &t.supplier, ESCROW_FEE * 3);
    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    assert_eq!(escrow.balance(&t.supplier), MILESTONE);
    assert_eq!(fee.balance(&t.treasury), ESCROW_FEE * 3);
}

#[test]
fn test_fee_token_same_as_escrow_token_charges_bearer() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    client.set_fee_token(&t.buyer, &t.token_id, &FeeRateSource::ConversionRate);

    let ext = ShipmentOptionsExt {
        fee_bearer: FeeBearer::Buyer,
        ..ShipmentOptionsExt::default()
    };
    let id = create(t, "FT-SAME", ext);
    token::Client::new(&t.env, &t.token_id).approve(&t.buyer, &t.contract_id, &ESCROW_FEE, &10_000);
    submit_proof(t, &id, 0);
    client.confirm_milestone(&t.buyer, &id, &0);

    // Supplier is paid gross; the buyer paid the fee on top of escrow.
    assert_eq!(escrow.balance(&t.supplier), MILESTONE);
    assert_eq!(escrow.balance(&t.treasury), ESCROW_FEE);
}

#[test]
fn test_payout_preview_matches_fee_token_release() {
    let f = fee_setup();
    let t = &f.t;
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);
    let escrow = token::Client::new(&t.env, &t.token_id);
    use_conversion_rate(&f);

    let locked = create(t, "FT-PREVIEW", ShipmentOptionsExt::default());
    submit_proof(t, &locked, 0);
    let preview = client.preview_milestone_payout(&locked, &0);
    assert_eq!(preview.platform_fee, 0);

    approve(&f, &t.supplier, ESCROW_FEE * 2);
    client.confirm_milestone(&t.buyer, &locked, &0);
    assert_eq!(escrow.balance(&t.supplier), preview.supplier_net_amount);
    assert_eq!(preview.supplier_net_amount, MILESTONE);
}
