#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, vec, Address, Env, String, Symbol};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[contract]
pub struct MockPriceOracle;

#[contractimpl]
impl MockPriceOracle {
    pub fn init(env: Env, price: i128) {
        env.storage().instance().set(&Symbol::new(&env, "price"), &price);
    }

    pub fn get_price(env: Env, _asset: Address) -> i128 {
        env.storage().instance().get(&Symbol::new(&env, "price")).unwrap()
    }
}

fn setup_with_oracle(price: i128) -> (crate::test_common::TestSetup, Address) {
    let s = setup();
    let oracle = s.env.register(MockPriceOracle, ());
    MockPriceOracleClient::new(&s.env, &oracle).init(&price);
    (s, oracle)
}

#[test]
fn upward_adjustment_requires_top_up_then_pays_scaled_amount() {
    let (s, oracle) = setup_with_oracle(110);
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);
    let ext = ShipmentOptionsExt {
        price_adjustment: Some(PriceAdjustmentClause {
            oracle: oracle.clone(),
            base_price: 100,
            max_up_bps: 2_000,
            max_down_bps: 2_000,
        }),
        ..Default::default()
    };
    client.create_shipment_ext(
        &String::from_str(&s.env, "ADJ-1"),
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &default_options(&s.env),
        &ext,
    );
    client.submit_proof(
        &s.supplier,
        &String::from_str(&s.env, "ADJ-1"),
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    let id = String::from_str(&s.env, "ADJ-1");
    assert!(client.try_confirm_milestone(&s.buyer, &id, &0).is_err());
    client.top_up_price_adjustment(&s.buyer, &id, &0, &250i128);
    client.confirm_milestone(&s.buyer, &id, &0);
    assert_eq!(token.balance(&s.supplier), 2_750);
}

#[test]
fn downward_adjustment_refunds_buyer_delta() {
    let (s, oracle) = setup_with_oracle(90);
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.token_id);
    let ext = ShipmentOptionsExt {
        price_adjustment: Some(PriceAdjustmentClause {
            oracle: oracle.clone(),
            base_price: 100,
            max_up_bps: 2_000,
            max_down_bps: 2_000,
        }),
        ..Default::default()
    };
    let id = String::from_str(&s.env, "ADJ-2");
    client.create_shipment_ext(
        &id,
        &single_buyer_vec(&s.env, &s.buyer),
        &s.supplier,
        &s.logistics,
        &s.arbiter,
        &s.token_id,
        &10_000i128,
        &build_milestones(&s.env),
        &default_options(&s.env),
        &ext,
    );
    client.submit_proof(
        &s.supplier,
        &id,
        &0,
        &String::from_str(&s.env, "proof"),
        &Symbol::new(&s.env, "ipfs"),
    );
    let before = token.balance(&s.buyer);
    client.confirm_milestone(&s.buyer, &id, &0);
    assert_eq!(token.balance(&s.supplier), 2_250);
    assert_eq!(token.balance(&s.buyer) - before, 250);
}
