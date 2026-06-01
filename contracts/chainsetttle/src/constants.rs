/// Minimum TTL ledgers for persistent storage entries (~1 day at 5s/ledger).
pub const TTL_INITIAL_LEDGERS: u32 = 100_000;

/// Maximum TTL ledgers for persistent storage entries (~1 year at 5s/ledger).
/// 6_300_000 ≈ 5s × 86_400s/day × 365 days.
pub const TTL_MAX_LEDGERS: u32 = 6_300_000;

/// Maximum number of milestones allowed per shipment.
pub const MAX_MILESTONES: u32 = 20;

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
