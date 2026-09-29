#![cfg(test)]

use super::test_common::*;
use super::*;
use soroban_sdk::String;

#[test]
fn test_global_tvl_cap_success() {
    let setup = setup();
    let env = &setup.env;
    let client = ChainSettleContractClient::new(env, &setup.contract_id);

    assert_eq!(client.get_tvl(&setup.token_id), 0);
    assert_eq!(client.get_tvl_cap(&setup.token_id), 0);

    let cap = 15_000_000i128;
    client.set_tvl_cap(&setup.buyer, &setup.token_id, &cap);
    assert_eq!(client.get_tvl_cap(&setup.token_id), cap);

    let shipment_id_1 = String::from_str(env, "SHIP-TVL-1");
    let options = default_options(env);
    client.create_shipment(
        &shipment_id_1,
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &10_000_000,
        &build_milestones(env),
        &options,
    );
    assert_eq!(client.get_tvl(&setup.token_id), 10_000_000);

    let shipment_id_2 = String::from_str(env, "SHIP-TVL-2");
    client.create_shipment(
        &shipment_id_2,
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &5_000_000,
        &build_milestones(env),
        &options,
    );
    assert_eq!(client.get_tvl(&setup.token_id), 15_000_000);
}

#[test]
#[should_panic(expected = "TVL cap exceeded")]
fn test_global_tvl_cap_exceeded_panics() {
    let setup = setup();
    let env = &setup.env;
    let client = ChainSettleContractClient::new(env, &setup.contract_id);

    let cap = 15_000_000i128;
    client.set_tvl_cap(&setup.buyer, &setup.token_id, &cap);

    let shipment_id_1 = String::from_str(env, "SHIP-TVL-EXCEED-1");
    let options = default_options(env);
    client.create_shipment(
        &shipment_id_1,
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &10_000_000,
        &build_milestones(env),
        &options,
    );

    let shipment_id_2 = String::from_str(env, "SHIP-TVL-EXCEED-2");
    client.create_shipment(
        &shipment_id_2,
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &10_000_000,
        &build_milestones(env),
        &options,
    );
}
