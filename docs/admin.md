# Admin and Governance Guide

ChainSettle layers multiple governance and safety controls around protocol administration: a primary admin with two-step succession, granular time-bounded operational roles, an M-of-N multisig council, a supermajority emergency freeze, a parameter timelock, delayed abandoned-escrow recovery, and an on-chain audit log.

All entry points referenced in this guide live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs).

---

## 1. Primary Admin vs. Granular Roles

At initialization (`init(env, admin)`), a single primary `Admin` address is stored at `DataKey::Admin` in instance storage (`get_admin(env) -> Address`). The primary admin implicitly holds every operational permission (`check_has_role` always returns `true` for the primary `Admin`) and can delegate routine operational duties to specialized addresses using the `Role` enum:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Role {
    Pauser,
    FeeManager,
    ComplianceOfficer,
    ArbiterManager,
}
```

### Managing Roles

| Function | Auth | Description |
| --- | --- | --- |
| `grant_role(env, admin, role, address, expires_at_ledger: Option<u32>)` | Primary `Admin` | Grants `role` to `address` (`DataKey::UserRole(role, address)`). If `expires_at_ledger` is `Some(l)`, requires `l > env.ledger().sequence()` and stores the expiry at `DataKeyExt3::RoleExpiry(role, address)`. Emits `role_granted`. |
| `revoke_role(env, admin, role, address)` | Primary `Admin` | Removes both `UserRole(role, address)` and any `RoleExpiry(role, address)`. Emits `role_revoked`. |
| `has_role(env, role, address) -> bool` | Read-only | Returns `true` if `address` is the primary `Admin` or holds an active (unexpired) grant for `role`. Automatically returns `false` once `env.ledger().sequence() >= expiry`. |
| `get_role_expiry(env, role, address) -> Option<u32>` | Read-only | Returns the ledger sequence at which `address`'s grant of `role` expires, or `None` if permanent/unset. |

### Role Capability Matrix

| Role | Authorized Entry Points | Primary-Admin-Only Counterparts (`assert_admin`) |
| --- | --- | --- |
| **`Role::Pauser`** | `pause`, `unpause` | `set_operation_paused`, `propose_emergency_freeze` (single-admin mode) |
| **`Role::FeeManager`** | `set_fee_config`, `schedule_fee_holiday`, `cancel_fee_holiday`, `set_fee_recipients`, `set_shipment_fee_override`, `clear_shipment_fee_override`, `set_referral_fee_bps`, `set_fee_tiers` | `set_long_hold_rebate`, `set_clean_completion_rebate_bps`, `set_arbiter_fee_tiers`, `set_appeal_fee` |
| **`Role::ComplianceOfficer`** | `blacklist_address`, `remove_from_blacklist`, `review_blacklist_appeal`, `add_to_whitelist`, `remove_from_whitelist`, `set_require_mutual_preapproval`, `set_max_allowed_tokens`, `set_buyer_allowed_tokens` | `set_compliance_hold`, `clear_compliance_hold`, `set_auto_blacklist_rule` |
| **`Role::ArbiterManager`** | `add_arbiter_to_pool`, `batch_add_arbiters_to_pool`, `remove_arbiter_from_pool` | `set_max_overturned_before_slash`, `set_arbiter_slash_bps`, `reinstate_arbiter`, `set_arbiter_inactivity_threshold`, `set_panel_escalation_threshold` |

---

## 2. Two-Step Admin Succession

To prevent bricking the contract by transferring `DataKey::Admin` to a mistyped or unready address, ChainSettle uses a two-step nomination handshake:

1. **Nominate (`nominate_admin(env, current_admin, nominee)`):**
   - Requires `current_admin` to be the current primary `Admin`.
   - Stores `nominee` at `DataKey::PendingAdmin` in instance storage and emits `admin_nominated`.
2. **Accept (`accept_admin(env, nominee)`):**
   - Requires `nominee.require_auth()` and verifies `nominee == DataKey::PendingAdmin`.
   - Updates `DataKey::Admin = nominee`, deletes `DataKey::PendingAdmin`, logs `"accept_admin"`, and emits `admin_transferred` with `(old_admin, nominee)`.
3. **Revoke before acceptance (`revoke_nomination(env, current_admin)`):**
   - Allows the current primary `Admin` to cancel a pending nomination (`DataKey::PendingAdmin`) at any time before `accept_admin` is called. Emits `admin_nomination_revoked`.

---

## 3. M-of-N Multisig Governance

The primary admin can configure an M-of-N multisig council (`MultiAdminConfig`) stored at `DataKey::MultiAdminConfig`:

```rust
pub struct MultiAdminConfig {
    pub admins: Vec<Address>,
    pub threshold: u32,
}
```

### Configuring the Council

- **`initialize_multisig_admin(env, admin, admins, threshold)`:**
  - Requires primary `Admin` authorization.
  - Validates `admins` is non-empty, contains no duplicate addresses, and satisfies `1 <= threshold <= admins.len()`.
  - Stores `MultiAdminConfig` in instance storage and emits `multisig_initialized`.
- **`get_multi_admin_config(env) -> Option<MultiAdminConfig>`:**
  - Returns the active multisig configuration, or `None` if the contract operates in single-admin mode.

### Multisig Workflows

Once `MultiAdminConfig` is set, sensitive governance flows require multi-admin consensus:

1. **General Admin Action Proposals (`propose_admin_action` / `get_pending_admin_actions`):**
   - Any council member can call `propose_admin_action(env, caller, action: Symbol) -> u32`, creating an `AdminProposal { proposal_id, action, proposer, approvals, executed }` at `DataKey::AdminProposal(id)` with the caller's initial vote recorded.
   - `get_pending_admin_actions(env) -> Vec<AdminProposal>` returns all unexecuted proposals.
2. **Contract WASM Upgrades (`propose_upgrade` / `approve_upgrade` / `cancel_upgrade`):**
   - Configuring `MultiAdminConfig` permanently disables the single-signer `upgrade` function (`"upgrade multisig is configured; use propose_upgrade/approve_upgrade instead"`).
   - Council members propose a `BytesN<32>` WASM hash via `propose_upgrade` and approve via `approve_upgrade(env, admin, proposal_id)`. Once `approvals.len() >= threshold`, the WASM swap executes automatically. See [`docs/upgrades.md`](upgrades.md) for the complete runbook.
3. **VIP Partner Fee Waivers (`propose_fee_waiver` / `approve_fee_waiver` / `revoke_fee_waiver`):**
   - Council members propose a time-bounded fee discount (`waiver_bps` in `1..=10_000`, `expires_at_ledger`) for a `partner` address via `propose_fee_waiver(env, admin, partner, waiver_bps, expires_at_ledger) -> u64`.
   - Additional signers call `approve_fee_waiver(env, admin, proposal_id)`. When `approvals.len() >= config.threshold`, `FeeWaiver` is written to `DataKeyExt2::FeeWaiver(partner)` and `fee_waiver_activated` is emitted.

---

## 4. Pause Controls vs. Supermajority Emergency Freeze

ChainSettle provides three tiers of circuit-breaking controls:

### 4.1 Per-Operation Pause (`set_operation_paused` / `is_operation_paused`)

The primary `Admin` can selectively pause individual workflows without halting the entire contract:

```bash
stellar contract invoke --id $CONTRACT_ID --network $NETWORK --source $ADMIN \
  -- set_operation_paused --admin $ADMIN_ADDRESS --op confirm --paused true
```

Supported operation symbols (`assert_operation_not_paused`):
- `"create"` — blocks `create_shipment`, `batch_create_shipments`, `create_shipment_from_template`
- `"confirm"` — blocks `confirm_milestone`, `batch_confirm_milestones`, `confirm_milestone_graded`, `confirm_partial_quantity`
- `"dispute"` — blocks `raise_dispute`, `raise_partial_dispute`
- `"advance"` — blocks `request_advance`, `approve_advance`
- `"claim"` — blocks `claim_auto_confirmation`, `claim_deadline_refund`, `claim_collateral`, `claim_payout`

### 4.2 Global Pause (`pause` / `unpause` / `is_paused`)

- Authorized by **`Role::Pauser`** or the primary **`Admin`**.
- Sets `DataKey::Paused = true` in instance storage (`assert_not_paused` panics with `ContractError::ContractPaused`).
- Blocks all state-changing shipment, milestone, payment, and dispute operations while leaving read-only queries and admin recovery/unpause actions available.

### 4.3 Supermajority Emergency Freeze (`#402`)

For severe security incidents, ChainSettle supports a supermajority **Emergency Freeze** (`DataKeyExt2::EmergencyFrozen`), which is tracked separately from `DataKey::Paused` (`assert_not_paused` checks `EmergencyFrozen` first and panics with `"contract is emergency frozen"`). Even if a `Pauser` role holder calls `unpause`, the contract remains halted while `EmergencyFrozen == true`.

| Function | Auth | Description |
| --- | --- | --- |
| `set_freeze_supermajority_bps(env, admin, bps)` | Primary `Admin` | Configures the supermajority threshold in `[5_001, 10_000]` bps (default `DEFAULT_FREEZE_SUPERMAJORITY_BPS = 8_000` = 80%). |
| `get_freeze_supermajority_bps(env) -> u32` | Read-only | Returns the active supermajority threshold in bps. |
| `propose_emergency_freeze(env, caller) -> u64` | Multisig member (or primary `Admin`) | Creates an `EmergencyFreezeProposal` (`is_unfreeze: false`). In single-admin mode (no `MultiAdminConfig`), activates the freeze immediately and returns `0`. |
| `approve_emergency_freeze(env, caller, proposal_id)` | Multisig member | Adds an approval. Activates `EmergencyFrozen = true` once approvals reach $\lceil N \times \text{bps} / 10{,}000 \rceil = (N \times \text{bps} + 9{,}999) / 10{,}000$. |
| `propose_emergency_unfreeze(env, caller) -> u64` | Multisig member (or primary `Admin`) | Proposes lifting an active emergency freeze (`is_unfreeze: true`). |
| `approve_emergency_unfreeze(env, caller, proposal_id)` | Multisig member | Adds an approval; clears `EmergencyFrozen = false` once the same supermajority threshold is reached. |
| `is_emergency_frozen(env) -> bool` | Read-only | Returns `true` if the emergency freeze is active. |
| `get_emergency_freeze_proposal(env, proposal_id)` / `get_emergency_unfreeze_proposal(env, proposal_id)` | Read-only | Inspects a pending freeze or unfreeze proposal. |

---

## 5. Governance Parameter Timelock

To give buyers and suppliers advance notice before critical economic or risk limits change, the primary `Admin` can enforce a ledger timelock (`DataKeyExt::TimelockDuration`):

1. **Configure the delay (`set_timelock_duration(env, admin, ledgers)`):**
   Sets the required waiting period in ledgers (`DataKeyExt::TimelockDuration`) and emits `timelock_duration_set`.
2. **Queue a parameter change (`propose_param_change(env, admin, param: Symbol, new_value: i128)`):**
   Stores `(new_value, current_ledger + timelock)` under `DataKeyExt::PendingParamChange(param)` and emits `param_change_proposed` with `(param, new_value, effective_ledger)`.
3. **Execute after expiry (`execute_param_change(env, admin, param: Symbol)`):**
   - Verifies `env.ledger().sequence() >= effective_ledger` (panics with `"TimelockNotExpired"` if called early).
   - Applies the update based on `param`:
     - `"fee_config"` — updates `FeeConfig.fee_bps` to `new_value as u32` (preserving `treasury`).
     - `"max_shipment_value"` — updates `DataKey::MaxShipmentValue` to `new_value`.
     - `"circuit_breaker_limit"` — updates `DataKey::CircuitBreakerLimit` to `new_value`.
   - Deletes `PendingParamChange(param)` and emits `param_change_executed`.
4. **Cancel a queued change (`cancel_param_change(env, admin, param: Symbol)`):**
   Deletes `PendingParamChange(param)` before execution and emits `param_change_cancelled`.

---

## 6. Emergency Recovery of Abandoned Escrows

If an active shipment (`ShipmentStatus::Created` or `InProgress`) is abandoned by all counterparties for more than **`RECOVERY_THRESHOLD_LEDGERS` (`12,614,400` ledgers, ~2 years at 5 seconds/ledger)** after `shipment.created_at`, the primary `Admin` can recover the locked tokens so they are not permanently stranded:

- **Preconditions:**
  - `shipment.status` must be `Created` or `InProgress`.
  - `env.ledger().sequence() > shipment.created_at + 12_614_400` (otherwise fails with `ContractError::RecoveryTooEarly`).
- **Immediate vs. Delayed Cancellable Recovery:**
  - `set_recovery_delay(env, admin, ledgers)` configures `DataKey::RecoveryDelayLedgers` (`0` by default).
  - `propose_emergency_recover(env, admin, shipment_id)`:
    - If `RecoveryDelayLedgers == 0` (or via legacy `emergency_recover(env, admin, shipment_id)`), executes immediately.
    - If `RecoveryDelayLedgers > 0`, stores `RecoveryProposal { shipment_id, proposed_by, effective_ledger: current_ledger + delay }` at `DataKey::PendingRecovery(shipment_id)` and emits `emergency_recover_proposed`.
  - `execute_emergency_recover(env, admin, shipment_id)`:
    - Requires `env.ledger().sequence() >= proposal.effective_ledger`.
    - Transfers remaining unreleased escrow (`total_amount - released_amount - total_advanced_amount`) to `admin`, refunds any withheld retainage and warranty balances to `buyers[0]`, sets `shipment.status = Cancelled` (`CancellationReason::AdminEmergencyRecovery`), decrements active shipment counters, and emits `emergency_recovered`.
  - `cancel_emergency_recover(env, admin, shipment_id)` cancels a pending recovery proposal (`get_pending_recovery(env, shipment_id)` inspects it).

---

## 7. On-Chain Admin Audit Log

Every privileged administrative call records an `AdminLogEntry` in a bounded ring buffer at `DataKey::AdminActionLog` (capped at `AUDIT_LOG_MAX_ENTRIES = 50`; oldest entry is evicted when full):

```rust
pub struct AdminLogEntry {
    pub caller: Address,
    pub action: Symbol,
    pub ledger: u32,
}
```

- **`get_admin_log(env) -> Vec<AdminLogEntry>`:** Returns all retained audit log entries in chronological order.
- **`get_admin_log_filtered(env, action: Option<Symbol>, from_ledger: u32, to_ledger: u32) -> Vec<AdminLogEntry>`:** Filters audit entries by action name (`Some(symbol_short!("pause"))`, etc.) and inclusive ledger range `[from_ledger, to_ledger]`.
