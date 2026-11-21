# Supplier collateral

This guide describes the supplier collateral flows in the current contract.
Collateral is denominated in integer token units and is separate from buyer
escrow, dispute bonds, and milestone fees.

## Requirement and deposit

The collateral base is selected when a shipment is created:

1. With no supplier terms profile, the base is `ShipmentOptions.supplier_collateral`.
2. With a profile where `enforce_terms` is false, a positive
   `supplier_collateral` overrides the profile. If it is zero, the base is
   `total_amount * default_collateral_bps / 10,000`.
3. With `enforce_terms` true, the profile overrides the supplied option:
   `base = total_amount * default_collateral_bps / 10,000`.

The `default_collateral_bps` profile value is capped at 10,000 bps. A zero base
means no collateral is deposited. For a positive requirement, the supplier
authorizes the transfer at shipment creation; the contract collects the
effective amount and records it as the shipment's locked collateral. Creation
with insufficient supplier balance fails, as covered by
[test_supplier_collateral.rs](../contracts/chainsetttle/src/test_supplier_collateral.rs).

**Example:** For a 2,000,000-unit shipment and a profile requirement of 500
bps, the base is `2,000,000 * 500 / 10,000 = 100,000` units. A Bronze
supplier locks all 100,000 before any tier discount.

`set_supplier_terms_profile` and `get_supplier_terms_profile` manage and read
the supplier's defaults. Profiles apply to new shipments, not shipments already
created.

## Tier discounts

An admin sets thresholds and multipliers with `set_supplier_tier_config`;
`get_supplier_tier_config` reads that configuration. `get_supplier_tier` reports
the supplier's current tier. With no config or no completed shipments, a
supplier is Bronze. Otherwise the tier is derived from completed shipments and
the disputed-to-completed ratio. Gold is checked before Silver. The configured
Silver and Gold multipliers must not exceed 10,000 bps, so a tier cannot
increase collateral.

At shipment creation, the contract applies the current tier multiplier to the
base:

```text
effective_collateral = base * tier_multiplier_bps / 10,000
```

Bronze uses 10,000 bps. A configured Silver multiplier of 8,000 bps reduces a
1,000,000-unit base to 800,000; a Gold multiplier of 5,000 bps reduces it to
500,000. The resulting amount is what the supplier must have available and
what is recorded as locked. Tier changes after creation do not reprice existing
collateral. These amounts are asserted in
[test_supplier_tiering.rs](../contracts/chainsetttle/src/test_supplier_tiering.rs).

## Adding collateral

While a shipment is active, its supplier can call `top_up_collateral` with a
positive amount. The transfer is added to the locked balance; it does not
re-run the tier discount. Calls by another account, non-positive amounts, or
top-ups after the shipment is no longer active are rejected.

`get_locked_collateral` reads the stored balance. It is most useful while the
shipment is active: terminal paths currently transfer collateral without
consistently clearing this stored value, so after completion or forfeiture the
query can report an amount that is no longer held. The top-up validation cases
are in
[test_feat_issues_551_553_554_556.rs](../contracts/chainsetttle/src/test_feat_issues_551_553_554_556.rs).

## Release on delivery

By default, collateral remains locked until shipment completion. On the regular
`confirm_milestone` completion path, the remaining collateral is returned to
the supplier.

The buyer may opt into incremental release with
`set_incremental_collateral(shipment_id, true)`. This is allowed only for an
active shipment with positive locked collateral and before a payout or a
milestone state that prevents changing the setting. The buyer can disable it
again before those conditions change. `get_incremental_collateral` reports
whether it is enabled.

When enabled, each milestone that reaches `Confirmed` through a non-dispute
confirmation path releases this share:

```text
share = min(original_locked_balance * milestone_payment_percent / 100,
            collateral_remaining)
```

The original locked balance is snapshotted when incremental release is enabled.
For a 1,000,000-unit balance and milestones weighted 25%, 50%, and 25%, the
first two confirmations release 250,000 and 500,000 respectively; the final
confirmation releases the remaining 250,000. A disputed milestone that resolves
does not release an incremental share. The payout tests check the proportional
amounts and dispute behavior in
[test_buyer_cap_collateral_merge.rs](../contracts/chainsetttle/src/test_buyer_cap_collateral_merge.rs).

**Completion-path limitation:** `release_held_payment`, `claim_auto_confirmation`,
and `batch_confirm_milestones` can release incremental shares, but their
completion paths do not perform the remaining-collateral transfer used by
`confirm_milestone`. Do not assume that reaching `Completed` through one of
these paths returns any remaining collateral.

## Slashing for a missed deadline

The buyer configures a per-miss rate with
`set_collateral_slash_bps(shipment_id, bps)`; zero disables slashing and the
maximum is 10,000 bps. `get_collateral_slash_bps` reads the rate.

Once a milestone's (possibly extension-adjusted) ledger deadline has passed,
any authenticated caller can invoke `slash_collateral_for_miss` if the shipment
is active, the milestone is still `Pending`, and collateral remains. A milestone
can be slashed only once. The slash is calculated from the **remaining**
collateral, not the original base:

```text
slash = remaining_collateral * slash_bps / 10,000
```

It is transferred to the buyer through the contract's buyer-refund path and
subtracted from the locked balance. A result that rounds to zero is rejected.
`is_collateral_slashed` checks whether a milestone has already been slashed.

**Example:** With 1,000,000 units remaining and a 1,000-bps rate, one missed
milestone forfeits 100,000 and leaves 900,000. A later eligible missed
milestone at the same rate would forfeit 90,000 (10% of the then-remaining
900,000). Preconditions, one-time enforcement, and rounding are covered by
[test_slash_collateral.rs](../contracts/chainsetttle/src/test_slash_collateral.rs).

## Cancellation and expiry

- Buyer cancellation with `cancel_shipment`, or cancellation for a blacklisted
  supplier, forfeits remaining collateral to the buyer.
- `claim_deadline_refund` also sends remaining collateral to the primary buyer.
- The unclaimed-refund sweep path sends remaining collateral to the treasury.
- The supplier-cancellation path does not currently transfer or clear the
  stored collateral.

Incremental releases and successful slashes reduce the amount left for any of
these paths. Check the shipment's terminal reason before interpreting its
collateral outcome.

## Alternate collateral token limitation

`set_collateral_token` lets the buyer record a token address for a shipment, and
`get_collateral_token` reads it (or returns the shipment token if unset).
However, the setter requires the shipment to exist, while the initial collateral
deposit happens during shipment creation. The deposit therefore cannot use a
token set through this public setter. In addition, top-ups, incremental
releases, slashes, and terminal transfers currently use the shipment token.
Setting a different token after creation does not move or convert collateral;
the getter alone is not evidence that collateral is held in that token. Treat
alternate collateral tokens as unsupported by the current end-to-end flow.

## Rounding and API checklist

Collateral calculations use integer division and truncate fractional token
units: tier scaling and incremental shares are rounded down, as are slashes.
Slashes use the remaining balance and are capped at that balance; incremental
shares are also capped at what remains.

Collateral-related APIs:

| API | Purpose |
| --- | --- |
| `set_supplier_terms_profile`, `get_supplier_terms_profile` | Set or read supplier defaults, including `default_collateral_bps`. |
| `set_supplier_tier_config`, `get_supplier_tier_config`, `get_supplier_tier` | Configure or inspect reputation-based collateral tiers. |
| `get_locked_collateral`, `top_up_collateral` | Read or add to the stored collateral balance. |
| `set_incremental_collateral`, `get_incremental_collateral` | Enable/disable and inspect milestone-linked release. |
| `set_collateral_slash_bps`, `get_collateral_slash_bps` | Configure or read the missed-deadline slash rate. |
| `slash_collateral_for_miss`, `is_collateral_slashed` | Apply a permitted slash or check the per-milestone flag. |
| `set_collateral_token`, `get_collateral_token` | Set or read the alternate-token address; see the limitation above. |

For the shipment-option shape, see
[shipment options](shipment_options_reference.md). For related payout math, see
the [fee model](fees.md) and [payment release reference](payments.md).
