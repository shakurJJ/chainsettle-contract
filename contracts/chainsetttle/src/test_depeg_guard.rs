#![cfg(test)]

use super::test_common::*;
use super::*;
use soroban_sdk::{
    contract, contractimpl, contracttype,
    Address, Env, String, Symbol,
};

#[contracttype]
enum MockOracleKey {
    Price(Address),
}

#[contract]
pub struct MockPriceOracle;

#[contractimpl]
impl MockPriceOracle {
    pub fn set_price(env: Env, asset: Address, price: i128) {
        env.storage().instance().set(&MockOracleKey::Price(asset), &price);
    }

    pub fn get_price(env: Env, asset: Address) -> i128 {
        env.storage().instance().get(&MockOracleKey::Price(asset)).unwrap_or(0)
    }
}

#[test]
fn test_depeg_guard_configuration_and_checks() {
    let setup = setup();
    let env = &setup.env;
    let client = ChainSettleContractClient::new(env, &setup.contract_id);

    let oracle_id = env.register(MockPriceOracle, ());
    let oracle_client = MockPriceOracleClient::new(env, &oracle_id);

    // Set 1 USDC = 1_000_000 (1.00 in 6 decimals)
    oracle_client.set_price(&setup.token_id, &1_000_000);

    // Admin sets depeg guard: min price 950_000 (0.95), max price 1_050_000 (1.05)
    client.set_depeg_guard(&setup.buyer, &setup.token_id, &oracle_id, &950_000, &1_050_000);

    let guard = client.get_depeg_guard(&setup.token_id).unwrap();
    assert_eq!(guard.oracle, oracle_id);
    assert_eq!(guard.min_price, 950_000);
    assert_eq!(guard.max_price, 1_050_000);

    // Currently price is 1.00 -> not depegged
    assert!(!client.is_token_depegged(&setup.token_id));

    // Depeg token: drop price to 900_000 (0.90)
    oracle_client.set_price(&setup.token_id, &900_000);
    assert!(client.is_token_depegged(&setup.token_id));

    // Restore price to 1.00 -> not depegged
    oracle_client.set_price(&setup.token_id, &1_000_000);
    assert!(!client.is_token_depegged(&setup.token_id));

    // Remove depeg guard
    client.remove_depeg_guard(&setup.buyer, &setup.token_id);
    assert!(client.get_depeg_guard(&setup.token_id).is_none());
}

#[test]
#[should_panic(expected = "token is depegged")]
fn test_depeg_guard_blocks_confirmation() {
    let setup = setup();
    let env = &setup.env;
    let client = ChainSettleContractClient::new(env, &setup.contract_id);

    let oracle_id = env.register(MockPriceOracle, ());
    let oracle_client = MockPriceOracleClient::new(env, &oracle_id);

    oracle_client.set_price(&setup.token_id, &1_000_000);
    client.set_depeg_guard(&setup.buyer, &setup.token_id, &oracle_id, &950_000, &1_050_000);

    let shipment_id = String::from_str(env, "SHIP-DEPEG-FAIL");
    let options = default_options(env);
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &10_000_000,
        &build_milestones(env),
        &options,
    );

    let proof_type = Symbol::new(env, "IPFS");
    client.submit_proof(&setup.supplier, &shipment_id, &0, &String::from_str(env, "proof1"), &proof_type);

    // Depeg token and attempt confirmation -> panics
    oracle_client.set_price(&setup.token_id, &900_000);
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);
}
