# Admin and governance guide

This guide is for contract operators. Function names and authorization
requirements below reflect the current implementation in
[`lib.rs`](../contracts/chainsetttle/src/lib.rs). Every state-changing
protected administrative call requires the caller address to authorize the
transaction; holding a role does not replace that signature. Read-only queries
are permissionless, and `migrate` is a permissionless state-changing exception.

## Authority model

There are three distinct authority mechanisms: the primary `Admin`, delegated
operational `Role`s, and the optional `MultiAdminConfig`. They are not
interchangeable. In particular, configuring multisig does not replace the
primary Admin or put every admin function behind a threshold.

### Role matrix

| Authority | Functions checked by the contract | Notes |
| --- | --- | --- |
| Primary `Admin` | Every entrypoint that calls `assert_admin`, plus all four scoped roles below | Set by `init`; the current Admin is implicitly treated as holding every `Role`. |
| `Pauser` | `pause`, `unpause` | Can change the contract-wide pause flag, not operation-specific pause flags. |
| `FeeManager` | `set_fee_config`, `schedule_fee_holiday`, `cancel_fee_holiday`, `set_fee_recipients`, `set_referral_fee_bps`, `set_fee_tiers`, `set_shipment_fee_override`, `clear_shipment_fee_override` | Does not grant unrelated Admin-only configuration or multisig approval rights. |
| `ComplianceOfficer` | `blacklist_address`, `remove_from_blacklist`, `review_blacklist_appeal`, `add_to_whitelist`, `remove_from_whitelist`, `set_require_mutual_preapproval`, `set_max_allowed_tokens`, `set_buyer_allowed_tokens` | Does not grant the Admin-only `set_compliance_hold` or `clear_compliance_hold`. |
| `ArbiterManager` | `add_arbiter_to_pool`, `batch_add_arbiters_to_pool`, `remove_arbiter_from_pool` | A slashed arbiter must be reinstated by the primary Admin with `reinstate_arbiter` before it can be re-added. |

`grant_role` and `revoke_role` are primary-Admin-only. `grant_role` may set
`expires_at_ledger` to a future ledger, or `None` for a permanent grant.
Time-limited roles stop working when the current ledger reaches the expiry
ledger. Re-granting updates the expiry; using `None` clears an earlier expiry.
`has_role` checks effective access, and `get_role_expiry` reads a grant's
recorded expiry. The primary Admin remains authorized for every role even if an
explicit grant to that address expires or is revoked.

## Bootstrap and role delegation

1. Deploy the contract and call `init(admin)` with the intended primary Admin.
2. Before relying on role separation, use `grant_role(admin, role, address,
   expires_at_ledger)` to grant only the needed role.
3. Check effective access with `has_role`; check a time-bound grant with
   `get_role_expiry`.
4. Revoke access with `revoke_role` when it is no longer needed. Revoke each
   role separately if an address has several.

> **Critical initialization warning:** `init` has no already-initialized guard
> in the current code. A later authorized call can overwrite the primary Admin
> and reset several instance defaults, including the global pause flag. Ensure
> initialization is performed once as part of the trusted deployment procedure
> and that the initializer cannot be invoked again by an untrusted party.

## Routine multisig

The primary Admin configures the member list and threshold with
`initialize_multisig_admin(admin, admins, threshold)`. The threshold must be
between 1 and the number of entries. Use distinct addresses: the code does not
validate uniqueness, while approvals themselves must be from distinct
addresses. The primary Admin is not automatically a multisig member; include
it explicitly if it must participate.

This configuration is used by:

- `propose_upgrade` / `approve_upgrade` for WASM upgrades;
- `propose_fee_waiver` / `approve_fee_waiver` for partner fee waivers;
- `propose_emergency_freeze` / `approve_emergency_freeze` and the matching
  unfreeze functions, with a stricter supermajority.

The proposer counts as the first approval. With threshold 1, an upgrade or fee
waiver executes immediately. For upgrades, any registered multisig Admin can
cancel a pending proposal with `cancel_upgrade`; inspect it with
`get_upgrade_proposal`. Fee-waiver proposals are inspected with
`get_fee_waiver_proposal`.

The primary Admin can call `initialize_multisig_admin` again to replace the
configured list and threshold. This does not change the primary Admin and does
not make other `assert_admin` functions multisig-gated. In particular, role
grants, timelock operations, pause controls, recoveries, and Admin succession
remain controlled by the primary Admin unless the entrypoint specifically
checks the multisig list.

`propose_admin_action` is **not** a general-purpose execution mechanism:
after its threshold is reached, the current `execute_admin_action` only emits
an event. It does not dispatch the requested operation or change contract
configuration. Do not use it as an approval gate for pause, upgrade, or any
other action.

## Upgrading the contract

1. Review and upload the intended WASM, then verify the resulting hash out of
   band.
2. If `MultiAdminConfig` is not configured, the primary Admin can call
   `upgrade(admin, new_wasm_hash)` directly.
3. If multisig is configured, `upgrade` is disabled. A registered member calls
   `propose_upgrade`; other distinct members call `approve_upgrade` until the
   configured threshold is reached. The upgrade executes automatically at that
   point. A registered member can cancel a pending proposal with
   `cancel_upgrade`.
4. Call `migrate` only when the release's migration instructions require it,
   then verify the deployed behavior and `version`/`storage_schema_version`.

`migrate` currently has no authorization check; it only writes the current
storage schema version. The configured upgrade threshold applies only to
`propose_upgrade` / `approve_upgrade`, not to every primary-Admin operation.

## Pause controls and incidents

There are separate global, operation-specific, per-shipment, and compliance
controls:

| Control | How to set or clear it | Scope and checks |
| --- | --- | --- |
| Global pause | `pause` / `unpause` | Primary Admin or `Pauser`; blocks state-changing paths that check the global pause flag. |
| Operation pause | `set_operation_paused(admin, op, paused)` | Primary Admin only. `op` must be `create`, `confirm`, `dispute`, `advance`, or `claim`. |
| Emergency freeze | Emergency-freeze proposal procedures below | Contract-wide freeze, separately tracked from `pause`. |
| Shipment pause | Buyer or supplier calls `request_shipment_pause`; the other party calls `approve_shipment_pause`. Resume uses `resume_shipment` from each party in turn. | Only that active shipment; approval freezes its deadlines, and resumption shifts them by the paused duration. |
| Compliance hold | `set_compliance_hold` / `clear_compliance_hold` | Primary Admin only; freezes state-changing operations on one shipment while review is pending. |

For an incident:

1. Inspect `is_paused`, `is_emergency_frozen`, `is_operation_paused(op)`, and
   `is_shipment_paused(shipment_id)` as applicable. `is_paused` reports only
   the global pause flag; `is_operation_paused` reports true when the global
   pause or emergency freeze overrides an operation.
2. Use `pause` for a broad, reversible stop; use `set_operation_paused` to
   restrict only one supported operation. A `Pauser` cannot set operation
   flags; that requires the primary Admin.
3. For an emergency freeze with multisig configured, call
   `propose_emergency_freeze`, then have distinct configured members call
   `approve_emergency_freeze`. The proposer is counted. The default required
   supermajority is 8,000 bps (80%), rounded up from the member count; it is
   separate from the routine multisig threshold.
4. Lift a freeze with `propose_emergency_unfreeze` and
   `approve_emergency_unfreeze`, using the same supermajority. Without
   multisig configured, either proposal call immediately activates or lifts
   the freeze for the primary Admin and returns proposal id 0.
5. Check every control before resuming service. Clearing the global pause does
   not lift an emergency freeze or an operation pause; lifting a freeze does
   not clear the global pause. Per-shipment pauses require the counterparty's
   cooperation, while compliance holds require the primary Admin to call
   `clear_compliance_hold`.

The primary Admin configures the freeze supermajority with
`set_freeze_supermajority_bps` (1 through 10,000 bps) and reads it with
`get_freeze_supermajority_bps`. Proposal state can be checked with
`get_emergency_freeze_proposal` and `get_emergency_unfreeze_proposal`.
`set_pause_min_notice_ledgers` can require shipment-pause requests to be made a
minimum number of ledgers before the next pending milestone deadline; zero
disables that check.

## Governance timelock

The timelock currently covers only parameter changes proposed through
`propose_param_change`. It is not a universal delay or a multisig approval
system.

1. Set a positive delay with `set_timelock_duration(admin, ledgers)`.
2. Propose a supported parameter using its exact symbol:
   `fee_config`, `max_shipment_value`, or `circuit_breaker_limit`.
3. Monitor the `param_change_proposed` event for the effective ledger.
4. At or after that ledger, call `execute_param_change`; before then, cancel
   with `cancel_param_change`.

A zero delay (the default) makes the proposal immediately executable. The
primary Admin can change the delay at any time. Direct setters such as
`set_fee_config`, `set_max_shipment_value`, and `set_circuit_breaker` remain
available outside this proposal flow, so setting a timelock does not force all
changes through it. The `fee_config` timelock branch changes `fee_bps` only
when a fee config already exists. It casts the proposed `i128` to `u32` without
the range validation in `set_fee_config`; keep this value between 0 and 1,000
inclusive. Use `set_fee_config` for the validated fee-configuration path.

## Emergency recovery of abandoned shipments

Recovery is for an active shipment that has exceeded the hard-coded age
threshold. In production, the shipment must be more than 12,614,400 ledgers old;
the condition is strictly greater than its creation ledger plus that threshold.
Recovery transfers the remaining shipment escrow to the primary Admin and
marks the shipment Cancelled. It is not a general sweep of contract balances.
Recovery entrypoints also check the global pause and emergency freeze, so clear
those controls before attempting recovery.

Preferred delayed procedure:

1. Set a positive delay with `set_recovery_delay(admin, ledgers)`.
2. After verifying the shipment and recovery eligibility, call
   `propose_emergency_recover(admin, shipment_id)`.
3. Review the pending proposal with `get_pending_recovery`.
4. At or after `effective_ledger`, a primary Admin calls
   `execute_emergency_recover`. A primary Admin can instead cancel the pending
   proposal with `cancel_emergency_recover`.

If the configured delay is zero, `propose_emergency_recover` executes
immediately. The legacy `emergency_recover` also remains available as an
immediate, single-Admin path even when a positive recovery delay is configured;
the delay does not gate or disable it. Recovery is not multisig-gated. The
recovery logic transfers remaining escrow and processes cancellation holdbacks,
but does not explicitly sweep supplier collateral.

## Admin succession

1. The current primary Admin nominates the successor with
   `nominate_admin(current_admin, nominee)`.
2. The nominee calls `accept_admin(nominee)` with their own authorization. Until
   acceptance, the current Admin retains authority. The wrong address cannot
   accept the nomination.
3. If the nomination should be abandoned before acceptance, the current Admin
   calls `revoke_nomination`.
4. After acceptance, review explicit role grants and the multisig list. Role
   grants are stored separately and are not automatically revoked, and the
   `MultiAdminConfig` is not automatically updated. The new primary Admin can
   revoke former-operator grants and call `initialize_multisig_admin` to update
   the multisig membership.

Use `get_admin_log` or `get_admin_log_filtered` and emitted governance events
to review administrative activity.

## Operational cautions

- `init` is not one-shot guarded; protect it as noted above.
- Multisig gates only the explicitly listed proposal flows. Primary-Admin
  operations are not automatically converted to multisig.
- `propose_admin_action` records approvals but does not execute its described
  action.
- A configured recovery delay does not disable `emergency_recover`.
- `migrate` is permissionless in the current code.

Relevant tests include
[test_role_expiry.rs](../contracts/chainsetttle/src/test_role_expiry.rs),
[test_operation_pause.rs](../contracts/chainsetttle/src/test_operation_pause.rs),
[test_emergency_freeze.rs](../contracts/chainsetttle/src/test_emergency_freeze.rs),
[test_upgrade_multisig.rs](../contracts/chainsetttle/src/test_upgrade_multisig.rs),
[test_new_issues.rs](../contracts/chainsetttle/src/test_new_issues.rs), and
[test_cancellation_reason.rs](../contracts/chainsetttle/src/test_cancellation_reason.rs).
