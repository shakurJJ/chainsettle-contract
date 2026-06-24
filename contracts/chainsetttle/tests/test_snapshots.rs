extern crate chainsetttle;
use chainsetttle::*;
use serde::Serialize;
use soroban_sdk::{testutils::{Address as _}, token, vec, Address, Env, String as SorobanString};
use std::{env, fs, path::PathBuf};

struct TestSetup {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    buyer2: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
    treasury: Address,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotMilestoneStatus {
    Pending,
    ProofSubmitted,
    Confirmed,
    Disputed,
    Resolved,
    ConfirmedHeld,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotShipmentStatus {
    Active,
    Completed,
    Cancelled,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotMilestoneMode {
    Sequential,
    Parallel,
}

#[derive(Serialize)]
struct SnapshotAuditEntry {
    action: String,
    caller: String,
    ledger: u32,
    detail: String,
}

#[derive(Serialize)]
struct SnapshotMilestone {
    name: String,
    payment_percent: u32,
    proof_hash: String,
    status: SnapshotMilestoneStatus,
    release_after_ledger: u32,
    proof_submitted_ledger: Option<u32>,
    dispute_opened_ledger: Option<u32>,
}

#[derive(Serialize)]
struct SnapshotShipment {
    id: String,
    audit_log: Vec<SnapshotAuditEntry>,
    buyers: Vec<String>,
    supplier: String,
    logistics: String,
    arbiter: String,
    token: String,
    total_amount: i128,
    released_amount: i128,
    milestones: Vec<SnapshotMilestone>,
    status: SnapshotShipmentStatus,
    milestone_mode: SnapshotMilestoneMode,
    created_at: u32,
    holdback_ledgers: u32,
    dispute_cooldown_ledgers: u32,
    last_dispute_resolved_ledger: Option<u32>,
    late_penalty_bps_per_ledger: u32,
    auto_confirm_ledgers: u32,
    open_dispute_count: u32,
    dispute_bond_amount: i128,
}

impl SnapshotShipment {
    fn from_shipment(shipment: Shipment) -> Self {
        Self {
            id: shipment.id.to_string(),
            audit_log: shipment
                .audit_log
                .iter()
                .map(|entry| SnapshotAuditEntry {
                    action: format!("{:?}", entry.action),
                    caller: format!("{:?}", entry.caller),
                    ledger: entry.ledger,
                    detail: format!("{:?}", entry.detail),
                })
                .collect(),
            buyers: shipment
                .buyers
                .iter()
                .map(|buyer| format!("{:?}", buyer))
                .collect(),
            supplier: format!("{:?}", shipment.supplier),
            logistics: format!("{:?}", shipment.logistics),
            arbiter: format!("{:?}", shipment.arbiter),
            token: format!("{:?}", shipment.token),
            total_amount: shipment.total_amount,
            released_amount: shipment.released_amount,
            milestones: shipment
                .milestones
                .iter()
                .map(|milestone| SnapshotMilestone {
                    name: milestone.name.to_string(),
                    payment_percent: milestone.payment_percent,
                    proof_hash: milestone.proof_hash.to_string(),
                    status: match milestone.status {
                        MilestoneStatus::Pending => SnapshotMilestoneStatus::Pending,
                        MilestoneStatus::ProofSubmitted => SnapshotMilestoneStatus::ProofSubmitted,
                        MilestoneStatus::Confirmed => SnapshotMilestoneStatus::Confirmed,
                        MilestoneStatus::Disputed => SnapshotMilestoneStatus::Disputed,
                        MilestoneStatus::Resolved => SnapshotMilestoneStatus::Resolved,
                        MilestoneStatus::ConfirmedHeld => SnapshotMilestoneStatus::ConfirmedHeld,
                    },
                    release_after_ledger: milestone.release_after_ledger,
                    proof_submitted_ledger: milestone.proof_submitted_ledger,
                    dispute_opened_ledger: milestone.dispute_opened_ledger,
                })
                .collect(),
            status: match shipment.status {
                ShipmentStatus::Active => SnapshotShipmentStatus::Active,
                ShipmentStatus::Completed => SnapshotShipmentStatus::Completed,
                ShipmentStatus::Cancelled => SnapshotShipmentStatus::Cancelled,
            },
            milestone_mode: match shipment.milestone_mode {
                MilestoneMode::Sequential => SnapshotMilestoneMode::Sequential,
                MilestoneMode::Parallel => SnapshotMilestoneMode::Parallel,
            },
            created_at: shipment.created_at,
            holdback_ledgers: shipment.holdback_ledgers,
            dispute_cooldown_ledgers: shipment.dispute_cooldown_ledgers,
            last_dispute_resolved_ledger: shipment.last_dispute_resolved_ledger,
            late_penalty_bps_per_ledger: shipment.late_penalty_bps_per_ledger,
            auto_confirm_ledgers: shipment.auto_confirm_ledgers,
            open_dispute_count: shipment.open_dispute_count,
            dispute_bond_amount: shipment.dispute_bond_amount,
        }
    }
}

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("snapshots")
}

fn snapshot_file(name: &str) -> PathBuf {
    snapshot_dir().join(name)
}

fn write_or_compare_snapshot(name: &str, content: &str) {
    let path = snapshot_file(name);
    if env::var("UPDATE_SNAPSHOTS").is_ok() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content.as_bytes()).unwrap();
    } else {
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Missing snapshot file: {}", path.display()));
        if expected != content {
            panic!("Snapshot mismatch for {}. Run UPDATE_SNAPSHOTS=1 cargo test --all to update.\n\nExpected:\n{}\n\nActual:\n{}", name, expected, content);
        }
    }
}

fn env_setup() -> TestSetup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = token::StellarAssetClient::new(&env, &token_id);

    let buyer = Address::generate(&env);
    let buyer2 = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);
    let treasury = Address::generate(&env);

    token_client.mint(&buyer, &10_000_000_000);
    token_client.mint(&buyer2, &10_000_000_000);

    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    TestSetup {
        env,
        contract_id,
        token_id,
        buyer,
        buyer2,
        supplier,
        logistics,
        arbiter,
        treasury,
    }
}

fn build_milestones(env: &Env) -> soroban_sdk::Vec<Milestone> {
    vec![
        env,
        Milestone {
            name: SorobanString::from_str(env, "Goods Dispatched"),
            payment_percent: 25,
            proof_hash: SorobanString::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: SorobanString::from_str(env, "In Transit"),
            payment_percent: 50,
            proof_hash: SorobanString::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
        Milestone {
            name: SorobanString::from_str(env, "Delivered"),
            payment_percent: 25,
            proof_hash: SorobanString::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        },
    ]
}

fn single_buyer_vec(env: &Env, buyer: &Address) -> soroban_sdk::Vec<Address> {
    vec![env, buyer.clone()]
}

fn default_options(_env: &Env) -> ShipmentOptions {
    ShipmentOptions {
        response_deadline: 0,
        penalty_bps: 0,
        milestone_mode: MilestoneMode::Parallel,
        holdback_ledgers: 0,
        dispute_cooldown_ledgers: 0,
        late_penalty_bps_per_ledger: 0,
        auto_confirm_ledgers: 0,
        dispute_bond_amount: 0,
    }
}

fn create_standard_shipment(
    client: &ChainSettleContractClient,
    env: &Env,
    shipment_id: &SorobanString,
    buyer: &Address,
    supplier: &Address,
    logistics: &Address,
    arbiter: &Address,
    token_id: &Address,
    total_amount: i128,
) {
    client.create_shipment(
        &shipment_id.clone(),
        &single_buyer_vec(env, buyer),
        supplier,
        logistics,
        arbiter,
        token_id,
        &total_amount,
        &build_milestones(env),
        &default_options(env),
    );
}

#[test]
fn test_shipment_lifecycle_snapshots() {
    let t = env_setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = SorobanString::from_str(&t.env, "SHIP-SNAPSHOT");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    let create_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &0,
        &SorobanString::from_str(&t.env, "ipfs://d0"),
    );
    let after_submit_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));

    client.confirm_milestone(&t.buyer, &shipment_id, &0);
    let after_confirm_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));

    client.submit_proof(
        &t.supplier,
        &shipment_id,
        &1,
        &SorobanString::from_str(&t.env, "ipfs://d1"),
    );
    client.raise_dispute(&t.buyer, &shipment_id, &1);
    let after_dispute_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));

    client.resolve_dispute(&t.arbiter, &shipment_id, &1, &false);
    let after_resolve_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));

    let snapshots = serde_json::json!({
        "create": create_snapshot,
        "after_submit_proof": after_submit_snapshot,
        "after_confirm": after_confirm_snapshot,
        "after_dispute": after_dispute_snapshot,
        "after_resolve": after_resolve_snapshot,
    });

    let snapshot_content = serde_json::to_string_pretty(&snapshots).unwrap();
    write_or_compare_snapshot("shipment_lifecycle.snap", &snapshot_content);
}

#[test]
fn test_shipment_cancel_snapshot() {
    let t = env_setup();
    let client = ChainSettleContractClient::new(&t.env, &t.contract_id);

    let shipment_id = SorobanString::from_str(&t.env, "SHIP-CANCEL-SNAPSHOT");
    let total_amount: i128 = 1_000_000_000;

    create_standard_shipment(
        &client,
        &t.env,
        &shipment_id,
        &t.buyer,
        &t.supplier,
        &t.logistics,
        &t.arbiter,
        &t.token_id,
        total_amount,
    );

    let create_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));
    client.cancel_shipment(&t.buyer, &shipment_id);
    let after_cancel_snapshot = SnapshotShipment::from_shipment(client.get_shipment(&shipment_id));

    let snapshots = serde_json::json!({
        "create": create_snapshot,
        "after_cancel": after_cancel_snapshot,
    });

    let snapshot_content = serde_json::to_string_pretty(&snapshots).unwrap();
    write_or_compare_snapshot("shipment_cancel.snap", &snapshot_content);
}
