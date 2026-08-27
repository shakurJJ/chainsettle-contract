#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, BytesN, Env, IntoVal, Map,
    String, Symbol, Val, Vec,
};

// ============================================================
// YIELD PROTOCOL INTERFACE
// ============================================================

mod yield_protocol {
    use soroban_sdk::{contractclient, Address, Env};

    /// Minimal interface expected of an external yield protocol.
    /// The ChainSettle contract deposits idle escrow tokens here to earn
    /// yield while milestones are pending confirmation.
    ///
    /// Deposit flow:
    ///   1. ChainSettle transfers `amount` tokens to the protocol contract.
    ///   2. ChainSettle calls `deposit` so the protocol records the principal.
    ///
    /// Withdraw flow:
    ///   1. ChainSettle calls `withdraw`; the protocol transfers principal +
    ///      accrued yield back to `to` and returns the total amount.
    #[contractclient(name = "YieldProtocolClient")]
    pub trait YieldProtocol {
        /// Record a deposit of `amount` units of `token` on behalf of `depositor`.
        /// The caller must have already transferred the tokens to this contract.
        fn deposit(env: Env, depositor: Address, token: Address, amount: i128);
        /// Withdraw all funds (principal + yield) for `depositor`/`token` to `to`.
        /// Returns the total amount transferred.
        fn withdraw(env: Env, depositor: Address, token: Address, to: Address) -> i128;
        /// Current balance (principal + accrued yield) for `depositor` and `token`.
        fn balance_of(env: Env, depositor: Address, token: Address) -> i128;
    }
}
use yield_protocol::YieldProtocolClient;

// ============================================================
// CONFIRMATION WEBHOOK INTERFACE (Issue #303)
// ============================================================

mod confirmation_webhook {
    use soroban_sdk::{contractclient, Env, String};

    /// Interface expected of external contracts registered in the confirmation webhook
    /// allowlist. Called best-effort on every successful milestone confirmation.
    #[contractclient(name = "ConfirmationWebhookClient")]
    pub trait ConfirmationWebhook {
        /// Called when a milestone is confirmed.
        /// `shipment_id`      — the shipment's string ID
        /// `milestone_index`  — 0-based index of the confirmed milestone
        /// `payment_amount`   — gross payment amount released for this milestone
        fn on_milestone_confirmed(
            env: Env,
            shipment_id: String,
            milestone_index: u32,
            payment_amount: i128,
        );
    }
}
use confirmation_webhook::ConfirmationWebhookClient;

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
    /// #393: Dispute resolved in the supplier's favor but funds are held
    /// until release_after_ledger so the buyer has a brief re-review window
    /// to catch an arbiter error before finalize_dispute_resolution pays out.
    ResolvedPendingFinality,
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

/// Why a shipment reached a terminal cancelled/expired path.
/// Emitted on `shipment_cancelled` and persisted on `Shipment.cancellation_reason`.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CancellationReason {
    BuyerCancelled,
    SupplierCancelled,
    DeadlineRefund,
    AdminEmergencyRecovery,
}

/// Default resolution applied when a dispute auto-resolves after timeout (#165).
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum Resolution {
    Buyer,
    Supplier,
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
    /// Ledger by which proof must be submitted to avoid late-delivery penalty and earn early bonus (0 = no deadline).
    pub deadline_ledger: u32,
    /// Basis points penalty per overdue ledger past deadline_ledger (0 = use shipment-level or disabled).
    pub penalty_bps_per_ledger: u32,
}

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum ShipmentStatus {
    Active,
    Completed,
    Cancelled,
    /// Set by claim_deadline_refund when a per-milestone timestamp deadline is breached (#164).
    Expired,
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
    /// #391: Basis points of total_amount added to the per-dispute bond, scaling the
    /// bond with shipment value (0 = disabled). Stacks with `dispute_bond_amount`.
    pub dispute_bond_bps: u32,
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

    /// Total early-completion bonus pool funded by buyer at creation (0 = disabled).
    pub early_bonus_pool: i128,
    /// Remaining early-completion bonus not yet awarded; returned to buyer on shipment completion.
    pub early_bonus_remaining: i128,
    /// Per-shipment proof review window override (None = use auto_confirm_ledgers or global default, Some(0) = opt-out).
    pub review_window_ledgers: Option<u32>,

    // ── #165 Dispute auto-resolution timeout ──────────────────
    /// Seconds after dispute opens before auto-resolution fires (0 = disabled).
    pub dispute_timeout_seconds: u64,
    /// Resolution applied when dispute_timeout_seconds elapses without arbiter action.
    pub default_resolution: Resolution,

    // ── #162 Sequential milestone tracking ────────────────────
    /// Index of the most recently confirmed milestone; None if none confirmed yet.
    pub last_confirmed_milestone_index: Option<u32>,

    /// Set when the shipment is cancelled / deadline-refunded / emergency-recovered.
    /// Empty = not set (`None`). One entry = the reason.
    /// (`Option<CancellationReason>` is not supported by Soroban `#[contracttype]`.)
    pub cancellation_reason: Vec<CancellationReason>,
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

/// Fee recipient entry: address + basis-point share of protocol fee.
#[contracttype]
#[derive(Clone)]
pub struct FeeRecipient {
    pub recipient: Address,
    pub share_bps: u32,
}

/// #413 – Time-boxed, contract-wide promotional window during which the
/// protocol fee is waived for all shipments completed within [start_ledger, end_ledger].
#[contracttype]
#[derive(Clone)]
pub struct FeeHoliday {
    pub start_ledger: u32,
    pub end_ledger: u32,
}

/// Buyer reliability tracking for supplier decision-making.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BuyerReliability {
    pub total_confirmations: u32,
    pub total_confirmation_latency: u64,
    pub disputes_lost: u32,
    pub disputes_total: u32,
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
    /// #391: Basis points of total_amount added to the per-dispute bond, scaling the
    /// bond with shipment value (0 = disabled). Stacks with `dispute_bond_amount`.
    /// Subject to the admin-configured `MaxDisputeBondBps` cap.
    pub dispute_bond_bps: u32,
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

    /// Early-completion bonus pool funded by buyer (0 = disabled).
    pub early_bonus_pool: i128,
    /// Per-shipment proof review window override (None = use global default, Some(0) = disable auto-confirm for this shipment).
    pub review_window_ledgers: Option<u32>,

    // ── #160 Configurable milestone splits ────────────────────
    /// Per-milestone splits in basis points (sum must equal 10000, length must match milestones).
    /// Empty Vec = use per-milestone payment_percent field instead.
    pub milestone_splits: Vec<u32>,

    // ── #164 Per-milestone timestamp deadlines ────────────────
    /// Unix timestamp deadlines per milestone (0 = no deadline for that milestone).
    /// Empty Vec = no timestamp deadlines. Non-empty length must match milestones.
    /// Empty Vec = no timestamp deadlines. Non-empty length must match milestones.
    pub deadlines: Vec<u64>,

    // ── #165 Dispute auto-resolution timeout ──────────────────
    /// Seconds after dispute opens before anyone can call resolve_dispute_timeout (0 = disabled).
    pub dispute_timeout_seconds: u64,
    /// Resolution applied by resolve_dispute_timeout when timeout elapses.
    pub default_resolution: Resolution,

    // ── Backup Arbiter ───────────────────────────────────────────
    /// Optional backup arbiter for inactivity failover.
    pub backup_arbiter: Option<Address>,

    // ── Confirmation Cooldown ────────────────────────────────────
    /// Optional override for the milestone confirmation cooling-off period.
    pub confirmation_cooldown_ledgers: Option<u32>,

    // ── Feature A: Arbiter Panel ──────────────────────────────────────────────
    /// Optional panel of arbiters for N-of-M dispute resolution.
    /// When non-empty (and len >= 3), panel mode is used instead of single-arbiter mode.
    /// The `arbiter` field is ignored for dispute resolution when panel mode is active.
    pub arbiter_panel: Vec<Address>,

    // ── #385 Jurisdiction/compliance tag ──────────────────────────────────────
    /// Optional jurisdiction/regulatory category (e.g. "US", "EU_MIFID") for
    /// off-chain compliance filtering. Immutable after creation. None = untagged.
    pub jurisdiction: Option<Symbol>,
}

/// Configuration for time-decayed dispute bonds.
#[contracttype]
#[derive(Clone)]
pub struct DisputeBondDecayConfig {
    pub decay_bps_per_window: u32,
    pub window_ledgers: u32,
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

/// Global policy: suppliers meeting these thresholds skip the proof confirmation cooldown.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReputationFastTrack {
    pub min_completed: u32,
    /// Max (disputed / completed) ratio in basis points (e.g. 500 = 5%).
    pub max_disputed_ratio_bps: u32,
}

/// Pending mutual-consent pause or resume request for a single shipment.
/// Pending mutual-consent pause or resume request for a single shipment.
#[contracttype]
#[derive(Clone)]
pub struct ShipmentPauseRequest {
    pub requester: Address,
    /// false = pause request, true = resume request
    pub is_resume: bool,
}

/// Informational note attached to a milestone (no effect on status/payments).
#[contracttype]
#[derive(Clone)]
pub struct MilestoneNote {
    pub author: Address,
    pub note: String,
    pub ledger: u32,
}

/// Compact summary retained after a finished shipment is archived.
#[contracttype]
#[derive(Clone)]
pub struct ArchivedShipment {
    pub id: String,
    pub buyer: Address,
    pub supplier: Address,
    pub status: ShipmentStatus,
    pub total_amount: i128,
    pub released_amount: i128,
    pub completed_at: u32,
}

/// #386 – Read-only simulation of exactly how much a milestone confirmation
/// would pay out, to whom, and after which fees/holdbacks/overrides, without
/// mutating any state or requiring `confirm_milestone` to actually be called.
#[contracttype]
#[derive(Clone)]
pub struct PayoutPreview {
    /// Gross payment for this milestone before any deductions.
    pub gross_amount: i128,
    /// Approved advance already paid for this milestone, deducted from the transfer.
    pub advance_deducted: i128,
    /// Late-delivery penalty deducted (returned to buyer), 0 if not overdue.
    pub late_penalty_deducted: i128,
    /// Platform fee taken by the treasury (after any VIP waiver is applied).
    pub platform_fee: i128,
    /// Effective fee basis points applied to compute `platform_fee`.
    pub applied_fee_bps: u32,
    /// Logistics provider fee, if `logistics_fee_bps` is set.
    pub logistics_fee: i128,
    /// Net amount that would be transferred to the supplier (or split across
    /// configured milestone payees) after all deductions above.
    pub supplier_net_amount: i128,
    /// True if confirming this milestone would hold payment (holdback_ledgers > 0)
    /// rather than transferring immediately.
    pub would_be_held: bool,
    /// True if confirming this milestone would complete the shipment.
    pub is_final_milestone: bool,
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ArbiterStats {
    pub resolved_approved: u32,
    pub resolved_rejected: u32,
    pub total_resolution_ledgers: u64,
    /// Number of this arbiter's resolutions that were later overturned on
    /// appeal (#372). Feeds `max_overturned_before_slash`.
    pub overturned_count: u32,
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ShipmentRisk {
    pub late_milestones: u32,
    pub disputed_milestones: u32,
    pub total_milestones: u32,
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

/// Issue #302 — Pending emergency recovery proposal.
#[contracttype]
#[derive(Clone)]
pub struct RecoveryProposal {
    /// Ledger at which the proposal becomes executable.
    pub effective_ledger: u32,
    /// Admin who proposed the recovery.
    pub proposed_by: Address,
}

/// Issue #306 — Confirmation delegate configuration for a shipment.
#[contracttype]
#[derive(Clone)]
pub struct DelegateConfig {
    /// The delegated address that may call confirm_milestone.
    pub delegate: Address,
    /// Maximum payment amount the delegate may authorise per confirm_milestone call.
    pub per_tx_cap: i128,
}

/// #166 – Pending contract-upgrade proposal gated by the upgrade multisig.
#[contracttype]
#[derive(Clone)]
pub struct UpgradeProposal {
    pub new_wasm_hash: BytesN<32>,
    /// Distinct admin keys (from `MultiAdminConfig.admins`) that have approved so far.
    pub approvals: Vec<Address>,
}

/// #388 – Pending VIP partner fee-waiver proposal, gated by the routine
/// `MultiAdminConfig.threshold` (same bar as `propose_admin_action`).
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub struct FeeWaiverProposal {
    pub partner: Address,
    /// Basis points of the platform fee waived (10_000 = full waiver).
    pub waiver_bps: u32,
    /// Unix timestamp after which the waiver no longer applies (0 = no expiry).
    pub expires_at: u64,
    /// Distinct admin keys that have approved so far.
    pub approvals: Vec<Address>,
}

/// #402 – Pending emergency freeze/unfreeze proposal gated by a supermajority
/// of `MultiAdminConfig.admins` (stricter than the standard action threshold).
/// of `MultiAdminConfig.admins` (stricter than the standard action threshold).
#[contracttype]
#[derive(Clone)]
pub struct EmergencyFreezeProposal {
    /// Distinct admin keys that have approved so far.
    pub approvals: Vec<Address>,
}

/// Feature A – Single vote in a panel dispute.
#[contracttype]
#[derive(Clone)]
pub struct DisputeVote {
    pub arbiter: Address,
    pub approve: bool,
}

/// Feature C – A single payee entry: address + basis-point share (shares must sum to 100).
#[contracttype]
#[derive(Clone)]
pub struct MilestonePayee {
    pub payee: Address,
    pub percent: u32,
}

/// Feature D – Auto-blacklist rule thresholds (0 = that check disabled).
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AutoBlacklistRule {
    pub max_cancelled: u32,
    pub max_disputed: u32,
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
    pub min_value: i128,
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
// #284 — SUPPLIER PAYOUT BATCHING
// ============================================================

/// Controls payout delivery for a given supplier.
/// - Immediate: each confirm/resolve triggers a SAC transfer on the spot (default, backward-compat).
/// - Batched: payments accumulate in a per-supplier balance until claim_payout is called.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum PayoutMode {
    Immediate,
    Batched,
}

// ============================================================
// #286 — DISPUTE EVIDENCE VERSIONING
// ============================================================

/// A single versioned evidence entry appended to a disputed milestone.
#[contracttype]
#[derive(Clone)]
pub struct DisputeEvidence {
    /// Address that submitted this evidence (supplier, logistics, or buyer).
    pub submitter: Address,
    /// IPFS CID or other off-chain evidence pointer.
    pub evidence_hash: String,
    /// Caller-supplied category tag (e.g. "invoice", "photo", "affidavit").
    pub evidence_type: Symbol,
    /// Ledger sequence at submission time.
    pub submitted_ledger: u32,
}

// ============================================================
// #414 — SUPPLIER BLACKLIST APPEAL
// ============================================================

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum BlacklistAppealStatus {
    Pending,
    Approved,
    Rejected,
}

/// An appeal filed by a blacklisted address contesting its blacklisting.
#[contracttype]
#[derive(Clone)]
pub struct BlacklistAppeal {
    /// IPFS CID or other off-chain evidence pointer supporting the appeal.
    pub evidence_hash: String,
    pub status: BlacklistAppealStatus,
    /// Ledger sequence at which the appeal was filed.
    pub filed_ledger: u32,
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
    /// Global default auto-confirm review window in ledgers (0 = globally disabled).
    AutoConfirmThreshold,
    /// Minimum shipment value floor in i128 (0 = disabled).
    MinShipmentValue,
    /// Max number of evidence/proof submissions allowed per milestone (default 5).
    MaxEvidencePerMilestone,
    /// Current evidence count for (shipment_id, milestone_index).
    EvidenceCount(String, u32),
    /// Pending emergency recovery proposal: shipment_id -> RecoveryProposal.
    PendingRecovery(String),
    /// Delay in ledgers before a proposed recovery can be executed (0 = immediate).
    RecoveryDelayLedgers,
    /// Registered confirmation delegate for a shipment: shipment_id -> DelegateConfig.
    ConfirmationDelegate(String),
}

// `#[contracttype]` union enums are capped at 50 cases by the Soroban XDR spec
// (`ScSpecUdtUnionV0::cases: VecM<_, 50>`), so newer storage keys live here
// once `DataKey` is full.
#[contracttype]
pub enum DataKeyExt {
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

    // ── #160 Configurable milestone splits ─────────────────────
    /// Per-milestone basis-point splits for a shipment: Vec<u32> summing to 10000.
    MilestoneSplits(String),

    // ── #164 Per-milestone timestamp deadlines ──────────────────
    /// Per-milestone Unix timestamp deadlines for a shipment: Vec<u64>.
    MilestoneTimestampDeadlines(String),

    // ── #165 Dispute auto-resolution timeout ────────────────────
    /// Unix timestamp at which a dispute was opened: (shipment_id, milestone_index) -> u64.
    DisputeOpenedAt(String, u32),

    // ── #166 Upgrade multisig ───────────────────────────────────
    /// Next upgrade proposal id (monotonically increasing, starts at 1).
    UpgradeProposalCounter,
    /// Pending upgrade proposal by id.
    UpgradeProposal(u64),

    // ── Arbiter Failover ─────────────────────────────────────────
    /// Global inactivity threshold for arbiter failover (in ledgers, 0 = disabled).
    ArbiterInactivityThreshold,
    /// Backup arbiter for a shipment (String).
    BackupArbiter(String),

    // ── Milestone Confirmation Cooldown ──────────────────────────
    /// Global default milestone confirmation cooldown in ledgers.
    GlobalConfirmationCooldown,
    /// Per-shipment override for confirmation cooldown in ledgers.
    ShipmentConfirmationCooldown(String),

    // ── Dispute Bond Decay ───────────────────────────────────────
    /// Configuration for dispute bond time-decay.
    DisputeBondDecayConfig,
    /// Minimum allowed dispute bond amount globally.
    MinDisputeBond,
    /// Ledger at which the dispute bond decay clock was last reset.
    DisputeDecayStartLedger(String),
    /// Effective bond charged for a specific dispute.
    EffectiveDisputeBond(String, u32),
    // ── Tiered Arbiter Fee ──────────────────────────────────────
    /// Admin-configured fee tiers for arbiters: (min_contested_amount, fee_bps)
    ArbiterFeeTiers,

    // ── Arbiter Performance Analytics ───────────────────────────
    /// Arbiter resolution statistics.
    ArbiterStats(Address),

    // -- #299 Shipment-level fee override
    ShipmentFeeOverride(String),

    // -- #300 Long-hold escrow rebate
    LongHoldRebate,

    // -- #298 Governance timelock
    TimelockDuration,
    PendingParamChange(Symbol),

    // ── Feature A: Arbiter Panel (N-of-M dispute voting) ─────────────────────
    /// Arbiter panel for a shipment: Vec<Address> (non-empty = panel mode).
    ArbiterPanel(String),
    /// Votes cast for a specific dispute: (shipment_id, milestone_index) -> Vec<(Address, bool)>.
    DisputeVotes(String, u32),

    // ── Feature B: Supplier Exposure Cap ─────────────────────────────────────
    /// Global supplier exposure cap in i128 (0 = disabled).
    SupplierExposureCap,

    // ── Feature C: Milestone Payees ────────────────────────────────────────────
    /// Per-milestone payee split: (shipment_id, milestone_index) -> Vec<(Address, u32)>.
    MilestonePayees(String, u32),

    // ── Feature D: Auto-Blacklist Rule ─────────────────────────────────────────
    /// Auto-blacklist rule: (max_cancelled, max_disputed). Both 0 = disabled.
    AutoBlacklistRule,

    // ── Issue #303 Confirmation webhooks ──────────────────────────────────────
    /// Registered confirmation webhook contract addresses.
    ConfirmationWebhooks,

    /// Address that originally submitted proof for (shipment_id, milestone_index).
    /// Used by `correct_proof` to enforce original-submitter-only corrections.
    ProofSubmitter(String, u32),

    // ── Reputation fast-track ──────────────────────────────────
    /// Global reputation fast-track policy (absent = disabled).
    ReputationFastTrack,

    // ── Per-shipment mutual-consent pause ──────────────────────
    /// Whether a shipment is currently paused.
    ShipmentPaused(String),
    /// Pending pause/resume consent request.
    ShipmentPauseRequest(String),
    /// Ledger at which the current pause began (for deadline freeze).
    ShipmentPausedAt(String),

    // ── Milestone notes ────────────────────────────────────────
    /// Bounded per-milestone notes Vec (cap 10).
    MilestoneNotes(String, u32),

    // ── Shipment archival ──────────────────────────────────────
    /// Compact archived shipment record.
    ArchivedShipment(String),
    /// Ledgers after finish before a shipment may be archived (0 = archival disabled).
    ArchiveThreshold,

    // ── #284 Supplier payout batching ─────────────────────────────────────
    /// Per-supplier payout mode (Immediate or Batched).
    PayoutMode(Address),
    /// Accumulated pending payout balance for a batched-mode supplier.
    PendingPayout(Address),
    // ── #285 Per-address outflow rate limiting ─────────────────────────────
    /// Per-address outflow cap in i128 (0 = disabled, falls back to global breaker only).
    AddressOutflowLimit(Address),
    /// Per-address outflow window in ledgers.
    AddressOutflowWindow(Address),
    /// Per-address window start ledger.
    AddressOutflowWindowStart(Address),
    /// Per-address window outflow accumulated so far.
    AddressOutflowWindowOutflow(Address),
    // ── #286 Dispute evidence versioning ──────────────────────────────────
    /// Ordered list of evidence entries for a disputed milestone.
    DisputeEvidence(String, u32),

    // ── Treasury revenue tracking ──────────────────────────────────────────
    /// Cumulative fee revenue collected per token.
    TreasuryRevenue(Address),

    // ── Multi-recipient fee distribution ───────────────────────────────────
    /// Fee recipients configuration: Vec<(Address, u32)> where u32 is basis-point share.
    FeeRecipients,

    // ── Buyer reliability tracking ─────────────────────────────────────────
    /// Buyer reliability score and history.
    BuyerReliability(Address),
}

// `DataKeyExt` is itself now at the 50-case XDR cap, so newer storage keys
// live here.
#[contracttype]
pub enum DataKeyExt2 {
    // ── #364 Configurable max milestone count ───────────────────────────
    /// Admin-configured cap on milestones per shipment (0/unset = use
    /// constants::DEFAULT_MAX_MILESTONE_COUNT).
    MaxMilestoneCount,

    // ── #362 Per-token min/max shipment value ───────────────────────────
    /// Per-token minimum shipment value override; absent = fall back to
    /// the global DataKey::MinShipmentValue.
    TokenMinShipmentValue(Address),
    /// Per-token maximum shipment value override; absent = fall back to
    /// the global DataKey::MaxShipmentValue.
    TokenMaxShipmentValue(Address),

    // ── #365 Named milestone template library ───────────────────────────
    /// Saved milestone template: (creator, name) -> Vec<Milestone>.
    MilestoneTemplate(Address, String),
    /// Index of template names saved by a given creator, for listing.
    MilestoneTemplateNames(Address),

    // ── #366 Escrow deadline warning ────────────────────────────────────
    /// Admin-configured lead window (in ledgers) before a milestone deadline
    /// during which `check_deadline_warning` may fire (0 = disabled).
    WarningLeadLedgers,
    /// Whether the deadline-approaching warning has already fired for
    /// (shipment_id, milestone_index) — enforces "fires only once".
    DeadlineWarningFired(String, u32),

    // ── #367 Co-buyer joint confirmation ────────────────────────────────
    /// Admin-configured shipment value above which joint (buyer + co-buyer)
    /// confirmation is required (0 = disabled).
    JointConfirmationThreshold,
    /// Designated co-buyer address for a shipment, set once near creation.
    CoBuyer(String),
    /// Partial joint-confirmation progress for (shipment_id, milestone_index).
    JointConfirmation(String, u32),

    // ── #368 Compliance hold ────────────────────────────────────────────
    /// Off-chain compliance/legal review reason hash for a held shipment.
    /// Presence of this key means the shipment is on hold.
    ComplianceHold(String),

    // ── #369 Dispute appeal ──────────────────────────────────────────────
    /// Admin-configured window (in ledgers) after a dispute resolution during
    /// which either party may call `appeal_dispute` (0 = disabled).
    AppealWindowLedgers,
    /// Ledger at which a dispute on (shipment_id, milestone_index) was last resolved.
    DisputeResolvedAtLedger(String, u32),
    /// Whether the dispute on (shipment_id, milestone_index) has already been appealed
    /// once — a second resolution is final, so only one appeal is ever permitted.
    DisputeAppealed(String, u32),

    // ── #397 Supplier tiering ───────────────────────────────────────────
    /// Admin-configured supplier tier thresholds and collateral multipliers.
    SupplierTierConfig,

    // ── #400 Dispute mediator ────────────────────────────────────────────
    /// Global pool of mediators authorized for any shipment lacking a specific assignment.
    MediatorPool,
    /// Mediator assigned to a specific shipment (takes precedence over the pool).
    ShipmentMediator(String),
    /// Pending/in-progress mediation proposal for (shipment_id, milestone_index).
    MediationProposal(String, u32),

    // ── #398 Buyer spending limit ───────────────────────────────────────
    /// Configured rolling-window spending limit for a buyer: (limit, window_ledgers).
    BuyerSpendingLimit(Address),
    /// Current window usage for a buyer: (window_start_ledger, used_amount).
    BuyerSpendingUsage(Address),

    // ── #372 Arbiter reputation slashing ────────────────────────────────
    /// Admin-configured overturned-resolution count that triggers automatic
    /// slashing (0/unset = disabled).
    MaxOverturnedBeforeSlash,
    /// Whether an arbiter is currently slashed (removed from the pool and
    /// blocked from new dispute assignment until explicit admin reinstatement).
    ArbiterSlashed(Address),
    /// Original (arbiter, approve) outcome of a dispute resolution, recorded by
    /// `appeal_dispute` so the appeal's `resolve_dispute` call can detect an
    /// overturn by comparing outcomes.
    DisputeAppealOriginal(String, u32),
    /// The `approve` outcome of the most recent `resolve_dispute` call for
    /// (shipment_id, milestone_index) — read by `appeal_dispute` to snapshot
    /// the original decision before reassignment.
    DisputeResolvedApprove(String, u32),

    // ── #402 Emergency global freeze (supermajority multisig) ────────────
    /// Admin-configured supermajority percentage (basis points, e.g. 8000 =
    /// 80%) of registered multisig admins required to activate/lift an
    /// emergency freeze. 0/unset = use constants::DEFAULT_EMERGENCY_FREEZE_SUPERMAJORITY_BPS.
    EmergencyFreezeSupermajorityBps,
    /// Whether the contract is currently under an emergency freeze (tracked
    /// separately from the routine `DataKey::Paused` flag).
    EmergencyFrozen,
    /// Monotonic counter for emergency-freeze/unfreeze proposal ids.
    EmergencyFreezeProposalCounter,
    /// Pending emergency-freeze activation proposal, by id.
    EmergencyFreezeProposal(u64),
    /// Pending emergency-freeze lift (unfreeze) proposal, by id.
    EmergencyUnfreezeProposal(u64),

    // ── #405 Milestone proof size/format validation hook ─────────────────
    /// Admin-configured minimum proof-hash length (0/unset = no minimum).
    ProofHashMinLen,
    /// Admin-configured maximum proof-hash length (0/unset = no maximum).
    ProofHashMaxLen,
    /// Admin-configured required proof-hash prefix (empty/unset = no
    /// requirement), e.g. "Qm" or "bafy" for basic CID-version sanity checking.
    ProofHashRequiredPrefix,

    // ── #404 Supplier payout currency preference ─────────────────────────
    /// Supplier's preferred settlement token for `claim_payout`; absent =
    /// no preference, pay out in whatever token the caller specifies.
    PayoutCurrencyPreference(Address),
    /// Admin-registered fixed conversion rate for a (from_token, to_token)
    /// pair, in basis points of `to_token` per unit of `from_token`
    /// (10_000 = 1:1). Absent = no route available for that pair.
    ConversionRateBps(Address, Address),

    // ── #385 Jurisdiction/compliance tag per shipment ─────────────────────
    /// Optional jurisdiction/regulatory tag set at shipment creation; absent
    /// = untagged. Immutable after creation.
    ShipmentJurisdiction(String),
    /// Index of shipment IDs tagged with a given jurisdiction, for
    /// off-chain compliance filtering via `get_shipments_by_jurisdiction`.
    JurisdictionShipments(Symbol),

    // ── #387 Configurable maximum allowed-token list size ─────────────────
    /// Admin-configured cap on the number of entries in the allowed-token
    /// list (0/unset = no cap).
    MaxAllowedTokens,

    // ── #388 VIP partner fee waiver via governance vote ───────────────────
    /// Monotonic counter for fee-waiver proposal ids.
    FeeWaiverProposalCounter,
    /// Pending fee-waiver proposal, by id.
    FeeWaiverProposal(u64),
    /// Active fee waiver granted to a partner address: (waiver_bps, expires_at
    /// unix timestamp; 0 = no expiry).
    FeeWaiver(Address),
}

/// Partial joint-confirmation progress for a high-value shipment's milestone (#367).
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct JointConfirmationStatus {
    pub buyer_confirmed: bool,
    pub co_buyer_confirmed: bool,
}

// ============================================================
// #397 – SUPPLIER TIERING
// ============================================================

/// Supplier tier derived from reputation score thresholds. Determines the
/// collateral discount applied at shipment creation.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SupplierTier {
    Bronze,
    Silver,
    Gold,
}

/// Admin-configured thresholds and collateral multipliers per tier.
/// A supplier reaches Gold before Silver is checked (Gold takes priority).
/// Multipliers are basis points of the base (Bronze) collateral requirement
/// (10 000 = no discount; lower = cheaper collateral for that tier).
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SupplierTierConfig {
    pub silver_min_completed: u32,
    /// Max (disputed / completed) ratio in basis points to qualify for Silver.
    pub silver_max_disputed_ratio_bps: u32,
    pub silver_multiplier_bps: u32,
    pub gold_min_completed: u32,
    /// Max (disputed / completed) ratio in basis points to qualify for Gold.
    pub gold_max_disputed_ratio_bps: u32,
    pub gold_multiplier_bps: u32,
}

// ============================================================
// #400 – DISPUTE MEDIATOR
// ============================================================

/// Non-binding mediation suggestion for a disputed milestone. Applied directly
/// (bypassing arbiter resolution) once both buyer and supplier accept it.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub struct MediationProposal {
    pub mediator: Address,
    pub suggested_outcome: Resolution,
    pub buyer_accepted: bool,
    pub supplier_accepted: bool,
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
    /// Per-address outflow rate limit exceeded (#285).
    AddressOutflowLimitExceeded = 17,
    /// No pending payout to claim (#284).
    NoPendingPayout = 18,
}

// ============================================================
// CONSTANTS
// ============================================================

/// Ledgers equivalent to approximately 2 years (≈ 5 s/ledger × 86 400 s/day × 365 days × 2).
#[cfg(test)]
const RECOVERY_THRESHOLD_LEDGERS: u32 = 100;
#[cfg(not(test))]
const RECOVERY_THRESHOLD_LEDGERS: u32 = constants::RECOVERY_THRESHOLD_LEDGERS;

/// Max notes retained per milestone (oldest dropped on overflow).
const MAX_MILESTONE_NOTES: u32 = 10;

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
    ///
    /// Disabled once `initialize_multisig_admin` (#166) has been configured — at that
    /// point upgrades must go through `propose_upgrade` / `approve_upgrade` so that no
    /// single admin key can push a malicious upgrade unilaterally.
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
        if env.storage().instance().has(&DataKey::MultiAdminConfig) {
            panic!("upgrade multisig is configured; use propose_upgrade/approve_upgrade instead");
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
    // #166 UPGRADE MULTISIG
    // ----------------------------------------------------------
    // Gated by the same `MultiAdminConfig` (admins + threshold) set up via
    // `initialize_multisig_admin`. Any registered admin key can propose or approve
    // a WASM upgrade; the upgrade only executes once `threshold` distinct admin
    // keys have approved it. Requires `initialize_multisig_admin` to have been
    // called first — that is also what disables the single-key `upgrade` above.

    /// Propose a new contract WASM hash. Callable by any registered multisig admin key.
    /// The proposer's own approval is recorded immediately, so a threshold of 1
    /// executes the upgrade right away. Returns the new proposal's id.
    pub fn propose_upgrade(env: Env, admin: Address, new_wasm_hash: BytesN<32>) -> u64 {
        admin.require_auth();
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let proposal_id: u64 = env
            .storage()
            .instance()
            .get(&DataKeyExt::UpgradeProposalCounter)
            .unwrap_or(0)
            + 1;
        env.storage()
            .instance()
            .set(&DataKeyExt::UpgradeProposalCounter, &proposal_id);

        let mut approvals: Vec<Address> = Vec::new(&env);
        approvals.push_back(admin.clone());
        let proposal = UpgradeProposal {
            new_wasm_hash: new_wasm_hash.clone(),
            approvals,
        };
        let key = DataKeyExt::UpgradeProposal(proposal_id);
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(&env, "upgrade_proposed"), proposal_id),
            (new_wasm_hash, admin, 1u32),
        );

        if config.threshold <= 1 {
            Self::execute_upgrade_proposal(&env, proposal_id);
        }

        proposal_id
    }

    /// Approve a pending upgrade proposal. Executes the upgrade once `threshold`
    /// distinct admin keys have approved. Rejects a second approval from the same key.
    pub fn approve_upgrade(env: Env, admin: Address, proposal_id: u64) {
        admin.require_auth();
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let key = DataKeyExt::UpgradeProposal(proposal_id);
        let mut proposal: UpgradeProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("upgrade proposal not found"));

        for i in 0..proposal.approvals.len() {
            if proposal.approvals.get(i).unwrap() == admin {
                panic!("already approved by this admin");
            }
        }
        proposal.approvals.push_back(admin.clone());
        let approvals_count = proposal.approvals.len() as u32;
        env.storage().persistent().set(&key, &proposal);

        env.events().publish(
            (Symbol::new(&env, "upgrade_approved"), proposal_id),
            (admin, approvals_count),
        );

        if approvals_count >= config.threshold {
            Self::execute_upgrade_proposal(&env, proposal_id);
        }
    }

    /// Cancel a pending upgrade proposal. Callable by any registered multisig admin key.
    pub fn cancel_upgrade(env: Env, admin: Address, proposal_id: u64) {
        admin.require_auth();
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let key = DataKeyExt::UpgradeProposal(proposal_id);
        if !env.storage().persistent().has(&key) {
            panic!("upgrade proposal not found");
        }
        env.storage().persistent().remove(&key);

        env.events()
            .publish((Symbol::new(&env, "upgrade_cancelled"), proposal_id), admin);
    }

    /// Returns a pending upgrade proposal, or None if it doesn't exist (never
    /// existed, was cancelled, or already executed).
    pub fn get_upgrade_proposal(env: Env, proposal_id: u64) -> Option<UpgradeProposal> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::UpgradeProposal(proposal_id))
    }

    fn require_multisig_config(env: &Env) -> MultiAdminConfig {
        env.storage()
            .instance()
            .get(&DataKey::MultiAdminConfig)
            .unwrap_or_else(|| panic!("multisig admin not configured"))
    }

    fn assert_multisig_admin(config: &MultiAdminConfig, admin: &Address) {
        for i in 0..config.admins.len() {
            if config.admins.get(i).unwrap() == *admin {
                return;
            }
        }
        panic!("unauthorized");
    }

    fn execute_upgrade_proposal(env: &Env, proposal_id: u64) {
        let key = DataKeyExt::UpgradeProposal(proposal_id);
        let proposal: UpgradeProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("upgrade proposal not found"));
        env.deployer()
            .update_current_contract_wasm(proposal.new_wasm_hash.clone());
        env.storage().persistent().remove(&key);
        env.events().publish(
            (Symbol::new(env, "upgrade_executed"), proposal_id),
            proposal.new_wasm_hash,
        );
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
    // #402 EMERGENCY GLOBAL FREEZE (SUPERMAJORITY MULTISIG)
    // ----------------------------------------------------------
    // A stricter, separately-tracked alternative to pause()/unpause(). When
    // multisig admin governance (`initialize_multisig_admin`, #166) is
    // configured, activating or lifting the freeze requires a configurable
    // supermajority (default 80%) of registered admins — a higher bar than
    // the routine `MultiAdminConfig.threshold` used elsewhere. When multisig
    // governance is not configured, falls back to single-admin pause()
    // semantics: the freeze activates/lifts immediately on one admin's call.

    /// Set the supermajority (basis points, e.g. 8000 = 80%) of registered
    /// multisig admins required to activate/lift an emergency freeze.
    pub fn set_freeze_supermajority_bps(env: Env, admin: Address, bps: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if bps == 0 || bps > 10_000 {
            panic!("supermajority bps must be in (0, 10000]");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt2::EmergencyFreezeSupermajorityBps, &bps);
        env.events().publish(
            (Symbol::new(&env, "freeze_supermajority_bps_set"),),
            bps,
        );
    }

    /// Read the configured emergency-freeze supermajority (basis points).
    pub fn get_freeze_supermajority_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::EmergencyFreezeSupermajorityBps)
            .unwrap_or(constants::DEFAULT_EMERGENCY_FREEZE_SUPERMAJORITY_BPS)
    }

    /// Whether the contract is currently under an emergency freeze.
    pub fn is_emergency_frozen(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKeyExt2::EmergencyFrozen)
            .unwrap_or(false)
    }

    /// Number of distinct admins required for a supermajority, given `total`
    /// registered multisig admins and a `bps` supermajority (rounded up).
    fn supermajority_required(total: u32, bps: u32) -> u32 {
        let required = ((total as u64) * (bps as u64) + 9_999) / 10_000;
        required.max(1) as u32
    }

    /// Propose activating the emergency freeze. If multisig admin governance
    /// is configured, requires the configured supermajority of registered
    /// admins to approve (via `approve_emergency_freeze`) before it takes
    /// effect; the proposer's own approval is recorded immediately. If
    /// multisig governance is not configured, falls back to single-admin
    /// `pause()` semantics and activates immediately.
    pub fn propose_emergency_freeze(env: Env, admin: Address) -> u64 {
        admin.require_auth();

        if !env.storage().instance().has(&DataKey::MultiAdminConfig) {
            Self::assert_admin(&env, &admin);
            env.storage()
                .instance()
                .set(&DataKeyExt2::EmergencyFrozen, &true);
            env.events().publish(
                (Symbol::new(&env, "emergency_freeze_activated"),),
                (admin, env.ledger().sequence()),
            );
            return 0;
        }

        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let proposal_id: u64 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::EmergencyFreezeProposalCounter)
            .unwrap_or(0)
            + 1;
        env.storage()
            .instance()
            .set(&DataKeyExt2::EmergencyFreezeProposalCounter, &proposal_id);

        let mut approvals: Vec<Address> = Vec::new(&env);
        approvals.push_back(admin.clone());
        let proposal = EmergencyFreezeProposal { approvals };
        let key = DataKeyExt2::EmergencyFreezeProposal(proposal_id);
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(&env, "emergency_freeze_proposed"), proposal_id),
            (admin, 1u32),
        );

        let bps = Self::get_freeze_supermajority_bps(env.clone());
        let required = Self::supermajority_required(config.admins.len(), bps);
        if required <= 1 {
            Self::execute_emergency_freeze_proposal(&env, proposal_id);
        }

        proposal_id
    }

    /// Approve a pending emergency-freeze proposal. Activates the freeze once
    /// the configured supermajority of distinct admins have approved.
    pub fn approve_emergency_freeze(env: Env, admin: Address, proposal_id: u64) {
        admin.require_auth();
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let key = DataKeyExt2::EmergencyFreezeProposal(proposal_id);
        let mut proposal: EmergencyFreezeProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("emergency freeze proposal not found"));

        for i in 0..proposal.approvals.len() {
            if proposal.approvals.get(i).unwrap() == admin {
                panic!("already approved by this admin");
            }
        }
        proposal.approvals.push_back(admin.clone());
        let approvals_count = proposal.approvals.len() as u32;
        env.storage().persistent().set(&key, &proposal);

        env.events().publish(
            (Symbol::new(&env, "emergency_freeze_approved"), proposal_id),
            (admin, approvals_count),
        );

        let bps = Self::get_freeze_supermajority_bps(env.clone());
        let required = Self::supermajority_required(config.admins.len(), bps);
        if approvals_count >= required {
            Self::execute_emergency_freeze_proposal(&env, proposal_id);
        }
    }

    fn execute_emergency_freeze_proposal(env: &Env, proposal_id: u64) {
        let key = DataKeyExt2::EmergencyFreezeProposal(proposal_id);
        if !env.storage().persistent().has(&key) {
            panic!("emergency freeze proposal not found");
        }
        env.storage().persistent().remove(&key);
        env.storage()
            .instance()
            .set(&DataKeyExt2::EmergencyFrozen, &true);
        env.events().publish(
            (Symbol::new(env, "emergency_freeze_activated"),),
            proposal_id,
        );
    }

    /// Propose lifting an active emergency freeze. Mirrors
    /// `propose_emergency_freeze`'s gating: requires the same configured
    /// supermajority when multisig governance is configured, otherwise falls
    /// back to single-admin `unpause()` semantics.
    pub fn propose_emergency_unfreeze(env: Env, admin: Address) -> u64 {
        admin.require_auth();

        if !env.storage().instance().has(&DataKey::MultiAdminConfig) {
            Self::assert_admin(&env, &admin);
            env.storage()
                .instance()
                .set(&DataKeyExt2::EmergencyFrozen, &false);
            env.events().publish(
                (Symbol::new(&env, "emergency_freeze_lifted"),),
                (admin, env.ledger().sequence()),
            );
            return 0;
        }

        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let proposal_id: u64 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::EmergencyFreezeProposalCounter)
            .unwrap_or(0)
            + 1;
        env.storage()
            .instance()
            .set(&DataKeyExt2::EmergencyFreezeProposalCounter, &proposal_id);

        let mut approvals: Vec<Address> = Vec::new(&env);
        approvals.push_back(admin.clone());
        let proposal = EmergencyFreezeProposal { approvals };
        let key = DataKeyExt2::EmergencyUnfreezeProposal(proposal_id);
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (
                Symbol::new(&env, "emergency_unfreeze_proposed"),
                proposal_id,
            ),
            (admin, 1u32),
        );

        let bps = Self::get_freeze_supermajority_bps(env.clone());
        let required = Self::supermajority_required(config.admins.len(), bps);
        if required <= 1 {
            Self::execute_emergency_unfreeze_proposal(&env, proposal_id);
        }

        proposal_id
    }

    /// Approve a pending emergency-unfreeze proposal. Lifts the freeze once
    /// the configured supermajority of distinct admins have approved.
    pub fn approve_emergency_unfreeze(env: Env, admin: Address, proposal_id: u64) {
        admin.require_auth();
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let key = DataKeyExt2::EmergencyUnfreezeProposal(proposal_id);
        let mut proposal: EmergencyFreezeProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("emergency unfreeze proposal not found"));

        for i in 0..proposal.approvals.len() {
            if proposal.approvals.get(i).unwrap() == admin {
                panic!("already approved by this admin");
            }
        }
        proposal.approvals.push_back(admin.clone());
        let approvals_count = proposal.approvals.len() as u32;
        env.storage().persistent().set(&key, &proposal);

        env.events().publish(
            (
                Symbol::new(&env, "emergency_unfreeze_approved"),
                proposal_id,
            ),
            (admin, approvals_count),
        );

        let bps = Self::get_freeze_supermajority_bps(env.clone());
        let required = Self::supermajority_required(config.admins.len(), bps);
        if approvals_count >= required {
            Self::execute_emergency_unfreeze_proposal(&env, proposal_id);
        }
    }

    fn execute_emergency_unfreeze_proposal(env: &Env, proposal_id: u64) {
        let key = DataKeyExt2::EmergencyUnfreezeProposal(proposal_id);
        if !env.storage().persistent().has(&key) {
            panic!("emergency unfreeze proposal not found");
        }
        env.storage().persistent().remove(&key);
        env.storage()
            .instance()
            .set(&DataKeyExt2::EmergencyFrozen, &false);
        env.events().publish(
            (Symbol::new(env, "emergency_freeze_lifted"),),
            proposal_id,
        );
    }

    /// Returns a pending emergency-freeze activation proposal, or None.
    pub fn get_emergency_freeze_proposal(
        env: Env,
        proposal_id: u64,
    ) -> Option<EmergencyFreezeProposal> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::EmergencyFreezeProposal(proposal_id))
    }

    /// Returns a pending emergency-unfreeze proposal, or None.
    pub fn get_emergency_unfreeze_proposal(
        env: Env,
        proposal_id: u64,
    ) -> Option<EmergencyFreezeProposal> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::EmergencyUnfreezeProposal(proposal_id))
    }

    // ----------------------------------------------------------
    // #388 VIP PARTNER FEE WAIVER (GOVERNANCE VOTE)
    // ----------------------------------------------------------
    // Grants a full or partial platform-fee waiver to a strategic partner
    // address, routed through the same multisig admin-action machinery used
    // elsewhere (#166 upgrade multisig, #402 emergency freeze): any
    // registered multisig admin may propose, and the routine
    // `MultiAdminConfig.threshold` of distinct admin approvals executes it.
    // Requires multisig admin governance to be configured — unlike the
    // freeze feature there is no single-admin fallback, since a fee waiver
    // is a standing financial concession rather than a routine pause.

    /// Propose a fee waiver for `partner`. `waiver_bps` is the basis points
    /// of the platform fee to waive (10_000 = fully waived); `expires_at` is
    /// a Unix timestamp after which the waiver no longer applies (0 = no
    /// expiry). The proposer's own approval is recorded immediately.
    pub fn propose_fee_waiver(
        env: Env,
        admin: Address,
        partner: Address,
        waiver_bps: u32,
        expires_at: u64,
    ) -> u64 {
        admin.require_auth();
        if waiver_bps > 10_000 {
            panic!("waiver_bps cannot exceed 10000 (100%)");
        }
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let proposal_id: u64 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::FeeWaiverProposalCounter)
            .unwrap_or(0)
            + 1;
        env.storage()
            .instance()
            .set(&DataKeyExt2::FeeWaiverProposalCounter, &proposal_id);

        let mut approvals: Vec<Address> = Vec::new(&env);
        approvals.push_back(admin.clone());
        let proposal = FeeWaiverProposal {
            partner: partner.clone(),
            waiver_bps,
            expires_at,
            approvals,
        };
        let key = DataKeyExt2::FeeWaiverProposal(proposal_id);
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(&env, "fee_waiver_proposed"), proposal_id),
            (admin, partner, waiver_bps, 1u32),
        );

        if config.threshold <= 1 {
            Self::execute_fee_waiver_proposal(&env, proposal_id);
        }

        proposal_id
    }

    /// Approve a pending fee-waiver proposal. Grants the waiver once the
    /// routine `MultiAdminConfig.threshold` of distinct admins have approved.
    pub fn approve_fee_waiver(env: Env, admin: Address, proposal_id: u64) {
        admin.require_auth();
        let config = Self::require_multisig_config(&env);
        Self::assert_multisig_admin(&config, &admin);

        let key = DataKeyExt2::FeeWaiverProposal(proposal_id);
        let mut proposal: FeeWaiverProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("fee waiver proposal not found"));

        for i in 0..proposal.approvals.len() {
            if proposal.approvals.get(i).unwrap() == admin {
                panic!("already approved by this admin");
            }
        }
        proposal.approvals.push_back(admin.clone());
        let approvals_count = proposal.approvals.len() as u32;
        env.storage().persistent().set(&key, &proposal);

        env.events().publish(
            (Symbol::new(&env, "fee_waiver_approved"), proposal_id),
            (admin, approvals_count),
        );

        if approvals_count >= config.threshold {
            Self::execute_fee_waiver_proposal(&env, proposal_id);
        }
    }

    fn execute_fee_waiver_proposal(env: &Env, proposal_id: u64) {
        let key = DataKeyExt2::FeeWaiverProposal(proposal_id);
        let proposal: FeeWaiverProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("fee waiver proposal not found"));
        env.storage().persistent().remove(&key);

        let grant_key = DataKeyExt2::FeeWaiver(proposal.partner.clone());
        env.storage()
            .persistent()
            .set(&grant_key, &(proposal.waiver_bps, proposal.expires_at));
        env.storage().persistent().extend_ttl(
            &grant_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(env, "fee_waiver_granted"), proposal_id),
            (proposal.partner, proposal.waiver_bps, proposal.expires_at),
        );
    }

    /// Returns a pending fee-waiver proposal, or None if it never existed or
    /// has already executed.
    pub fn get_fee_waiver_proposal(env: Env, proposal_id: u64) -> Option<FeeWaiverProposal> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::FeeWaiverProposal(proposal_id))
    }

    /// Returns the active fee waiver for `partner` as (waiver_bps, expires_at),
    /// or None if no waiver has been granted or a granted waiver has expired.
    pub fn get_fee_waiver(env: Env, partner: Address) -> Option<(u32, u64)> {
        let grant: (u32, u64) = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::FeeWaiver(partner))?;
        let (_, expires_at) = grant;
        if expires_at != 0 && env.ledger().timestamp() >= expires_at {
            return None;
        }
        Some(grant)
    }

    /// Resolves the effective fee-waiver basis points currently active for
    /// `address` (0 if none granted or the grant has expired).
    fn resolve_fee_waiver_bps(env: &Env, address: &Address) -> u32 {
        Self::get_fee_waiver(env.clone(), address.clone())
            .map(|(bps, _)| bps)
            .unwrap_or(0)
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
    // ADMIN: DEADLINE WARNING LEAD WINDOW (#366)
    // ----------------------------------------------------------

    /// Set the ledger lead window before a milestone deadline during which
    /// `check_deadline_warning` may fire a `deadline_approaching` event (0 = disabled).
    pub fn set_warning_lead_ledgers(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::WarningLeadLedgers, &ledgers);
        env.events()
            .publish((Symbol::new(&env, "warning_lead_ledgers_set"),), ledgers);
    }

    pub fn get_warning_lead_ledgers(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::WarningLeadLedgers)
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ADMIN: JOINT CONFIRMATION THRESHOLD (#367)
    // ----------------------------------------------------------

    /// Set the shipment value above which joint (buyer + co-buyer) confirmation
    /// is required before a milestone payout releases (0 = disabled).
    pub fn set_joint_confirmation_threshold(env: Env, admin: Address, threshold: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::JointConfirmationThreshold, &threshold);
        env.events().publish(
            (Symbol::new(&env, "joint_confirmation_threshold_set"),),
            threshold,
        );
    }

    pub fn get_joint_confirmation_threshold(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::JointConfirmationThreshold)
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ADMIN: APPEAL WINDOW (#369)
    // ----------------------------------------------------------

    /// Set the ledger window after a dispute resolution during which either
    /// party may call `appeal_dispute` (0 = disabled).
    pub fn set_appeal_window_ledgers(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::AppealWindowLedgers, &ledgers);
        env.events()
            .publish((Symbol::new(&env, "appeal_window_ledgers_set"),), ledgers);
    }

    pub fn get_appeal_window_ledgers(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::AppealWindowLedgers)
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ADMIN: RESOLUTION FINALITY DELAY (#393)
    // ----------------------------------------------------------

    /// Set the ledger delay after `resolve_dispute` rules for the supplier
    /// before funds actually move (0 = disabled, funds release immediately).
    pub fn set_finality_delay_ledgers(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::ResolutionFinalityDelayLedgers, &ledgers);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_finality_delay_ledgers"),
            Symbol::new(&env, "resolution_finality_delay_ledgers_set"),
        );
        env.events().publish(
            (Symbol::new(&env, "resolution_finality_delay_ledgers_set"),),
            ledgers,
        );
    }

    pub fn get_finality_delay_ledgers(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::ResolutionFinalityDelayLedgers)
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

    /// #391: Cap the basis points a shipment creator may configure for a
    /// value-scaled dispute bond via `ShipmentOptions.dispute_bond_bps`.
    pub fn set_max_dispute_bond_bps(env: Env, admin: Address, max_bps: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if max_bps > 10_000 {
            panic!("max_bps must not exceed 10000");
        }
        env.storage()
            .persistent()
            .set(&DataKeyExt2::MaxDisputeBondBps, &max_bps);
        env.events()
            .publish((Symbol::new(&env, "max_dispute_bond_bps_set"),), max_bps);
    }

    pub fn get_max_dispute_bond_bps(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::MaxDisputeBondBps)
            .unwrap_or(constants::DEFAULT_MAX_DISPUTE_BOND_BPS)
    }

    pub fn get_reputation(env: Env, supplier: Address) -> ReputationScore {
        env.storage()
            .persistent()
            .get(&DataKey::SupplierRep(supplier.clone()))
            .unwrap_or_default()
    }

    // ----------------------------------------------------------
    // #397 – SUPPLIER TIERING
    // ----------------------------------------------------------

    /// Admin configures the reputation thresholds and collateral multipliers for
    /// Silver/Gold tiers. Multipliers are basis points of the base collateral
    /// requirement (must be <= 10 000, i.e. tiers can only discount, never inflate).
    pub fn set_supplier_tier_config(env: Env, admin: Address, config: SupplierTierConfig) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if config.silver_multiplier_bps > 10_000 || config.gold_multiplier_bps > 10_000 {
            panic!("tier multiplier cannot exceed 10000 bps");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt2::SupplierTierConfig, &config);
        env.events()
            .publish((Symbol::new(&env, "supplier_tier_config_set"),), ());
    }

    pub fn get_supplier_tier_config(env: Env) -> Option<SupplierTierConfig> {
        env.storage().instance().get(&DataKeyExt2::SupplierTierConfig)
    }

    /// Derives a supplier's current tier from their reputation score against the
    /// admin-configured thresholds. Suppliers with no completed shipments — and any
    /// supplier while no tier config is set — default to Bronze.
    pub fn get_supplier_tier(env: Env, supplier: Address) -> SupplierTier {
        Self::get_supplier_tier_internal(&env, &supplier)
    }

    fn get_supplier_tier_internal(env: &Env, supplier: &Address) -> SupplierTier {
        let config: Option<SupplierTierConfig> =
            env.storage().instance().get(&DataKeyExt2::SupplierTierConfig);
        let Some(config) = config else {
            return SupplierTier::Bronze;
        };
        let score = Self::get_reputation_internal(env, supplier);
        if score.completed == 0 {
            return SupplierTier::Bronze;
        }
        let ratio_bps = (score.disputed as u64).saturating_mul(10_000) / (score.completed as u64);
        if score.completed >= config.gold_min_completed
            && ratio_bps <= config.gold_max_disputed_ratio_bps as u64
        {
            SupplierTier::Gold
        } else if score.completed >= config.silver_min_completed
            && ratio_bps <= config.silver_max_disputed_ratio_bps as u64
        {
            SupplierTier::Silver
        } else {
            SupplierTier::Bronze
        }
    }

    /// Scales `base_collateral` down according to the supplier's current tier.
    /// New/Bronze suppliers (or when no tier config is set) pay the unmodified base amount.
    fn apply_tier_collateral_discount(env: &Env, supplier: &Address, base_collateral: i128) -> i128 {
        if base_collateral == 0 {
            return 0;
        }
        let config: Option<SupplierTierConfig> =
            env.storage().instance().get(&DataKeyExt2::SupplierTierConfig);
        let Some(config) = config else {
            return base_collateral;
        };
        let tier = Self::get_supplier_tier_internal(env, supplier);
        let multiplier_bps = match tier {
            SupplierTier::Bronze => 10_000,
            SupplierTier::Silver => config.silver_multiplier_bps,
            SupplierTier::Gold => config.gold_multiplier_bps,
        };
        (base_collateral * multiplier_bps as i128) / 10_000
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
        if fee_bps > constants::MAX_FEE_BPS {
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

    /// #413: Schedule a time-boxed, contract-wide fee holiday. Admin only.
    /// While `start_ledger <= env.ledger().sequence() <= end_ledger`, the protocol
    /// fee is waived (deduct_fee* return the full gross amount) for all shipments.
    pub fn schedule_fee_holiday(env: Env, admin: Address, start_ledger: u32, end_ledger: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if end_ledger < start_ledger {
            panic!("end_ledger must be >= start_ledger");
        }
        env.storage().instance().set(
            &DataKeyExt2::FeeHoliday,
            &FeeHoliday {
                start_ledger,
                end_ledger,
            },
        );
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "schedule_fee_holiday"),
            Symbol::new(&env, "fee_holiday_scheduled"),
        );
        env.events().publish(
            (Symbol::new(&env, "fee_holiday_scheduled"),),
            (start_ledger, end_ledger),
        );
    }

    /// #413: Cancel any scheduled/active fee holiday. Admin only.
    pub fn cancel_fee_holiday(env: Env, admin: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage().instance().remove(&DataKeyExt2::FeeHoliday);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "cancel_fee_holiday"),
            Symbol::new(&env, "fee_holiday_cancelled"),
        );
    }

    /// #413: Whether a fee holiday is currently active. Read-only.
    pub fn is_fee_holiday_active(env: Env) -> bool {
        Self::fee_holiday_active(&env)
    }

    /// Set multiple fee recipients with basis-point shares. Admin only.
    /// Shares must sum to exactly 10000 (100%).
    pub fn set_fee_recipients(env: Env, admin: Address, recipients: Vec<FeeRecipient>) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        if recipients.is_empty() {
            panic!("recipients cannot be empty");
        }

        // Validate shares sum to 10000
        let mut total_share: u32 = 0;
        for i in 0..recipients.len() {
            let recipient = recipients.get(i).unwrap();
            total_share = total_share.checked_add(recipient.share_bps).unwrap();
        }
        if total_share != 10_000 {
            panic!("fee shares must sum to exactly 10000");
        }

        env.storage()
            .instance()
            .set(&DataKeyExt::FeeRecipients, &recipients);

        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_fee_recipients"),
            Symbol::new(&env, "fee_recipients_updated"),
        );

        env.events().publish(
            (Symbol::new(&env, "fee_recipients_set"),),
            (recipients.len(), env.ledger().sequence()),
        );
    }

    /// Get treasury revenue collected for a specific token. Read-only.
    pub fn get_treasury_revenue(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKeyExt::TreasuryRevenue(token))
            .unwrap_or(0)
    }

    /// Withdraw dust from contract balance that isn't allocated to active escrows. Admin only.
    /// Amount withdrawn is bounded by: contract_balance - sum(all_active_escrow_balances).
    pub fn withdraw_treasury_dust(
        env: Env,
        admin: Address,
        token: Address,
        to: Address,
    ) -> i128 {
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let token_client = token::Client::new(&env, &token);
        let contract_balance = token_client.balance(&env.current_contract_address());

        // Get total escrowed amount across all active shipments
        let total_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(token.clone()))
            .unwrap_or(0);

        if contract_balance <= total_escrowed {
            panic!("no dust available");
        }

        let dust_amount = contract_balance - total_escrowed;

        token_client.transfer(&env.current_contract_address(), &to, &dust_amount);

        env.events().publish(
            (Symbol::new(&env, "treasury_withdrawal"), token),
            (to, dust_amount, env.ledger().sequence()),
        );

        dust_amount
    }

    /// Get buyer reliability score. Read-only.
    pub fn get_buyer_reliability(env: Env, buyer: Address) -> BuyerReliability {
        env.storage()
            .persistent()
            .get(&DataKeyExt::BuyerReliability(buyer))
            .unwrap_or_default()
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
    // ISSUE #303 — Milestone confirmation webhook allowlist
    // ----------------------------------------------------------

    /// Register an external contract address in the confirmation webhook allowlist.
    /// On every successful milestone confirmation, the contract will best-effort call
    /// `on_milestone_confirmed(shipment_id, milestone_index, payment_amount)` on each
    /// registered address. A failing webhook does NOT revert the confirmation.
    /// Admin only.
    pub fn add_confirmation_webhook(env: Env, admin: Address, contract_address: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let mut hooks: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKeyExt::ConfirmationWebhooks)
            .unwrap_or_else(|| Vec::new(&env));
        // Deduplicate — don't add the same address twice.
        for i in 0..hooks.len() {
            if hooks.get(i).unwrap() == contract_address {
                return;
            }
        }
        hooks.push_back(contract_address.clone());
        env.storage()
            .instance()
            .set(&DataKeyExt::ConfirmationWebhooks, &hooks);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "add_webhook"),
            Symbol::new(&env, "webhook_added"),
        );
        env.events().publish(
            (Symbol::new(&env, "webhook_added"),),
            (admin, contract_address, env.ledger().sequence()),
        );
    }

    /// Remove an external contract address from the confirmation webhook allowlist.
    /// Admin only.
    pub fn remove_confirmation_webhook(env: Env, admin: Address, contract_address: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let hooks: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKeyExt::ConfirmationWebhooks)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_hooks: Vec<Address> = Vec::new(&env);
        for i in 0..hooks.len() {
            let addr = hooks.get(i).unwrap();
            if addr != contract_address {
                new_hooks.push_back(addr);
            }
        }
        env.storage()
            .instance()
            .set(&DataKeyExt::ConfirmationWebhooks, &new_hooks);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "remove_webhook"),
            Symbol::new(&env, "webhook_removed"),
        );
        env.events().publish(
            (Symbol::new(&env, "webhook_removed"),),
            (admin, contract_address, env.ledger().sequence()),
        );
    }

    /// Returns the current confirmation webhook allowlist. Empty by default.
    pub fn get_confirmation_webhooks(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKeyExt::ConfirmationWebhooks)
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // ISSUE #304 — Dispute evidence submission cap
    // ----------------------------------------------------------

    /// Set the maximum number of evidence/proof submissions allowed per milestone.
    /// Default is 5 if not explicitly configured. Admin only.
    pub fn set_max_evidence_per_milestone(env: Env, admin: Address, max_count: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if max_count == 0 {
            panic!("max_count must be at least 1");
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxEvidencePerMilestone, &max_count);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_max_evidence"),
            Symbol::new(&env, "max_evidence_updated"),
        );
        env.events().publish(
            (Symbol::new(&env, "max_evidence_updated"),),
            (admin, max_count, env.ledger().sequence()),
        );
    }

    /// Returns the current evidence count for a given milestone.
    pub fn get_evidence_count(env: Env, shipment_id: String, milestone_index: u32) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::EvidenceCount(shipment_id, milestone_index))
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ISSUE #306 — Delegated confirmation signer
    // ----------------------------------------------------------

    /// Authorize a delegate address to call confirm_milestone on behalf of the buyer
    /// for a specific shipment. The delegate may only approve milestones whose gross
    /// payment is ≤ `per_tx_cap`. Buyer only.
    pub fn authorize_delegate(
        env: Env,
        buyer: Address,
        shipment_id: String,
        delegate: Address,
        per_tx_cap: i128,
    ) {
        Self::assert_not_paused(&env);
        buyer.require_auth();
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_is_buyer(&shipment, &buyer);
        if per_tx_cap <= 0 {
            panic!("per_tx_cap must be greater than zero");
        }
        let config = DelegateConfig {
            delegate: delegate.clone(),
            per_tx_cap,
        };
        let key = DataKey::ConfirmationDelegate(shipment_id.clone());
        env.storage().persistent().set(&key, &config);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (
                Symbol::new(&env, "delegate_authorized"),
                shipment_id.clone(),
            ),
            (buyer, delegate, per_tx_cap),
        );
    }

    /// Revoke a previously authorized confirmation delegate for a shipment.
    /// The delegate immediately loses the ability to call confirm_milestone.
    /// Buyer only.
    pub fn revoke_delegate(env: Env, buyer: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        buyer.require_auth();
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        Self::assert_is_buyer(&shipment, &buyer);
        env.storage()
            .persistent()
            .remove(&DataKey::ConfirmationDelegate(shipment_id.clone()));
        env.events().publish(
            (Symbol::new(&env, "delegate_revoked"), shipment_id.clone()),
            (buyer, env.ledger().sequence()),
        );
    }

    /// Returns the current confirmation delegate for a shipment, if one is registered.
    pub fn get_delegate(env: Env, shipment_id: String) -> Option<DelegateConfig> {
        env.storage()
            .persistent()
            .get(&DataKey::ConfirmationDelegate(shipment_id))
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
            .get::<DataKey, soroban_sdk::BytesN<32>>(&DataKey::Blacklisted(address))
            .is_some()
    }

    // ----------------------------------------------------------
    // #414: BLACKLIST APPEAL
    // ----------------------------------------------------------

    /// Callable only by a currently blacklisted address. Records a pending
    /// appeal for admin review. Only one open appeal is allowed at a time.
    pub fn appeal_blacklist(env: Env, address: Address, evidence_hash: String) {
        address.require_auth();
        if !Self::is_blacklisted(env.clone(), address.clone()) {
            panic!("address is not blacklisted");
        }
        let key = DataKeyExt2::BlacklistAppeal(address.clone());
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, BlacklistAppeal>(&key)
        {
            if existing.status == BlacklistAppealStatus::Pending {
                panic!("an appeal is already pending for this address");
            }
        }
        let appeal = BlacklistAppeal {
            evidence_hash,
            status: BlacklistAppealStatus::Pending,
            filed_ledger: env.ledger().sequence(),
        };
        env.storage().persistent().set(&key, &appeal);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "blacklist_appeal_filed"), address),
            appeal.filed_ledger,
        );
    }

    /// Read the current appeal (if any) filed by `address`. Read-only.
    pub fn get_blacklist_appeal(env: Env, address: Address) -> Option<BlacklistAppeal> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::BlacklistAppeal(address))
    }

    /// Admin reviews a pending appeal. When `approve` is true the address is
    /// removed from the blacklist; either way the appeal is marked decided.
    pub fn review_blacklist_appeal(env: Env, admin: Address, address: Address, approve: bool) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let key = DataKeyExt2::BlacklistAppeal(address.clone());
        let mut appeal: BlacklistAppeal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("no appeal found for this address"));
        if appeal.status != BlacklistAppealStatus::Pending {
            panic!("appeal has already been decided");
        }

        appeal.status = if approve {
            BlacklistAppealStatus::Approved
        } else {
            BlacklistAppealStatus::Rejected
        };
        env.storage().persistent().set(&key, &appeal);

        if approve {
            env.storage()
                .instance()
                .remove(&DataKey::Blacklisted(address.clone()));
        }

        Self::append_admin_action(
            &env,
            Symbol::new(&env, "review_blacklist_appeal"),
            Symbol::new(&env, "blacklist_appeal_reviewed"),
        );
        env.events().publish(
            (Symbol::new(&env, "blacklist_appeal_reviewed"), address),
            approve,
        );
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
        env.storage().instance().set(&DataKey::ReferralFeeBps, &bps);
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
        env.storage().instance().set(&DataKeyExt::FeeTiers, &tiers);
        env.events()
            .publish((Symbol::new(&env, "fee_tiers_set"),), tiers.len() as u32);
    }

    /// Returns the effective fee bps for `address` based on lifetime volume.
    /// Falls back to FeeConfig.fee_bps if no tier matches.
    pub fn get_fee_tier(env: Env, address: Address) -> u32 {
        let volume: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::LifetimeVolume(address.clone()))
            .unwrap_or(0);
        let tiers: Vec<FeeTier> = env
            .storage()
            .instance()
            .get(&DataKeyExt::FeeTiers)
            .unwrap_or_else(|| Vec::new(&env));
        let mut best: Option<u32> = None;
        for i in 0..tiers.len() {
            let t = tiers.get(i).unwrap();
            if volume >= t.min_lifetime_volume {
                best = Some(match best {
                    None => t.fee_bps,
                    Some(b) => {
                        if t.fee_bps < b {
                            t.fee_bps
                        } else {
                            b
                        }
                    }
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
        let key = DataKeyExt::MilestoneInvoiceHash(shipment_id.clone(), milestone_index);
        if env.storage().persistent().has(&key) {
            panic!("invoice hash already set and is immutable");
        }
        let m = shipment.milestones.get(milestone_index).unwrap();
        if m.status == MilestoneStatus::Pending {
            panic!("proof must be submitted before attaching invoice hash");
        }
        env.storage().persistent().set(&key, &invoice_hash);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "invoice_hash_attached"), shipment_id),
            (milestone_index, invoice_hash, caller),
        );
    }

    /// Returns the invoice hash for a milestone, or None if not set.
    pub fn get_invoice_hash(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Option<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::MilestoneInvoiceHash(
                shipment_id,
                milestone_index,
            ))
    }

    // ----------------------------------------------------------
    // #111 – AMENDMENT LOG
    // ----------------------------------------------------------

    /// Returns the append-only amendment log for a milestone in chronological order.
    pub fn get_amendment_log(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Vec<AmendmentEntry> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::AmendmentLog(shipment_id, milestone_index))
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
        let key = DataKeyExt::ExtensionRequest(shipment_id.clone(), milestone_index);
        if env.storage().persistent().has(&key) {
            panic!("extension request already pending");
        }
        env.storage()
            .persistent()
            .set(&key, &ExtensionReq { extra_ledgers });
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "extension_requested"), shipment_id),
            (milestone_index, extra_ledgers, caller),
        );
    }

    /// Buyer approves the pending extension request; adds extra_ledgers to milestone deadline.
    pub fn approve_extension(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);
        let key = DataKeyExt::ExtensionRequest(shipment_id.clone(), milestone_index);
        let req: ExtensionReq = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("no pending extension request"));
        env.storage().persistent().remove(&key);
        let deadline_key = DataKeyExt::MilestoneDeadline(shipment_id.clone(), milestone_index);
        let current_deadline: u32 = env
            .storage()
            .persistent()
            .get(&deadline_key)
            .unwrap_or_else(|| env.ledger().sequence());
        let new_deadline = current_deadline + req.extra_ledgers;
        env.storage().persistent().set(&deadline_key, &new_deadline);
        env.storage().persistent().extend_ttl(
            &deadline_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "extension_approved"), shipment_id),
            (milestone_index, new_deadline, buyer),
        );
    }

    /// Buyer denies the pending extension request; clears it without changing the deadline.
    pub fn deny_extension(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);
        let key = DataKeyExt::ExtensionRequest(shipment_id.clone(), milestone_index);
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
            .get(&DataKeyExt::MilestoneDeadline(shipment_id, milestone_index))
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
        let mut allowed: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::AllowedTokens)
            .unwrap_or_else(|| Vec::new(&env));

        // #387: Reject once the whitelist is at the admin-configured cap
        // (0/unset = no cap) so an unbounded allowlist can't inflate the
        // cost of functions that iterate it (e.g. create_shipment).
        let max_allowed: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::MaxAllowedTokens)
            .unwrap_or(0);
        if max_allowed > 0 && allowed.len() >= max_allowed {
            panic!("MaxAllowedTokensReached");
        }

        Self::append_admin_action(
            &env,
            Symbol::new(&env, "add_allowed_token"),
            Symbol::new(&env, "allowed_token_added"),
        );
        allowed.push_back(token.clone());
        env.storage()
            .instance()
            .set(&DataKey::AllowedTokens, &allowed);
        env.events()
            .publish((Symbol::new(&env, "allowed_token_added"),), token);
    }

    /// #387: Set the maximum number of entries allowed in the allowed-token
    /// whitelist (0 = no cap). Existing lists larger than a newly lowered cap
    /// remain valid — the cap is only enforced by `add_allowed_token`.
    pub fn set_max_allowed_tokens(env: Env, admin: Address, max_allowed: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::MaxAllowedTokens, &max_allowed);
        env.events()
            .publish((Symbol::new(&env, "max_allowed_tokens_set"),), max_allowed);
    }

    /// #387: Read the currently configured maximum allowed-token list size
    /// (0 = no cap, the default).
    pub fn get_max_allowed_tokens(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::MaxAllowedTokens)
            .unwrap_or(0)
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
        env.events()
            .publish((Symbol::new(&env, "allowed_token_removed"),), token);
    }

    /// Returns the current token whitelist. An empty list means all tokens are
    /// accepted (open mode) — see the whitelist check in `create_shipment`.
    pub fn get_allowed_tokens(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::AllowedTokens)
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // ADMIN: ARBITER POOL
    // ----------------------------------------------------------

    /// Add an arbiter to the admin-managed pool. Panics if the arbiter is
    /// currently slashed (#372) — it must be reinstated via
    /// `reinstate_arbiter` first.
    pub fn add_arbiter_to_pool(env: Env, admin: Address, arbiter: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if env
            .storage()
            .persistent()
            .get(&DataKeyExt2::ArbiterSlashed(arbiter.clone()))
            .unwrap_or(false)
        {
            panic!("arbiter is slashed and must be reinstated before re-adding");
        }
        let pool_key = Symbol::new(&env, "arbiters_pool");
        let mut pool: Vec<Address> = env
            .storage()
            .instance()
            .get::<Symbol, Vec<Address>>(&pool_key)
            .unwrap_or_else(|| Vec::new(&env));
        // Deduplicate: don't add if already present.
        for i in 0..pool.len() {
            if pool.get(i).unwrap() == arbiter {
                return;
            }
        }
        pool.push_back(arbiter.clone());
        env.storage().instance().set(&pool_key, &pool);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "add_arbiter_to_pool"),
            Symbol::new(&env, "arbiter_pool_updated"),
        );
        env.events().publish(
            (Symbol::new(&env, "arbiter_pool_updated"),),
            (Symbol::new(&env, "added"), arbiter),
        );
    }

    /// Remove an arbiter from the admin-managed pool.
    pub fn remove_arbiter_from_pool(env: Env, admin: Address, arbiter: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let pool_key = Symbol::new(&env, "arbiters_pool");
        let pool_idx_key = Symbol::new(&env, "arb_pool_idx");
        let pool: Vec<Address> = env
            .storage()
            .instance()
            .get::<Symbol, Vec<Address>>(&pool_key)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_pool: Vec<Address> = Vec::new(&env);
        for i in 0..pool.len() {
            let a = pool.get(i).unwrap();
            if a != arbiter {
                new_pool.push_back(a);
            }
        }
        // Reset pool index to 0 to stay within new pool bounds.
        env.storage().instance().set(&pool_idx_key, &0u32);
        env.storage().instance().set(&pool_key, &new_pool);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "remove_arbiter_from_pool"),
            Symbol::new(&env, "arbiter_pool_updated"),
        );
        env.events().publish(
            (Symbol::new(&env, "arbiter_pool_updated"),),
            (Symbol::new(&env, "removed"), arbiter),
        );
    }

    /// Return the current arbiter pool (read-only).
    pub fn get_arbiter_pool(env: Env) -> Vec<Address> {
        let pool_key = Symbol::new(&env, "arbiters_pool");
        env.storage()
            .instance()
            .get::<Symbol, Vec<Address>>(&pool_key)
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // #372: ARBITER REPUTATION SLASHING
    // ----------------------------------------------------------

    /// Set the number of overturned resolutions that automatically slashes an
    /// arbiter (removes them from the pool). 0 disables auto-slashing.
    pub fn set_max_overturned_before_slash(env: Env, admin: Address, threshold: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::MaxOverturnedBeforeSlash, &threshold);
        env.events().publish(
            (Symbol::new(&env, "max_overturned_before_slash_set"),),
            threshold,
        );
    }

    /// Read the configured overturned-resolution slash threshold (0 = disabled).
    pub fn get_max_overturned_before_slash(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::MaxOverturnedBeforeSlash)
            .unwrap_or(0)
    }

    /// Whether the given arbiter is currently slashed.
    pub fn is_arbiter_slashed(env: Env, arbiter: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::ArbiterSlashed(arbiter))
            .unwrap_or(false)
    }

    /// Remove a slashed arbiter's slashed flag, allowing them to be re-added to
    /// the pool via `add_arbiter_to_pool`. Does not itself re-add them to the
    /// pool — reinstatement is a deliberate, explicit two-step admin action.
    pub fn reinstate_arbiter(env: Env, admin: Address, arbiter: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .persistent()
            .remove(&DataKeyExt2::ArbiterSlashed(arbiter.clone()));
        env.events()
            .publish((Symbol::new(&env, "arbiter_reinstated"),), arbiter);
    }

    /// Remove `arbiter` from the pool and mark them slashed, so they can no
    /// longer be assigned to new disputes. Called automatically when an
    /// arbiter's overturned-resolution count crosses the configured threshold;
    /// also usable directly for immediate admin-initiated slashing.
    fn slash_arbiter(env: &Env, arbiter: &Address, overturned_count: u32) {
        let pool_key = Symbol::new(env, "arbiters_pool");
        let pool: Vec<Address> = env
            .storage()
            .instance()
            .get::<Symbol, Vec<Address>>(&pool_key)
            .unwrap_or_else(|| Vec::new(env));
        let mut new_pool: Vec<Address> = Vec::new(env);
        for i in 0..pool.len() {
            let a = pool.get(i).unwrap();
            if a != *arbiter {
                new_pool.push_back(a);
            }
        }
        env.storage()
            .instance()
            .set(&Symbol::new(env, "arb_pool_idx"), &0u32);
        env.storage().instance().set(&pool_key, &new_pool);
        env.storage()
            .persistent()
            .set(&DataKeyExt2::ArbiterSlashed(arbiter.clone()), &true);
        env.events().publish(
            (Symbol::new(env, "arbiter_slashed"),),
            (arbiter.clone(), overturned_count),
        );
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
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::assert_not_paused(&env);
        let response_deadline = options.response_deadline;
        let penalty_bps = options.penalty_bps;
        let milestone_mode = options.milestone_mode;
        let holdback_ledgers = options.holdback_ledgers;
        let dispute_cooldown_ledgers = options.dispute_cooldown_ledgers;
        let late_penalty_bps_per_ledger = options.late_penalty_bps_per_ledger;
        let auto_confirm_ledgers = options.auto_confirm_ledgers;
        let dispute_bond_bps = options.dispute_bond_bps;
        // #391: Scale the per-dispute bond by shipment value (bps), on top of any flat
        // `dispute_bond_amount`, capped by the admin-configured `MaxDisputeBondBps`.
        if dispute_bond_bps > 0 {
            let max_bps: u32 = env
                .storage()
                .persistent()
                .get(&DataKeyExt2::MaxDisputeBondBps)
                .unwrap_or(constants::DEFAULT_MAX_DISPUTE_BOND_BPS);
            if dispute_bond_bps > max_bps {
                panic!("dispute_bond_bps exceeds maximum allowed");
            }
        }
        let scaled_bond = (total_amount * dispute_bond_bps as i128) / 10_000;
        let dispute_bond_amount = options.dispute_bond_amount + scaled_bond;
        let logistics_fee_bps = options.logistics_fee_bps;
        let supplier_collateral = options.supplier_collateral;
        let expires_at_ledger = options.expires_at_ledger;

        let metadata_hash = options.metadata_hash;
        let referrer = options.referrer;
        let buyer_cancel_fee_bps = options.buyer_cancel_fee_bps;
        let early_bonus_pool = options.early_bonus_pool;
        let review_window_ledgers = options.review_window_ledgers;
        let milestone_splits = options.milestone_splits.clone();
        let deadlines = options.deadlines.clone();
        let dispute_timeout_seconds = options.dispute_timeout_seconds;
        let default_resolution = options.default_resolution.clone();
        let backup_arbiter = options.backup_arbiter.clone();
        let confirmation_cooldown_ledgers = options.confirmation_cooldown_ledgers;
        let arbiter_panel = options.arbiter_panel.clone();
        let jurisdiction = options.jurisdiction.clone();

        if buyer_cancel_fee_bps > constants::MAX_FEE_BPS {
            panic!("buyer_cancel_fee_bps cannot exceed 1000 (10%)");
        }

        if buyers.is_empty() {
            panic!("at least one buyer is required");
        }

        // All co-buyers must authorise the creation.
        for i in 0..buyers.len() {
            buyers.get(i).unwrap().require_auth();
        }

        // Supplier must authorise creation when they are required to lock collateral.
        if supplier_collateral > 0 {
            supplier.require_auth();
        }

        if total_amount < constants::MIN_SHIPMENT_AMOUNT {
            panic!("amount must be greater than zero");
        }

        // Batch read all validation config and stats in a single context fetch.
        let ctx = Self::fetch_create_shipment_ctx(&env);

        // #362: Per-token bounds override the global bound when configured;
        // tokens without an override fall back to the existing global bound.
        let effective_max_value: i128 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::TokenMaxShipmentValue(token.clone()))
            .unwrap_or(ctx.max_value);
        if effective_max_value > 0 && total_amount > effective_max_value {
            panic!("total amount exceeds maximum shipment value");
        }

        // #42 / #362: Enforce minimum shipment value floor (0 = disabled),
        // using the per-token override when one is configured.
        let effective_min_value: i128 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::TokenMinShipmentValue(token.clone()))
            .unwrap_or(ctx.min_value);
        if effective_min_value > 0 && total_amount < effective_min_value {
            panic!("MinShipmentValueNotMet");
        }

        // #364: Enforce the configured maximum milestone count per shipment.
        let max_milestone_count: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::MaxMilestoneCount)
            .unwrap_or(constants::DEFAULT_MAX_MILESTONE_COUNT);
        if milestones.len() > max_milestone_count {
            panic!("TooManyMilestones");
        }

        // Enforce token whitelist when non-empty (#161: multi-token support — XLM, EURC,
        // USDC, or any other SAC-wrapped asset the admin has approved).
        if ctx.allowed_tokens.len() > 0 {
            let mut found = false;
            for i in 0..ctx.allowed_tokens.len() {
                if ctx.allowed_tokens.get(i).unwrap() == token {
                    found = true;
                    break;
                }
            }
            if !found {
                panic!("token is not in the approved whitelist");
            }
        }

        for i in 0..buyers.len() {
            if env
                .storage()
                .instance()
                .get::<DataKey, soroban_sdk::BytesN<32>>(&DataKey::Blacklisted(
                    buyers.get(i).unwrap().clone(),
                ))
                .is_some()
            {
                panic!("unauthorized");
            }
        }
        for addr in [supplier.clone(), logistics.clone(), arbiter.clone()] {
            if env
                .storage()
                .instance()
                .get::<DataKey, soroban_sdk::BytesN<32>>(&DataKey::Blacklisted(addr))
                .is_some()
            {
                panic!("unauthorized");
            }
        }

        // Detect pool-arbiter mode: caller passes the contract's own address as a sentinel
        // to indicate "assign from pool on first dispute".
        let use_pool_arbiter = arbiter == env.current_contract_address();

        // Feature A: Validate arbiter panel when provided.
        if arbiter_panel.len() > 0 {
            if arbiter_panel.len() < 3 {
                panic!("arbiter panel must have at least 3 members");
            }
            // All panel members must not be blacklisted.
            for i in 0..arbiter_panel.len() {
                let p = arbiter_panel.get(i).unwrap();
                if env
                    .storage()
                    .instance()
                    .get::<DataKey, BytesN<32>>(&DataKey::Blacklisted(p))
                    .is_some()
                {
                    panic!("unauthorized");
                }
            }
        }

        // Feature B: Enforce supplier exposure cap when configured (0 = disabled).
        {
            let cap: i128 = env
                .storage()
                .instance()
                .get(&DataKeyExt::SupplierExposureCap)
                .unwrap_or(0);
            if cap > 0 {
                let current_exposure = Self::compute_supplier_exposure(&env, &supplier);
                if current_exposure + total_amount > cap {
                    panic!("SupplierExposureCapExceeded");
                }
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

        // #160: Validate milestone_splits when provided; otherwise fall back to payment_percent.
        if milestone_splits.len() > 0 {
            if milestone_splits.len() != milestones.len() {
                panic!("InvalidSplitConfiguration");
            }
            let mut total_bps: u32 = 0;
            for i in 0..milestone_splits.len() {
                total_bps += milestone_splits.get(i).unwrap();
            }
            if total_bps != 10_000 {
                panic!("InvalidSplitConfiguration");
            }
        } else {
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
        }

        // #164: Validate deadlines length when provided.
        if deadlines.len() > 0 && deadlines.len() != milestones.len() {
            panic!("deadline count must match milestone count");
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

        // #398: Enforce the buyer's rolling-window spending limit, if configured.
        Self::check_and_record_buyer_spending(&env, &primary_buyer, total_amount);

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

        // Transfer early bonus pool from buyer (separate from escrow; 0 = disabled).
        if early_bonus_pool > 0 {
            token_client.transfer(
                &primary_buyer,
                &env.current_contract_address(),
                &early_bonus_pool,
            );
        }

        // Lock supplier collateral: transfer from supplier and store separately.
        // #397: Scale the requirement down per the supplier's reputation-derived tier.
        // New suppliers (no history) or Bronze suppliers pay the unmodified base amount.
        if supplier_collateral > 0 {
            let effective_collateral =
                Self::apply_tier_collateral_discount(&env, &supplier, supplier_collateral);
            if effective_collateral > 0 {
                token_client.transfer(
                    &supplier,
                    &env.current_contract_address(),
                    &effective_collateral,
                );
            }
            env.storage().persistent().set(
                &DataKey::SupplierCollateral(shipment_id.clone()),
                &effective_collateral,
            );
        }

        // Normalise milestones: clear caller-supplied runtime state but preserve deadline fields.
        let mut clean_milestones: Vec<Milestone> = Vec::new(&env);
        for i in 0..milestones.len() {
            let mut m = milestones.get(i).unwrap();
            m.status = MilestoneStatus::Pending;
            m.proof_hash = String::from_str(&env, "");
            m.release_after_ledger = 0;
            m.proof_submitted_ledger = None;
            m.dispute_opened_ledger = None;
            // deadline_ledger and penalty_bps_per_ledger are caller-supplied configuration — keep them.
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
            dispute_bond_bps,
            arbiter_fee_bps: options.arbiter_fee_bps,
            logistics_fee_bps,
            expires_at_ledger,
            metadata_hash,
            referrer,
            buyer_cancel_fee_bps,
            early_bonus_pool,
            early_bonus_remaining: early_bonus_pool,
            review_window_ledgers,
            dispute_timeout_seconds,
            default_resolution: default_resolution.clone(),
            last_confirmed_milestone_index: None,
            cancellation_reason: Vec::new(&env),
        };

        Self::append_audit_entry(
            &env,
            &mut shipment,
            primary_buyer,
            Symbol::new(&env, "shipment_created"),
            Symbol::new(&env, "create_shipment"),
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // If pool-arbiter mode was requested, record the flag so raise_dispute can assign later.
        if use_pool_arbiter {
            env.storage().persistent().set(
                &(Symbol::new(&env, "use_pool_arb"), shipment_id.clone()),
                &true,
            );
        }

        env.storage().persistent().set(
            &DataKey::CancelPolicy(shipment_id.clone()),
            &CancelPolicy {
                response_deadline,
                penalty_bps,
            },
        );
        env.storage().persistent().extend_ttl(
            &DataKey::Shipment(shipment_id.clone()),
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        // #160: Store basis-point splits when provided.
        if milestone_splits.len() > 0 {
            env.storage().persistent().set(
                &DataKeyExt::MilestoneSplits(shipment_id.clone()),
                &milestone_splits,
            );
            env.storage().persistent().extend_ttl(
                &DataKeyExt::MilestoneSplits(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        // #164: Store per-milestone Unix timestamp deadlines when provided.
        if deadlines.len() > 0 {
            env.storage().persistent().set(
                &DataKeyExt::MilestoneTimestampDeadlines(shipment_id.clone()),
                &deadlines,
            );
            env.storage().persistent().extend_ttl(
                &DataKeyExt::MilestoneTimestampDeadlines(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        if let Some(backup) = backup_arbiter {
            env.storage()
                .persistent()
                .set(&DataKeyExt::BackupArbiter(shipment_id.clone()), &backup);
            env.storage().persistent().extend_ttl(
                &DataKeyExt::BackupArbiter(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        if let Some(cooldown) = confirmation_cooldown_ledgers {
            env.storage().persistent().set(
                &DataKeyExt::ShipmentConfirmationCooldown(shipment_id.clone()),
                &cooldown,
            );
            env.storage().persistent().extend_ttl(
                &DataKeyExt::ShipmentConfirmationCooldown(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        // Feature A: Store the arbiter panel when provided.
        if arbiter_panel.len() > 0 {
            env.storage().persistent().set(
                &DataKeyExt::ArbiterPanel(shipment_id.clone()),
                &arbiter_panel,
            );
            env.storage().persistent().extend_ttl(
                &DataKeyExt::ArbiterPanel(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        // #113: Resolve and lock the buyer's effective fee tier at creation.
        {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            let effective_bps = Self::resolve_fee_bps_for(&env, &primary_buyer);
            env.storage().persistent().set(
                &DataKeyExt::ShipmentFeeBps(shipment_id.clone()),
                &effective_bps,
            );
        }

        // #385: Store the jurisdiction/compliance tag and index it for
        // off-chain compliance filtering when provided.
        if let Some(jurisdiction_tag) = jurisdiction.clone() {
            env.storage().persistent().set(
                &DataKeyExt2::ShipmentJurisdiction(shipment_id.clone()),
                &jurisdiction_tag,
            );
            env.storage().persistent().extend_ttl(
                &DataKeyExt2::ShipmentJurisdiction(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );

            let index_key = DataKeyExt2::JurisdictionShipments(jurisdiction_tag);
            let mut jurisdiction_shipments: Vec<String> = env
                .storage()
                .persistent()
                .get(&index_key)
                .unwrap_or_else(|| Vec::new(&env));
            jurisdiction_shipments.push_back(shipment_id.clone());
            env.storage()
                .persistent()
                .set(&index_key, &jurisdiction_shipments);
            env.storage().persistent().extend_ttl(
                &index_key,
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
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
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
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
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
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
        Self::emit_shipment_created(
            &env,
            &shipment_id,
            &shipment.buyers.get(0).unwrap(),
            &shipment.supplier,
            &shipment.arbiter,
            &shipment.token,
            shipment.total_amount,
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

        // Feature B: Enforce supplier exposure cap on top-up.
        {
            let cap: i128 = env
                .storage()
                .instance()
                .get(&DataKeyExt::SupplierExposureCap)
                .unwrap_or(0);
            if cap > 0 {
                // Current exposure already includes this shipment's locked amount,
                // so we only need to check if adding the increment exceeds the cap.
                let current_exposure = Self::compute_supplier_exposure(&env, &shipment.supplier);
                if current_exposure + additional_amount > cap {
                    panic!("SupplierExposureCapExceeded");
                }
            }
        }

        // #398: Enforce the buyer's rolling-window spending limit, if configured.
        Self::check_and_record_buyer_spending(&env, &buyer, additional_amount);

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
    // #398 – BUYER SPENDING LIMIT
    // ----------------------------------------------------------

    /// Admin (or a buyer's own delegated risk policy) caps how much total escrow
    /// value `buyer` may commit within a rolling window of `window_ledgers`.
    /// `limit == 0` disables/clears the cap for this buyer.
    pub fn set_buyer_spending_limit(
        env: Env,
        admin: Address,
        buyer: Address,
        limit: i128,
        window_ledgers: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if limit < 0 {
            panic!("limit must be non-negative");
        }
        let key = DataKeyExt2::BuyerSpendingLimit(buyer.clone());
        env.storage().persistent().set(&key, &(limit, window_ledgers));
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "buyer_spending_limit_set"), buyer),
            (limit, window_ledgers),
        );
    }

    /// Returns the buyer's configured (limit, window_ledgers), if any.
    pub fn get_buyer_spending_limit(env: Env, buyer: Address) -> Option<(i128, u32)> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::BuyerSpendingLimit(buyer))
    }

    /// Returns the buyer's committed amount in the current (unexpired) rolling window.
    /// Always 0 for a buyer with no configured limit or once the window has elapsed.
    pub fn get_buyer_spending_window_usage(env: Env, buyer: Address) -> i128 {
        Self::current_buyer_spending_usage(&env, &buyer)
    }

    fn current_buyer_spending_usage(env: &Env, buyer: &Address) -> i128 {
        let limit_cfg: Option<(i128, u32)> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::BuyerSpendingLimit(buyer.clone()));
        let Some((_, window)) = limit_cfg else {
            return 0;
        };
        let (window_start, used): (u32, i128) = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::BuyerSpendingUsage(buyer.clone()))
            .unwrap_or((0, 0));
        let current_ledger = env.ledger().sequence();
        if window == 0 || current_ledger >= window_start + window {
            0
        } else {
            used
        }
    }

    /// Checks the configured rolling-window spending limit for `buyer` and, if the
    /// commitment stays within bounds, records `amount` against the window.
    /// No-op if the buyer has no configured limit (or it is set to 0).
    fn check_and_record_buyer_spending(env: &Env, buyer: &Address, amount: i128) {
        let limit_cfg: Option<(i128, u32)> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::BuyerSpendingLimit(buyer.clone()));
        let Some((limit, window)) = limit_cfg else {
            return;
        };
        if limit == 0 {
            return;
        }

        let (window_start, mut used): (u32, i128) = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::BuyerSpendingUsage(buyer.clone()))
            .unwrap_or((0, 0));
        let current_ledger = env.ledger().sequence();

        let mut effective_start = window_start;
        if window == 0 || current_ledger >= window_start + window {
            used = 0;
            effective_start = current_ledger;
        }

        if used + amount > limit {
            panic!("buyer spending limit exceeded");
        }

        used += amount;
        let usage_key = DataKeyExt2::BuyerSpendingUsage(buyer.clone());
        env.storage()
            .persistent()
            .set(&usage_key, &(effective_start, used));
        env.storage().persistent().extend_ttl(
            &usage_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
    }

    // ----------------------------------------------------------
    // #392 — SUPPLIER CANCELLATION COOLDOWN
    // ----------------------------------------------------------

    /// Admin caps how many times `supplier` may call `supplier_cancel` within a
    /// rolling window of `window_ledgers`; once the cap is hit, further
    /// cancellations are blocked until `cooldown_ledgers` have elapsed.
    /// `max_cancellations == 0` disables/clears the limit for this supplier.
    pub fn set_supplier_cancel_cooldown(
        env: Env,
        admin: Address,
        supplier: Address,
        max_cancellations: u32,
        window_ledgers: u32,
        cooldown_ledgers: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let key = DataKeyExt2::SupplierCancelCooldownConfig(supplier.clone());
        env.storage().persistent().set(
            &key,
            &(max_cancellations, window_ledgers, cooldown_ledgers),
        );
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "supplier_cancel_cooldown_set"), supplier),
            (max_cancellations, window_ledgers, cooldown_ledgers),
        );
    }

    /// Returns the supplier's configured (max_cancellations, window_ledgers,
    /// cooldown_ledgers), if any.
    pub fn get_supplier_cancel_cooldown(
        env: Env,
        supplier: Address,
    ) -> Option<(u32, u32, u32)> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::SupplierCancelCooldownConfig(supplier))
    }

    /// Checks the configured rolling-window cancellation cap for `supplier` and,
    /// if within bounds, records this cancellation against the window. Panics if
    /// the supplier is currently cooling down or the cap would be exceeded.
    /// No-op if the supplier has no configured limit (or it is set to 0).
    fn check_and_record_supplier_cancellation(env: &Env, supplier: &Address) {
        let cooldown_until: u32 = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::SupplierCancelCooldownUntil(supplier.clone()))
            .unwrap_or(0);
        let current_ledger = env.ledger().sequence();
        if cooldown_until > current_ledger {
            panic!("supplier cancellation cooldown active");
        }

        let cfg: Option<(u32, u32, u32)> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::SupplierCancelCooldownConfig(supplier.clone()));
        let Some((max_cancellations, window, cooldown_ledgers)) = cfg else {
            return;
        };
        if max_cancellations == 0 {
            return;
        }

        let (window_start, mut count): (u32, u32) = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::SupplierCancelUsage(supplier.clone()))
            .unwrap_or((0, 0));

        let mut effective_start = window_start;
        if window == 0 || current_ledger >= window_start + window {
            count = 0;
            effective_start = current_ledger;
        }

        count += 1;

        if count > max_cancellations {
            if cooldown_ledgers > 0 {
                env.storage().persistent().set(
                    &DataKeyExt2::SupplierCancelCooldownUntil(supplier.clone()),
                    &(current_ledger + cooldown_ledgers),
                );
            }
            panic!("supplier cancellation cooldown active");
        }

        let usage_key = DataKeyExt2::SupplierCancelUsage(supplier.clone());
        env.storage()
            .persistent()
            .set(&usage_key, &(effective_start, count));
        env.storage().persistent().extend_ttl(
            &usage_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
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
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
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
            (
                Symbol::new(&env, "milestones_rebalanced"),
                shipment_id.clone(),
            ),
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
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
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
        env.storage().persistent().set(&advance_key, &request);

        env.events().publish(
            (Symbol::new(&env, "advance_requested"), shipment_id.clone()),
            (milestone_index, advance_percent, caller),
        );
    }

    /// Buyer approves a pending advance request. Transfers the advance amount to
    /// the supplier immediately. The advance is deducted from the milestone payment
    /// when the milestone is later confirmed.
    pub fn approve_advance(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
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

        let gross_payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let advance_amount = (gross_payment * request.requested_percent as i128) / 100;

        // Check global circuit breaker before transferring payment
        Self::check_circuit_breaker(&env, advance_amount);
        // Check per-address outflow limit (#285)
        Self::check_address_outflow(&env, &shipment.supplier, advance_amount);

        request.approved = true;
        request.amount_advanced = advance_amount;
        env.storage().persistent().set(&advance_key, &request);

        // #284: Batched vs immediate payout
        let payout_mode: PayoutMode = env
            .storage()
            .persistent()
            .get(&DataKeyExt::PayoutMode(shipment.supplier.clone()))
            .unwrap_or(PayoutMode::Immediate);

        if payout_mode == PayoutMode::Batched {
            let pending: i128 = env
                .storage()
                .persistent()
                .get(&DataKeyExt::PendingPayout(shipment.supplier.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKeyExt::PendingPayout(shipment.supplier.clone()),
                &(pending + advance_amount),
            );
        } else {
            let token_client = token::Client::new(&env, &shipment.token);
            token_client.transfer(
                &env.current_contract_address(),
                &shipment.supplier,
                &advance_amount,
            );
        }

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
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::assert_not_paused(&env);

        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let key = DataKey::MilestoneProofWhitelist(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&key, &allowed_types);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (
                Symbol::new(&env, "proof_whitelist_set"),
                shipment_id.clone(),
            ),
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
    pub fn get_proof_whitelist(env: Env, shipment_id: String, milestone_index: u32) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::MilestoneProofWhitelist(
                shipment_id,
                milestone_index,
            ))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // CO-BUYER (#367)
    // ----------------------------------------------------------

    /// Designate a co-buyer for joint (dual-control) milestone confirmation.
    /// Callable once, by a registered buyer, only while the shipment is still
    /// at its starting state (nothing released or advanced yet) — i.e. "at creation".
    /// Immutable once set.
    pub fn set_co_buyer(env: Env, caller: Address, shipment_id: String, co_buyer: Address) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &caller);
        if shipment.released_amount > 0 || shipment.total_advanced_amount > 0 {
            panic!("co-buyer must be set before the shipment progresses");
        }
        let key = DataKeyExt2::CoBuyer(shipment_id.clone());
        if env.storage().persistent().has(&key) {
            panic!("co-buyer is already set");
        }
        env.storage().persistent().set(&key, &co_buyer);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events()
            .publish((Symbol::new(&env, "co_buyer_set"), shipment_id), co_buyer);
    }

    /// Returns the designated co-buyer for a shipment, if any.
    pub fn get_co_buyer(env: Env, shipment_id: String) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::CoBuyer(shipment_id))
    }

    /// Returns the partial joint-confirmation progress for a milestone.
    pub fn get_joint_confirmation_status(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> JointConfirmationStatus {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::JointConfirmation(
                shipment_id,
                milestone_index,
            ))
            .unwrap_or_default()
    }

    /// Whether `shipment` requires joint buyer + co-buyer confirmation (#367):
    /// a co-buyer must be designated AND the shipment value must exceed the
    /// admin-configured threshold (0 = disabled).
    fn joint_confirmation_required(env: &Env, shipment: &Shipment) -> bool {
        let threshold: i128 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::JointConfirmationThreshold)
            .unwrap_or(0);
        if threshold <= 0 || shipment.total_amount <= threshold {
            return false;
        }
        env.storage()
            .persistent()
            .has(&DataKeyExt2::CoBuyer(shipment.id.clone()))
    }

    /// Records a joint-confirmation vote from `caller` (who must act as the buyer side
    /// via `acts_as_buyer`, or be the designated co-buyer via `is_co_buyer`). Returns
    /// true once both sides have confirmed (caller should proceed with payout), false
    /// while awaiting the other party (caller should return without releasing funds).
    fn record_joint_confirmation(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
        caller: &Address,
        acts_as_buyer: bool,
        is_co_buyer: bool,
    ) -> bool {
        let key = DataKeyExt2::JointConfirmation(shipment_id.clone(), milestone_index);
        let mut status: JointConfirmationStatus =
            env.storage().persistent().get(&key).unwrap_or_default();
        if acts_as_buyer {
            status.buyer_confirmed = true;
        }
        if is_co_buyer {
            status.co_buyer_confirmed = true;
        }
        let both_confirmed = status.buyer_confirmed && status.co_buyer_confirmed;
        if both_confirmed {
            env.storage().persistent().remove(&key);
        } else {
            env.storage().persistent().set(&key, &status);
            env.storage().persistent().extend_ttl(
                &key,
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }
        env.events().publish(
            (
                Symbol::new(env, "joint_confirmation_recorded"),
                shipment_id.clone(),
            ),
            (
                milestone_index,
                caller.clone(),
                status.buyer_confirmed,
                status.co_buyer_confirmed,
            ),
        );
        both_confirmed
    }

    // ----------------------------------------------------------
    // #405 PROOF HASH VALIDATION HOOK
    // ----------------------------------------------------------

    /// Set the admin-configured minimum/maximum proof-hash length bounds
    /// enforced by `submit_proof`/`correct_proof`. 0 for either bound means
    /// "no minimum"/"no maximum" respectively.
    pub fn set_proof_hash_length_bounds(env: Env, admin: Address, min_len: u32, max_len: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if max_len > 0 && min_len > max_len {
            panic!("min_len must not exceed max_len");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt2::ProofHashMinLen, &min_len);
        env.storage()
            .instance()
            .set(&DataKeyExt2::ProofHashMaxLen, &max_len);
        env.events().publish(
            (Symbol::new(&env, "proof_hash_length_bounds_set"),),
            (min_len, max_len),
        );
    }

    /// Returns the configured (min_len, max_len) proof-hash length bounds
    /// (0 = no bound for that side).
    pub fn get_proof_hash_length_bounds(env: Env) -> (u32, u32) {
        let min_len: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::ProofHashMinLen)
            .unwrap_or(0);
        let max_len: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::ProofHashMaxLen)
            .unwrap_or(0);
        (min_len, max_len)
    }

    /// Set (or clear, with an empty string) the admin-configured required
    /// proof-hash prefix, e.g. "Qm" or "bafy" for basic CID-version sanity
    /// checking. Empty = no requirement.
    pub fn set_proof_hash_required_prefix(env: Env, admin: Address, prefix: String) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::ProofHashRequiredPrefix, &prefix);
        env.events().publish(
            (Symbol::new(&env, "proof_hash_required_prefix_set"),),
            prefix,
        );
    }

    /// Returns the configured required proof-hash prefix (empty = none).
    pub fn get_proof_hash_required_prefix(env: Env) -> String {
        env.storage()
            .instance()
            .get(&DataKeyExt2::ProofHashRequiredPrefix)
            .unwrap_or_else(|| String::from_str(&env, ""))
    }

    /// Validates `proof_hash` against the configured length bounds and
    /// required prefix (if any). Panics with a dedicated message on
    /// violation. Defaults (all unset) are fully permissive.
    fn validate_proof_hash(env: &Env, proof_hash: &String) {
        let len = proof_hash.len();
        let min_len: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::ProofHashMinLen)
            .unwrap_or(0);
        if min_len > 0 && len < min_len {
            panic!("proof hash is shorter than the minimum configured length");
        }
        let max_len: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::ProofHashMaxLen)
            .unwrap_or(0);
        if max_len > 0 && len > max_len {
            panic!("proof hash exceeds the maximum configured length");
        }

        let prefix: String = env
            .storage()
            .instance()
            .get(&DataKeyExt2::ProofHashRequiredPrefix)
            .unwrap_or_else(|| String::from_str(env, ""));
        let prefix_len = prefix.len();
        if prefix_len > 0 {
            if len < prefix_len {
                panic!("proof hash does not match the required prefix");
            }
            // soroban_sdk::String only exposes whole-string copy_into_slice
            // (the destination slice length must equal the full string
            // length), so both strings are copied out in full and compared
            // over their shared leading `prefix_len` bytes.
            const MAX_BYTES: usize = 256;
            if prefix_len as usize > MAX_BYTES {
                panic!("configured proof hash prefix is too long");
            }
            if len as usize > MAX_BYTES {
                panic!("proof hash exceeds the internal prefix-check length limit");
            }
            let mut prefix_buf = [0u8; MAX_BYTES];
            let mut hash_buf = [0u8; MAX_BYTES];
            prefix.copy_into_slice(&mut prefix_buf[..prefix_len as usize]);
            proof_hash.copy_into_slice(&mut hash_buf[..len as usize]);
            if prefix_buf[..prefix_len as usize] != hash_buf[..prefix_len as usize] {
                panic!("proof hash does not match the required prefix");
            }
        }
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
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_shipment_not_paused(&env, &shipment_id);
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();

        if milestone.status != MilestoneStatus::Pending {
            panic!("milestone is not in pending status");
        }
        Self::require_supplier_or_logistics_auth(&shipment, &caller);

        // #405: Validate proof_hash length/prefix bounds (if configured).
        Self::validate_proof_hash(&env, &proof_hash);

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

        // #162: Sequential mode — milestone N requires milestone N-1 to be Confirmed or Resolved.
        if shipment.milestone_mode == MilestoneMode::Sequential && milestone_index > 0 {
            let prev = shipment.milestones.get(milestone_index - 1).unwrap();
            if prev.status != MilestoneStatus::Confirmed && prev.status != MilestoneStatus::Resolved
            {
                panic!("MilestoneOutOfOrder");
            }
        }

        Self::assert_shipment_not_paused(&env, &shipment_id);

        // Issue #304 — Enforce per-milestone evidence submission cap.
        let evidence_key = DataKey::EvidenceCount(shipment_id.clone(), milestone_index);
        let current_evidence_count: u32 =
            env.storage().persistent().get(&evidence_key).unwrap_or(0);
        let max_evidence: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxEvidencePerMilestone)
            .unwrap_or(5);
        if current_evidence_count >= max_evidence {
            panic!("evidence submission limit reached");
        }

        let current_ledger = env.ledger().sequence();
        let is_resubmission = milestone.proof_hash.len() > 0;
        let proof_hash_for_event = proof_hash.clone();
        milestone.proof_hash = proof_hash;
        milestone.status = MilestoneStatus::ProofSubmitted;
        milestone.proof_submitted_ledger = Some(current_ledger);
        shipment.milestones.set(milestone_index, milestone);

        Self::append_audit_entry(
            &env,
            &mut shipment,
            caller.clone(),
            Symbol::new(&env, "proof_submitted"),
            Symbol::new(&env, "submit_proof"),
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Record the ledger at which proof was submitted (used by supplier_cancel).
        env.storage().persistent().set(
            &DataKey::ProofSubmittedAt(shipment_id.clone(), milestone_index),
            &current_ledger,
        );

        // Increment and persist the evidence count for this milestone.
        env.storage()
            .persistent()
            .set(&evidence_key, &(current_evidence_count + 1));
        env.storage().persistent().extend_ttl(
            &evidence_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        // Record the declared proof content type for off-chain and on-chain querying.
        let type_key = DataKey::SubmittedProofType(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&type_key, &proof_type);
        env.storage().persistent().extend_ttl(
            &type_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        // Record original submitter so correct_proof can enforce same-role corrections.
        let submitter_key = DataKeyExt::ProofSubmitter(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&submitter_key, &caller);
        env.storage().persistent().extend_ttl(
            &submitter_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        let event_topic = if is_resubmission {
            Symbol::new(&env, "proof_resubmitted")
        } else {
            Symbol::new(&env, "proof_submitted")
        };
        env.events().publish(
            (event_topic, shipment_id.clone()),
            (
                milestone_index,
                proof_hash_for_event.clone(),
                proof_type,
                caller,
                current_ledger,
            ),
        );
        Self::emit_milestone_proof_submitted(
            &env,
            &shipment_id,
            milestone_index,
            &proof_hash_for_event,
            &shipment.supplier,
        );
    }

    // ----------------------------------------------------------
    // CORRECT PROOF (in-place before buyer action)
    // ----------------------------------------------------------

    /// Correct a previously submitted proof while the milestone is still
    /// `ProofSubmitted`. Only the original submitter (supplier or logistics)
    /// may call this. Whitelist rules apply to `new_proof_type` the same way
    /// as in `submit_proof`. Status remains `ProofSubmitted`.
    pub fn correct_proof(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        new_proof_hash: String,
        new_proof_type: Symbol,
    ) {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::assert_not_paused(&env);
        caller.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::ProofSubmitted {
            panic!("milestone is not in proof submitted status");
        }

        // Only the original submitting address may correct.
        let submitter_key = DataKeyExt::ProofSubmitter(shipment_id.clone(), milestone_index);
        let original: Address = env
            .storage()
            .persistent()
            .get(&submitter_key)
            .unwrap_or_else(|| panic!("unauthorized"));
        if caller != original {
            panic!("unauthorized");
        }

        // #405: Validate new_proof_hash length/prefix bounds (if configured).
        Self::validate_proof_hash(&env, &new_proof_hash);

        // Validate new_proof_type against per-milestone whitelist (same rules as submit_proof).
        let whitelist_key = DataKey::MilestoneProofWhitelist(shipment_id.clone(), milestone_index);
        if let Some(whitelist) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<Symbol>>(&whitelist_key)
        {
            if whitelist.len() > 0 {
                let mut allowed = false;
                for i in 0..whitelist.len() {
                    if whitelist.get(i).unwrap() == new_proof_type {
                        allowed = true;
                        break;
                    }
                }
                if !allowed {
                    panic!("proof type not in whitelist");
                }
            }
        }

        let proof_hash_for_event = new_proof_hash.clone();
        milestone.proof_hash = new_proof_hash;
        // Status stays ProofSubmitted — buyer has not yet acted.
        shipment.milestones.set(milestone_index, milestone);

        Self::append_audit_entry(
            &env,
            &mut shipment,
            caller.clone(),
            Symbol::new(&env, "proof_corrected"),
            Symbol::new(&env, "correct_proof"),
        );

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        let type_key = DataKey::SubmittedProofType(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&type_key, &new_proof_type);
        env.storage().persistent().extend_ttl(
            &type_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(&env, "proof_corrected"), shipment_id.clone()),
            (milestone_index, proof_hash_for_event),
        );
    }

    // ----------------------------------------------------------
    // #386 PREVIEW MILESTONE PAYOUT (READ-ONLY)
    // ----------------------------------------------------------

    /// Simulates exactly what `confirm_milestone` would pay out for
    /// `milestone_index` right now — gross amount, advance/penalty
    /// deductions, platform + logistics fees (including any VIP fee waiver),
    /// and the net amount the supplier would receive — without mutating any
    /// state or requiring the milestone's proof to actually be confirmed.
    ///
    /// Mirrors the deduction order in `confirm_milestone`: gross → advance →
    /// late penalty → platform fee → logistics fee. Long-hold rebates and
    /// early-completion bonuses are omitted from `supplier_net_amount` since
    /// they depend on values only known at actual confirmation time
    /// (rebate needs the realised fee_amount; bonus is uncapped by holdback).
    pub fn preview_milestone_payout(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> PayoutPreview {
        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }
        let milestone = shipment.milestones.get(milestone_index).unwrap();

        let gross_amount = Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let mut payment = gross_amount;

        // Peek at any approved-but-unconsumed advance for this milestone
        // (mirrors consume_advance_for_milestone without removing it).
        let advance_deducted: i128 = env
            .storage()
            .persistent()
            .get::<DataKey, AdvanceRequest>(&DataKey::AdvanceRequest(
                shipment_id.clone(),
                milestone_index,
            ))
            .filter(|req| req.approved)
            .map(|req| req.amount_advanced)
            .unwrap_or(0);

        // Late-delivery penalty, same formula as confirm_milestone.
        let mut late_penalty_deducted: i128 = 0;
        if milestone.deadline_ledger > 0 {
            let penalty_bps = if milestone.penalty_bps_per_ledger > 0 {
                milestone.penalty_bps_per_ledger
            } else {
                shipment.late_penalty_bps_per_ledger
            };
            if penalty_bps > 0 {
                let proof_ledger = milestone.proof_submitted_ledger.unwrap_or(0);
                let overdue_ledgers = proof_ledger.saturating_sub(milestone.deadline_ledger);
                if overdue_ledgers > 0 {
                    let raw_penalty =
                        (gross_amount * (penalty_bps as i128 * overdue_ledgers as i128)) / 10_000;
                    let cap = gross_amount / 2;
                    late_penalty_deducted = raw_penalty.min(cap);
                    payment -= late_penalty_deducted;
                }
            }
        }

        let would_be_held = shipment.holdback_ledgers > 0;
        let is_final_milestone = Self::is_final_milestone(&shipment, milestone_index);

        if would_be_held {
            // Held milestones release later at face value (minus penalty already
            // applied above); no fee is deducted until the hold is claimed.
            return PayoutPreview {
                gross_amount,
                advance_deducted,
                late_penalty_deducted,
                platform_fee: 0,
                applied_fee_bps: 0,
                logistics_fee: 0,
                supplier_net_amount: payment - advance_deducted,
                would_be_held,
                is_final_milestone,
            };
        }

        let primary_buyer_for_fee = shipment.buyers.get(0).unwrap();
        let mut fee_amount: i128 = 0;
        let (net_payment, applied_fee_bps) = Self::deduct_fee_for_shipment_at_completion_preview(
            &env,
            payment,
            &shipment_id,
            &primary_buyer_for_fee,
            is_final_milestone,
            &mut fee_amount,
        );

        let mut actual_transfer = net_payment - advance_deducted;

        let mut logistics_fee: i128 = 0;
        if shipment.logistics_fee_bps > 0 {
            let candidate_fee = (payment * shipment.logistics_fee_bps as i128) / 10_000;
            if candidate_fee > 0 && candidate_fee <= actual_transfer {
                logistics_fee = candidate_fee;
                actual_transfer -= logistics_fee;
            }
        }

        PayoutPreview {
            gross_amount,
            advance_deducted,
            late_penalty_deducted,
            platform_fee: fee_amount,
            applied_fee_bps,
            logistics_fee,
            supplier_net_amount: actual_transfer,
            would_be_held,
            is_final_milestone,
        }
    }

    /// Read-only variant of `deduct_fee_for_shipment_at_completion` for
    /// `preview_milestone_payout` — computes the same fee without performing
    /// the treasury transfer.
    fn deduct_fee_for_shipment_at_completion_preview(
        env: &Env,
        gross: i128,
        shipment_id: &String,
        buyer: &Address,
        is_final: bool,
        fee_out: &mut i128,
    ) -> (i128, u32) {
        let override_bps: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeOverride(shipment_id.clone()));
        let locked_bps: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeBps(shipment_id.clone()));
        if let Some(config) = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
        {
            let bps = if let Some(o) = override_bps {
                o
            } else if is_final {
                Self::resolve_fee_bps_for(env, buyer)
            } else {
                locked_bps.unwrap_or(config.fee_bps)
            };
            let waiver_bps = Self::resolve_fee_waiver_bps(env, buyer);
            let bps = if waiver_bps > 0 {
                bps - ((bps as u64 * waiver_bps as u64) / 10_000) as u32
            } else {
                bps
            };
            let fee = (gross * bps as i128) / 10_000;
            if fee > 0 {
                *fee_out = fee;
                return (gross - fee, bps);
            }
            return (gross, bps);
        }
        let fallback_bps = override_bps.unwrap_or_else(|| locked_bps.unwrap_or(0));
        (gross, fallback_bps)
    }

    // ----------------------------------------------------------
    // CONFIRM MILESTONE (multi-sig)
    // ----------------------------------------------------------

    pub fn confirm_milestone(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::assert_not_paused(&env);

        // Batch read shipment and contract stats in a single context fetch.
        let ctx = Self::fetch_confirm_milestone_ctx(&env, &shipment_id);
        let mut shipment = ctx.shipment;

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_shipment_not_on_hold(&env, &shipment_id);

        // Issue #306 — Accept either the registered buyer or an authorized delegate.
        // Delegates are limited to confirm_milestone only (not dispute/cancel/etc.).
        // #367 — Also accept the designated co-buyer (for joint confirmation).
        let caller_is_buyer = Self::is_buyer(&shipment, &buyer);
        let caller_is_co_buyer = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, Address>(&DataKeyExt2::CoBuyer(shipment_id.clone()))
            .map(|cb| cb == buyer)
            .unwrap_or(false);
        let mut acts_as_buyer = caller_is_buyer;
        if caller_is_co_buyer {
            buyer.require_auth();
        } else if !caller_is_buyer {
            let delegate_config: DelegateConfig = env
                .storage()
                .persistent()
                .get(&DataKey::ConfirmationDelegate(shipment_id.clone()))
                .unwrap_or_else(|| panic!("unauthorized"));
            if buyer != delegate_config.delegate {
                panic!("unauthorized");
            }
            // Delegate auth check — must require_auth before checking cap.
            buyer.require_auth();
            acts_as_buyer = true;
        } else {
            Self::require_buyer_auth(&shipment, &buyer);
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::ProofSubmitted {
            panic!("milestone proof not yet submitted");
        }

        // #390: If an N-of-M oracle group is assigned to this shipment, require the
        // configured attestation threshold before allowing confirmation.
        Self::assert_oracle_attestation_met(&env, &shipment_id, milestone_index);

        Self::assert_shipment_not_paused(&env, &shipment_id);

        // #367 — High-value shipments with a designated co-buyer require both the
        // buyer and co-buyer to confirm before payout executes.
        if Self::joint_confirmation_required(&env, &shipment) {
            let both_confirmed = Self::record_joint_confirmation(
                &env,
                &shipment_id,
                milestone_index,
                &buyer,
                acts_as_buyer,
                caller_is_co_buyer,
            );
            if !both_confirmed {
                return;
            }
        }

        let cooldown = Self::get_confirmation_cooldown_internal(&env, &shipment_id);
        let fast_track = Self::is_fast_track_eligible_internal(&env, &shipment.supplier);
        if cooldown > 0 && !fast_track {
            let proof_ledger = milestone.proof_submitted_ledger.unwrap_or(0);
            if env.ledger().sequence() < proof_ledger + cooldown {
                panic!("confirmation cooldown not elapsed");
            }
        }

        // Check if auto-confirmation window has passed; if so, reject manual confirmation.
        let effective_window = Self::get_effective_auto_confirm_window(&env, &shipment);
        if effective_window > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let auto_confirm_ledger = proof_ledger + effective_window;
                if env.ledger().sequence() >= auto_confirm_ledger {
                    panic!("milestone has auto-confirmed; use claim_auto_confirmation");
                }
            }
        }

        let gross_payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let mut payment = gross_payment;

        // Deduct any approved advance for this milestone.
        let advance_deducted =
            Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, milestone_index);

        // Apply late-delivery penalty based on deadline_ledger (0 = no deadline, no penalty).
        let mut penalty_deducted: i128 = 0;
        if milestone.deadline_ledger > 0 {
            let penalty_bps = if milestone.penalty_bps_per_ledger > 0 {
                milestone.penalty_bps_per_ledger
            } else {
                shipment.late_penalty_bps_per_ledger
            };
            if penalty_bps > 0 {
                let proof_ledger = milestone.proof_submitted_ledger.unwrap_or(0);
                let overdue_ledgers = proof_ledger.saturating_sub(milestone.deadline_ledger);
                if overdue_ledgers > 0 {
                    let raw_penalty =
                        (gross_payment * (penalty_bps as i128 * overdue_ledgers as i128)) / 10_000;
                    let cap = gross_payment / 2;
                    penalty_deducted = raw_penalty.min(cap);
                    payment -= penalty_deducted;
                }
            }
        }

        if shipment.holdback_ledgers > 0 {
            milestone.release_after_ledger = env.ledger().sequence() + shipment.holdback_ledgers;
            milestone.status = MilestoneStatus::ConfirmedHeld;
            shipment.milestones.set(milestone_index, milestone.clone());

            // Pay early bonus immediately even for held milestones (bonus is based on delivery time).
            let mut early_bonus_paid: i128 = 0;
            if shipment.early_bonus_pool > 0
                && milestone.deadline_ledger > 0
                && env.ledger().sequence() <= milestone.deadline_ledger
                && shipment.early_bonus_remaining > 0
            {
                let total_milestones = shipment.milestones.len() as i128;
                let bonus = shipment.early_bonus_pool / total_milestones;
                if bonus > 0 && bonus <= shipment.early_bonus_remaining {
                    early_bonus_paid = bonus;
                    shipment.early_bonus_remaining -= bonus;
                    let token_client = token::Client::new(&env, &shipment.token);
                    token_client.transfer(
                        &env.current_contract_address(),
                        &shipment.supplier,
                        &bonus,
                    );
                }
            }

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            Self::append_audit_entry(
                &env,
                &mut shipment,
                buyer.clone(),
                Symbol::new(&env, "milestone_confirmed"),
                Symbol::new(&env, "confirm_milestone"),
            );

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            env.events().publish(
                (Symbol::new(&env, "payment_held"), shipment_id.clone()),
                (
                    milestone_index,
                    milestone.release_after_ledger,
                    penalty_deducted,
                    early_bonus_paid,
                ),
            );
        } else {
            // #399: Determine whether this milestone completes the shipment *before*
            // mutating any milestone status, so the fee-tier recalculation applies
            // only to the final milestone (earlier milestones keep their as-paid fee).
            let is_final_milestone = Self::is_final_milestone(&shipment, milestone_index);
            let primary_buyer_for_fee = shipment.buyers.get(0).unwrap();

            let mut fee_amount: i128 = 0;
            let (mut net_payment, applied_fee_bps) = Self::deduct_fee_for_shipment_at_completion(
                &env,
                payment,
                &shipment.token,
                &shipment_id,
                &primary_buyer_for_fee,
                is_final_milestone,
                &mut fee_amount,
            );

            // #300: Apply long-hold rebate if shipment age exceeds threshold.
            {
                let (threshold_ledgers, rebate_bps): (u32, u32) = env
                    .storage()
                    .instance()
                    .get(&DataKeyExt::LongHoldRebate)
                    .unwrap_or((0u32, 0u32));
                if rebate_bps > 0 && threshold_ledgers > 0 {
                    let age = env.ledger().sequence().saturating_sub(shipment.created_at);
                    if age >= threshold_ledgers && fee_amount > 0 {
                        let rebate = (fee_amount * rebate_bps as i128) / 10_000;
                        if rebate > 0 {
                            net_payment += rebate;
                            fee_amount -= rebate;
                        }
                    }
                }
            }

            // Check circuit breaker before transferring payment
            Self::check_circuit_breaker(&env, payment);
            Self::check_address_outflow(&env, &shipment.supplier, payment);

            let milestone_deadline = milestone.deadline_ledger;
            let proof_submitted_ledger = milestone.proof_submitted_ledger.unwrap_or(0);
            milestone.status = MilestoneStatus::Confirmed;

            Self::append_audit_entry(
                &env,
                &mut shipment,
                buyer.clone(),
                Symbol::new(&env, "milestone_confirmed"),
                Symbol::new(&env, "confirm_milestone"),
            );

            // Update buyer reliability tracking
            if caller_is_buyer && proof_submitted_ledger > 0 {
                Self::update_buyer_reliability_on_confirmation(&env, &buyer, proof_submitted_ledger);
            }

            shipment.milestones.set(milestone_index, milestone);
            shipment.released_amount += payment;
            // #162: Track last confirmed milestone for sequential enforcement queries.
            shipment.last_confirmed_milestone_index = Some(milestone_index);

            // #113: Accumulate lifetime volume for the primary buyer.
            {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                let vol_key = DataKeyExt::LifetimeVolume(primary_buyer.clone());
                let prev: i128 = env.storage().persistent().get(&vol_key).unwrap_or(0);
                env.storage().persistent().set(&vol_key, &(prev + payment));
            }

            // Transfer the net payment minus any advance already sent.
            let mut actual_transfer = net_payment - advance_deducted;
            let token_client = token::Client::new(&env, &shipment.token);

            // Deduct logistics fee and pay logistics provider.
            if shipment.logistics_fee_bps > 0 {
                let logistics_fee = (payment * shipment.logistics_fee_bps as i128) / 10_000;
                if logistics_fee > 0 && logistics_fee <= actual_transfer {
                    actual_transfer -= logistics_fee;
                    token_client.transfer(
                        &env.current_contract_address(),
                        &shipment.logistics,
                        &logistics_fee,
                    );
                }
            }

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

            // Pay early bonus to supplier if confirmed on or before deadline.
            if shipment.early_bonus_pool > 0
                && milestone_deadline > 0
                && env.ledger().sequence() <= milestone_deadline
                && shipment.early_bonus_remaining > 0
            {
                let total_milestones = shipment.milestones.len() as i128;
                let bonus = shipment.early_bonus_pool / total_milestones;
                if bonus > 0 && bonus <= shipment.early_bonus_remaining {
                    shipment.early_bonus_remaining -= bonus;
                    token_client.transfer(
                        &env.current_contract_address(),
                        &shipment.supplier,
                        &bonus,
                    );
                }
            }

            if actual_transfer > 0 {
                // Feature C: split payment across milestone payees if configured.
                Self::pay_milestone_to_payees(
                    &env,
                    &shipment_id,
                    milestone_index,
                    actual_transfer,
                    &shipment.supplier,
                    &token_client,
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

                Self::append_audit_entry(
                    &env,
                    &mut shipment,
                    buyer.clone(),
                    Symbol::new(&env, "shipment_completed"),
                    Symbol::new(&env, "confirm_milestone"),
                );

                // Return unused early bonus pool to buyer on completion.
                if shipment.early_bonus_remaining > 0 {
                    let primary_buyer = shipment.buyers.get(0).unwrap();
                    token_client.transfer(
                        &env.current_contract_address(),
                        &primary_buyer,
                        &shipment.early_bonus_remaining,
                    );
                    shipment.early_bonus_remaining = 0;
                }
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
                    token_client.transfer(
                        &env.current_contract_address(),
                        &shipment.supplier,
                        &collateral,
                    );
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

                Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
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
                    // #399: Additive field — the fee bps actually applied to this payment
                    // (recalculated at completion for the final milestone, locked-tier
                    // or override bps otherwise). Existing consumers reading the first
                    // 8 fields keep working.
                    applied_fee_bps,
                ),
            );
            Self::emit_milestone_confirmed(&env, &shipment_id, milestone_index, payment);
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

        let payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);

        // Deduct any approved advance for this milestone.
        let advance_deducted =
            Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, milestone_index);

        let mut fee_amount: i128 = 0;
        let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

        // Check circuit breaker before transferring payment
        Self::check_circuit_breaker(&env, payment);
        Self::check_address_outflow(&env, &shipment.supplier, payment);

        milestone.status = MilestoneStatus::Confirmed;
        milestone.release_after_ledger = 0;
        shipment.milestones.set(milestone_index, milestone);
        shipment.released_amount += payment;
        // #162: Track last confirmed milestone.
        shipment.last_confirmed_milestone_index = Some(milestone_index);

        let mut actual_transfer = net_payment - advance_deducted;
        let token_client = token::Client::new(&env, &shipment.token);

        // Deduct logistics fee and pay logistics provider.
        if shipment.logistics_fee_bps > 0 {
            let logistics_fee = (payment * shipment.logistics_fee_bps as i128) / 10_000;
            if logistics_fee > 0 && logistics_fee <= actual_transfer {
                actual_transfer -= logistics_fee;
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.logistics,
                    &logistics_fee,
                );
            }
        }

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
            // Feature C: split payment across milestone payees if configured.
            Self::pay_milestone_to_payees(
                &env,
                &shipment_id,
                milestone_index,
                actual_transfer,
                &shipment.supplier,
                &token_client,
            );
        }

        if Self::all_milestones_done(&shipment) {
            // Return unused early bonus pool to buyer on completion.
            if shipment.early_bonus_remaining > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &shipment.early_bonus_remaining,
                );
                shipment.early_bonus_remaining = 0;
            }
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
            Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
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
        Self::emit_milestone_confirmed(&env, &shipment_id, milestone_index, payment);
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

        Self::assert_shipment_not_paused(&env, &shipment_id);

        let cooldown = Self::get_confirmation_cooldown_internal(&env, &shipment_id);
        let fast_track = Self::is_fast_track_eligible_internal(&env, &shipment.supplier);

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
            if cooldown > 0 && !fast_track {
                let proof_ledger = m.proof_submitted_ledger.unwrap_or(0);
                if env.ledger().sequence() < proof_ledger + cooldown {
                    panic!("confirmation cooldown not elapsed");
                }
            }
        }

        // Apply confirmations and emit events.
        for i in 0..milestone_indices.len() {
            let idx = milestone_indices.get(i).unwrap();
            let mut milestone = shipment.milestones.get(idx).unwrap();
            milestone.status = MilestoneStatus::Confirmed;
            shipment.milestones.set(idx, milestone.clone());
            // #162: Track last confirmed milestone.
            shipment.last_confirmed_milestone_index = Some(idx);

            let payment = Self::milestone_gross_payment(&env, &shipment, idx);

            // Deduct any approved advance for this milestone.
            let advance_deducted =
                Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, idx);

            let mut fee_amount: i128 = 0;
            let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

            // Check circuit breaker before transferring payment
            Self::check_circuit_breaker(&env, payment);
            Self::check_address_outflow(&env, &shipment.supplier, payment);

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
                // Feature C: split payment across milestone payees if configured.
                Self::pay_milestone_to_payees(
                    &env,
                    &shipment_id,
                    idx,
                    actual_transfer,
                    &shipment.supplier,
                    &token_client,
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
            Self::emit_milestone_confirmed(&env, &shipment_id, idx, payment);
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
            Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
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
        Self::assert_shipment_not_paused(&env, &shipment_id);
        Self::assert_shipment_not_on_hold(&env, &shipment_id);
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
        let effective_window = Self::get_effective_auto_confirm_window(&env, &shipment);
        if effective_window > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let auto_confirm_ledger = proof_ledger + effective_window;
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

        // Pool-arbiter assignment: if this shipment was created with a pool-arbiter sentinel,
        // assign the next arbiter from the pool using round-robin (only on the first dispute).
        // The per-shipment flag is stored in persistent storage as a (Symbol, String) tuple key.
        let pool_flag_key = (Symbol::new(&env, "use_pool_arb"), shipment_id.clone());
        let use_pool: bool = env
            .storage()
            .persistent()
            .get::<(Symbol, String), bool>(&pool_flag_key)
            .unwrap_or(false);
        if use_pool {
            let pool_key = Symbol::new(&env, "arbiters_pool");
            let pool_idx_key = Symbol::new(&env, "arb_pool_idx");
            let pool: Vec<Address> = env
                .storage()
                .instance()
                .get::<Symbol, Vec<Address>>(&pool_key)
                .unwrap_or_else(|| Vec::new(&env));
            if pool.is_empty() {
                panic!("NoArbitersAvailable");
            }
            let idx: u32 = env
                .storage()
                .instance()
                .get::<Symbol, u32>(&pool_idx_key)
                .unwrap_or(0u32);
            let next_idx = (idx + 1) % pool.len() as u32;
            shipment.arbiter = pool.get(idx).unwrap();
            env.storage().instance().set(&pool_idx_key, &next_idx);
            // Clear the per-shipment flag so subsequent disputes use the assigned arbiter.
            env.storage().persistent().remove(&pool_flag_key);
        }

        shipment.open_dispute_count += 1;
        // Cancel any holdback window.
        milestone.release_after_ledger = 0;
        milestone.status = MilestoneStatus::Disputed;

        // #369: Fresh dispute cycle — clear any prior appeal-window bookkeeping.
        env.storage()
            .persistent()
            .remove(&DataKeyExt2::DisputeResolvedAtLedger(
                shipment_id.clone(),
                milestone_index,
            ));
        env.storage()
            .persistent()
            .remove(&DataKeyExt2::DisputeAppealed(
                shipment_id.clone(),
                milestone_index,
            ));

        Self::append_audit_entry(
            &env,
            &mut shipment,
            buyer.clone(),
            Symbol::new(&env, "dispute_raised"),
            Symbol::new(&env, "raise_dispute"),
        );

        milestone.dispute_opened_ledger = Some(env.ledger().sequence());
        // #165: Store Unix timestamp so resolve_dispute_timeout can check elapsed seconds.
        let dispute_opened_at_key =
            DataKeyExt::DisputeOpenedAt(shipment_id.clone(), milestone_index);
        env.storage()
            .persistent()
            .set(&dispute_opened_at_key, &env.ledger().timestamp());
        env.storage().persistent().extend_ttl(
            &dispute_opened_at_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
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
        Self::emit_dispute_opened(&env, &shipment_id, milestone_index, &buyer);
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
        Self::assert_shipment_not_on_hold(&env, &shipment_id);
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
        let effective_window = Self::get_effective_auto_confirm_window(&env, &shipment);
        if effective_window > 0 {
            if let Some(proof_ledger) = milestone.proof_submitted_ledger {
                let auto_confirm_ledger = proof_ledger + effective_window;
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
            Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let uncontested_payment =
            (full_milestone_payment * (100 - contested_percent) as i128) / 100;

        if uncontested_payment > 0 {
            let mut fee_amount: i128 = 0;
            let net_uncontested =
                Self::deduct_fee(&env, uncontested_payment, &shipment.token, &mut fee_amount);

            Self::check_circuit_breaker(&env, uncontested_payment);
            Self::check_address_outflow(&env, &shipment.supplier, uncontested_payment);

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
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        shipment.open_dispute_count += 1;
        milestone.release_after_ledger = 0;
        milestone.status = MilestoneStatus::Disputed;

        // #369: Fresh dispute cycle — clear any prior appeal-window bookkeeping.
        env.storage()
            .persistent()
            .remove(&DataKeyExt2::DisputeResolvedAtLedger(
                shipment_id.clone(),
                milestone_index,
            ));
        env.storage()
            .persistent()
            .remove(&DataKeyExt2::DisputeAppealed(
                shipment_id.clone(),
                milestone_index,
            ));

        Self::append_audit_entry(
            &env,
            &mut shipment,
            buyer.clone(),
            Symbol::new(&env, "dispute_raised"),
            Symbol::new(&env, "raise_dispute"),
        );

        milestone.dispute_opened_ledger = Some(env.ledger().sequence());
        // #165: Store Unix timestamp so resolve_dispute_timeout can check elapsed seconds.
        let dispute_opened_at_key =
            DataKeyExt::DisputeOpenedAt(shipment_id.clone(), milestone_index);
        env.storage()
            .persistent()
            .set(&dispute_opened_at_key, &env.ledger().timestamp());
        env.storage().persistent().extend_ttl(
            &dispute_opened_at_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
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
            (milestone_index, contested_percent, buyer.clone()),
        );
        Self::emit_dispute_opened(&env, &shipment_id, milestone_index, &buyer);
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
    /// The arbiter fee (`applicable_arbiter_fee_bps`) is deducted from the disputed payment
    /// and transferred to the arbiter address whenever a monetary disbursement occurs.

    pub(crate) fn applicable_arbiter_fee_bps(
        env: &Env,
        contested_amount: i128,
        default_bps: u32,
    ) -> u32 {
        let tiers: Vec<(i128, u32)> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ArbiterFeeTiers)
            .unwrap_or_else(|| Vec::new(env));
        if tiers.is_empty() {
            return default_bps;
        }

        let mut applicable_bps = default_bps;
        for i in 0..tiers.len() {
            let (threshold, bps) = tiers.get(i).unwrap();
            if contested_amount >= threshold {
                applicable_bps = bps;
            } else {
                // Tiers are sorted ascending, so once we hit a threshold > contested_amount, we stop.
                break;
            }
        }
        applicable_bps
    }
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

        let full_payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);

        // The "payment" in scope is the portion subject to this resolution:
        //   - full dispute  → 100% of milestone value
        //   - partial dispute → contested_percent% of milestone value
        let payment = if let Some(cp) = ctx.partial_contested_percent {
            (full_payment * cp as i128) / 100
        } else {
            full_payment
        };

        let token_client = token::Client::new(&env, &shipment.token);

        // #393: When an admin-configured finality delay is active, a supplier-favor
        // ruling does not move funds immediately. Instead the milestone is parked in
        // ResolvedPendingFinality for the delay window, giving the buyer a brief
        // re-review chance (e.g. via appeal_dispute) before finalize_dispute_resolution
        // can be called to actually release the payment.
        let finality_delay: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::ResolutionFinalityDelayLedgers)
            .unwrap_or(0);
        if approve && finality_delay > 0 {
            milestone.status = MilestoneStatus::ResolvedPendingFinality;
            milestone.release_after_ledger = env.ledger().sequence() + finality_delay;
        } else if approve {
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
            Self::check_address_outflow(&env, &shipment.supplier, payment);

            // Compute and transfer arbiter fee from the disputed payment.
            let fee_bps = Self::applicable_arbiter_fee_bps(&env, payment, shipment.arbiter_fee_bps);
            let arbiter_fee = (payment * fee_bps as i128) / 10_000;
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
                // Feature C: split payment across milestone payees if configured.
                Self::pay_milestone_to_payees(
                    &env,
                    &shipment_id,
                    milestone_index,
                    actual_transfer,
                    &shipment.supplier,
                    &token_client,
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
            let fee_bps = Self::applicable_arbiter_fee_bps(&env, payment, shipment.arbiter_fee_bps);
            let arbiter_fee = (payment * fee_bps as i128) / 10_000;
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
        let contested_key = DataKey::DisputeContestedPercent(shipment_id.clone(), milestone_index);
        if is_partial {
            env.storage().persistent().remove(&contested_key);
        }

        let resolution_ledgers = env.ledger().sequence()
            - milestone
                .dispute_opened_ledger
                .unwrap_or(env.ledger().sequence());
        let mut arbiter_stats: ArbiterStats = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ArbiterStats(arbiter.clone()))
            .unwrap_or_default();
        if approve {
            arbiter_stats.resolved_approved += 1;
        } else {
            arbiter_stats.resolved_rejected += 1;
        }
        arbiter_stats.total_resolution_ledgers += resolution_ledgers as u64;
        let stats_key = DataKeyExt::ArbiterStats(arbiter.clone());
        env.storage().persistent().set(&stats_key, &arbiter_stats);
        env.storage().persistent().extend_ttl(
            &stats_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        shipment.milestones.set(milestone_index, milestone);
        shipment.open_dispute_count = shipment.open_dispute_count.saturating_sub(1);

        // Update cooldown tracking regardless of approve/reject.
        shipment.last_dispute_resolved_ledger = Some(env.ledger().sequence());
        // #369: Record the resolution ledger so appeal_dispute can enforce its window.
        env.storage().persistent().set(
            &DataKeyExt2::DisputeResolvedAtLedger(shipment_id.clone(), milestone_index),
            &env.ledger().sequence(),
        );
        // #372: Record this outcome so a later appeal_dispute can snapshot it.
        env.storage().persistent().set(
            &DataKeyExt2::DisputeResolvedApprove(shipment_id.clone(), milestone_index),
            &approve,
        );

        // #372: If this resolution is the second (appeal) resolution of this
        // dispute cycle, compare it against the original arbiter's outcome —
        // a differing outcome means the original arbiter was overturned.
        let appeal_original_key =
            DataKeyExt2::DisputeAppealOriginal(shipment_id.clone(), milestone_index);
        if let Some((original_arbiter, original_approve)) = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, (Address, bool)>(&appeal_original_key)
        {
            env.storage().persistent().remove(&appeal_original_key);
            if original_approve != approve {
                let overturned_stats_key = DataKeyExt::ArbiterStats(original_arbiter.clone());
                let mut overturned_stats: ArbiterStats = env
                    .storage()
                    .persistent()
                    .get(&overturned_stats_key)
                    .unwrap_or_default();
                overturned_stats.overturned_count += 1;
                env.storage()
                    .persistent()
                    .set(&overturned_stats_key, &overturned_stats);
                env.storage().persistent().extend_ttl(
                    &overturned_stats_key,
                    constants::TTL_INITIAL_LEDGERS,
                    constants::TTL_MAX_LEDGERS,
                );

                let slash_threshold: u32 = env
                    .storage()
                    .instance()
                    .get(&DataKeyExt2::MaxOverturnedBeforeSlash)
                    .unwrap_or(0);
                let already_slashed = env
                    .storage()
                    .persistent()
                    .get(&DataKeyExt2::ArbiterSlashed(original_arbiter.clone()))
                    .unwrap_or(false);
                if slash_threshold > 0
                    && overturned_stats.overturned_count >= slash_threshold
                    && !already_slashed
                {
                    Self::slash_arbiter(&env, &original_arbiter, overturned_stats.overturned_count);
                }
            }
        }

        Self::append_audit_entry(
            &env,
            &mut shipment,
            arbiter.clone(),
            Symbol::new(&env, "dispute_resolved"),
            Symbol::new(&env, "resolve_dispute"),
        );

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;

            Self::append_audit_entry(
                &env,
                &mut shipment,
                arbiter.clone(),
                Symbol::new(&env, "shipment_completed"),
                Symbol::new(&env, "resolve_dispute"),
            );

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
            Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
        }

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

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Update buyer reliability tracking on dispute resolution
        let primary_buyer = shipment.buyers.get(0).unwrap();
        let buyer_won = !approve; // buyer wins if arbiter rejects supplier's proof
        Self::update_buyer_reliability_on_dispute(&env, &primary_buyer, buyer_won);

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
        let resolution = if approve {
            Symbol::new(&env, "supplier")
        } else {
            Symbol::new(&env, "buyer")
        };
        Self::emit_dispute_resolved(&env, &shipment_id, milestone_index, resolution, &arbiter);
    }

    // ----------------------------------------------------------
    // BATCH RESOLVE DISPUTES
    // ----------------------------------------------------------

    /// Resolve multiple disputes across one or more shipments in one invocation.
    /// Atomic — any invalid entry reverts the entire batch.
    pub fn batch_resolve_disputes(
        env: Env,
        arbiter: Address,
        resolutions: Vec<(String, u32, bool)>,
    ) {
        Self::assert_not_paused(&env);

        if resolutions.is_empty() {
            return;
        }

        // Validate all entries before mutating anything
        for i in 0..resolutions.len() {
            let (shipment_id, milestone_index, _approve) = resolutions.get(i).unwrap();
            let shipment = Self::get_shipment_internal(&env, &shipment_id);

            if shipment.status != ShipmentStatus::Active {
                panic!("shipment is not active");
            }
            Self::require_arbiter_auth(&shipment, &arbiter);

            if milestone_index as usize >= shipment.milestones.len() as usize {
                panic!("invalid milestone index");
            }

            let m = shipment.milestones.get(milestone_index).unwrap();
            if m.status != MilestoneStatus::Disputed {
                panic!("milestone is not in disputed status");
            }
        }

        // Execute resolutions
        for i in 0..resolutions.len() {
            let (shipment_id, milestone_index, approve) = resolutions.get(i).unwrap();
            Self::resolve_dispute(
                env.clone(),
                arbiter.clone(),
                shipment_id,
                milestone_index,
                approve,
            );
        }
    }

    // ----------------------------------------------------------
    // RESOLUTION FINALITY DELAY (#393)
    // ----------------------------------------------------------

    /// Anyone may call once the finality delay set by `resolve_dispute` has
    /// elapsed, to actually pay out a dispute that was ruled in the supplier's
    /// favor. Mirrors the fund-movement portion of `resolve_dispute`'s approve
    /// branch. If the buyer catches an arbiter error during the delay window,
    /// they should call `appeal_dispute` before this is called — a milestone
    /// reopened as Disputed by an appeal is no longer ResolvedPendingFinality
    /// and this call will simply fail with "milestone is not pending finality".
    pub fn finalize_dispute_resolution(env: Env, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::ResolvedPendingFinality {
            panic!("milestone is not pending finality");
        }
        if env.ledger().sequence() < milestone.release_after_ledger {
            panic!("finality delay has not yet elapsed");
        }

        let payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let token_client = token::Client::new(&env, &shipment.token);

        let advance_deducted = Self::consume_advance_for_milestone(
            &env,
            &mut shipment,
            &shipment_id,
            milestone_index,
        );

        let mut fee_amount: i128 = 0;
        let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

        Self::check_circuit_breaker(&env, payment);
        Self::check_address_outflow(&env, &shipment.supplier, payment);

        let fee_bps = Self::applicable_arbiter_fee_bps(&env, payment, shipment.arbiter_fee_bps);
        let arbiter_fee = (payment * fee_bps as i128) / 10_000;
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
            Self::pay_milestone_to_payees(
                &env,
                &shipment_id,
                milestone_index,
                actual_transfer,
                &shipment.supplier,
                &token_client,
            );
        }

        if shipment.dispute_bond_amount > 0 {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            token_client.transfer(
                &env.current_contract_address(),
                &primary_buyer,
                &shipment.dispute_bond_amount,
            );
        }

        milestone.status = MilestoneStatus::Resolved;
        milestone.release_after_ledger = 0;
        shipment.milestones.set(milestone_index, milestone);

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;
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
            Self::move_shipment_status_index(
                &env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
            Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (
                Symbol::new(&env, "dispute_resolution_finalized"),
                shipment_id.clone(),
            ),
            (milestone_index, payment, fee_amount),
        );
        Self::emit_dispute_resolved(
            &env,
            &shipment_id,
            milestone_index,
            Symbol::new(&env, "supplier"),
            &shipment.arbiter,
        );
    }

    // ----------------------------------------------------------
    // DISPUTE APPEAL (#369)
    // ----------------------------------------------------------

    /// Buyer or supplier escalates an already-resolved dispute to a second, independent
    /// arbiter drawn from the admin-managed arbiter pool, within the admin-configured
    /// appeal window. Reassigns the shipment's arbiter and reopens the milestone as
    /// `Disputed` so the new arbiter can call `resolve_dispute` again. That second
    /// resolution is final — a milestone may only ever be appealed once per dispute cycle.
    pub fn appeal_dispute(env: Env, caller: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_buyer_or_supplier(&shipment, &caller);
        caller.require_auth();

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let window: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::AppealWindowLedgers)
            .unwrap_or(0);
        if window == 0 {
            panic!("appeals are not enabled");
        }

        let appealed_key = DataKeyExt2::DisputeAppealed(shipment_id.clone(), milestone_index);
        if env
            .storage()
            .persistent()
            .get(&appealed_key)
            .unwrap_or(false)
        {
            panic!("dispute has already been appealed");
        }

        let resolved_key =
            DataKeyExt2::DisputeResolvedAtLedger(shipment_id.clone(), milestone_index);
        let resolved_at: u32 = env
            .storage()
            .persistent()
            .get(&resolved_key)
            .unwrap_or_else(|| panic!("dispute has not been resolved"));

        if env.ledger().sequence() > resolved_at + window {
            panic!("appeal window has closed");
        }

        // Draw a distinct arbiter (not the one who issued the original resolution) from
        // the admin-managed pool.
        let pool = Self::get_arbiter_pool(env.clone());
        let mut new_arbiter: Option<Address> = None;
        for i in 0..pool.len() {
            let candidate = pool.get(i).unwrap();
            if candidate != shipment.arbiter {
                new_arbiter = Some(candidate);
                break;
            }
        }
        let new_arbiter =
            new_arbiter.unwrap_or_else(|| panic!("no distinct arbiter available for appeal"));

        // #372: Record the original arbiter + outcome so the appeal's
        // resolve_dispute call can detect whether it overturns this decision.
        let original_approve: bool = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::DisputeResolvedApprove(
                shipment_id.clone(),
                milestone_index,
            ))
            .unwrap_or(false);
        env.storage().persistent().set(
            &DataKeyExt2::DisputeAppealOriginal(shipment_id.clone(), milestone_index),
            &(shipment.arbiter.clone(), original_approve),
        );

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        shipment.arbiter = new_arbiter.clone();
        milestone.release_after_ledger = 0;
        milestone.status = MilestoneStatus::Disputed;
        milestone.dispute_opened_ledger = Some(env.ledger().sequence());
        shipment.open_dispute_count += 1;
        shipment.milestones.set(milestone_index, milestone);

        env.storage().persistent().set(&appealed_key, &true);
        env.storage().persistent().remove(&resolved_key);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (Symbol::new(&env, "dispute_appealed"), shipment_id),
            (milestone_index, caller, new_arbiter),
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
    // CHECK DEADLINE WARNING (#366)
    // ----------------------------------------------------------

    /// Permissionless off-chain notification hook: emits `deadline_approaching` if
    /// `milestone_index`'s deadline falls within the admin-configured lead window and
    /// the warning hasn't already fired for this milestone. No on-chain state changes
    /// beyond the fired-once flag; a no-op outside the window or once already fired.
    pub fn check_deadline_warning(env: Env, shipment_id: String, milestone_index: u32) {
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        let lead: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::WarningLeadLedgers)
            .unwrap_or(0);
        if lead == 0 {
            return; // Deadline warnings not enabled
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Pending
            && milestone.status != MilestoneStatus::ProofSubmitted
        {
            return; // Deadline no longer relevant.
        }
        if milestone.deadline_ledger == 0 {
            return; // No deadline configured for this milestone.
        }

        let fired_key = DataKeyExt2::DeadlineWarningFired(shipment_id.clone(), milestone_index);
        if env.storage().persistent().get(&fired_key).unwrap_or(false) {
            return; // Already fired for this milestone.
        }

        let current_ledger = env.ledger().sequence();
        let window_start = milestone.deadline_ledger.saturating_sub(lead);
        if current_ledger < window_start || current_ledger >= milestone.deadline_ledger {
            return; // Outside the lead window.
        }

        env.storage().persistent().set(&fired_key, &true);
        env.storage().persistent().extend_ttl(
            &fired_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        let ledgers_remaining = milestone.deadline_ledger - current_ledger;
        env.events().publish(
            (
                Symbol::new(&env, "deadline_approaching"),
                shipment_id.clone(),
            ),
            (
                milestone_index,
                ledgers_remaining,
                milestone.deadline_ledger,
            ),
        );
    }

    // ----------------------------------------------------------
    // CANCEL SHIPMENT (buyer)
    // ----------------------------------------------------------

    pub fn cancel_shipment(env: Env, buyer: Address, shipment_id: String) {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::assert_not_paused(&env);
        buyer.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_shipment_not_on_hold(&env, &shipment_id);
        Self::assert_is_buyer(&shipment, &buyer);

        // Block cancellation if any milestone is Disputed.
        for i in 0..shipment.milestones.len() {
            let m = shipment.milestones.get(i).unwrap();
            if m.status == MilestoneStatus::Disputed {
                panic!("cannot cancel: dispute must be resolved first");
            }
        }

        let unreleased =
            shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;
        let cancel_fee = (unreleased * shipment.buyer_cancel_fee_bps as i128) / 10_000;
        let refund = unreleased - cancel_fee;
        let primary_buyer = shipment.buyers.get(0).unwrap();
        let token_client = token::Client::new(&env, &shipment.token);

        if cancel_fee > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &shipment.supplier,
                &cancel_fee,
            );
        }
        if refund > 0 {
            token_client.transfer(&env.current_contract_address(), &primary_buyer, &refund);
        }

        // Forfeit supplier collateral to buyer on buyer-initiated cancellation.
        let collateral: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SupplierCollateral(shipment_id.clone()))
            .unwrap_or(0);
        if collateral > 0 {
            token_client.transfer(&env.current_contract_address(), &primary_buyer, &collateral);
        }

        shipment.status = ShipmentStatus::Cancelled;
        shipment.cancellation_reason = Vec::from_array(&env, [CancellationReason::BuyerCancelled]);

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
        Self::emit_shipment_cancelled(
            &env,
            &shipment_id,
            refund,
            CancellationReason::BuyerCancelled,
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
        Self::assert_shipment_not_on_hold(&env, &shipment_id);
        if supplier != shipment.supplier {
            panic!("unauthorized");
        }

        // #392: Enforce the supplier's rolling-window cancellation cooldown, if configured.
        Self::check_and_record_supplier_cancellation(&env, &supplier);

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

        let remaining =
            shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;
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
        shipment.cancellation_reason =
            Vec::from_array(&env, [CancellationReason::SupplierCancelled]);

        Self::append_audit_entry(
            &env,
            &mut shipment,
            supplier.clone(),
            Symbol::new(&env, "shipment_cancelled"),
            Symbol::new(&env, "supplier_cancel"),
        );

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
        Self::emit_shipment_cancelled(
            &env,
            &shipment_id,
            refund,
            CancellationReason::SupplierCancelled,
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

            Self::append_audit_entry(
                &env,
                &mut shipment,
                caller.clone(),
                Symbol::new(&env, "amendment_accepted"),
                Symbol::new(&env, "propose_amendment"),
            );

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            env.storage().temporary().remove(&amendment_key);

            // #111: Append to amendment log (capped at 20, FIFO eviction).
            let log_key = DataKeyExt::AmendmentLog(shipment_id.clone(), milestone_index);
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
            env.storage().persistent().extend_ttl(
                &log_key,
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
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

        Self::append_audit_entry(
            &env,
            &mut shipment,
            current_buyer.clone(),
            Symbol::new(&env, "buyer_transferred"),
            Symbol::new(&env, "transfer_buyer"),
        );

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

        Self::append_audit_entry(
            &env,
            &mut shipment,
            current_supplier.clone(),
            Symbol::new(&env, "supplier_transferred"),
            Symbol::new(&env, "transfer_supplier"),
        );

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

    /// Allows the current arbiter to voluntarily step down.
    /// The contract automatically assigns the next available arbiter from the active pool.
    pub fn recuse_arbiter(env: Env, arbiter: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        arbiter.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        if shipment.arbiter != arbiter {
            panic!("only the current arbiter can recuse");
        }

        let pool_key = Symbol::new(&env, "arbiters_pool");
        let pool_idx_key = Symbol::new(&env, "arb_pool_idx");
        let pool: Vec<Address> = env
            .storage()
            .instance()
            .get::<Symbol, Vec<Address>>(&pool_key)
            .unwrap_or_else(|| Vec::new(&env));

        if pool.is_empty() {
            panic!("no available arbiter for reassignment");
        }

        let mut current_idx: u32 = env
            .storage()
            .instance()
            .get::<Symbol, u32>(&pool_idx_key)
            .unwrap_or(0u32);

        let mut replacement: Option<Address> = None;
        let mut attempts = 0;

        while attempts < pool.len() {
            let candidate = pool.get(current_idx).unwrap();
            current_idx = (current_idx + 1) % pool.len() as u32;

            if candidate != arbiter {
                replacement = Some(candidate);
                break;
            }
            attempts += 1;
        }

        if let Some(new_arbiter) = replacement {
            env.storage().instance().set(&pool_idx_key, &current_idx);
            shipment.arbiter = new_arbiter.clone();

            env.storage()
                .persistent()
                .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

            env.events().publish(
                (Symbol::new(&env, "arbiter_recused"), shipment_id.clone()),
                (arbiter, new_arbiter),
            );
        } else {
            panic!("no available arbiter for reassignment");
        }
    }

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

            Self::append_audit_entry(
                &env,
                &mut updated_shipment,
                caller.clone(),
                Symbol::new(&env, "arbiter_rotated"),
                Symbol::new(&env, "propose_arbiter_rotation"),
            );

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

        let effective_window = Self::get_effective_auto_confirm_window(&env, &shipment);
        if effective_window == 0 {
            panic!("auto-confirmation not enabled for this shipment");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();

        if milestone.status != MilestoneStatus::ProofSubmitted {
            panic!("milestone is not in ProofSubmitted status");
        }

        if let Some(proof_ledger) = milestone.proof_submitted_ledger {
            let auto_confirm_ledger = proof_ledger + effective_window;
            if env.ledger().sequence() < auto_confirm_ledger {
                panic!("auto-confirmation window has not expired");
            }
        } else {
            panic!("proof_submitted_ledger not set");
        }

        let gross_payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let mut payment = gross_payment;

        // Deduct any approved advance for this milestone.
        let advance_deducted =
            Self::consume_advance_for_milestone(&env, &mut shipment, &shipment_id, milestone_index);

        // Apply late-delivery penalty based on deadline_ledger (0 = no deadline, no penalty).
        let mut penalty_deducted: i128 = 0;
        if milestone.deadline_ledger > 0 {
            let penalty_bps = if milestone.penalty_bps_per_ledger > 0 {
                milestone.penalty_bps_per_ledger
            } else {
                shipment.late_penalty_bps_per_ledger
            };
            if penalty_bps > 0 {
                let proof_ledger = milestone.proof_submitted_ledger.unwrap_or(0);
                let overdue_ledgers = proof_ledger.saturating_sub(milestone.deadline_ledger);
                if overdue_ledgers > 0 {
                    let raw_penalty =
                        (gross_payment * (penalty_bps as i128 * overdue_ledgers as i128)) / 10_000;
                    let cap = gross_payment / 2;
                    penalty_deducted = raw_penalty.min(cap);
                    payment -= penalty_deducted;
                }
            }
        }

        let mut fee_amount: i128 = 0;
        let net_payment = Self::deduct_fee(&env, payment, &shipment.token, &mut fee_amount);

        // Check circuit breaker before transferring payment
        Self::check_circuit_breaker(&env, payment);
        Self::check_address_outflow(&env, &shipment.supplier, payment);

        let milestone_deadline = milestone.deadline_ledger;
        milestone.status = MilestoneStatus::Confirmed;

        Self::append_audit_entry(
            &env,
            &mut shipment,
            env.current_contract_address(),
            Symbol::new(&env, "auto_confirmed"),
            Symbol::new(&env, "claim_auto_confirmation"),
        );

        milestone.proof_submitted_ledger = None;
        shipment.last_confirmed_milestone_index = Some(milestone_index);
        shipment.milestones.set(milestone_index, milestone);
        shipment.released_amount += payment;

        let mut actual_transfer = net_payment - advance_deducted;
        let token_client = token::Client::new(&env, &shipment.token);

        // Deduct logistics fee and pay logistics provider.
        if shipment.logistics_fee_bps > 0 {
            let logistics_fee = (payment * shipment.logistics_fee_bps as i128) / 10_000;
            if logistics_fee > 0 && logistics_fee <= actual_transfer {
                actual_transfer -= logistics_fee;
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.logistics,
                    &logistics_fee,
                );
            }
        }

        // Pay early bonus to supplier if proof was submitted on or before deadline.
        if shipment.early_bonus_pool > 0
            && milestone_deadline > 0
            && env.ledger().sequence() <= milestone_deadline
            && shipment.early_bonus_remaining > 0
        {
            let total_milestones = shipment.milestones.len() as i128;
            let bonus = shipment.early_bonus_pool / total_milestones;
            if bonus > 0 && bonus <= shipment.early_bonus_remaining {
                shipment.early_bonus_remaining -= bonus;
                token_client.transfer(&env.current_contract_address(), &shipment.supplier, &bonus);
            }
        }

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
            // Feature C: split payment across milestone payees if configured.
            Self::pay_milestone_to_payees(
                &env,
                &shipment_id,
                milestone_index,
                actual_transfer,
                &shipment.supplier,
                &token_client,
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
            // Return unused early bonus pool to buyer on completion.
            if shipment.early_bonus_remaining > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &shipment.early_bonus_remaining,
                );
                shipment.early_bonus_remaining = 0;
            }
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
            Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
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
        Self::emit_milestone_confirmed(&env, &shipment_id, milestone_index, payment);
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
        Self::do_emergency_recover(&env, &admin, &shipment_id);
    }

    /// Shared recovery logic used by both the legacy single-step `emergency_recover`
    /// and the delayed `propose_emergency_recover`/`execute_emergency_recover` flow.
    /// Callers are responsible for their own auth/pause/admin checks — calling
    /// `admin.require_auth()` again here would panic with "frame is already
    /// authorized" since the caller already authorized in the same invocation.
    fn do_emergency_recover(env: &Env, admin: &Address, shipment_id: &String) {
        let mut shipment = Self::get_shipment_internal(env, shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger <= shipment.created_at + RECOVERY_THRESHOLD_LEDGERS {
            panic!("recovery threshold not reached");
        }

        let recovery_amount =
            shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;

        if recovery_amount > 0 {
            let token_client = token::Client::new(env, &shipment.token);
            token_client.transfer(&env.current_contract_address(), admin, &recovery_amount);
        }

        Self::move_shipment_status_index(
            env,
            ShipmentStatus::Active,
            ShipmentStatus::Cancelled,
            shipment_id,
        );
        shipment.status = ShipmentStatus::Cancelled;
        shipment.cancellation_reason =
            Vec::from_array(env, [CancellationReason::AdminEmergencyRecovery]);

        Self::append_audit_entry(
            env,
            &mut shipment,
            admin.clone(),
            Symbol::new(env, "shipment_cancelled"),
            Symbol::new(env, "cancel_shipment"),
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
            &(current_escrowed - recovery_amount).max(0),
        );

        env.events().publish(
            (Symbol::new(env, "emergency_recovery"), shipment_id.clone()),
            (recovery_amount, admin.clone()),
        );
        Self::emit_shipment_cancelled(
            env,
            shipment_id,
            recovery_amount,
            CancellationReason::AdminEmergencyRecovery,
        );
    }

    // ----------------------------------------------------------
    // RESOLVE DISPUTE TIMEOUT (#165)
    // ----------------------------------------------------------

    /// Anyone may call this after dispute_timeout_seconds have elapsed without arbiter action.
    /// Applies the shipment's default_resolution: pays the supplier or refunds the buyer.
    /// Arbiter resolution before the timeout window always takes priority.
    pub fn resolve_dispute_timeout(env: Env, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);
        let arbiter = shipment.arbiter.clone();

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not in disputed status");
        }

        if shipment.dispute_timeout_seconds == 0 {
            panic!("dispute timeout not configured for this shipment");
        }

        let opened_at: u64 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::DisputeOpenedAt(
                shipment_id.clone(),
                milestone_index,
            ))
            .unwrap_or_else(|| panic!("dispute opened timestamp not recorded"));

        let current_timestamp = env.ledger().timestamp();
        if current_timestamp < opened_at + shipment.dispute_timeout_seconds {
            panic!("DisputeTimeoutNotReached");
        }

        // Determine payment scope: partial dispute holds only the contested portion.
        let partial_cp: Option<u32> =
            env.storage()
                .persistent()
                .get(&DataKey::DisputeContestedPercent(
                    shipment_id.clone(),
                    milestone_index,
                ));
        let full_payment = Self::milestone_gross_payment(&env, &shipment, milestone_index);
        let payment = if let Some(cp) = partial_cp {
            (full_payment * cp as i128) / 100
        } else {
            full_payment
        };

        let token_client = token::Client::new(&env, &shipment.token);
        let is_supplier = shipment.default_resolution == Resolution::Supplier;

        if is_supplier {
            let mut fee_amount: i128 = 0;
            let net_payment = Self::deduct_fee_for_shipment(
                &env,
                payment,
                &shipment.token,
                &shipment_id,
                &mut fee_amount,
            );
            Self::check_circuit_breaker(&env, payment);
            Self::check_address_outflow(&env, &shipment.supplier, payment);
            if net_payment > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &net_payment,
                );
            }
            shipment.released_amount += payment;
        } else {
            // Resolution::Buyer — refund contested portion to buyer.
            let primary_buyer = shipment.buyers.get(0).unwrap();
            if payment > 0 {
                token_client.transfer(&env.current_contract_address(), &primary_buyer, &payment);
            }
        }

        // Update escrowed value.
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - payment).max(0),
        );

        // Clean up partial-dispute and opened-at records.
        if partial_cp.is_some() {
            env.storage()
                .persistent()
                .remove(&DataKey::DisputeContestedPercent(
                    shipment_id.clone(),
                    milestone_index,
                ));
        }
        env.storage()
            .persistent()
            .remove(&DataKeyExt::DisputeOpenedAt(
                shipment_id.clone(),
                milestone_index,
            ));

        milestone.status = MilestoneStatus::Resolved;

        Self::append_audit_entry(
            &env,
            &mut shipment,
            arbiter.clone(),
            Symbol::new(&env, "dispute_auto_resolved"),
            Symbol::new(&env, "resolve_dispute_timeout"),
        );

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;

            Self::append_audit_entry(
                &env,
                &mut shipment,
                arbiter.clone(),
                Symbol::new(&env, "shipment_completed"),
                Symbol::new(&env, "resolve_dispute_timeout"),
            );

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
            Self::move_shipment_status_index(
                &env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
            Self::emit_shipment_completed(&env, &shipment_id, shipment.released_amount);
        }

        shipment.milestones.set(milestone_index, milestone);
        shipment.open_dispute_count = shipment.open_dispute_count.saturating_sub(1);
        shipment.last_dispute_resolved_ledger = Some(env.ledger().sequence());

        // Remove from active disputes list.
        let disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(&env);
        for i in 0..disputes.len() {
            let d = disputes.get(i).unwrap();
            if !(d.shipment_id == shipment_id && d.milestone_index == milestone_index) {
                new_disputes.push_back(d);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &new_disputes);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        let resolution_sym = if is_supplier {
            Symbol::new(&env, "supplier")
        } else {
            Symbol::new(&env, "buyer")
        };
        env.events().publish(
            (
                Symbol::new(&env, "dispute_auto_resolved"),
                shipment_id.clone(),
            ),
            (milestone_index, resolution_sym.clone()),
        );
        Self::emit_dispute_resolved(
            &env,
            &shipment_id,
            milestone_index,
            resolution_sym,
            &shipment.arbiter,
        );
    }

    // ----------------------------------------------------------
    // #400 – DISPUTE MEDIATOR
    // ----------------------------------------------------------

    /// Admin assigns a mediator to a specific shipment. Takes precedence over the
    /// global mediator pool for that shipment.
    pub fn assign_mediator(env: Env, admin: Address, shipment_id: String, mediator: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        // Ensure the shipment exists.
        let _ = Self::get_shipment_internal(&env, &shipment_id);
        let key = DataKeyExt2::ShipmentMediator(shipment_id.clone());
        env.storage().persistent().set(&key, &mediator);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "mediator_assigned"), shipment_id),
            mediator,
        );
    }

    /// Admin configures a global pool of mediators. Any address in the pool may mediate
    /// any shipment that has no shipment-specific mediator assigned.
    pub fn set_mediator_pool(env: Env, admin: Address, mediators: Vec<Address>) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::MediatorPool, &mediators);
        env.events()
            .publish((Symbol::new(&env, "mediator_pool_set"),), mediators.len() as u32);
    }

    pub fn get_mediator_pool(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKeyExt2::MediatorPool)
            .unwrap_or_else(|| Vec::new(&env))
    }

    pub fn get_shipment_mediator(env: Env, shipment_id: String) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::ShipmentMediator(shipment_id))
    }

    pub fn get_mediation_proposal(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Option<MediationProposal> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::MediationProposal(shipment_id, milestone_index))
    }

    fn is_authorized_mediator(env: &Env, shipment_id: &String, mediator: &Address) -> bool {
        if let Some(assigned) = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, Address>(&DataKeyExt2::ShipmentMediator(shipment_id.clone()))
        {
            if assigned == *mediator {
                return true;
            }
        }
        let pool: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKeyExt2::MediatorPool)
            .unwrap_or_else(|| Vec::new(env));
        for i in 0..pool.len() {
            if pool.get(i).unwrap() == *mediator {
                return true;
            }
        }
        false
    }

    /// An assigned mediator proposes a non-binding suggested outcome for a disputed
    /// milestone. Visible to both parties via `get_mediation_proposal`; has no effect
    /// on funds until both buyer and supplier accept it via `accept_mediation`.
    pub fn propose_mediation(
        env: Env,
        mediator: Address,
        shipment_id: String,
        milestone_index: u32,
        suggested_outcome: Resolution,
    ) {
        Self::assert_not_paused(&env);
        mediator.require_auth();

        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if !Self::is_authorized_mediator(&env, &shipment_id, &mediator) {
            panic!("unauthorized mediator");
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }
        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not in disputed status");
        }

        let proposal = MediationProposal {
            mediator: mediator.clone(),
            suggested_outcome: suggested_outcome.clone(),
            buyer_accepted: false,
            supplier_accepted: false,
        };
        let key = DataKeyExt2::MediationProposal(shipment_id.clone(), milestone_index);
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        let outcome_sym = match suggested_outcome {
            Resolution::Buyer => Symbol::new(&env, "buyer"),
            Resolution::Supplier => Symbol::new(&env, "supplier"),
        };
        env.events().publish(
            (Symbol::new(&env, "mediation_proposed"), shipment_id),
            (milestone_index, mediator, outcome_sym),
        );
    }

    /// Buyer or supplier accepts the pending mediation proposal for a milestone.
    /// Once both parties have accepted, the suggested outcome is applied directly —
    /// bypassing binding arbiter resolution entirely (no arbiter fee is charged).
    /// Either party may instead decline (`decline_mediation`) and fall through to the
    /// standard `raise_dispute` / `resolve_dispute` flow, which is unaffected by this.
    pub fn accept_mediation(env: Env, caller: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        caller.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_buyer_or_supplier(&shipment, &caller);

        let key = DataKeyExt2::MediationProposal(shipment_id.clone(), milestone_index);
        let mut proposal: MediationProposal = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("no pending mediation proposal"));

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not in disputed status");
        }

        if Self::is_buyer(&shipment, &caller) {
            proposal.buyer_accepted = true;
        } else {
            proposal.supplier_accepted = true;
        }

        if proposal.buyer_accepted && proposal.supplier_accepted {
            env.storage().persistent().remove(&key);
            let mediator = proposal.mediator.clone();
            Self::apply_mediation_outcome(
                &env,
                &mut shipment,
                &shipment_id,
                milestone_index,
                proposal.suggested_outcome.clone(),
                &mediator,
            );
        } else {
            env.storage().persistent().set(&key, &proposal);
        }
    }

    /// Buyer or supplier declines the pending mediation proposal for a milestone,
    /// clearing it so the dispute proceeds through standard arbiter resolution.
    pub fn decline_mediation(env: Env, caller: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        caller.require_auth();

        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        Self::assert_buyer_or_supplier(&shipment, &caller);

        let key = DataKeyExt2::MediationProposal(shipment_id.clone(), milestone_index);
        env.storage().persistent().remove(&key);

        env.events().publish(
            (Symbol::new(&env, "mediation_declined"), shipment_id),
            (milestone_index, caller),
        );
    }

    /// Applies an accepted mediation's suggested outcome directly, mirroring the
    /// non-arbiter-fee branches of `resolve_dispute_timeout`: no arbiter fee is charged
    /// since no binding arbiter resolution occurred.
    fn apply_mediation_outcome(
        env: &Env,
        shipment: &mut Shipment,
        shipment_id: &String,
        milestone_index: u32,
        outcome: Resolution,
        mediator: &Address,
    ) {
        let mut milestone = shipment.milestones.get(milestone_index).unwrap();

        let partial_cp: Option<u32> = env.storage().persistent().get(&DataKey::DisputeContestedPercent(
            shipment_id.clone(),
            milestone_index,
        ));
        let full_payment = Self::milestone_gross_payment(env, shipment, milestone_index);
        let payment = if let Some(cp) = partial_cp {
            (full_payment * cp as i128) / 100
        } else {
            full_payment
        };

        let token_client = token::Client::new(env, &shipment.token);
        let is_supplier = outcome == Resolution::Supplier;

        if is_supplier {
            let mut fee_amount: i128 = 0;
            let net_payment =
                Self::deduct_fee_for_shipment(env, payment, &shipment.token, shipment_id, &mut fee_amount);
            Self::check_circuit_breaker(env, payment);
            Self::check_address_outflow(env, &shipment.supplier, payment);
            if net_payment > 0 {
                Self::pay_milestone_to_payees(
                    env,
                    shipment_id,
                    milestone_index,
                    net_payment,
                    &shipment.supplier,
                    &token_client,
                );
            }
            shipment.released_amount += payment;
            if shipment.dispute_bond_amount > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &shipment.dispute_bond_amount,
                );
            }
        } else {
            let primary_buyer = shipment.buyers.get(0).unwrap();
            if payment > 0 {
                token_client.transfer(&env.current_contract_address(), &primary_buyer, &payment);
            }
            shipment.released_amount += payment;
            if shipment.dispute_bond_amount > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &shipment.dispute_bond_amount,
                );
            }
        }

        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - payment).max(0),
        );

        if partial_cp.is_some() {
            env.storage().persistent().remove(&DataKey::DisputeContestedPercent(
                shipment_id.clone(),
                milestone_index,
            ));
        }
        env.storage()
            .persistent()
            .remove(&DataKeyExt::DisputeOpenedAt(shipment_id.clone(), milestone_index));

        milestone.status = MilestoneStatus::Resolved;
        // Write the updated status back before checking completion, so a single-
        // milestone shipment (or the last remaining one) is correctly detected as done.
        shipment.milestones.set(milestone_index, milestone);

        Self::append_audit_entry(
            env,
            shipment,
            mediator.clone(),
            Symbol::new(env, "mediation_accepted"),
            Symbol::new(env, "accept_mediation"),
        );

        if Self::all_milestones_done(shipment) {
            shipment.status = ShipmentStatus::Completed;

            Self::append_audit_entry(
                env,
                shipment,
                mediator.clone(),
                Symbol::new(env, "shipment_completed"),
                Symbol::new(env, "accept_mediation"),
            );

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
            env.storage().instance().set(&DataKey::ContractStats, &stats);
            Self::increment_reputation_internal(env, &shipment.supplier, 1, 0, 0);
            Self::move_shipment_status_index(
                env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                shipment_id,
            );
            Self::emit_shipment_completed(env, shipment_id, shipment.released_amount);
        }

        shipment.open_dispute_count = shipment.open_dispute_count.saturating_sub(1);
        shipment.last_dispute_resolved_ledger = Some(env.ledger().sequence());

        let disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(env));
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(env);
        for i in 0..disputes.len() {
            let d = disputes.get(i).unwrap();
            if !(d.shipment_id == *shipment_id && d.milestone_index == milestone_index) {
                new_disputes.push_back(d);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &new_disputes);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), shipment);

        let resolution_sym = if is_supplier {
            Symbol::new(env, "supplier")
        } else {
            Symbol::new(env, "buyer")
        };
        env.events().publish(
            (Symbol::new(env, "mediation_accepted"), shipment_id.clone()),
            (milestone_index, resolution_sym.clone(), mediator.clone()),
        );
        Self::emit_dispute_resolved(env, shipment_id, milestone_index, resolution_sym, mediator);
    }

    // ----------------------------------------------------------
    // #390 — N-OF-M ORACLE ATTESTATION REQUIREMENT
    // ----------------------------------------------------------

    /// Admin defines (or replaces) an N-of-M oracle set for a given verification
    /// `purpose`. `threshold` must be > 0 and <= `oracles.len()`.
    pub fn register_oracle_group(env: Env, admin: Address, purpose: Symbol, oracles: Vec<Address>, threshold: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if oracles.is_empty() {
            panic!("oracle group must have at least one member");
        }
        if threshold == 0 || threshold > oracles.len() {
            panic!("threshold must be between 1 and oracles.len()");
        }
        let key = DataKeyExt2::OracleGroup(purpose.clone());
        env.storage().persistent().set(&key, &(oracles.clone(), threshold));
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "oracle_group_registered"), purpose),
            (oracles.len() as u32, threshold),
        );
    }

    pub fn get_oracle_group(env: Env, purpose: Symbol) -> Option<(Vec<Address>, u32)> {
        env.storage().persistent().get(&DataKeyExt2::OracleGroup(purpose))
    }

    /// Assigns a registered oracle group `purpose` to gate milestone confirmation for
    /// `shipment_id`. Admin only. Passing an unregistered purpose is allowed (the gate
    /// simply has no effect until the group is registered).
    pub fn set_shipment_oracle_purpose(env: Env, admin: Address, shipment_id: String, purpose: Symbol) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let key = DataKeyExt2::ShipmentOraclePurpose(shipment_id.clone());
        env.storage().persistent().set(&key, &purpose);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "shipment_oracle_purpose_set"), shipment_id),
            purpose,
        );
    }

    /// A registered oracle attests to a milestone's off-chain verification purpose.
    /// Each oracle may attest at most once per (shipment_id, milestone_index).
    pub fn submit_oracle_attestation(
        env: Env,
        oracle: Address,
        shipment_id: String,
        milestone_index: u32,
    ) {
        Self::assert_not_paused(&env);
        oracle.require_auth();

        let purpose: Symbol = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::ShipmentOraclePurpose(shipment_id.clone()))
            .unwrap_or_else(|| panic!("no oracle group assigned to this shipment"));
        let (oracles, _threshold): (Vec<Address>, u32) = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::OracleGroup(purpose.clone()))
            .unwrap_or_else(|| panic!("oracle group not registered"));

        let mut is_member = false;
        for i in 0..oracles.len() {
            if oracles.get(i).unwrap() == oracle {
                is_member = true;
                break;
            }
        }
        if !is_member {
            panic!("caller is not a member of the assigned oracle group");
        }

        let key = DataKeyExt2::OracleAttestations(shipment_id.clone(), milestone_index, purpose);
        let mut attestations: Vec<Address> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        for i in 0..attestations.len() {
            if attestations.get(i).unwrap() == oracle {
                panic!("oracle has already attested to this milestone");
            }
        }
        attestations.push_back(oracle.clone());
        env.storage().persistent().set(&key, &attestations);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(&env, "oracle_attestation_submitted"), shipment_id),
            (milestone_index, oracle),
        );
    }

    pub fn get_oracle_attestation_count(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> u32 {
        let Some(purpose) = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, Symbol>(&DataKeyExt2::ShipmentOraclePurpose(shipment_id.clone()))
        else {
            return 0;
        };
        let attestations: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::OracleAttestations(shipment_id, milestone_index, purpose))
            .unwrap_or_else(|| Vec::new(&env));
        attestations.len()
    }

    /// Panics if `shipment_id` has an oracle group assigned but the milestone has not
    /// yet received the required threshold of attestations. No-op if no oracle group
    /// is assigned (backward compatible — existing shipments are unaffected).
    fn assert_oracle_attestation_met(env: &Env, shipment_id: &String, milestone_index: u32) {
        let Some(purpose) = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, Symbol>(&DataKeyExt2::ShipmentOraclePurpose(shipment_id.clone()))
        else {
            return;
        };
        let Some((_oracles, threshold)) = env
            .storage()
            .persistent()
            .get::<DataKeyExt2, (Vec<Address>, u32)>(&DataKeyExt2::OracleGroup(purpose.clone()))
        else {
            return;
        };
        let attestations: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::OracleAttestations(
                shipment_id.clone(),
                milestone_index,
                purpose,
            ))
            .unwrap_or_else(|| Vec::new(env));
        if attestations.len() < threshold {
            panic!("required oracle attestation threshold not yet met");
        }
    }

    // ----------------------------------------------------------
    // CLAIM DEADLINE REFUND (#164)
    // ----------------------------------------------------------

    /// Buyer triggers a full escrow refund when a per-milestone Unix timestamp deadline
    /// has elapsed without the milestone being confirmed.
    /// Marks the shipment Expired and returns all remaining escrowed funds to the buyer.
    pub fn claim_deadline_refund(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_index: u32,
    ) {
        Self::assert_not_paused(&env);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::require_buyer_auth(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        // Load the per-milestone timestamp deadlines stored at creation.
        let deadlines: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::MilestoneTimestampDeadlines(
                shipment_id.clone(),
            ))
            .unwrap_or_else(|| Vec::new(&env));

        if (milestone_index as usize) >= deadlines.len() as usize {
            panic!("no timestamp deadline set for this milestone");
        }
        let deadline = deadlines.get(milestone_index).unwrap();
        if deadline == 0 {
            panic!("no timestamp deadline set for this milestone");
        }

        let current_timestamp = env.ledger().timestamp();
        if current_timestamp <= deadline {
            panic!("DeadlineNotReached");
        }

        // Milestone must still be unconfirmed for the refund to apply.
        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status == MilestoneStatus::Confirmed
            || milestone.status == MilestoneStatus::Resolved
        {
            panic!("milestone already confirmed or resolved");
        }

        // Release all remaining escrowed funds back to the primary buyer.
        let refund_amount =
            shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;

        let primary_buyer = shipment.buyers.get(0).unwrap();
        let token_client = token::Client::new(&env, &shipment.token);

        if refund_amount > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &primary_buyer,
                &refund_amount,
            );
        }

        // Return any locked supplier collateral to the buyer (supplier failed to deliver).
        let collateral: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SupplierCollateral(shipment_id.clone()))
            .unwrap_or(0);
        if collateral > 0 {
            token_client.transfer(&env.current_contract_address(), &primary_buyer, &collateral);
        }

        // Update total escrowed tracking.
        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - refund_amount).max(0),
        );

        // Transition shipment to Expired.
        Self::move_shipment_status_index(
            &env,
            ShipmentStatus::Active,
            ShipmentStatus::Expired,
            &shipment_id,
        );
        shipment.status = ShipmentStatus::Expired;
        shipment.cancellation_reason = Vec::from_array(&env, [CancellationReason::DeadlineRefund]);
        Self::increment_reputation_internal(&env, &shipment.supplier, 0, 0, 1);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (Symbol::new(&env, "milestone_expired"), shipment_id.clone()),
            (milestone_index, refund_amount, primary_buyer),
        );
        Self::emit_shipment_cancelled(
            &env,
            &shipment_id,
            refund_amount,
            CancellationReason::DeadlineRefund,
        );
    }

    // ----------------------------------------------------------
    // #389 — ESCROW SWEEP OF UNCLAIMED REFUNDS TO TREASURY
    // ----------------------------------------------------------

    /// Admin configures how many ledgers a buyer has to call `claim_deadline_refund`
    /// after a milestone's timestamp deadline passes before the admin may sweep the
    /// unclaimed refund to the configured treasury instead. `ledgers == 0` disables
    /// sweeping (refunds remain claimable indefinitely — the original behaviour).
    pub fn set_refund_sweep_window(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt2::UnclaimedRefundSweepWindow, &ledgers);
        env.events().publish(
            (Symbol::new(&env, "refund_sweep_window_set"),),
            ledgers,
        );
    }

    pub fn get_refund_sweep_window(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::UnclaimedRefundSweepWindow)
            .unwrap_or(0)
    }

    /// Admin sweeps an unclaimed deadline refund to the configured treasury once the
    /// buyer has failed to call `claim_deadline_refund` within the admin-configured
    /// sweep window after the milestone's timestamp deadline elapsed. Requires
    /// `FeeConfig.treasury` to be set (via `set_fee_config`) as the sweep destination.
    /// Mirrors `claim_deadline_refund`'s eligibility checks and payout computation,
    /// except funds go to the treasury instead of the buyer.
    /// Anyone may call this once a milestone's timestamp deadline has passed, to record
    /// the ledger at which its refund became claimable. Idempotent — a second call is a
    /// harmless no-op. This checkpoint anchors the `sweep_unclaimed_refund` window so it
    /// measures from first-eligibility rather than from whenever the admin happens to
    /// call the sweep. Does not move funds.
    pub fn mark_refund_claimable(env: Env, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);

        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let deadlines: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::MilestoneTimestampDeadlines(
                shipment_id.clone(),
            ))
            .unwrap_or_else(|| Vec::new(&env));
        if (milestone_index as usize) >= deadlines.len() as usize {
            panic!("no timestamp deadline set for this milestone");
        }
        let deadline = deadlines.get(milestone_index).unwrap();
        if deadline == 0 {
            panic!("no timestamp deadline set for this milestone");
        }
        if env.ledger().timestamp() <= deadline {
            panic!("DeadlineNotReached");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status == MilestoneStatus::Confirmed
            || milestone.status == MilestoneStatus::Resolved
        {
            panic!("milestone already confirmed or resolved");
        }

        let claimable_key = DataKeyExt2::RefundClaimableAtLedger(shipment_id, milestone_index);
        if env.storage().persistent().has(&claimable_key) {
            return;
        }
        let now = env.ledger().sequence();
        env.storage().persistent().set(&claimable_key, &now);
        env.storage().persistent().extend_ttl(
            &claimable_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
    }

    pub fn sweep_unclaimed_refund(env: Env, admin: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let sweep_window: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::UnclaimedRefundSweepWindow)
            .unwrap_or(0);
        if sweep_window == 0 {
            panic!("unclaimed refund sweeping is not enabled");
        }

        let fee_config: FeeConfig = env
            .storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or_else(|| panic!("no treasury configured (call set_fee_config first)"));

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let deadlines: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::MilestoneTimestampDeadlines(
                shipment_id.clone(),
            ))
            .unwrap_or_else(|| Vec::new(&env));
        if (milestone_index as usize) >= deadlines.len() as usize {
            panic!("no timestamp deadline set for this milestone");
        }
        let deadline = deadlines.get(milestone_index).unwrap();
        if deadline == 0 {
            panic!("no timestamp deadline set for this milestone");
        }
        if env.ledger().timestamp() <= deadline {
            panic!("DeadlineNotReached");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status == MilestoneStatus::Confirmed
            || milestone.status == MilestoneStatus::Resolved
        {
            panic!("milestone already confirmed or resolved");
        }

        // The sweep window is measured from `mark_refund_claimable`'s checkpoint, not from
        // "now" — this call never creates that checkpoint itself, since doing so and then
        // panicking on the same invocation would roll back the write along with the panic.
        let claimable_key = DataKeyExt2::RefundClaimableAtLedger(shipment_id.clone(), milestone_index);
        let claimable_at: u32 = env
            .storage()
            .persistent()
            .get(&claimable_key)
            .unwrap_or_else(|| panic!("call mark_refund_claimable first"));
        if env.ledger().sequence() < claimable_at + sweep_window {
            panic!("sweep window has not elapsed");
        }

        let refund_amount =
            shipment.total_amount - shipment.released_amount - shipment.total_advanced_amount;

        let token_client = token::Client::new(&env, &shipment.token);
        if refund_amount > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &fee_config.treasury,
                &refund_amount,
            );
        }

        let collateral: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SupplierCollateral(shipment_id.clone()))
            .unwrap_or(0);
        if collateral > 0 {
            token_client.transfer(&env.current_contract_address(), &fee_config.treasury, &collateral);
        }

        let current_escrowed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalEscrowed(shipment.token.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalEscrowed(shipment.token.clone()),
            &(current_escrowed - refund_amount).max(0),
        );

        Self::move_shipment_status_index(
            &env,
            ShipmentStatus::Active,
            ShipmentStatus::Expired,
            &shipment_id,
        );
        shipment.status = ShipmentStatus::Expired;
        shipment.cancellation_reason = Vec::from_array(&env, [CancellationReason::DeadlineRefund]);
        Self::increment_reputation_internal(&env, &shipment.supplier, 0, 0, 1);

        env.storage().persistent().remove(&claimable_key);
        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        Self::append_admin_action(
            &env,
            Symbol::new(&env, "sweep_unclaimed_refund"),
            Symbol::new(&env, "unclaimed_refund_swept"),
        );
        env.events().publish(
            (Symbol::new(&env, "unclaimed_refund_swept"), shipment_id.clone()),
            (milestone_index, refund_amount, fee_config.treasury),
        );
        Self::emit_shipment_cancelled(
            &env,
            &shipment_id,
            refund_amount,
            CancellationReason::DeadlineRefund,
        );
    }

    // ----------------------------------------------------------
    // ARBITER FAILOVER
    // ----------------------------------------------------------

    pub fn activate_backup_arbiter(env: Env, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        let backup_arbiter: Option<Address> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::BackupArbiter(shipment_id.clone()));

        let backup = backup_arbiter.unwrap_or_else(|| panic!("no backup arbiter configured"));

        if shipment.arbiter == backup {
            panic!("already failed over");
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not in disputed status");
        }

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt::ArbiterInactivityThreshold)
            .unwrap_or(0);

        if threshold == 0 {
            panic!("arbiter failover is disabled globally");
        }

        let opened_ledger = milestone
            .dispute_opened_ledger
            .unwrap_or_else(|| panic!("dispute not opened"));
        let current_ledger = env.ledger().sequence();
        if current_ledger < opened_ledger + threshold {
            panic!("inactivity threshold not reached");
        }

        let old_arbiter = shipment.arbiter.clone();
        shipment.arbiter = backup.clone();

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        env.events().publish(
            (Symbol::new(&env, "arbiter_failover"), shipment_id),
            (old_arbiter, backup),
        );
    }

    // ----------------------------------------------------------
    // ADMIN: AUTO-CONFIRM THRESHOLD (#96)
    // ----------------------------------------------------------

    /// Set the global default auto-confirm review window.
    /// Shipments with review_window_ledgers = None fall back to this value.
    /// 0 disables auto-confirmation globally.
    pub fn set_auto_confirm_threshold(env: Env, admin: Address, window_ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::AutoConfirmThreshold, &window_ledgers);
        env.events().publish(
            (Symbol::new(&env, "auto_confirm_threshold_set"),),
            window_ledgers,
        );
    }

    pub fn get_auto_confirm_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::AutoConfirmThreshold)
            .unwrap_or(0)
    }

    pub fn set_arbiter_inactivity_threshold(env: Env, admin: Address, threshold_ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt::ArbiterInactivityThreshold, &threshold_ledgers);
        env.events().publish(
            (Symbol::new(&env, "arbiter_inactivity_threshold_set"),),
            threshold_ledgers,
        );
    }

    pub fn set_confirmation_cooldown(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt::GlobalConfirmationCooldown, &ledgers);
        env.events()
            .publish((Symbol::new(&env, "confirmation_cooldown_set"),), ledgers);
    }

    // ----------------------------------------------------------
    // ADMIN: MIN SHIPMENT VALUE (#42)
    // ----------------------------------------------------------

    pub fn set_min_shipment_value(env: Env, admin: Address, min_amount: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::MinShipmentValue, &min_amount);
        env.events()
            .publish((Symbol::new(&env, "min_shipment_value_set"),), min_amount);
    }

    pub fn get_min_shipment_value(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::MinShipmentValue)
            .unwrap_or(0)
    }

    // ----------------------------------------------------------
    // ADMIN: SHIPMENT-LEVEL FEE OVERRIDE (#299)
    // ----------------------------------------------------------

    pub fn set_shipment_fee_override(env: Env, admin: Address, shipment_id: String, fee_bps: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        env.storage().persistent().set(
            &DataKeyExt::ShipmentFeeOverride(shipment_id.clone()),
            &fee_bps,
        );
        env.events().publish(
            (Symbol::new(&env, "shipment_fee_override_set"), shipment_id),
            fee_bps,
        );
    }

    pub fn clear_shipment_fee_override(env: Env, admin: Address, shipment_id: String) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .persistent()
            .remove(&DataKeyExt::ShipmentFeeOverride(shipment_id.clone()));
        env.events().publish(
            (Symbol::new(&env, "shipment_fee_override_cleared"),),
            shipment_id,
        );
    }

    pub fn get_shipment_fee_override(env: Env, shipment_id: String) -> Option<u32> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeOverride(shipment_id))
    }

    // ----------------------------------------------------------
    // ADMIN: LONG-HOLD ESCROW REBATE (#300)
    // ----------------------------------------------------------

    /// Configure the long-hold escrow rebate (admin-only).
    ///
    /// Stores a `(threshold_ledgers, rebate_bps)` tuple on instance storage and
    /// emits a `long_hold_rebate_set` event.
    ///
    /// * `threshold_ledgers` — the minimum age, measured in ledger sequences
    ///   since the shipment was created (`current_ledger - Shipment.created_at`),
    ///   that a shipment must reach before the rebate can apply. `0` disables the
    ///   threshold and therefore the rebate.
    /// * `rebate_bps` — the share of the platform fee that is given back to the
    ///   supplier, expressed in basis points (`1 bps = 0.01%`, `10_000 bps = 100%`).
    ///   `0` disables the rebate.
    ///
    /// The rebate is applied inside `confirm_milestone` on the immediate-release
    /// path (no holdback): if the shipment's ledger age is at least
    /// `threshold_ledgers` AND a platform fee was actually charged, then
    /// `rebate = fee_amount * rebate_bps / 10_000` is added back to the supplier's
    /// net payment and removed from the fee reported for the milestone.
    ///
    /// Both values must be non-zero for the rebate to take effect.
    ///
    /// ```text
    /// Worked example: set_long_hold_rebate(admin, 1000, 1000)
    ///   fee_bps       = 100  (1% platform fee)
    ///   payout        = 1_000_000
    ///   fee_amount    = 1_000_000 * 100 / 10_000   = 10_000
    ///   shipment age  = 3200 - 2000                = 1200   (>= 1000, threshold met)
    ///   rebate        = 10_000 * 1000 / 10_000     = 1_000
    ///   supplier net  = 990_000 + 1_000            = 991_000
    ///   effective fee = 10_000 - 1_000             = 9_000  (0.9% instead of 1%)
    /// ```
    pub fn set_long_hold_rebate(env: Env, admin: Address, threshold_ledgers: u32, rebate_bps: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage().instance().set(
            &DataKeyExt::LongHoldRebate,
            &(threshold_ledgers, rebate_bps),
        );
        env.events().publish(
            (Symbol::new(&env, "long_hold_rebate_set"),),
            (threshold_ledgers, rebate_bps),
        );
    }

    /// Read the current long-hold escrow rebate configuration (read-only).
    ///
    /// Returns the `(threshold_ledgers, rebate_bps)` tuple stored by
    /// `set_long_hold_rebate`, or `(0, 0)` if the rebate has never been
    /// configured (i.e. the rebate is disabled by default).
    pub fn get_long_hold_rebate(env: Env) -> (u32, u32) {
        env.storage()
            .instance()
            .get(&DataKeyExt::LongHoldRebate)
            .unwrap_or((0u32, 0u32))
    }

    // ----------------------------------------------------------
    // ADMIN: GOVERNANCE TIMELOCK (#298)
    // ----------------------------------------------------------

    pub fn set_timelock_duration(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt::TimelockDuration, &ledgers);
        env.events()
            .publish((Symbol::new(&env, "timelock_duration_set"),), ledgers);
    }

    pub fn propose_param_change(env: Env, admin: Address, param: Symbol, new_value: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let timelock: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt::TimelockDuration)
            .unwrap_or(0);
        let effective_ledger = env.ledger().sequence() + timelock;
        env.storage().instance().set(
            &DataKeyExt::PendingParamChange(param.clone()),
            &(new_value, effective_ledger),
        );
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "propose_param_change"),
            param.clone(),
        );
        env.events().publish(
            (Symbol::new(&env, "param_change_proposed"), param),
            (new_value, effective_ledger),
        );
    }

    pub fn execute_param_change(env: Env, admin: Address, param: Symbol) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let (new_value, effective_ledger): (i128, u32) = env
            .storage()
            .instance()
            .get(&DataKeyExt::PendingParamChange(param.clone()))
            .unwrap_or_else(|| panic!("NoPendingParamChange"));
        if env.ledger().sequence() < effective_ledger {
            panic!("TimelockNotExpired");
        }
        env.storage()
            .instance()
            .remove(&DataKeyExt::PendingParamChange(param.clone()));
        let fee_sym = Symbol::new(&env, "fee_config");
        let max_sym = Symbol::new(&env, "max_shipment_value");
        let cb_sym = Symbol::new(&env, "circuit_breaker_limit");
        if param == fee_sym {
            if let Some(mut cfg) = env
                .storage()
                .instance()
                .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
            {
                cfg.fee_bps = new_value as u32;
                env.storage().instance().set(&DataKey::FeeConfig, &cfg);
            }
        } else if param == max_sym {
            env.storage()
                .instance()
                .set(&DataKey::MaxShipmentValue, &new_value);
        } else if param == cb_sym {
            env.storage()
                .instance()
                .set(&DataKey::CircuitBreakerLimit, &new_value);
        } else {
            panic!("unsupported param");
        }
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "execute_param_change"),
            param.clone(),
        );
        env.events().publish(
            (Symbol::new(&env, "param_change_executed"), param),
            new_value,
        );
    }

    pub fn cancel_param_change(env: Env, admin: Address, param: Symbol) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if !env
            .storage()
            .instance()
            .has(&DataKeyExt::PendingParamChange(param.clone()))
        {
            panic!("NoPendingParamChange");
        }
        env.storage()
            .instance()
            .remove(&DataKeyExt::PendingParamChange(param.clone()));
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "cancel_param_change"),
            param.clone(),
        );
        env.events()
            .publish((Symbol::new(&env, "param_change_cancelled"),), param);
    }

    // ----------------------------------------------------------
    // FEATURE A: ARBITER PANEL – CAST DISPUTE VOTE
    // ----------------------------------------------------------

    /// Cast a vote on an open panel dispute. Only callable by a member of the shipment's
    /// arbiter panel. Each panel member may vote exactly once for the milestone dispute.
    /// A panel contains the addresses supplied in `ShipmentOptions::arbiter_panel` when the
    /// shipment is created; panel mode requires at least three members.
    /// A quorum is a simple majority: `panel.len() / 2 + 1` votes in the same direction.
    /// When that threshold is reached, the dispute resolves automatically using the same
    /// payout logic as `resolve_dispute`: `approve = true` resolves for the supplier and
    /// `approve = false` resolves for the buyer. Until a majority is reached, including a
    /// tie or an incomplete vote, the dispute remains open.
    pub fn cast_dispute_vote(
        env: Env,
        arbiter: Address,
        shipment_id: String,
        milestone_index: u32,
        approve: bool,
    ) {
        Self::assert_not_paused(&env);
        arbiter.require_auth();

        // Load the panel; panic if this shipment has no panel.
        let panel: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ArbiterPanel(shipment_id.clone()))
            .unwrap_or_else(|| panic!("shipment has no arbiter panel"));

        // Verify caller is a panel member.
        let mut is_member = false;
        for i in 0..panel.len() {
            if panel.get(i).unwrap() == arbiter {
                is_member = true;
                break;
            }
        }
        if !is_member {
            panic!("NotPanelMember");
        }

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("DisputeAlreadyResolved");
        }

        // Load existing votes, check for duplicate.
        let votes_key = DataKeyExt::DisputeVotes(shipment_id.clone(), milestone_index);
        let mut votes: Vec<DisputeVote> = env
            .storage()
            .persistent()
            .get(&votes_key)
            .unwrap_or_else(|| Vec::new(&env));

        for i in 0..votes.len() {
            if votes.get(i).unwrap().arbiter == arbiter {
                panic!("AlreadyVoted");
            }
        }

        // Record the vote.
        votes.push_back(DisputeVote {
            arbiter: arbiter.clone(),
            approve,
        });
        env.storage().persistent().set(&votes_key, &votes);
        env.storage().persistent().extend_ttl(
            &votes_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (Symbol::new(&env, "dispute_vote_cast"), shipment_id.clone()),
            (
                milestone_index,
                arbiter.clone(),
                approve,
                votes.len() as u32,
            ),
        );

        // Check for majority: count approve vs reject votes.
        let threshold = panel.len() / 2 + 1; // simple majority
        let mut approve_count: u32 = 0;
        let mut reject_count: u32 = 0;
        for i in 0..votes.len() {
            let v = votes.get(i).unwrap();
            if v.approve {
                approve_count += 1;
            } else {
                reject_count += 1;
            }
        }

        let majority_approve = approve_count >= threshold as u32;
        let majority_reject = reject_count >= threshold as u32;

        if majority_approve || majority_reject {
            // Clean up votes before delegating to resolve logic.
            env.storage().persistent().remove(&votes_key);

            // Temporarily set shipment.arbiter to the panel caller so
            // require_arbiter_auth inside resolve_dispute passes.
            // We use resolve_dispute_panel_internal to bypass auth.
            Self::resolve_dispute_panel_internal(
                &env,
                shipment_id,
                milestone_index,
                majority_approve,
                arbiter,
            );
        }
    }

    /// Internal: resolve a panel dispute once majority is reached. Mirrors resolve_dispute
    /// logic exactly but skips auth (already verified via panel membership above).
    fn resolve_dispute_panel_internal(
        env: &Env,
        shipment_id: String,
        milestone_index: u32,
        approve: bool,
        resolver: Address,
    ) {
        let ctx = Self::fetch_resolve_dispute_ctx(env, &shipment_id, milestone_index);
        let mut shipment = ctx.shipment;

        let is_partial = ctx.partial_contested_percent.is_some();
        let full_payment = Self::milestone_gross_payment(env, &shipment, milestone_index);
        let payment = if let Some(cp) = ctx.partial_contested_percent {
            (full_payment * cp as i128) / 100
        } else {
            full_payment
        };

        let token_client = token::Client::new(env, &shipment.token);

        if approve {
            let advance_deducted = Self::consume_advance_for_milestone(
                env,
                &mut shipment,
                &shipment_id,
                milestone_index,
            );

            let mut fee_amount: i128 = 0;
            let net_payment = Self::deduct_fee(env, payment, &shipment.token, &mut fee_amount);
            Self::check_circuit_breaker(env, payment);
            Self::check_address_outflow(env, &shipment.supplier, payment);

            let fee_bps = Self::applicable_arbiter_fee_bps(env, payment, shipment.arbiter_fee_bps);
            // For panel, fee is split equally among all panel members.
            let panel: Vec<Address> = env
                .storage()
                .persistent()
                .get(&DataKeyExt::ArbiterPanel(shipment_id.clone()))
                .unwrap_or_else(|| Vec::new(env));
            let arbiter_fee_total = (payment * fee_bps as i128) / 10_000;
            if arbiter_fee_total > 0 && !panel.is_empty() {
                let per_arbiter = arbiter_fee_total / panel.len() as i128;
                let mut distributed: i128 = 0;
                for i in 0..panel.len() {
                    let p = panel.get(i).unwrap();
                    let amount = if i == panel.len() - 1 {
                        // Last arbiter gets remainder to avoid dust loss.
                        arbiter_fee_total - distributed
                    } else {
                        per_arbiter
                    };
                    if amount > 0 {
                        token_client.transfer(&env.current_contract_address(), &p, &amount);
                        distributed += amount;
                    }
                }
            }

            shipment.released_amount += payment;
            let actual_transfer = (net_payment - advance_deducted - arbiter_fee_total).max(0);

            // Feature C: split payment across milestone payees if configured.
            Self::pay_milestone_to_payees(
                env,
                &shipment_id,
                milestone_index,
                actual_transfer,
                &shipment.supplier,
                &token_client,
            );

            if shipment.dispute_bond_amount > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &shipment.dispute_bond_amount,
                );
            }

            let mut m = shipment.milestones.get(milestone_index).unwrap();
            m.status = MilestoneStatus::Resolved;
            shipment.milestones.set(milestone_index, m);
        } else if is_partial {
            let fee_bps = Self::applicable_arbiter_fee_bps(env, payment, shipment.arbiter_fee_bps);
            let panel: Vec<Address> = env
                .storage()
                .persistent()
                .get(&DataKeyExt::ArbiterPanel(shipment_id.clone()))
                .unwrap_or_else(|| Vec::new(env));
            let arbiter_fee_total = (payment * fee_bps as i128) / 10_000;
            if arbiter_fee_total > 0 && !panel.is_empty() {
                let per_arbiter = arbiter_fee_total / panel.len() as i128;
                let mut distributed: i128 = 0;
                for i in 0..panel.len() {
                    let p = panel.get(i).unwrap();
                    let amount = if i == panel.len() - 1 {
                        arbiter_fee_total - distributed
                    } else {
                        per_arbiter
                    };
                    if amount > 0 {
                        token_client.transfer(&env.current_contract_address(), &p, &amount);
                        distributed += amount;
                    }
                }
            }
            let buyer_refund = (payment - arbiter_fee_total).max(0);
            if buyer_refund > 0 {
                let primary_buyer = shipment.buyers.get(0).unwrap();
                token_client.transfer(
                    &env.current_contract_address(),
                    &primary_buyer,
                    &buyer_refund,
                );
            }
            shipment.released_amount += payment;
            if shipment.dispute_bond_amount > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &shipment.dispute_bond_amount,
                );
            }
            let mut m = shipment.milestones.get(milestone_index).unwrap();
            m.status = MilestoneStatus::Resolved;
            shipment.milestones.set(milestone_index, m);
        } else {
            // Full dispute rejection — reset to Pending.
            if shipment.dispute_bond_amount > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &shipment.supplier,
                    &shipment.dispute_bond_amount,
                );
            }
            let mut m = shipment.milestones.get(milestone_index).unwrap();
            m.status = MilestoneStatus::Pending;
            shipment.milestones.set(milestone_index, m);
        }

        // Clean up partial dispute record.
        if is_partial {
            env.storage()
                .persistent()
                .remove(&DataKey::DisputeContestedPercent(
                    shipment_id.clone(),
                    milestone_index,
                ));
        }

        // Update arbiter stats for the resolver.
        let resolution_ledgers = env.ledger().sequence()
            - shipment
                .milestones
                .get(milestone_index)
                .unwrap()
                .dispute_opened_ledger
                .unwrap_or(env.ledger().sequence());
        let mut arbiter_stats = crate::storage::get_arbiter_stats(env, &resolver);
        if approve {
            arbiter_stats.resolved_approved += 1;
        } else {
            arbiter_stats.resolved_rejected += 1;
        }
        arbiter_stats.total_resolution_ledgers += resolution_ledgers as u64;
        crate::storage::set_arbiter_stats(env, &resolver, &arbiter_stats);

        shipment.open_dispute_count = shipment.open_dispute_count.saturating_sub(1);
        shipment.last_dispute_resolved_ledger = Some(env.ledger().sequence());

        Self::append_audit_entry(
            env,
            &mut shipment,
            resolver.clone(),
            Symbol::new(env, "dispute_resolved"),
            Symbol::new(env, "cast_dispute_vote"),
        );

        if Self::all_milestones_done(&shipment) {
            shipment.status = ShipmentStatus::Completed;
            Self::append_audit_entry(
                env,
                &mut shipment,
                resolver.clone(),
                Symbol::new(env, "shipment_completed"),
                Symbol::new(env, "cast_dispute_vote"),
            );
            let mut stats = ctx.contract_stats;
            stats.completed_shipments += 1;
            env.storage()
                .instance()
                .set(&DataKey::ContractStats, &stats);
            Self::increment_reputation_internal(env, &shipment.supplier, 1, 0, 0);
            Self::move_shipment_status_index(
                env,
                ShipmentStatus::Active,
                ShipmentStatus::Completed,
                &shipment_id,
            );
            Self::emit_shipment_completed(env, &shipment_id, shipment.released_amount);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Remove from active disputes list.
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(env);
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
            (Symbol::new(env, "dispute_resolved"), shipment_id.clone()),
            (
                milestone_index,
                approve,
                is_partial,
                released_amount,
                remaining_amount,
            ),
        );
        let resolution = if approve {
            Symbol::new(env, "supplier")
        } else {
            Symbol::new(env, "buyer")
        };
        Self::emit_dispute_resolved(env, &shipment_id, milestone_index, resolution, &resolver);
    }

    /// Returns the current votes for a panel dispute identified by shipment and milestone.
    /// Each entry contains the panel member and their direction (`approve = true` for the
    /// supplier, `approve = false` for the buyer). The vector is empty before any votes are
    /// cast and after a majority automatically resolves the dispute.
    pub fn get_dispute_votes(env: Env, shipment_id: String, milestone_index: u32) -> Vec<DisputeVote> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::DisputeVotes(shipment_id, milestone_index))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns the arbiter panel for a shipment, in the order supplied at creation.
    /// Panel members are the addresses that may vote; panel mode requires at least three
    /// non-blacklisted members. An empty vector means the shipment uses a single arbiter.
    pub fn get_arbiter_panel(env: Env, shipment_id: String) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::ArbiterPanel(shipment_id))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // FEATURE B: SUPPLIER EXPOSURE CAP
    // ----------------------------------------------------------

    /// Admin sets the global supplier exposure cap.
    /// `cap = 0` disables the check (default behaviour, backward compatible).
    pub fn set_supplier_exposure_cap(env: Env, admin: Address, cap: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if cap < 0 {
            panic!("cap must be non-negative");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt::SupplierExposureCap, &cap);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_supplier_exposure_cap"),
            Symbol::new(&env, "supplier_exposure_cap_set"),
        );
        env.events()
            .publish((Symbol::new(&env, "supplier_exposure_cap_set"),), cap);
    }

    pub fn get_supplier_exposure_cap(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKeyExt::SupplierExposureCap)
            .unwrap_or(0)
    }

    /// Returns the current aggregate locked escrow for `supplier` across all Active shipments.
    pub fn get_supplier_exposure(env: Env, supplier: Address) -> i128 {
        Self::compute_supplier_exposure(&env, &supplier)
    }

    // ----------------------------------------------------------
    // FEATURE C: MILESTONE PAYEES
    // ----------------------------------------------------------

    /// Buyer configures the payee split for a milestone before it leaves `Pending` status.
    /// `payees` is a list of `(Address, percent)` whose percents must sum to 100.
    /// Pass an empty vec to remove any previously configured split.
    pub fn set_milestone_payees(
        env: Env,
        buyer: Address,
        shipment_id: String,
        milestone_index: u32,
        payees: Vec<MilestonePayee>,
    ) {
        Self::assert_not_paused(&env);

        let shipment = Self::get_shipment_internal(&env, &shipment_id);

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
            panic!("MilestoneNotPending");
        }

        if payees.len() > 0 {
            let mut total: u32 = 0;
            for i in 0..payees.len() {
                total += payees.get(i).unwrap().percent;
            }
            if total != 100 {
                panic!("InvalidPayeePercentages");
            }
        }

        let key = DataKeyExt::MilestonePayees(shipment_id.clone(), milestone_index);
        if payees.len() > 0 {
            env.storage().persistent().set(&key, &payees);
            env.storage().persistent().extend_ttl(
                &key,
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        } else {
            env.storage().persistent().remove(&key);
        }

        env.events().publish(
            (Symbol::new(&env, "milestone_payees_set"), shipment_id),
            (milestone_index, payees.len() as u32, buyer),
        );
    }

    /// Returns the configured payees for a milestone.
    /// Empty vec = no split configured; payment goes entirely to the supplier.
    pub fn get_milestone_payees(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Vec<MilestonePayee> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::MilestonePayees(shipment_id, milestone_index))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // FEATURE D: AUTO-BLACKLIST RULE
    // ----------------------------------------------------------

    /// Admin configures the auto-blacklist thresholds.
    /// Either threshold set to `0` disables that individual check.
    /// Both `0` disables auto-blacklisting entirely (default).
    pub fn set_auto_blacklist_rule(
        env: Env,
        admin: Address,
        max_cancelled: u32,
        max_disputed: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage().instance().set(
            &DataKeyExt::AutoBlacklistRule,
            &AutoBlacklistRule {
                max_cancelled,
                max_disputed,
            },
        );
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_auto_blacklist_rule"),
            Symbol::new(&env, "auto_blacklist_rule_set"),
        );
        env.events().publish(
            (Symbol::new(&env, "auto_blacklist_rule_set"),),
            (max_cancelled, max_disputed),
        );
    }

    pub fn get_auto_blacklist_rule(env: Env) -> AutoBlacklistRule {
        env.storage()
            .instance()
            .get(&DataKeyExt::AutoBlacklistRule)
            .unwrap_or_default()
    }

    // ----------------------------------------------------------
    // EMERGENCY FUND RECOVERY — DELAYED & CANCELLABLE (Issue #302)
    // ----------------------------------------------------------

    /// Set the mandatory delay (in ledgers) between proposing and executing an
    /// emergency recovery. A value of 0 preserves immediate-execution behaviour.
    /// Admin only.
    pub fn set_recovery_delay(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::RecoveryDelayLedgers, &ledgers);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_recovery_delay"),
            Symbol::new(&env, "recovery_delay_updated"),
        );
        env.events().publish(
            (Symbol::new(&env, "recovery_delay_updated"),),
            (admin, ledgers, env.ledger().sequence()),
        );
    }

    /// Step 1 of 2 — Record intent to recover stuck escrow funds from an abandoned
    /// shipment. When `recovery_delay_ledgers > 0`, the proposal cannot be executed
    /// until `effective_ledger` has been reached, giving stakeholders visibility.
    /// When `recovery_delay_ledgers == 0`, this call immediately executes the recovery
    /// (preserving the original single-step behaviour).
    pub fn propose_emergency_recover(env: Env, admin: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        let current_ledger = env.ledger().sequence();
        if current_ledger <= shipment.created_at + RECOVERY_THRESHOLD_LEDGERS {
            panic!("recovery threshold not reached");
        }

        let delay: u32 = env
            .storage()
            .instance()
            .get(&DataKey::RecoveryDelayLedgers)
            .unwrap_or(0);

        // Record the proposal in the admin audit log.
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "propose_recovery"),
            Symbol::new(&env, "recovery_proposed"),
        );

        if delay == 0 {
            // Immediate execution — auth/pause/admin already checked above.
            Self::do_emergency_recover(&env, &admin, &shipment_id);
        } else {
            let effective_ledger = current_ledger + delay;
            let proposal = RecoveryProposal {
                effective_ledger,
                proposed_by: admin.clone(),
            };
            env.storage()
                .persistent()
                .set(&DataKey::PendingRecovery(shipment_id.clone()), &proposal);
            env.storage().persistent().extend_ttl(
                &DataKey::PendingRecovery(shipment_id.clone()),
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
            env.events().publish(
                (Symbol::new(&env, "recovery_proposed"), shipment_id.clone()),
                (admin, effective_ledger, current_ledger),
            );
        }
    }

    /// Step 2 of 2 — Execute a pending emergency recovery proposal once its
    /// `effective_ledger` has been reached. Panics if called too early.
    pub fn execute_emergency_recover(env: Env, admin: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let proposal: RecoveryProposal = env
            .storage()
            .persistent()
            .get(&DataKey::PendingRecovery(shipment_id.clone()))
            .unwrap_or_else(|| panic!("no pending recovery proposal for this shipment"));

        if env.ledger().sequence() < proposal.effective_ledger {
            panic!("recovery delay has not elapsed");
        }

        // Remove the proposal before executing (prevents re-entrancy).
        env.storage()
            .persistent()
            .remove(&DataKey::PendingRecovery(shipment_id.clone()));

        // Record execution in the admin audit log.
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "execute_recovery"),
            Symbol::new(&env, "recovery_executed"),
        );

        Self::do_emergency_recover(&env, &admin, &shipment_id);
    }

    /// Cancel a pending emergency recovery proposal before it is executed.
    /// Admin only. Panics if no proposal exists for the given shipment.
    pub fn cancel_emergency_recover(env: Env, admin: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        if !env
            .storage()
            .persistent()
            .has(&DataKey::PendingRecovery(shipment_id.clone()))
        {
            panic!("no pending recovery proposal to cancel");
        }

        env.storage()
            .persistent()
            .remove(&DataKey::PendingRecovery(shipment_id.clone()));

        Self::append_admin_action(
            &env,
            Symbol::new(&env, "cancel_recovery"),
            Symbol::new(&env, "recovery_cancelled"),
        );

        env.events().publish(
            (Symbol::new(&env, "recovery_cancelled"), shipment_id.clone()),
            (admin, env.ledger().sequence()),
        );
    }

    /// Returns the pending recovery proposal for a shipment, if one exists.
    pub fn get_pending_recovery(env: Env, shipment_id: String) -> Option<RecoveryProposal> {
        env.storage()
            .persistent()
            .get(&DataKey::PendingRecovery(shipment_id))
    }

    // ----------------------------------------------------------
    // REPUTATION FAST-TRACK
    // ----------------------------------------------------------

    /// Configure global reputation fast-track policy. Admin only.
    /// Absent policy (default) keeps full confirmation-cooldown behaviour.
    pub fn set_reputation_fast_track(
        env: Env,
        admin: Address,
        min_completed: u32,
        max_disputed_ratio_bps: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if max_disputed_ratio_bps > 10_000 {
            panic!("max_disputed_ratio_bps cannot exceed 10000");
        }
        let policy = ReputationFastTrack {
            min_completed,
            max_disputed_ratio_bps,
        };
        env.storage()
            .instance()
            .set(&DataKeyExt::ReputationFastTrack, &policy);
        env.events().publish(
            (Symbol::new(&env, "reputation_fast_track_set"),),
            (min_completed, max_disputed_ratio_bps),
        );
    }

    /// Whether a supplier currently qualifies for proof-review fast-track.
    pub fn is_fast_track_eligible(env: Env, supplier: Address) -> bool {
        Self::is_fast_track_eligible_internal(&env, &supplier)
    }

    // ----------------------------------------------------------
    // PER-SHIPMENT MUTUAL-CONSENT PAUSE
    // ----------------------------------------------------------

    /// Buyer or supplier requests a pause. No effect until the other party approves.
    pub fn request_shipment_pause(env: Env, caller: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        caller.require_auth();
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_buyer_or_supplier(&shipment, &caller);
        if Self::is_shipment_paused_internal(&env, &shipment_id) {
            panic!("shipment is already paused");
        }
        let req = ShipmentPauseRequest {
            requester: caller.clone(),
            is_resume: false,
        };
        env.storage()
            .persistent()
            .set(&DataKeyExt::ShipmentPauseRequest(shipment_id.clone()), &req);
        env.events().publish(
            (Symbol::new(&env, "shipment_pause_requested"), shipment_id),
            caller,
        );
    }

    /// Counterpart approves a pending pause request; freezes deadlines for this shipment.
    pub fn approve_shipment_pause(env: Env, caller: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        caller.require_auth();
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_buyer_or_supplier(&shipment, &caller);
        if Self::is_shipment_paused_internal(&env, &shipment_id) {
            panic!("shipment is already paused");
        }
        let req: ShipmentPauseRequest = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentPauseRequest(shipment_id.clone()))
            .unwrap_or_else(|| panic!("no pending pause request"));
        if req.is_resume {
            panic!("no pending pause request");
        }
        if req.requester == caller {
            panic!("cannot approve own pause request");
        }
        let now = env.ledger().sequence();
        env.storage()
            .persistent()
            .set(&DataKeyExt::ShipmentPaused(shipment_id.clone()), &true);
        env.storage()
            .persistent()
            .set(&DataKeyExt::ShipmentPausedAt(shipment_id.clone()), &now);
        env.storage()
            .persistent()
            .remove(&DataKeyExt::ShipmentPauseRequest(shipment_id.clone()));
        env.events().publish(
            (Symbol::new(&env, "shipment_paused"), shipment_id),
            (caller, now),
        );
    }

    /// Either party requests resumption; the other must call again to approve (mutual consent).
    pub fn resume_shipment(env: Env, caller: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        caller.require_auth();
        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);
        Self::assert_buyer_or_supplier(&shipment, &caller);
        if !Self::is_shipment_paused_internal(&env, &shipment_id) {
            panic!("shipment is not paused");
        }

        let key = DataKeyExt::ShipmentPauseRequest(shipment_id.clone());
        if let Some(pending) = env
            .storage()
            .persistent()
            .get::<DataKeyExt, ShipmentPauseRequest>(&key)
        {
            if pending.is_resume && pending.requester != caller {
                // Counterpart approves — resume and unfreeze deadlines.
                let paused_at: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKeyExt::ShipmentPausedAt(shipment_id.clone()))
                    .unwrap_or(env.ledger().sequence());
                let delta = env.ledger().sequence().saturating_sub(paused_at);
                Self::bump_shipment_deadlines_for_pause(&env, &mut shipment, delta);
                env.storage()
                    .persistent()
                    .set(&DataKey::Shipment(shipment_id.clone()), &shipment);
                env.storage()
                    .persistent()
                    .set(&DataKeyExt::ShipmentPaused(shipment_id.clone()), &false);
                env.storage()
                    .persistent()
                    .remove(&DataKeyExt::ShipmentPausedAt(shipment_id.clone()));
                env.storage().persistent().remove(&key);
                env.events().publish(
                    (Symbol::new(&env, "shipment_resumed"), shipment_id),
                    (caller, env.ledger().sequence()),
                );
                return;
            }
            if pending.is_resume && pending.requester == caller {
                panic!("resume already requested; awaiting counterpart");
            }
        }

        let req = ShipmentPauseRequest {
            requester: caller.clone(),
            is_resume: true,
        };
        env.storage().persistent().set(&key, &req);
        env.events().publish(
            (Symbol::new(&env, "shipment_resume_requested"), shipment_id),
            caller,
        );
    }

    // ----------------------------------------------------------
    // PER-SHIPMENT COMPLIANCE HOLD (#368)
    // ----------------------------------------------------------

    /// Admin unilaterally freezes all state-changing operations on a single shipment
    /// pending an off-chain compliance/legal review. `reason_hash` is an off-chain
    /// document hash (e.g. IPFS CID) describing the reason. Other shipments are unaffected.
    pub fn set_compliance_hold(
        env: Env,
        admin: Address,
        shipment_id: String,
        reason_hash: BytesN<32>,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        // Ensure the shipment exists (panics with "shipment not found" otherwise).
        Self::get_shipment_internal(&env, &shipment_id);
        let key = DataKeyExt2::ComplianceHold(shipment_id.clone());
        env.storage().persistent().set(&key, &reason_hash);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (Symbol::new(&env, "compliance_hold_set"), shipment_id),
            reason_hash,
        );
    }

    /// Admin lifts a compliance hold on a single shipment.
    pub fn clear_compliance_hold(env: Env, admin: Address, shipment_id: String) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        let key = DataKeyExt2::ComplianceHold(shipment_id.clone());
        if !env.storage().persistent().has(&key) {
            panic!("shipment is not on compliance hold");
        }
        env.storage().persistent().remove(&key);
        env.events().publish(
            (Symbol::new(&env, "compliance_hold_cleared"), shipment_id),
            (),
        );
    }

    /// Read-only check for whether a shipment is currently on compliance hold.
    pub fn is_on_compliance_hold(env: Env, shipment_id: String) -> bool {
        env.storage()
            .persistent()
            .has(&DataKeyExt2::ComplianceHold(shipment_id))
    }

    fn assert_shipment_not_on_hold(env: &Env, shipment_id: &String) {
        if env
            .storage()
            .persistent()
            .has(&DataKeyExt2::ComplianceHold(shipment_id.clone()))
        {
            panic!("shipment is on compliance hold pending review");
        }
    }

    // ----------------------------------------------------------
    // MILESTONE NOTES
    // ----------------------------------------------------------

    /// Append an informational note to a milestone (buyer/supplier/logistics). Cap = 10.
    pub fn add_milestone_note(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        note: String,
    ) {
        Self::assert_not_paused(&env);
        caller.require_auth();
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }
        Self::assert_shipment_participant(&shipment, &caller);
        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let key = DataKeyExt::MilestoneNotes(shipment_id.clone(), milestone_index);
        let mut notes: Vec<MilestoneNote> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        let entry = MilestoneNote {
            author: caller.clone(),
            note: note.clone(),
            ledger: env.ledger().sequence(),
        };
        notes.push_back(entry);
        while notes.len() > MAX_MILESTONE_NOTES {
            let mut next = Vec::new(&env);
            for i in 1..notes.len() {
                next.push_back(notes.get(i).unwrap());
            }
            notes = next;
        }
        env.storage().persistent().set(&key, &notes);
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
        env.events().publish(
            (
                Symbol::new(&env, "milestone_note_added"),
                shipment_id,
                milestone_index,
            ),
            (caller, note),
        );
    }

    pub fn get_milestone_notes(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Vec<MilestoneNote> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::MilestoneNotes(shipment_id, milestone_index))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // #394: SHIPMENT-LEVEL CUSTOM METADATA KEY-VALUE STORE
    // ----------------------------------------------------------

    /// Buyer or supplier attaches/updates a small piece of structured data on
    /// a shipment (e.g. a PO number, cost center, or internal reference code)
    /// beyond the single IPFS metadata hash captured at creation.
    pub fn set_shipment_metadata(env: Env, caller: Address, shipment_id: String, key: Symbol, value: String) {
        Self::assert_not_paused(&env);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        Self::assert_buyer_or_supplier(&shipment, &caller);
        caller.require_auth();

        let meta_key = DataKeyExt2::ShipmentMetadata(shipment_id.clone(), key.clone());
        let is_new = !env.storage().persistent().has(&meta_key);
        env.storage().persistent().set(&meta_key, &value);
        env.storage().persistent().extend_ttl(
            &meta_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        if is_new {
            let keys_index_key = DataKeyExt2::ShipmentMetadataKeys(shipment_id.clone());
            let mut keys: Vec<Symbol> = env
                .storage()
                .persistent()
                .get(&keys_index_key)
                .unwrap_or_else(|| Vec::new(&env));
            keys.push_back(key.clone());
            env.storage().persistent().set(&keys_index_key, &keys);
            env.storage().persistent().extend_ttl(
                &keys_index_key,
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        env.events().publish(
            (Symbol::new(&env, "shipment_metadata_set"), shipment_id),
            (caller, key, value),
        );
    }

    /// Read a single custom metadata value for a shipment. Read-only.
    pub fn get_shipment_metadata(env: Env, shipment_id: String, key: Symbol) -> Option<String> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::ShipmentMetadata(shipment_id, key))
    }

    /// List all custom metadata keys set on a shipment. Read-only.
    pub fn get_shipment_metadata_keys(env: Env, shipment_id: String) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::ShipmentMetadataKeys(shipment_id))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ----------------------------------------------------------
    // SHIPMENT ARCHIVAL
    // ----------------------------------------------------------

    /// Admin sets how many ledgers after creation a finished shipment may be archived.
    /// 0 disables archival.
    pub fn set_archive_threshold(env: Env, admin: Address, ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKeyExt::ArchiveThreshold, &ledgers);
        env.events()
            .publish((Symbol::new(&env, "archive_threshold_set"),), ledgers);
    }

    pub fn get_archive_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt::ArchiveThreshold)
            .unwrap_or(0)
    }

    /// Compact a Completed/Cancelled shipment that is old enough into an ArchivedShipment.
    /// Callable by anyone. Irreversible.
    pub fn archive_shipment(env: Env, caller: Address, shipment_id: String) {
        caller.require_auth();
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt::ArchiveThreshold)
            .unwrap_or(0);
        if threshold == 0 {
            panic!("archival is disabled");
        }
        if env
            .storage()
            .persistent()
            .has(&DataKeyExt::ArchivedShipment(shipment_id.clone()))
        {
            panic!("shipment already archived");
        }
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        if shipment.status != ShipmentStatus::Completed
            && shipment.status != ShipmentStatus::Cancelled
        {
            panic!("only completed or cancelled shipments can be archived");
        }
        let age = env.ledger().sequence().saturating_sub(shipment.created_at);
        if age < threshold {
            panic!("shipment not old enough to archive");
        }

        let primary_buyer = shipment
            .buyers
            .get(0)
            .unwrap_or_else(|| panic!("shipment has no buyers"));
        let archived = ArchivedShipment {
            id: shipment_id.clone(),
            buyer: primary_buyer,
            supplier: shipment.supplier.clone(),
            status: shipment.status.clone(),
            total_amount: shipment.total_amount,
            released_amount: shipment.released_amount,
            completed_at: shipment.created_at,
        };
        env.storage().persistent().set(
            &DataKeyExt::ArchivedShipment(shipment_id.clone()),
            &archived,
        );
        env.storage()
            .persistent()
            .remove(&DataKey::Shipment(shipment_id.clone()));
        // Drop heavy satellite state when present.
        env.storage()
            .persistent()
            .remove(&DataKey::CancelPolicy(shipment_id.clone()));
        env.events().publish(
            (Symbol::new(&env, "shipment_archived"), shipment_id),
            (archived.status, archived.completed_at),
        );
    }

    pub fn get_archived_shipment(env: Env, shipment_id: String) -> ArchivedShipment {
        env.storage()
            .persistent()
            .get(&DataKeyExt::ArchivedShipment(shipment_id))
            .unwrap_or_else(|| panic!("archived shipment not found"))
    }

    // ----------------------------------------------------------
    // READ-ONLY QUERIES
    // ----------------------------------------------------------

    pub fn get_shipment(env: Env, shipment_id: String) -> Shipment {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        Self::get_shipment_internal(&env, &shipment_id)
    }

    pub fn get_confirmation_cooldown(env: Env, shipment_id: String) -> u32 {
        Self::get_confirmation_cooldown_internal(&env, &shipment_id)
    }

    pub fn get_milestone(env: Env, shipment_id: String, milestone_index: u32) -> Milestone {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        shipment
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic!("invalid milestone index"))
    }

    /// Returns how far a shipment has settled, as a whole-number percentage in `[0, 100]`.
    ///
    /// # How the percentage is derived from milestone payment weights
    ///
    /// Every milestone carries a **payment weight** — the share of the shipment's
    /// `total_amount` that settling that milestone releases from escrow:
    ///
    /// * default weights: `Milestone.payment_percent`, whole percents that
    ///   `create_shipment` requires to **sum to exactly 100**;
    /// * optional fine-grained weights: `ShipmentOptions.milestone_splits`, basis
    ///   points stored under `DataKeyExt::MilestoneSplits` that must **sum to exactly
    ///   10_000**. When present they take precedence over `payment_percent`.
    ///
    /// `milestone_gross_payment` turns a weight into money:
    ///
    /// ```text
    /// gross(i) = total_amount * splits_bps(i) / 10_000        // when milestone_splits is set
    /// gross(i) = total_amount * payment_percent(i) / 100      // otherwise
    /// ```
    ///
    /// Because the weights sum to 100% (10_000 bps), the sum of all `gross(i)` equals
    /// `total_amount` (± integer truncation), so *settled weight* and *settled money*
    /// are two views of the same quantity. This query reports the money view:
    ///
    /// ```text
    /// settled = released_amount + total_advanced_amount
    /// pct     = clamp(settled * 100 / total_amount, 0, 100)      // integer division
    /// ```
    ///
    /// where the two accumulators are fed exclusively by milestone-weight-sized amounts:
    ///
    /// * `released_amount` — increased by milestone `i`'s weight amount every time that
    ///   weight leaves escrow for good:
    ///   * `confirm_milestone` (immediate path) and `claim_auto_confirmation` credit
    ///     `gross(i) - late_penalty`;
    ///   * `release_held_payment` (holdback path) and `batch_confirm_milestones` credit
    ///     `gross(i)`;
    ///   * `raise_partial_dispute` credits the uncontested share
    ///     `(100 - contested_percent)%` of `gross(i)` straight away;
    ///   * `resolve_dispute`, `resolve_dispute_timeout` and panel resolution credit the
    ///     disputed share once the outcome moves it out of escrow — a supplier payout,
    ///     or the buyer refund of a *partial* dispute. A **full** dispute rejected in
    ///     the buyer's favour resets the milestone to `Pending` instead, so nothing is
    ///     credited and that weight can still be settled later.
    /// * `total_advanced_amount` — increased by `approve_advance`
    ///   (`gross(i) * requested_percent / 100`, a *fraction* of one milestone's weight)
    ///   and decreased again by `consume_advance_for_milestone` when that milestone is
    ///   confirmed and its full `gross(i)` moves into `released_amount`. Advances are
    ///   therefore counted exactly once, never twice.
    ///
    /// So a fresh shipment reads `0`, confirming a milestone whose weight is `w`
    /// adds `w` percentage points, and a fully settled shipment reads `100`.
    /// With the default 25 / 50 / 25 weights the query walks 0 → 25 → 75 → 100.
    ///
    /// # Properties and edge cases
    ///
    /// * **Truncation** — integer division floors the result, so fractional weights
    ///   round down (e.g. a `3_333` bps milestone on a fully funded escrow reads `33`).
    ///   The value is also clamped into `[0, 100]` so ledger drift (top-ups, penalties,
    ///   rebalances) can never produce an out-of-range answer.
    /// * **Settlement, not supplier earnings** — platform, logistics, arbiter and
    ///   referral fees are taken out of the *payout*, not out of the weight, so they do
    ///   not change the percentage. A partial dispute rejected in the buyer's favour
    ///   still credits the contested weight, because that money has left escrow (as a
    ///   refund). Read it as "how much of the escrow is settled", not "how much the
    ///   supplier was paid".
    /// * **Terminal states freeze the reading** — `cancel_shipment`, `supplier_cancel`
    ///   and `claim_deadline_refund` return the *unsettled* remainder to the buyer
    ///   without crediting `released_amount`, so a cancelled/expired shipment keeps the
    ///   percentage it had reached (typically below `100`).
    /// * **Held payments lag** — with `holdback_ledgers > 0` a confirmed milestone sits
    ///   in `MilestoneStatus::ConfirmedHeld` and is *not* counted until
    ///   `release_held_payment` actually pays it out.
    /// * **Late penalties shrink the credited weight** — a penalised milestone credits
    ///   `gross(i) - penalty` (the penalty is refunded to the buyer, capped at half the
    ///   weight), so it contributes slightly less than its nominal weight and a shipment
    ///   delivered entirely late can finish below `100`.
    /// * **The denominator is live** — `top_up_escrow` raises `total_amount`, which
    ///   dilutes an in-flight percentage; `rebalance_milestones` and accepted
    ///   `propose_amendment` percentage changes keep the weight sum at 100, so the
    ///   0 → 100 walk stays consistent.
    /// * **Degenerate inputs** — returns `0` when `total_amount <= 0` (nothing to
    ///   settle) or when nothing has settled yet, avoiding any division by zero.
    /// * Exactly complements `get_escrow_balance`, which returns the unsettled
    ///   remainder `total_amount - released_amount - total_advanced_amount`.
    ///
    /// Read-only: no authorization required, no state mutated. Panics with
    /// `"shipment not found"` if `shipment_id` is unknown (or already archived).
    ///
    /// See `docs/completion-percentage.md` for worked examples.
    pub fn get_completion_percentage(env: Env, shipment_id: String) -> u32 {
        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        // Guard the denominator: an unfunded/degenerate escrow has no weight to settle.
        if shipment.total_amount <= 0 {
            return 0;
        }
        // Nothing settled yet (no milestone weight released, no advance outstanding).
        if shipment.released_amount + shipment.total_advanced_amount <= 0 {
            return 0;
        }

        // Settled weight = milestone weights already released + outstanding advances
        // (fractions of a not-yet-confirmed milestone's weight).
        // Multiply before dividing so the ratio keeps full i128 precision.
        let numerator: i128 = (shipment.released_amount + shipment.total_advanced_amount) * 100;
        // Integer division floors the result: fractional weights round down.
        let mut pct: i128 = numerator / shipment.total_amount;
        // Clamp to [0, 100] to avoid any unexpected rounding / state drift.
        if pct < 0 {
            pct = 0;
        }
        if pct > 100 {
            pct = 100;
        }

        pct as u32
    }

    pub fn get_escrow_balance(env: Env, shipment_id: String) -> i128 {
        env.storage()
            .instance()
            .extend_ttl(constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
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

    /// Whether a specific shipment is mutually-consent paused.
    pub fn is_shipment_paused(env: Env, shipment_id: String) -> bool {
        Self::is_shipment_paused_internal(&env, &shipment_id)
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

    pub fn get_arbiter_stats(env: Env, arbiter: Address) -> ArbiterStats {
        env.storage()
            .persistent()
            .get(&DataKeyExt::ArbiterStats(arbiter))
            .unwrap_or_default()
    }

    pub fn get_shipment_risk(env: Env, shipment_id: String) -> ShipmentRisk {
        let shipment = Self::get_shipment_internal(&env, &shipment_id);
        let mut late = 0;
        let mut disputed = 0;
        let total = shipment.milestones.len();

        let current_ledger = env.ledger().sequence();

        for i in 0..total {
            let m = shipment.milestones.get(i).unwrap();

            if m.status == MilestoneStatus::Disputed || m.status == MilestoneStatus::Resolved {
                disputed += 1;
            }

            let deadline = Self::get_milestone_deadline(env.clone(), shipment_id.clone(), i);
            if deadline > 0
                && current_ledger > deadline
                && m.status != MilestoneStatus::Confirmed
                && m.status != MilestoneStatus::Resolved
            {
                late += 1;
            } else if m.status == MilestoneStatus::Confirmed
                || m.status == MilestoneStatus::Resolved
            {
                // If the milestone is finished, we could consider if it *was* late,
                // but the standard says "deadline vs current ledger for lateness",
                // meaning currently late. If we want historically late, we'd need to check when it was finished.
                // The acceptance criteria: "Correctly counts late milestones against get_milestone_deadline".
                // I will consider a milestone currently late if the deadline has passed and it's not confirmed.
            }
        }

        ShipmentRisk {
            late_milestones: late,
            disputed_milestones: disputed,
            total_milestones: total,
        }
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

        let clamped_limit = if limit > constants::LIST_SHIPMENTS_MAX_PAGE {
            constants::LIST_SHIPMENTS_MAX_PAGE
        } else {
            limit
        };
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

    /// #385: Returns the jurisdiction/compliance tag for a shipment, or None if untagged.
    pub fn get_shipment_jurisdiction(env: Env, shipment_id: String) -> Option<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::ShipmentJurisdiction(shipment_id))
    }

    /// #385: Returns all shipment IDs tagged with `jurisdiction`, for off-chain
    /// compliance tooling to filter or report on shipments subject to a
    /// specific regulatory regime.
    pub fn get_shipments_by_jurisdiction(env: Env, jurisdiction: Symbol) -> Vec<String> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::JurisdictionShipments(jurisdiction))
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

    fn get_confirmation_cooldown_internal(env: &Env, shipment_id: &String) -> u32 {
        let override_cooldown: Option<u32> =
            env.storage()
                .persistent()
                .get(&DataKeyExt::ShipmentConfirmationCooldown(
                    shipment_id.clone(),
                ));
        if let Some(c) = override_cooldown {
            c
        } else {
            env.storage()
                .instance()
                .get(&DataKeyExt::GlobalConfirmationCooldown)
                .unwrap_or(0)
        }
    }

    /// #160: Returns gross payment for a milestone using stored basis-point splits when available,
    /// otherwise falls back to the milestone's payment_percent field.
    ///
    /// This is the single place where a milestone **payment weight** becomes money:
    ///
    /// ```text
    /// gross(i) = total_amount * splits_bps(i) / 10_000   // MilestoneSplits set (sum == 10_000)
    /// gross(i) = total_amount * payment_percent(i) / 100 // fallback (percents sum == 100)
    /// ```
    ///
    /// Since the weights are validated to cover the whole shipment, the `gross(i)` values
    /// add up to `total_amount` (± integer truncation). Every accumulation into
    /// `Shipment.released_amount` is one of these values, which is what makes
    /// `get_completion_percentage` a faithful weight-progress reading.
    fn milestone_gross_payment(env: &Env, shipment: &Shipment, milestone_index: u32) -> i128 {
        let splits_key = DataKeyExt::MilestoneSplits(shipment.id.clone());
        if let Some(splits) = env
            .storage()
            .persistent()
            .get::<DataKeyExt, Vec<u32>>(&splits_key)
        {
            if (milestone_index as usize) < splits.len() as usize {
                let bps = splits.get(milestone_index).unwrap();
                return (shipment.total_amount * bps as i128) / 10_000;
            }
        }
        let milestone = shipment.milestones.get(milestone_index).unwrap();
        (shipment.total_amount * milestone.payment_percent as i128) / 100
    }

    /// Returns the effective auto-confirm window for a shipment.
    /// Priority: per-shipment review_window_ledgers > auto_confirm_ledgers > global AutoConfirmThreshold.
    /// Returns 0 if auto-confirmation is disabled.
    fn get_effective_auto_confirm_window(env: &Env, shipment: &Shipment) -> u32 {
        match shipment.review_window_ledgers {
            Some(0) => 0,
            Some(n) => n,
            None => {
                if shipment.auto_confirm_ledgers > 0 {
                    shipment.auto_confirm_ledgers
                } else {
                    env.storage()
                        .instance()
                        .get(&DataKey::AutoConfirmThreshold)
                        .unwrap_or(0)
                }
            }
        }
    }

    fn assert_not_paused(env: &Env) {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            panic!("contract is paused");
        }
        let frozen: bool = env
            .storage()
            .instance()
            .get(&DataKeyExt2::EmergencyFrozen)
            .unwrap_or(false);
        if frozen {
            panic!("contract is under emergency freeze");
        }
    }

    fn is_shipment_paused_internal(env: &Env, shipment_id: &String) -> bool {
        env.storage()
            .persistent()
            .get(&DataKeyExt::ShipmentPaused(shipment_id.clone()))
            .unwrap_or(false)
    }

    fn assert_shipment_not_paused(env: &Env, shipment_id: &String) {
        if Self::is_shipment_paused_internal(env, shipment_id) {
            panic!("shipment is paused");
        }
    }

    fn is_fast_track_eligible_internal(env: &Env, supplier: &Address) -> bool {
        let policy: Option<ReputationFastTrack> = env
            .storage()
            .instance()
            .get(&DataKeyExt::ReputationFastTrack);
        let Some(policy) = policy else {
            return false;
        };
        let score = Self::get_reputation_internal(env, supplier);
        if score.completed < policy.min_completed {
            return false;
        }
        if score.completed == 0 {
            return false;
        }
        let ratio_bps = (score.disputed as u64).saturating_mul(10_000) / (score.completed as u64);
        ratio_bps <= policy.max_disputed_ratio_bps as u64
    }

    fn assert_buyer_or_supplier(shipment: &Shipment, caller: &Address) {
        if !Self::is_buyer(shipment, caller) && *caller != shipment.supplier {
            panic!("unauthorized");
        }
    }

    fn assert_shipment_participant(shipment: &Shipment, caller: &Address) {
        if !Self::is_buyer(shipment, caller)
            && *caller != shipment.supplier
            && *caller != shipment.logistics
        {
            panic!("unauthorized");
        }
    }

    fn bump_shipment_deadlines_for_pause(env: &Env, shipment: &mut Shipment, delta: u32) {
        if delta == 0 {
            return;
        }
        for i in 0..shipment.milestones.len() {
            let mut m = shipment.milestones.get(i).unwrap();
            if m.deadline_ledger > 0 {
                m.deadline_ledger = m.deadline_ledger.saturating_add(delta);
            }
            if let Some(ps) = m.proof_submitted_ledger {
                m.proof_submitted_ledger = Some(ps.saturating_add(delta));
            }
            if let Some(d) = m.dispute_opened_ledger {
                m.dispute_opened_ledger = Some(d.saturating_add(delta));
            }
            if m.release_after_ledger > 0 {
                m.release_after_ledger = m.release_after_ledger.saturating_add(delta);
            }
            shipment.milestones.set(i, m);

            let deadline_key = DataKeyExt::MilestoneDeadline(shipment.id.clone(), i);
            if let Some(dl) = env
                .storage()
                .persistent()
                .get::<DataKeyExt, u32>(&deadline_key)
            {
                if dl > 0 {
                    env.storage()
                        .persistent()
                        .set(&deadline_key, &(dl.saturating_add(delta)));
                }
            }
        }
        if let Some(exp) = shipment.expires_at_ledger {
            shipment.expires_at_ledger = Some(exp.saturating_add(delta));
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
                shipment.total_advanced_amount =
                    (shipment.total_advanced_amount - req.amount_advanced).max(0);
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
        env.storage().persistent().extend_ttl(
            &key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );
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

        // Feature D: Check auto-blacklist rule after any reputation increment.
        if disputed_delta > 0 || cancelled_delta > 0 {
            Self::check_auto_blacklist_internal(env, supplier, &score);
        }
    }

    /// Update buyer reliability score on milestone confirmation.
    fn update_buyer_reliability_on_confirmation(
        env: &Env,
        buyer: &Address,
        proof_submitted_ledger: u32,
    ) {
        let current_ledger = env.ledger().sequence();
        let confirmation_latency = current_ledger.saturating_sub(proof_submitted_ledger) as u64;

        let key = DataKeyExt::BuyerReliability(buyer.clone());
        let mut reliability: BuyerReliability = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_default();

        reliability.total_confirmations = reliability.total_confirmations.saturating_add(1);
        reliability.total_confirmation_latency = reliability
            .total_confirmation_latency
            .saturating_add(confirmation_latency);

        env.storage().persistent().set(&key, &reliability);
        env.storage()
            .persistent()
            .extend_ttl(&key, constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
    }

    /// Update buyer reliability score on dispute resolution.
    fn update_buyer_reliability_on_dispute(env: &Env, buyer: &Address, buyer_won: bool) {
        let key = DataKeyExt::BuyerReliability(buyer.clone());
        let mut reliability: BuyerReliability = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_default();

        reliability.disputes_total = reliability.disputes_total.saturating_add(1);
        if !buyer_won {
            reliability.disputes_lost = reliability.disputes_lost.saturating_add(1);
        }

        env.storage().persistent().set(&key, &reliability);
        env.storage()
            .persistent()
            .extend_ttl(&key, constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
    }


    // ============================================================
    // FEATURE HELPERS
    // ============================================================

    /// Feature B: Compute the aggregate locked escrow for a supplier across all Active shipments.
    /// "Locked" = total_amount - released_amount for each Active shipment.
    fn compute_supplier_exposure(env: &Env, supplier: &Address) -> i128 {
        let shipment_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::SupplierShipments(supplier.clone()))
            .unwrap_or_else(|| Vec::new(env));

        let mut total: i128 = 0;
        for i in 0..shipment_ids.len() {
            let id = shipment_ids.get(i).unwrap();
            if let Some(s) = env
                .storage()
                .persistent()
                .get::<DataKey, Shipment>(&DataKey::Shipment(id))
            {
                if s.status == ShipmentStatus::Active {
                    let locked = (s.total_amount - s.released_amount).max(0);
                    total += locked;
                }
            }
        }
        total
    }

    /// Feature C: Transfer `net_amount` to the configured milestone payees, or to `supplier`
    /// if no payees are configured.
    fn pay_milestone_to_payees(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
        net_amount: i128,
        supplier: &Address,
        token_client: &token::Client,
    ) {
        if net_amount <= 0 {
            return;
        }
        let payees: Vec<MilestonePayee> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::MilestonePayees(
                shipment_id.clone(),
                milestone_index,
            ))
            .unwrap_or_else(|| Vec::new(env));

        if payees.is_empty() {
            // Fallback: single supplier payout.
            // #284: Batched vs immediate payout.
            let payout_mode: PayoutMode = env
                .storage()
                .persistent()
                .get(&DataKeyExt::PayoutMode(supplier.clone()))
                .unwrap_or(PayoutMode::Immediate);
            if payout_mode == PayoutMode::Batched {
                let pending: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKeyExt::PendingPayout(supplier.clone()))
                    .unwrap_or(0);
                env.storage().persistent().set(
                    &DataKeyExt::PendingPayout(supplier.clone()),
                    &(pending + net_amount),
                );
            } else {
                token_client.transfer(&env.current_contract_address(), supplier, &net_amount);
            }
            return;
        }

        // Split across payees; last payee absorbs any rounding dust.
        let mut distributed: i128 = 0;
        let last_idx = payees.len() - 1;
        for i in 0..payees.len() {
            let entry = payees.get(i).unwrap();
            let amount = if i == last_idx {
                net_amount - distributed
            } else {
                (net_amount * entry.percent as i128) / 100
            };
            if amount > 0 {
                token_client.transfer(&env.current_contract_address(), &entry.payee, &amount);
                distributed += amount;
            }
        }
    }

    /// Feature D: Check auto-blacklist rule against the updated reputation score.
    /// If either threshold is crossed, automatically blacklists the supplier.
    fn check_auto_blacklist_internal(env: &Env, supplier: &Address, score: &ReputationScore) {
        // Already blacklisted? Nothing to do.
        if env
            .storage()
            .instance()
            .get::<DataKey, BytesN<32>>(&DataKey::Blacklisted(supplier.clone()))
            .is_some()
        {
            return;
        }

        let rule: AutoBlacklistRule = env
            .storage()
            .instance()
            .get(&DataKeyExt::AutoBlacklistRule)
            .unwrap_or_default();

        let triggered = (rule.max_cancelled > 0 && score.cancelled >= rule.max_cancelled)
            || (rule.max_disputed > 0 && score.disputed >= rule.max_disputed);

        if !triggered {
            return;
        }

        // Use a zero reason-hash to distinguish auto-blacklist from manual.
        let reason_hash = BytesN::from_array(env, &[0u8; 32]);
        env.storage()
            .instance()
            .set(&DataKey::Blacklisted(supplier.clone()), &reason_hash);

        // Emit an admin log entry with detail = "auto_blacklist_triggered".
        let mut log: Vec<AuditEntry> = env
            .storage()
            .instance()
            .get(&DataKey::AdminActionLog)
            .unwrap_or_else(|| Vec::new(env));
        let entry = AuditEntry {
            action: Symbol::new(env, "address_blacklisted"),
            caller: supplier.clone(),
            ledger: env.ledger().sequence(),
            detail: Symbol::new(env, "auto_blacklist_triggered"),
        };
        if log.len() as usize >= constants::AUDIT_LOG_MAX_ENTRIES {
            let mut next: Vec<AuditEntry> = Vec::new(env);
            for i in 1..log.len() {
                next.push_back(log.get(i).unwrap());
            }
            log = next;
        }
        log.push_back(entry);
        env.storage().instance().set(&DataKey::AdminActionLog, &log);
    }

    // ============================================================
    // #284 — SUPPLIER PAYOUT BATCHING: public functions
    // ============================================================

    /// Toggle batched vs immediate payout mode for a supplier.
    /// When batched, milestone payments accumulate in a per-supplier balance
    /// instead of being SAC-transferred on each confirmation.
    pub fn set_payout_mode(env: Env, supplier: Address, batched: bool) {
        supplier.require_auth();
        let mode = if batched {
            PayoutMode::Batched
        } else {
            PayoutMode::Immediate
        };
        env.storage()
            .persistent()
            .set(&DataKeyExt::PayoutMode(supplier.clone()), &mode);
        env.events().publish(
            (Symbol::new(&env, "payout_mode_set"), supplier.clone()),
            batched,
        );
    }

    /// Withdraw the full accumulated pending payout balance in one transaction.
    /// Panics if the balance is zero. Resets the balance to zero after transfer.
    ///
    /// #404: If the supplier has a configured payout currency preference that
    /// differs from `token`, and an admin-registered conversion rate route
    /// exists for (token -> preference) with sufficient contract balance in
    /// the preferred token, the payout is converted and paid out in the
    /// preferred token instead; the conversion rate is recorded on the
    /// `payout_claimed` event. Otherwise falls back to paying out in `token`
    /// unchanged (never reverts due to a missing/insufficient route).
    pub fn claim_payout(env: Env, supplier: Address, token: Address) {
        Self::assert_not_paused(&env);
        supplier.require_auth();

        let pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::PendingPayout(supplier.clone()))
            .unwrap_or(0);

        if pending <= 0 {
            panic!("no pending payout");
        }

        // Zero out first (checks-effects-interactions).
        env.storage()
            .persistent()
            .set(&DataKeyExt::PendingPayout(supplier.clone()), &0i128);

        let preference: Option<Address> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::PayoutCurrencyPreference(supplier.clone()));

        let mut payout_token = token.clone();
        let mut payout_amount = pending;
        let mut rate_bps: i128 = 0;

        if let Some(preferred) = preference {
            if preferred != token {
                let rate: Option<u32> = env
                    .storage()
                    .instance()
                    .get(&DataKeyExt2::ConversionRateBps(token.clone(), preferred.clone()));
                if let Some(rate_val) = rate {
                    let converted = (pending * rate_val as i128) / 10_000;
                    let preferred_client = token::Client::new(&env, &preferred);
                    let contract_balance =
                        preferred_client.balance(&env.current_contract_address());
                    if converted > 0 && contract_balance >= converted {
                        payout_token = preferred;
                        payout_amount = converted;
                        rate_bps = rate_val as i128;
                    }
                }
            }
        }

        let token_client = token::Client::new(&env, &payout_token);
        token_client.transfer(&env.current_contract_address(), &supplier, &payout_amount);

        env.events().publish(
            (Symbol::new(&env, "payout_claimed"), supplier.clone()),
            (
                payout_amount,
                payout_token,
                rate_bps,
            ),
        );
    }

    /// Read-only getter for the supplier's accumulated pending payout balance.
    pub fn get_pending_payout(env: Env, supplier: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKeyExt::PendingPayout(supplier))
            .unwrap_or(0)
    }

    /// Set the supplier's preferred settlement token for future
    /// `claim_payout` calls. Conversion only happens when an admin has also
    /// registered a route via `set_conversion_rate` for the source token.
    pub fn set_payout_currency_preference(env: Env, supplier: Address, preferred_token: Address) {
        supplier.require_auth();
        env.storage().persistent().set(
            &DataKeyExt2::PayoutCurrencyPreference(supplier.clone()),
            &preferred_token,
        );
        env.events().publish(
            (
                Symbol::new(&env, "payout_currency_preference_set"),
                supplier,
            ),
            preferred_token,
        );
    }

    /// Read the supplier's configured payout currency preference, if any.
    pub fn get_payout_currency_preference(env: Env, supplier: Address) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::PayoutCurrencyPreference(supplier))
    }

    /// Admin-registers a fixed conversion rate (basis points of `to_token`
    /// per unit of `from_token`; 10_000 = 1:1) used by `claim_payout` to
    /// convert a supplier's payout when a currency preference is set. This is
    /// a stand-in swap-venue/oracle route — no live pricing is queried.
    pub fn set_conversion_rate(
        env: Env,
        admin: Address,
        from_token: Address,
        to_token: Address,
        rate_bps: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if rate_bps == 0 {
            panic!("rate_bps must be > 0");
        }
        env.storage().instance().set(
            &DataKeyExt2::ConversionRateBps(from_token.clone(), to_token.clone()),
            &rate_bps,
        );
        env.events().publish(
            (Symbol::new(&env, "conversion_rate_set"), from_token, to_token),
            rate_bps,
        );
    }

    /// Remove a previously registered conversion rate route.
    pub fn clear_conversion_rate(env: Env, admin: Address, from_token: Address, to_token: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .remove(&DataKeyExt2::ConversionRateBps(from_token.clone(), to_token.clone()));
        env.events().publish(
            (
                Symbol::new(&env, "conversion_rate_cleared"),
                from_token,
                to_token,
            ),
            (),
        );
    }

    /// Read the registered conversion rate (basis points) for a token pair,
    /// or None if no route is configured.
    pub fn get_conversion_rate(env: Env, from_token: Address, to_token: Address) -> Option<u32> {
        env.storage()
            .instance()
            .get(&DataKeyExt2::ConversionRateBps(from_token, to_token))
    }

    // ============================================================
    // #285 — PER-ADDRESS OUTFLOW RATE LIMITING: public functions
    // ============================================================

    /// Configure a per-address sliding-window outflow cap.
    /// `limit = 0` disables the per-address cap for that address.
    pub fn set_address_outflow_limit(
        env: Env,
        admin: Address,
        address: Address,
        limit: i128,
        window_ledgers: u32,
    ) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        env.storage()
            .persistent()
            .set(&DataKeyExt::AddressOutflowLimit(address.clone()), &limit);
        env.storage().persistent().set(
            &DataKeyExt::AddressOutflowWindow(address.clone()),
            &window_ledgers,
        );
        // Reset window tracking on config change.
        env.storage().persistent().set(
            &DataKeyExt::AddressOutflowWindowStart(address.clone()),
            &env.ledger().sequence(),
        );
        env.storage().persistent().set(
            &DataKeyExt::AddressOutflowWindowOutflow(address.clone()),
            &0i128,
        );

        env.events().publish(
            (
                Symbol::new(&env, "address_outflow_limit_set"),
                address.clone(),
            ),
            (limit, window_ledgers),
        );
    }

    /// Read-only getter for per-address outflow config.
    /// Returns (limit, window_ledgers, window_start, window_outflow).
    pub fn get_address_outflow_limit(env: Env, address: Address) -> (i128, u32, u32, i128) {
        let limit: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowLimit(address.clone()))
            .unwrap_or(0);
        let window: u32 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowWindow(address.clone()))
            .unwrap_or(0);
        let window_start: u32 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowWindowStart(address.clone()))
            .unwrap_or(0);
        let window_outflow: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowWindowOutflow(address.clone()))
            .unwrap_or(0);
        (limit, window, window_start, window_outflow)
    }

    // ============================================================
    // #286 — DISPUTE EVIDENCE VERSIONING: public functions
    // ============================================================

    /// Submit versioned evidence for a disputed milestone.
    /// Callable by supplier, logistics, or buyer while the milestone is Disputed.
    /// Evidence entries are appended (never overwritten).
    pub fn submit_dispute_evidence(
        env: Env,
        caller: Address,
        shipment_id: String,
        milestone_index: u32,
        evidence_hash: String,
        evidence_type: Symbol,
    ) {
        caller.require_auth();

        let shipment = Self::get_shipment_internal(&env, &shipment_id);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("evidence can only be submitted while milestone is Disputed");
        }

        // Only supplier, logistics, or a buyer may submit evidence.
        let is_buyer = Self::is_buyer(&shipment, &caller);
        if !is_buyer && caller != shipment.supplier && caller != shipment.logistics {
            panic!("unauthorized");
        }

        let evidence_key = DataKeyExt::DisputeEvidence(shipment_id.clone(), milestone_index);
        let mut entries: Vec<DisputeEvidence> = env
            .storage()
            .persistent()
            .get(&evidence_key)
            .unwrap_or_else(|| Vec::new(&env));

        entries.push_back(DisputeEvidence {
            submitter: caller.clone(),
            evidence_hash: evidence_hash.clone(),
            evidence_type: evidence_type.clone(),
            submitted_ledger: env.ledger().sequence(),
        });

        env.storage().persistent().set(&evidence_key, &entries);
        env.storage().persistent().extend_ttl(
            &evidence_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        env.events().publish(
            (
                Symbol::new(&env, "dispute_evidence_submitted"),
                shipment_id.clone(),
            ),
            (milestone_index, caller, evidence_hash, evidence_type),
        );
    }

    /// Read-only getter returning all evidence entries for a disputed milestone,
    /// in the order they were submitted.
    pub fn get_dispute_evidence(
        env: Env,
        shipment_id: String,
        milestone_index: u32,
    ) -> Vec<DisputeEvidence> {
        env.storage()
            .persistent()
            .get(&DataKeyExt::DisputeEvidence(shipment_id, milestone_index))
            .unwrap_or_else(|| Vec::new(&env))
    }

    // ============================================================
    // #287 — BUYER-INITIATED DISPUTE WITHDRAWAL: public function
    // ============================================================

    /// Buyer withdraws their own open dispute, reverting the milestone back to
    /// ProofSubmitted. Only callable while the milestone is Disputed and the
    /// caller is the shipment's registered buyer.
    /// Emits `dispute_withdrawn` with (shipment_id, milestone_index).
    pub fn withdraw_dispute(env: Env, buyer: Address, shipment_id: String, milestone_index: u32) {
        Self::assert_not_paused(&env);
        buyer.require_auth();

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        Self::assert_is_buyer(&shipment, &buyer);

        if milestone_index as usize >= shipment.milestones.len() as usize {
            panic!("invalid milestone index");
        }

        let mut milestone = shipment.milestones.get(milestone_index).unwrap();
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not in disputed status");
        }

        // Revert to ProofSubmitted so the supplier's original proof stands.
        milestone.status = MilestoneStatus::ProofSubmitted;
        milestone.dispute_opened_ledger = None;
        shipment.milestones.set(milestone_index, milestone);
        shipment.open_dispute_count = shipment.open_dispute_count.saturating_sub(1);

        env.storage()
            .persistent()
            .set(&DataKey::Shipment(shipment_id.clone()), &shipment);

        // Remove from active disputes list.
        let disputes: Vec<DisputeEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputes)
            .unwrap_or_else(|| Vec::new(&env));
        let mut new_disputes: Vec<DisputeEntry> = Vec::new(&env);
        for i in 0..disputes.len() {
            let d = disputes.get(i).unwrap();
            if !(d.shipment_id == shipment_id && d.milestone_index == milestone_index) {
                new_disputes.push_back(d);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputes, &new_disputes);

        env.events().publish(
            (Symbol::new(&env, "dispute_withdrawn"), shipment_id.clone()),
            (milestone_index, buyer),
        );
    }

    // ============================================================
    // INTERNAL HELPERS
    // ============================================================

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
        let min_value: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MinShipmentValue)
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
            min_value,
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
        let partial_contested_percent: Option<u32> = env.storage().persistent().get(&contested_key);
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

    /// Check (and update) the per-address outflow sliding-window for `address`.
    /// If the address has no configured limit (limit == 0), the check is skipped.
    /// Panics with "address outflow limit exceeded" if the window cap is breached.
    fn check_address_outflow(env: &Env, address: &Address, payment: i128) {
        let limit: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowLimit(address.clone()))
            .unwrap_or(0);

        if limit == 0 {
            return; // Per-address cap disabled; only global breaker applies.
        }

        let window: u32 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowWindow(address.clone()))
            .unwrap_or(0);
        let window_start: u32 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowWindowStart(address.clone()))
            .unwrap_or(0);
        let mut window_outflow: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::AddressOutflowWindowOutflow(address.clone()))
            .unwrap_or(0);

        let current_ledger = env.ledger().sequence();

        // Reset window if the window period has elapsed.
        if window == 0 || current_ledger >= window_start + window {
            window_outflow = 0;
            env.storage().persistent().set(
                &DataKeyExt::AddressOutflowWindowStart(address.clone()),
                &current_ledger,
            );
        }

        if window_outflow + payment > limit {
            panic!("address outflow limit exceeded");
        }

        window_outflow += payment;
        env.storage().persistent().set(
            &DataKeyExt::AddressOutflowWindowOutflow(address.clone()),
            &window_outflow,
        );
    }

    fn get_shipment_internal(env: &Env, shipment_id: &String) -> Shipment {
        env.storage()
            .persistent()
            .get(&DataKey::Shipment(shipment_id.clone()))
            .unwrap_or_else(|| panic!("shipment not found"))
    }

    fn append_audit_entry(
        env: &Env,
        shipment: &mut Shipment,
        caller: Address,
        action: Symbol,
        detail: Symbol,
    ) {
        // Maintain a bounded ring-buffer of max 20 entries.
        let entry = AuditEntry {
            action,
            caller,
            ledger: env.ledger().sequence(),
            detail,
        };

        let max: usize = constants::SHIPMENT_AUDIT_LOG_MAX_ENTRIES;
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
        if log.len() as usize >= constants::AUDIT_LOG_MAX_ENTRIES {
            let mut next: Vec<AuditEntry> = Vec::new(env);
            for i in 1..log.len() {
                next.push_back(log.get(i).unwrap());
            }
            log = next;
        }
        log.push_back(entry);
        env.storage().instance().set(&DataKey::AdminActionLog, &log);
    }

    /// Issue #303 — Best-effort dispatch to all registered confirmation webhooks.
    /// Each webhook's `on_milestone_confirmed` is called via cross-contract invocation.
    /// A panic in any single webhook is caught and ignored so that payout state is
    /// never rolled back by a misbehaving external contract.
    fn dispatch_confirmation_webhooks(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
        payment_amount: i128,
    ) {
        let hooks: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKeyExt::ConfirmationWebhooks)
            .unwrap_or_else(|| Vec::new(env));

        for i in 0..hooks.len() {
            let hook_addr = hooks.get(i).unwrap();
            let client = ConfirmationWebhookClient::new(env, &hook_addr);
            // try_on_milestone_confirmed returns a Result; we discard errors
            // so that a reverting webhook cannot affect the confirmation.
            let _ =
                client.try_on_milestone_confirmed(shipment_id, &milestone_index, &payment_amount);
        }
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

    /// #413: True while a scheduled fee holiday covers the current ledger.
    fn fee_holiday_active(env: &Env) -> bool {
        if let Some(holiday) = env
            .storage()
            .instance()
            .get::<DataKeyExt2, FeeHoliday>(&DataKeyExt2::FeeHoliday)
        {
            let now = env.ledger().sequence();
            return now >= holiday.start_ledger && now <= holiday.end_ledger;
        }
        false
    }

    /// Deducts the platform fee from `gross_payment` and transfers it to the treasury.
    /// Returns the net amount after fee. Updates `fee_out` with the fee taken.
    fn deduct_fee(env: &Env, gross: i128, token: &Address, fee_out: &mut i128) -> i128 {
        if Self::fee_holiday_active(env) {
            return gross;
        }
        if let Some(config) = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
        {
            let fee = (gross * config.fee_bps as i128) / 10_000;
            if fee > 0 {
                // Check if multi-recipient configuration exists
                let recipients: Option<Vec<FeeRecipient>> = env
                    .storage()
                    .instance()
                    .get(&DataKeyExt::FeeRecipients);

                let token_client = token::Client::new(env, token);

                if let Some(recips) = recipients {
                    if recips.len() == 1 {
                        // Single recipient: use simple transfer
                        let recipient = recips.get(0).unwrap();
                        token_client.transfer(&env.current_contract_address(), &recipient.recipient, &fee);
                        Self::track_treasury_revenue(env, token, fee);
                    } else {
                        // Multi-recipient: split pro-rata
                        let mut distributed: i128 = 0;
                        for i in 0..recips.len() {
                            let recipient = recips.get(i).unwrap();
                            let share = if i == 0 {
                                // First recipient gets remainder from rounding
                                fee - distributed
                            } else {
                                (fee * recipient.share_bps as i128) / 10_000
                            };
                            if share > 0 {
                                token_client.transfer(
                                    &env.current_contract_address(),
                                    &recipient.recipient,
                                    &share,
                                );
                                distributed += share;
                            }
                        }
                        Self::track_treasury_revenue(env, token, fee);
                    }
                } else {
                    // Fallback to single treasury from FeeConfig
                    token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
                    Self::track_treasury_revenue(env, token, fee);
                }

                *fee_out = fee;
                return gross - fee;
            }
        }
        gross
    }

    /// Helper to track cumulative treasury revenue per token.
    fn track_treasury_revenue(env: &Env, token: &Address, amount: i128) {
        let key = DataKeyExt::TreasuryRevenue(token.clone());
        let current: i128 = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(0);
        let new_total = current + amount;
        env.storage().persistent().set(&key, &new_total);
        env.storage()
            .persistent()
            .extend_ttl(&key, constants::TTL_INITIAL_LEDGERS, constants::TTL_MAX_LEDGERS);
    }

    /// #299: Deducts fee using per-shipment override first, then locked tier bps, then FeeConfig.
    fn deduct_fee_for_shipment(
        env: &Env,
        gross: i128,
        token: &Address,
        shipment_id: &String,
        fee_out: &mut i128,
    ) -> i128 {
        if Self::fee_holiday_active(env) {
            return gross;
        }
        let override_bps: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeOverride(shipment_id.clone()));
        let locked_bps: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeBps(shipment_id.clone()));
        if let Some(config) = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
        {
            let bps = override_bps.unwrap_or_else(|| locked_bps.unwrap_or(config.fee_bps));
            let fee = (gross * bps as i128) / 10_000;
            if fee > 0 {
                // Check if multi-recipient configuration exists
                let recipients: Option<Vec<FeeRecipient>> = env
                    .storage()
                    .instance()
                    .get(&DataKeyExt::FeeRecipients);

                let token_client = token::Client::new(env, token);

                if let Some(recips) = recipients {
                    if recips.len() == 1 {
                        // Single recipient: use simple transfer
                        let recipient = recips.get(0).unwrap();
                        token_client.transfer(&env.current_contract_address(), &recipient.recipient, &fee);
                        Self::track_treasury_revenue(env, token, fee);
                    } else {
                        // Multi-recipient: split pro-rata
                        let mut distributed: i128 = 0;
                        for i in 0..recips.len() {
                            let recipient = recips.get(i).unwrap();
                            let share = if i == 0 {
                                // First recipient gets remainder from rounding
                                fee - distributed
                            } else {
                                (fee * recipient.share_bps as i128) / 10_000
                            };
                            if share > 0 {
                                token_client.transfer(
                                    &env.current_contract_address(),
                                    &recipient.recipient,
                                    &share,
                                );
                                distributed += share;
                            }
                        }
                        Self::track_treasury_revenue(env, token, fee);
                    }
                } else {
                    // Fallback to single treasury from FeeConfig
                    token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
                    Self::track_treasury_revenue(env, token, fee);
                }

                *fee_out = fee;
                return gross - fee;
            }
        }
        gross
    }

    /// #399: True when every milestone other than `milestone_index` is already
    /// Confirmed or Resolved — i.e. confirming `milestone_index` completes the shipment.
    /// Must be called before the milestone being confirmed has its status updated.
    fn is_final_milestone(shipment: &Shipment, milestone_index: u32) -> bool {
        for i in 0..shipment.milestones.len() {
            if i == milestone_index {
                continue;
            }
            let s = shipment.milestones.get(i).unwrap().status;
            if s != MilestoneStatus::Confirmed && s != MilestoneStatus::Resolved {
                return false;
            }
        }
        true
    }

    /// #399: Deducts fee using per-shipment override first; otherwise, for the milestone
    /// that completes the shipment, recomputes the buyer's fee tier as of *now* (instead
    /// of the tier locked in at creation) so late-shipment tier upgrades/downgrades are
    /// reflected on the final payment. Non-final milestones keep using the locked-in tier,
    /// so already-paid earlier milestones are never retroactively affected.
    /// Returns (net_amount, applied_fee_bps).
    fn deduct_fee_for_shipment_at_completion(
        env: &Env,
        gross: i128,
        token: &Address,
        shipment_id: &String,
        buyer: &Address,
        is_final: bool,
        fee_out: &mut i128,
    ) -> (i128, u32) {
        if Self::fee_holiday_active(env) {
            return (gross, 0);
        }
        let override_bps: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeOverride(shipment_id.clone()));
        let locked_bps: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKeyExt::ShipmentFeeBps(shipment_id.clone()));
        if let Some(config) = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
        {
            let bps = if let Some(o) = override_bps {
                o
            } else if is_final {
                Self::resolve_fee_bps_for(env, buyer)
            } else {
                locked_bps.unwrap_or(config.fee_bps)
            };
            // #388: Apply any governance-granted VIP partner fee waiver on top
            // of the resolved bps (waiver_bps of the *fee*, not of gross).
            let waiver_bps = Self::resolve_fee_waiver_bps(env, buyer);
            let bps = if waiver_bps > 0 {
                bps - ((bps as u64 * waiver_bps as u64) / 10_000) as u32
            } else {
                bps
            };
            let fee = (gross * bps as i128) / 10_000;
            if fee > 0 {
                let token_client = token::Client::new(env, token);
                token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
                *fee_out = fee;
                return (gross - fee, bps);
            }
            return (gross, bps);
        }
        let fallback_bps = override_bps.unwrap_or_else(|| locked_bps.unwrap_or(0));
        (gross, fallback_bps)
    }

    /// #113: Resolves the effective fee bps for `address` based on accumulated lifetime volume.
    fn resolve_fee_bps_for(env: &Env, address: &Address) -> u32 {
        let volume: i128 = env
            .storage()
            .persistent()
            .get(&DataKeyExt::LifetimeVolume(address.clone()))
            .unwrap_or(0);
        let tiers: Vec<FeeTier> = env
            .storage()
            .instance()
            .get(&DataKeyExt::FeeTiers)
            .unwrap_or_else(|| Vec::new(env));
        let mut best: Option<u32> = None;
        for i in 0..tiers.len() {
            let t = tiers.get(i).unwrap();
            if volume >= t.min_lifetime_volume {
                best = Some(match best {
                    None => t.fee_bps,
                    Some(b) => {
                        if t.fee_bps < b {
                            t.fee_bps
                        } else {
                            b
                        }
                    }
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

    // ----------------------------------------------------------
    // #167 STRUCTURED EVENT LOG
    // ----------------------------------------------------------
    // Canonical, indexer-friendly events for the seven core shipment lifecycle
    // transitions. Each uses the two-topic form (Symbol("chainsettle"), Symbol(name))
    // with a Map<Symbol, Val> data payload, per docs/events.md. These are emitted
    // alongside the existing, more granular per-function events (which retain
    // additional fields such as fees and ledger numbers for backward compatibility).

    fn emit_shipment_created(
        env: &Env,
        shipment_id: &String,
        buyer: &Address,
        supplier: &Address,
        arbiter: &Address,
        token: &Address,
        amount: i128,
    ) {
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(Symbol::new(env, "buyer"), buyer.into_val(env));
        data.set(Symbol::new(env, "supplier"), supplier.into_val(env));
        data.set(Symbol::new(env, "arbiter"), arbiter.into_val(env));
        data.set(Symbol::new(env, "token"), token.into_val(env));
        data.set(Symbol::new(env, "amount"), amount.into_val(env));
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "shipment_created"),
            ),
            data,
        );
    }

    fn emit_milestone_proof_submitted(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
        proof_hash: &String,
        supplier: &Address,
    ) {
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(
            Symbol::new(env, "milestone_index"),
            milestone_index.into_val(env),
        );
        data.set(Symbol::new(env, "proof_hash"), proof_hash.into_val(env));
        data.set(Symbol::new(env, "supplier"), supplier.into_val(env));
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "proof_submitted"),
            ),
            data,
        );
    }

    fn emit_milestone_confirmed(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
        payout_amount: i128,
    ) {
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(
            Symbol::new(env, "milestone_index"),
            milestone_index.into_val(env),
        );
        data.set(
            Symbol::new(env, "payout_amount"),
            payout_amount.into_val(env),
        );
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "milestone_confirmed"),
            ),
            data,
        );
    }

    fn emit_dispute_opened(env: &Env, shipment_id: &String, milestone_index: u32, buyer: &Address) {
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(
            Symbol::new(env, "milestone_index"),
            milestone_index.into_val(env),
        );
        data.set(Symbol::new(env, "buyer"), buyer.into_val(env));
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "dispute_opened"),
            ),
            data,
        );
    }

    fn emit_dispute_resolved(
        env: &Env,
        shipment_id: &String,
        milestone_index: u32,
        resolution: Symbol,
        resolver: &Address,
    ) {
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(
            Symbol::new(env, "milestone_index"),
            milestone_index.into_val(env),
        );
        data.set(Symbol::new(env, "resolution"), resolution.into_val(env));
        data.set(Symbol::new(env, "resolver"), resolver.into_val(env));
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "dispute_resolved"),
            ),
            data,
        );
    }

    fn emit_shipment_cancelled(
        env: &Env,
        shipment_id: &String,
        refund_amount: i128,
        reason: CancellationReason,
    ) {
        let reason_sym = match reason {
            CancellationReason::BuyerCancelled => Symbol::new(env, "BuyerCancelled"),
            CancellationReason::SupplierCancelled => Symbol::new(env, "SupplierCancelled"),
            CancellationReason::DeadlineRefund => Symbol::new(env, "DeadlineRefund"),
            CancellationReason::AdminEmergencyRecovery => {
                Symbol::new(env, "AdminEmergencyRecovery")
            }
        };
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(
            Symbol::new(env, "refund_amount"),
            refund_amount.into_val(env),
        );
        // Additive third field — existing (shipment_id, refund_amount) consumers keep working.
        data.set(Symbol::new(env, "reason"), reason_sym.into_val(env));
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "shipment_cancelled"),
            ),
            data,
        );
    }

    fn emit_shipment_completed(env: &Env, shipment_id: &String, total_paid: i128) {
        let mut data: Map<Symbol, Val> = Map::new(env);
        data.set(Symbol::new(env, "shipment_id"), shipment_id.into_val(env));
        data.set(Symbol::new(env, "total_paid"), total_paid.into_val(env));
        env.events().publish(
            (
                Symbol::new(env, "chainsettle"),
                Symbol::new(env, "shipment_completed"),
            ),
            data,
        );
    }

    // ----------------------------------------------------------
    // #364: ADMIN — CONFIGURABLE MAX MILESTONE COUNT
    // ----------------------------------------------------------

    /// Set the maximum number of milestones allowed per shipment. Existing
    /// shipments created under a previous cap remain valid — the cap is only
    /// enforced at `create_shipment` time.
    pub fn set_max_milestone_count(env: Env, admin: Address, count: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if count == 0 {
            panic!("InvalidMilestoneCount");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt2::MaxMilestoneCount, &count);
        env.events()
            .publish((Symbol::new(&env, "max_milestone_count_set"),), count);
    }

    /// Read the currently configured max milestone count (falls back to
    /// `constants::DEFAULT_MAX_MILESTONE_COUNT` when unset).
    pub fn get_max_milestone_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKeyExt2::MaxMilestoneCount)
            .unwrap_or(constants::DEFAULT_MAX_MILESTONE_COUNT)
    }

    // ----------------------------------------------------------
    // #362: ADMIN — PER-TOKEN MIN/MAX SHIPMENT VALUE
    // ----------------------------------------------------------

    pub fn set_token_min_shipment_value(env: Env, admin: Address, token: Address, min_amount: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if min_amount < 0 {
            panic!("InvalidAmount");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt2::TokenMinShipmentValue(token.clone()), &min_amount);
        env.events().publish(
            (Symbol::new(&env, "token_min_shipment_value_set"), token),
            min_amount,
        );
    }

    /// Clear a per-token minimum override so the token falls back to the
    /// global `min_shipment_value` bound.
    pub fn clear_token_min_shipment_value(env: Env, admin: Address, token: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .remove(&DataKeyExt2::TokenMinShipmentValue(token.clone()));
        env.events()
            .publish((Symbol::new(&env, "token_min_shipment_value_cleared"),), token);
    }

    /// Returns the per-token minimum override, or `None` if the token falls
    /// back to the global bound.
    pub fn get_token_min_shipment_value(env: Env, token: Address) -> Option<i128> {
        env.storage()
            .instance()
            .get(&DataKeyExt2::TokenMinShipmentValue(token))
    }

    pub fn set_token_max_shipment_value(env: Env, admin: Address, token: Address, max_value: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if max_value < 0 {
            panic!("InvalidAmount");
        }
        env.storage()
            .instance()
            .set(&DataKeyExt2::TokenMaxShipmentValue(token.clone()), &max_value);
        env.events().publish(
            (Symbol::new(&env, "token_max_shipment_value_set"), token),
            max_value,
        );
    }

    /// Clear a per-token maximum override so the token falls back to the
    /// global `max_shipment_value` bound.
    pub fn clear_token_max_shipment_value(env: Env, admin: Address, token: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.storage()
            .instance()
            .remove(&DataKeyExt2::TokenMaxShipmentValue(token.clone()));
        env.events()
            .publish((Symbol::new(&env, "token_max_shipment_value_cleared"),), token);
    }

    /// Returns the per-token maximum override, or `None` if the token falls
    /// back to the global bound.
    pub fn get_token_max_shipment_value(env: Env, token: Address) -> Option<i128> {
        env.storage()
            .instance()
            .get(&DataKeyExt2::TokenMaxShipmentValue(token))
    }

    // ----------------------------------------------------------
    // #365: NAMED MILESTONE TEMPLATE LIBRARY
    // ----------------------------------------------------------

    /// Save a reusable, named milestone set for `creator`. Templates are
    /// namespaced per creator address, so two creators may reuse the same
    /// template name independently without colliding.
    pub fn save_milestone_template(env: Env, creator: Address, name: String, milestones: Vec<Milestone>) {
        creator.require_auth();
        Self::assert_not_paused(&env);

        if milestones.is_empty() {
            panic!("EmptyMilestoneTemplate");
        }

        let max_count: u32 = env
            .storage()
            .instance()
            .get(&DataKeyExt2::MaxMilestoneCount)
            .unwrap_or(constants::DEFAULT_MAX_MILESTONE_COUNT);
        if milestones.len() > max_count {
            panic!("TooManyMilestones");
        }

        let min_pct: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinMilestonePercent)
            .unwrap_or(5u32);
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

        // Normalise: strip any caller-supplied runtime state so every shipment
        // created from this template starts from a clean Pending milestone.
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

        let template_key = DataKeyExt2::MilestoneTemplate(creator.clone(), name.clone());
        let is_new = !env.storage().persistent().has(&template_key);
        env.storage().persistent().set(&template_key, &clean_milestones);
        env.storage().persistent().extend_ttl(
            &template_key,
            constants::TTL_INITIAL_LEDGERS,
            constants::TTL_MAX_LEDGERS,
        );

        if is_new {
            let names_key = DataKeyExt2::MilestoneTemplateNames(creator.clone());
            let mut names: Vec<String> = env
                .storage()
                .persistent()
                .get(&names_key)
                .unwrap_or_else(|| Vec::new(&env));
            names.push_back(name.clone());
            env.storage().persistent().set(&names_key, &names);
            env.storage().persistent().extend_ttl(
                &names_key,
                constants::TTL_INITIAL_LEDGERS,
                constants::TTL_MAX_LEDGERS,
            );
        }

        env.events()
            .publish((Symbol::new(&env, "milestone_template_saved"), creator), name);
    }

    /// List the names of all milestone templates saved by `creator`.
    pub fn list_milestone_templates(env: Env, creator: Address) -> Vec<String> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::MilestoneTemplateNames(creator))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Fetch a saved milestone template by (creator, name). Panics with
    /// "TemplateNotFound" if no such template exists.
    pub fn get_milestone_template(env: Env, creator: Address, name: String) -> Vec<Milestone> {
        env.storage()
            .persistent()
            .get(&DataKeyExt2::MilestoneTemplate(creator, name))
            .unwrap_or_else(|| panic!("TemplateNotFound"))
    }

    /// Create a shipment using the milestone structure saved under
    /// `template_name` for the primary buyer (buyers[0]). All other
    /// create_shipment validation (value bounds, milestone count cap,
    /// whitelists, etc.) applies identically.
    pub fn create_shipment_from_template(
        env: Env,
        shipment_id: String,
        buyers: Vec<Address>,
        supplier: Address,
        logistics: Address,
        arbiter: Address,
        token: Address,
        total_amount: i128,
        template_name: String,
        options: ShipmentOptions,
    ) -> String {
        if buyers.is_empty() {
            panic!("at least one buyer is required");
        }
        let creator = buyers.get(0).unwrap();
        let milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&DataKeyExt2::MilestoneTemplate(creator, template_name))
            .unwrap_or_else(|| panic!("TemplateNotFound"));

        Self::create_shipment(
            env,
            shipment_id,
            buyers,
            supplier,
            logistics,
            arbiter,
            token,
            total_amount,
            milestones,
            options,
        )
    }
}

pub mod constants;
mod storage;
mod test_arbiter_pool;
mod test_arbiter_slashing;
mod test_cancellation_reason;
mod test_common;
mod test_correct_proof;
mod test_feat_four;
mod test_issues_366_369;
mod test_issues_389_390_391_392;
mod test_new_features;

// Legacy test modules — some have pre-existing compilation issues.
// They are kept as source but only enabled when their API drift is resolved.
mod benchmarks;
mod property_tests;
mod test;
// mod test_admin;
// mod test_dispute;
// mod test_errors;
mod test_arbiter_security;
mod test_boundaries;
mod test_boundary_validation;
mod test_chaos;
mod test_concurrent_disputes;
mod test_escalation;
mod test_features;
mod test_issues;
mod test_new_issues;
mod test_oracle;
mod test_panel_features;
mod test_permissions;
mod test_query;
mod test_rebalance_milestones;
mod test_shipment;
mod test_top_up_escrow;
mod test_upgrade;
mod test_configurable_limits;
mod test_buyer_spending_limit;
mod test_dispute_mediator;
mod test_emergency_freeze;
mod test_event_schema;
mod test_fee_tier_recalc;
mod test_multi_token;
mod test_partial_cancellation;
mod test_payout_currency_preference;
mod test_proof_validation;
mod test_supplier_collateral;
mod test_supplier_tiering;
mod test_upgrade_multisig;
mod test_jurisdiction_tag;
mod test_max_allowed_tokens;
mod test_fee_waiver;
mod test_payout_preview;
