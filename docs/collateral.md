# Supplier Collateral and Dispute Bonds Guide

ChainSettle uses two distinct, complementary security deposits to align incentives between buyers and suppliers:

1. **Supplier Collateral (`ShipmentOptions.supplier_collateral`):** Posted by the **supplier** when a shipment is created (`DataKey::SupplierCollateral(shipment_id)`). Protects the buyer against supplier non-performance or missed delivery deadlines.
2. **Buyer Dispute Bonds (`ShipmentOptions.dispute_bond_amount` & `ShipmentOptions.dispute_bond_bps`):** Posted by the **primary buyer** (`buyers[0]`) when a shipment is created (`shipment.dispute_bond_amount` per milestone). Protects the supplier against frivolous or bad-faith buyer disputes.

All entry points referenced below live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs).

---

## Side-by-Side Comparison

| Property | Supplier Collateral | Buyer Dispute Bond |
| --- | --- | --- |
| **Depositor** | `shipment.supplier` (requires `supplier.require_auth()` at creation when `> 0`) | `shipment.buyers[0]` (pulled alongside escrow at `create_shipment`) |
| **Configured Via** | `ShipmentOptions.supplier_collateral` (or supplier's `SupplierTermsProfile.default_collateral_bps`) | `ShipmentOptions.dispute_bond_amount` (flat) + `ShipmentOptions.dispute_bond_bps` (% of `total_amount`) |
| **Reputation Discount** | Discounted automatically by `SupplierTier` (`Silver` 10%, `Gold` 25%, `Platinum` 50% by default) | None |
| **Returned to Depositor** | On shipment completion, incremental milestone confirmation (`enable_incremental_collateral`), or mutual cancellation (`approve_mutual_cancel`) | On shipment completion/cancellation for uncontested milestones, or when a disputed milestone resolves in the supplier's favor (`approve = true`) |
| **Forfeited to Counterparty** | Transferred to `buyers[0]` (or `RefundRecipient`) on `cancel_shipment` or `claim_deadline_refund` | Transferred to `shipment.supplier` when a dispute is rejected (`approve = false`), or split pro-rata on `resolve_dispute_split` |

---

## 1. Supplier Collateral

### 1.1 Posting Collateral at Shipment Creation

When `ShipmentOptions.supplier_collateral > 0` (or when the supplier's registered `SupplierTermsProfile` sets `default_collateral_bps > 0`):
1. **Default from Supplier Terms Profile (`#667`):**
   If a supplier has configured `set_supplier_terms_profile(env, supplier, profile: SupplierTermsProfile)` with `default_collateral_bps > 0`:
   - When `options.supplier_collateral == 0` (or when `profile.enforce_terms == true` and `options.supplier_collateral < min_collateral`), base collateral defaults to:
     $$\text{base\_collateral} = \left\lfloor \frac{\text{total\_amount} \times \text{default\_collateral\_bps}}{10{,}000} \right\rfloor$$
2. **Reputation Tier Discount (`#397`):**
   The contract computes `effective_collateral = apply_tier_collateral_discount(env, supplier, base_collateral)` (see Section 1.2).
3. **Token Transfer:**
   If `effective_collateral > 0`, `create_shipment` requires `supplier.require_auth()`, transfers `effective_collateral` from `supplier` to the contract, stores the balance at `DataKey::SupplierCollateral(shipment_id)`, and sets `shipment.collateral_Deposited = true`.

### 1.2 Supplier Reputation Tiers & Collateral Discounts (`#397`)

Suppliers with strong on-chain track records automatically qualify for reduced collateral requirements based on their `ReputationRecord` (`completed_shipments`, `disputed_shipments`):

```rust
pub enum SupplierTier {
    Bronze,
    Silver,
    Gold,
    Platinum,
}
```

A supplier's dispute rate in basis points is computed as:
$$\text{dispute\_rate\_bps} = \left\lfloor \frac{\text{disputed\_shipments} \times 10{,}000}{\text{completed\_shipments} + \text{disputed\_shipments}} \right\rfloor$$

#### Default Tier Thresholds & Discounts (`SupplierTierConfig`)

| Tier | Min Completed Shipments | Max Dispute Rate (`bps`) | Default Collateral Discount (`bps`) | Effective Collateral Posted |
| --- | --- | --- | --- | --- |
| **`Platinum`** | `>= 25` | `<= 200` (`2.00%`) | `5_000` (`50%`) | `50%` of `base_collateral` |
| **`Gold`** | `>= 10` | `<= 500` (`5.00%`) | `2_500` (`25%`) | `75%` of `base_collateral` |
| **`Silver`** | `>= 3` | `<= 1_000` (`10.00%`) | `1_000` (`10%`) | `90%` of `base_collateral` |
| **`Bronze`** | `< 3` (or higher dispute rate) | Any | `0` (`0%`) | `100%` of `base_collateral` |

$$\text{effective\_collateral} = \text{base\_collateral} - \left\lfloor \frac{\text{base\_collateral} \times \text{discount\_bps}}{10{,}000} \right\rfloor$$

- **Admin Configuration:** `set_supplier_tier_config(env, admin, config: SupplierTierConfig)` (`get_supplier_tier_config(env) -> SupplierTierConfig`). Every discount field is validated to be `<= 10_000` bps.
- **Querying a Supplier's Tier:** `get_supplier_tier(env, supplier) -> SupplierTier`.

### 1.3 Alternate Collateral Token (`#557`)

By default, supplier collateral is denominated in `shipment.token`. For shipments that want to denominate collateral in a separate whitelisted token (e.g., stablecoin escrow with governance-token or wrapped-asset collateral):
- **`set_collateral_token(env, buyer, shipment_id, collateral_token)`:**
  - Must be called by a buyer on the shipment while the shipment is `Created` or `InProgress`.
  - Requires `collateral_token` to be in the contract's allowed token list (`DataKey::AllowedTokens` or `DataKey::TokenAddress`).
  - Stores `collateral_token` at `DataKeyExt4::CollateralToken(shipment_id)` and emits `collateral_token_set`.
- **`get_collateral_token(env, shipment_id) -> Address`:**
  - Returns `DataKeyExt4::CollateralToken(shipment_id)` if set, or falls back to `shipment.token`.

### 1.4 Incremental Per-Milestone Collateral Release (`#475`)

By default, supplier collateral stays locked until the entire shipment completes. Counterparties can opt an active shipment into **incremental per-milestone collateral release** so collateral unlocks proportionally as each milestone is confirmed:

1. **Opt In (`enable_incremental_collateral(env, caller, shipment_id)`):**
   - Callable by any buyer on the shipment or by `shipment.supplier` while the shipment is `Created` or `InProgress`.
   - Sets `DataKeyExt3::IncrementalCollateral(shipment_id) = true` and snapshots the initial deposit at `DataKeyExt3::IncrementalCollateralBase(shipment_id) = current_collateral`.
   - Emits `incremental_collateral_enabled`.
2. **Proportional Release on Each Milestone (`release_milestone_collateral_share`):**
   - Whenever a milestone is confirmed (`confirm_milestone`, `batch_confirm_milestones`, `release_held_payment`, or `claim_auto_confirmation`), the contract computes the milestone's share of `IncrementalCollateralBase`:
     - Using `milestone_splits`: $\text{share} = \lfloor \text{base} \times \text{split\_bps} / 10{,}000 \rfloor$
     - Using `payment_percent`: $\text{share} = \lfloor \text{base} \times \text{payment\_percent} / 100 \rfloor$
   - Caps `release = min(share, remaining_collateral)`, transfers `release` back to `shipment.supplier`, updates `DataKey::SupplierCollateral(shipment_id)`, and emits `collateral_share_released`.

### 1.5 Full Collateral Return vs. Forfeiture

| Scenario | Entry Point | Collateral Outcome |
| --- | --- | --- |
| **All milestones confirmed** | `confirm_milestone` / `release_held_payment` / `claim_auto_confirmation` | Any remaining `SupplierCollateral(shipment_id)` is returned to `shipment.supplier` (`collateral_returned` event) and `collateral_returned` flag is set to `true`. |
| **Post-completion manual claim** | `claim_collateral(env, supplier, shipment_id)` | If a shipment reached `Completed` (e.g., via dispute resolution) with `SupplierCollateral(shipment_id) > 0` still held, the supplier calls `claim_collateral` to withdraw the full remaining collateral (`collateral_claimed`). |
| **Mutual cancellation** | `propose_mutual_cancel` → `approve_mutual_cancel` | Full remaining `SupplierCollateral(shipment_id)` is returned to `shipment.supplier` without penalty. |
| **Buyer cancellation (`Created` status)** | `cancel_shipment` / `batch_cancel_shipments` | Full remaining `SupplierCollateral(shipment_id)` is **forfeited** and transferred to `buyers[0]` (`collateral_slashed` event). |
| **Milestone deadline expiry** | `claim_deadline_refund(env, shipment_id, milestone_index)` | Full remaining `SupplierCollateral(shipment_id)` is **forfeited** and transferred to `buyers[0]` (or `RefundRecipient` if set via `set_refund_recipient`), emitting `collateral_forfeited`. |

---

## 2. Buyer Dispute Bonds

### 2.1 Configuring and Escrowing Dispute Bonds

A shipment can require a per-milestone dispute bond using either a flat token amount, a basis-point percentage of `total_amount`, or both:

- **`ShipmentOptions.dispute_bond_amount: i128`** — flat token amount per milestone (`>= 0`).
- **`ShipmentOptions.dispute_bond_bps: u32`** — basis points of `total_amount` per milestone (`0..=max_dispute_bond_bps`).
  - Capped by `DataKeyExt3::MaxDisputeBondBps` (default `DEFAULT_MAX_DISPUTE_BOND_BPS = 2_000` = 20%; configurable by admin via `set_max_dispute_bond_bps(env, admin, max_bps)` and queried via `get_max_dispute_bond_bps(env)`).

At `create_shipment`, the contract computes:
$$\text{scaled\_bond} = \left\lfloor \frac{\text{total\_amount} \times \text{dispute\_bond\_bps}}{10{,}000} \right\rfloor$$
$$\text{per\_dispute\_bond} = \text{dispute\_bond\_amount} + \text{scaled\_bond}$$
$$\text{bond\_total} = \text{per\_dispute\_bond} \times \text{milestones.len()}$$

`buyers[0]` transfers `total_amount + bond_total + early_bonus_pool` into the contract at creation, and `shipment.dispute_bond_amount` is set to `per_dispute_bond`.

### 2.2 Dispute Bond Settlement Matrix

When a disputed milestone is resolved, its `per_dispute_bond` is settled according to the ruling:

| Resolution Path | Ruling | Dispute Bond Settlement |
| --- | --- | --- |
| **Standard / Panel Resolution (`resolve_dispute`, `cast_dispute_vote`)** | **Supplier wins (`approve = true`)** | Returned to `buyers[0]` (`dispute_bond_returned`). *(If `set_loser_pays_arbiter_fee(true)` is enabled, the buyer's returned bond first covers up to `arbiter_fee` so the supplier receives their full milestone payout.)* |
| **Standard / Panel Resolution (`resolve_dispute`, `cast_dispute_vote`)** | **Buyer wins (`approve = false`)** | **Forfeited** to `shipment.supplier` (`dispute_bond_forfeited`). *(In a partial dispute with `loser_pays_arbiter_fee` enabled, the forfeited bond covers the arbiter fee first before the remainder goes to the supplier.)* |
| **Percentage Split Resolution (`resolve_dispute_split`)** | **Split (`supplier_bps` in `0..=10_000`)** | Split proportionally: $\text{bond\_to\_buyer} = \lfloor \text{bond} \times (10{,}000 - \text{supplier\_bps}) / 10{,}000 \rfloor$ is returned to `buyers[0]`, and $\text{bond} - \text{bond\_to\_buyer}$ is transferred to `shipment.supplier`. |
| **Non-Binding Mediation (`accept_mediation`)** | `Resolution::Supplier` or `Resolution::Buyer` | Mirrors standard resolution: returned to `buyers[0]` on `Resolution::Supplier`, or transferred to `shipment.supplier` on `Resolution::Buyer` (with zero arbiter fee). |
| **Oracle Condition Breach (`report_condition_breach`)** | Auto-opened dispute (`#546`) | Does **not** require buyer auth or draw an additional bond beyond what was escrowed at shipment creation. |
