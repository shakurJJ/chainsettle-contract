#![cfg(test)]

use super::test_common::*;
use super::*;
use soroban_sdk::{String};

#[test]
fn test_auto_generated_shipment_ids() {
    let setup = setup();
    let env = &setup.env;
    let client = ChainSettleContractClient::new(env, &setup.contract_id);

    let options = default_options(env);

    // Create first auto ID shipment -> SHIP-1
    let id1 = client.create_shipment_auto_id(
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &10_000_000,
        &build_milestones(env),
        &options,
    );
    assert_eq!(id1, String::from_str(env, "SHIP-1"));

    // Verify shipment was created and is active
    let shipment1 = client.get_shipment(&id1);
    assert_eq!(shipment1.id, id1);
    assert_eq!(shipment1.status, ShipmentStatus::Active);

    // Create second auto ID shipment -> SHIP-2
    let id2 = client.create_shipment_auto_id(
        &single_buyer_vec(env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &5_000_000,
        &build_milestones(env),
        &options,
    );
    assert_eq!(id2, String::from_str(env, "SHIP-2"));

    let shipment2 = client.get_shipment(&id2);
    assert_eq!(shipment2.id, id2);
    assert_eq!(shipment2.status, ShipmentStatus::Active);
}
