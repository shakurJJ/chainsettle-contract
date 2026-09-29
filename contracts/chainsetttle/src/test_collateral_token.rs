#![cfg(test)]

use super::test_common::*;
use super::*;
use soroban_sdk::{
    testutils::Address as _,
    token, Address, Env, String, Symbol,
};

#[test]
fn test_collateral_denominated_in_different_token() {
    let setup = setup();
    let env = &setup.env;
    let client = ChainSettleContractClient::new(env, &setup.contract_id);

    // Register a second token for collateral
    let col_admin = Address::generate(env);
    let col_token_id = env
        .register_stellar_asset_contract_v2(col_admin.clone())
        .address();
    let col_token_client = token::StellarAssetClient::new(env, &col_token_id);
    let col_token_user = token::Client::new(env, &col_token_id);

    // Mint collateral tokens to supplier
    let collateral_amount = 5_000_000i128;
    col_token_client.mint(&setup.supplier, &collateral_amount);

    let mut options = default_options(env);
    options.supplier_collateral = collateral_amount;

    let shipment_id = String::from_str(env, "SHIP-ALT-COL");
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

    // Set custom collateral token
    client.set_collateral_token(&setup.buyer, &shipment_id, &col_token_id);

    // Verify collateral token getter
    assert_eq!(client.get_collateral_token(&shipment_id), col_token_id);

    let proof_type = Symbol::new(env, "IPFS");

    // Confirm all milestones to complete shipment
    client.submit_proof(&setup.supplier, &shipment_id, &0, &String::from_str(env, "proof1"), &proof_type);
    client.confirm_milestone(&setup.buyer, &shipment_id, &0);

    client.submit_proof(&setup.supplier, &shipment_id, &1, &String::from_str(env, "proof2"), &proof_type);
    client.confirm_milestone(&setup.buyer, &shipment_id, &1);

    client.submit_proof(&setup.supplier, &shipment_id, &2, &String::from_str(env, "proof3"), &proof_type);
    client.confirm_milestone(&setup.buyer, &shipment_id, &2);
}
