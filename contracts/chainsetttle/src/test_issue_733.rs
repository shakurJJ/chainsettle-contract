#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, vec, BytesN, String, Symbol};

use crate::test_common::{build_milestones, default_options, setup, single_buyer_vec};

#[test]
fn confirmation_blocked_until_all_required_documents_are_attached() {
    let s = setup();
    let client = ChainSettleContractClient::new(&s.env, &s.contract_id);
    let id = String::from_str(&s.env, "DOC-1");
    let requirements = vec![
        &s.env,
        DocumentRequirement {
            milestone_index: 0,
            doc_types: vec![
                &s.env,
                DocumentType::Invoice,
                DocumentType::PackingList,
            ],
        },
    ];
    let ext = ShipmentOptionsExt {
        required_documents: Some(requirements),
        ..Default::default()
    };
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

    assert!(client.try_confirm_milestone(&s.buyer, &id, &0).is_err());
    client.attach_document(
        &s.supplier,
        &id,
        &0,
        &DocumentType::Invoice,
        &BytesN::<32>::from_array(&s.env, &[1u8; 32]),
    );
    assert!(client.try_confirm_milestone(&s.buyer, &id, &0).is_err());
    client.attach_document(
        &s.supplier,
        &id,
        &0,
        &DocumentType::PackingList,
        &BytesN::<32>::from_array(&s.env, &[2u8; 32]),
    );
    client.confirm_milestone(&s.buyer, &id, &0);

    // Existing invoice/BOL helpers still work alongside typed documents.
    client.attach_invoice_hash(
        &s.supplier,
        &id,
        &0,
        &BytesN::<32>::from_array(&s.env, &[3u8; 32]),
    );
    assert!(client.get_invoice_hash(&id, &0).is_some());
}
