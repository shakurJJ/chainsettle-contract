# Payment Release Variants and Deduction Pipeline

ChainSettle supports multiple milestone settlement paths—immediate confirmation, post-confirmation holdbacks, permissionless auto-confirmation, quality-graded payouts, incremental partial-quantity releases, pre-confirmation advances, retainage, and post-completion warranty holdbacks—as well as flexible recipient routing (invoice factoring, multi-payee splits, payout address overrides, and batched multi-currency claims).

All entry points referenced below live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs).

---

## 1. Gross Milestone Weight Calculation

Each milestone's gross allocation (`milestone_gross_payment`) is computed from `shipment.total_amount` using one of two weighting modes:

1. **Whole-Percent Mode (default):**
   Each `Milestone` defines `payment_percent: u32` (`1..=100`, summing to `100` across all milestones):
   $$\text{full\_gross} = \left\lfloor \frac{\text{total\_amount} \times \text{payment\_percent}}{100} \right\rfloor$$
2. **Basis-Point Split Mode (`ShipmentOptions.milestone_splits`):**
   When `options.milestone_splits` is non-empty (`DataKeyExt::MilestoneSplits(shipment_id)`), each milestone weight is specified in basis points (`1..=10_000`, summing to `10_000`):
   $$\text{full\_gross} = \left\lfloor \frac{\text{total\_amount} \times \text{split\_bps}}{10{,}000} \right\rfloor$$

*(If partial quantities have already been released on a quantity-based milestone via `confirm_partial_quantity`, `milestone_gross_payment` returns `full_gross - PartialQtyReleased(shipment_id, milestone_index)` so the final confirmation settles only the remaining balance.)*

---

## 2. Deduction and Bonus Pipeline Order

When a milestone settles through standard confirmation (`confirm_milestone` with `holdback_ledgers == 0`), holdback release (`release_held_payment`), or auto-confirmation (`claim_auto_confirmation`), adjustments are applied in the following strict order:

| Step | Adjustment | Formula & Recipient |
| --- | --- | --- |
| **1. Gross Payment** | `gross_payment` | Computed from `payment_percent` or `milestone_splits` (less any prior partial-quantity releases). |
| **2. Late-Delivery Penalty** | Subtracted from `gross_payment` (`payment = gross_payment - penalty`) | Applied if `milestone.deadline_ledger > 0` and `proof_submitted_ledger > deadline_ledger`. Uses `milestone.penalty_bps_per_ledger` if `> 0`, else `shipment.late_penalty_bps_per_ledger`. Raw penalty $\lfloor \text{gross} \times (\text{bps} \times \text{overdue\_ledgers}) / 10{,}000 \rfloor$ is **capped at 50% of `gross_payment`** (`gross_payment / 2`) and refunded immediately to `buyers[0]`. |
| **3. Platform Fee** | Subtracted from `payment` (`net = payment - fee`) | Computed via `deduct_fee_for_shipment_at_completion` on the post-penalty `payment` and transferred to `FeeConfig.treasury` (or split across `FeeRecipients`). Applies fee holidays, per-shipment overrides, volume tiers (recalculated live on the final milestone), VIP waivers, and long-hold escrow rebates. See [`docs/fees.md`](fees.md). |
| **4. Advance Consumption** | Subtracted from `net` (`after_advance = net - advanced`) | If the buyer previously approved an advance (`approve_advance`) for this milestone, `consume_advance_for_milestone` deducts the already-disbursed `amount_advanced` and decrements `shipment.total_advanced_amount`. |
| **5. Logistics Fee** | Subtracted from `after_advance` (`actual_transfer = after_advance - logistics_fee`) | $\lfloor \text{payment} \times \text{logistics\_fee\_bps} / 10{,}000 \rfloor$ (bounded by `after_advance`) is transferred directly to `shipment.logistics`. |
| **6. Referral Fee (Final Milestone Only)** | Subtracted from `actual_transfer` | On the final milestone only, if `shipment.referrer` is `Some(referrer)` and `FeeConfig` is set: $\text{total\_fee} = \lfloor \text{total\_amount} \times \text{fee\_bps} / 10{,}000 \rfloor$, $\text{referral} = \lfloor \text{total\_fee} \times \text{referral\_bps} / 10{,}000 \rfloor$ (default `500` bps = 5% of protocol fee), transferred to `referrer`. |
| **7. Retainage & Warranty Holdbacks** | Subtracted from `actual_transfer` inside `pay_milestone_to_payees` | $\text{retained} = \lfloor \text{actual\_transfer} \times \text{retainage\_bps} / 10{,}000 \rfloor$ and $\text{warranty} = \lfloor \text{actual\_transfer} \times \text{warranty\_bps} / 10{,}000 \rfloor$ are parked in `RetainageBalance(shipment_id)` and `WarrantyBalance(shipment_id)`. The remainder $\text{net\_to\_payees} = \text{actual\_transfer} - \text{retained} - \text{warranty}$ is disbursed to the supplier/payees. |
| **8. Early-Completion Bonus** | Added from `early_bonus_pool` | If `shipment.early_bonus_pool > 0`, `milestone.deadline_ledger > 0`, and confirmation occurs at `current_ledger <= deadline_ledger`, pays $\lfloor \text{early\_bonus\_pool} / \text{milestones.len()} \rfloor$ directly to `shipment.supplier`. Any unearned bonus pool is refunded to `buyers[0]` when the shipment completes. |

---

## 3. Milestone Release Variants

### 3.1 Standard Confirmation (`confirm_milestone` & `batch_confirm_milestones`)

- **Authorized Callers:**
  - Any buyer in `shipment.buyers`.
  - **Co-Buyer Joint Confirmation (`#367`):** If `set_joint_confirmation_threshold(env, admin, min_amount)` is configured and `shipment.total_amount >= min_amount` on a shipment with multiple buyers, **every** buyer in `shipment.buyers` must call `confirm_milestone`. Preliminary calls record the buyer in `DataKeyExt2::MilestoneBuyerApprovals(shipment_id, milestone_index)` and emit `milestone_co_buyer_approved`; settlement executes only when the final buyer approves.
  - **Confirmation Delegate (`#306`):** A delegate registered via `set_confirmation_delegate(env, buyer, shipment_id, delegate, per_tx_cap)` can confirm milestones whose gross payment is $\le \text{per\_tx\_cap}$.
- **Preconditions:**
  - Milestone status is `MilestoneStatus::ProofSubmitted`.
  - Sequential order (`milestone_index > 0` requires `milestones[milestone_index - 1]` to be `Confirmed`, `ConfirmedHeld`, `Resolved`, or `ResolvedPendingFinality`, unless `parallel_milestones` is enabled).
  - If an inspector is assigned (`assign_inspector`) and `set_require_inspection_for_confirm` is `true`, `InspectionReport.passed` must exist (`#543`).
  - If an oracle attestation requirement is set (`set_milestone_oracle_requirement`), at least `required_count` registered oracles must have called `submit_oracle_attestation` (`#390`).
  - If `confirmation_cooldown_ledgers > 0`, `current_ledger >= proof_submitted_ledger + cooldown` (bypassed when the supplier qualifies for fast-track via `is_fast_track_eligible`).
  - The auto-confirmation window has not yet elapsed.

### 3.2 Holdback Release (`holdback_ledgers > 0` → `release_held_payment`)

When `shipment.holdback_ledgers > 0`:
1. Calling `confirm_milestone` deducts any late-delivery penalty, pays any early-completion bonus, and parks the milestone in `MilestoneStatus::ConfirmedHeld` with `release_after_ledger = current_ledger + holdback_ledgers` (emits `payment_held`).
2. While in `ConfirmedHeld` (before `release_after_ledger`), the buyer can still open a dispute (`raise_dispute` or `raise_partial_dispute`).
3. Once `env.ledger().sequence() >= release_after_ledger`, anyone can call **`release_held_payment(env, shipment_id, milestone_index)`** (permissionless) to run Steps 3–7 of the deduction pipeline, transition the milestone to `Confirmed`, and emits `held_payment_released`.

### 3.3 Auto-Confirmation (`claim_auto_confirmation`)

If the buyer neither confirms nor disputes a `ProofSubmitted` milestone within the review window, the supplier is protected against buyer ghosting:
- **Effective window resolution:**
  1. `shipment.review_window_ledgers` (if `Some(w)` with `w > 0`), else
  2. `shipment.auto_confirm_ledgers` (if `> 0`), else
  3. Global `DataKey::AutoConfirmThreshold` (`set_auto_confirm_threshold`).
- Once `env.ledger().sequence() >= proof_submitted_ledger + effective_window`, manual `confirm_milestone` and `raise_dispute` are rejected (`AutoConfirmWindowElapsed`), and anyone can call **`claim_auto_confirmation(env, shipment_id, milestone_index)`** to execute the full deduction and payout pipeline.

### 3.4 Quality-Graded Confirmation (`confirm_milestone_graded`)

For goods where delivery quality determines payout percentage (e.g., Grade A = 100%, Grade B = 85%, Grade C = 70%):
1. **Configuration:** Set `ShipmentOptions.quality_grades: Vec<u32>` at shipment creation (up to `MAX_QUALITY_GRADES = 10` entries in basis points, non-increasing, with `quality_grades[0] == 10_000`). Query via `get_quality_grades(env, shipment_id)`.
2. **Buyer Grades Delivery:** Buyer calls **`confirm_milestone_graded(env, buyer, shipment_id, milestone_index, grade)`**.
   - **Grade `0` (`10_000` bps):** Immediately executes standard `confirm_milestone` at 100% payout and records `(0, 10_000)` at `DataKeyExt3::PendingGrade`.
   - **Lower Grade (`grade > 0`, `grade_bps < 10_000`):** Parks the milestone in `MilestoneStatus::ConfirmedHeld` for `review_window = max(holdback_ledgers, effective_auto_confirm_window)` (or `DEFAULT_GRADE_REVIEW_WINDOW_LEDGERS = 1_000` if both are `0`), stores `(grade, grade_bps)` at `DataKeyExt3::PendingGrade(shipment_id, milestone_index)`, and emits `milestone_graded`.
3. **Supplier Challenge Window:**
   - Before `release_after_ledger`, **only the supplier** may challenge the reduced grade by calling `raise_dispute(env, supplier, shipment_id, milestone_index)`.
   - If unchallenged once `current_ledger >= release_after_ledger`, calling `release_held_payment(env, shipment_id, milestone_index)` computes:
     $$\text{supplier\_gross} = \left\lfloor \frac{\text{gross\_payment} \times \text{grade\_bps}}{10{,}000} \right\rfloor, \quad \text{buyer\_refund} = \text{gross\_payment} - \text{supplier\_gross}$$
     Deducts platform and logistics fees from `supplier_gross`, pays the net amount to the supplier (with retainage/warranty holdbacks applied), refunds `buyer_refund` to `buyers[0]`, and emits `grade_settled`.

### 3.5 Quantity-Based Pro-Rata Confirmation (`confirm_partial_quantity`)

For bulk shipments delivered in sub-lots against a single milestone:
1. **Configuration:** Set `ShipmentOptions.milestone_quantities: Vec<u32>` (`DataKeyExt3::MilestoneQuantities(shipment_id)`), where entry `i` is the expected unit count for milestone `i` (`0` = not quantity-based). Note: partial-quantity confirmation cannot be combined with `holdback_ledgers > 0`, co-buyer joint confirmation, or an approved milestone advance.
2. **Incremental Calls:** While the milestone is `ProofSubmitted`, the buyer calls **`confirm_partial_quantity(env, buyer, shipment_id, milestone_index, delivered_qty)`**:
   - Updates `cumulative_qty = previous_qty + delivered_qty` (`cumulative_qty <= expected_qty`).
   - **If `cumulative_qty < expected_qty`:**
     $$\text{target\_released} = \left\lfloor \frac{\text{full\_gross} \times \text{cumulative\_qty}}{\text{expected\_qty}} \right\rfloor, \quad \text{release\_amount} = \text{target\_released} - \text{released\_before}$$
     Deducts platform fee (`deduct_fee_for_shipment`) and logistics fee from `release_amount`, transfers the net share via `pay_milestone_to_payees` (applying retainage/warranty holdbacks), updates `DataKeyExt3::PartialQtyReleased(shipment_id, milestone_index) = target_released`, keeps the milestone in `ProofSubmitted`, and emits `partial_quantity_confirmed`.
   - **If `cumulative_qty == expected_qty`:**
     Delegates to `confirm_milestone`, which uses `milestone_gross_payment = full_gross - released_before` (automatically absorbing any integer rounding dust) and transitions the milestone to `Confirmed`.
3. **Query Progress:** Call `get_delivered_quantity(env, shipment_id, milestone_index) -> (u32, u32)` to read `(delivered_so_far, expected_total)`.

### 3.6 Pre-Confirmation Advances (`request_advance` / `approve_advance`)

Suppliers can request working-capital advances on `Pending` milestones before submitting proof of delivery:
1. **`request_advance(env, supplier, shipment_id, milestone_index, amount)`:**
   - Milestone must be `MilestoneStatus::Pending` with no existing advance.
   - `amount` is capped by global `DataKey::MaxAdvancePercent` (default `30%`, configurable via `set_max_advance_percent`):
     $$\text{amount} \le \left\lfloor \frac{\text{milestone\_gross} \times \text{max\_advance\_pct}}{100} \right\rfloor$$
   - Stores `MilestoneAdvance { amount_requested: amount, amount_advanced: 0, approved: false }` at `DataKey::MilestoneAdvance(shipment_id, milestone_index)`.
2. **`approve_advance(env, buyer, shipment_id, milestone_index)`:**
   - Buyer approves the request; contract deducts the platform fee on `amount`, transfers `net_advance` immediately to the supplier (or active invoice-factoring `financier`), increments `shipment.total_advanced_amount += amount`, and sets `approved = true`.
   - When the milestone is later confirmed or resolved in the supplier's favor, `consume_advance_for_milestone` subtracts `amount_advanced` from the milestone payout.

### 3.7 Retainage (`#520`) and Post-Completion Warranty Holdback (`#521`)

Configured via `ShipmentOptions`:
- **`retainage_bps`:** `0..=2_000` (up to `MAX_RETAINAGE_BPS = 20%`), stored at `DataKeyExt3::RetainageBps(shipment_id)`.
- **`warranty_bps` & `warranty_ledgers`:** `warranty_bps` in `0..=2_000` (up to `MAX_WARRANTY_BPS = 20%`); both must be `> 0` together (`DataKeyExt3::WarrantyConfig(shipment_id)`).

**Lifecycle:**
1. **Withholding on Each Milestone (`withhold_settlement_holdbacks`):**
   Every supplier milestone payout withholds `retained = actual_transfer * retainage_bps / 10_000` into `RetainageBalance(shipment_id)` and `warranty = actual_transfer * warranty_bps / 10_000` into `WarrantyBalance(shipment_id)`, emitting `holdback_withheld`.
2. **Retainage Release on Shipment Completion (`settle_on_completion`):**
   As soon as the final milestone is confirmed or resolved and `shipment.status` becomes `Completed`, 100% of `RetainageBalance(shipment_id)` is automatically released to `shipment.supplier` (emitting `retainage_released`).
3. **Post-Completion Warranty Window:**
   - On shipment completion, if `WarrantyBalance(shipment_id) > 0`, the contract sets `DataKeyExt3::WarrantyEndsAt(shipment_id) = current_ledger + warranty_ledgers` and emits `warranty_started`.
   - **Filing a Claim (`file_warranty_claim(env, buyer, shipment_id, evidence_hash)`):** Before `WarrantyEndsAt`, any buyer on the shipment can submit a non-zero `BytesN<32>` evidence hash. This records a `WarrantyClaim` (`resolved: false`) and blocks `release_warranty`.
   - **Resolving a Claim (`resolve_warranty_claim(env, arbiter, shipment_id, refund_buyer)`):** The shipment's arbiter rules on the claim:
     - `refund_buyer = true`: zeroes `WarrantyBalance` and transfers the full warranty pool to `claim.buyer`.
     - `refund_buyer = false`: marks the claim resolved and leaves `WarrantyBalance` intact until `WarrantyEndsAt`.
   - **Releasing Unclaimed Warranty (`release_warranty(env, shipment_id)`):** Once `env.ledger().sequence() >= WarrantyEndsAt` and no unresolved `WarrantyClaim` exists, anyone can call `release_warranty` to pay the warranty balance to `shipment.supplier` (`warranty_released`).
   - *(If a shipment is cancelled or expires before completion, `refund_holdbacks_on_cancel` refunds any accumulated retainage and warranty balances to `buyers[0]`.)*

---

## 4. Payout Routing and Recipient Precedence

After retainage and warranty holdbacks are withheld in `pay_milestone_to_payees`, the remaining `net_to_payees` is routed according to the following strict priority order:

1. **Invoice Factoring Assignment (`assign_payout` / `transfer_payout_assignment` / `release_payout_assignment`):**
   - If the supplier has assigned shipment receivables to a `financier` via `assign_payout(env, supplier, shipment_id, financier)` (`DataKeyExt4::PayoutAssignment(shipment_id)`), **100% of `net_to_payees` (as well as any approved advance) is transferred directly to `assignment.financier`**, bypassing milestone payee splits and batched payout mode.
2. **Multi-Payee Milestone Splits (`set_milestone_payees` / `get_milestone_payees`):**
   - While a milestone is `Pending`, the buyer can configure `Vec<MilestonePayee>` (`address`, `percent` summing to `100`).
   - Each payee receives $\lfloor \text{net\_to\_payees} \times \text{percent} / 100 \rfloor$, with the final payee receiving `net_to_payees - distributed` so zero rounding dust is lost.
3. **Supplier Payout Address Override (`set_supplier_payout_address` / `clear_supplier_payout_address`):**
   - When no factoring assignment or payee split applies, funds route to `shipment.supplier_payout_address` if set, or else to the effective milestone supplier (`get_milestone_supplier(shipment_id, milestone_index)` if sub-contracted via `assign_milestone_supplier`, otherwise `shipment.supplier`).
4. **Immediate Transfer vs. Batched Payout Mode (`set_payout_mode` & `claim_payout`):**
   - By default (`PayoutMode::Immediate`), each payout executes a token transfer immediately.
   - If a recipient calls `set_payout_mode(env, supplier, true)` (`PayoutMode::Batched`), payouts accumulate in `DataKeyExt::PendingPayout(recipient)` (`get_pending_payout`).
   - Calling **`claim_payout(env, supplier, token)`** withdraws the accumulated balance. If the supplier configured `set_payout_currency_preference(env, supplier, preferred_token)` and the admin registered a conversion rate via `set_conversion_rate(env, admin, from_token, to_token, rate_bps)` (and the contract holds sufficient `preferred_token` liquidity), `claim_payout` converts $\lfloor \text{pending} \times \text{rate\_bps} / 10{,}000 \rfloor$ and transfers `preferred_token`; otherwise it transfers `pending` in `token`.

---

## 5. Worked Numerical Example

Consider a shipment with the following parameters:
- **`total_amount`:** `100_000` units
- **Milestones (3):** M0 = `25%` (`25_000`), M1 = `50%` (`50_000`), M2 = `25%` (`25_000`)
- **Platform Fee (`fee_bps`):** `100` bps (`1.00%`)
- **Logistics Fee (`logistics_fee_bps`):** `50` bps (`0.50%`)
- **Retainage (`retainage_bps`):** `500` bps (`5.00%`)
- **Warranty (`warranty_bps`):** `500` bps (`5.00%`) for `10_000` ledgers
- **Late Penalty (`late_penalty_bps_per_ledger`):** `10` bps (`0.10%`) per overdue ledger

### Milestone 0 (`25_000` gross) — Confirmed On Time (`overdue = 0`)

1. **Gross Payment:** `25_000`
2. **Late Penalty:** `0` (`payment = 25_000`)
3. **Platform Fee (`1.00%` of `25_000`):** `250` → transferred to Treasury (`net = 24_750`)
4. **Logistics Fee (`0.50%` of `25_000`):** `125` → transferred to Logistics (`actual_transfer = 24_625`)
5. **Retainage (`5.00%` of `24_625`):** $\lfloor 24{,}625 \times 500 / 10{,}000 \rfloor = 1{,}231$ → parked in `RetainageBalance`
6. **Warranty (`5.00%` of `24_625`):** $\lfloor 24{,}625 \times 500 / 10{,}000 \rfloor = 1{,}231$ → parked in `WarrantyBalance`
7. **Immediate Net to Supplier:** $24{,}625 - 1{,}231 - 1{,}231 = \mathbf{22{,}163}$

### Milestone 1 (`50_000` gross) — Confirmed 100 Ledgers Late (`overdue = 100`)

1. **Gross Payment:** `50_000`
2. **Late Penalty (`10 bps * 100 = 1_000 bps = 10%` of `50_000`):** `5_000` → refunded to `buyers[0]` (`payment = 45_000`)
3. **Platform Fee (`1.00%` of `45_000`):** `450` → transferred to Treasury (`net = 44_550`)
4. **Logistics Fee (`0.50%` of `45_000`):** `225` → transferred to Logistics (`actual_transfer = 44_325`)
5. **Retainage (`5.00%` of `44_325`):** $\lfloor 44{,}325 \times 500 / 10{,}000 \rfloor = 2{,}216$ → parked in `RetainageBalance` (total retainage = `3_447`)
6. **Warranty (`5.00%` of `44_325`):** $\lfloor 44{,}325 \times 500 / 10{,}000 \rfloor = 2{,}216$ → parked in `WarrantyBalance` (total warranty = `3_447`)
7. **Immediate Net to Supplier:** $44{,}325 - 2{,}216 - 2{,}216 = \mathbf{39{,}893}$

### Milestone 2 (`25_000` gross, Final Milestone) — Confirmed On Time (`overdue = 0`)

1. **Gross Payment:** `25_000`
2. **Platform Fee (`1.00%`):** `250` → Treasury; **Logistics Fee (`0.50%`):** `125` → Logistics (`actual_transfer = 24_625`)
3. **Retainage & Warranty (`5.00%` each):** `1_231` each (total `RetainageBalance = 4_678`, total `WarrantyBalance = 4_678`)
4. **Immediate Milestone Net to Supplier:** $\mathbf{22{,}163}$
5. **Automatic Completion Settlement (`settle_on_completion`):**
   - Full `RetainageBalance` (**`4_678`**) is transferred immediately to Supplier.
   - `WarrantyEndsAt` is set to `current_ledger + 10_000`. After `10_000` ledgers with no warranty claim, calling `release_warranty` transfers the remaining **`4_678`** to Supplier.

### Summary of Total `100_000` Escrow Distribution

| Recipient / Bucket | M0 | M1 | M2 + Completion | Post-Warranty | **Total** |
| --- | --- | --- | --- | --- | --- |
| **Buyer (Late Penalty Refund)** | `0` | `5_000` | `0` | `0` | **`5_000`** |
| **Treasury (Platform Fee)** | `250` | `450` | `250` | `0` | **`950`** |
| **Logistics Provider** | `125` | `225` | `125` | `0` | **`475`** |
| **Supplier (Immediate + Retainage + Warranty)** | `22_163` | `39_893` | `22_163 + 4_678 = 26_841` | `4_678` | **`93_575`** |
| **Total Accounted** | `22_538` (+`2_462` held) | `45_568` (+`4_432` held) | — | — | **`100_000`** |

---

## 6. Read-Only Cost & Payout Simulation

Integrators can preview exact deductions without submitting a transaction:
- **`preview_milestone_payout(env, shipment_id, milestone_index) -> PayoutPreview`:** Returns `{ gross_amount, fee_amount, logistics_fee_amount, penalty_amount, advance_deduction, early_bonus, net_supplier_payout }` evaluated at the current ledger.
- **`estimate_shipment_costs(env, buyer, token, total_amount, milestones, options) -> ShipmentCostEstimate`:** Returns `{ escrow_amount, platform_fee, total_dispute_bonds, total_buyer_cost, supplier_collateral, net_supplier_payout, fee_bps }` prior to calling `create_shipment`.
