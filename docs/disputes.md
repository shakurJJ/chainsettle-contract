# Dispute Resolution Guide

This guide walks through the full lifecycle of a milestone dispute in ChainSettle: how disputes are filed (full, partial, quality-grade challenge, or oracle condition breach), how evidence is attached, how parties can withdraw or settle via zero-fee mediation, how binding rulings work (single arbiter binary, percentage split, or multi-arbiter panel), and how finality delays, appeals, and timeout fallbacks operate.

All entry points referenced below live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs). For arbiter pool management, staking, and slashing mechanics, see [`docs/arbiters.md`](arbiters.md).

---

## Dispute Lifecycle Overview

```
ProofSubmitted / ConfirmedHeld
  │
  ├──► raise_dispute / raise_partial_dispute / report_condition_breach
  │      │  (auto-escalates to panel if disputed_amount >= PanelEscalation threshold)
  │      ▼
  │    MilestoneStatus::Disputed
  │      ├──► submit_dispute_evidence (buyer, supplier, or logistics)
  │      ├──► withdraw_dispute (buyer reverts to ProofSubmitted)
  │      ├──► propose_mediation ──► accept_mediation (both parties; 0 arbiter fee)
  │      ├──► resolve_dispute_timeout (backup handoff #421 ──► default_resolution)
  │      └──► Binding Arbitration:
  │             • resolve_dispute (approve: true / false)
  │             • resolve_dispute_split (supplier_bps: 0..=10_000)
  │             • cast_dispute_vote (panel majority = N/2 + 1)
  │             │
  │             ├──► If FinalityDelay > 0 (or Split with AppealWindow > 0):
  │             │      MilestoneStatus::ResolvedPendingFinality
  │             │        ├──► appeal_dispute (within AppealWindow) ──► Re-opens Disputed
  │             │        └──► finalize_dispute_resolution (after release_after_ledger)
  │             │
  │             └──► MilestoneStatus::Resolved (or Pending if full dispute rejected)
  │                    └──► appeal_dispute (within AppealWindow) ──► Re-opens Disputed
```

---

## 1. Filing a Dispute

### 1.1 Four Ways a Dispute Can Be Opened

| Path | Entry Point | Who Calls | Behavior |
| --- | --- | --- | --- |
| **1. Full Dispute** | `raise_dispute(env, caller, shipment_id, milestone_index)` | Any buyer in `shipment.buyers` | Contests **100%** of a `ProofSubmitted` or `ConfirmedHeld` milestone. Transitions status to `MilestoneStatus::Disputed`. |
| **2. Quality-Grade Challenge (`#519`)** | `raise_dispute(env, supplier, shipment_id, milestone_index)` | `shipment.supplier` | Contests a buyer's reduced quality grade (`PendingGrade` with `grade_bps < 10_000`) while the milestone is in `ConfirmedHeld` before `release_after_ledger`. |
| **3. Partial Dispute** | `raise_partial_dispute(env, buyer, shipment_id, milestone_index, contested_percent)` | Any buyer in `shipment.buyers` | Contests only `contested_percent` (`1..=99`) of a `ProofSubmitted` or `ConfirmedHeld` milestone. Immediately releases the uncontested `(100 - contested_percent)%` (less platform fee) to `shipment.supplier` and locks only `contested_percent%` in `DataKey::DisputeContestedPercent(shipment_id, milestone_index)`. *(Not permitted if an approved advance exists on the milestone.)* |
| **4. Oracle Condition Breach (`#546`)** | `report_condition_breach(env, oracle, shipment_id, milestone_index, data_hash)` | Registered members of the shipment's oracle group | When the number of distinct oracle breach reports reaches the group's `threshold` (`register_oracle_group`, `set_shipment_oracle_purpose`), `open_condition_breach_dispute` automatically transitions the milestone (`Pending`, `ProofSubmitted`, or `ConfirmedHeld`) to `Disputed` without requiring buyer auth or pulling an extra dispute bond. |

### 1.2 Filing Guards & Rate Limits

Before `raise_dispute` or `raise_partial_dispute` succeeds, the contract enforces:
1. **Auto-Confirm Window Not Elapsed:** `current_ledger < proof_submitted_ledger + effective_auto_confirm_window` (`AutoConfirmWindowElapsed` otherwise).
2. **Post-Deadline Filing Window (`set_dispute_filing_window` / `get_dispute_filing_window`):** If `milestone.deadline_ledger > 0` and global `DisputeFilingWindowLedgers > 0`, requires `current_ledger < deadline_ledger + window` (`DisputeFilingWindowExpired` otherwise).
3. **Per-Shipment Cooldown (`shipment.dispute_cooldown_ledgers`):** Requires `current_ledger >= shipment.last_dispute_resolved_ledger + dispute_cooldown_ledgers` (`DisputeCooldownActive` otherwise).
4. **Concurrent Dispute Cap (`set_max_concurrent_disputes`):** `shipment.open_dispute_count < MaxConcurrentDisputes` (default `1`; panics with `MaxConcurrentDisputesReached` otherwise).
5. **Per-Buyer Rolling-Window Cap (`#477` — `set_buyer_dispute_cap` / `get_buyer_dispute_cap`):** If configured (`max_disputes > 0`, `window_ledgers > 0`), caps how many disputes a buyer address can file across all shipments within the last `window_ledgers` ledgers.

---

## 2. Submitting Evidence (`submit_dispute_evidence`)

While a milestone is in `MilestoneStatus::Disputed`, any counterparty (**buyer**, **supplier**, or **logistics provider**) can attach immutable off-chain document hashes (e.g., IPFS CIDs or SHA-256 hashes of photos, bills of lading, or sensor logs):

```rust
pub struct DisputeEvidence {
    pub submitter: Address,
    pub evidence_hash: BytesN<32>,
    pub evidence_type: Symbol,
    pub submitted_ledger: u32,
}
```

- **`submit_dispute_evidence(env, caller, shipment_id, milestone_index, evidence_hash, evidence_type)`:**
  - Appends to `DataKeyExt::DisputeEvidence(shipment_id, milestone_index)` and emits `dispute_evidence_submitted`.
- **`get_dispute_evidence(env, shipment_id, milestone_index) -> Vec<DisputeEvidence>`:**
  - Returns all evidence entries in submission order.

---

## 3. Pre-Arbitration Exits: Withdrawal & Non-Binding Mediation

### 3.1 Buyer Withdrawal (`withdraw_dispute`)

A buyer who opened a dispute can withdraw it before the arbiter rules:
- **`withdraw_dispute(env, buyer, shipment_id, milestone_index)`:**
  - Reverts the milestone from `Disputed` back to `MilestoneStatus::ProofSubmitted`, decrements `shipment.open_dispute_count`, records `DisputeOutcome::Withdrawn` in dispute history, and emits `dispute_withdrawn`.
  - *(Not available for supplier-initiated quality-grade disputes.)*

### 3.2 Non-Binding Mediation (`#400`)

To settle disputes amicably **without paying an arbiter fee**, an authorized mediator can propose an outcome (`Resolution::Supplier` or `Resolution::Buyer`):

1. **Assign Mediator(s):**
   - Primary `Admin` calls `assign_mediator(env, admin, shipment_id, mediator)` (per-shipment) or `set_mediator_pool(env, admin, mediators: Vec<Address>)` (global pool; queried via `get_mediator` and `get_mediator_pool`).
2. **Propose Outcome (`propose_mediation(env, mediator, shipment_id, milestone_index, suggested_outcome)`):**
   - Stores `MediationProposal { mediator, suggested_outcome, buyer_accepted: false, supplier_accepted: false, proposed_at_ledger }` at `DataKeyExt2::MediationProposal(shipment_id, milestone_index)`.
   - Query via `get_mediation_proposal(env, shipment_id, milestone_index)`.
3. **Accept or Decline (`accept_mediation` / `decline_mediation`):**
   - Buyer and supplier each call **`accept_mediation(env, caller, shipment_id, milestone_index)`**.
   - Once **both** `buyer_accepted` and `supplier_accepted` are `true`, `apply_mediation_outcome` executes the resolution immediately with **zero arbiter fee** and emits `mediation_settled`.
   - Either party can call **`decline_mediation(env, caller, shipment_id, milestone_index)`** at any time before dual acceptance to delete the proposal (`mediation_declined`) and proceed to binding arbitration.

---

## 4. Binding Arbitration Paths

*(Note: When `ArbiterPanel(shipment_id)` is configured or auto-escalated via `set_panel_escalation_threshold`, single-arbiter `resolve_dispute` and `resolve_dispute_split` are disabled; panel members must use `cast_dispute_vote`.)*

### 4.1 Single-Arbiter Binary Ruling (`resolve_dispute` & `batch_resolve_disputes`)

The shipment's assigned `arbiter` calls **`resolve_dispute(env, arbiter, shipment_id, milestone_index, approve: bool)`** (or `resolve_dispute_with_reason(..., reason_hash: BytesN<32>)`, queried via `get_dispute_resolution_reason`).

| Dispute Type | `approve = true` (Supplier Wins) | `approve = false` (Buyer Wins) |
| --- | --- | --- |
| **Full Dispute** | Supplier receives `milestone_gross - platform_fee - advance - arbiter_fee` (with retainage/warranty holdbacks applied). Buyer's `dispute_bond_amount` is returned (less loser-pays arbiter fee if enabled). Milestone becomes `Resolved` (or `ResolvedPendingFinality`). | Arbiter receives `arbiter_fee` (deducted from milestone gross). Milestone resets to **`MilestoneStatus::Pending`** (`proof_hash = 0`) so the supplier can re-deliver and submit new proof. Buyer's `dispute_bond_amount` is **forfeited to the supplier**. |
| **Partial Dispute (`contested_percent`)** | Supplier receives the contested portion (less platform fee and arbiter fee). Buyer's dispute bond is returned. Milestone becomes `Resolved` (or `ResolvedPendingFinality`). | Buyer is refunded `contested_amount - arbiter_fee` (unless covered by forfeited bond under `loser_pays_arbiter_fee`). Buyer's dispute bond is forfeited to the supplier. Milestone becomes `Resolved`. |
| **Quality-Grade Challenge (`#519`)** | Supplier's challenge succeeds: supplier receives 100% of `milestone_gross` (less platform fee and arbiter fee). Milestone becomes `Resolved`. | Supplier's challenge fails: supplier receives only the graded share $\lfloor \text{gross} \times \text{grade\_bps} / 10{,}000 \rfloor$ (less platform/logistics fees), and the buyer is refunded the remainder (less `arbiter_fee`). Milestone becomes `Confirmed`. |

### 4.2 Percentage Split Ruling (`#664` — `resolve_dispute_split`)

For cases where both parties share fault on a full or partial dispute, the arbiter can split the contested funds in basis points (`0..=10_000`) via **`resolve_dispute_split(env, arbiter, shipment_id, milestone_index, supplier_bps)`**:

- **Contested Fund Split:**
  $$\text{supplier\_share} = \left\lfloor \frac{\text{contested} \times \text{supplier\_bps}}{10{,}000} \right\rfloor, \quad \text{buyer\_share} = \text{contested} - \text{supplier\_share}$$
- **Pro-Rata Arbiter Fee Split:**
  $$\text{arbiter\_fee} = \left\lfloor \frac{\text{contested} \times \text{applicable\_arbiter\_bps}}{10{,}000} \right\rfloor$$
  $$\text{supplier\_fee\_part} = \left\lfloor \frac{\text{arbiter\_fee} \times \text{supplier\_bps}}{10{,}000} \right\rfloor, \quad \text{buyer\_fee\_part} = \text{arbiter\_fee} - \text{supplier\_fee\_part}$$
- **Platform Fee:** Charged only on `supplier_share` (`deduct_fee_for_shipment`).
- **Pro-Rata Dispute Bond Split:**
  $$\text{bond\_to\_buyer} = \left\lfloor \frac{\text{bond} \times (10{,}000 - \text{supplier\_bps})}{10{,}000} \right\rfloor, \quad \text{bond\_to\_supplier} = \text{bond} - \text{bond\_to\_buyer}$$
- **Query Split:** `get_dispute_split(env, shipment_id, milestone_index) -> Option<u32>` returns `Some(supplier_bps)`.
- *(Not supported on quality-grade disputes.)*

### 4.3 Multi-Arbiter Panel Voting (`cast_dispute_vote`)

When an `ArbiterPanel` is active on the shipment (either set in `ShipmentOptions.arbiter_panel` or auto-escalated via `#665` when `disputed_amount >= PanelEscalation.min_amount`):
- Each panel member calls **`cast_dispute_vote(env, arbiter, shipment_id, milestone_index, approve: bool)`**.
- Once simple majority (`panel.len() / 2 + 1`) votes `true` or `false`, the milestone settles with the same payouts as `resolve_dispute` and splits the arbiter fee equally across voting panel members.

---

## 5. Finality Delay (`#393`) and Appeals (`#369`)

### 5.1 Resolution Finality Delay (`finalize_dispute_resolution`)

To give the losing party time to inspect a ruling and file an appeal before funds leave escrow:
- Primary `Admin` configures **`set_resolution_finality_delay(env, admin, delay_ledgers)`** (`get_resolution_finality_delay`).
- When `delay_ledgers > 0` (or when `resolve_dispute_split` is called while `AppealWindow > 0`), the ruling parks the milestone in **`MilestoneStatus::ResolvedPendingFinality`** with `release_after_ledger = current_ledger + delay` and emits `dispute_resolution_pending`.
- Once `env.ledger().sequence() >= release_after_ledger` with no appeal filed, anyone can call **`finalize_dispute_resolution(env, shipment_id, milestone_index)`** (permissionless) to execute the token transfers and transition the milestone to `MilestoneStatus::Resolved` (`dispute_finalized`).

### 5.2 Appealing a Ruling (`appeal_dispute`)

1. **Configuration:**
   - `set_appeal_window(env, admin, window_ledgers)` (`get_appeal_window`): sets how many ledgers after `DisputeResolvedAt` an appeal is permitted.
   - `set_appeal_fee(env, admin, fee)` (`get_appeal_fee`): optional flat token deposit required from the appellant.
2. **Filing the Appeal (`appeal_dispute(env, caller, shipment_id, milestone_index)`):**
   - Callable by any buyer on the shipment or by `shipment.supplier` within `resolved_at + window_ledgers`.
   - **One appeal per dispute cycle:** Panics with `"milestone has already been appealed"` if `DisputeAppealed(shipment_id, milestone_index)` is already `true`.
   - Escrows `appeal_fee` from `caller` (if `> 0`), clears any parked `PendingSplit` or `PanelEscalated` state, selects a **new distinct arbiter** from `arbiters_pool`, and transitions the milestone back to `MilestoneStatus::Disputed` (`dispute_appealed`).
3. **Appeal Settlement:**
   - When the appeal arbiter resolves the milestone:
     - **Appellant wins:** `appeal_fee` is refunded to `appellant` (`appeal_fee_refunded`).
     - **Appellant loses:** `appeal_fee` is transferred to the appeal arbiter (`appeal_fee_forfeited`).
     - **Overturn check (`apply_appeal_overturn`):** If the appeal ruling flips the original arbiter's decision, `overturned_count` is incremented on the original arbiter and pool/stake slashing rules are evaluated (see [`docs/arbiters.md`](arbiters.md)).

---

## 6. Escalation & Timeout Fallbacks

If an assigned arbiter becomes unresponsive while a milestone is `Disputed`:

| Mechanism | Entry Point | Trigger Condition | Effect |
| --- | --- | --- | --- |
| **Escalation Signal** | `check_escalation(env, shipment_id, milestone_index)` | `current_ledger >= dispute_opened_ledger + EscalationThreshold` (`set_escalation_threshold`) | Permissionless check; emits `dispute_escalated` for off-chain monitoring. |
| **Ledger Failover to Backup** | `activate_backup_arbiter(env, shipment_id, milestone_index)` | `current_ledger >= dispute_opened_ledger + ArbiterInactivityThreshold` (`set_arbiter_inactivity_threshold`) | Promotes `BackupArbiter(shipment_id)` to `shipment.arbiter` and emits `arbiter_failover`. |
| **Timestamp Timeout Resolution (`#421`)** | `resolve_dispute_timeout(env, shipment_id, milestone_index)` | `current_timestamp >= DisputeOpenedAt + shipment.dispute_timeout_seconds` | **1st timeout (if `BackupArbiter` exists):** Hands off to `BackupArbiter`, resets `DisputeOpenedAt`, and emits `dispute_timeout_handoff`.<br>**2nd timeout (or no backup):** Settles the milestone using `shipment.default_resolution` (`Resolution::Supplier` or `Resolution::Buyer`) with **zero arbiter fee** (`dispute_timeout_resolved`). |

---

## 7. Dispute History & Analytics Queries

- **`get_active_disputes(env) -> Vec<(u64, u32)>`:** Returns all currently open `(shipment_id, milestone_index)` pairs.
- **`get_dispute_history(env, address, cursor, limit) -> Vec<DisputeRecord>`:** Paginated per-party dispute history (bounded ring buffer up to `DISPUTE_HISTORY_MAX_ENTRIES = 50` per buyer/supplier), where each `DisputeRecord` includes `shipment_id`, `milestone_index`, `raised_at_ledger`, `resolved_at_ledger`, `outcome` (`SupplierWon`, `BuyerWon`, `TimeoutSupplier`, `TimeoutBuyer`, `Withdrawn`, `MediatedSupplier`, `MediatedBuyer`), and `arbiter`.
