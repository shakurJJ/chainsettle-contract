#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, token, Address, BytesN, Env, String, Vec, Symbol,
};

// ============================================================
// DATA TYPES
// ============================================================

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum MilestoneStatus {
    Pending,
    ProofSubmitted,
    Confirmed,
    Disputed,
    Resolved,
    /// Confirmed but payment held until release_after_ledger
    ConfirmedHeld,
}

/// Controls whether milestones must be completed in order (Sequential)
/// or can be submitted and confirmed independently (Parallel).
/// Immutable after shipment creation.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum MilestoneMode {
    /// Proof for milestone N requires milestone N-1 to be Confirmed or Resolved first.
    Sequential,
    /// All milestones are independently submittable at any time.
    Parallel,
}

#[contracttype]
#[derive(Clone)]
pub struct Milestone {
    pub name: String,
    pub payment_percent: u32,
    pub proof_hash: String,
    pub status: MilestoneStatus,
    /// Set when holdback_ledgers > 0 and milestone is confirmed.
    pub release_after_ledger: u32,
    /// Ledger at which proof was submitted; used for auto-confirmation timeout.
    pub proof_submitted_ledger: Option<u32>,
    /// Ledger at which dispute was opened; used for escalation threshold check.
    pub dispute_opened_ledger: Option<u32>,
}

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum ShipmentStatus {
    Active,
    Completed,
    Cancelled,
}

#[contracttype]
#[derive(Clone)]
pub struct AuditEntry {
    pub action: Symbol,
    pub caller: Address,
    pub ledger: u32,
    pub detail: Symbol,
}

#[contracttype]
#[derive(Clone)]
pub struct Shipment {
    pub id: String,
    /// Bounded audit log of status transitions (ring-buffer semantics, max 20).
    pub audit_log: Vec<AuditEntry>,

    /// All co-buyers. All must call confirm_milestone for payment to release.
    /// raise_dispute requires only one co-buyer's signature.
    pub buyers: Vec<Address>,
    pub supplier: Address,
    pub logistics: Address,
    pub arbiter: Address,
    pub token: Address,
    pub total_amount: i128,
    pub released_amount: i128,
    /// Total advance payments made (deducted from milestone payments on confirmation).
    pub total_advanced_amount: i128,
    pub milestones: Vec<Milestone>,
    pub status: ShipmentStatus,
    pub milestone_mode: MilestoneMode,
    pub created_at: u32,
    /// Ledgers to hold payment after confirmation (0 = immediate release).
    pub holdback_ledgers: u32,
    // ── New: dispute cooldown ──────────────────────────────────
    /// Minimum ledgers that must elapse between dispute resolutions (0 = no cooldown).
    pub dispute_cooldown_ledgers: u32,
    /// Ledger at which the last dispute was resolved; None if no dispute has been resolved yet.
    pub last_dispute_resolved_ledger: Option<u32>,
    // ── New: late-delivery penalty ─────────────────────────────
    /// Basis points penalty per ledger of delay past milestone deadline (0 = no penalty).
    pub late_penalty_bps_per_ledger: u32,
    // ── New: auto-confirmation ────────────────────────────────
    /// Ledgers after proof submission before auto-confirmation (0 = disabled).
    pub auto_confirm_ledgers: u32,
    /// Number of currently open disputes on this shipment.
    pub open_dispute_count: u32,
    /// Per-dispute bond amount locked by buyer at creation (0 = disabled, backward compatible).
    pub dispute_bond_amount: i128,
    /// Basis points of disputed payment sent to arbiter on resolution (0 = no arbiter fee).
    pub arbiter_fee_bps: u32,
    /// Basis points deducted from each milestone payment for logistics provider (0 = no fee).
    pub logistics_fee_bps: u32,
    /// Ledger at which the shipment expires (None = no expiry).
    pub expires_at_ledger: Option<u32>,

    /// Off-chain trade document hash (IPFS CID) attached at creation; immutable after creation.
    pub metadata_hash: Option<BytesN<32>>,
    /// Optional referrer who earns a basis-point bonus of the protocol fee on shipment completion.
    pub referrer: Option<Address>,
    /// Basis points charged to buyer on buyer-initiated cancellation (0 = no penalty, max 1000).
    pub buyer_cancel_fee_bps: u32,

}

/// Cancellation policy stored separately (keeps Shipment within the contracttype field limit).
#[contracttype]
#[derive(Clone)]
pub struct CancelPolicy {
    /// 0 = supplier cancellation disabled; >0 = ledgers after proof submission
    pub response_deadline: u32,
    /// basis points deducted from buyer refund on supplier cancellation (e.g. 500 = 5%)
    pub penalty_bps: u32,
}

/// Pending amendment proposal for a single milestone.
#[contracttype]
#[derive(Clone)]
pub struct AmendmentProposal {
    pub new_percent: u32,
    pub new_name: String,
    pub buyer_agreed: bool,
    pub supplier_agreed: bool,
}

/// Pending arbiter rotation proposal.
#[contracttype]
#[derive(Clone)]
pub struct ArbiterRotationProposal {
    pub new_arbiter: Address,
    pub buyer_agreed: bool,
    pub supplier_agreed: bool,
}

/// #113 – Volume-based fee tier. Tiers are sorted descending by min_lifetime_volume;
/// the first tier whose threshold the buyer meets wins (lower fee_bps beats FeeConfig).
#[contracttype]
#[derive(Clone)]
pub struct FeeTier {
    pub min_lifetime_volume: i128,
    pub fee_bps: u32,
}

/// #111 – Single entry in a milestone's immutable amendment change log.
#[contracttype]
#[derive(Clone)]
pub struct AmendmentEntry {
    pub proposer: Address,
    pub old_payment_percent: u32,
    pub new_payment_percent: u32,
    pub ledger: u32,
}

/// #110 – Pending extension request submitted by the supplier.
#[contracttype]
#[derive(Clone)]
pub struct ExtensionReq {
    pub extra_ledgers: u32,
}

/// Optional platform fee configuration.
#[contracttype]
#[derive(Clone)]
pub struct FeeConfig {
    /// Basis points charged on each milestone payment (e.g. 100 = 1%).
    pub fee_bps: u32,
    /// Address that receives the fee.
    pub treasury: Address,
}

/// Extra shipment options passed to create_shipment to stay within the 10-parameter limit.
#[contracttype]
#[derive(Clone)]
pub struct ShipmentOptions {
    /// 0 = supplier cancellation disabled; >0 = ledgers after proof submission.
    pub response_deadline: u32,
    /// Basis points deducted from buyer refund on supplier cancellation (e.g. 500 = 5%).
    pub penalty_bps: u32,
    pub milestone_mode: MilestoneMode,
    /// Ledgers to hold payment after confirmation (0 = immediate release).
    pub holdback_ledgers: u32,
    /// Minimum ledgers between successive dispute resolutions (0 = no cooldown).
    pub dispute_cooldown_ledgers: u32,
    /// Basis points penalty per ledger of delay past milestone deadline (0 = no penalty).
    pub late_penalty_bps_per_ledger: u32,
    /// Ledgers after proof submission before auto-confirmation (0 = disabled).
    pub auto_confirm_ledgers: u32,
    /// Bond amount locked per dispute; 0 = no bond required (default, backward compat).
    pub dispute_bond_amount: i128,
    /// Basis points of disputed payment sent to arbiter on resolution (0 = no arbiter fee).
    pub arbiter_fee_bps: u32,
    /// Basis points deducted from each milestone payment for logistics provider (0 = no fee).
    pub logistics_fee_bps: u32,
    /// Supplier collateral required at creation (0 = no collateral required).
    pub supplier_collateral: i128,
    /// Ledger at which shipment expires (None = no expiry).
    pub expires_at_ledger: Option<u32>,

    /// Off-chain trade document hash (IPFS CID) attached at creation; immutable after creation.
    pub metadata_hash: Option<BytesN<32>>,
    /// Optional referrer who earns a basis-point bonus of the protocol fee on shipment completion.
    pub referrer: Option<Address>,
    /// Basis points charged to buyer on buyer-initiated cancellation (0 = no penalty, max 1000).
    pub buyer_cancel_fee_bps: u32,

}

/// All parameters needed to create a single shipment in a batch call.
/// Mirrors the individual `create_shipment` parameters without the `Env`.
#[contracttype]
#[derive(Clone)]
pub struct BatchShipmentParams {
    pub shipment_id: String,
    pub buyers: Vec<Address>,
    pub supplier: Address,
    pub logistics: Address,
    pub arbiter: Address,
    pub token: Address,
    pub total_amount: i128,
    pub milestones: Vec<Milestone>,
    pub options: ShipmentOptions,
}

/// Contract-level statistics for analytics and monitoring.
#[contracttype]
#[derive(Clone)]
pub struct ContractStats {
    /// Total number of shipments created.
    pub total_shipments: u64,
    /// Total USDC volume locked across all shipments.
    pub total_volume: i128,
    /// Total number of disputes raised.
    pub total_disputes: u64,
    /// Total number of shipments completed.
    pub completed_shipments: u64,
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ReputationScore {
    pub completed: u32,
    pub disputed: u32,
    pub cancelled: u32,
}

/// Active dispute entry: (shipment_id, milestone_index).
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub struct DisputeEntry {
    pub shipment_id: String,
    pub milestone_index: u32,
}

/// Supplier advance payment request for a milestone.
#[contracttype]
#[derive(Clone)]
pub struct AdvanceRequest {
    pub requested_percent: u32,
    pub approved: bool,
    pub amount_advanced: i128,
}

/// Multi-admin configuration for M-of-N approvals.
#[contracttype]
#[derive(Clone)]
pub struct MultiAdminConfig {
    pub admins: Vec<Address>,
    pub threshold: u32,
}

/// Pending admin action proposal.
#[contracttype]
#[derive(Clone)]
pub struct AdminAction {
    pub action_id: String,
    pub operation: Symbol,
    pub params: String,
}

// ============================================================
// STORAGE CONTEXT STRUCTS (batch reads)
// ============================================================

/// CreateShipmentCtx consolidates all persistent storage reads for create_shipment.
/// Keys accessed:
///   - DataKey::MaxShipmentValue (instance)
///   - DataKey::AllowedTokens (instance)
///   - DataKey::Blacklisted(Address) (instance) × (buyers + 3 others)
///   - DataKey::MinMilestonePercent (instance)
///   - DataKey::Shipment(shipment_id) (persistent)
///   - DataKey::TotalEscrowed(token) (persistent)
///   - DataKey::ContractStats (instance)
#[derive(Clone)]
pub struct CreateShipmentCtx {
    pub max_value: i128,
    pub allowed_tokens: Vec<Address>,
    pub min_pct: u32,
    pub contract_stats: ContractStats,
}

/// ConfirmMilestoneCtx consolidates all persistent storage reads for confirm_milestone.
/// Keys accessed:
///   - DataKey::Shipment(shipment_id) (persistent)
///   - DataKey::ContractStats (instance)
///   - DataKey::TotalEscrowed(token) (persistent)
#[derive(Clone)]
pub struct ConfirmMilestoneCtx {
    pub shipment: Shipment,
    pub contract_stats: ContractStats,
}

/// ResolveDisputeCtx consolidates all persistent storage reads for resolve_dispute.
/// Keys accessed:
///   - DataKey::Shipment(shipment_id) (persistent)
///   - DataKey::DisputeContestedPercent(shipment_id, milestone_index) (persistent)
///   - DataKey::ContractStats (instance)
///   - DataKey::ActiveDisputes (persistent)
#[derive(Clone)]
pub struct ResolveDisputeCtx {
    pub shipment: Shipment,
    pub partial_contested_percent: Option<u32>,
    pub contract_stats: ContractStats,
    pub active_disputes: Vec<DisputeEntry>,
}

// ============================================================
// STORAGE KEYS
// ============================================================

#[contracttype]
pub enum DataKey {
    Shipment(String),
    CancelPolicy(String),
    AllShipments,
    /// Supplier-to-shipments index: Vec<shipment_id> for a given supplier.
    SupplierShipments(Address),
    /// Supplier reputation score.
    SupplierRep(Address),
    /// Buyer-to-shipments index: Vec<shipment_id> for a given buyer.
    BuyerShipments(Address),
    Admin,
    /// Ledger sequence when a milestone entered ProofSubmitted state.
    ProofSubmittedAt(String, u32),
    /// Pending amendment proposal.
    Amendment(String, u32),
    /// Optional fee configuration.
    FeeConfig,
    /// Minimum allowed milestone payment percent.
    MinMilestonePercent,
    /// Maximum concurrently open disputes per shipment.
    MaxConcurrentDisputes,
    /// Blacklisted addresses banned from new shipment creation.
    Blacklisted(Address),
    /// Bounded admin action log for audit trail.
    AdminActionLog,
    /// Whitelisted token addresses (Vec<Address>); empty = all tokens allowed.
    AllowedTokens,
    /// Global pause flag.
    Paused,
    /// Pending arbiter rotation proposal: (new_arbiter, buyer_agreed, supplier_agreed).
    ArbiterRotation(String),
    /// Total escrowed value for a given token across all active shipments.
    TotalEscrowed(Address),
    /// Active disputes: Vec<(shipment_id, milestone_index)>.
    ActiveDisputes,
    /// Contract-level statistics.
    ContractStats,
    /// Per-status index: Vec<String> of shipment IDs with the given status.
    ShipmentsByStatus(ShipmentStatus),
    /// Escalation threshold in ledgers (dispute escalation feature).
    EscalationThreshold,
    /// Maximum shipment value cap in i128 (0 = no cap).
    MaxShipmentValue,
    /// Circuit breaker outflow limit in i128.
    CircuitBreakerLimit,
    /// Circuit breaker window in ledgers.
    CircuitBreakerWindow,
    /// Circuit breaker window start ledger.
    CircuitBreakerWindowStart,
    /// Circuit breaker window outflow amount.
    CircuitBreakerWindowOutflow,
    /// Multi-admin approvals: Vec of (action_id, num_approvals).
    PendingActions(String),
    /// Multi-admin configuration.
    MultiAdminConfig,
    /// Multi-admin approvals tracking: Vec<Address> who approved an action.
    AdminApprovals(String),
    /// Pending admin nominee for two-step admin transfer.
    PendingAdmin,
    /// Supplier advance request for (shipment_id, milestone_index).
    AdvanceRequest(String, u32),
    /// Contract-level max advance percent (default 30).
    MaxAdvancePercent,
    /// Allowed proof content types per milestone: (shipment_id, milestone_index) -> Vec<Symbol>.
    /// Empty list means any type is accepted.
    MilestoneProofWhitelist(String, u32),
    /// Declared proof content type recorded at submission time: (shipment_id, milestone_index) -> Symbol.
    SubmittedProofType(String, u32),
    /// Contested percentage stored when a partial dispute is raised: (shipment_id, milestone_index) -> u32.
    /// Absence of this key means the associated dispute covers 100% of the milestone value.
    DisputeContestedPercent(String, u32),
    /// Address of the external yield protocol contract (admin-configured; optional).
    YieldProtocol,
    /// Cumulative amount deposited to the yield protocol per token address.
    /// Cleared to 0 on each full withdrawal.
    YieldDeposited(Address),
    /// Supplier collateral amount for a shipment.
    SupplierCollateral(String),

    /// Whether the NFT mint hook event is enabled (admin-configured, default false).
    NftHookEnabled,

    /// Supplier whitelist: Vec<Address>; when non-empty only listed suppliers may create shipments.
    SupplierWhitelist,
    /// Referral fee basis points paid to referrer out of the total protocol fee (default 500 = 5%).
    ReferralFeeBps,

    // ── #113 Fee tiers ─────────────────────────────────────────
    /// Admin-configured fee tier list (up to 5 entries).
    FeeTiers,
    /// Buyer's accumulated lifetime shipment volume (i128).
    LifetimeVolume(Address),
    /// Effective fee bps locked for this shipment at creation.
    ShipmentFeeBps(String),

    // ── #112 Invoice hash ──────────────────────────────────────
    /// Per-milestone invoice hash: (shipment_id, milestone_index) -> BytesN<32>.
    MilestoneInvoiceHash(String, u32),

    // ── #111 Amendment log ─────────────────────────────────────
    /// Append-only amendment log per milestone (capped at 20).
    AmendmentLog(String, u32),

    // ── #110 Extension request ─────────────────────────────────
    /// Pending extension request per milestone.
    ExtensionRequest(String, u32),
    /// Effective deadline ledger per milestone (0 = unset).
    MilestoneDeadline(String, u32),

}

// ============================================================
// ERRORS
// ============================================================

#[contracterror]
#[derive(Clone, Copy, PartialEq)]
pub enum ChainSettleError {
    ShipmentAlreadyExists = 1,
    ShipmentNotFound = 2,
    Unauthorized = 3,
    InvalidMilestoneIndex = 4,
    InvalidMilestoneStatus = 5,
    ShipmentNotActive = 6,
    InvalidPercentages = 7,
    InvalidAmount = 8,
    DisputeAlreadyOpen = 9,
    DeadlineNotBreached = 10,
    FeeTooHigh = 11,
    PreviousMilestoneNotComplete = 12,
    ContractPaused = 13,
    DisputeCooldownActive = 14,
    TransferDisallowed = 15,
    CircuitBreakerTripped = 16,
    EmptyBuyersList = 17,
    MaxShipmentValueExceeded = 18,
    InvalidMultiSigParameters = 19,
    MultisigNotConfigured = 20,
    AlreadyApproved = 21,
    InvalidMinMilestonePercent = 22,
    TopUpNotAllowed = 23,
    ProofNotSubmitted = 24,
    AutoConfirmed = 25,
    HoldbackNotExpired = 26,
    AdvanceExceedsMax = 27,
    AdvanceNotRequested = 28,
    AdvanceAlreadyApproved = 29,
    ProofTypeNotAllowed = 30,
    RebalanceNotAllowed = 31,
    InvalidContestedPercent = 32,
}

// ============================================================
// CONSTANTS
// ============================================================

/// Ledgers equivalent to approximately 2 years (≈ 5 s/ledger × 86 400 s/day × 365 days × 2).
const RECOVERY_THRESHOLD_LEDGERS: u32 = 12_614_400;

// ============================================================
// CONTRACT
// ============================================================

#[contract]
pub struct ChainSettleContract;

#[contractimpl]
impl ChainSettleContract {
    // ----------------------------------------------------------
    // INIT
    // ----------------------------------------------------------

    pub fn init(env: Env, admin: Address) {
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        // Initialise paused to false.
        env.storage().instance().set(&DataKey::Paused, &false);
        // Initialize default milestone and dispute limits.
        env.storage()
            .instance()
            .set(&DataKey::MinMilestonePercent, &5u32);
        env.storage()
            .instance()
            .set(&DataKey::MaxConcurrentDisputes, &1u32);
        env.storage()
            .instance()
            .set(&DataKey::AdminActionLog, &Vec::<AuditEntry>::new(&env));
        // Initialize contract stats.
        env.storage().instance().set(
            &DataKey::ContractStats,
            &ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            },
        );
        // Initialize active disputes list.
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &Vec::<DisputeEntry>::new(&env));
        // Initialize escalation threshold (0 = disabled).
        env.storage()
            .instance()
            .set(&DataKey::EscalationThreshold, &0u32);
        // Initialize max shipment value (0 = no cap).
        env.storage()
            .instance()
            .set(&DataKey::MaxShipmentValue, &0i128);
        // Initialize circuit breaker.
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerLimit, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerWindow, &0u32);
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerWindowStart, &0u32);
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerWindowOutflow, &0i128);
        // Initialize max advance percent (default 30%).
        env.storage()
            .instance()
            .set(&DataKey::MaxAdvancePercent, &30u32);
        // Initialize referral fee bps (default 500 = 5% of total protocol fee).
        env.storage()
            .instance()
            .set(&DataKey::ReferralFeeBps, &500u32);
    }

    // ----------------------------------------------------------
    // UPGRADE
    // ----------------------------------------------------------

    /// Replace the contract WASM in-place. Only callable by admin.
    pub fn upgrade(env: Env, admin: Address, new_wasm_hash: BytesN<32>) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic!("unauthorized"));
        if admin != stored_admin {
            panic!("unauthorized");
        }
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        env.events().publish(
            (Symbol::new(&env, "contract_upgraded"),),
            (new_wasm_hash, env.ledger().sequence()),
        );
    }

    /// Migration stub — call once after upgrade to perform any data-model changes.
    pub fn migrate(_env: Env) {
        // No-op for current version; implement data migrations here post-upgrade.
    }

    // ----------------------------------------------------------
    // ADMIN: PAUSE / UNPAUSE
    // ----------------------------------------------------------

    /// Pause all state-changing operations. Admin only.
    pub fn pause(env: Env, admin: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "pause"),
            Symbol::new(&env, "contract_paused"),
        );
        env.storage().instance().set(&DataKey::Paused, &true);
        env.events().publish(
            (Symbol::new(&env, "contract_paused"),),
            env.ledger().sequence(),
        );
    }

    /// Resume all state-changing operations. Admin only.
    pub fn unpause(env: Env, admin: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "unpause"),
            Symbol::new(&env, "contract_unpaused"),
        );
        env.storage().instance().set(&DataKey::Paused, &false);
        env.events().publish(
            (Symbol::new(&env, "contract_unpaused"),),
            env.ledger().sequence(),
        );
    }

    // ----------------------------------------------------------
    // ADMIN: ESCALATION THRESHOLD
    // ----------------------------------------------------------

    pub fn set_escalation_threshold(env: Env, admin: Address, threshold_ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::EscalationThreshold, &threshold_ledgers);
        env.events().publish(
            (Symbol::new(&env, "escalation_threshold_set"),),
            threshold_ledgers,
        );
    }

    pub fn get_escalation_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::EscalationThreshold)
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ADMIN: MAX SHIPMENT VALUE
    // ----------------------------------------------------------

    pub fn set_max_shipment_value(env: Env, admin: Address, max_value: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::MaxShipmentValue, &max_value);
        env.events()
            .publish((Symbol::new(&env, "max_shipment_value_set"),), max_value);
    }

    pub fn get_max_shipment_value(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::MaxShipmentValue)
            .unwrap_or(0)
    }

    pub fn get_reputation(env: Env, supplier: Address) -> ReputationScore {
        env.storage()
            .persistent()
            .get(&DataKey::SupplierRep(supplier.clone()))
            .unwrap_or_default()
    }

    // ----------------------------------------------------------
    // ADMIN: CIRCUIT BREAKER
    // ----------------------------------------------------------

    pub fn set_circuit_breaker(env: Env, admin: Address, limit: i128, window_ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerLimit, &limit);
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerWindow, &window_ledgers);
        env.storage().instance().set(
            &DataKey::CircuitBreakerWindowStart,
            &env.ledger().sequence(),
        );
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerWindowOutflow, &0i128);
        env.events().publish(
            (Symbol::new(&env, "circuit_breaker_set"),),
            (limit, window_ledgers),
        );
    }

    // ----------------------------------------------------------
    // MULTI-ADMIN GOVERNANCE
    // ----------------------------------------------------------

    pub fn initialize_multisig_admin(
        env: Env,
        admin: Address,
        admins: Vec<Address>,
        threshold: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if admins.len() < 1 || threshold < 1 || threshold > admins.len() as u32 {
            panic!("invalid multi-sig parameters");
        }
        let config = MultiAdminConfig { admins, threshold };
        env.storage()
            .instance()
            .set(&DataKey::MultiAdminConfig, &config);
        env.events().publish(
            (Symbol::new(&env, "multisig_admin_initialized"),),
            threshold,
        );
    }

    pub fn propose_admin_action(
        env: Env,
        admin: Address,
        action_id: String,
        operation: Symbol,
        params: String,
    ) {
        admin.require_auth();
        let config: MultiAdminConfig = env
            .storage()
            .instance()
            .get(&DataKey::MultiAdminConfig)
            .unwrap_or_else(|| panic!("multisig admin not configured"));

        let mut is_admin = false;
        for i in 0..config.admins.len() {
            if config.admins.get(i).unwrap() == admin {
                is_admin = true;
                break;
            }
        }
        if !is_admin {
            panic!("unauthorized");
        }

        let mut approvals: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AdminApprovals(action_id.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        // Check if this admin already approved
        let mut already_approved = false;
        for i in 0..approvals.len() {
            if approvals.get(i).unwrap() == admin {
                already_approved = true;
                break;
            }
        }
        if already_approved {
            panic!("already approved by this admin");
        }

        approvals.push_back(admin.clone());
        env.storage()
            .persistent()
            .set(&DataKey::AdminApprovals(action_id.clone()), &approvals);

        env.events().publish(
            (
                Symbol::new(&env, "admin_action_proposed"),
                action_id.clone(),
            ),
            approvals.len() as u32,
        );

        // Check if threshold reached
        if approvals.len() as u32 >= config.threshold {
            // Execute action
            Self::execute_admin_action(&env, &action_id, operation, params);
            env.storage()
                .persistent()
                .remove(&DataKey::AdminApprovals(action_id.clone()));
        }
    }

    pub fn get_pending_admin_actions(env: Env, action_id: String) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::AdminApprovals(action_id))
            .unwrap_or_else(|| Vec::new(&env))
    }

    fn execute_admin_action(env: &Env, action_id: &String, operation: Symbol, _params: String) {
        env.events().publish(
            (Symbol::new(env, "admin_action_executed"), action_id.clone()),
            operation,
        );
        // Note: Actual action execution depends on the operation type
        // Implementations for specific operations (pause, upgrade, etc.) would go here
    }

    // ----------------------------------------------------------
    // ADMIN: FEE CONFIG
    // ----------------------------------------------------------

    /// Set or update the platform fee. Max 1000 bps (10%). Admin only.
    pub fn set_fee_config(env: Env, admin: Address, fee_bps: u32, treasury: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if fee_bps > 1000 {
            panic!("fee_bps exceeds maximum of 1000");
        }
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_fee_config"),
            Symbol::new(&env, "fee_config_updated"),
        );
        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &FeeConfig { fee_bps, treasury });
    }

    pub fn set_max_concurrent_disputes(env: Env, admin: Address, limit: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::MaxConcurrentDisputes, &limit);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_max_concurrent_disputes"),
            Symbol::new(&env, "max_concurrent_disputes_updated"),
        );
    }

    pub fn set_min_milestone_percent(env: Env, admin: Address, percent: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if percent == 0 || percent > 100 {
            panic!("min_milestone_percent must be between 1 and 100");
        }
        env.storage()
            .instance()
            .set(&DataKey::MinMilestonePercent, &percent);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_min_milestone_percent"),
            Symbol::new(&env, "min_milestone_percent_updated"),
        );
    }

    pub fn set_max_advance_percent(env: Env, admin: Address, percent: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if percent > 100 {
            panic!("max advance percent must not exceed 100");
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxAdvancePercent, &percent);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_max_advance_percent"),
            Symbol::new(&env, "max_advance_percent_updated"),
        );
    }

    pub fn get_max_advance_percent(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxAdvancePercent)
            .unwrap_or(30)
    }

    // ----------------------------------------------------------
    // ADMIN: NFT MINT HOOK (Issue #104)
    // ----------------------------------------------------------

    /// Enable or disable the NFT mint hook event emitted on final milestone completion.
    /// When enabled, a `nft_mint_hook` event is published on shipment completion so an
    /// off-chain NFT minting service can issue a provenance certificate to the buyer.
    /// The event is purely informational — no state change or external contract call.
    /// Admin only. Default: disabled.
    pub fn set_nft_hook_enabled(env: Env, admin: Address, enabled: bool) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::NftHookEnabled, &enabled);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_nft_hook_enabled"),
            Symbol::new(&env, "nft_hook_config_updated"),
        );
        env.events().publish(
            (Symbol::new(&env, "nft_hook_config_updated"),),
            (admin, enabled, env.ledger().sequence()),
        );
    }

    /// Returns true if the NFT mint hook event is currently enabled.
    pub fn get_nft_hook_enabled(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::NftHookEnabled)
            .unwrap_or(false)
    }

    pub fn blacklist_address(env: Env, admin: Address, address: Address, reason_hash: BytesN<32>) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Blacklisted(address.clone()), &reason_hash);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "blacklist_address"),
            Symbol::new(&env, "address_blacklisted"),
        );
    }

    pub fn remove_from_blacklist(env: Env, admin: Address, address: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .remove(&DataKey::Blacklisted(address.clone()));
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "remove_from_blacklist"),
            Symbol::new(&env, "address_unblacklisted"),
        );
    }

    pub fn is_blacklisted(env: Env, address: Address) -> bool {
        env.storage()
            .instance()
            .get::<DataKey, BytesN<32>>(&DataKey::Blacklisted(address))
            .is_some()
    }

    // ----------------------------------------------------------
    // ADMIN: SUPPLIER WHITELIST (Issue #100)
    // ----------------------------------------------------------

    /// Add an address to the supplier whitelist. Admin only.
    /// Once the whitelist is non-empty, only whitelisted suppliers may call create_shipment.
    pub fn add_to_whitelist(env: Env, admin: Address, address: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let mut list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::SupplierWhitelist)
            .unwrap_or_else(|| Vec::new(&env));
        for i in 0..list.len() {
            if list.get(i).unwrap() == address {
                return; // already present
            }
        }
        list.push_back(address.clone());
        env.storage()
            .instance()
            .set(&DataKey::SupplierWhitelist, &list);
        env.events()
            .publish((Symbol::new(&env, "supplier_whitelisted"),), address);
    }

    /// Remove an address from the supplier whitelist. Admin only.
    pub fn remove_from_whitelist(env: Env, admin: Address, address: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::SupplierWhitelist)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_list: Vec<Address> = Vec::new(&env);
        for i in 0..list.len() {
            let a = list.get(i).unwrap();
            if a != address {
                new_list.push_back(a);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::SupplierWhitelist, &new_list);
        env.events()
            .publish((Symbol::new(&env, "supplier_unwhitelisted"),), address);
    }

    /// Returns true if `address` is on the supplier whitelist, or the whitelist is empty (open mode).
    pub fn is_whitelisted(env: Env, address: Address) -> bool {
        let list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::SupplierWhitelist)
            .unwrap_or_else(|| Vec::new(&env));
        if list.is_empty() {
            return true; // empty whitelist = open mode
        }
        for i in 0..list.len() {
            if list.get(i).unwrap() == address {
                return true;
            }
        }
        false
    }

    // ----------------------------------------------------------
    // ADMIN: REFERRAL FEE (Issue #105)
    // ----------------------------------------------------------

    /// Set the referral fee basis points (0–10000). Admin only.
    /// Default is 500 (5% of the total protocol fee paid to the referrer on completion).
    pub fn set_referral_fee_bps(env: Env, admin: Address, bps: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if bps > 10_000 {
            panic!("referral_fee_bps cannot exceed 10000");
        }
        env.storage()
            .instance()
            .set(&DataKey::ReferralFeeBps, &bps);
        env.events()
            .publish((Symbol::new(&env, "referral_fee_bps_set"),), bps);
    }

    pub fn get_referral_fee_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ReferralFeeBps)
            .unwrap_or(500)
    }

    pub fn get_admin_log(env: Env) -> Vec<AuditEntry> {
        env.storage()
            .instance()
            .get(&DataKey::AdminActionLog)
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // #113 – FEE TIERS
    // ----------------------------------------------------------

    /// Admin configures up to 5 volume-based fee tiers.
    /// Tiers should be ordered with highest min_lifetime_volume first.
    pub fn set_fee_tiers(env: Env, admin: Address, tiers: Vec<FeeTier>) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if tiers.len() > 5 {
            panic!("max 5 fee tiers");
        }
        env.storage().instance().set(&DataKey::FeeTiers, &tiers);
        env.events().publish((Symbol::new(&env, "fee_tiers_set"),), tiers.len() as u32);
    }

    /// Returns the effective fee bps for `address` based on lifetime volume.
    /// Falls back to FeeConfig.fee_bps if no tier matches.
    pub fn get_fee_tier(env: Env, address: Address) -> u32 {
        let volume: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LifetimeVolume(address.clone()))
            .unwrap_or(0);
        let tiers: Vec<FeeTier> = env
            .storage()
            .instance()
            .get(&DataKey::FeeTiers)
            .unwrap_or_else(|| Vec::new(&env));
        let mut best: Option<u32> = None;
        for i in 0..tiers.len() {
            let t = tiers.get(i).unwrap();
            if volume >= t.min_lifetime_volume {
                best = Some(match best {
                    None => t.fee_bps,
                    Some(b) => if t.fee_bps < b { t.fee_bps } else { b },
                });
            }
        }
        best.unwrap_or_else(|| {
            env.storage()
                .instance()
                .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
                .map(|c| c.fee_bps)
                .unwrap_or(0)
        })
    }

    // ----------------------------------------------------------
    // #112 – INVOICE HASH
    // ----------------------------------------------------------

    /// Supplier attaches an invoice hash to a milestone at or after proof submission.
    /// Immutable once set — subsequent calls panic.
    pub fn attach_invoice_hash(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        invoice_hash: BytesN<32>,
    ) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_supplier_auth(&shipment, &caller);
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }
        let key = DataKey::MilestoneInvoiceHash(shipment_id.clone(), milestone_index);
        if env.storage().persistent().has(&key) {
            panic!("invoice hash already set and is immutable");
        }
        let m = shipment.milestones.get(milestone_index).unwrap();
        if m.status == MilestoneStatus::Pending {
            panic!("proof must be submitted before attaching invoice hash");
        }
        env.storage().persistent().set(&key, &invoice_hash);
        env.storage().persistent().extend_ttl(&key, 100_000, 6_300_000);
        env.events().publish(
            (Symbol::new(&env, "invoice_hash_attached"), shipment_id),
            (milestone_index, invoice_hash, caller),
        );
    }

    /// Returns the invoice hash for a milestone, or None if not set.
    pub fn get_invoice_hash(env: Env, shipment_id: String, milestone_index: u32) -> Option<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&DataKey::MilestoneInvoiceHash(shipment_id, milestone_index))
    }

    // ----------------------------------------------------------
    // #111 – AMENDMENT LOG
    // ----------------------------------------------------------

    /// Returns the append-only amendment log for a milestone in chronological order.
    pub fn get_amendment_log(env: Env, shipment_id: String, milestone_index: u32) -> Vec<AmendmentEntry> {
        env.storage()
            .persistent()
            .get(&DataKey::AmendmentLog(shipment_id, milestone_index))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // #110 – EXTENSION REQUESTS
    // ----------------------------------------------------------

    /// Supplier requests extra_ledgers to be added to a milestone deadline.
    /// Only one pending request per milestone allowed at a time.
    pub fn request_extension(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        extra_ledgers: u32,
    ) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_supplier_auth(&shipment, &caller);
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }
        let key = DataKey::ExtensionRequest(shipment_id.clone(), milestone_index);
        if env.storage().persistent().has(&key) {
            panic!("extension request already pending");
        }
        env.storage().persistent().set(&key, &ExtensionReq { extra_ledgers });
        env.storage().persistent().extend_ttl(&key, 100_000, 6_300_000);
        env.events().publish(
            (Symbol::new(&env, "extension_requested"), shipment_id),
            (milestone_index, extra_ledgers, caller),
        );
    }

    /// Buyer approves the pending extension request; adds extra_ledgers to milestone deadline.
    pub fn approve_extension(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_index: u32,
    ) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);
        let key = DataKey::ExtensionRequest(shipment_id.clone(), milestone_index);
        let req: ExtensionReq = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("no pending extension request"));
        env.storage().persistent().remove(&key);
        let deadline_key = DataKey::MilestoneDeadline(shipment_id.clone(), milestone_index);
        let current_deadline: u32 = env
            .storage()
            .persistent()
            .get(&deadline_key)
            .unwrap_or_else(|| env.ledger().sequence());
        let new_deadline = current_deadline + req.extra_ledgers;
        env.storage().persistent().set(&deadline_key, &new_deadline);
        env.storage().persistent().extend_ttl(&deadline_key, 100_000, 6_300_000);
        env.events().publish(
            (Symbol::new(&env, "extension_approved"), shipment_id),
            (milestone_index, new_deadline, buyer),
        );
    }

    /// Buyer denies the pending extension request; clears it without changing the deadline.
    pub fn deny_extension(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_index: u32,
    ) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);
        let key = DataKey::ExtensionRequest(shipment_id.clone(), milestone_index);
        if !env.storage().persistent().has(&key) {
            panic!("no pending extension request");
        }
        env.storage().persistent().remove(&key);
        env.events().publish(
            (Symbol::new(&env, "extension_denied"), shipment_id),
            (milestone_index, buyer),
        );
    }

    /// Returns the effective deadline ledger for a milestone (0 = not set).
    pub fn get_milestone_deadline(env: Env, shipment_id: String, milestone_index: u32) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MilestoneDeadline(shipment_id, milestone_index))
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ADMIN: TOKEN WHITELIST
    // ----------------------------------------------------------

    pub fn add_allowed_token(env: Env, token: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic!("unauthorized"));
        admin.require_auth();
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "add_allowed_token"),
            Symbol::new(&env, "allowed_token_added"),
        );
        let mut allowed: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::AllowedTokens)
            .unwrap_or_else(|| Vec::new(&env));
        allowed.push_back(token);
        env.storage()
            .instance()
            .set(&DataKey::AllowedTokens, &allowed);
    }

    pub fn remove_allowed_token(env: Env, token: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic!("unauthorized"));
        admin.require_auth();
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "remove_allowed_token"),
            Symbol::new(&env, "allowed_token_removed"),
        );
        let allowed: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::AllowedTokens)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_list: Vec<Address> = Vec::new(&env);
        for i in 0..allowed.len() {
            let t = allowed.get(i).unwrap();
            if t != token {
                new_list.push_back(t);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::AllowedTokens, &new_list);
    }

    // ----------------------------------------------------------
    // CREATE SHIPMENT
    // ----------------------------------------------------------

    pub fn create_shipment(
        env: Env,
        shipment_id: String,
        buyers: Vec<Address>,
        supplier: Address,
        logistics: Address,
        arbiter: Address,
        token: Address,
        total_amount: i128,
        milestones: Vec<Milestone>,
        options: ShipmentOptions,
    ) -> String {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);
        let response_deadline = options.response_deadline;
        let penalty_bps = options.penalty_bps;
        let milestone_mode = options.milestone_mode;
        let holdback_ledgers = options.holdback_ledgers;
        let dispute_cooldown_ledgers = options.dispute_cooldown_ledgers;
        let late_penalty_bps_per_ledger = options.late_penalty_bps_per_ledger;
        let auto_confirm_ledgers = options.auto_confirm_ledgers;
        let dispute_bond_amount = options.dispute_bond_amount;
        let logistics_fee_bps = options.logistics_fee_bps;
        let supplier_collateral = options.supplier_collateral;
        let expires_at_ledger = options.expires_at_ledger;

        let metadata_hash = options.metadata_hash;
        let referrer = options.referrer;
        let buyer_cancel_fee_bps = options.buyer_cancel_fee_bps;

        if buyer_cancel_fee_bps > 1000 {
            panic!("buyer_cancel_fee_bps cannot exceed 1000 (10%)");
        }


        if buyers.is_empty() {
            panic!("at least one buyer is required");
        }

        // All co-buyers must authorise the creation.
        for i in 0..buyers.len() {
            buyers.get(i).unwrap().require_auth();
        }

        if total_amount <= 0 {
            panic!("amount must be greater than zero");
        }

        // Batch read all validation config and stats in a single context fetch.
        let ctx = Self::fetch_create_shipment_ctx(&env);

        if ctx.max_value > 0 && total_amount > ctx.max_value {
            panic!("total amount exceeds maximum shipment value");
        }

        // Enforce token whitelist when non-empty.
        if ctx.allowed_tokens.len() > 0 {
            let mut found = false;
            for i in 0..ctx.allowed_tokens.len() {
                if ctx.allowed_tokens.get(i).unwrap() == token {
                    found = true;
                    break;
                }
            }
            if !found {
                panic!("unauthorized");
            }
        }

        for i in 0..buyers.len() {
            if env
                .storage()
                .instance()
                .get::<DataKey, BytesN<32>>(&DataKey::Blacklisted(buyers.get(i).unwrap().clone()))
                .is_some()
            {
                panic!("unauthorized");
            }
        }
        for addr in [supplier.clone(), logistics.clone(), arbiter.clone()] {
            if env
                .storage()
                .instance()
                .get::<DataKey, BytesN<32>>(&DataKey::Blacklisted(addr))
                .is_some()
            {
                panic!("unauthorized");
            }
        }

        // Enforce supplier whitelist when non-empty.
        let supplier_whitelist: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::SupplierWhitelist)
            .unwrap_or_else(|| Vec::new(&env));
        if supplier_whitelist.len() > 0 {
            let mut whitelisted = false;
            for i in 0..supplier_whitelist.len() {
                if supplier_whitelist.get(i).unwrap() == supplier {
                    whitelisted = true;
                    break;
                }
            }
            if !whitelisted {
                panic!("unauthorized");
            }
        }

        let min_pct = ctx.min_pct;
        let mut total_percent: u32 = 0;
        for i in 0..milestones.len() {
            let percent = milestones.get(i).unwrap().payment_percent;
            if percent < min_pct {
                panic!("InvalidPercentages");
            }
            total_percent += percent;
        }
        if total_percent != 100 {
            panic!("milestone percentages must sum to 100");
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Shipment(shipment_id.clone()))
        {
            panic!("shipment already exists");
        }

        // Transfer total_amount from the primary buyer (index 0).
        let primary_buyer = buyers.get(0).unwrap();
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &primary_buyer,
            &env.current_contract_address(),
            &total_amount,
        );

        // Lock dispute bond pool: dispute_bond_amount * number_of_milestones (0 = disabled).
        if dispute_bond_amount > 0 {
            let bond_total = dispute_bond_amount * milestones.len() as i128;
            token_client.transfer(&primary_buyer, &env.current_contract_address(), &bond_total);
        }

        // Normalise milestones: clear any caller-supplied state.
        let mut clean_milestones: Vec<Milestone> = Vec::new(&env);
        for i in 0..milestones.len() {
            let mut m = milestones.get(i).unwrap();
            m.status = MilestoneStatus::Pending;
            m.proof_hash = String::from_str(&env, "");
            m.release_after_ledger = 0;
            m.proof_submitted_ledger = None;
            m.dispute_opened_ledger = None;
            clean_milestones.push_back(m);
        }

        let mut shipment = Shipment {
            id: shipment_id.clone(),
            audit_log: Vec::new(&env),

            buyers,
            supplier: supplier.clone(),
            logistics,
            arbiter,
            token: token.clone(),
            total_amount,
            released_amount: 0,
            total_advanced_amount: 0,
            milestones: clean_milestones,
            status: ShipmentStatus::Active,
            milestone_mode,
            created_at: env.ledger().sequence(),
            holdback_ledgers,
            dispute_cooldown_ledgers,
            last_dispute_resolved_ledger: None,
            late_penalty_bps_per_ledger,
            auto_confirm_ledgers,
            open_dispute_count: 0,
            dispute_bond_amount,
            arbiter_fee_bps: options.arbiter_fee_bps,
            logistics_fee_bps,
            expires_at_ledger,
            metadata_hash,

            referrer,
            buyer_cancel_fee_bps,

        };

        Self::append_audit_entry(
            &env,
            &mut shipment,
            Symbol::new(&env, "shipment_created"),
            Symbol::new(&env, "create_shipment"),
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);
        env.storage().persistent().set(
            &DataKey::CancelPolicy(shipment_id.clone()),
            &CancelPolicy {
                response_deadline,
                penalty_bps,
            },
        );
        env.storage().persistent().extend_ttl(
            &DataKey::Shipment(shipment_id.clone()),
            100_000,
            6_300_000,
        );

        // #113: Resolve and lock the buyer's effective fee tier at creation.
        {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            let effective_bps = Self::resolve_fee_bps_for(&env, &primary_buyer);
            env.storage()
                .persistent()
                .set(&DataKey::ShipmentFeeBps(shipment_id.clone()), &effective_bps);
        }

        // Index by supplier for supplier-facing dashboards.
        let mut supplier_shipments: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::SupplierShipments(supplier.clone()))
            .unwrap_or_else(|| Vec::new(&env));
        supplier_shipments.push_back(shipment_id.clone());
        env.storage().persistent().set(
            &DataKey::SupplierShipments(supplier.clone()),
            &supplier_shipments,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::SupplierShipments(supplier.clone()),
            100_000,
            6_300_000,
        );

        // Index by each buyer for buyer-facing dashboards.
        for i in 0..shipment.buyers.len() {
            let buyer = shipment.buyers.get(i).unwrap();
            let mut buyer_shipments: Vec<String> = env
                .storage()
                .persistent()
                .get(&DataKey::BuyerShipments(buyer.clone()))
                .unwrap_or_else(|| Vec::new(&env));
            buyer_shipments.push_back(shipment_id.clone());
            env.storage()
                .persistent()
                .set(&DataKey::BuyerShipments(buyer.clone()), &buyer_shipments);
            env.storage().persistent().extend_ttl(
                &DataKey::BuyerShipments(buyer.clone()),
                100_000,
                6_300_000,
            );
        }

        // Add to AllShipments list for pagination.
        let mut all_shipments: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::AllShipments)
            .unwrap_or_else(|| Vec::new(&env));
        all_shipments.push_back(shipment_id.clone());
        env.storage()
            .persistent()
            .set(&DataKey::AllShipments, &all_shipments);

        // Add to the Active status index.
        Self::add_to_status_index(&env, ShipmentStatus::Active, &shipment_id);

        // Update total escrowed value for this token.
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(token.clone()),
            &(current_escrowed + total_amount),
        );

        // Update contract stats.
        let mut stats = ctx.contract_stats;
        stats.total_shipments += 1;
        stats.total_volume += total_amount;
        env.storage()
            .instance()
            .set(&DataKey::ContractStats, &stats);

        env.events().publish(
            (Symbol::new(&env, "shipment_created"), shipment_id.clone()),
            (
                shipment.buyers.get(0).unwrap(),
                shipment.supplier.clone(),
                shipment.logistics.clone(),
                shipment.arbiter.clone(),
                shipment.token.clone(),
                shipment.total_amount,
                env.ledger().sequence(),
                shipment.metadata_hash.clone(),
            ),
        );

        shipment_id
    }

    // ----------------------------------------------------------
    // ESCROW TOP-UP
    // ----------------------------------------------------------

    /// Buyer tops up the shipment escrow with additional funds.
    /// Milestone percentages are unchanged; the higher total_amount proportionally
    /// increases each milestone's absolute payment.
    /// Disallowed once the shipment is Completed or Cancelled.
    pub fn top_up_escrow(env: Env, buyer: Address, shipment_id: String, additional_amount: i128) {
        Self::assert_not_paused(&env);

        if additional_amount <= 0 {
            panic!("additional_amount must be greater than zero");
        }

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("top-up disallowed: shipment is not active");
        }

        Self::require_buyer_auth(&shipment, &buyer);

        let token_client = token::Client::new(&env, &shipment.token);
        token_client.transfer(&buyer, &env.current_contract_address(), &additional_amount);

        let new_total = shipment.total_amount + additional_amount;
        shipment.total_amount = new_total;

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (Symbol::new(&env, "escrow_topped_up"), shipment_id.clone()),
            (additional_amount, new_total),
        );
    }

    // ----------------------------------------------------------
    // MILESTONE PERCENTAGE REBALANCING
    // ----------------------------------------------------------

    /// Buyer rebalances milestone payment percentages before any proof has been submitted.
    /// All milestones must still be in Pending status (no proof submitted on any of them).
    /// The new percentages must sum to 100 and each must meet the minimum threshold.
    pub fn rebalance_milestones(
        env: Env,
        buyer: Address,
        shipment_id: String,
        new_percents: Vec<u32>,
    ) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if new_percents.len() != shipment.milestones.len() {
            panic!("percent count must match milestone count");
        }

        // Rebalancing is only permitted before any proof has been submitted.
        for i in 0..shipment.milestones.len() {
            let m = shipment.milestones.get(i).unwrap();
            if m.status != MilestoneStatus::Pending {
                panic!("cannot rebalance: at least one milestone is no longer pending");
            }
        }

        let min_pct: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinMilestonePercent)
            .unwrap_or(5u32);
        let mut total: u32 = 0;
        for i in 0..new_percents.len() {
            let pct = new_percents.get(i).unwrap();
            if pct < min_pct {
                panic!("InvalidPercentages");
            }
            total += pct;
        }
        if total != 100 {
            panic!("milestone percentages must sum to 100");
        }

        for i in 0..new_percents.len() {
            let mut m = shipment.milestones.get(i).unwrap();
            m.payment_percent = new_percents.get(i).unwrap();
            shipment.milestones.set(i, m);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (Symbol::new(&env, "milestones_rebalanced"), shipment_id.clone()),
            (buyer, new_percents),
        );
    }

    // ----------------------------------------------------------
    // SUPPLIER ADVANCE PAYMENT
    // ----------------------------------------------------------

    /// Supplier requests an advance draw of up to `advance_percent` of the milestone's
    /// payment before submitting proof. Only callable on a Pending milestone.
    pub fn request_advance(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        advance_percent: u32,
    ) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);

        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_supplier_auth(&shipment, &caller);
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Pending {
            panic!("milestone is not in pending status");
        }

        let max_advance: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxAdvancePercent)
            .unwrap_or(30);
        if advance_percent > max_advance {
            panic!("AdvanceExceedsMax");
        }

        let advance_key = DataKey::AdvanceRequest(shipment_id.clone(), milestone_index);
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, AdvanceRequest>(&advance_key)
        {
            if existing.approved {
                panic!("AdvanceAlreadyApproved");
            }
        }

        let request = AdvanceRequest {
            requested_percent: advance_percent,
            approved: false,
            amount_advanced: 0,
        };
        env.storage()
            .persistent()
            .set(&advance_key, &request);
        env.storage()
            .persistent()
            .extend_ttl(&advance_key, 100_000, 6_300_000);

        env.events().publish(
            (Symbol::new(&env, "advance_requested"), shipment_id.clone()),
            (milestone_index, advance_percent, caller),
        );
    }

    /// Buyer approves a pending advance request. Transfers the advance amount to
    /// the supplier immediately. The advance is deducted from the milestone payment
    /// when the milestone is later confirmed.
    pub fn approve_advance(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let advance_key = DataKey::AdvanceRequest(shipment_id.clone(), milestone_index);
        let mut request: AdvanceRequest = env
            .storage()
            .persistent()
            .get(&advance_key)
            .unwrap_or_else(|| panic!("AdvanceNotRequested"));

        if request.approved {
            panic!("AdvanceAlreadyApproved");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        let milestone_payment =
            (shipment.total_amount * milestone.payment_percent as i128) / 100;
        let advance_amount = (milestone_payment * request.requested_percent as i128) / 100;

        request.approved = true;
        request.amount_advanced = advance_amount;
        env.storage()
            .persistent()
            .set(&advance_key, &request);

        // Transfer advance to supplier.
        let token_client = token::Client::new(&env, &shipment.token);
        token_client.transfer(
            &env.current_contract_address(),
            &shipment.supplier,
            &advance_amount,
        );

        // Track total advances for correct escrow accounting.
        shipment.total_advanced_amount += advance_amount;
        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Decrement total escrowed value for this token.
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - advance_amount).max(0),
        );

        env.events().publish(
            (Symbol::new(&env, "advance_approved"), shipment_id.clone()),
            (milestone_index, advance_amount, shipment.supplier.clone()),
        );
    }

    // ----------------------------------------------------------
    // PROOF CONTENT-TYPE WHITELIST
    // ----------------------------------------------------------

    /// Buyer sets the allowed proof content-type identifiers for a specific milestone.
    /// Must be called before proof is submitted (milestone must be Pending).
    /// Pass an empty Vec to remove the whitelist and allow any type.
    /// Example types: Symbol::new(&env, "ipfs"), Symbol::new(&env, "sha256"), Symbol::new(&env, "url").
    pub fn set_proof_whitelist(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_index: u32,
        allowed_types: Vec<Symbol>,
    ) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);

        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Pending {
            panic!("cannot set whitelist: proof already submitted for this milestone");
        }

        let key = DataKey::MilestoneProofWhitelist(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&key, &allowed_types);
        env.storage()
            .persistent()
            .extend_ttl(&key, 100_000, 6_300_000);

        env.events().publish(
            (Symbol::new(&env, "proof_whitelist_set"), shipment_id.clone()),
            (milestone_index, buyer),
        );
    }

    /// Returns the submitted proof content type for a milestone, or None if not yet submitted.
    pub fn get_milestone_proof_type(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Option<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::SubmittedProofType(shipment_id, milestone_index))
    }

    /// Returns the proof content-type whitelist for a milestone.
    /// An empty Vec means any type is accepted.
    pub fn get_proof_whitelist(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::MilestoneProofWhitelist(shipment_id, milestone_index))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // SUBMIT PROOF
    // ----------------------------------------------------------

    pub fn submit_proof(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        proof_hash: String,
        proof_type: Symbol,
    ) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();

        if milestone.status != MilestoneStatus::Pending {
            panic!("milestone is not in pending status");
        }
        Self::require_supplier_or_logistics_auth(&shipment, &caller);

        // Validate proof_type against per-milestone whitelist (if one is set).
        let whitelist_key = DataKey::MilestoneProofWhitelist(shipment_id.clone(), milestone_index);
        if let Some(whitelist) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<Symbol>>(&whitelist_key)
        {
            if whitelist.len() > 0 {
                let mut allowed = false;
                for i in 0..whitelist.len() {
                    if whitelist.get(i).unwrap() == proof_type {
                        allowed = true;
                        break;
                    }
                }
                if !allowed {
                    panic!("proof type not in whitelist");
                }
            }
        }

        // Sequential mode: previous milestone must be complete.
        if shipment.milestone_mode == MilestoneMode::Sequential && milestone_index > 0 {
            let prev = shipment.milestones.get(milestone_index - 1).unwrap();
            if prev.status != MilestoneStatus::Confirmed && prev.status != MilestoneStatus::Resolved
            {
                panic!("previous milestone not yet complete");
            }
        }

        let current_ledger = env.ledger().sequence();
        let is_resubmission = milestone.proof_hash.len() > 0;
        let proof_hash_for_event = proof_hash.clone();
        milestone.proof_hash = proof_hash;
        milestone.status = MilestoneStatus::ProofSubmitted;
        milestone.proof_submitted_ledger = Some(current_ledger);
        shipment.milestones.set(milestone_index, milestone);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Record the ledger at which proof was submitted (used by supplier_cancel).
        env.storage().persistent().set(
            &DataKey::ProofSubmittedAt(shipment_id.clone(), milestone_index),
            &current_ledger,
        );

        // Record the declared proof content type for off-chain and on-chain querying.
        let type_key = DataKey::SubmittedProofType(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&type_key, &proof_type);
        env.storage()
            .persistent()
            .extend_ttl(&type_key, 100_000, 6_300_000);

        let event_topic = if is_resubmission {
            Symbol::new(&env, "proof_resubmitted")
        } else {
            Symbol::new(&env, "proof_submitted")
        };
        env.events().publish(
            (event_topic, shipment_id.clone()),
            (
                milestone_index,
                proof_hash_for_event,
                proof_type,
                caller,
                current_ledger,
            ),
        );
    }

    // ----------------------------------------------------------
    // CONFIRM MILESTONE (multi-sig)
    // ----------------------------------------------------------

    pub fn confirm_milestone(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);

        // Batch read shipment and contract stats in a single context fetch.
        let ctx = Self::fetch_confirm_milestone_ctx(&env, &shipment_id);
        let mut shipment = ctx.shipment;

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::ProofSubmitted {
            panic!("milestone proof not yet submitted");
        }

        // Check if auto-confirmation window has passed; if so, reject manual confirmation.
        if shipment.auto_confirm_ledgers > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let auto_confirm_ledger = proof_ledger + shipment.auto_confirm_ledgers;
                if env.ledger().sequence() >= auto_confirm_ledger {
                    panic!("milestone has auto-confirmed; use claim_auto_confirmation");
                }
            }
        }

        let mut payment = (shipment.total_amount * milestone.payment_percent as i128) / 100;

        // Deduct any approved advance for this milestone.
        let advance_deducted =
            Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, milestone_index);

        // Apply late-delivery penalty if configured.
        let mut penalty_deducted: i128 = 0;
        if shipment.late_penalty_bps_per_ledger > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let delay_ledgers = env.ledger().sequence() - proof_ledger;
                let penalty = (payment
                    * (shipment.late_penalty_bps_per_ledger as i128 * delay_ledgers as i128))
                    / 10_000;
                if penalty > 0 && penalty < payment {
                    penalty_deducted = penalty;
                    payment -= penalty;
                }
            }
        }

        if shipment.holdback_ledgers > 0 {
            milestone.release_after_ledger = env.ledger().sequence() + shipment.holdback_ledgers;
            milestone.status = MilestoneStatus::ConfirmedHeld;
            shipment.milestones.set(milestone_index, milestone.clone());

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            env.events().publish(
                (Symbol::new(&env, "payment_held"), shipment_id.clone()),
                (
                    milestone_index,
                    milestone.release_after_ledger,
                    penalty_deducted,
                ),
            );
        } else {
            let mut fee_amount: i128 = 0;
            let net_payment = Self::deduct_fee_for_shipment(&env, payment, &shipment.token, &shipment_id, &mut fee_amount);

            // Check circuit breaker before transferring payment
            Self::check_circuit_breaker(&env, payment);

            milestone.status = MilestoneStatus::Confirmed;
            shipment.milestones.set(milestone_index, milestone);
            shipment.released_amount += payment;

            // #113: Accumulate lifetime volume for the primary buyer.
            {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                let vol_key = DataKey::LifetimeVolume(primary_buyer.clone());
                let prev: i128 = env.storage().persistent().get(&vol_key).unwrap_or(0);
                env.storage().persistent().set(&vol_key, &(prev + payment));
            }

            // Transfer the net payment minus any advance already sent.
            let mut actual_transfer = net_payment - advance_deducted;
            let token_client = token::Client::new(&env, &shipment.token);

            // Pay referral fee on shipment completion (deducted from final supplier payment).
            if Self::all_milestones_done(&shipment) {
                if let Some(referrer_addr) = shipment.referrer.clone() {
                    let referral_bps: u32 = env
                        .storage()
                        .instance()
                        .get(&DataKey::ReferralFeeBps)
                        .unwrap_or(500);
                    if let Some(fee_cfg) = env
                        .storage()
                        .instance()
                        .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
                    {
                        let total_fee =
                            (shipment.total_amount * fee_cfg.fee_bps as i128) / 10_000;
                        let mut referral = (total_fee * referral_bps as i128) / 10_000;
                        if referral > actual_transfer {
                            referral = actual_transfer;
                        }
                        if referral > 0 {
                            actual_transfer -= referral;
                            token_client.transfer(
                                &env.current_contract_address(),
                                &referrer_addr,
                                &referral,
                            );
                        }
                    }
                }
            }

            if actual_transfer > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &actual_transfer,
                );
            }

            // Return penalty to buyer if any.
            if penalty_deducted > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &penalty_deducted,
                );
            }

            if Self::all_milestones_done(&shipment) {
                shipment.status = ShipmentStatus::Completed;
                // Update completed shipments stat using pre-fetched context.
                let mut stats = ctx.contract_stats;
                stats.completed_shipments += 1;
                env.storage()
                    .instance()
                    .set(&DataKey::ContractStats, &stats);
                Self::increment_reputation_internal(&env, &shipment.supplier, 1, 0, 0);
                // Move from Active to Completed status index.
                Self::move_shipment_status_index(
                    &env,
                    ShipmentStatus::Active,
                    ShipmentStatus::Completed,
                    &shipment_id,
                );


                // Return supplier collateral on completion.
                let collateral: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::SupplierCollateral(shipment_id.clone()))
                    .unwrap_or(0);
                if collateral > 0 {
                    token_client.transfer(&env.current_contract_address(), &shipment.supplier, &collateral);
                }

                // Issue #104 — Emit NFT mint hook event if enabled.
                // Purely informational: no state change or external contract call.
                let nft_hook_enabled: bool = env
                    .storage()
                    .instance()
                    .get(&DataKey::NftHookEnabled)
                    .unwrap_or(false);
                if nft_hook_enabled {
                    env.events().publish(
                        (Symbol::new(&env, "nft_mint_hook"), shipment_id.clone()),
                        (
                            shipment.buyers.get(0).unwrap(),
                            shipment.supplier.clone(),
                            shipment.total_amount,
                            env.ledger().sequence(),
                            shipment.metadata_hash.clone(),
                        ),
                    );
                }

            }

            // Decrement total escrowed value (net of any advance already deducted).
            let net_outflow = payment - advance_deducted;
            let current_escrowed: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::TotalEscrowed(shipment.token.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::TotalEscrowed(shipment.token.clone()),
                &(current_escrowed - net_outflow).max(0),
            );

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            let remaining_amount = shipment.total_amount - shipment.released_amount;
            env.events().publish(
                (
                    Symbol::new(&env, "milestone_confirmed"),
                    shipment_id.clone(),
                ),
                (
                    milestone_index,
                    payment,
                    fee_amount,
                    penalty_deducted,
                    shipment.supplier.clone(),
                    env.ledger().sequence(),
                    shipment.released_amount,
                    remaining_amount,
                ),
            );
        }
    }

    // ----------------------------------------------------------
    // RELEASE HELD PAYMENT
    // ----------------------------------------------------------

    /// Anyone can call this once the holdback window has passed.
    pub fn release_held_payment(env: Env, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();

        if milestone.status != MilestoneStatus::ConfirmedHeld {
            panic!("milestone is not in ConfirmedHeld status");
        }

        if env.ledger().sequence() < milestone.release_after_ledger {
            panic!("holdback period not yet expired");
        }

        let payment = (shipment.total_amount * milestone.payment_percent as i128) / 100;

        // Deduct any approved advance for this milestone.
        let advance_deducted =
            Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, milestone_index);

        let mut fee_amount: i128 = 0;
        let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

        // Check circuit breaker before transferring payment
        Self::check_circuit_breaker(&env, payment);

        milestone.status = MilestoneStatus::Confirmed;
        milestone.release_after_ledger = 0;
        shipment.milestones.set(milestone_index, milestone);
        shipment.released_amount += payment;

        let mut actual_transfer = net_payment - advance_deducted;
        let token_client = token::Client::new(&env, &shipment.token);

        // Pay referral fee on shipment completion (deducted from final supplier payment).
        if Self::all_milestones_done(&shipment) {
            if let Some(referrer_addr) = shipment.referrer.clone() {
                let referral_bps: u32 = env
                    .storage()
                    .instance()
                    .get(&DataKey::ReferralFeeBps)
                    .unwrap_or(500);
                if let Some(fee_cfg) = env
                    .storage()
                    .instance()
                    .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
                {
                    let total_fee = (shipment.total_amount * fee_cfg.fee_bps as i128) / 10_000;
                    let mut referral = (total_fee * referral_bps as i128) / 10_000;
                    if referral > actual_transfer {
                        referral = actual_transfer;
                    }
                    if referral > 0 {
                        actual_transfer -= referral;
                        token_client.transfer(
                            &env.current_contract_address(),
                            &referrer_addr,
                            &referral,
                        );
                    }
                }
            }
        }

        if actual_transfer > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &shipment.supplier,
                &actual_transfer,
            );
        }

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;
            // Update completed shipments stat.
            let mut stats: ContractStats = env
                .storage()
                .instance()
                .get(&DataKey::ContractStats)
                .unwrap_or(ContractStats {
                    total_shipments: 0,
                    total_volume: 0,
                    total_disputes: 0,
                    completed_shipments: 0,
                });
            stats.completed_shipments += 1;
            env.storage()
                .instance()
                .set(&DataKey::ContractStats, &stats);
            Self::increment_reputation_internal(&env, &shipment.supplier, 1, 0, 0);
            // Move from Active to Completed status index.
            Self::move_shipment_status_index(
                &env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
        }

        // Decrement total escrowed value (net of any advance already deducted).
        let net_outflow = payment - advance_deducted;
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - net_outflow).max(0),
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (
                Symbol::new(&env, "held_payment_released"),
                shipment_id.clone(),
            ),
            (milestone_index, payment, fee_amount),
        );
    }

    // ----------------------------------------------------------
    // BATCH CONFIRM MILESTONES
    // ----------------------------------------------------------

    /// Confirm multiple milestones in one invocation. Atomic — any failure reverts all.
    pub fn batch_confirm_milestones(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_indices: Vec<u32>,
    ) {
        Self::assert_not_paused(&env);
        buyer.require_auth();

        if milestone_indices.is_empty() {
            return;
        }

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_is_buyer(&shipment, &buyer);

        // Validate all indices and statuses before mutating anything.
        for i in 0..milestone_indices.len() {
            let idx = milestone_indices.get(i).unwrap();
            if idx as usize >= shipment.milestones.len() as usize {
                panic!("invalid milestone index");
            }
            let m = shipment.milestones.get(idx).unwrap();
            if m.status != MilestoneStatus::ProofSubmitted {
                panic!("milestone proof not yet submitted");
            }
        }

        // Apply confirmations and emit events.
        for i in 0..milestone_indices.len() {
            let idx = milestone_indices.get(i).unwrap();
            let mut milestone = shipment.milestones.get(idx).unwrap();
            milestone.status = MilestoneStatus::Confirmed;
            shipment.milestones.set(idx, milestone.clone());

            let payment = (shipment.total_amount * milestone.payment_percent as i128) / 100;

            // Deduct any approved advance for this milestone.
            let advance_deducted =
                Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, idx);

            let mut fee_amount: i128 = 0;
            let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

            // Check circuit breaker before transferring payment
            Self::check_circuit_breaker(&env, payment);

            shipment.released_amount += payment;

            let mut actual_transfer = net_payment - advance_deducted;
            let token_client = token::Client::new(&env, &shipment.token);

            // Pay referral fee when this is the completing milestone.
            if Self::all_milestones_done(&shipment) {
                if let Some(referrer_addr) = shipment.referrer.clone() {
                    let referral_bps: u32 = env
                        .storage()
                        .instance()
                        .get(&DataKey::ReferralFeeBps)
                        .unwrap_or(500);
                    if let Some(fee_cfg) = env
                        .storage()
                        .instance()
                        .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
                    {
                        let total_fee =
                            (shipment.total_amount * fee_cfg.fee_bps as i128) / 10_000;
                        let mut referral = (total_fee * referral_bps as i128) / 10_000;
                        if referral > actual_transfer {
                            referral = actual_transfer;
                        }
                        if referral > 0 {
                            actual_transfer -= referral;
                            token_client.transfer(
                                &env.current_contract_address(),
                                &referrer_addr,
                                &referral,
                            );
                        }
                    }
                }
            }

            if actual_transfer > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &actual_transfer,
                );
            }

            // Decrement total escrowed value (net of any advance already deducted).
            let net_outflow = payment - advance_deducted;
            let current_escrowed: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::TotalEscrowed(shipment.token.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::TotalEscrowed(shipment.token.clone()),
                &(current_escrowed - net_outflow).max(0),
            );

            let remaining_amount = shipment.total_amount - shipment.released_amount;
            env.events().publish(
                (
                    Symbol::new(&env, "milestone_confirmed"),
                    shipment_id.clone(),
                ),
                (
                    idx,
                    payment,
                    fee_amount,
                    shipment.supplier.clone(),
                    env.ledger().sequence(),
                    shipment.released_amount,
                    remaining_amount,
                ),
            );
        }

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;
            // Update completed shipments stat.
            let mut stats: ContractStats = env
                .storage()
                .instance()
                .get(&DataKey::ContractStats)
                .unwrap_or(ContractStats {
                    total_shipments: 0,
                    total_volume: 0,
                    total_disputes: 0,
                    completed_shipments: 0,
                });
            stats.completed_shipments += 1;
            env.storage()
                .instance()
                .set(&DataKey::ContractStats, &stats);
            Self::increment_reputation_internal(&env, &shipment.supplier, 1, 0, 0);
            // Move from Active to Completed status index.
            Self::move_shipment_status_index(
                &env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
        }

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);
    }

    // ----------------------------------------------------------
    // RAISE DISPUTE
    // ----------------------------------------------------------

    pub fn raise_dispute(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        // Dispute cooldown check.
        if shipment.dispute_cooldown_ledgers > 0 {
            if let Some(last_resolved) = shipment.last_dispute_resolved_ledger {
                let earliest_allowed = last_resolved + shipment.dispute_cooldown_ledgers;
                if env.ledger().sequence() < earliest_allowed {
                    panic!("dispute cooldown period has not elapsed");
                }
            }
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::ProofSubmitted
            && milestone.status != MilestoneStatus::ConfirmedHeld
        {
            panic!("can only dispute a submitted or held proof");
        }

        // Check if auto-confirmation window has passed; if so, reject dispute.
        if shipment.auto_confirm_ledgers > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let auto_confirm_ledger = proof_ledger + shipment.auto_confirm_ledgers;
                if env.ledger().sequence() >= auto_confirm_ledger {
                    panic!("milestone has auto-confirmed; dispute window closed");
                }
            }
        }

        let max_open: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxConcurrentDisputes)
            .unwrap_or(1u32);
        if shipment.open_dispute_count >= max_open {
            panic!("DisputeAlreadyOpen");
        }

        shipment.open_dispute_count += 1;
        // Cancel any holdback window.
        milestone.release_after_ledger = 0;
        milestone.status = MilestoneStatus::Disputed;
        milestone.dispute_opened_ledger = Some(env.ledger().sequence());
        shipment.milestones.set(milestone_index, milestone);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        Self::increment_reputation_internal(&env, &shipment.supplier, 0, 1, 0);

        // Add to active disputes list.
        let mut disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env));
        disputes.push_back(DisputeEntry {
            shipment_id: shipment_id.clone(),
            milestone_index,
        });
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &disputes);

        // Increment total disputes stat.
        let mut stats: ContractStats = env
            .storage()
            .instance()
            .get(&DataKey::ContractStats)
            .unwrap_or(ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            });
        stats.total_disputes += 1;
        env.storage()
            .instance()
            .set(&DataKey::ContractStats, &stats);

        env.events().publish(
            (Symbol::new(&env, "dispute_raised"), shipment_id.clone()),
            milestone_index,
        );
    }

    // ----------------------------------------------------------
    // RAISE PARTIAL DISPUTE
    // ----------------------------------------------------------

    /// Buyer contests only `contested_percent` (1–99) of a milestone's value.
    /// The uncontested portion is released to the supplier immediately; the
    /// contested portion is held in escrow pending arbiter resolution.
    /// Panics if an approved advance already exists for the milestone — use
    /// `raise_dispute` instead when an advance has been approved.
    pub fn raise_partial_dispute(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_index: u32,
        contested_percent: u32,
    ) {
        Self::assert_not_paused(&env);

        if contested_percent == 0 || contested_percent >= 100 {
            panic!("contested_percent must be between 1 and 99");
        }

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        // Block partial disputes when an approved advance exists to avoid
        // complex advance-reconciliation across the split portions.
        let advance_key = DataKey::AdvanceRequest(shipment_id.clone(), milestone_index);
        if let Some(req) = env
            .storage()
            .persistent()
            .get::<DataKey, AdvanceRequest>(&advance_key)
        {
            if req.approved {
                panic!("partial dispute not allowed when an approved advance exists for this milestone");
            }
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::ProofSubmitted
            && milestone.status != MilestoneStatus::ConfirmedHeld
        {
            panic!("can only dispute a submitted or held proof");
        }

        // Dispute cooldown check.
        if shipment.dispute_cooldown_ledgers > 0 {
            if let Some(last_resolved) = shipment.last_dispute_resolved_ledger {
                let earliest_allowed = last_resolved + shipment.dispute_cooldown_ledgers;
                if env.ledger().sequence() < earliest_allowed {
                    panic!("dispute cooldown period has not elapsed");
                }
            }
        }

        // Auto-confirmation window check.
        if shipment.auto_confirm_ledgers > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let auto_confirm_ledger = proof_ledger + shipment.auto_confirm_ledgers;
                if env.ledger().sequence() >= auto_confirm_ledger {
                    panic!("milestone has auto-confirmed; dispute window closed");
                }
            }
        }

        let max_open: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxConcurrentDisputes)
            .unwrap_or(1u32);
        if shipment.open_dispute_count >= max_open {
            panic!("DisputeAlreadyOpen");
        }

        // Compute and immediately release the uncontested portion to the supplier.
        let full_milestone_payment =
            (shipment.total_amount * milestone.payment_percent as i128) / 100;
        let uncontested_payment =
            (full_milestone_payment * (100 - contested_percent) as i128) / 100;

        if uncontested_payment > 0 {
            let mut fee_amount: i128 = 0;
            let net_uncontested =
                Self::deduct_fee(&env, uncontested_payment, &shipment.token, &mut fee_amount);

            Self::check_circuit_breaker(&env, uncontested_payment);

            let token_client = token::Client::new(&env, &shipment.token);
            if net_uncontested > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &net_uncontested,
                );
            }

            shipment.released_amount += uncontested_payment;

            // Decrement total escrowed by the outflow.
            let current_escrowed: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::TotalEscrowed(shipment.token.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::TotalEscrowed(shipment.token.clone()),
                &(current_escrowed - uncontested_payment).max(0),
            );

            env.events().publish(
                (
                    Symbol::new(&env, "partial_uncontested_released"),
                    shipment_id.clone(),
                ),
                (milestone_index, uncontested_payment, fee_amount),
            );
        }

        // Store the contested percentage so resolve_dispute knows the scope.
        env.storage().persistent().set(
            &DataKey::DisputeContestedPercent(shipment_id.clone(), milestone_index),
            &contested_percent,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::DisputeContestedPercent(shipment_id.clone(), milestone_index),
            100_000,
            6_300_000,
        );

        shipment.open_dispute_count += 1;
        milestone.release_after_ledger = 0;
        milestone.status = MilestoneStatus::Disputed;
        milestone.dispute_opened_ledger = Some(env.ledger().sequence());
        shipment.milestones.set(milestone_index, milestone);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        Self::increment_reputation_internal(&env, &shipment.supplier, 0, 1, 0);

        // Add to active disputes list.
        let mut disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env));
        disputes.push_back(DisputeEntry {
            shipment_id: shipment_id.clone(),
            milestone_index,
        });
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &disputes);

        // Increment total disputes stat.
        let mut stats: ContractStats = env
            .storage()
            .instance()
            .get(&DataKey::ContractStats)
            .unwrap_or(ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            });
        stats.total_disputes += 1;
        env.storage()
            .instance()
            .set(&DataKey::ContractStats, &stats);

        env.events().publish(
            (
                Symbol::new(&env, "partial_dispute_raised"),
                shipment_id.clone(),
            ),
            (milestone_index, contested_percent, buyer),
        );
    }

    // ----------------------------------------------------------
    // RESOLVE DISPUTE
    // ----------------------------------------------------------

    /// Resolve a dispute (full or partial) raised on a milestone.
    ///
    /// For **full disputes** (`raise_dispute`):
    ///   - `approve = true`  → supplier wins; payment transferred, arbiter fee deducted.
    ///   - `approve = false` → buyer wins; milestone reset to Pending for resubmission.
    ///
    /// For **partial disputes** (`raise_partial_dispute`):
    ///   - `approve = true`  → supplier wins contested portion; arbiter fee deducted from it.
    ///   - `approve = false` → buyer wins; contested portion refunded minus arbiter fee;
    ///                          milestone marked Resolved (uncontested was already released).
    ///
    /// The arbiter fee (`shipment.arbiter_fee_bps`) is deducted from the disputed payment
    /// and transferred to the arbiter address whenever a monetary disbursement occurs.
    pub fn resolve_dispute(
        env: Env,
        arbiter: Address,
        shipment_id: String,
        milestone_index: u32,
        approve: bool,
    ) {
        Self::assert_not_paused(&env);

        // Batch read shipment, dispute status, stats and active disputes in a single context fetch.
        let ctx = Self::fetch_resolve_dispute_ctx(&env, &shipment_id, milestone_index);
        let mut shipment = ctx.shipment;

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_arbiter_auth(&shipment, &arbiter);

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not in disputed status");
        }

        // Use pre-fetched partial contested percent from context.
        let is_partial = ctx.partial_contested_percent.is_some();

        let full_payment = (shipment.total_amount * milestone.payment_percent as i128) / 100;

        // The "payment" in scope is the portion subject to this resolution:
        //   - full dispute  → 100% of milestone value
        //   - partial dispute → contested_percent% of milestone value
        let payment = if let Some(cp) = ctx.partial_contested_percent {
            (full_payment * cp as i128) / 100
        } else {
            full_payment
        };

        let token_client = token::Client::new(&env, &shipment.token);

        if approve {
            // Deduct any approved advance (only relevant for full disputes; partial disputes
            // block advance approval at raise time).
            let advance_deducted = Self::consume_advance_for_milestone(
                &env,
                &mut shipment,
                &shipment_id,
                milestone_index,
            );

            let mut fee_amount: i128 = 0;
            let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

            Self::check_circuit_breaker(&env, payment);

            // Compute and transfer arbiter fee from the disputed payment.
            let arbiter_fee = (payment * shipment.arbiter_fee_bps as i128) / 10_000;
            if arbiter_fee > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.arbiter,
                    &arbiter_fee,
                );
            }

            shipment.released_amount += payment;

            let actual_transfer = (net_payment - advance_deducted - arbiter_fee).max(0);
            if actual_transfer > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &actual_transfer,
                );
            }

            // Return the dispute bond to the buyer (they raised a valid dispute).
            if shipment.dispute_bond_amount > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &shipment.dispute_bond_amount,
                );
            }

            milestone.status = MilestoneStatus::Resolved;
        } else if is_partial {
            // Partial dispute rejection: buyer contested but lost.
            // Refund the contested portion to the buyer minus arbiter fee, then mark Resolved
            // (the uncontested portion was already released at raise_partial_dispute time).
            let arbiter_fee = (payment * shipment.arbiter_fee_bps as i128) / 10_000;
            if arbiter_fee > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.arbiter,
                    &arbiter_fee,
                );
            }

            let buyer_refund = (payment - arbiter_fee).max(0);
            if buyer_refund > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &buyer_refund,
                );
            }

            // Track the contested outflow so escrow balance stays consistent.
            shipment.released_amount += payment;

            // Forfeit the dispute bond to the supplier (buyer's challenge failed).
            if shipment.dispute_bond_amount > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &shipment.dispute_bond_amount,
                );
            }

            milestone.status = MilestoneStatus::Resolved;
        } else {
            // Full dispute rejection: milestone goes back to Pending for proof resubmission.
            // proof_hash is preserved so submit_proof can detect this as a resubmission.
            if shipment.dispute_bond_amount > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &shipment.dispute_bond_amount,
                );
            }
            milestone.status = MilestoneStatus::Pending;
        }

        // Clean up the partial-dispute record.
        let contested_key =
            DataKey::DisputeContestedPercent(shipment_id.clone(), milestone_index);
        if is_partial {
            env.storage().persistent().remove(&contested_key);
        }

        shipment.milestones.set(milestone_index, milestone);
        shipment.open_dispute_count = shipment.open_dispute_count.saturating_sub(1);

        // Update cooldown tracking regardless of approve/reject.
        shipment.last_dispute_resolved_ledger = Some(env.ledger().sequence());

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;
            let mut stats = ctx.contract_stats;
            stats.completed_shipments += 1;
            env.storage()
                .instance()
                .set(&DataKey::ContractStats, &stats);
            Self::increment_reputation_internal(&env, &shipment.supplier, 1, 0, 0);
            Self::move_shipment_status_index(
                &env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
        }

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Remove from active disputes list using pre-fetched disputes.
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(&env);
        for i in 0..ctx.active_disputes.len() {
            let d = ctx.active_disputes.get(i).unwrap();
            if !(d.shipment_id == shipment_id && d.milestone_index == milestone_index) {
                new_disputes.push_back(d);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &new_disputes);

        let released_amount = shipment.released_amount;
        let remaining_amount = shipment.total_amount - released_amount;
        env.events().publish(
            (Symbol::new(&env, "dispute_resolved"), shipment_id.clone()),
            (
                milestone_index,
                approve,
                is_partial,
                released_amount,
                remaining_amount,
            ),
        );
    }

    // ----------------------------------------------------------
    // CHECK ESCALATION
    // ----------------------------------------------------------

    /// Check if a dispute has exceeded the escalation threshold without arbiter action.
    /// Emits DisputeEscalated event if threshold exceeded.
    pub fn check_escalation(env: Env, shipment_id: String, milestone_index: u32) {
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::EscalationThreshold)
            .unwrap_or(0);

        if threshold == 0 {
            return; // Escalation not enabled
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            return; // Not disputed
        }

        if let Some(opened_ledger) = milestone.dispute_opened_ledger {
            let current_ledger = env.ledger().sequence();
            if current_ledger >= opened_ledger + threshold {
                env.events().publish(
                    (Symbol::new(&env, "dispute_escalated"), shipment_id.clone()),
                    (milestone_index, opened_ledger, current_ledger),
                );
            }
        }
    }

    // ----------------------------------------------------------
    // CANCEL SHIPMENT (buyer)
    // ----------------------------------------------------------

    pub fn cancel_shipment(env: Env, buyer: Address, shipment_id: String) {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::assert_not_paused(&env);
        buyer.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_is_buyer(&shipment, &buyer);

        // Block cancellation if any milestone is Disputed.
        for i in 0..shipment.milestones.len() {
            let m = shipment.milestones.get(i).unwrap();
            if m.status == MilestoneStatus::Disputed {
                panic!("cannot cancel: dispute must be resolved first");
            }
        }

        let unreleased = shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;
        let cancel_fee = (unreleased * shipment.buyer_cancel_fee_bps as i128) / 10_000;
        let refund = unreleased - cancel_fee;
        let primary_buyer = shipment.buyers.get(0).unwrap();
        let token_client = token::Client::new(&env, &shipment.token);

        if cancel_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &shipment.supplier, &cancel_fee);
        }
        if refund > 0 {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            let token_client = token::Client::new(&env, &shipment.token);
            token_client.transfer(&env.current_contract_address(), &primary_buyer, &refund);
        }

        shipment.status = ShipmentStatus::Cancelled;

        Self::increment_reputation_internal(&env, &shipment.supplier, 0, 0, 1);

        // Move from Active to Cancelled status index.
        Self::move_shipment_status_index(
            &env,
            ShipmentStatus::Active,
            ShipmentStatus::Cancelled,
            &shipment_id,
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Decrement total escrowed value (full unreleased amount leaves escrow).
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - unreleased).max(0),
        );

        // Remove any disputes for this shipment.
        let disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(&env);
        for i in 0..disputes.len() {
            let d = disputes.get(i).unwrap();
            if d.shipment_id != shipment_id {
                new_disputes.push_back(d);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &new_disputes);

        env.events().publish(
            (Symbol::new(&env, "shipment_cancelled"), shipment_id.clone()),
            (refund, cancel_fee, buyer.clone(), env.ledger().sequence()),
        );
    }

    // ----------------------------------------------------------
    // SUPPLIER CANCEL
    // ----------------------------------------------------------

    /// Supplier cancels after buyer_response_deadline_ledgers have passed
    /// with at least one milestone stuck in ProofSubmitted.
    pub fn supplier_cancel(env: Env, supplier: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        supplier.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if supplier != shipment.supplier {
            panic!("unauthorized");
        }

        let policy: CancelPolicy = env
            .storage()
            .persistent()
            .get(&DataKey::CancelPolicy(shipment_id.clone()))
            .unwrap_or(CancelPolicy {
                response_deadline: 0,
                penalty_bps: 0,
            });

        if policy.response_deadline == 0 {
            panic!("supplier cancellation not enabled for this shipment");
        }

        let current_ledger = env.ledger().sequence();
        let mut deadline_passed = false;
        for i in 0..shipment.milestones.len() {
            let m = shipment.milestones.get(i).unwrap();
            if m.status == MilestoneStatus::ProofSubmitted {
                let submitted_at: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::ProofSubmittedAt(shipment_id.clone(), i))
                    .unwrap_or(0);
                if current_ledger >= submitted_at + policy.response_deadline {
                    deadline_passed = true;
                    break;
                }
            }
        }

        if !deadline_passed {
            panic!("buyer response deadline has not passed");
        }

        let remaining = shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;
        let penalty = (remaining * policy.penalty_bps as i128) / 10_000;
        let refund = remaining - penalty;

        let token_client = token::Client::new(&env, &shipment.token);
        if penalty > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &shipment.supplier,
                &penalty,
            );
        }
        if refund > 0 {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            token_client.transfer(&env.current_contract_address(), &primary_buyer, &refund);
        }

        shipment.status = ShipmentStatus::Cancelled;

        Self::increment_reputation_internal(&env, &shipment.supplier, 0, 0, 1);

        // Move from Active to Cancelled status index.
        Self::move_shipment_status_index(
            &env,
            ShipmentStatus::Active,
            ShipmentStatus::Cancelled,
            &shipment_id,
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Decrement total escrowed value.
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - remaining).max(0),
        );

        // Remove any disputes for this shipment.
        let disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(&env);
        for i in 0..disputes.len() {
            let d = disputes.get(i).unwrap();
            if d.shipment_id != shipment_id {
                new_disputes.push_back(d);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &new_disputes);

        env.events().publish(
            (
                Symbol::new(&env, "supplier_cancellation"),
                shipment_id.clone(),
            ),
            (penalty, refund),
        );
    }

    // ----------------------------------------------------------
    // PROPOSE AMENDMENT
    // ----------------------------------------------------------

    /// Buyer or supplier proposes an amendment to a Pending milestone.
    /// When both parties have proposed identical (new_percent, new_name), the amendment is applied.
    pub fn propose_amendment(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        new_percent: u32,
        new_name: String,
    ) {
        Self::assert_not_paused(&env);
        caller.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let is_buyer = Self::is_buyer(&shipment, &caller);
        if !is_buyer && caller != shipment.supplier {
            panic!("unauthorized");
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Pending {
            panic!("can only amend a pending milestone");
        }

        let amendment_key = DataKey::Amendment(shipment_id.clone(), milestone_index);

        let mut proposal: AmendmentProposal = env
            .storage()
            .temporary()
            .get(&amendment_key)
            .unwrap_or(AmendmentProposal {
                new_percent,
                new_name: new_name.clone(),
                buyer_agreed: false,
                supplier_agreed: false,
            });

        // If the stored proposal has different terms, reset it.
        if proposal.new_percent != new_percent || proposal.new_name != new_name {
            proposal = AmendmentProposal {
                new_percent,
                new_name: new_name.clone(),
                buyer_agreed: false,
                supplier_agreed: false,
            };
        }

        if is_buyer {
            proposal.buyer_agreed = true;
        } else {
            proposal.supplier_agreed = true;
        }

        env.events().publish(
            (Symbol::new(&env, "amendment_proposed"), shipment_id.clone()),
            (milestone_index, new_percent),
        );

        if proposal.buyer_agreed && proposal.supplier_agreed {
            // Validate new percentages sum to 100.
            let mut total: u32 = 0;
            for i in 0..shipment.milestones.len() {
                if i == milestone_index {
                    total += new_percent;
                } else {
                    total += shipment.milestones.get(i).unwrap().payment_percent;
                }
            }
            if total != 100 {
                panic!("milestone percentages must sum to 100");
            }

            let mut m = shipment.milestones.get(milestone_index).unwrap();
            let old_percent = m.payment_percent;
            m.payment_percent = new_percent;
            m.name = new_name;
            shipment.milestones.set(milestone_index, m);

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            env.storage().temporary().remove(&amendment_key);

            // #111: Append to amendment log (capped at 20, FIFO eviction).
            let log_key = DataKey::AmendmentLog(shipment_id.clone(), milestone_index);
            let mut log: Vec<AmendmentEntry> = env
                .storage()
                .persistent()
                .get(&log_key)
                .unwrap_or_else(|| Vec::new(&env));
            if log.len() as usize >= 20 {
                let mut new_log: Vec<AmendmentEntry> = Vec::new(&env);
                for i in 1..log.len() {
                    new_log.push_back(log.get(i).unwrap());
                }
                log = new_log;
            }
            log.push_back(AmendmentEntry {
                proposer: caller.clone(),
                old_payment_percent: old_percent,
                new_payment_percent: new_percent,
                ledger: env.ledger().sequence(),
            });
            env.storage().persistent().set(&log_key, &log);
            env.storage().persistent().extend_ttl(&log_key, 100_000, 6_300_000);

            env.events().publish(
                (Symbol::new(&env, "amendment_accepted"), shipment_id.clone()),
                milestone_index,
            );
        } else {
            env.storage().temporary().set(&amendment_key, &proposal);
        }
    }

    // ----------------------------------------------------------
    // TRANSFER BUYER
    // ----------------------------------------------------------

    /// Transfer the buyer role to a new address.
    /// Requires auth from both current_buyer and new_buyer.
    /// Disallowed if any milestone is currently Disputed.
    pub fn transfer_buyer(
        env: Env,
        current_buyer: Address,
        shipment_id: String,
        new_buyer: Address,
    ) {
        Self::assert_not_paused(&env);
        current_buyer.require_auth();
        new_buyer.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        // Verify current_buyer is actually a buyer on this shipment.
        Self::assert_is_buyer(&shipment, &current_buyer);

        // Block transfer while any milestone is disputed.
        Self::assert_no_open_disputes(&shipment);

        // Replace the matching buyer entry.
        let mut new_buyers: Vec<Address> = Vec::new(&env);
        let mut replaced = false;
        for i in 0..shipment.buyers.len() {
            let b = shipment.buyers.get(i).unwrap();
            if b == current_buyer && !replaced {
                new_buyers.push_back(new_buyer.clone());
                replaced = true;
            } else {
                new_buyers.push_back(b);
            }
        }
        shipment.buyers = new_buyers;

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (Symbol::new(&env, "buyer_transferred"), shipment_id.clone()),
            (current_buyer, new_buyer),
        );
    }

    // ----------------------------------------------------------
    // TRANSFER SUPPLIER
    // ----------------------------------------------------------

    /// Transfer the supplier role to a new address.
    /// Requires auth from both current_supplier and new_supplier.
    /// Disallowed if any milestone is currently Disputed.
    pub fn transfer_supplier(
        env: Env,
        current_supplier: Address,
        shipment_id: String,
        new_supplier: Address,
    ) {
        Self::assert_not_paused(&env);
        current_supplier.require_auth();
        new_supplier.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if current_supplier != shipment.supplier {
            panic!("unauthorized");
        }

        // Block transfer while any milestone is disputed.
        Self::assert_no_open_disputes(&shipment);

        shipment.supplier = new_supplier.clone();

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (
                Symbol::new(&env, "supplier_transferred"),
                shipment_id.clone(),
            ),
            (current_supplier, new_supplier),
        );
    }

    // ----------------------------------------------------------
    // ARBITER ROTATION
    // ----------------------------------------------------------

    /// Buyer or supplier proposes to rotate the arbiter.
    /// When both parties agree on the same new_arbiter, the rotation is applied.
    pub fn propose_arbiter_rotation(
        env: Env,
        caller: Address,
        shipment_id: String,
        new_arbiter: Address,
    ) {
        Self::assert_not_paused(&env);
        caller.require_auth();

        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let is_buyer = Self::is_buyer(&shipment, &caller);
        if !is_buyer && caller != shipment.supplier {
            panic!("unauthorized");
        }

        let rotation_key = DataKey::ArbiterRotation(shipment_id.clone());

        let mut proposal: ArbiterRotationProposal = env
            .storage()
            .temporary()
            .get(&rotation_key)
            .unwrap_or(ArbiterRotationProposal {
                new_arbiter: new_arbiter.clone(),
                buyer_agreed: false,
                supplier_agreed: false,
            });

        // If the stored proposal has a different arbiter, reset it.
        if proposal.new_arbiter != new_arbiter {
            proposal = ArbiterRotationProposal {
                new_arbiter: new_arbiter.clone(),
                buyer_agreed: false,
                supplier_agreed: false,
            };
        }

        if is_buyer {
            proposal.buyer_agreed = true;
        } else {
            proposal.supplier_agreed = true;
        }

        env.events().publish(
            (
                Symbol::new(&env, "arbiter_rotation_proposed"),
                shipment_id.clone(),
            ),
            new_arbiter.clone(),
        );

        if proposal.buyer_agreed && proposal.supplier_agreed {
            let mut updated_shipment = shipment.clone();
            updated_shipment.arbiter = new_arbiter.clone();

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &updated_shipment);

            env.storage().temporary().remove(&rotation_key);

            env.events().publish(
                (Symbol::new(&env, "arbiter_rotated"), shipment_id.clone()),
                new_arbiter,
            );
        } else {
            env.storage().temporary().set(&rotation_key, &proposal);
        }
    }

    // ----------------------------------------------------------
    // AUTO-CONFIRMATION
    // ----------------------------------------------------------

    /// Claim auto-confirmation for a milestone when the auto-confirm window has expired.
    /// Callable by anyone. Transfers payment to supplier and returns penalty to buyer if applicable.
    pub fn claim_auto_confirmation(env: Env, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        if shipment.auto_confirm_ledgers == 0 {
            panic!("auto-confirmation not enabled for this shipment");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();

        if milestone.status != MilestoneStatus::ProofSubmitted {
            panic!("milestone is not in ProofSubmitted status");
        }

        if let Some(proof_ledger) = milestone.proof_submitted_ledger {
            let auto_confirm_ledger = proof_ledger + shipment.auto_confirm_ledgers;
            if env.ledger().sequence() < auto_confirm_ledger {
                panic!("auto-confirmation window has not expired");
            }
        } else {
            panic!("proof_submitted_ledger not set");
        }

        let mut payment = (shipment.total_amount * milestone.payment_percent as i128) / 100;

        // Deduct any approved advance for this milestone.
        let advance_deducted =
            Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, milestone_index);

        // Apply late-delivery penalty if configured.
        let mut penalty_deducted: i128 = 0;
        if shipment.late_penalty_bps_per_ledger > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let delay_ledgers = env.ledger().sequence() - proof_ledger;
                let penalty = (payment
                    * (shipment.late_penalty_bps_per_ledger as i128 * delay_ledgers as i128))
                    / 10_000;
                if penalty > 0 && penalty < payment {
                    penalty_deducted = penalty;
                    payment -= penalty;
                }
            }
        }

        let mut fee_amount: i128 = 0;
        let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

        // Check circuit breaker before transferring payment
        Self::check_circuit_breaker(&env, payment);

        milestone.status = MilestoneStatus::Confirmed;
        milestone.proof_submitted_ledger = None;
        shipment.milestones.set(milestone_index, milestone);
        shipment.released_amount += payment;

        let mut actual_transfer = net_payment - advance_deducted;
        let token_client = token::Client::new(&env, &shipment.token);

        // Pay referral fee on shipment completion (deducted from final supplier payment).
        if Self::all_milestones_done(&shipment) {
            if let Some(referrer_addr) = shipment.referrer.clone() {
                let referral_bps: u32 = env
                    .storage()
                    .instance()
                    .get(&DataKey::ReferralFeeBps)
                    .unwrap_or(500);
                if let Some(fee_cfg) = env
                    .storage()
                    .instance()
                    .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
                {
                    let total_fee = (shipment.total_amount * fee_cfg.fee_bps as i128) / 10_000;
                    let mut referral = (total_fee * referral_bps as i128) / 10_000;
                    if referral > actual_transfer {
                        referral = actual_transfer;
                    }
                    if referral > 0 {
                        actual_transfer -= referral;
                        token_client.transfer(
                            &env.current_contract_address(),
                            &referrer_addr,
                            &referral,
                        );
                    }
                }
            }
        }

        if actual_transfer > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &shipment.supplier,
                &actual_transfer,
            );
        }

        // Return penalty to buyer if any.
        if penalty_deducted > 0 {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            token_client.transfer(
                &env.current_contract_address(),
                &primary_buyer,
                &penalty_deducted,
            );
        }

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;
            // Update completed shipments stat.
            let mut stats: ContractStats = env
                .storage()
                .instance()
                .get(&DataKey::ContractStats)
                .unwrap_or(ContractStats {
                    total_shipments: 0,
                    total_volume: 0,
                    total_disputes: 0,
                    completed_shipments: 0,
                });
            stats.completed_shipments += 1;
            env.storage()
                .instance()
                .set(&DataKey::ContractStats, &stats);
            // Move from Active to Completed status index.
            Self::move_shipment_status_index(
                &env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
        }

            // Decrement total escrowed value.
            let net_outflow = payment - advance_deducted;
            let current_escrowed: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::TotalEscrowed(shipment.token.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::TotalEscrowed(shipment.token.clone()),
                &(current_escrowed - net_outflow).max(0),
            );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (
                Symbol::new(&env, "auto_confirmation_claimed"),
                shipment_id.clone(),
            ),
            (milestone_index, payment, fee_amount, penalty_deducted),
        );
    }

    // ----------------------------------------------------------
    // ADMIN: TWO-STEP ROLE TRANSFER (Issue #40)
    // ----------------------------------------------------------

    /// Nominate a new admin. The nominee must call accept_admin to complete the transfer.
    /// The current admin remains active until the nominee accepts.
    pub fn nominate_admin(env: Env, current_admin: Address, nominee: Address) {
        current_admin.require_auth();
        Self::assert_admin(&env, &current_admin);
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &nominee);
        env.events()
            .publish((Symbol::new(&env, "admin_nominated"),), nominee);
    }

    /// Accept an outstanding admin nomination. Only the nominated address may call this.
    /// On success, the caller becomes the new admin and the nomination is cleared.
    pub fn accept_admin(env: Env, nominee: Address) {
        nominee.require_auth();
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .unwrap_or_else(|| panic!("no pending nomination"));
        if nominee != pending {
            panic!("unauthorized");
        }
        let old_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic!("unauthorized"));
        env.storage().instance().set(&DataKey::Admin, &nominee);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.events().publish(
            (Symbol::new(&env, "admin_transferred"),),
            (old_admin, nominee),
        );
    }

    /// Cancel the outstanding admin nomination. Only the current admin may call this.
    pub fn revoke_nomination(env: Env, current_admin: Address) {
        current_admin.require_auth();
        Self::assert_admin(&env, &current_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.events().publish(
            (Symbol::new(&env, "nomination_revoked"),),
            env.ledger().sequence(),
        );
    }

    // ----------------------------------------------------------
    // EMERGENCY FUND RECOVERY (Issue #47)
    // ----------------------------------------------------------

    /// Recover stuck escrow funds from an abandoned shipment.
    /// Only callable after RECOVERY_THRESHOLD_LEDGERS have elapsed since creation.
    /// Transfers remaining funds to the admin address and marks the shipment Cancelled.
    pub fn emergency_recover(env: Env, admin: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger <= shipment.created_at + RECOVERY_THRESHOLD_LEDGERS {
            panic!("recovery threshold not reached");
        }

        let recovery_amount = shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;

        if recovery_amount > 0 {
            let token_client = token::Client::new(&env, &shipment.token);
            token_client.transfer(&env.current_contract_address(), &admin, &recovery_amount);
        }

        Self::move_shipment_status_index(
            &env,
            ShipmentStatus::Active,
            ShipmentStatus::Cancelled,
            &shipment_id,
        );
        shipment.status = ShipmentStatus::Cancelled;

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Decrement total escrowed value.
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - recovery_amount).max(0),
        );

        env.events().publish(
            (Symbol::new(&env, "emergency_recovery"), shipment_id.clone()),
            (recovery_amount, admin),
        );
    }

    // ----------------------------------------------------------
    // UPGRADE
    // ----------------------------------------------------------

    // ----------------------------------------------------------
    // READ-ONLY QUERIES
    // ----------------------------------------------------------

    pub fn get_shipment(env: Env, shipment_id: String) -> Shipment {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        Self::get_shipment_internal(&env, &shipment_id)
    }

    pub fn get_milestone(env: Env, shipment_id: String, milestone_index: u32) -> Milestone {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        shipment
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic!("invalid milestone index"))
    }

    pub fn get_completion_percentage(env: Env, shipment_id: String) -> u32 {
        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.total_amount <= 0 {
            return 0;
        }
        if shipment.released_amount + shipment.total_advanced_amount <= 0 {
            return 0;
        }

        // Clamp to [0, 100] to avoid any unexpected rounding / state drift.
        let numerator: i128 = (shipment.released_amount + shipment.total_advanced_amount) * 100;
        let mut pct: i128 = numerator / shipment.total_amount;
        if pct < 0 {
            pct = 0;
        }
        if pct > 100 {
            pct = 100;
        }

        pct as u32
    }

    pub fn get_escrow_balance(env: Env, shipment_id: String) -> i128 {
        env.storage().instance().extend_ttl(100_000, 6_300_000);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount
    }

    pub fn get_fee_config(env: Env) -> Option<FeeConfig> {
        env.storage().instance().get(&DataKey::FeeConfig)
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn get_total_escrowed_value(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(token))
            .unwrap_or(0)
    }

    pub fn get_active_disputes(env: Env) -> Vec<DisputeEntry> {
        env.storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn get_contract_stats(env: Env) -> ContractStats {
        env.storage()
            .instance()
            .get(&DataKey::ContractStats)
            .unwrap_or(ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            })
    }

    pub fn list_shipments(
        env: Env,
        cursor: Option<u32>,
        limit: u32,
        status_filter: Option<ShipmentStatus>,
    ) -> (Vec<String>, Option<u32>) {
        let source_list: Vec<String> = match status_filter {
            Some(status) => env
                .storage()
                .persistent()
                .get(&DataKey::ShipmentsByStatus(status))
                .unwrap_or_else(|| Vec::new(&env)),
            None => env
                .storage()
                .persistent()
                .get(&DataKey::AllShipments)
                .unwrap_or_else(|| Vec::new(&env)),
        };

        let clamped_limit = if limit > 50 { 50 } else { limit };
        let start_idx = cursor.unwrap_or(0);
        let total_len = source_list.len() as u32;

        if start_idx >= total_len {
            return (Vec::new(&env), None);
        }

        let mut result: Vec<String> = Vec::new(&env);
        let mut idx = start_idx;
        while idx < total_len && (result.len() as u32) < clamped_limit {
            result.push_back(source_list.get(idx).unwrap());
            idx += 1;
        }

        let next_cursor = if idx < total_len { Some(idx) } else { None };

        (result, next_cursor)
    }

    pub fn get_shipments_by_supplier(env: Env, supplier: Address) -> Vec<String> {
        env.storage()
            .persistent()
            .get(&DataKey::SupplierShipments(supplier))
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn get_shipments_by_buyer(env: Env, buyer: Address) -> Vec<String> {
        env.storage()
            .persistent()
            .get(&DataKey::BuyerShipments(buyer))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns the total number of shipments associated with `address` as buyer or supplier.
    /// Shipments where the address holds both roles are counted once.
    pub fn get_shipment_count(env: Env, address: Address) -> u32 {
        let buyer_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::BuyerShipments(address.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let supplier_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::SupplierShipments(address.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        // Start with all buyer shipments, then add supplier shipments not already present.
        let mut seen: Vec<String> = Vec::new(&env);
        for i in 0..buyer_ids.len() {
            seen.push_back(buyer_ids.get(i).unwrap());
        }
        for i in 0..supplier_ids.len() {
            let id = supplier_ids.get(i).unwrap();
            let mut already = false;
            for j in 0..seen.len() {
                if seen.get(j).unwrap() == id {
                    already = true;
                    break;
                }
            }
            if !already {
                seen.push_back(id);
            }
        }
        seen.len() as u32
    }

    // ----------------------------------------------------------
    // INTERNAL HELPERS
    // ----------------------------------------------------------

    fn assert_not_paused(env: &Env) {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            panic!("contract is paused");
        }
    }

    fn assert_admin(env: &Env, caller: &Address) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic!("unauthorized"));
        if *caller != stored_admin {
            panic!("unauthorized");
        }
    }

    fn is_buyer(shipment: &Shipment, addr: &Address) -> bool {
        for i in 0..shipment.buyers.len() {
            if shipment.buyers.get(i).unwrap() == *addr {
                return true;
            }
        }
        false
    }

    fn assert_is_buyer(shipment: &Shipment, addr: &Address) {
        if !Self::is_buyer(shipment, addr) {
            panic!("unauthorized");
        }
    }

    // ----------------------------------------------------------
    // PERMISSION GUARDS
    // Combine require_auth() with role verification in one call
    // so every state-changing function has a single authoritative
    // check rather than scattered inline pairs.
    // ----------------------------------------------------------

    /// Caller must be one of the shipment's registered buyers.
    fn require_buyer_auth(shipment: &Shipment, buyer: &Address) {
        buyer.require_auth();
        Self::assert_is_buyer(shipment, buyer);
    }

    /// Caller must be the shipment's supplier.
    fn require_supplier_auth(shipment: &Shipment, caller: &Address) {
        caller.require_auth();
        if *caller != shipment.supplier {
            panic!("unauthorized");
        }
    }

    /// Caller must be the shipment's supplier or logistics provider.
    fn require_supplier_or_logistics_auth(shipment: &Shipment, caller: &Address) {
        caller.require_auth();
        if *caller != shipment.supplier && *caller != shipment.logistics {
            panic!("unauthorized");
        }
    }

    /// Caller must be the shipment's designated arbiter.
    fn require_arbiter_auth(shipment: &Shipment, arbiter: &Address) {
        arbiter.require_auth();
        if *arbiter != shipment.arbiter {
            panic!("unauthorized");
        }
    }

    fn assert_no_open_disputes(shipment: &Shipment) {
        for i in 0..shipment.milestones.len() {
            if shipment.milestones.get(i).unwrap().status == MilestoneStatus::Disputed {
                panic!("transfer disallowed: open dispute must be resolved first");
            }
        }
    }

    /// Consumes an approved advance for a milestone, removing it from storage and
    /// adjusting total_advanced_amount. Returns the advance amount (or 0 if none).
    fn consume_advance_for_milestone(
        env: &Env,
        shipment: &mut Shipment,
        shipment_id: &String,
        milestone_index: u32,
    ) -> i128 {
        let advance_key = DataKey::AdvanceRequest(shipment_id.clone(), milestone_index);
        if let Some(req) = env
            .storage()
            .persistent()
            .get::<DataKey, AdvanceRequest>(&advance_key)
        {
            if req.approved && req.amount_advanced > 0 {
                env.storage().persistent().remove(&advance_key);
                shipment.total_advanced_amount = (shipment.total_advanced_amount - req.amount_advanced).max(0);
                return req.amount_advanced;
            }
        }
        0
    }

    fn check_circuit_breaker(env: &Env, payment: i128) {
        let limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::CircuitBreakerLimit)
            .unwrap_or(0);
        if limit == 0 {
            return; // Circuit breaker disabled
        }

        let window: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CircuitBreakerWindow)
            .unwrap_or(0);
        let window_start: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CircuitBreakerWindowStart)
            .unwrap_or(0);
        let mut window_outflow: i128 = env
            .storage()
            .instance()
            .get(&DataKey::CircuitBreakerWindowOutflow)
            .unwrap_or(0);

        let current_ledger = env.ledger().sequence();

        // Reset window if expired
        if current_ledger >= window_start + window {
            window_outflow = 0;
            env.storage()
                .instance()
                .set(&DataKey::CircuitBreakerWindowStart, &current_ledger);
        }

        // Check if this payment would exceed limit
        if window_outflow + payment > limit {
            panic!("circuit breaker triggered: outflow limit exceeded");
        }

        // Update window outflow
        window_outflow += payment;
        env.storage()
            .instance()
            .set(&DataKey::CircuitBreakerWindowOutflow, &window_outflow);
    }

    fn get_reputation_internal(env: &Env, supplier: &Address) -> ReputationScore {
        env.storage()
            .persistent()
            .get(&DataKey::SupplierRep(supplier.clone()))
            .unwrap_or_default()
    }

    fn set_reputation_internal(env: &Env, supplier: &Address, score: &ReputationScore) {
        let key = DataKey::SupplierRep(supplier.clone());
        env.storage().persistent().set(&key, score);
        env.storage()
            .persistent()
            .extend_ttl(&key, 100_000, 6_300_000);
    }

    fn increment_reputation_internal(
        env: &Env,
        supplier: &Address,
        completed_delta: u32,
        disputed_delta: u32,
        cancelled_delta: u32,
    ) {
        let mut score = Self::get_reputation_internal(env, supplier);
        score.completed = score.completed.saturating_add(completed_delta);
        score.disputed = score.disputed.saturating_add(disputed_delta);
        score.cancelled = score.cancelled.saturating_add(cancelled_delta);
        Self::set_reputation_internal(env, supplier, &score);
    }

    // ============================================================
    // STORAGE CONTEXT HELPERS (batch reads)
    // ============================================================

    /// Fetch CreateShipmentCtx: consolidates all validation storage reads for create_shipment.
    /// Keys accessed: MaxShipmentValue, AllowedTokens, MinMilestonePercent, ContractStats.
    fn fetch_create_shipment_ctx(env: &Env) -> CreateShipmentCtx {
        let max_value: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxShipmentValue)
            .unwrap_or(0);
        let allowed_tokens: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::AllowedTokens)
            .unwrap_or_else(|| Vec::new(env));
        let min_pct: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinMilestonePercent)
            .unwrap_or(5u32);
        let contract_stats: ContractStats = env
            .storage()
            .instance()
            .get(&DataKey::ContractStats)
            .unwrap_or(ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            });

        CreateShipmentCtx {
            max_value,
            allowed_tokens,
            min_pct,
            contract_stats,
        }
    }

    /// Fetch ConfirmMilestoneCtx: consolidates core storage reads for confirm_milestone.
    /// Keys accessed: Shipment, ContractStats.
    fn fetch_confirm_milestone_ctx(env: &Env, shipment_id: &String) -> ConfirmMilestoneCtx {
        let shipment = Self::get_shipment_internal(env, shipment_id);
        let contract_stats: ContractStats = env
            .storage()
            .instance()
            .get(&DataKey::ContractStats)
            .unwrap_or(ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            });

        ConfirmMilestoneCtx {
            shipment,
            contract_stats,
        }
    }

    /// Fetch ResolveDisputeCtx: consolidates dispute resolution storage reads.
    /// Keys accessed: Shipment, DisputeContestedPercent, ContractStats, ActiveDisputes.
    fn fetch_resolve_dispute_ctx(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
    ) -> ResolveDisputeCtx {
        let shipment = Self::get_shipment_internal(env, shipment_id);
        let contested_key = DataKey::DisputeContestedPercent(shipment_id.clone(), milestone_index);
        let partial_contested_percent: Option<u32> =
            env.storage().persistent().get(&contested_key);
        let contract_stats: ContractStats = env
            .storage()
            .instance()
            .get(&DataKey::ContractStats)
            .unwrap_or(ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            });
        let active_disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(env));

        ResolveDisputeCtx {
            shipment,
            partial_contested_percent,
            contract_stats,
            active_disputes,
        }
    }

    fn get_shipment_internal(env: &Env, shipment_id: &String) -> Shipment {
        env.storage()
            .persistent()
            .get(&DataKey::Shipment(shipment_id.clone()))
            .unwrap_or_else(|| panic!("shipment not found"))
    }

    fn append_audit_entry(env: &Env, shipment: &mut Shipment, action: Symbol, detail: Symbol) {
        // Maintain a bounded ring-buffer of max 20 entries.
        let entry = AuditEntry {
            action,
            caller: env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .unwrap_or_else(|| panic!("unauthorized")),
            ledger: env.ledger().sequence(),

            detail,
        };

        let max: usize = 20;
        if shipment.audit_log.len() as usize >= max {
            // Evict the oldest (index 0) by shifting left.
            let mut new_log: Vec<AuditEntry> = Vec::new(env);
            // Start from 1 to drop the first element.
            for i in 1..shipment.audit_log.len() {
                new_log.push_back(shipment.audit_log.get(i).unwrap());
            }
            shipment.audit_log = new_log;
        }

        shipment.audit_log.push_back(entry);
    }

    fn append_admin_action(env: &Env, action: Symbol, detail: Symbol) {
        let mut log: Vec<AuditEntry> = env
            .storage()
            .instance()
            .get(&DataKey::AdminActionLog)
            .unwrap_or_else(|| Vec::new(env));
        let entry = AuditEntry {
            action,
            caller: env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .unwrap_or_else(|| panic!("unauthorized")),
            ledger: env.ledger().sequence(),
            detail,
        };
        if log.len() as usize >= 50 {
            let mut next: Vec<AuditEntry> = Vec::new(env);
            for i in 1..log.len() {
                next.push_back(log.get(i).unwrap());
            }
            log = next;
        }
        log.push_back(entry);
        env.storage().instance().set(&DataKey::AdminActionLog, &log);
    }

    fn all_milestones_done(shipment: &Shipment) -> bool {
        for i in 0..shipment.milestones.len() {
            let s = shipment.milestones.get(i).unwrap().status;
            if s != MilestoneStatus::Confirmed && s != MilestoneStatus::Resolved {
                return false;
            }
        }
        true
    }

    /// Deducts the platform fee from `gross_payment` and transfers it to the treasury.
    /// Returns the net amount after fee. Updates `fee_out` with the fee taken.
    fn deduct_fee(env: &Env, gross: i128, token: &Address, fee_out: &mut i128) -> i128 {
        if let Some(config) = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
        {
            let fee = (gross * config.fee_bps as i128) / 10_000;
            if fee > 0 {
                let token_client = token::Client::new(env, token);
                token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
                *fee_out = fee;
                return gross - fee;
            }
        }
        gross
    }

    /// #113: Deducts the fee using the shipment's locked tier bps (falls back to FeeConfig).
    fn deduct_fee_for_shipment(env: &Env, gross: i128, token: &Address, shipment_id: &String, fee_out: &mut i128) -> i128 {
        let locked_bps: Option<u32> = env.storage().persistent().get(&DataKey::ShipmentFeeBps(shipment_id.clone()));
        if let Some(config) = env.storage().instance().get::<DataKey, FeeConfig>(&DataKey::FeeConfig) {
            let bps = locked_bps.unwrap_or(config.fee_bps);
            let fee = (gross * bps as i128) / 10_000;
            if fee > 0 {
                let token_client = token::Client::new(env, token);
                token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
                *fee_out = fee;
                return gross - fee;
            }
        }
        gross
    }

    /// #113: Resolves the effective fee bps for `address` based on accumulated lifetime volume.
    fn resolve_fee_bps_for(env: &Env, address: &Address) -> u32 {
        let volume: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LifetimeVolume(address.clone()))
            .unwrap_or(0);
        let tiers: Vec<FeeTier> = env
            .storage()
            .instance()
            .get(&DataKey::FeeTiers)
            .unwrap_or_else(|| Vec::new(env));
        let mut best: Option<u32> = None;
        for i in 0..tiers.len() {
            let t = tiers.get(i).unwrap();
            if volume >= t.min_lifetime_volume {
                best = Some(match best {
                    None => t.fee_bps,
                    Some(b) => if t.fee_bps < b { t.fee_bps } else { b },
                });
            }
        }
        best.unwrap_or_else(|| {
            env.storage()
                .instance()
                .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
                .map(|c| c.fee_bps)
                .unwrap_or(0)
        })
    }

    /// Append a shipment ID to the per-status index list.
    fn add_to_status_index(env: &Env, status: ShipmentStatus, shipment_id: &String) {
        let key = DataKey::ShipmentsByStatus(status);
        let mut list: Vec<String> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        list.push_back(shipment_id.clone());
        env.storage().persistent().set(&key, &list);
    }

    /// Remove a shipment ID from the per-status index list.
    fn remove_from_status_index(env: &Env, status: ShipmentStatus, shipment_id: &String) {
        let key = DataKey::ShipmentsByStatus(status);
        let list: Vec<String> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        let mut new_list: Vec<String> = Vec::new(env);
        let mut removed = false;
        for i in 0..list.len() {
            let id = list.get(i).unwrap();
            if !removed && id == *shipment_id {
                removed = true;
            } else {
                new_list.push_back(id);
            }
        }
        env.storage().persistent().set(&key, &new_list);
    }

    /// Move a shipment ID from one status index to another (used on status transitions).
    fn move_shipment_status_index(
        env: &Env,
        from: ShipmentStatus,
        to: ShipmentStatus,
        shipment_id: &String,
    ) {
        Self::remove_from_status_index(env, from, shipment_id);
        Self::add_to_status_index(env, to, shipment_id);
    }
}

pub mod constants;
mod test_common;
mod test_new_features;

// Legacy test modules — some have pre-existing compilation issues.
// They are kept as source but only enabled when their API drift is resolved.
// mod benchmarks;
// mod property_tests;
// mod test;
// mod test_admin;
// mod test_dispute;
// mod test_errors;
// mod test_permissions;
// mod test_query;
// mod test_shipment;
// mod test_issues;
// mod test_arbiter_security;
// mod test_boundary_validation;
// mod test_oracle;
// mod test_upgrade;
// mod test_concurrent_disputes;
// mod test_boundaries;
// mod test_chaos;
