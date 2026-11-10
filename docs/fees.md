# Fee Model Reference

ChainSettle's fee engine combines a configurable protocol fee (with volume tiers, per-shipment overrides, scheduled fee holidays, multisig VIP waivers, and rebates) with counterparty fees for logistics providers, arbiters, referrers, and shipment cancellations.

All entry points referenced below live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs).

---

## 1. Platform Fee Evaluation & Precedence Order

Whenever a milestone or advance deducts the protocol fee (`deduct_fee_for_shipment` or `deduct_fee_for_shipment_at_completion`), the effective rate and net fee are resolved in the following strict order:

```
┌───────────────────────────────────────────────────────────────────────────┐
│ Step 1: Scheduled Fee Holiday (#413)                                      │
│   Is current_ledger in [holiday.start_ledger, holiday.end_ledger]?        │
│   ──► YES: Effective fee = 0 (no tokens transferred). Stop.               │
└───────────────────────────────────────┬───────────────────────────────────┘
                                        │ NO
                                        ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ Step 2: Base Rate Resolution                                              │
│   2a. Per-shipment override set? (`ShipmentFeeOverride(shipment_id)`)     │
│       ──► Use override `o` bps.                                           │
│   2b. Else if confirming the FINAL milestone (#399)?                      │
│       ──► Recalculate buyer's live volume tier `resolve_fee_bps_for`      │
│           (lowest matching `FeeTier.fee_bps`, or `FeeConfig.fee_bps`).    │
│   2c. Else (non-final milestone or advance):                              │
│       ──► Use `ShipmentFeeBps(shipment_id)` locked at creation            │
│           (or `FeeConfig.fee_bps`).                                       │
└───────────────────────────────────────┬───────────────────────────────────┘
                                        │
                                        ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ Step 3: VIP Partner Fee Waiver (#388)                                     │
│   Does `buyers[0]` hold an active, unexpired `FeeWaiver` (`waiver_bps`)?  │
│   ──► effective_bps = bps - (bps * waiver_bps / 10_000)                   │
│   ──► raw_fee = amount * effective_bps / 10_000                           │
└───────────────────────────────────────┬───────────────────────────────────┘
                                        │
                                        ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ Step 4: Long-Hold Escrow Rebate (#300, on immediate `confirm_milestone`)  │
│   Is `current_ledger - shipment.created_at >= threshold_ledgers`?         │
│   ──► rebate = raw_fee * rebate_bps / 10_000 (transferred to supplier)    │
│   ──► net_treasury_fee = raw_fee - rebate                                 │
└───────────────────────────────────────┬───────────────────────────────────┘
                                        │
                                        ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ Step 5: Clean-Completion Fee Rebate (#528, on `settle_on_completion`)     │
│   Did the shipment complete with zero disputes (`!ShipmentHadDispute`)?   │
│   ──► Refunds `ShipmentFeesPaid * clean_rebate_bps / 10_000` to supplier. │
└───────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Platform Fee Configuration & Modifiers

### 2.1 Base Protocol Fee (`set_fee_config`) and Multi-Recipient Splits (`set_fee_recipients`)

| Function | Auth | Description |
| --- | --- | --- |
| `set_fee_config(env, admin, treasury, fee_bps)` | `Role::FeeManager` or `Admin` | Sets the default `FeeConfig { treasury, fee_bps }` at `DataKey::FeeConfig`. `fee_bps` is capped at **`MAX_FEE_BPS = 1_000` (`10.00%`)**. Emits `fee_config_updated`. |
| `set_fee_recipients(env, admin, recipients: Vec<FeeRecipient>)` | `Role::FeeManager` or `Admin` | Configures up to 10 `FeeRecipient { recipient, share_bps }` entries (`DataKeyExt3::FeeRecipients`) whose `share_bps` sum to **`10_000` (`100%`)**. When non-empty, every protocol fee is split across `recipients` instead of going solely to `FeeConfig.treasury`. |
| `get_fee_recipients(env) -> Vec<FeeRecipient>` | Read-only | Returns the configured multi-recipient split table. |
| `get_treasury_revenue(env, token) -> i128` | Read-only | Returns cumulative protocol fees collected for `token` (`DataKeyExt::TreasuryRevenue(token)`). |
| `get_shipment_fees_paid(env, shipment_id) -> i128` | Read-only | Returns cumulative protocol fees paid across all milestones of `shipment_id` (`DataKeyExt4::ShipmentFeesPaid(shipment_id)`). |

> **Rounding Note for Multi-Recipient Splits:** Recipients `1..N-1` each receive $\lfloor \text{fee} \times \text{share\_bps} / 10{,}000 \rfloor$, and recipient `0` receives `fee - distributed` so zero dust is stranded.

### 2.2 Volume-Based Fee Tiers (`#113` & `#399`)

High-volume buyers unlock lower protocol fee rates automatically based on their cumulative `DataKeyExt::LifetimeVolume(buyer)` (incremented by `total_amount` at `create_shipment`):

- **`set_fee_tiers(env, admin, tiers: Vec<FeeTier>)`** (`Role::FeeManager` or `Admin`):
  - Up to **5** `FeeTier { min_lifetime_volume: i128, fee_bps: u32 }` entries (`fee_bps <= 1_000`).
- **`get_fee_tier(env, address) -> u32`** (Read-only):
  - Evaluates all configured tiers where `lifetime_volume >= tier.min_lifetime_volume` and selects the **lowest `fee_bps`** (falling back to `FeeConfig.fee_bps` if no tier matches).
- **Creation Lock vs. Final-Milestone Recalculation (`#399`):**
  - At `create_shipment`, the primary buyer's resolved rate is snapshotted at `DataKeyExt::ShipmentFeeBps(shipment_id)` and used for all non-final milestones.
  - When confirming the **final milestone** (`is_final_milestone == true`), `deduct_fee_for_shipment_at_completion` recomputes `resolve_fee_bps_for(env, buyer)` live so volume accumulated while the shipment was in flight can unlock a lower tier on the closing payment.

### 2.3 Per-Shipment Fee Override (`#299`)

For negotiated enterprise contracts, a `FeeManager` or `Admin` can override the protocol fee for a specific shipment:
- **`set_shipment_fee_override(env, admin, shipment_id, fee_bps)`:** Sets `DataKeyExt::ShipmentFeeOverride(shipment_id)` (`fee_bps <= 1_000`). Takes precedence over both `ShipmentFeeBps` and volume tiers.
- **`clear_shipment_fee_override(env, admin, shipment_id)`:** Removes the override so the shipment reverts to its volume tier / `FeeConfig`.
- **`get_shipment_fee_override(env, shipment_id) -> Option<u32>`:** Inspects the current override.

### 2.4 Scheduled Fee Holidays (`#413`)

A `FeeManager` or `Admin` can schedule promotional windows during which protocol fees are waived (`0 bps`) across all shipments:
- **`schedule_fee_holiday(env, admin, start_ledger, end_ledger)`:** Requires `end_ledger >= start_ledger` and `end_ledger > current_ledger`. Stores `FeeHoliday { start_ledger, end_ledger }` at `DataKeyExt3::FeeHoliday`.
- **`cancel_fee_holiday(env, admin)`:** Deletes the active or upcoming holiday (`get_fee_holiday(env) -> Option<FeeHoliday>`).

### 2.5 Multisig VIP Partner Fee Waivers (`#388`)

When `MultiAdminConfig` is configured, multisig admins can grant time-bounded multiplicative fee discounts (`1..=10_000` bps) to a `partner` buyer address:
- **`propose_fee_waiver(env, admin, partner, waiver_bps, expires_at_ledger) -> u64`:** Proposes a waiver (`expires_at_ledger > current_ledger`).
- **`approve_fee_waiver(env, admin, proposal_id)`:** Once `approvals.len() >= config.threshold`, activates `FeeWaiver { partner, waiver_bps, expires_at_ledger }` at `DataKeyExt2::FeeWaiver(partner)`.
- **`revoke_fee_waiver(env, admin, partner)`:** Any multisig admin (or primary `Admin`) can revoke an active waiver immediately.
- **Calculation:** Reduces the resolved base `bps` multiplicatively:
  $$\text{effective\_bps} = \text{bps} - \left\lfloor \frac{\text{bps} \times \text{waiver\_bps}}{10{,}000} \right\rfloor$$
  (`waiver_bps = 10_000` yields a 100% waiver = `0 bps`). Automatically expires once `env.ledger().sequence() >= expires_at_ledger`.

### 2.6 Long-Hold Escrow Rebate (`#300`) and Clean-Completion Rebate (`#528`)

| Rebate | Configuration | Trigger & Formula |
| --- | --- | --- |
| **Long-Hold Escrow Rebate (`#300`)** | `set_long_hold_rebate(env, admin, threshold_ledgers, rebate_bps)` / `get_long_hold_rebate(env) -> (u32, u32)` (`rebate_bps <= 10_000`) | On immediate `confirm_milestone`, if `current_ledger - shipment.created_at >= threshold_ledgers`, refunds $\lfloor \text{fee\_amount} \times \text{rebate\_bps} / 10{,}000 \rfloor$ to `shipment.supplier` (`fee_rebate_issued`) and sends only the remainder to treasury. |
| **Clean-Completion Fee Rebate (`#528`)** | `set_clean_completion_rebate_bps(env, admin, bps)` / `get_clean_completion_rebate_bps(env) -> u32` (`bps <= 10_000`) | On `settle_on_completion`, if the shipment completed with **zero disputes** (`!ShipmentHadDispute(shipment_id)`), refunds $\lfloor \text{ShipmentFeesPaid} \times \text{bps} / 10{,}000 \rfloor$ (bounded by available contract token balance above `TotalEscrowed`) to `shipment.supplier` (`clean_completion_rebate`). |

---

## 3. Counterparty & Auxiliary Fees

| Fee Type | Configured By | Cap / Default | When Charged & How Calculated | Recipient |
| --- | --- | --- | --- | --- |
| **Logistics Fee** | `ShipmentOptions.logistics_fee_bps` | `0..=1_000` bps (`10%`) | Deducted on milestone confirmation, holdback release, auto-confirm, or partial-quantity confirm: $\lfloor \text{payment} \times \text{logistics\_fee\_bps} / 10{,}000 \rfloor$ (bounded by post-fee/post-advance balance). | `shipment.logistics` |
| **Arbiter Fee** | `ShipmentOptions.arbiter_fee_bps` or global `set_arbiter_fee_tiers` | Per-shipment `0..=1_000` bps (`10%`); tiered schedule up to `5_000` bps (`50%`) | Deducted when an arbiter or panel resolves a dispute (`resolve_dispute`, `resolve_dispute_split`, `cast_dispute_vote`, `finalize_dispute_resolution`): $\lfloor \text{contested\_amount} \times \text{applicable\_bps} / 10{,}000 \rfloor$. Not charged on mediation (`accept_mediation`) or timeout resolution (`resolve_dispute_timeout`). | `shipment.arbiter` (or split equally among panel voters via `distribute_panel_fee`) |
| **Appeal Fee (`#369`)** | `set_appeal_fee(env, admin, fee)` (`get_appeal_fee`) | Flat token amount (`>= 0`, default `0`) | Escrowed by the appellant when calling `appeal_dispute`. If the appeal arbiter rules in the appellant's favor, refunded in full (`appeal_fee_refunded`); otherwise forfeited (`appeal_fee_forfeited`). | Refunded to appellant on win; paid to appeal arbiter on loss |
| **Referral Fee (`#105`)** | `set_referral_fee_bps(env, admin, bps)` + `ShipmentOptions.referrer` | `0..=10_000` bps of protocol fee (default `500` bps = `5%` of protocol fee) | Paid on the **final milestone** when `shipment.referrer` is `Some(r)` and `FeeConfig` is set: $\text{total\_fee} = \lfloor \text{total\_amount} \times \text{fee\_bps} / 10{,}000 \rfloor$, $\text{referral} = \lfloor \text{total\_fee} \times \text{referral\_bps} / 10{,}000 \rfloor$, deducted from the supplier's final milestone transfer. | `shipment.referrer` |
| **Buyer Cancellation Fee** | `ShipmentOptions.buyer_cancel_fee_bps` | `0..=1_000` bps (`10%`) | Charged when a buyer calls `cancel_shipment` while the shipment is in `Created` status: $\lfloor \text{unreleased} \times \text{buyer\_cancel\_fee\_bps} / 10{,}000 \rfloor$. Remainder is refunded to `buyers`. | `shipment.supplier` |
| **Supplier Cancellation Penalty** | `set_cancel_policy(env, admin, shipment_id, CancelPolicy)` | `penalty_bps` in `0..=10_000` bps | Charged when `supplier_cancel` is called on a `Created` shipment after `buyer_response_deadline_ledgers` has elapsed with zero confirmed milestones: $\lfloor \text{remaining} \times \text{penalty\_bps} / 10{,}000 \rfloor$. | `shipment.supplier` |

### Arbiter Fee Tier Schedule & Loser-Pays Option

- **Global Arbiter Fee Tiers (`set_arbiter_fee_tiers(env, admin, tiers: Vec<(i128, u32)>)` / `get_arbiter_fee_tiers`):**
  - Sorted ascending by `min_contested_amount`, with `fee_bps <= 5_000`.
  - When a dispute is resolved, `applicable_arbiter_fee_bps` selects the highest tier where `contested_amount >= min_contested_amount`; if no tier matches (or none are configured), it falls back to `shipment.arbiter_fee_bps`.
- **Loser-Pays Allocation (`set_loser_pays_arbiter_fee(env, admin, enabled)` / `get_loser_pays_arbiter_fee`):**
  - By default (`false`), the arbiter fee is deducted from the prevailing payout/refund pool.
  - When `true`, on `approve = true` (supplier wins, buyer loses), the buyer's returned `dispute_bond_amount` first covers up to `arbiter_fee` so the supplier's milestone payout is only reduced by any uncovered remainder. In a partial dispute where `approve = false` (buyer wins), the forfeited dispute bond covers `arbiter_fee` first before the remainder goes to the supplier.
