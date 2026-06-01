use soroban_sdk::{contracttype, Address, Env, String, Vec};

use crate::{
    constants::{TTL_INITIAL_LEDGERS, TTL_MAX_LEDGERS},
    AuditEntry, CancelPolicy, ContractStats, DisputeEntry, FeeConfig, MultiAdminConfig, Shipment,
    ShipmentStatus,
};

// ============================================================
// VERSIONED STORAGE KEYS  (#77)
// All variants are prefixed with V1 so that a future upgrade can
// introduce V2_* keys and run migrate_v1_to_v2 without clobbering
// live state.
// ============================================================

#[contracttype]
pub enum DataKey {
    V1Shipment(String),
    V1CancelPolicy(String),
    V1AllShipments,
    /// Supplier-to-shipments index.
    V1SupplierShipments(Address),
    /// Buyer-to-shipments index.
    V1BuyerShipments(Address),
    V1Admin,
    /// Ledger sequence when a milestone entered ProofSubmitted state.
    V1ProofSubmittedAt(String, u32),
    /// Pending amendment proposal.
    V1Amendment(String, u32),
    /// Optional fee configuration.
    V1FeeConfig,
    /// Minimum allowed milestone payment percent.
    V1MinMilestonePercent,
    /// Maximum concurrently open disputes per shipment.
    V1MaxConcurrentDisputes,
    /// Blacklisted addresses.
    V1Blacklisted(Address),
    /// Bounded admin action log.
    V1AdminActionLog,
    /// Whitelisted token addresses.
    V1AllowedTokens,
    /// Global pause flag.
    V1Paused,
    /// Pending arbiter rotation proposal.
    V1ArbiterRotation(String),
    /// Total escrowed value for a given token.
    V1TotalEscrowed(Address),
    /// Active disputes list.
    V1ActiveDisputes,
    /// Contract-level statistics.
    V1ContractStats,
    /// Per-status index of shipment IDs.
    V1ShipmentsByStatus(ShipmentStatus),
    /// Escalation threshold in ledgers.
    V1EscalationThreshold,
    /// Maximum shipment value cap (0 = no cap).
    V1MaxShipmentValue,
    /// Circuit breaker outflow limit.
    V1CircuitBreakerLimit,
    /// Circuit breaker window in ledgers.
    V1CircuitBreakerWindow,
    /// Circuit breaker window start ledger.
    V1CircuitBreakerWindowStart,
    /// Circuit breaker window outflow amount.
    V1CircuitBreakerWindowOutflow,
    /// Multi-admin approvals tracking.
    V1AdminApprovals(String),
    /// Multi-admin configuration.
    V1MultiAdminConfig,
    /// Pending admin nominee for two-step transfer.
    V1PendingAdmin,
}

// ============================================================
// SHIPMENT ACCESSORS
// ============================================================

pub fn get_shipment(env: &Env, shipment_id: &String) -> Option<Shipment> {
    env.storage()
        .persistent()
        .get(&DataKey::V1Shipment(shipment_id.clone()))
}

pub fn set_shipment(env: &Env, shipment_id: &String, shipment: &Shipment) {
    let key = DataKey::V1Shipment(shipment_id.clone());
    env.storage().persistent().set(&key, shipment);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_INITIAL_LEDGERS, TTL_MAX_LEDGERS);
}

pub fn shipment_exists(env: &Env, shipment_id: &String) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::V1Shipment(shipment_id.clone()))
}

// ============================================================
// CANCEL POLICY ACCESSORS
// ============================================================

pub fn get_cancel_policy(env: &Env, shipment_id: &String) -> Option<CancelPolicy> {
    env.storage()
        .persistent()
        .get(&DataKey::V1CancelPolicy(shipment_id.clone()))
}

pub fn set_cancel_policy(env: &Env, shipment_id: &String, policy: &CancelPolicy) {
    env.storage()
        .persistent()
        .set(&DataKey::V1CancelPolicy(shipment_id.clone()), policy);
}

// ============================================================
// ALL SHIPMENTS LIST
// ============================================================

pub fn get_all_shipments(env: &Env) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::V1AllShipments)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn push_shipment_id(env: &Env, shipment_id: &String) {
    let mut list = get_all_shipments(env);
    list.push_back(shipment_id.clone());
    env.storage()
        .persistent()
        .set(&DataKey::V1AllShipments, &list);
}

// ============================================================
// SUPPLIER / BUYER INDEX ACCESSORS
// ============================================================

pub fn get_supplier_shipments(env: &Env, supplier: &Address) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::V1SupplierShipments(supplier.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_supplier_shipments(env: &Env, supplier: &Address, list: &Vec<String>) {
    let key = DataKey::V1SupplierShipments(supplier.clone());
    env.storage().persistent().set(&key, list);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_INITIAL_LEDGERS, TTL_MAX_LEDGERS);
}

pub fn get_buyer_shipments(env: &Env, buyer: &Address) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::V1BuyerShipments(buyer.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_buyer_shipments(env: &Env, buyer: &Address, list: &Vec<String>) {
    let key = DataKey::V1BuyerShipments(buyer.clone());
    env.storage().persistent().set(&key, list);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_INITIAL_LEDGERS, TTL_MAX_LEDGERS);
}

// ============================================================
// ADMIN ACCESSORS
// ============================================================

pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::V1Admin)
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::V1Admin, admin);
}

// ============================================================
// PROOF SUBMITTED AT
// ============================================================

pub fn get_proof_submitted_at(env: &Env, shipment_id: &String, index: u32) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::V1ProofSubmittedAt(shipment_id.clone(), index))
}

pub fn set_proof_submitted_at(env: &Env, shipment_id: &String, index: u32, ledger: u32) {
    env.storage().persistent().set(
        &DataKey::V1ProofSubmittedAt(shipment_id.clone(), index),
        &ledger,
    );
}

// ============================================================
// ACTIVE DISPUTES
// ============================================================

pub fn get_active_disputes(env: &Env) -> Vec<DisputeEntry> {
    env.storage()
        .persistent()
        .get(&DataKey::V1ActiveDisputes)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_active_disputes(env: &Env, disputes: &Vec<DisputeEntry>) {
    env.storage()
        .persistent()
        .set(&DataKey::V1ActiveDisputes, disputes);
}

// ============================================================
// TOTAL ESCROWED
// ============================================================

pub fn get_total_escrowed(env: &Env, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::V1TotalEscrowed(token.clone()))
        .unwrap_or(0)
}

pub fn set_total_escrowed(env: &Env, token: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&DataKey::V1TotalEscrowed(token.clone()), &amount);
}

// ============================================================
// CONTRACT STATS
// ============================================================

pub fn get_contract_stats(env: &Env) -> ContractStats {
    env.storage()
        .instance()
        .get(&DataKey::V1ContractStats)
        .unwrap_or(ContractStats {
            total_shipments: 0,
            total_volume: 0,
            total_disputes: 0,
            completed_shipments: 0,
        })
}

pub fn set_contract_stats(env: &Env, stats: &ContractStats) {
    env.storage()
        .instance()
        .set(&DataKey::V1ContractStats, stats);
}

// ============================================================
// STATUS INDEX
// ============================================================

pub fn get_shipments_by_status(env: &Env, status: ShipmentStatus) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::V1ShipmentsByStatus(status))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_shipments_by_status(env: &Env, status: ShipmentStatus, list: &Vec<String>) {
    env.storage()
        .persistent()
        .set(&DataKey::V1ShipmentsByStatus(status), list);
}

// ============================================================
// INSTANCE STORAGE HELPERS (paused, fee config, limits, etc.)
// ============================================================

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::V1Paused)
        .unwrap_or(false)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::V1Paused, &paused);
}

pub fn get_fee_config(env: &Env) -> Option<FeeConfig> {
    env.storage().instance().get(&DataKey::V1FeeConfig)
}

pub fn set_fee_config(env: &Env, config: &FeeConfig) {
    env.storage().instance().set(&DataKey::V1FeeConfig, config);
}

pub fn get_min_milestone_percent(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::V1MinMilestonePercent)
        .unwrap_or(5u32)
}

pub fn set_min_milestone_percent(env: &Env, pct: u32) {
    env.storage()
        .instance()
        .set(&DataKey::V1MinMilestonePercent, &pct);
}

pub fn get_max_concurrent_disputes(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::V1MaxConcurrentDisputes)
        .unwrap_or(1u32)
}

pub fn set_max_concurrent_disputes(env: &Env, limit: u32) {
    env.storage()
        .instance()
        .set(&DataKey::V1MaxConcurrentDisputes, &limit);
}

pub fn is_blacklisted(env: &Env, address: &Address) -> bool {
    env.storage()
        .instance()
        .get::<DataKey, soroban_sdk::BytesN<32>>(&DataKey::V1Blacklisted(address.clone()))
        .is_some()
}

pub fn set_blacklisted(env: &Env, address: &Address, reason: &soroban_sdk::BytesN<32>) {
    env.storage()
        .instance()
        .set(&DataKey::V1Blacklisted(address.clone()), reason);
}

pub fn remove_blacklisted(env: &Env, address: &Address) {
    env.storage()
        .instance()
        .remove(&DataKey::V1Blacklisted(address.clone()));
}

pub fn get_admin_action_log(env: &Env) -> Vec<AuditEntry> {
    env.storage()
        .instance()
        .get(&DataKey::V1AdminActionLog)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_admin_action_log(env: &Env, log: &Vec<AuditEntry>) {
    env.storage()
        .instance()
        .set(&DataKey::V1AdminActionLog, log);
}

pub fn get_allowed_tokens(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::V1AllowedTokens)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_allowed_tokens(env: &Env, tokens: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::V1AllowedTokens, tokens);
}

pub fn get_escalation_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::V1EscalationThreshold)
        .unwrap_or(0)
}

pub fn set_escalation_threshold(env: &Env, threshold: u32) {
    env.storage()
        .instance()
        .set(&DataKey::V1EscalationThreshold, &threshold);
}

pub fn get_max_shipment_value(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::V1MaxShipmentValue)
        .unwrap_or(0)
}

pub fn set_max_shipment_value(env: &Env, value: i128) {
    env.storage()
        .instance()
        .set(&DataKey::V1MaxShipmentValue, &value);
}

pub fn get_circuit_breaker_limit(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::V1CircuitBreakerLimit)
        .unwrap_or(0)
}

pub fn set_circuit_breaker_limit(env: &Env, limit: i128) {
    env.storage()
        .instance()
        .set(&DataKey::V1CircuitBreakerLimit, &limit);
}

pub fn get_circuit_breaker_window(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::V1CircuitBreakerWindow)
        .unwrap_or(0)
}

pub fn set_circuit_breaker_window(env: &Env, window: u32) {
    env.storage()
        .instance()
        .set(&DataKey::V1CircuitBreakerWindow, &window);
}

pub fn get_circuit_breaker_window_start(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::V1CircuitBreakerWindowStart)
        .unwrap_or(0)
}

pub fn set_circuit_breaker_window_start(env: &Env, start: u32) {
    env.storage()
        .instance()
        .set(&DataKey::V1CircuitBreakerWindowStart, &start);
}

pub fn get_circuit_breaker_window_outflow(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::V1CircuitBreakerWindowOutflow)
        .unwrap_or(0)
}

pub fn set_circuit_breaker_window_outflow(env: &Env, outflow: i128) {
    env.storage()
        .instance()
        .set(&DataKey::V1CircuitBreakerWindowOutflow, &outflow);
}

pub fn get_multi_admin_config(env: &Env) -> Option<MultiAdminConfig> {
    env.storage().instance().get(&DataKey::V1MultiAdminConfig)
}

pub fn set_multi_admin_config(env: &Env, config: &MultiAdminConfig) {
    env.storage()
        .instance()
        .set(&DataKey::V1MultiAdminConfig, config);
}

pub fn get_admin_approvals(env: &Env, action_id: &String) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::V1AdminApprovals(action_id.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_admin_approvals(env: &Env, action_id: &String, approvals: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&DataKey::V1AdminApprovals(action_id.clone()), approvals);
}

pub fn remove_admin_approvals(env: &Env, action_id: &String) {
    env.storage()
        .persistent()
        .remove(&DataKey::V1AdminApprovals(action_id.clone()));
}

pub fn get_pending_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::V1PendingAdmin)
}

pub fn set_pending_admin(env: &Env, nominee: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::V1PendingAdmin, nominee);
}

pub fn remove_pending_admin(env: &Env) {
    env.storage().instance().remove(&DataKey::V1PendingAdmin);
}

// ============================================================
// AMENDMENT / ARBITER ROTATION (temporary storage)
// ============================================================

pub fn get_amendment(
    env: &Env,
    shipment_id: &String,
    index: u32,
) -> Option<crate::AmendmentProposal> {
    env.storage()
        .temporary()
        .get(&DataKey::V1Amendment(shipment_id.clone(), index))
}

pub fn set_amendment(
    env: &Env,
    shipment_id: &String,
    index: u32,
    proposal: &crate::AmendmentProposal,
) {
    env.storage()
        .temporary()
        .set(&DataKey::V1Amendment(shipment_id.clone(), index), proposal);
}

pub fn remove_amendment(env: &Env, shipment_id: &String, index: u32) {
    env.storage()
        .temporary()
        .remove(&DataKey::V1Amendment(shipment_id.clone(), index));
}

pub fn get_arbiter_rotation(
    env: &Env,
    shipment_id: &String,
) -> Option<crate::ArbiterRotationProposal> {
    env.storage()
        .temporary()
        .get(&DataKey::V1ArbiterRotation(shipment_id.clone()))
}

pub fn set_arbiter_rotation(
    env: &Env,
    shipment_id: &String,
    proposal: &crate::ArbiterRotationProposal,
) {
    env.storage()
        .temporary()
        .set(&DataKey::V1ArbiterRotation(shipment_id.clone()), proposal);
}

pub fn remove_arbiter_rotation(env: &Env, shipment_id: &String) {
    env.storage()
        .temporary()
        .remove(&DataKey::V1ArbiterRotation(shipment_id.clone()));
}

// ============================================================
// MIGRATION STUB  (#77)
// Call once after a contract upgrade that introduces V2_* keys.
// Currently a no-op; add data-model transformation logic here.
// ============================================================

/// Migrate persistent state from V1 schema to V2 schema.
///
/// Pattern:
/// 1. Read each V1_* key.
/// 2. Transform the value if the data model changed.
/// 3. Write to the corresponding V2_* key.
/// 4. Optionally remove the V1_* key to reclaim storage rent.
///
/// This function is idempotent — safe to call multiple times.
pub fn migrate_v1_to_v2(_env: &Env) {
    // No-op for current version.
    // Example future implementation:
    //
    // let ids = get_all_shipments(env);
    // for i in 0..ids.len() {
    //     let id = ids.get(i).unwrap();
    //     if let Some(v1) = get_shipment(env, &id) {
    //         let v2 = transform_shipment_v1_to_v2(v1);
    //         env.storage().persistent().set(&DataKey::V2Shipment(id.clone()), &v2);
    //         env.storage().persistent().remove(&DataKey::V1Shipment(id));
    //     }
    // }
}
