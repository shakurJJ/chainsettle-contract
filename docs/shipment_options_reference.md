# ShipmentOptions Field Reference

This document provides a comprehensive reference for all fields in the `ShipmentOptions` struct used during shipment creation. Each field controls specific behavior of the escrow lifecycle, dispute handling, payment flows, and advanced features.

---

## Table of Contents

- [Core Escrow Fields](#core-escrow-fields)
- [Dispute & Resolution](#dispute--resolution)
- [Fee Configuration](#fee-configuration)
- [Milestone Behavior](#milestone-behavior)
- [Expiry & Deadlines](#expiry--deadlines)
- [Arbiter Configuration](#arbiter-configuration)
- [Metadata & Referrals](#metadata--referrals)
- [Advanced Features](#advanced-features)
  - [Quality Grading](#quality-grading)
  - [Quantity-Based Pro-Rata](#quantity-based-pro-rata)
  - [Retainage & Warranty](#retainage--warranty)
  - [Inspector Sign-Off](#inspector-sign-off)
  - [Proof Submission Controls](#proof-submission-controls)
  - [Multi-Supplier Consortium](#multi-supplier-consortium)
  - [Buyer Vault Funding](#buyer-vault-funding)
  - [Refund Recipient Override](#refund-recipient-override)
- [Extended Options (`ShipmentOptionsExt`)](#extended-options-shipmentoptionsext)

---

## Core Escrow Fields

### `milestone_mode: MilestoneMode`

**Type:** Enum (`Sequential` | `Parallel`)  
**Default:** `Parallel`

Controls whether milestones must be completed in order or can be completed independently.

- **`Sequential`**: Milestones must be confirmed in order (0 → 1 → 2...). A milestone cannot be confirmed until all previous milestones are confirmed.
- **`Parallel`**: Milestones can be confirmed in any order.

**Use case:** Sequential for staged deliveries, Parallel for independent work streams.

---

### `holdback_ledgers: u32`

**Type:** `u32` (ledger count)  
**Default:** `0`

Number of ledgers to hold payment after milestone confirmation before releasing funds to the supplier.

- **`0`**: Immediate release upon confirmation.
- **`> 0`**: Payment is held for the specified number of ledgers after confirmation.

**Use case:** Quality inspection periods, dispute window enforcement.

---

### `supplier_collateral: i128`

**Type:** `i128` (token amount)  
**Default:** `0`

Amount of collateral the supplier must lock when the shipment is created. Collateral is returned upon successful completion or slashed on supplier-caused cancellation.

**Use case:** Performance bonds, skin-in-the-game for high-value shipments.

---

### `expires_at_ledger: Option<u32>`

**Type:** `Option<u32>` (ledger sequence)  
**Default:** `None`

Optional expiry ledger. If set and the current ledger exceeds this value, the shipment is marked `Expired` and funds are refunded to the buyer.

**Use case:** Time-sensitive contracts, regulatory compliance.

---

## Dispute & Resolution

### `dispute_bond_amount: i128`

**Type:** `i128` (token amount)  
**Default:** `0`

Fixed bond amount locked from the disputing party when a dispute is opened. If the disputing party loses, the bond is slashed.

- **`0`**: No bond required (backward compatible).
- **`> 0`**: Dispute requires locking this amount.

**Use case:** Deter frivolous disputes.

---

### `dispute_bond_bps: u32`

**Type:** `u32` (basis points)  
**Default:** `0`

Percentage of the shipment's `total_amount` added to the per-dispute bond. Stacks with `dispute_bond_amount`.

- **`0`**: Disabled.
- **`> 0`**: Bond scales with shipment value.

**Example:** `dispute_bond_bps = 100` (1%) on a 1M shipment adds 10K to the bond.

**Note:** Subject to admin-configured `MaxDisputeBondBps` cap.

---

### `dispute_cooldown_ledgers: u32`

**Type:** `u32` (ledger count)  
**Default:** `0`

Minimum ledgers between successive dispute resolutions on the same shipment.

- **`0`**: No cooldown.
- **`> 0`**: Enforces cooldown period.

**Use case:** Prevent rapid-fire disputes, allow time for corrective action.

---

### `dispute_timeout_seconds: u64`

**Type:** `u64` (seconds)  
**Default:** `0`

Seconds after dispute opens before anyone can invoke auto-resolution via `resolve_dispute_timeout`.

- **`0`**: Disabled.
- **`> 0`**: Enables timeout-based resolution.

**Use case:** Handle arbiter inactivity.

---

### `default_resolution: Resolution`

**Type:** Enum (`Buyer` | `Supplier`)  
**Default:** `Resolution::Buyer`

Resolution applied by `resolve_dispute_timeout` when the timeout elapses.

**Use case:** Define fallback behavior for unresolved disputes.

---

## Fee Configuration

### `arbiter_fee_bps: u32`

**Type:** `u32` (basis points)  
**Default:** `0`

Percentage of the disputed payment sent to the arbiter upon dispute resolution.

- **`0`**: No arbiter fee.
- **`> 0`**: Arbiter earns fee from resolved disputes.

**Example:** `arbiter_fee_bps = 50` (0.5%) on a 100K milestone = 500 token fee.

---

### `logistics_fee_bps: u32`

**Type:** `u32` (basis points)  
**Default:** `0`

Percentage deducted from each milestone payment for the logistics provider.

- **`0`**: No logistics fee.
- **`> 0`**: Logistics provider earns a cut of each milestone.

**Use case:** Freight forwarders, 3PL providers.

---

### `buyer_cancel_fee_bps: u32`

**Type:** `u32` (basis points, max `1000`)  
**Default:** `0`

Penalty charged to the buyer on buyer-initiated cancellation.

- **`0`**: No penalty.
- **`> 0`**: Buyer forfeits this percentage on cancellation.

**Use case:** Compensate supplier for setup costs.

---

### `penalty_bps: u32`

**Type:** `u32` (basis points)  
**Default:** `0`

Percentage deducted from the buyer's refund when the supplier cancels the shipment.

**Use case:** Compensate buyer for disruption caused by supplier cancellation.

---

### `late_penalty_bps_per_ledger: u32`

**Type:** `u32` (basis points per ledger)  
**Default:** `0`

Penalty accrued per ledger of delay past a milestone's deadline.

- **`0`**: No late penalty.
- **`> 0`**: Supplier loses this percentage per ledger of delay.

**Use case:** Time-sensitive deliveries with liquidated damages.

---

## Milestone Behavior

### `milestone_splits: Vec<u32>`

**Type:** `Vec<u32>` (basis points per milestone)  
**Default:** Empty vector

Per-milestone payment splits in basis points. Sum must equal `10000`. Length must match number of milestones.

- **Empty vector**: Use per-milestone `payment_percent` field instead.
- **Non-empty**: Overrides `payment_percent` with precise splits.

**Use case:** Sub-1% precision splits (e.g., `[3333, 3333, 3334]` for three equal parts).

---

### `deadlines: Vec<u64>`

**Type:** `Vec<u64>` (Unix timestamps per milestone)  
**Default:** Empty vector

Per-milestone Unix timestamp deadlines.

- **Empty vector**: No timestamp deadlines.
- **Non-empty**: Length must match milestones. `0` = no deadline for that milestone.

**Use case:** Calendar-based deadlines (e.g., delivery by specific date).

---

### `auto_confirm_ledgers: u32`

**Type:** `u32` (ledger count)  
**Default:** `0`

Ledgers after proof submission before auto-confirmation occurs.

- **`0`**: Disabled (buyer must manually confirm).
- **`> 0`**: Milestone auto-confirms after this period.

**Use case:** Prevent buyer inaction from blocking payment.

---

### `confirmation_cooldown_ledgers: Option<u32>`

**Type:** `Option<u32>` (ledger count)  
**Default:** `None`

Override for the global milestone confirmation cooling-off period.

- **`None`**: Use global default.
- **`Some(n)`**: Use `n` ledgers as cooldown.

**Use case:** Per-shipment confirmation timing control.

---

### `response_deadline: u32`

**Type:** `u32` (ledger count)  
**Default:** `0`

Ledgers after proof submission before supplier can cancel the shipment if buyer doesn't respond.

- **`0`**: Supplier cancellation disabled.
- **`> 0`**: Supplier can cancel after deadline.

**Use case:** Protect supplier from indefinite buyer inaction.

---

## Expiry & Deadlines

### `grace_period_ledgers: u32`

**Type:** `u32` (ledger count)  
**Default:** `0`

Ledgers of grace period before late-delivery penalty accrual starts.

**Use case:** Allow reasonable buffer before penalties kick in.

---

### `review_window_ledgers: Option<u32>`

**Type:** `Option<u32>` (ledger count)  
**Default:** `None`

Per-shipment override for the proof review window before auto-confirmation.

- **`None`**: Use global default.
- **`Some(0)`**: Disable auto-confirm for this shipment.
- **`Some(n)`**: Use `n` ledgers as review window.

**Use case:** High-scrutiny shipments needing manual approval.

---

## Arbiter Configuration

### `arbiter_panel: Vec<Address>`

**Type:** `Vec<Address>`  
**Default:** Empty vector

Optional panel of arbiters for N-of-M dispute resolution.

- **Empty**: Single-arbiter mode (uses `arbiter` field from `create_shipment`).
- **Non-empty (len ≥ 3)**: Panel mode activated, `arbiter` field ignored for disputes.

**Use case:** Multi-signature dispute resolution for high-stakes shipments.

---

### `backup_arbiter: Option<Address>`

**Type:** `Option<Address>`  
**Default:** `None`

Optional backup arbiter for inactivity failover.

**Use case:** Arbiter redundancy for critical shipments.

---

## Metadata & Referrals

### `metadata_hash: Option<BytesN<32>>`

**Type:** `Option<BytesN<32>>` (32-byte hash)  
**Default:** `None`

Off-chain trade document hash (e.g., IPFS CID) attached at creation. Immutable after creation.

**Use case:** Link on-chain escrow to off-chain purchase orders, bills of lading, etc.

---

### `referrer: Option<Address>`

**Type:** `Option<Address>`  
**Default:** `None`

Optional referrer who earns a basis-point bonus of the protocol fee on shipment completion.

**Use case:** Affiliate marketing, partner rewards.

---

### `jurisdiction: Option<Symbol>`

**Type:** `Option<Symbol>`  
**Default:** `None`

Optional jurisdiction/regulatory category (e.g., `"US"`, `"EU_MIFID"`). Immutable after creation.

**Use case:** Off-chain compliance filtering, regulatory reporting.

---

## Advanced Features

### Quality Grading

#### `quality_grades: Vec<u32>`

**Type:** `Vec<u32>` (basis points per grade)  
**Default:** Empty vector

Payout basis points per quality grade index. Grade 0 must be `10000` (full payment). Grades must be non-increasing.

- **Empty**: Quality grading disabled.
- **Non-empty**: Buyer selects grade on confirmation.

**Example:** `[10000, 9000, 7500]` = 100%, 90%, 75% payout.

**Use case:** Inspect delivered goods and adjust payment based on quality.

---

### Quantity-Based Pro-Rata

#### `milestone_quantities: Vec<u32>`

**Type:** `Vec<u32>` (expected quantity per milestone)  
**Default:** Empty vector

Expected quantity per milestone. `0` = not quantity-based for that milestone.

- **Empty**: No quantity-based milestones.
- **Non-empty**: Length must match milestones. Buyer specifies actual delivered quantity on confirmation.

**Use case:** Partial deliveries where payment scales with delivered units.

---

### Retainage & Warranty

#### `retainage_bps: u32`

**Type:** `u32` (basis points, max `constants::MAX_RETAINAGE_BPS`)  
**Default:** `0`

Percentage of each net milestone payment withheld until shipment completes.

- **`0`**: Disabled.
- **`> 0`**: Retained amount released on completion.

**Use case:** Construction contracts with retainage holdback.

---

#### `warranty_bps: u32`

**Type:** `u32` (basis points, max `constants::MAX_WARRANTY_BPS`)  
**Default:** `0`

Percentage of each net milestone payment kept in escrow as warranty holdback after completion.

**Note:** Must specify `warranty_ledgers > 0` when `warranty_bps > 0`.

**Use case:** Product warranties, defect liability periods.

---

#### `warranty_ledgers: u32`

**Type:** `u32` (ledger count)  
**Default:** `0`

Ledgers after completion during which the buyer may file a warranty claim.

**Use case:** Define warranty claim window.

---

### Inspector Sign-Off

#### `inspector: Option<Address>`

**Type:** `Option<Address>`  
**Default:** `None`

Optional third-party inspector whose sign-off gates confirmation of milestones listed in `inspected_milestones`.

**Use case:** Quality control, regulatory compliance inspections.

---

#### `inspected_milestones: Vec<u32>`

**Type:** `Vec<u32>` (milestone indices)  
**Default:** Empty vector

Milestone indices requiring inspector sign-off before confirmation.

- **Empty**: No milestones require inspection.
- **Ignored when `inspector` is `None`.**

**Use case:** Critical milestones requiring third-party verification.

---

### Proof Submission Controls

#### `proof_submitters: Vec<Address>`

**Type:** `Vec<Address>` (one per milestone)  
**Default:** Empty vector

Optional per-milestone designated proof submitter.

- **Empty**: Supplier-only submission (default).
- **Non-empty**: Length must match milestones. Each milestone requires proof from specified address.

**Use case:** Delegated proof submission (e.g., subcontractors, IoT devices).

---

#### `require_dual_attestation: bool`

**Type:** `bool`  
**Default:** `false`

When `true`, both supplier and logistics must attest a milestone proof before it counts as submitted.

**Use case:** Dual-signature proof of delivery.

---

### Multi-Supplier Consortium

#### `milestone_suppliers: Vec<Address>`

**Type:** `Vec<Address>` (one per milestone)  
**Default:** Empty vector

Per-milestone supplier addresses.

- **Empty**: Single-supplier (default).
- **Non-empty**: Length must match milestones. Each milestone pays its own supplier and only that supplier may submit proof for it.

**Use case:** Multi-party supply chains with different suppliers per phase.

---

### Buyer Vault Funding

#### `fund_from_vault: bool`

**Type:** `bool`  
**Default:** `false`

When `true`, `create_shipment` debits the primary buyer's vault balance instead of transferring tokens. Refunds return to the vault.

**Use case:** Recurring payments, enterprise escrow accounts.

---

### Refund Recipient Override

#### `refund_recipient: Option<Address>`

**Type:** `Option<Address>`  
**Default:** `None`

Alternative address to receive refunds in place of the buyer.

- **`None`**: Refunds go to the buyer (default).
- **`Some(addr)`**: Refunds go to `addr`.

**Use case:** Corporate treasury separation, third-party payment processors.

---

## Early Completion Bonus

### `early_bonus_pool: i128`

**Type:** `i128` (token amount)  
**Default:** `0`

Bonus pool funded by the buyer, distributed to the supplier for early completion.

**Use case:** Incentivize ahead-of-schedule delivery.

---

## Extended Options (`ShipmentOptionsExt`)

`ShipmentOptions` is at the 40-field limit of a Soroban `#[contracttype]`
struct, so newer creation options live in `ShipmentOptionsExt` and are passed
to `create_shipment_ext`, which takes the same arguments as `create_shipment`
plus `ext`. `ShipmentOptionsExt::default()` turns every option off, and the
call then behaves exactly like `create_shipment`.

### `perishable: bool` (#744)

**Default:** `false`

Marks the shipment as perishable. When the admin has set windows with
`set_expedited_config`, its disputes run on the expedited track:

- **Review window:** the proof review (auto-confirm) window is the shorter of
  the standard window and `review_window_ledgers`. An explicit
  `review_window_ledgers: Some(0)` opt-out still disables auto-confirmation.
- **Evidence window:** `submit_dispute_evidence` is rejected with
  `"evidence window closed"` once `evidence_window_ledgers` ledgers have passed
  since the dispute opened.
- **Resolution window:** `resolve_dispute_timeout` may run after the shorter of
  the shipment's `dispute_timeout_seconds` and `resolution_timeout_seconds`.
  If the shipment set no timeout, the expedited timeout applies with
  `fallback_resolution` as the outcome.

Pool-assigned arbiters (the pool sentinel, appeals and dispute panels) are
drawn from arbiters who called `set_arbiter_expedited(arbiter, true)` first.
Seats no expedited arbiter can fill fall back to the rest of the pool, so a
dispute never goes unassigned. Without an expedited config, perishable
shipments use the standard windows and only the routing applies.
Non-perishable shipments are unaffected.

### `po_reference: Option<String>` (#745)

**Default:** `None`

The buyer's purchase-order number, at most `MAX_PO_REFERENCE_LEN` (64) bytes
and not empty. It must be unique for the primary buyer: a second shipment with
the same PO for the same buyer is rejected with
`"duplicate po_reference for buyer"`. Different buyers may reuse a PO.

- `get_shipment_by_po(buyer, po_reference)` returns the shipment, or `None`
  for an unknown PO.
- `get_shipment_po_reference(shipment_id)` returns the PO attached to a
  shipment.

A PO stays bound to its shipment after cancellation or completion.

### `refund_to_vault: bool` (#746)

**Default:** `false`

Credits refunds to the refund recipient's vault balance (see
`fund_from_vault`) instead of transferring tokens. Buyers can opt in for all
their shipments with `set_buyer_profile(buyer, BuyerProfile { refund_to_vault:
true })`. Vault-funded shipments are routed the same way.

It applies to every buyer refund: buyer and supplier cancellation, deadline
refunds, blacklisted-supplier cancellation, holdback refunds on cancellation,
and the buyer side of dispute outcomes (arbiter, split, panel, mediation and
timeout resolutions, returned dispute bonds, grade refunds and warranty
claims). Late-penalty and early-bonus returns on normal completion are not
refunds and are still transferred. The `shipment_cancelled` and
`dispute_resolved` events carry a `to_vault` flag. The tokens stay in the
contract and are counted in `TotalVaulted`, so they remain withdrawable with
`vault_withdraw`.

### `fee_bearer: FeeBearer` (#750)

**Default:** `FeeBearer::Supplier`

Who pays the protocol fee when the shipment uses a fee token. When the admin
has called `set_fee_token(token, rate_source)`, every shipment created
afterwards locks those terms in (`get_shipment_fee_token`). At each release
the escrow-token fee is converted, either with the `set_conversion_rate`
route (`FeeRateSource::ConversionRate`) or with oracle prices
(`FeeRateSource::Oracle`), and pulled from the bearer with `transfer_from`.
The supplier then gets the gross escrow payout.

- The bearer must `approve` the contract for the fee token. A missing
  allowance or balance fails with `"insufficient fee token allowance"` /
  `"insufficient fee token balance"` before any payout.
- With `ConversionRate`, creation fails with
  `"fee token conversion rate not configured"` if no route exists from the
  escrow token.
- Shipments created before `set_fee_token`, or after `clear_fee_token`, keep
  deducting fees from the escrow token. The fee goes to the `FeeConfig`
  treasury. Fee-recipient splits and the clean-completion rebate apply only
  to escrow-token fees.

---

## Summary

The `ShipmentOptions` struct provides granular control over escrow behavior, dispute handling, payment flows, and advanced features. Understanding each field allows you to tailor ChainSettle shipments to your specific trade agreements, regulatory requirements, and risk management strategies.

For implementation examples, refer to the test suite in `contracts/chainsetttle/src/test_*.rs` files.
