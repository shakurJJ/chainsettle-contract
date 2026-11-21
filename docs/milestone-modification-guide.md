# Modifying Milestones After Creation

Milestones are not immutable: the buyer and supplier can change scope, weights,
timing and even split a shipment after creation. Each flow has different
consent and state requirements, so pick the narrowest one that achieves the
goal. Source of truth: [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs).

## Decision table

| Goal | Function(s) | Who must approve | Allowed state |
| --- | --- | --- | --- |
| Rename one milestone or change its payment percent | `propose_amendment(caller, shipment_id, milestone_index, new_percent, new_name)` | Buyer **and** supplier — both call the same function with identical terms | `ShipmentStatus::Active`; target milestone `Pending`; all percentages must still sum to 100 |
| Redistribute all milestone weights | `rebalance_milestones(buyer, shipment_id, new_percents)` | Buyer only | `Active`; **all** milestones `Pending`; percentages must sum to 100 |
| Split one milestone into several `MilestoneSplitPart`s | `propose_milestone_split(caller, shipment_id, milestone_index, parts)` then `approve_milestone_split(counterparty, shipment_id, milestone_index)` | Either party proposes, the counterparty approves | `Active`; target milestone `Pending` |
| Merge a milestone into its neighbour | `propose_milestone_merge(caller, shipment_id, first_index)` then `approve_milestone_merge(counterparty, shipment_id, first_index)` | Either party proposes, the counterparty approves | `Active`; affected milestones `Pending` |
| Add a new final milestone (buyer funds a top-up) | `propose_new_milestone(caller, shipment_id, name, amount)` then `accept_new_milestone(caller, shipment_id)`; `cancel_new_milestone(caller, shipment_id)` withdraws | Either party proposes, the counterparty accepts | `Active`; new milestone is appended as `Pending` |
| Delay when a milestone's proof may be submitted | `propose_milestone_not_before(caller, shipment_id, milestone_index, not_before_ledger)` then `agree_milestone_not_before(caller, shipment_id, milestone_index)`; `cancel_milestone_not_before(...)` withdraws | Buyer **and** supplier (either may propose, the other agrees) | `Active`; target milestone `Pending` |
| Extend one milestone's deadline | `request_extension(caller, shipment_id, milestone_index, extra_ledgers)` then `approve_extension(buyer, shipment_id, milestone_index)` / `deny_extension(buyer, shipment_id, milestone_index)` | Supplier proposes, buyer approves or denies | `Active`; one pending request per milestone, subject to the configured maximum |
| Extend the shipment's overall expiry | `propose_expiry_extension(caller, shipment_id, new_expiry_ledger)` then `approve_expiry_extension(counterparty, shipment_id)` | Either party proposes, the counterparty approves; a proposal from the supplier must be approved by a buyer and vice versa | `Active`; proposal must not be stale; bounded by `get_max_expiry_extension_ledgers` |
| Move a subset of milestones into a second shipment | `propose_split_shipment(caller, shipment_id, new_shipment_id, milestone_indices)` then `agree_split_shipment(caller, shipment_id)`; `cancel_split_shipment(proposer, shipment_id)` withdraws | Buyer **and** supplier (proposal carries each party's agreement flag) | `Active`; every selected milestone `Pending`; escrow is conserved across both shipments |

For the append flow in detail — funding, payout-weight re-derivation and
refunds on cancellation — see [append-milestone.md](append-milestone.md).

## Rules that apply across flows

- **Pending is the key state.** Amendment, rebalance, split, merge, not-before
  and shipment-split all reject milestones that are `ProofSubmitted`,
  `Confirmed`, `Disputed`, `Resolved` or otherwise no longer `Pending`.
  Wait for resolution or use a different flow.
- **Mutual consent is explicit.** Functions ending in `approve_*` /
  `agree_*` / `accept_*` must be called by the counterparty, not the proposer.
  `propose_amendment` is the exception: both parties call the same function.
- **One proposal at a time.** Each flow keeps a single pending proposal per
  shipment (or per milestone for extensions). A new proposal with different
  terms resets or is rejected rather than stacking. Use the `cancel_*`
  function to withdraw cleanly.
- **Weights and escrow stay consistent.** Amendment and rebalance require
  percentages summing to 100; append re-derives basis-point splits so existing
  milestone values do not inflate; shipment split divides `total_amount` by
  the moved milestones' weights, so `old_total == remaining_total + new_total`.

## Edge cases

- **Open dispute.** `ShipmentStatus` stays `Active` while a milestone is
  disputed, so the status check alone is not enough. The per-milestone
  `Pending` checks block changes to the disputed milestone and to any
  all-milestones operation such as `rebalance_milestones`. Flows that only
  require an `Active` shipment — appending a milestone, extending expiry —
  are *not* blocked by an open dispute; check `open_dispute_count` on the
  shipment (or the active dispute list) before using them. Do not change
  commercial terms while an arbiter is evaluating evidence.
- **Proof already submitted.** A milestone with `proof_submitted_ledger` set
  is no longer safely modifiable; auto-confirmation may release it. Use the
  expiry/extension flows only for deadlines that have not passed.
- **Rejected or stale proposals.** `cancel_*` is the supported way to clear a
  pending proposal; expiry-extension approvals additionally fail once the
  proposal is stale. Always read the pending proposal back before acting on
  it off-chain.
- **Paused or frozen contract.** Most flows call `assert_not_paused` first and
  fail while the contract is paused or under emergency freeze.
