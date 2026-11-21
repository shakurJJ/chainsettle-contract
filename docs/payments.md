# Payment release variants

This guide covers the ways a confirmed milestone can move escrow to a supplier,
other payees, or back to the buyer. It describes the contract's current behavior;
see [fee model](fees.md) for fee calculations and rounding.

## Standard confirmation

The buyer calls `confirm_milestone` after proof is submitted. The contract
releases the milestone amount, less any late penalty, approved advance, platform
fee, logistics fee, and applicable completion deductions. Any retainage or
warranty amount is withheld from the resulting supplier transfer. A shipment
with `holdback_ledgers > 0` instead marks the milestone `ConfirmedHeld`; the
supplier claims it later with `release_held_payment`.

**Example:** For a 1,000,000-unit shipment with a 25% first milestone and no
deductions, that milestone's gross payout is 250,000. The preview test checks
this amount in [test_payout_preview.rs](../contracts/chainsetttle/src/test_payout_preview.rs).

## Supplier advance

The supplier requests an advance as a percentage of one pending milestone with
`request_advance`; the buyer approves it with `approve_advance`. Approval sends
the advance before proof submission (or accrues it for a batched-mode supplier).
When the milestone is later paid, the advance is subtracted from the supplier's
remaining transfer; it does not reduce the gross fee base. Advances are subject
to the configured maximum advance percentage.

**Example:** A 10% advance on a 250,000-unit milestone sends 25,000 up front.
The eventual settlement deducts that 25,000 from the amount otherwise payable.
The tests cover the deduction in
[test_payout_preview.rs](../contracts/chainsetttle/src/test_payout_preview.rs)
and the 10% request in [test_query.rs](../contracts/chainsetttle/src/test_query.rs).

## Time-based holdback

Set `ShipmentOptions.holdback_ledgers` to delay each confirmed milestone.
`confirm_milestone` records `ConfirmedHeld` and a release ledger instead of
transferring the milestone payment. After that ledger, anyone may call
`release_held_payment`. An approved advance remains outstanding until this
release, when it is deducted. The holdback period is separate from retainage
and warranty holdbacks.

**Example:** A 250,000-unit milestone with no other deductions remains held at
confirmation and is paid after its release ledger. The preview and release
amount are checked in
[test_payout_preview.rs](../contracts/chainsetttle/src/test_payout_preview.rs).

## Retainage

`retainage_bps` withholds that share of each net supplier transfer. The balance
remains escrowed until the shipment completes, when all retainage is released to
the supplier. If the shipment is cancelled or expires first, the withheld
retainage is refunded to the primary buyer.

**Example:** With 10% retainage and no fees, a 250,000-unit milestone pays
225,000 now and adds 25,000 to retainage. The exact withholding and completion
release are checked in
[test_retainage.rs](../contracts/chainsetttle/src/test_retainage.rs).

## Warranty holdback

`warranty_bps` withholds a share of each net supplier transfer and requires a
non-zero `warranty_ledgers` period. The warranty balance stays in escrow after
the shipment completes. Until the period ends, the buyer may file a warranty
claim; only one claim can be open at a time. The arbiter either refunds the whole
balance to the buyer or dismisses the claim. Once the period ends with no open
claim, anyone may call `release_warranty` to pay the remaining balance to the
supplier. If the shipment is cancelled or expires, the warranty holdback is
refunded to the buyer.

**Example:** With a 5% warranty holdback and no fees on a 1,000,000-unit
shipment, 50,000 is withheld across the milestones. The holdback, claim, and
release amounts are checked in
[test_warranty.rs](../contracts/chainsetttle/src/test_warranty.rs).

## Quality-graded confirmation

Configure `quality_grades` as non-increasing bps values, starting at 10,000.
The buyer passes a grade index to `confirm_milestone_graded`. Grade 0 (10,000
bps) follows the normal confirmation path. A lower grade holds the milestone
through its review window; after the window, `release_held_payment` sends the
graded share to the supplier and refunds the unaccepted remainder to the buyer.
The review duration is the greater of the configured holdback and review
windows (or a default if neither is set). A lower-grade confirmation cannot
have an approved advance; the supplier may dispute the grade during the review
window.

**Example:** With grades `[10,000, 9,000, 7,500]`, grade index 1 on a
250,000-unit milestone allocates 225,000 to the supplier and refunds 25,000 to
the buyer, before any applicable fees or holdbacks. This split is asserted in
[test_quality_grades.rs](../contracts/chainsetttle/src/test_quality_grades.rs).

## Partial-quantity confirmation

Configure `milestone_quantities` with the expected quantity for a milestone.
After proof submission, the buyer can call `confirm_partial_quantity` multiple
times. Each call releases the difference between the cumulative pro-rata target
and the amount already released. The final quantity releases the remaining
amount (including rounding dust) and confirms the milestone. Fees are applied
per release.

Partial-quantity confirmation is unavailable when the shipment has a
time-based holdback, the milestone has an approved advance, or joint
confirmation is required. Retainage and warranty withholding still apply to
each supplier transfer.

**Example:** A 250,000-unit milestone for 3 units pays 83,333, 83,333, then
83,334; the final unit absorbs the rounding remainder. This is asserted in
[test_partial_quantity.rs](../contracts/chainsetttle/src/test_partial_quantity.rs).

## Milestone payee splits

Before a milestone leaves `Pending`, the buyer can set its `MilestonePayee`
entries with `set_milestone_payees`. Percentages must total exactly 100. The
split applies to the amount left for payment after deductions and configured
retainage/warranty withholding. With no payees configured, payment goes to the
supplier. The last payee receives any rounding remainder.

An assigned receivable pays the financier instead and takes precedence over
payee splits and supplier payout batching.

**Example:** Split a 250,000-unit payment 30% / 70% between two payees: they
receive 75,000 and 175,000. The amounts are asserted in
[test_payout_preview.rs](../contracts/chainsetttle/src/test_payout_preview.rs).

## Batching

There are two distinct batching features:

- **Batch confirmation:** `batch_confirm_milestones` confirms several
  `ProofSubmitted` milestones in one atomic call. If any index or status is
  invalid, the call reverts; an empty index list is a no-op. The oracle
  confirmation flow is exercised in
  [test_oracle.rs](../contracts/chainsetttle/src/test_oracle.rs).
- **Supplier payout batching:** A supplier opts in with
  `set_payout_mode(supplier, true)`. Eligible supplier amounts accumulate in
  `get_pending_payout` instead of being transferred per milestone. The supplier
  withdraws the accumulated balance with `claim_payout`. This behavior and
  claiming are tested in
  [test_payout_currency_preference.rs](../contracts/chainsetttle/src/test_payout_currency_preference.rs).

With a 1,000,000-unit shipment and no deductions, a 25% milestone in supplier
batch mode adds 250,000 to the pending balance; a later 50% milestone adds
another 500,000, for 750,000 pending before the claim. Payee splits bypass
supplier batching and pay each configured payee directly. Approved advances
also follow supplier payout mode unless an assigned financier receives the
advance. If the supplier has a payout-currency preference, `claim_payout` uses
it only when a conversion route is configured and the contract has enough of
the preferred token; otherwise it pays in the source token.

**Batch-confirm caveat:** `batch_confirm_milestones` is not identical to calling
`confirm_milestone` repeatedly. Its current path uses the basic platform fee
calculation and does not apply the individual path's late-delivery penalty or
logistics fee; it also does not defer a milestone for `holdback_ledgers`.
Use individual confirmations when those settings must be honored.

## Combining mechanisms

The normal immediate-confirmation path applies fee/penalty and advance
deductions before logistics, completion deductions, and `pay_milestone_to_payees`.
That final payout step withholds retainage and warranty amounts from the
remaining supplier transfer, then either sends the remainder to the supplier,
queues it for supplier batching, or splits it among configured payees.

**Advance + retainage example:** For a 250,000-unit milestone, a 10% platform
fee is 25,000. A 10% advance is paid separately as 25,000. The remaining
transfer is `250,000 - 25,000 fee - 25,000 advance = 200,000`; 10% retainage
withholds 20,000, leaving 180,000 for the supplier at confirmation. The
supplier has received 205,000 in total including the earlier advance, and
20,000 remains in retainage. This combined calculation is asserted in
[test_retainage.rs](../contracts/chainsetttle/src/test_retainage.rs).

Key compatibility rules:

| Combination | Behavior |
| --- | --- |
| Advance + normal confirmation | Supported; the advance is netted from that milestone's eventual transfer. |
| Advance + `holdback_ledgers` | Supported; the advance remains outstanding until `release_held_payment`. |
| Advance + lower quality grade | Not supported for that milestone; lower-grade confirmation rejects an approved advance. |
| Advance + partial quantity | Not supported for that milestone; partial-quantity confirmation rejects an approved advance. |
| Time-based holdback + partial quantity | Not supported. |
| Retainage + warranty | Both are calculated independently from the same net supplier transfer and both remain escrowed under their own release rules. |
| Retainage/warranty + payee split | Supported; both holdbacks are withheld before the remainder is split. |
| Payee split + supplier batching | The configured split is paid directly to its payees; it does not accrue to the supplier's pending balance. |
| Assigned receivable + split/batching | The financier receives the payout instead of the configured payees or the supplier's pending balance. |

`batch_confirm_milestones` has the caveats above; do not assume it applies every
combination in the table the same way as individual confirmation.
