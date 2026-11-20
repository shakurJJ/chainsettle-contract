# Fee model

This reference describes the fee and payout calculations in the contract. Amounts
are integer token units; rates are basis points (bps), where `10,000 bps = 100%`
and `100 bps = 1%`.

## Milestone payout order

For an immediately released milestone, `confirm_milestone` applies deductions in
this order:

1. **Gross amount.** For a milestone weighted in percent,
   `gross = total_amount * payment_percent / 100`; for a milestone split in bps,
   `gross = total_amount * split_bps / 10,000`. Previously released partial
   quantities are subtracted from this amount.
2. **Late-delivery penalty.** If the proof was submitted after its deadline,
   `penalty = min(gross * (penalty_bps_per_ledger * overdue_ledgers) / 10,000,
   gross / 2)`. A non-zero milestone penalty rate takes precedence over the
   shipment rate. The penalty is returned to the primary buyer.
3. **Approved advance.** The amount already sent to the supplier is subtracted
   from the eventual transfer. It is not subtracted from the platform-fee or
   logistics-fee bases.
4. **Platform fee.** Resolve the rate using the precedence below and apply it
   to `payment = gross - penalty`.
5. **Long-hold rebate.** If enabled, the shipment is old enough, and a platform
   fee was charged, a share of that fee is added back to the supplier's net
   payment.
6. **Logistics fee.** Compute it from `payment`, not from the remaining supplier
   transfer. Deduct and transfer it to the logistics address only if it is
   positive and no greater than the transfer remaining after the advance and
   platform fee.
7. **Completion-only deductions or credits.** On the final milestone, a
   configured referral payment is deducted from the supplier transfer. A clean
   completion rebate, when eligible, is paid separately after shipment
   completion. An early-completion bonus is an additional transfer, not a fee.

With a holdback, confirmation marks the milestone as held rather than performing
the immediate platform and logistics deductions. `preview_milestone_payout`
reports a hypothetical amount after its late-penalty and advance calculations,
but sets platform and logistics fees to zero; it is not the final held-release
transfer. On release, the contract uses a separate platform-fee path: holiday,
then shipment override, then the creation-locked rate, then current
`FeeConfig.fee_bps`. That path does not apply a VIP waiver, recalculate the final
tier, or apply a long-hold rebate. It then deducts any approved advance and
computes logistics from the releasable payment.

### Platform fee rate precedence

Fee resolution is performed when each milestone is paid:

1. An active **fee holiday** takes precedence over every rate setting: the
   platform fee is zero for a confirmation whose current ledger is within the
   configured inclusive start/end range.
2. A per-shipment **fee override**, if present, takes precedence over all other
   rates.
3. Without an override:
   - On a **non-final milestone**, use the buyer's effective rate locked when the
     shipment was created. That rate was the lowest `fee_bps` among the buyer's
     matching lifetime-volume tiers, or the then-current `FeeConfig.fee_bps` if
     no tier matched. At most five tiers may be configured.
   - On the **final milestone**, resolve the buyer's volume tier again using
     current lifetime volume. Use the lowest `fee_bps` among matching tiers, or
     the current `FeeConfig.fee_bps` if no tier matches.
4. Apply an active VIP waiver to the selected rate. A waiver reduces the
   platform rate, not the payout amount: `effective_bps = selected_bps -
   floor(selected_bps * waiver_bps / 10,000)`. The waiver is for the primary
   buyer and expires when its configured Unix timestamp is reached.
5. If a `FeeConfig` exists, `platform_fee = payment * effective_bps / 10,000`.
   If there is no `FeeConfig`, the completion fee path does not charge a
   platform fee. `set_fee_config` caps its configured rate at 1,000 bps; tiers
   and shipment overrides are separate inputs to resolution.

Thus a holiday beats an override; otherwise the override beats the
locked/recalculated tier, and the waiver is applied last. Earlier paid milestones
are not repriced if a tier changes. The final milestone uses the then-current
tier unless an override applies.

### Worked preview example

This example uses a non-final 25% milestone on a `1,000,000` token-unit
shipment. It assumes the primary buyer had a 10% configured base rate, qualified
for an 8% tier when the shipment was created, the shipment override is 7%, an
active 25% VIP waiver applies, the milestone is two ledgers late at 100 bps per
ledger, an approved advance is 4% of gross, and the logistics rate is 500 bps.
There is no active fee holiday or holdback.

| Step | Calculation | Amount |
| --- | --- | ---: |
| Gross | `1,000,000 * 25 / 100` | `250,000` |
| Advance already paid | `250,000 * 4 / 100` | `10,000` |
| Late penalty | `min(250,000 * (100 * 2) / 10,000, 250,000 / 2)` | `5,000` |
| Fee rate selection | Override `700` wins over locked tier `800` and config `1,000` bps | `700 bps` |
| Waiver-adjusted rate | `700 - floor(700 * 2,500 / 10,000)` | `525 bps` |
| Platform fee | `(250,000 - 5,000) * 525 / 10,000` | `12,862` |
| Before logistics | `250,000 - 5,000 - 12,862 - 10,000` | `222,138` |
| Logistics fee | `(250,000 - 5,000) * 500 / 10,000` | `12,250` |
| Supplier net | `222,138 - 12,250` | `209,888` |

`preview_milestone_payout` therefore reports:

```text
gross_amount          = 250000
advance_deducted      = 10000
late_penalty_deducted = 5000
applied_fee_bps       = 525
platform_fee          = 12862
logistics_fee         = 12250
supplier_net_amount   = 209888
would_be_held         = false
is_final_milestone    = false
```

The preview computes the same immediate-milestone fields as confirmation, but
does not mutate state. It does not include a long-hold rebate, final-milestone
referral payment, early-completion bonus, or clean-completion rebate. A held
milestone preview reports zero platform/logistics fee because those deductions
are deferred until release.

## Other fee and rebate paths

These are separate from the ordinary immediate milestone payout calculation:

- **Long-hold rebate:** Admin-configured `threshold_ledgers` and `rebate_bps`.
  When shipment age (`current_ledger - created_at`) meets the threshold and a
  platform fee was charged, `rebate = platform_fee * rebate_bps / 10,000`.
  This is added to the supplier payment and reduces the fee reported for that
  milestone. It applies on the immediate-release confirmation path; either
  setting being zero disables it. For the preview example above, a 1,000 bps
  rebate would add `floor(12,862 * 1,000 / 10,000) = 1,286` to the supplier's
  payment, but does not change the preview result.
- **Clean-completion rebate:** On completion with no dispute, the configured
  `bps` share of the shipment's tracked platform fees is paid to the supplier:
  `rebate = fees_paid * bps / 10,000` (capped at `fees_paid`). It is skipped if
  disabled, there were no platform fees, or there is insufficient available
  contract/treasury revenue. This is paid separately after the final milestone
  and is not in its preview. For example, `fees_paid = 10,000` and a `5,000 bps`
  setting gives a `5,000` rebate. A shipment that incurred an arbiter fee is
  disputed and therefore does not qualify.
- **Referral payment:** If a referrer is set, a referral payment is deducted
  from the final supplier transfer. The base is
  `shipment.total_amount * FeeConfig.fee_bps / 10,000`; the referral rate is
  applied to that base (default `500 bps`, configurable up to `10,000 bps`).
  It is capped at the supplier transfer remaining after the advance, platform
  fee, long-hold rebate, and logistics fee. This uses the configured base rate,
  not the shipment's tier/override/waiver-adjusted milestone fee. For a
  `1,000,000` shipment, a `1,000 bps` `FeeConfig` rate and the default
  `500 bps` referral rate produce a `5,000` referral payment, if the final
  supplier transfer can cover it.
- **Arbiter fee:** Only a disputed monetary settlement pays this fee. The
  contract selects the arbiter rate from configured contested-amount tiers (or
  falls back to the shipment's `arbiter_fee_bps`). Tiers are ordered by
  ascending contested-amount threshold; the rate for the greatest threshold
  not exceeding the contested amount applies. The fee is
  `contested_amount * selected_bps / 10,000` and is paid to the arbiter. For
  example, `100,000` contested at `500 bps` gives a `5,000` fee. The
  dispute-resolution branch and loser-pays rules determine which party
  ultimately bears it; it is not part of `preview_milestone_payout`.
- **Buyer-initiated cancellation fee:** On cancellation,
  `unreleased = total_amount - released_amount - total_advanced_amount`;
  `cancel_fee = unreleased * buyer_cancel_fee_bps / 10,000`. It is paid to the
  supplier and the remainder is refunded to the buyer. For example, `10,000`
  unreleased at `500 bps` means a `500` fee. This rate is capped at 1,000 bps
  (10%).
- **Supplier-cancellation penalty:** If enabled and its response deadline has
  passed, `penalty = remaining * penalty_bps / 10,000` is paid to the supplier;
  the rest is refunded to the buyer.
- **Dispute bond:** This is collateral, not a fee. The per-dispute bond is the
  configured flat amount plus `total_amount * dispute_bond_bps / 10,000`, then
  locked for each milestone-dispute bond. It may be returned or used to cover
  arbiter fees under loser-pays rules.

## Rounding

All calculations use integer division in token base units, so fractional results
are truncated toward zero (for the non-negative fee amounts described here,
this is rounding down). This applies independently at each fee/penalty/rebate
calculation; the contract does not carry fractional remainders between
milestones. Rate and amount formulas therefore use the exact integer operations
shown above. A fee or logistics transfer is made only when its computed amount
is positive; the logistics fee is also skipped if it exceeds the remaining
supplier transfer.

For the APIs and related configuration, see [query reference](queries.md),
[shipment options](shipment_options_reference.md), and
[late-confirmation fee](late_confirmation_fee.md).
