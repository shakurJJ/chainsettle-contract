/// Minimum TTL ledgers for persistent storage entries (~1 day at 5s/ledger).
pub const TTL_INITIAL_LEDGERS: u32 = 100_000;

/// Maximum TTL ledgers for persistent storage entries (~1 year at 5s/ledger).
/// 6_300_000 ≈ 5s × 86_400s/day × 365 days.
pub const TTL_MAX_LEDGERS: u32 = 6_300_000;

/// Minimum shipment amount in token base units (stroops). Must be > 0.
pub const MIN_SHIPMENT_AMOUNT: i128 = 1;

/// Maximum platform fee in basis points (1000 bps = 10%).
pub const MAX_FEE_BPS: u32 = 1_000;

/// Ledgers after shipment creation before emergency recovery is allowed.
/// ≈ 2 years at 5s/ledger: 5s × 86_400s/day × 365 days × 2.
pub const RECOVERY_THRESHOLD_LEDGERS: u32 = 12_614_400;

/// Maximum entries retained in the bounded admin action audit log.
pub const AUDIT_LOG_MAX_ENTRIES: usize = 50;

/// Maximum entries retained in the per-shipment audit log (ring-buffer).
pub const SHIPMENT_AUDIT_LOG_MAX_ENTRIES: usize = 20;

/// Maximum shipments returned per page in list_shipments.
pub const LIST_SHIPMENTS_MAX_PAGE: u32 = 50;

/// Maximum shipments returned per page in `get_shipments_by_token` (#629).
pub const LIST_SHIPMENTS_BY_TOKEN_MAX_PAGE: u32 = 50;

/// Default maximum number of milestones allowed per shipment when the admin
/// has not configured an override (#364). Chosen well above any milestone
/// count exercised by existing tests/benchmarks so default behaviour is
/// unchanged until an admin opts in to a different cap.
pub const DEFAULT_MAX_MILESTONE_COUNT: u32 = 50;

/// Default supermajority (basis points) of registered multisig admins
/// required to activate/lift an emergency global freeze (#402) when the
/// admin has not configured an override. 8000 = 80%.
pub const DEFAULT_EMERGENCY_FREEZE_SUPERMAJORITY_BPS: u32 = 8_000;

/// Max shipments accepted by `batch_cancel_shipments` (#580) in one call.
pub const MAX_BATCH_CANCEL_SHIPMENTS: u32 = 20;

/// Max proof entries accepted by `batch_submit_proofs` (#666) in one call.
pub const MAX_BATCH_PROOF_SUBMISSIONS: u32 = 20;

/// Max arbiters accepted by `batch_add_arbiters_to_pool` (#663) in one call.
pub const MAX_BATCH_ADD_ARBITERS: u32 = 20;

/// Smallest panel size accepted for automatic panel escalation (#665),
/// matching the minimum enforced by `assign_dispute_panel`.
pub const MIN_ESCALATION_PANEL_SIZE: u32 = 3;

/// Maximum basis points a shipment creator may configure for a value-scaled
/// dispute bond (#391) when the admin has not set a stricter cap via
/// `set_max_dispute_bond_bps`. 2000 = 20% of shipment value.
pub const DEFAULT_MAX_DISPUTE_BOND_BPS: u32 = 2_000;

/// Default supplier onboarding stake amount (can be configured by admin).
pub const DEFAULT_SUPPLIER_STAKE: i128 = 1_000_000;

/// Number of dispute losses before supplier stake is slashed (default 3).
pub const DEFAULT_DISPUTE_LOSS_THRESHOLD: u32 = 3;

/// Percentage of stake slashed per dispute loss (default 33.33% = 3333 bps).
pub const DEFAULT_SLASH_PERCENTAGE_BPS: u32 = 3_333;

/// Maximum number of quality grades a shipment may configure (#519).
pub const MAX_QUALITY_GRADES: u32 = 10;

/// Grade review window (in ledgers) used by `confirm_milestone_graded` when the
/// shipment has neither a holdback nor a review/auto-confirm window configured
/// (#519). ~1 day at 5 s/ledger.
pub const DEFAULT_GRADE_REVIEW_WINDOW_LEDGERS: u32 = 17_280;

/// Maximum retainage a shipment may withhold from each milestone payment (#520).
pub const MAX_RETAINAGE_BPS: u32 = 2_000;

/// Maximum warranty holdback a shipment may withhold from each milestone payment (#521).
pub const MAX_WARRANTY_BPS: u32 = 2_000;

/// Semantic contract version — must match `contracts/chainsetttle/Cargo.toml`.
/// #571
pub const VERSION_MAJOR: u32 = 0;
pub const VERSION_MINOR: u32 = 1;
pub const VERSION_PATCH: u32 = 0;

/// Storage schema version written by `migrate` (#571). Bump when persistent
/// layout changes require a post-upgrade migration.
pub const STORAGE_SCHEMA_VERSION: u32 = 1;

/// Default max shipment IDs accepted by `get_shipment_summaries` in one call
/// when the admin has not configured an override (#575).
pub const DEFAULT_MAX_SUMMARY_BATCH: u32 = 50;

/// Maximum entries retained in a per-address dispute history (#577), ring-buffer
/// style like `SHIPMENT_AUDIT_LOG_MAX_ENTRIES` (oldest dropped on overflow).
pub const DISPUTE_HISTORY_MAX_ENTRIES: usize = 100;

/// Maximum number of parties checked against the external compliance registry
/// per `create_shipment` call (#652). Bounds the number of cross-contract
/// calls so a single shipment cannot exhaust the instruction budget.
pub const MAX_COMPLIANCE_PARTIES: u32 = 8;

/// Maximum byte length of a metadata value that may be stored under an
/// indexed metadata key (#662). Keeps index storage keys bounded.
pub const MAX_METADATA_INDEX_VALUE_LEN: u32 = 128;

/// Maximum shipment IDs held in a single (metadata key, value) index bucket (#662).
pub const MAX_METADATA_INDEX_ENTRIES: u32 = 100;
