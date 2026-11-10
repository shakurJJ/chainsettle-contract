# Arbiter Pool, Staking, Panels, and Slashing Guide

ChainSettle supports three ways to assign dispute arbiters to a shipment—a dedicated single arbiter, automatic round-robin selection from a curated on-chain **Arbiter Pool** (with repeat-pairing avoidance), or an odd-sized **Multi-Arbiter Panel** (configured at creation or auto-escalated for high-value disputes)—backed by on-chain performance metrics, optional token staking, appeal overturn tracking, and automatic slashing.

All entry points referenced below live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs) and [`contracts/chainsetttle/src/storage.rs`](../contracts/chainsetttle/src/storage.rs).

---

## 1. Managing the Arbiter Pool

The global arbiter pool (`Symbol("arbiters_pool")` in instance storage) is managed by **`Role::ArbiterManager`** or the primary **`Admin`**:

| Function | Auth | Description |
| --- | --- | --- |
| `add_arbiter_to_pool(env, admin, arbiter)` | `Role::ArbiterManager` or `Admin` | Appends `arbiter` to the pool (no-op if already present). Rejects blacklisted addresses (`"arbiter is blacklisted"`) and slashed arbiters (`"arbiter has been slashed; call reinstate_arbiter first"`). Emits `arbiter_added_to_pool`. |
| `batch_add_arbiters_to_pool(env, admin, arbiters: Vec<Address>)` | `Role::ArbiterManager` or `Admin` | Atomically adds between `1` and **`MAX_ARBITER_BATCH_SIZE` (`50`)** arbiters. Validates the entire batch upfront (no duplicates within the batch, none blacklisted, none slashed) before modifying storage. |
| `remove_arbiter_from_pool(env, admin, arbiter)` | `Role::ArbiterManager` or `Admin` | Removes `arbiter` from the pool and emits `arbiter_removed_from_pool`. |
| `get_arbiter_pool(env) -> Vec<Address>` | Read-only | Returns the ordered list of addresses currently in the pool. |

---

## 2. Pool Assignment & Repeat-Pairing Avoidance (`#470`)

### 2.1 Using the Pool Sentinel at Shipment Creation

To defer arbiter assignment until a dispute actually occurs:
1. Pass the **contract's own address** (`env.current_contract_address()`) as the `arbiter` argument to `create_shipment`.
2. `create_shipment` verifies `arbiters_pool` is non-empty (otherwise fails with `ContractError::ArbiterPoolEmpty`) and records a persistent flag `(Symbol("use_pool_arb"), shipment_id) = true`.
3. When the first dispute is raised on the shipment (`raise_dispute`), the contract assigns an arbiter from `arbiters_pool`, writes the chosen address to `shipment.arbiter`, clears `("use_pool_arb", shipment_id)`, and advances the global round-robin index `Symbol("arb_pool_idx")`.

### 2.2 Round-Robin Selection with Repeat-Pairing Avoidance (`select_pool_arbiters`)

To prevent collusion or bias from repeatedly pairing the same supplier with the same pool arbiter, the contract tracks the last **`RECENT_ARBITER_WINDOW` (`3`)** arbiters assigned to each supplier at `DataKeyExt3::SupplierRecentArbiters(supplier)`:

- **Pass 1 (Avoid recent pairings):** Starting from `arb_pool_idx % pool.len()`, scans the pool in round-robin order and selects the first candidate who is **not** in `SupplierRecentArbiters(supplier)` (and not in `exclude`).
- **Pass 2 (Small-pool fallback):** If every non-excluded arbiter in the pool is in the supplier's recent window (for example, when `pool.len() <= 3`), Pass 2 relaxes the recent-history filter so selection never deadlocks.
- Each selected arbiter is appended to `SupplierRecentArbiters(supplier)` (evicting the oldest entry once length exceeds `3`), and `arb_pool_idx` is updated to `(last_selected_idx + 1) % pool.len()`.

---

## 3. Recusal, Mutual Rotation, and Backup Failover

### 3.1 Voluntary Recusal (`recuse_arbiter`)

If the assigned arbiter has a conflict of interest or is unavailable on an active shipment (`Created` or `InProgress`):
- **`recuse_arbiter(env, arbiter, shipment_id)`:**
  - Requires `arbiter.require_auth()` and `shipment.arbiter == arbiter`.
  - Selects the next distinct arbiter from `arbiters_pool` (via `select_pool_arbiters` with `exclude = [arbiter]` and supplier repeat-pairing avoidance).
  - Updates `shipment.arbiter = new_arbiter` and emits `arbiter_recused` with `(shipment_id, old_arbiter, new_arbiter)`.

### 3.2 Counterparty Mutual Rotation (`propose_arbiter_rotation`)

The primary buyer and supplier can jointly replace `shipment.arbiter` on an active shipment:
- **`propose_arbiter_rotation(env, caller, shipment_id, new_arbiter)`:**
  - Stored in temporary storage at `DataKey::ArbiterRotation(shipment_id)` (`ArbiterRotationProposal { new_arbiter, buyer_approved, supplier_approved }`).
  - Once both `buyers[0]` and `shipment.supplier` call `propose_arbiter_rotation` with the same `new_arbiter` (who cannot be a buyer or the supplier), `shipment.arbiter` is updated, the proposal is cleared, and `arbiter_rotated` is emitted.

### 3.3 Backup Arbiter Failover (`BackupArbiter`)

A shipment can designate a standby arbiter via `ShipmentOptions.backup_arbiter` (`DataKeyExt::BackupArbiter(shipment_id)`). Failover can be triggered in two ways:

1. **Ledger-Based Inactivity Failover (`activate_backup_arbiter`):**
   - Primary `Admin` configures `set_arbiter_inactivity_threshold(env, admin, threshold_ledgers)` (`DataKeyExt::ArbiterInactivityThreshold`).
   - If a milestone remains `Disputed` at `current_ledger >= dispute_opened_ledger + threshold_ledgers`, anyone can call **`activate_backup_arbiter(env, shipment_id, milestone_index)`** to promote `BackupArbiter` to `shipment.arbiter`, remove `BackupArbiter(shipment_id)`, and emit `arbiter_failover`.
2. **Timestamp-Based Timeout Handoff (`#421` in `resolve_dispute_timeout`):**
   - When `resolve_dispute_timeout(env, shipment_id, milestone_index)` is called after `shipment.dispute_timeout_seconds` Unix seconds have elapsed since `DisputeOpenedAt`:
     - **First timeout call (when `BackupArbiter` is set and `shipment.arbiter != backup`):** Hands the dispute off to `backup`, resets `DisputeOpenedAt` to `env.ledger().timestamp()`, and emits `dispute_timeout_handoff`.
     - **Second timeout call (if the backup also times out, or if no backup was configured):** Settles the milestone automatically using `shipment.default_resolution` (`Resolution::Supplier` or `Resolution::Buyer`) with zero arbiter fee.

---

## 4. Multi-Arbiter Panels & High-Value Auto-Escalation (`#665`)

### 4.1 Explicit Panel vs. High-Value Auto-Escalation

A dispute can be governed by a panel of `N >= 3` arbiters (`DataKeyExt::ArbiterPanel(shipment_id)`) in two ways:

1. **Configured at Shipment Creation:**
   - Pass `ShipmentOptions.arbiter_panel: Vec<Address>` with at least `3` non-blacklisted addresses (recommended odd count such as `3` or `5`).
2. **Automatic High-Value Escalation (`#665`):**
   - Primary `Admin` configures **`set_panel_escalation_threshold(env, admin, min_amount: i128, panel_size: u32)`** (`panel_size >= 3`; `min_amount <= 0` disables auto-escalation; queried via `get_panel_escalation_threshold(env) -> Option<(i128, u32)>`).
   - When `raise_dispute` or `raise_partial_dispute` is called on a shipment that does not already have an `ArbiterPanel`, the contract computes the contested token amount (`gross_milestone` for a full dispute, or `gross_milestone * contested_percent / 100` for a partial dispute).
   - If `disputed_amount >= min_amount`, `maybe_escalate_to_panel` draws `panel_size` distinct arbiters from `arbiters_pool` (using `select_pool_arbiters` and excluding `shipment.arbiter` when the pool has enough members, or including `shipment.arbiter` plus `panel_size - 1` pool arbiters when `pool.len() == panel_size`), saves the panel to `DataKeyExt::ArbiterPanel(shipment_id)`, marks `DataKeyExt5::PanelEscalated(shipment_id, milestone_index) = true`, and emits `dispute_escalated_to_panel`.

### 4.2 Panel Voting (`cast_dispute_vote`)

- Each panel member calls **`cast_dispute_vote(env, arbiter, shipment_id, milestone_index, approve: bool)`**.
- Votes are stored in `DataKeyExt::DisputeVotes(shipment_id, milestone_index)` (`Vec<ArbiterVote>`, queried via `get_dispute_votes`); each panel member can vote once.
- **Simple majority threshold:** `majority = panel.len() / 2 + 1`.
- As soon as either `approve == true` or `approve == false` reaches `majority` votes, the dispute settles immediately and the arbiter fee is divided equally among the panel members who cast a vote (`distribute_panel_fee`).

---

## 5. Arbiter Staking, Performance Stats, Appeals, and Slashing

### 5.1 Performance Statistics (`get_arbiter_stats`)

Every resolution updates the arbiter's `ArbiterStats` at `DataKeyExt::ArbiterStats(arbiter)`:

```rust
pub struct ArbiterStats {
    pub resolved_approved: u32,
    pub resolved_rejected: u32,
    pub total_resolution_ledgers: u64,
    pub overturned_count: u32,
}
```

Query via `get_arbiter_stats(env, arbiter) -> ArbiterStats`.

### 5.2 Arbiter Staking (`#531`)

Arbiters can lock an on-chain bond to back the integrity of their rulings:
- **`deposit_arbiter_stake(env, arbiter, token, amount)`:**
  - Requires `arbiter.require_auth()` and `amount > 0`.
  - Transfers `amount` of `token` from `arbiter` to the contract and adds it to `DataKeyExt4::ArbiterStake(arbiter)` (`(Address, i128)`). Additional deposits by the same arbiter must use the same `token`.
  - Emits `arbiter_stake_deposited`.
- **`get_arbiter_stake(env, arbiter) -> i128`:** Returns the arbiter's current staked balance (`0` if none).

### 5.3 Appeal Overturn Detection & Dual Slashing (`#372` & `#531`)

When a resolved milestone is appealed within the appeal window (`appeal_dispute` — see [`docs/disputes.md`](disputes.md)), the contract records `(original_arbiter, original_approve)` at `DataKeyExt2::DisputeOriginalDecision(shipment_id, milestone_index)` and assigns a new distinct arbiter from `arbiters_pool`.

When the appeal arbiter resolves the dispute (`resolve_dispute` or `resolve_dispute_split`), `apply_appeal_overturn` compares the appeal outcome (`new_approve`, where `supplier_bps > 5_000` counts as `true` for a split) against `original_approve`. If **`new_approve != original_approve`**, the original arbiter's decision was **overturned**, triggering three actions:

1. **Increment Overturn Counter:**
   `orig_stats.overturned_count += 1` is saved to `DataKeyExt::ArbiterStats(original_arbiter)`.
2. **Pool / Reputation Slashing (`#372`):**
   - Configured by primary `Admin` via **`set_max_overturned_before_slash(env, admin, max_overturned: u32)`** (`get_max_overturned_before_slash`).
   - If `max_overturned > 0` and `orig_stats.overturned_count >= max_overturned`, `slash_arbiter` automatically:
     - Removes `original_arbiter` from `arbiters_pool`,
     - Sets `DataKeyExt2::ArbiterSlashed(original_arbiter) = true` (`is_arbiter_slashed(env, arbiter) -> bool`), and
     - Emits `arbiter_slashed` with `(original_arbiter, overturned_count)`.
   - **Reinstatement (`reinstate_arbiter(env, admin, arbiter)`):** Primary `Admin` can clear `ArbiterSlashed(arbiter)`, reset `overturned_count = 0`, re-add `arbiter` to `arbiters_pool`, and emit `arbiter_reinstated`.
3. **Financial Stake Slashing (`#531`):**
   - Configured by primary `Admin` via **`set_arbiter_slash_bps(env, admin, bps: u32)`** (`0..=10_000`, queried via `get_arbiter_slash_bps`).
   - If `slash_bps > 0` and `original_arbiter` holds a stake in `shipment.token`, `maybe_slash_arbiter_stake` deducts:
     $$\text{slash\_amount} = \left\lfloor \frac{\text{stake\_balance} \times \text{slash\_bps}}{10{,}000} \right\rfloor$$
     from `ArbiterStake(original_arbiter)` and transfers `slash_amount` directly to the **wronged party** (`shipment.supplier` if the appeal ruled in the supplier's favor, or `buyers[0]` if the appeal ruled in the buyer's favor), emitting `arbiter_stake_slashed`.

---

## 6. Arbiter Compensation Summary

- **Base or Tiered Arbiter Fee:** Computed on the contested milestone amount using either `shipment.arbiter_fee_bps` (`0..=1_000` bps) or the highest matching tier in `set_arbiter_fee_tiers(env, admin, tiers: Vec<(i128, u32)>)` (`0..=5_000` bps).
- **Single vs. Panel Distribution:** Paid 100% to `shipment.arbiter` in single-arbiter mode, or divided equally (`fee / voter_count`) among voting panel members in panel mode (`distribute_panel_fee`).
- **Forfeited Appeal Fees (`set_appeal_fee`):** When an appellant loses an appeal, the escrowed flat `AppealFee` is transferred to the appeal arbiter (`appeal_fee_forfeited`).
- See [`docs/fees.md`](fees.md) for `set_loser_pays_arbiter_fee` and fee deduction examples.
