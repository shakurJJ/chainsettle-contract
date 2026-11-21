# ChainSettle Events

Every event the `ChainSettleContract` publishes with `env.events().publish`,
audited against [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs),
the source of truth. Function names below are the exact Rust function names.

The contract emits two families of events:

1. **Canonical lifecycle events**: seven indexer-friendly events with a
   two-topic form and a self-describing `Map` payload. See
   [Canonical lifecycle events](#canonical-lifecycle-events).
2. **Granular events**: one or more per state-changing function, with the
   event name as the first topic and a positional tuple (or single value) as
   data. See [Event reference](#event-reference).

Some names appear in both families (`shipment_created`, `proof_submitted`,
`milestone_confirmed`, `dispute_resolved`, `shipment_cancelled`). Tell them
apart by the first topic: canonical events always start with
`Symbol("chainsettle")`.

`test_every_published_topic_is_documented` in
[`test_events_doc.rs`](../contracts/chainsetttle/src/test_events_doc.rs)
fails if a literal topic passed to `publish` is missing from this file.

## Conventions

- **Topics.** Granular events use `(Symbol(<name>), <key>...)`. The key is
  usually `shipment_id` (`String`). Admin-wide settings often have no key.
- **Data.** Shown as the Rust expression the contract publishes, so field
  names match the local variables in `lib.rs`. `.clone()`/`.unwrap()` calls
  are omitted. `()` means an empty payload.
- **Placeholders used in the tables:**

| Placeholder | Type | Meaning |
|---|---|---|
| `ledger_seq` | `u32` | `env.ledger().sequence()` at emission |
| `primary_buyer` | `Address` | `shipment.buyers.get(0)`, the first buyer |
| `<name>_len` | `u32` | Length of the named `Vec`/`Map` (`x.len()`) |
| `shipment_id`, `proof_hash`, `note` | `String` | |
| `milestone_index`, `idx`, `*_ledgers`, `*_ledger`, `*_bps`, `approvals_count` | `u32` | |
| `amount`, `payment`, `*_amount`, `fee_amount`, `refund`, `cap`, `limit` | `i128` | In the escrow token's smallest unit |
| `proposal_id` | `u64` | Upgrade, freeze, unfreeze and fee-waiver proposals |
| `buyer`, `supplier`, `arbiter`, `admin`, `caller`, `token` | `Address` | |

## Canonical lifecycle events

```
topics: (Symbol("chainsettle"), Symbol(<event_name>))
data:   Map<Symbol, Val>
```

The payload is a `Map<Symbol, Val>` keyed by the field names below, so new
optional fields can be added without breaking positional decoders. They are
published by private `emit_*` helpers. The "Emitted by" lists give the public
functions that reach them.

### `shipment_created`

Published by `emit_shipment_created`. Emitted by `create_shipment`, after the
shipment is persisted and escrow is funded.

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The new shipment's identifier. |
| `buyer` | `Address` | The primary (first) buyer. |
| `supplier` | `Address` | The supplier. |
| `arbiter` | `Address` | The assigned arbiter (or the pool sentinel arbiter). |
| `token` | `Address` | The SAC token address used for escrow. |
| `amount` | `i128` | Total shipment value locked in escrow. |

### `proof_submitted`

Published by `emit_milestone_proof_submitted`. Emitted by `submit_proof` (on
every submission and resubmission) and by `attest_proof` (when dual
attestation completes).

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The shipment identifier. |
| `milestone_index` | `u32` | Milestone the proof is for. |
| `proof_hash` | `String` | The submitted proof hash or off-chain reference. |
| `supplier` | `Address` | The shipment's supplier (`Shipment.supplier`), not necessarily the caller. Consortium members and substitutes appear in the granular `proof_submitted` event's `caller` field. |

### `milestone_confirmed`

Published by `emit_milestone_confirmed`. Emitted whenever a milestone payment is
released through the confirmation path: `confirm_milestone`,
`release_held_payment` (once the holdback window expires),
`batch_confirm_milestones` (once per milestone) and `claim_auto_confirmation`.

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The shipment identifier. |
| `milestone_index` | `u32` | Index of the confirmed milestone. |
| `payout_amount` | `i128` | Gross milestone payment, before fee, logistics and penalty deductions. |

### `dispute_opened`

Published by `emit_dispute_opened`. Emitted by `raise_dispute`,
`raise_partial_dispute`, and `report_condition_breach` (when the oracle breach
threshold opens a dispute).

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The shipment identifier. |
| `milestone_index` | `u32` | Index of the disputed milestone. |
| `buyer` | `Address` | The buyer who raised the dispute (primary buyer for oracle-opened disputes). |

### `dispute_resolved`

Published by `emit_dispute_resolved`. Emitted by `resolve_dispute`,
`finalize_dispute_resolution`, `resolve_dispute_timeout`, `accept_mediation`
and `cast_dispute_vote` (when a panel reaches a decision).

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The shipment identifier. |
| `milestone_index` | `u32` | Index of the resolved milestone. |
| `resolution` | `Symbol` | `"supplier"` if the disputed funds were released to the supplier, `"buyer"` if refunded/withheld from the supplier, `"split"` if `resolve_dispute_split` divided them by percentage (see [dispute-split-resolution.md](dispute-split-resolution.md)). |
| `resolver` | `Address` | The arbiter who resolved the dispute (for a timeout resolution, the shipment's designated arbiter). |

### `shipment_cancelled`

Published by `emit_shipment_cancelled`. Emitted by `cancel_shipment`,
`batch_cancel_shipments`, `supplier_cancel`, `claim_deadline_refund`,
`sweep_unclaimed_refund`, `cancel_for_blacklisted_supplier`, and the emergency
recovery path (`emergency_recover`, `propose_emergency_recover`,
`execute_emergency_recover`).

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The shipment identifier. |
| `refund_amount` | `i128` | Amount refunded: the unconfirmed escrow balance, net of any cancellation fee. Funds already released for confirmed milestones are never included. For `AdminEmergencyRecovery` it is the amount recovered to the admin. |
| `reason` | `Symbol` | `"BuyerCancelled"`, `"SupplierCancelled"`, `"DeadlineRefund"`, `"AdminEmergencyRecovery"` or `"BlacklistedSupplier"`. Additive third field. The typed `CancellationReason` is also stored on `Shipment.cancellation_reason`. |

### `shipment_completed`

Published by `emit_shipment_completed` from `settle_on_completion`. Emitted when
the last milestone settles and the shipment becomes `Completed`: from
`confirm_milestone`, `release_held_payment`, `batch_confirm_milestones`,
`claim_auto_confirmation`, `resolve_dispute`, `finalize_dispute_resolution`,
`resolve_dispute_timeout`, `accept_mediation` and `cast_dispute_vote`.

| Field | Type | Description |
|---|---|---|
| `shipment_id` | `String` | The shipment identifier. |
| `total_paid` | `i128` | Total released to the supplier (`Shipment.released_amount` at completion). |

## Changed and inconsistent payloads

Changes found in this audit. Indexers decoding positional tuples should check
these first.

| Event | Status | Details |
|---|---|---|
| `chainsettle` / `dispute_resolved` | **Changed** (additive) | New `reason` field (`ResolutionReason`). Not in earlier versions of this doc. |
| `chainsettle` / `shipment_cancelled` | **Changed** (additive) | `reason` can now be `"BlacklistedSupplier"` (from `cancel_for_blacklisted_supplier`). |
| `milestone_confirmed` (granular) | **Changed** (additive) | `confirm_milestone` appends `applied_fee_bps` as a 9th field (#399). Decoders reading the first 8 fields still work. |
| `milestone_confirmed` (granular) | **Shape differs by emitter** | `batch_confirm_milestones` publishes 7 fields with no `penalty_deducted` or `applied_fee_bps`, so field 4 is `supplier` rather than `penalty_deducted`. Decode by tuple length. |
| `dispute_resolved` (granular) | **Shape differs by emitter** | `resolve_dispute` publishes 6 fields, ending with `reason`. Panel decisions (`cast_dispute_vote`) publish 5, without `reason`. |
| `proof_submitted` (granular) | **Shape differs by emitter** | `submit_proof` sends `(milestone_index, proof_hash, proof_type, caller, current_ledger)`. `attest_proof` sends the first attester in the `caller` position. |
| `proof_resubmitted` | Split from `proof_submitted` | `submit_proof` publishes `proof_resubmitted` instead of `proof_submitted` when the milestone already had a proof. Same payload. |
| `shipment_cancelled` (granular) | **Shape differs by emitter** | `cancel_shipment_internal` publishes `(refund, cancel_fee, buyer, ledger_seq)`. `cancel_for_blacklisted_supplier` publishes `(unreleased, 0, buyer, ledger_seq)`. |
| `grade_settled` | **Meaning differs by emitter** | Same 5-field shape. From `release_held_payment` the 4th and 5th fields are `(payment, grade_refund)`. From `resolve_dispute` they are `(payment - contested, buyer_refund)`. |
| `emergency_freeze_activated`, `emergency_freeze_lifted` | **Shape differs by emitter** | Single-admin path (`propose_emergency_freeze` / `propose_emergency_unfreeze` with threshold 1) sends `(admin, ledger_seq)`. Multisig execution sends `proposal_id` (`u64`). |
| `dispute_raised` | Additional emitter | Also published when an oracle breach opens a dispute (`report_condition_breach`). |
| `arbiter_pool_updated` | Note | First data element is the symbol `"added"` or `"removed"`. |
| `payout_reference` | **New** (additive, #752) | Emitted next to each payment release when the supplier has set a payout reference with `set_payout_reference` (max 64 bytes). Existing release payloads are unchanged; join on `shipment_id` + `milestone_index`. `claim_payout` (batched mode) aggregates across shipments, so the reference is emitted when the milestone is credited, not at claim time. |
| `shipment_fee_override_cleared` | **Inconsistent** with `_set` | `shipment_id` is in data, not topics. |
| `token_min_shipment_value_cleared`, `token_max_shipment_value_cleared` | **Inconsistent** with `_set` | `token` is in data, not topics. |
| `contract_upgraded` / `upgrade_executed` | Two paths | `upgrade` emits `contract_upgraded`. Multisig upgrades emit `upgrade_executed`. |
| `max_expiry_ext_set` | Note | Abbreviated name for `set_max_expiry_extension_ledgers` (32-character symbol limit). |

No event has been removed. None is formally deprecated: the shape differences
above are kept for backward compatibility.

## Event reference

All granular events, sorted by name. Where a row is published from a private
helper, "Emitted by" names the helper and the public functions that reach it.

| Event | Topics | Data | Emitted by |
|---|---|---|---|
| `address_outflow_limit_set` | (`address_outflow_limit_set`, `address`) | `(limit, window_ledgers)` | `set_address_outflow_limit` |
| `admin_action_executed` | (`admin_action_executed`, `action_id`) | `operation` | `execute_admin_action` (internal; via `propose_admin_action`) |
| `admin_action_proposed` | (`admin_action_proposed`, `action_id`) | `approvals_len` | `propose_admin_action` |
| `admin_nominated` | (`admin_nominated`) | `nominee` | `nominate_admin` |
| `admin_transferred` | (`admin_transferred`) | `(old_admin, nominee)` | `accept_admin` |
| `advance_approved` | (`advance_approved`, `shipment_id`) | `(milestone_index, advance_amount, shipment.supplier)` | `approve_advance` |
| `advance_requested` | (`advance_requested`, `shipment_id`) | `(milestone_index, advance_percent, caller)` | `request_advance` |
| `allowed_token_added` | (`allowed_token_added`) | `token` | `add_allowed_token` |
| `allowed_token_removed` | (`allowed_token_removed`) | `token` | `remove_allowed_token` |
| `amendment_proposed` | (`amendment_proposed`, `shipment_id`) | `(milestone_index, new_percent)` | `propose_amendment` |
| `appeal_fee_forfeited` | (`appeal_fee_forfeited`, `shipment_id`) | `(milestone_index, recipient, amount)` | `settle_appeal_fee` (internal; via `resolve_dispute`) |
| `appeal_fee_refunded` | (`appeal_fee_refunded`, `shipment_id`) | `(milestone_index, recipient, amount)` | `settle_appeal_fee` (internal; via `resolve_dispute`) |
| `appeal_fee_set` | (`appeal_fee_set`) | `amount` | `set_appeal_fee` |
| `appeal_window_ledgers_set` | (`appeal_window_ledgers_set`) | `ledgers` | `set_appeal_window_ledgers` |
| `approved_buyer_added` | (`approved_buyer_added`, `supplier`) | `buyer` | `add_approved_buyer` |
| `approved_buyer_removed` | (`approved_buyer_removed`, `supplier`) | `buyer` | `remove_approved_buyer` |
| `arbiter_availability_set` | (`arbiter_availability_set`, `arbiter`) | `available` | `set_arbiter_availability` |
| `arbiter_failover` | (`arbiter_failover`, `shipment_id`) | `(old_arbiter, backup)` | `activate_backup_arbiter` |
| `arbiter_fee_allocated` | (`arbiter_fee_allocated`, `shipment_id`) | `(milestone_index, arbiter_fee, cover, arbiter_fee - cover)` | `loser_pays_cover` (internal; via `cast_dispute_vote`, `finalize_dispute_resolution`, `resolve_dispute`) |
| `arbiter_fee_rate_set` | (`arbiter_fee_rate_set`, `arbiter`) | `bps` (0 = declaration cleared) | `set_arbiter_fee_rate` |
| `arbiter_inactivity_threshold_set` | (`arbiter_inactivity_threshold_set`) | `threshold_ledgers` | `set_arbiter_inactivity_threshold` |
| `arbiter_pool_updated` | (`arbiter_pool_updated`) | `("added", arbiter)` | `add_arbiter_to_pool` |
| `arbiter_pool_updated` | (`arbiter_pool_updated`) | `("removed", arbiter)` | `remove_arbiter_from_pool` |
| `arbiter_recused` | (`arbiter_recused`, `shipment_id`) | `(arbiter, new_arbiter)` | `recuse_arbiter` |
| `arbiter_reinstated` | (`arbiter_reinstated`) | `arbiter` | `reinstate_arbiter` |
| `arbiter_repetition_size_set` | (`arbiter_repetition_size_set`) | `size` | `set_arbiter_history_size` |
| `arbiter_rotated` | (`arbiter_rotated`, `shipment_id`) | `new_arbiter` | `propose_arbiter_rotation` |
| `arbiter_rotation_proposed` | (`arbiter_rotation_proposed`, `shipment_id`) | `new_arbiter` | `propose_arbiter_rotation` |
| `arbiter_slash_bps_set` | (`arbiter_slash_bps_set`) | `bps` | `set_arbiter_slash_bps` |
| `arbiter_slashed` | (`arbiter_slashed`) | `(arbiter, overturned_count)` | `slash_arbiter` (internal; via `resolve_dispute`) |
| `arbiter_stake_deposited` | (`arbiter_stake_deposited`, `arbiter`) | `(token, amount, new_amt)` | `deposit_arbiter_stake` |
| `arbiter_stake_slashed` | (`arbiter_stake_slashed`, `original_arbiter`) | `(wronged_party, slash_amt, remaining, slash_bps)` | `maybe_slash_arbiter_stake` (internal; via `resolve_dispute`) |
| `archive_threshold_set` | (`archive_threshold_set`) | `ledgers` | `set_archive_threshold` |
| `assigned_payout` | (`assigned_payout`, `shipment_id`) | `(milestone_index, financier, net_amount)` | `pay_milestone_to_payees` (internal; via `accept_mediation`, `batch_confirm_milestones`, `cast_dispute_vote`, `claim_auto_confirmation`, `confirm_milestone`, `confirm_partial_quantity`, `finalize_dispute_resolution`, `release_held_payment`, `resolve_dispute`) |
| `auto_blacklist_rule_set` | (`auto_blacklist_rule_set`) | `(max_cancelled, max_disputed)` | `set_auto_blacklist_rule` |
| `auto_confirm_threshold_set` | (`auto_confirm_threshold_set`) | `window_ledgers` | `set_auto_confirm_threshold` |
| `auto_confirmation_claimed` | (`auto_confirmation_claimed`, `shipment_id`) | `(milestone_index, payment, fee_amount, penalty_deducted)` | `claim_auto_confirmation` |
| `batch_cancel_reason` | (`batch_cancel_reason`, `shipment_id`) | `r` | `cancel_shipment_internal` (internal; via `batch_cancel_shipments`, `cancel_shipment`) |
| `blacklist_appeal_filed` | (`blacklist_appeal_filed`, `address`) | `appeal.filed_ledger` | `appeal_blacklist` |
| `blacklist_appeal_reviewed` | (`blacklist_appeal_reviewed`, `address`) | `approve` | `review_blacklist_appeal` |
| `buyer_allowed_tokens_set` | (`buyer_allowed_tokens_set`, `buyer`) | `tokens` | `set_buyer_allowed_tokens` |
| `buyer_dispute_cap_cleared` | (`buyer_dispute_cap_cleared`, `buyer`) | `()` | `clear_buyer_dispute_cap` |
| `buyer_dispute_cap_set` | (`buyer_dispute_cap_set`, `buyer`) | `limit` | `set_buyer_dispute_cap` |
| `buyer_spending_limit_set` | (`buyer_spending_limit_set`, `buyer`) | `(limit, window_ledgers)` | `set_buyer_spending_limit` |
| `buyer_transferred` | (`buyer_transferred`, `shipment_id`) | `(current_buyer, new_buyer)` | `transfer_buyer` |
| `circuit_breaker_set` | (`circuit_breaker_set`) | `(limit, window_ledgers)` | `set_circuit_breaker` |
| `clean_completion_rebate` | (`clean_completion_rebate`, `shipment_id`) | `(shipment.supplier, rebate, fees_paid, bps)` | `maybe_pay_clean_completion_rebate` (internal; via `accept_mediation`, `batch_confirm_milestones`, `cast_dispute_vote`, `claim_auto_confirmation`, `confirm_milestone`, `finalize_dispute_resolution`, `release_held_payment`, `resolve_dispute`, `resolve_dispute_timeout`) |
| `clean_completion_rebate_bps_set` | (`clean_completion_rebate_bps_set`) | `bps` | `set_clean_completion_rebate_bps` |
| `co_buyer_set` | (`co_buyer_set`, `shipment_id`) | `co_buyer` | `set_co_buyer` |
| `collateral_partial_release` | (`collateral_partial_release`, `shipment_id`) | `(milestone_index, share, remaining - share)` | `release_milestone_collateral_share` (internal; via `batch_confirm_milestones`, `claim_auto_confirmation`, `confirm_milestone`, `release_held_payment`) |
| `collateral_slashed` | (`collateral_slashed`, `shipment_id`) | `(milestone_index, slash, remaining - slash, primary_buyer, caller)` | `slash_collateral_for_miss` |
| `collateral_token_set` | (`collateral_token_set`, `shipment_id`) | `collateral_token` | `set_collateral_token` |
| `collateral_topped_up` | (`collateral_topped_up`, `shipment_id`) | `(supplier, amount, new_balance)` | `top_up_collateral` |
| `compliance_hold_cleared` | (`compliance_hold_cleared`, `shipment_id`) | `()` | `clear_compliance_hold` |
| `compliance_hold_set` | (`compliance_hold_set`, `shipment_id`) | `reason_hash` | `set_compliance_hold` |
| `condition_breach_reported` | (`condition_breach_reported`, `shipment_id`) | `(milestone_index, data_hash, oracle, reports_len)` | `report_condition_breach` |
| `confirmation_cooldown_set` | (`confirmation_cooldown_set`) | `ledgers` | `set_confirmation_cooldown` |
| `contract_paused` | (`contract_paused`) | `ledger_seq` | `pause` |
| `contract_unpaused` | (`contract_unpaused`) | `ledger_seq` | `unpause` |
| `contract_upgraded` | (`contract_upgraded`) | `(new_wasm_hash, ledger_seq)` | `upgrade` |
| `conversion_rate_cleared` | (`conversion_rate_cleared`, `from_token`, `to_token`) | `()` | `clear_conversion_rate` |
| `conversion_rate_set` | (`conversion_rate_set`, `from_token`, `to_token`) | `rate_bps` | `set_conversion_rate` |
| `counterparty_rated` | (`counterparty_rated`, `shipment_id`) | `(caller, rated_address, stars, comment_hash)` | `rate_counterparty` |
| `deadline_approaching` | (`deadline_approaching`, `shipment_id`) | `(milestone_index, ledgers_remaining, milestone.deadline_ledger)` | `check_deadline_warning` |
| `delegate_authorized` | (`delegate_authorized`, `shipment_id`) | `(buyer, delegate, per_tx_cap)` | `authorize_delegate` |
| `delegate_revoked` | (`delegate_revoked`, `shipment_id`) | `(buyer, ledger_seq)` | `revoke_delegate` |
| `depeg_guard_removed` | (`depeg_guard_removed`, `token`) | `()` | `remove_depeg_guard` |
| `depeg_guard_set` | (`depeg_guard_set`, `token`) | `(min_price, max_price)` | `set_depeg_guard` |
| `dispute_appealed` | (`dispute_appealed`, `shipment_id`) | `(milestone_index, caller, new_arbiter)` | `appeal_dispute` |
| `dispute_consolidation_approved` | (`dispute_consolidation_approved`, `consolidation_id`) | `(caller, fully_approved)` | `approve_dispute_consolidation` |
| `dispute_consolidation_proposed` | (`dispute_consolidation_proposed`, `consolidation_id`) | `(arbiter, members_len)` | `propose_dispute_consolidation` |
| `dispute_consolidation_resolved` | (`dispute_consolidation_resolved`, `consolidation_id`) | `(approve, members_len, combined_contested, fee_bps)` | `resolve_consolidated_dispute` (each member also emits its usual `dispute_resolved`) |
| `dispute_auto_resolved` | (`dispute_auto_resolved`, `shipment_id`) | `(milestone_index, resolution_sym)` | `resolve_dispute_timeout` |
| `dispute_escalated` | (`dispute_escalated`, `shipment_id`) | `(milestone_index, opened_ledger, current_ledger)` | `check_escalation` |
| `dispute_evidence_submitted` | (`dispute_evidence_submitted`, `shipment_id`) | `(milestone_index, caller, evidence_hash, evidence_type)` | `submit_dispute_evidence` |
| `dispute_filing_window_set` | (`dispute_filing_window_set`) | `ledgers` | `set_dispute_filing_window` |
| `dispute_panel_assigned` | (`dispute_panel_assigned`, `shipment_id`) | `(milestone_index, panel)` | `assign_dispute_panel` |
| `dispute_raised` | (`dispute_raised`, `shipment_id`) | `milestone_index` | `raise_dispute` |
| `dispute_raised` | (`dispute_raised`, `shipment_id`) | `milestone_index` | `open_condition_breach_dispute` (internal; via `report_condition_breach`) |
| `dispute_resolution_finalized` | (`dispute_resolution_finalized`, `shipment_id`) | `(milestone_index, payment, fee_amount)` | `finalize_dispute_resolution` |
| `dispute_resolved` | (`dispute_resolved`, `shipment_id`) | `(milestone_index, approve, is_partial, released_amount, remaining_amount, reason)` | `resolve_dispute` |
| `dispute_resolved` | (`dispute_resolved`, `shipment_id`) | `(milestone_index, approve, is_partial, released_amount, remaining_amount)` | `resolve_dispute_panel_internal` (internal; via `cast_dispute_vote`) |
| `dispute_timeout_handoff` | (`dispute_timeout_handoff`, `shipment_id`) | `(milestone_index, old_arbiter, backup)` | `resolve_dispute_timeout` |
| `dispute_vote_cast` | (`dispute_vote_cast`, `shipment_id`) | `(milestone_index, arbiter, approve, votes_len)` | `cast_dispute_vote` |
| `dispute_withdrawn` | (`dispute_withdrawn`, `shipment_id`) | `(milestone_index, buyer)` | `withdraw_dispute` |
| `dual_proof_pending` | (`dual_proof_pending`, `shipment_id`) | `(milestone_index, proof_hash, proof_type, caller)` | `submit_proof` |
| `emergency_freeze_activated` | (`emergency_freeze_activated`) | `(admin, ledger_seq)` | `propose_emergency_freeze` |
| `emergency_freeze_activated` | (`emergency_freeze_activated`) | `proposal_id` | `execute_emergency_freeze_proposal` (internal; via `approve_emergency_freeze`, `propose_emergency_freeze`) |
| `emergency_freeze_approved` | (`emergency_freeze_approved`, `proposal_id`) | `(admin, approvals_count)` | `approve_emergency_freeze` |
| `emergency_freeze_lifted` | (`emergency_freeze_lifted`) | `(admin, ledger_seq)` | `propose_emergency_unfreeze` |
| `emergency_freeze_lifted` | (`emergency_freeze_lifted`) | `proposal_id` | `execute_emergency_unfreeze_proposal` (internal; via `approve_emergency_unfreeze`, `propose_emergency_unfreeze`) |
| `emergency_freeze_proposed` | (`emergency_freeze_proposed`, `proposal_id`) | `(admin, 1u32)` | `propose_emergency_freeze` |
| `emergency_recovery` | (`emergency_recovery`, `shipment_id`) | `(recovery_amount, admin)` | `do_emergency_recover` (internal; via `emergency_recover`, `execute_emergency_recover`, `propose_emergency_recover`) |
| `emergency_unfreeze_approved` | (`emergency_unfreeze_approved`, `proposal_id`) | `(admin, approvals_count)` | `approve_emergency_unfreeze` |
| `emergency_unfreeze_proposed` | (`emergency_unfreeze_proposed`, `proposal_id`) | `(admin, 1u32)` | `propose_emergency_unfreeze` |
| `escalation_threshold_set` | (`escalation_threshold_set`) | `threshold_ledgers` | `set_escalation_threshold` |
| `escrow_topped_up` | (`escrow_topped_up`, `shipment_id`) | `(additional_amount, new_total)` | `top_up_escrow` |
| `expiry_extended` | (`expiry_extended`, `shipment_id`) | `(old_expiry, proposal.new_expiry_ledger, counterparty)` | `approve_expiry_extension` |
| `expiry_extension_proposed` | (`expiry_extension_proposed`, `shipment_id`) | `(caller, current, new_expiry_ledger)` | `propose_expiry_extension` |
| `extension_approved` | (`extension_approved`, `shipment_id`) | `(milestone_index, new_deadline, buyer)` | `approve_extension` |
| `extension_denied` | (`extension_denied`, `shipment_id`) | `(milestone_index, buyer)` | `deny_extension` |
| `extension_requested` | (`extension_requested`, `shipment_id`) | `(milestone_index, extra_ledgers, caller)` | `request_extension` |
| `fee_holiday_scheduled` | (`fee_holiday_scheduled`) | `(start_ledger, end_ledger)` | `schedule_fee_holiday` |
| `fee_recipients_set` | (`fee_recipients_set`) | `(recipients_len, ledger_seq)` | `set_fee_recipients` |
| `fee_tiers_set` | (`fee_tiers_set`) | `tiers_len` | `set_fee_tiers` |
| `fee_waiver_approved` | (`fee_waiver_approved`, `proposal_id`) | `(admin, approvals_count)` | `approve_fee_waiver` |
| `fee_waiver_granted` | (`fee_waiver_granted`, `proposal_id`) | `(proposal.partner, proposal.waiver_bps, proposal.expires_at)` | `execute_fee_waiver_proposal` (internal; via `approve_fee_waiver`, `propose_fee_waiver`) |
| `fee_waiver_proposed` | (`fee_waiver_proposed`, `proposal_id`) | `(admin, partner, waiver_bps, 1u32)` | `propose_fee_waiver` |
| `finality_delay_ledgers_set` | (`finality_delay_ledgers_set`) | `ledgers` | `set_finality_delay_ledgers` |
| `freeze_supermajority_bps_set` | (`freeze_supermajority_bps_set`) | `bps` | `set_freeze_supermajority_bps` |
| `grade_disputed` | (`grade_disputed`, `shipment_id`) | `(milestone_index, grade.0, grade.1, buyer)` | `raise_dispute` |
| `grade_settled` | (`grade_settled`, `shipment_id`) | `(milestone_index, grade, grade_bps, payment, grade_refund)` | `release_held_payment` |
| `grade_settled` | (`grade_settled`, `shipment_id`) | `(milestone_index, grade, grade_bps, payment - contested, buyer_refund)` | `resolve_dispute` |
| `held_payment_released` | (`held_payment_released`, `shipment_id`) | `(milestone_index, payment, fee_amount)` | `release_held_payment` |
| `holdback_refunded` | (`holdback_refunded`, `shipment_id`) | `(primary_buyer, retainage, warranty)` | `refund_holdbacks_on_cancel` (internal; via `batch_cancel_shipments`, `cancel_for_blacklisted_supplier`, `cancel_shipment`, `claim_deadline_refund`, `emergency_recover`, `execute_emergency_recover`, `propose_emergency_recover`, `supplier_cancel`, `sweep_unclaimed_refund`) |
| `holdback_withheld` | (`holdback_withheld`, `shipment_id`) | `(milestone_index, retained, warranty)` | `withhold_settlement_holdbacks` (internal; via `accept_mediation`, `batch_confirm_milestones`, `cast_dispute_vote`, `claim_auto_confirmation`, `confirm_milestone`, `confirm_partial_quantity`, `finalize_dispute_resolution`, `release_held_payment`, `resolve_dispute`) |
| `incremental_collateral_set` | (`incremental_collateral_set`, `shipment_id`) | `(buyer, enabled)` | `set_incremental_collateral` |
| `inspector_signed_off` | (`inspector_signed_off`, `shipment_id`) | `(milestone_index, report_hash, inspector)` | `inspector_sign_off` |
| `invoice_hash_attached` | (`invoice_hash_attached`, `shipment_id`) | `(milestone_index, invoice_hash, caller)` | `attach_invoice_hash` |
| `joint_confirmation_recorded` | (`joint_confirmation_recorded`, `shipment_id`) | `(milestone_index, caller, status.buyer_confirmed, status.co_buyer_confirmed)` | `record_joint_confirmation` (internal; via `confirm_milestone`) |
| `joint_confirmation_threshold_set` | (`joint_confirmation_threshold_set`) | `threshold` | `set_joint_confirmation_threshold` |
| `logistics_transferred` | (`logistics_transferred`, `shipment_id`) | `(current_logistics, new_logistics)` | `transfer_logistics` |
| `long_hold_rebate_set` | (`long_hold_rebate_set`) | `(threshold_ledgers, rebate_bps)` | `set_long_hold_rebate` |
| `loser_pays_arbiter_fee_set` | (`loser_pays_arbiter_fee_set`, `shipment_id`) | `enabled` | `set_loser_pays_arbiter_fee` |
| `max_allowed_tokens_set` | (`max_allowed_tokens_set`) | `max_allowed` | `set_max_allowed_tokens` |
| `max_arbiter_fee_bps_set` | (`max_arbiter_fee_bps_set`) | `max_bps` | `set_max_arbiter_fee_bps` |
| `max_dispute_bond_bps_set` | (`max_dispute_bond_bps_set`) | `max_bps` | `set_max_dispute_bond_bps` |
| `max_evidence_updated` | (`max_evidence_updated`) | `(admin, max_count, ledger_seq)` | `set_max_evidence_per_milestone` |
| `max_expiry_ext_set` | (`max_expiry_ext_set`) | `ledgers` | `set_max_expiry_extension_ledgers` |
| `max_extension_requests_set` | (`max_extension_requests_set`) | `max_requests` | `set_max_extension_requests` |
| `max_milestone_count_set` | (`max_milestone_count_set`) | `count` | `set_max_milestone_count` |
| `max_overturned_before_slash_set` | (`max_overturned_before_slash_set`) | `threshold` | `set_max_overturned_before_slash` |
| `max_shipment_value_set` | (`max_shipment_value_set`) | `max_value` | `set_max_shipment_value` |
| `max_summary_batch_set` | (`max_summary_batch_set`) | `max` | `set_max_summary_batch` |
| `mediation_accepted` | (`mediation_accepted`, `shipment_id`) | `(milestone_index, resolution_sym, mediator)` | `apply_mediation_outcome` (internal; via `accept_mediation`) |
| `mediation_declined` | (`mediation_declined`, `shipment_id`) | `(milestone_index, caller)` | `decline_mediation` |
| `mediation_proposed` | (`mediation_proposed`, `shipment_id`) | `(milestone_index, mediator, outcome_sym)` | `propose_mediation` |
| `mediator_assigned` | (`mediator_assigned`, `shipment_id`) | `mediator` | `assign_mediator` |
| `mediator_pool_set` | (`mediator_pool_set`) | `mediators_len` | `set_mediator_pool` |
| `milestone_confirmed` | (`milestone_confirmed`, `shipment_id`) | `(milestone_index, payment, fee_amount, penalty_deducted, shipment.supplier, ledger_seq, shipment.released_amount, remaining_amount, applied_fee_bps)` | `confirm_milestone` |
| `milestone_confirmed` | (`milestone_confirmed`, `shipment_id`) | `(idx, payment, fee_amount, shipment.supplier, ledger_seq, shipment.released_amount, remaining_amount)` | `batch_confirm_milestones` |
| `milestone_expired` | (`milestone_expired`, `shipment_id`) | `(milestone_index, refund_amount, primary_buyer)` | `claim_deadline_refund` |
| `milestone_graded` | (`milestone_graded`, `shipment_id`) | `(milestone_index, grade_index, grade_bps, payout, refund, release_after_ledger)` | `record_milestone_grade` (internal; via `confirm_milestone_graded`) |
| `milestone_merge_proposed` | (`milestone_merge_proposed`, `shipment_id`) | `(first_index, caller)` | `propose_milestone_merge` |
| `milestone_note_added` | (`milestone_note_added`, `shipment_id`, `milestone_index`) | `(caller, note)` | `add_milestone_note` |
| `milestone_payees_set` | (`milestone_payees_set`, `shipment_id`) | `(milestone_index, payees_len, buyer)` | `set_milestone_payees` |
| `milestone_split_proposed` | (`milestone_split_proposed`, `shipment_id`) | `(milestone_index, caller, parts_len)` | `propose_milestone_split` |
| `milestone_template_saved` | (`milestone_template_saved`, `creator`) | `name` | `save_milestone_template` |
| `milestones_merged` | (`milestones_merged`, `shipment_id`) | `(first_index, merged.payment_percent, merged.deadline_ledger, counterparty)` | `approve_milestone_merge` |
| `milestones_rebalanced` | (`milestones_rebalanced`, `shipment_id`) | `(buyer, new_percents)` | `rebalance_milestones` |
| `milestones_split` | (`milestones_split`, `shipment_id`) | `(milestone_index, part_count, counterparty)` | `approve_milestone_split` |
| `min_shipment_value_set` | (`min_shipment_value_set`) | `min_amount` | `set_min_shipment_value` |
| `multisig_admin_initialized` | (`multisig_admin_initialized`) | `threshold` | `initialize_multisig_admin` |
| `mutual_preapproval_updated` | (`mutual_preapproval_updated`) | `(admin, enabled, ledger_seq)` | `set_require_mutual_preapproval` |
| `nft_hook_config_updated` | (`nft_hook_config_updated`) | `(admin, enabled, ledger_seq)` | `set_nft_hook_enabled` |
| `nft_mint_hook` | (`nft_mint_hook`, `shipment_id`) | `(primary_buyer, shipment.supplier, shipment.total_amount, ledger_seq, shipment.metadata_hash)` | `confirm_milestone` |
| `nomination_revoked` | (`nomination_revoked`) | `ledger_seq` | `revoke_nomination` |
| `observer_added` | (`observer_added`, `shipment_id`) | `(buyer, observer_address)` | `add_shipment_observer` |
| `observer_removed` | (`observer_removed`, `shipment_id`) | `(buyer, observer_address)` | `remove_shipment_observer` |
| `operation_paused_set` | (`operation_paused_set`, `op`) | `paused` | `set_operation_paused` |
| `oracle_attestation_submitted` | (`oracle_attestation_submitted`, `shipment_id`) | `(milestone_index, oracle)` | `submit_oracle_attestation` |
| `oracle_group_registered` | (`oracle_group_registered`, `purpose`) | `(oracles_len, threshold)` | `register_oracle_group` |
| `panel_fee_distributed` | (`panel_fee_distributed`, `shipment_id`) | `(milestone_index, voters, distributed)` | `distribute_panel_fee` (internal; via `cast_dispute_vote`) |
| `param_change_cancelled` | (`param_change_cancelled`) | `param` | `cancel_param_change` |
| `param_change_executed` | (`param_change_executed`, `param`) | `new_value` | `execute_param_change` |
| `param_change_proposed` | (`param_change_proposed`, `param`) | `(new_value, effective_ledger)` | `propose_param_change` |
| `partial_dispute_raised` | (`partial_dispute_raised`, `shipment_id`) | `(milestone_index, contested_percent, buyer)` | `raise_partial_dispute` |
| `partial_quantity_confirmed` | (`partial_quantity_confirmed`, `shipment_id`) | `(milestone_index, delivered, cumulative, expected, released, fee_amount)` | `emit_partial_quantity_confirmed` (internal; via `confirm_partial_quantity`) |
| `partial_uncontested_released` | (`partial_uncontested_released`, `shipment_id`) | `(milestone_index, uncontested_payment, fee_amount)` | `raise_partial_dispute` |
| `pause_min_notice_set` | (`pause_min_notice_set`) | `ledgers` | `set_pause_min_notice_ledgers` |
| `payment_held` | (`payment_held`, `shipment_id`) | `(milestone_index, milestone.release_after_ledger, penalty_deducted, early_bonus_paid)` | `confirm_milestone` |
| `payout_assigned` | (`payout_assigned`, `shipment_id`) | `(supplier, financier)` | `assign_payout` |
| `payout_assignment_released` | (`payout_assignment_released`, `shipment_id`) | `(assignment.supplier, financier)` | `release_payout_assignment` |
| `payout_assignment_transferred` | (`payout_assignment_transferred`, `shipment_id`) | `(financier, new_financier)` | `transfer_payout_assignment` |
| `payout_claimed` | (`payout_claimed`, `supplier`) | `(payout_amount, payout_token, rate_bps)` | `claim_payout` |
| `payout_currency_preference_set` | (`payout_currency_preference_set`, `supplier`) | `preferred_token` | `set_payout_currency_preference` |
| `payout_mode_set` | (`payout_mode_set`, `supplier`) | `batched` | `set_payout_mode` |
| `payout_reference` | (`payout_reference`, `shipment_id`) | `(milestone_index, net_amount, reference)` | `pay_milestone_to_payees` (internal; every milestone payment release, including `claim_auto_confirmation`), only when a payout reference is set |
| `payout_reference_set` | (`payout_reference_set`, `shipment_id`) | `reference` (empty = cleared) | `set_payout_reference` |
| `proof_attested` | (`proof_attested`, `shipment_id`) | `(milestone_index, proof_hash, caller, current_ledger)` | `attest_proof` |
| `proof_corrected` | (`proof_corrected`, `shipment_id`) | `(milestone_index, proof_hash_for_event)` | `correct_proof` |
| `proof_hash_length_bounds_set` | (`proof_hash_length_bounds_set`) | `(min_len, max_len)` | `set_proof_hash_length_bounds` |
| `proof_hash_required_prefix_set` | (`proof_hash_required_prefix_set`) | `prefix` | `set_proof_hash_required_prefix` |
| `proof_resubmitted` | (`proof_resubmitted`, `shipment_id`) | `(milestone_index, proof_hash_for_event, proof_type, caller, current_ledger)` | `submit_proof` |
| `proof_submitted` | (`proof_submitted`, `shipment_id`) | `(milestone_index, proof_hash_for_event, proof_type, caller, current_ledger)` | `submit_proof` |
| `proof_submitted` | (`proof_submitted`, `shipment_id`) | `(milestone_index, proof_hash, proof_type, pending.first_attester, current_ledger)` | `attest_proof` |
| `proof_whitelist_set` | (`proof_whitelist_set`, `shipment_id`) | `(milestone_index, buyer)` | `set_proof_whitelist` |
| `rate_limit_exemption_added` | (`rate_limit_exemption_added`, `supplier`) | `()` | `add_rate_limit_exemption` |
| `rate_limit_exemption_removed` | (`rate_limit_exemption_removed`, `supplier`) | `()` | `remove_rate_limit_exemption` |
| `rating_window_ledgers_set` | (`rating_window_ledgers_set`) | `window_ledgers` | `set_rating_window_ledgers` |
| `recovery_cancelled` | (`recovery_cancelled`, `shipment_id`) | `(admin, ledger_seq)` | `cancel_emergency_recover` |
| `recovery_delay_updated` | (`recovery_delay_updated`) | `(admin, ledgers, ledger_seq)` | `set_recovery_delay` |
| `recovery_proposed` | (`recovery_proposed`, `shipment_id`) | `(admin, effective_ledger, current_ledger)` | `propose_emergency_recover` |
| `referral_fee_bps_set` | (`referral_fee_bps_set`) | `bps` | `set_referral_fee_bps` |
| `refund_recipient_set` | (`refund_recipient_set`, `shipment_id`) | `(buyer, recipient)` | `set_refund_recipient` |
| `refund_sweep_window_set` | (`refund_sweep_window_set`) | `ledgers` | `set_refund_sweep_window` |
| `reputation_fast_track_set` | (`reputation_fast_track_set`) | `(min_completed, max_disputed_ratio_bps)` | `set_reputation_fast_track` |
| `retainage_released` | (`retainage_released`, `shipment_id`) | `(shipment.supplier, retainage)` | `settle_on_completion` (internal; via `accept_mediation`, `batch_confirm_milestones`, `cast_dispute_vote`, `claim_auto_confirmation`, `confirm_milestone`, `finalize_dispute_resolution`, `release_held_payment`, `resolve_dispute`, `resolve_dispute_timeout`) |
| `role_granted` | (`role_granted`, `admin`) | `(role, address)` | `grant_role` |
| `role_revoked` | (`role_revoked`, `admin`) | `(role, address)` | `revoke_role` |
| `shipment_archived` | (`shipment_archived`, `shipment_id`) | `(archived.status, archived.completed_at)` | `archive_shipment` |
| `shipment_cancelled` | (`shipment_cancelled`, `shipment_id`) | `(unreleased, 0i128, buyer, ledger_seq)` | `cancel_for_blacklisted_supplier` |
| `shipment_cancelled` | (`shipment_cancelled`, `shipment_id`) | `(refund, cancel_fee, buyer, ledger_seq)` | `cancel_shipment_internal` (internal; via `batch_cancel_shipments`, `cancel_shipment`) |
| `shipment_created` | (`shipment_created`, `shipment_id`) | `(primary_buyer, shipment.supplier, shipment.logistics, shipment.arbiter, shipment.token, shipment.total_amount, ledger_seq, shipment.metadata_hash)` | `create_shipment` |
| `shipment_fee_override_cleared` | (`shipment_fee_override_cleared`) | `shipment_id` | `clear_shipment_fee_override` |
| `shipment_fee_override_set` | (`shipment_fee_override_set`, `shipment_id`) | `fee_bps` | `set_shipment_fee_override` |
| `shipment_metadata_set` | (`shipment_metadata_set`, `shipment_id`) | `(caller, key, value)` | `set_shipment_metadata` |
| `shipment_oracle_purpose_set` | (`shipment_oracle_purpose_set`, `shipment_id`) | `purpose` | `set_shipment_oracle_purpose` |
| `shipment_pause_requested` | (`shipment_pause_requested`, `shipment_id`) | `caller` | `request_shipment_pause` |
| `shipment_paused` | (`shipment_paused`, `shipment_id`) | `(caller, now)` | `approve_shipment_pause` |
| `shipment_rate_limit_set` | (`shipment_rate_limit_set`) | `(max_shipments, window_ledgers)` | `set_shipment_creation_rate_limit` |
| `shipment_resume_requested` | (`shipment_resume_requested`, `shipment_id`) | `caller` | `resume_shipment` |
| `shipment_resumed` | (`shipment_resumed`, `shipment_id`) | `(caller, ledger_seq)` | `resume_shipment` |
| `shipment_ttl_extended` | (`shipment_ttl_extended`, `shipment_id`) | `TTL_MAX_LEDGERS` | `extend_shipment_ttl` |
| `standing_order_cancelled` | (`standing_order_cancelled`, `order_id`) | `(buyer, ledger_seq)` | `cancel_standing_order` |
| `standing_order_created` | (`standing_order_created`, `order_id`) | `(buyer, params.template_name, params.amount, params.interval_ledgers, params.max_occurrences)` | `create_standing_order` |
| `standing_order_executed` | (`standing_order_executed`, `order_id`) | `(created, order.occurrences, order.next_ledger)` | `execute_standing_order` |
| `substitute_approved` | (`substitute_approved`, `shipment_id`) | `(milestone_index, new_supplier, arbiter)` | `approve_substitute_supplier` |
| `substitute_proposed` | (`substitute_proposed`, `shipment_id`) | `(milestone_index, new_supplier, buyer)` | `propose_substitute_supplier` |
| `supplier_cancel_cooldown_set` | (`supplier_cancel_cooldown_set`, `supplier`) | `(max_cancellations, window_ledgers, cooldown_ledgers)` | `set_supplier_cancel_cooldown` |
| `supplier_cancellation` | (`supplier_cancellation`, `shipment_id`) | `(penalty, refund)` | `supplier_cancel` |
| `supplier_exposure_cap_set` | (`supplier_exposure_cap_set`) | `cap` | `set_supplier_exposure_cap` |
| `supplier_substituted` | (`supplier_substituted`, `shipment_id`) | `(milestone_index, new_supplier)` | `approve_substitute_supplier` |
| `supplier_tier_changed` | (`supplier_tier_changed`, `supplier`) | `(supplier, old_tier, new_tier)` | `evaluate_supplier_tier_change` (internal; via `accept_mediation`, `batch_cancel_shipments`, `batch_confirm_milestones`, `cancel_shipment`, `cast_dispute_vote`, `claim_deadline_refund`, `confirm_milestone`, `create_shipment`, `finalize_dispute_resolution`, `raise_dispute`, `raise_partial_dispute`, `release_held_payment`, `report_condition_breach`, `resolve_dispute`, `resolve_dispute_timeout`, `supplier_cancel`, `sweep_unclaimed_refund`) |
| `supplier_tier_config_set` | (`supplier_tier_config_set`) | `()` | `set_supplier_tier_config` |
| `supplier_transferred` | (`supplier_transferred`, `shipment_id`) | `(current_supplier, new_supplier)` | `transfer_supplier` |
| `supplier_unwhitelisted` | (`supplier_unwhitelisted`) | `address` | `remove_from_whitelist` |
| `supplier_whitelisted` | (`supplier_whitelisted`) | `address` | `add_to_whitelist` |
| `timelock_duration_set` | (`timelock_duration_set`) | `ledgers` | `set_timelock_duration` |
| `token_max_shipment_value_cleared` | (`token_max_shipment_value_cleared`) | `token` | `clear_token_max_shipment_value` |
| `token_max_shipment_value_set` | (`token_max_shipment_value_set`, `token`) | `max_value` | `set_token_max_shipment_value` |
| `token_min_shipment_value_cleared` | (`token_min_shipment_value_cleared`) | `token` | `clear_token_min_shipment_value` |
| `token_min_shipment_value_set` | (`token_min_shipment_value_set`, `token`) | `min_amount` | `set_token_min_shipment_value` |
| `treasury_withdrawal` | (`treasury_withdrawal`, `token`) | `(to, dust_amount, ledger_seq)` | `withdraw_treasury_dust` |
| `tvl_cap_set` | (`tvl_cap_set`, `token`) | `cap` | `set_tvl_cap` |
| `unclaimed_refund_swept` | (`unclaimed_refund_swept`, `shipment_id`) | `(milestone_index, refund_amount, fee_config.treasury)` | `sweep_unclaimed_refund` |
| `upgrade_approved` | (`upgrade_approved`, `proposal_id`) | `(admin, approvals_count)` | `approve_upgrade` |
| `upgrade_cancelled` | (`upgrade_cancelled`, `proposal_id`) | `admin` | `cancel_upgrade` |
| `upgrade_executed` | (`upgrade_executed`, `proposal_id`) | `proposal.new_wasm_hash` | `execute_upgrade_proposal` (internal; via `approve_upgrade`, `propose_upgrade`) |
| `upgrade_proposed` | (`upgrade_proposed`, `proposal_id`) | `(new_wasm_hash, admin, 1u32)` | `propose_upgrade` |
| `vault_deposited` | (`vault_deposited`, `buyer`, `token`) | `(amount, ledger_seq)` | `vault_deposit` |
| `vault_withdrawn` | (`vault_withdrawn`, `buyer`, `token`) | `(amount, ledger_seq)` | `vault_withdraw` |
| `warning_lead_ledgers_set` | (`warning_lead_ledgers_set`) | `ledgers` | `set_warning_lead_ledgers` |
| `warranty_claim_filed` | (`warranty_claim_filed`, `shipment_id`) | `(buyer, shipment.arbiter, evidence_hash)` | `file_warranty_claim` |
| `warranty_claim_resolved` | (`warranty_claim_resolved`, `shipment_id`) | `(arbiter, refund_buyer, refunded)` | `resolve_warranty_claim` |
| `warranty_released` | (`warranty_released`, `shipment_id`) | `(shipment.supplier, amount)` | `release_warranty` |
| `warranty_started` | (`warranty_started`, `shipment_id`) | `(warranty, ends_at)` | `settle_on_completion` (internal; via `accept_mediation`, `batch_confirm_milestones`, `cast_dispute_vote`, `claim_auto_confirmation`, `confirm_milestone`, `finalize_dispute_resolution`, `release_held_payment`, `resolve_dispute`, `resolve_dispute_timeout`) |
| `webhook_added` | (`webhook_added`) | `(admin, contract_address, ledger_seq)` | `add_confirmation_webhook` |
| `webhook_removed` | (`webhook_removed`) | `(admin, contract_address, ledger_seq)` | `remove_confirmation_webhook` |
