# Architecture and Storage Key Layout Guide

This guide explains the internal structure of the ChainSettle Soroban smart contract, why storage keys are partitioned across `DataKey` through `DataKeyExt5`, how data is assigned across Soroban's three storage tiers (`instance`, `persistent`, and `temporary`), and the core state machines and accounting invariants that contributors must preserve.

---

## 1. Crate and Module Map

All contract source files live under [`contracts/chainsetttle/src/`](../contracts/chainsetttle/src/):

| File | Responsibility |
| --- | --- |
| [`lib.rs`](../contracts/chainsetttle/src/lib.rs) | `ChainSettleContract` entry points (`#[contractimpl]`), domain structs (`Shipment`, `Milestone`, `ShipmentOptions`, etc.), primary storage key enums (`DataKey`..`DataKeyExt5`), validation helpers, TTL extension routines, and business logic. |
| [`constants.rs`](../contracts/chainsetttle/src/constants.rs) | Protocol-wide constants: semantic version (`VERSION_MAJOR/MINOR/PATCH`), `STORAGE_SCHEMA_VERSION`, TTL parameters (`TTL_INITIAL_LEDGERS = 100_000`, `TTL_MAX_LEDGERS = 6_300_000`), batch caps, and fee/holdback limits. |
| [`storage.rs`](../contracts/chainsetttle/src/storage.rs) | Storage helper routines (`get_arbiter_stats`, `get_supplier_terms_profile`), terminal auxiliary-key pruning (`prune_shipment_auxiliary_keys`), legacy `V1*` keys (`storage::DataKey`), and `migrate_v1_to_v2`. |
| [`paged_migration.rs`](../contracts/chainsetttle/src/paged_migration.rs) | Cursor-based batched migration (`MigrationState`, `MigrationKey`, `MAX_MIGRATION_BATCH_SIZE = 50`) for upgrading `V1Shipment` entries to V2 `Shipment` without exceeding per-tx resource limits. |
| [`custody_log.rs`](../contracts/chainsetttle/src/custody_log.rs) | Append-only physical custody handoff log per shipment (`CustodyEvent`, `CustodyLogKey`, capped at `MAX_CUSTODY_EVENTS = 50`). |
| [`errors.rs`](../contracts/chainsetttle/src/errors.rs) | `#[contracterror] pub enum ContractError` (`1..=33`). See [`docs/errors.md`](errors.md). |
| [`events.rs`](../contracts/chainsetttle/src/events.rs) | Typed event payload structs (`ShipmentCreatedEvent`, `ProofSubmittedEvent`, etc.). See [`docs/events.md`](events.md). |
| `test_*.rs` | Feature-scoped unit and integration test modules (`test_common.rs` provides shared fixtures). |

---

## 2. The 50-Variant Soroban XDR Limit & `DataKey`..`DataKeyExt5` Layout

### 2.1 Why Multiple `DataKey` Enums Exist

In the Soroban Rust SDK, any `#[contracttype]` enum is compiled into a contract spec XDR union (`ScSpecUdtUnionV0`) whose `cases` vector is hard-capped at **50 variants** (`VecM<ScSpecUdtUnionCaseV0, 50>`). Adding a 51st variant to a `#[contracttype]` enum causes compilation/spec generation to fail.

Crucially, when Soroban serializes a `#[contracttype]` enum value as a ledger storage key, the on-chain key is encoded from the **variant name** (`Symbol`) and its tuple payload—**not** the Rust enum type name. This means partitioning keys across multiple `#[contracttype]` enums (`DataKey`, `DataKeyExt`, `DataKeyExt2`, `DataKeyExt3`, `DataKeyExt4`, `DataKeyExt5`) has **zero runtime overhead** and preserves backward compatibility with existing on-chain keys.

### 2.2 Current Key Enum Capacity Table

| Enum | Location | Variant Count | Status | Representative Keys |
| --- | --- | --- | --- | --- |
| **`DataKey`** | `src/lib.rs` | **49 / 50** | Closed (keep `< 50`) | `Admin`, `Paused`, `Shipment(u64)`, `FeeConfig`, `SupplierCollateral(u64)`, `TotalEscrowed`, `ContractStats`, `AllShipments` |
| **`DataKeyExt`** | `src/lib.rs` | **50 / 50** | **FULL** | `ArbiterPanel(u64)`, `DisputeVotes(u64, u32)`, `MilestonePayees(u64, u32)`, `FeeTiers`, `LifetimeVolume(Address)`, `ArbiterStats(Address)` |
| **`DataKeyExt2`** | `src/lib.rs` | **50 / 50** | **FULL** | `EmergencyFrozen`, `UpgradeProposal(u64)`, `FeeWaiver(Address)`, `MediationProposal(u64, u32)`, `ArbiterSlashed(Address)`, `DisputeAppealed(u64, u32)` |
| **`DataKeyExt3`** | `src/lib.rs` | **50 / 50** | **FULL** | `MilestoneQuantities(u64)`, `PendingGrade(u64, u32)`, `RetainageBalance(u64)`, `WarrantyBalance(u64)`, `FeeHoliday`, `IncrementalCollateral(u64)` |
| **`DataKeyExt4`** | `src/lib.rs` | **50 / 50** | **FULL** | `ArbiterStake(Address)`, `PayoutAssignment(u64)`, `CollateralToken(u64)`, `StorageSchemaVersion`, `AppealFee`, `ShipmentFeesPaid(u64)` |
| **`DataKeyExt5`** | `src/lib.rs` | **8 / 50** | **Active (`42` free slots)** | `PanelEscalation`, `PanelEscalated(u64, u32)`, `DisputeSplitBps(u64, u32)`, `PendingSplit(u64, u32)`, `SupplierTimeliness(Address)`, `SplitShipmentProposal(u64)`, `SupplierTermsProfile(Address)`, `MilestoneNotBeforeProposal(u64, u32)` |
| **`CustodyLogKey`** | `src/custody_log.rs` | **2 / 50** | Module-scoped | `CustodyLog(u64)`, `CustodyCount(u64)` |
| **`MigrationKey`** | `src/paged_migration.rs` | **1 / 50** | Module-scoped | `MigrationState` |
| **`storage::DataKey`** | `src/storage.rs` | **30 / 50** | Legacy V1 migration | `V1Shipment(u64)`, `V1Admin`, etc. |

*(In addition, three arbiter-pool keys use raw `Symbol` / tuple keys directly: `Symbol("arbiters_pool")` and `Symbol("arb_pool_idx")` in instance storage, and `(Symbol("use_pool_arb"), shipment_id)` in persistent storage.)*

### 2.3 Contributor Rules for Adding a New Storage Key

1. **Never add variants to `DataKey` or `DataKeyExt`..`DataKeyExt4`** — they are at capacity (`50/50`).
2. **Add new variants to `DataKeyExt5`** until it reaches `50` variants. When `DataKeyExt5` reaches `50` variants, define a new `#[contracttype] #[derive(Clone)] pub enum DataKeyExt6` in `src/lib.rs`.
3. **Never duplicate a variant name across `DataKey*` enums** with the same payload shape—because Soroban encodes the variant name (`Symbol`) as the key tag, two enums with an identically named variant would collide in the same storage tier!
4. **Never rename an existing variant or alter its tuple payload types** without a storage migration, as doing so changes its serialized XDR key and orphans existing ledger entries.
5. **Register per-shipment and per-milestone persistent keys in lifecycle helpers:**
   - Add per-shipment persistent keys to the `related_keys` array in `extend_shipment_related_keys_ttl` (`src/lib.rs`).
   - Add `(shipment_id, milestone_index)` persistent keys to `per_milestone_keys` (`src/lib.rs`).
   - If the key holds temporary/operational metadata that is no longer needed once a shipment is `Completed` or `Cancelled`, add it to `storage::prune_shipment_auxiliary_keys` (`src/storage.rs`).

---

## 3. Soroban Storage Tiers (`Instance` vs. `Persistent` vs. `Temporary`)

Soroban provides three storage tiers with distinct cost, TTL, and archival semantics. ChainSettle assigns keys to tiers as follows (see [`docs/storage-ttl.md`](storage-ttl.md) for TTL extension entry points):

| Storage Tier | Accessor | Characteristics | ChainSettle Keys Stored Here |
| --- | --- | --- | --- |
| **Instance Storage** | `env.storage().instance()` | Stored inside the contract instance's single ledger entry; loaded into memory on every invocation. Cannot grow unboundedly. | **Global Configuration & Singletons:**<br>• Governance: `Admin`, `PendingAdmin`, `Paused`, `EmergencyFrozen`, `MultiAdminConfig`, `AdminActionLog`, `TimelockDuration`, `PendingParamChange`<br>• Fees: `FeeConfig`, `FeeTiers`, `FeeRecipients`, `FeeHoliday`, `ReferralFeeBps`, `CleanCompletionRebateBps`, `LongHoldRebate`, `ArbiterFeeTiers`, `AppealFee`<br>• Risk & Compliance: `AllowedTokens`, `TokenAddress`, `Blacklisted(addr)`, `Whitelisted(addr)`, `WhitelistEnabled`, `CircuitBreakerLimit`, `MaxShipmentValue`, `TotalEscrowed`, `ContractStats`, `StorageSchemaVersion`<br>• Pools & Thresholds: `"arbiters_pool"`, `"arb_pool_idx"`, `MediatorPool`, `PanelEscalation`, `SupplierTierConfig`, `AutoConfirmThreshold`, `MaxAdvancePercent` |
| **Persistent Storage** | `env.storage().persistent()` | Dedicated ledger entry per key. Must have its TTL periodically extended (`extend_ttl`), or can be restored from ledger archival if expired. Scales to arbitrary numbers of shipments and users. | **Per-Shipment, Per-Milestone & Per-Address Records:**<br>• Core Shipment State: `Shipment(id)`, `SupplierCollateral(id)`, `MilestoneAdvance(id, idx)`, `MilestoneSplits(id)`, `MilestoneQuantities(id)`, `PartialQtyDelivered(id, idx)`, `PartialQtyReleased(id, idx)`, `PendingGrade(id, idx)`<br>• Holdbacks & Routing: `RetainageBps(id)`, `RetainageBalance(id)`, `WarrantyConfig(id)`, `WarrantyBalance(id)`, `WarrantyEndsAt(id)`, `WarrantyClaim(id)`, `PayoutAssignment(id)`, `MilestonePayees(id, idx)`, `PendingPayout(addr)`<br>• Disputes & Arbiters: `DisputeContestedPercent(id, idx)`, `DisputeEvidence(id, idx)`, `DisputeVotes(id, idx)`, `ArbiterPanel(id)`, `PanelEscalated(id, idx)`, `DisputeSplitBps(id, idx)`, `PendingSplit(id, idx)`, `MediationProposal(id, idx)`, `ArbiterStats(addr)`, `ArbiterStake(addr)`, `DisputeHistory(addr)`<br>• User & Role State: `UserRole(role, addr)`, `RoleExpiry(role, addr)`, `Reputation(addr)`, `LifetimeVolume(addr)`, `ActiveVolume(token)`, `SupplierExposure(addr)`, `UpgradeProposal(id)`, `FeeWaiver(addr)` |
| **Temporary Storage** | `env.storage().temporary()` | Cheapest rent; automatically deleted by the network once its TTL expires (`cannot` be restored after expiry). Ideal for short-lived two-party handshakes. | **Ephemeral Handshake Proposals:**<br>• `MutualCancel(shipment_id)` (`propose_mutual_cancel` / `approve_mutual_cancel`)<br>• `ArbiterRotation(shipment_id)` (`propose_arbiter_rotation`) |

---

## 4. Shipment and Milestone State Machines

### 4.1 `ShipmentStatus` State Machine

```
                      create_shipment / batch_create_shipments
                                         │
                                         ▼
              ┌──────────────────►   Created   ◄──────────────────┐
              │                          │                        │
   cancel_shipment /            submit_milestone_proof     propose_mutual_cancel +
   supplier_cancel /                     │                 approve_mutual_cancel
   claim_deadline_refund                 ▼                        │
              │                     InProgress ───────────────────┤
              │                          │                        │
              │             All milestones Confirmed /            │
              │                   Resolved                        │
              │                          │                        │
              ▼                          ▼                        ▼
          Cancelled                  Completed                Cancelled
                                         │
                               archive_shipment
                                         │
                                         ▼
                                     Archived
```

- **Active statuses:** `Created` and `InProgress`.
- **Terminal statuses:** `Completed`, `Cancelled`, and `Archived` (once terminal, no further milestones can be modified, and `storage::prune_shipment_auxiliary_keys` cleans up operational auxiliary keys when `AutoPruneOnTerminal` is enabled).

### 4.2 `MilestoneStatus` State Machine

```
                                  Pending
                                     │
                          submit_milestone_proof
                                     │
                                     ▼
                              ProofSubmitted ◄─────────────── withdraw_dispute
                               │     │     │
         confirm_milestone     │     │     │   raise_dispute / raise_partial_dispute /
        (holdback == 0) or     │     │     │   report_condition_breach
      claim_auto_confirmation  │     │     │
                │              │     │     └──────────────────────┐
                │              │     │ confirm_milestone          │
                │              │     │ (holdback > 0) or          │
                │              │     │ confirm_milestone_graded   │
                │              │     ▼                            ▼
                │              │  ConfirmedHeld ──────────►   Disputed ◄────── appeal_dispute
                │              │     │                            │
                │              │     │ release_held_payment       ├─► approve = false (full dispute)
                │              │     │                            │     ──► Resets to Pending
                ▼              ▼     ▼                            │
               Confirmed ◄───────────┘                            ├─► FinalityDelay > 0
                                                                  │     ──► ResolvedPendingFinality
                                                                  │           │ finalize_dispute_resolution
                                                                  ▼           ▼
                                                                 Resolved ◄───┘
```

---

## 5. Core Accounting Invariants

Every state transition that moves tokens or updates escrow state must preserve the following invariants:

1. **Per-Shipment Escrow Bound:**
   $$\text{shipment.released\_amount} + \text{shipment.total\_advanced\_amount} \le \text{shipment.total\_amount}$$
   `get_escrow_balance(env, shipment_id)` returns `total_amount - released_amount - total_advanced_amount` (or `0` once `Completed` or `Cancelled`).
2. **Global TVL Tracking (`DataKey::TotalEscrowed`):**
   Incremented by `total_amount` at `create_shipment`, decremented by each milestone's gross settlement (`gross_payment`) or cancellation refund (`subtract_from_total_escrowed`). Enforced against `DataKey::CircuitBreakerLimit` at shipment creation.
3. **Per-Token Active Volume (`DataKey::ActiveVolume(token)`):**
   Tracks live escrowed volume per token (`get_active_volume_by_token`), enforced against `DataKey::TokenExposureCap(token)` (`set_token_exposure_cap`).
4. **Per-Supplier Exposure (`DataKey::SupplierExposure(supplier)`):**
   Tracks total unreleased escrow across all active shipments for `supplier` (`get_supplier_exposure`), enforced against `DataKey::MaxSupplierExposure`.
