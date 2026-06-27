#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, vec, Address, BytesN, Env, String as SorobanString, Symbol,
};
use std::fs;
use std::path::Path;
use std::string::String;
use std::vec::Vec;

// ============================================================
// BENCHMARK CONFIGURATION
// ============================================================

const BASELINE_FILE: &str = "benchmarks/baselines.json";
const REGRESSION_THRESHOLD: f64 = 1.10; // 10% increase allowed

// ============================================================
// BENCHMARK HELPERS
// ============================================================

struct BenchmarkSetup {
    env: Env,
    contract_id: Address,
    token_id: Address,
    buyer: Address,
    supplier: Address,
    logistics: Address,
    arbiter: Address,
}

fn setup_benchmark() -> BenchmarkSetup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(ChainSettleContract, ());

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = token::StellarAssetClient::new(&env, &token_id);

    let buyer = Address::generate(&env);
    let supplier = Address::generate(&env);
    let logistics = Address::generate(&env);
    let arbiter = Address::generate(&env);

    token_client.mint(&buyer, &100_000_000_000);

    let client = ChainSettleContractClient::new(&env, &contract_id);
    client.init(&buyer);

    BenchmarkSetup {
        env,
        contract_id,
        token_id,
        buyer,
        supplier,
        logistics,
        arbiter,
    }
}

fn build_milestones_n(env: &Env, count: u32) -> soroban_sdk::Vec<Milestone> {
    let mut milestones = soroban_sdk::Vec::new(env);
    let percent_each = 100 / count;
    let mut total = 0;

    for i in 0..count {
        let percent = if i == count - 1 {
            // Last milestone gets the remainder to ensure sum = 100
            100 - total
        } else {
            percent_each
        };
        total += percent;

        milestones.push_back(Milestone {
            name: SorobanString::from_str(env, &std::format!("Milestone {}", i + 1)),
            payment_percent: percent,
            proof_hash: SorobanString::from_str(env, ""),
            status: MilestoneStatus::Pending,
            release_after_ledger: 0,
            proof_submitted_ledger: None,
            dispute_opened_ledger: None,
        });
    }

    milestones
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

        arbiter_fee_bps: 0,
        logistics_fee_bps: 0,
        supplier_collateral: 0,
        expires_at_ledger: None,
        metadata_hash: None,
        referrer: None,
        buyer_cancel_fee_bps: 0,
    }
}

// ============================================================
// INSTRUCTION MEASUREMENT
// ============================================================

struct BenchmarkResult {
    function_name: String,
    milestone_count: u32,
    instructions: u64,
}

impl BenchmarkResult {
    fn print(&self) {
        std::println!(
            "  {} (milestones={}): {} instructions",
            self.function_name,
            self.milestone_count,
            self.instructions
        );
    }
}

fn measure_instructions<F>(env: &Env, f: F) -> u64
where
    F: FnOnce(),
{
    // Reset budget to get clean measurement
    env.cost_estimate().budget().reset_unlimited();

    // Execute the function
    f();

    // Get CPU instructions consumed
    env.cost_estimate().budget().cpu_instruction_cost()
}

// ============================================================
// BENCHMARK FUNCTIONS
// ============================================================

fn benchmark_create_shipment(milestone_count: u32) -> BenchmarkResult {
    let setup = setup_benchmark();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = SorobanString::from_str(
        &setup.env,
        &std::format!("BENCH-CREATE-{}", milestone_count),
    );
    let milestones = build_milestones_n(&setup.env, milestone_count);
    let total_amount: i128 = 1_000_000_000;

    let instructions = measure_instructions(&setup.env, || {
        client.create_shipment(
            &shipment_id,
            &single_buyer_vec(&setup.env, &setup.buyer),
            &setup.supplier,
            &setup.logistics,
            &setup.arbiter,
            &setup.token_id,
            &total_amount,
            &milestones,
            &default_options(&setup.env),
        );
    });

    BenchmarkResult {
        function_name: String::from("create_shipment"),
        milestone_count,
        instructions,
    }
}

fn benchmark_submit_proof(milestone_count: u32) -> BenchmarkResult {
    let setup = setup_benchmark();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id =
        SorobanString::from_str(&setup.env, &std::format!("BENCH-PROOF-{}", milestone_count));
    let milestones = build_milestones_n(&setup.env, milestone_count);
    let total_amount: i128 = 1_000_000_000;

    // Create shipment first
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &milestones,
        &default_options(&setup.env),
    );

    let proof_hash = SorobanString::from_str(&setup.env, "ipfs://QmTest123");

    let instructions = measure_instructions(&setup.env, || {
        client.submit_proof(&setup.supplier, &shipment_id, &0, &proof_hash, &Symbol::new(&env, "ipfs"));
    });

    BenchmarkResult {
        function_name: String::from("submit_proof"),
        milestone_count,
        instructions,
    }
}

fn benchmark_confirm_milestone(milestone_count: u32) -> BenchmarkResult {
    let setup = setup_benchmark();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = SorobanString::from_str(
        &setup.env,
        &std::format!("BENCH-CONFIRM-{}", milestone_count),
    );
    let milestones = build_milestones_n(&setup.env, milestone_count);
    let total_amount: i128 = 1_000_000_000;

    // Create shipment and submit proof
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &milestones,
        &default_options(&setup.env),
    );

    let proof_hash = SorobanString::from_str(&setup.env, "ipfs://QmTest123");
    client.submit_proof(&setup.supplier, &shipment_id, &0, &proof_hash, &Symbol::new(&env, "ipfs"));

    let instructions = measure_instructions(&setup.env, || {
        client.confirm_milestone(&setup.buyer, &shipment_id, &0);
    });

    BenchmarkResult {
        function_name: String::from("confirm_milestone"),
        milestone_count,
        instructions,
    }
}

fn benchmark_raise_dispute(milestone_count: u32) -> BenchmarkResult {
    let setup = setup_benchmark();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = SorobanString::from_str(
        &setup.env,
        &std::format!("BENCH-DISPUTE-{}", milestone_count),
    );
    let milestones = build_milestones_n(&setup.env, milestone_count);
    let total_amount: i128 = 1_000_000_000;

    // Create shipment and submit proof
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &milestones,
        &default_options(&setup.env),
    );

    let proof_hash = SorobanString::from_str(&setup.env, "ipfs://QmTest123");
    client.submit_proof(&setup.supplier, &shipment_id, &0, &proof_hash, &Symbol::new(&env, "ipfs"));

    let instructions = measure_instructions(&setup.env, || {
        client.raise_dispute(&setup.buyer, &shipment_id, &0);
    });

    BenchmarkResult {
        function_name: String::from("raise_dispute"),
        milestone_count,
        instructions,
    }
}

fn benchmark_resolve_dispute(milestone_count: u32) -> BenchmarkResult {
    let setup = setup_benchmark();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = SorobanString::from_str(
        &setup.env,
        &std::format!("BENCH-RESOLVE-{}", milestone_count),
    );
    let milestones = build_milestones_n(&setup.env, milestone_count);
    let total_amount: i128 = 1_000_000_000;

    // Create shipment, submit proof, and raise dispute
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &milestones,
        &default_options(&setup.env),
    );

    let proof_hash = SorobanString::from_str(&setup.env, "ipfs://QmTest123");
    client.submit_proof(&setup.supplier, &shipment_id, &0, &proof_hash, &Symbol::new(&env, "ipfs"));
    client.raise_dispute(&setup.buyer, &shipment_id, &0);

    let instructions = measure_instructions(&setup.env, || {
        client.resolve_dispute(&setup.arbiter, &shipment_id, &0, &true);
    });

    BenchmarkResult {
        function_name: String::from("resolve_dispute"),
        milestone_count,
        instructions,
    }
}

fn benchmark_cancel_shipment(milestone_count: u32) -> BenchmarkResult {
    let setup = setup_benchmark();
    let client = ChainSettleContractClient::new(&setup.env, &setup.contract_id);

    let shipment_id = SorobanString::from_str(
        &setup.env,
        &std::format!("BENCH-CANCEL-{}", milestone_count),
    );
    let milestones = build_milestones_n(&setup.env, milestone_count);
    let total_amount: i128 = 1_000_000_000;

    // Create shipment
    client.create_shipment(
        &shipment_id,
        &single_buyer_vec(&setup.env, &setup.buyer),
        &setup.supplier,
        &setup.logistics,
        &setup.arbiter,
        &setup.token_id,
        &total_amount,
        &milestones,
        &default_options(&setup.env),
    );

    let instructions = measure_instructions(&setup.env, || {
        client.cancel_shipment(&setup.buyer, &shipment_id);
    });

    BenchmarkResult {
        function_name: String::from("cancel_shipment"),
        milestone_count,
        instructions,
    }
}

// ============================================================
// BASELINE MANAGEMENT
// ============================================================

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Baseline {
    function: String,
    milestones: u32,
    instructions: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct BaselineData {
    version: String,
    timestamp: String,
    baselines: Vec<Baseline>,
}

fn load_baselines() -> Option<BaselineData> {
    if Path::new(BASELINE_FILE).exists() {
        let content = fs::read_to_string(BASELINE_FILE).ok()?;
        serde_json::from_str(&content).ok()
    } else {
        None
    }
}

fn save_baselines(results: &Vec<BenchmarkResult>) {
    let baselines: Vec<Baseline> = results
        .iter()
        .map(|r| Baseline {
            function: r.function_name.clone(),
            milestones: r.milestone_count,
            instructions: r.instructions,
        })
        .collect();

    let data = BaselineData {
        version: String::from("1.0.0"),
        timestamp: chrono::Utc::now().to_rfc3339(),
        baselines,
    };

    // Create directory if it doesn't exist
    if let Some(parent) = Path::new(BASELINE_FILE).parent() {
        fs::create_dir_all(parent).ok();
    }

    let json = serde_json::to_string_pretty(&data).unwrap();
    fs::write(BASELINE_FILE, json).unwrap();
}

fn check_regression(results: &Vec<BenchmarkResult>) -> bool {
    let baseline_data = match load_baselines() {
        Some(data) => data,
        None => {
            std::println!("⚠️  No baseline found. Run with UPDATE_BASELINES=1 to create baseline.");
            return true;
        }
    };

    let mut passed = true;
    std::println!(
        "\n📊 Regression Check (threshold: +{}%)",
        (REGRESSION_THRESHOLD - 1.0) * 100.0
    );
    std::println!("{:-<80}", "");

    for result in results {
        let baseline = baseline_data
            .baselines
            .iter()
            .find(|b| b.function == result.function_name && b.milestones == result.milestone_count);

        if let Some(baseline) = baseline {
            let ratio = result.instructions as f64 / baseline.instructions as f64;
            let change_pct = (ratio - 1.0) * 100.0;
            let status = if ratio <= REGRESSION_THRESHOLD {
                "✅ PASS"
            } else {
                passed = false;
                "❌ FAIL"
            };

            std::println!(
                "{} {} (m={}): {} → {} ({:+.2}%)",
                status,
                result.function_name,
                result.milestone_count,
                baseline.instructions,
                result.instructions,
                change_pct
            );
        } else {
            std::println!(
                "⚠️  {} (m={}): No baseline found",
                result.function_name,
                result.milestone_count
            );
        }
    }

    std::println!("{:-<80}", "");
    passed
}

// ============================================================
// BENCHMARK RUNNER
// ============================================================

fn run_all_benchmarks() -> Vec<BenchmarkResult> {
    let mut results = Vec::new();

    std::println!("\n🔬 Running ChainSettle Contract Benchmarks");
    std::println!("{:=<80}", "");

    // Test with 1 milestone (minimum)
    std::println!("\n📦 Benchmarking with 1 milestone:");
    results.push(benchmark_create_shipment(1));
    results.last().unwrap().print();
    results.push(benchmark_submit_proof(1));
    results.last().unwrap().print();
    results.push(benchmark_confirm_milestone(1));
    results.last().unwrap().print();
    results.push(benchmark_raise_dispute(1));
    results.last().unwrap().print();
    results.push(benchmark_resolve_dispute(1));
    results.last().unwrap().print();
    results.push(benchmark_cancel_shipment(1));
    results.last().unwrap().print();

    // Test with 10 milestones (maximum typical)
    std::println!("\n📦 Benchmarking with 10 milestones:");
    results.push(benchmark_create_shipment(10));
    results.last().unwrap().print();
    results.push(benchmark_submit_proof(10));
    results.last().unwrap().print();
    results.push(benchmark_confirm_milestone(10));
    results.last().unwrap().print();
    results.push(benchmark_raise_dispute(10));
    results.last().unwrap().print();
    results.push(benchmark_resolve_dispute(10));
    results.last().unwrap().print();
    results.push(benchmark_cancel_shipment(10));
    results.last().unwrap().print();

    std::println!("\n{:=<80}", "");

    results
}

// ============================================================
// TEST ENTRY POINTS
// ============================================================

#[test]
fn benchmark_all_functions() {
    let results = run_all_benchmarks();

    // Check if we should update baselines
    let update_baselines = std::env::var("UPDATE_BASELINES").is_ok();

    if update_baselines {
        std::println!("\n💾 Updating baselines...");
        save_baselines(&results);
        std::println!("✅ Baselines saved to {}", BASELINE_FILE);
    } else {
        // Check for regressions
        let passed = check_regression(&results);
        if !passed {
            panic!("❌ Benchmark regression detected! Instructions exceeded threshold.");
        } else {
            std::println!("\n✅ All benchmarks passed!");
        }
    }
}

#[test]
fn benchmark_create_shipment_only() {
    std::println!("\n🔬 Benchmarking create_shipment");
    let result1 = benchmark_create_shipment(1);
    result1.print();
    let result10 = benchmark_create_shipment(10);
    result10.print();
}

#[test]
fn benchmark_submit_proof_only() {
    std::println!("\n🔬 Benchmarking submit_proof");
    let result1 = benchmark_submit_proof(1);
    result1.print();
    let result10 = benchmark_submit_proof(10);
    result10.print();
}

#[test]
fn benchmark_confirm_milestone_only() {
    std::println!("\n🔬 Benchmarking confirm_milestone");
    let result1 = benchmark_confirm_milestone(1);
    result1.print();
    let result10 = benchmark_confirm_milestone(10);
    result10.print();
}

#[test]
fn benchmark_raise_dispute_only() {
    std::println!("\n🔬 Benchmarking raise_dispute");
    let result1 = benchmark_raise_dispute(1);
    result1.print();
    let result10 = benchmark_raise_dispute(10);
    result10.print();
}

#[test]
fn benchmark_resolve_dispute_only() {
    std::println!("\n🔬 Benchmarking resolve_dispute");
    let result1 = benchmark_resolve_dispute(1);
    result1.print();
    let result10 = benchmark_resolve_dispute(10);
    result10.print();
}

#[test]
fn benchmark_cancel_shipment_only() {
    std::println!("\n🔬 Benchmarking cancel_shipment");
    let result1 = benchmark_cancel_shipment(1);
    result1.print();
    let result10 = benchmark_cancel_shipment(10);
    result10.print();
}
