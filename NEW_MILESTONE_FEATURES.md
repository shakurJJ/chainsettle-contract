# New Milestone Features: Insurance Holdback & Oracle Price Conditions

## Overview

This document describes two new proposed features for the ChainSettle contract:

1. **Per-Milestone Insurance Holdback** - A configurable percentage of each milestone payment that is held back as insurance
2. **Oracle Price Condition** - Gate milestone confirmation on an oracle price meeting a minimum threshold

## Feature 1: Per-Milestone Insurance Holdback

### Problem

Currently, there's no mechanism for a buyer to require a small insurance-style holdback per milestone that's:

- Refunded to the supplier if delivered on time and without disputes
- Forfeited to the buyer if the milestone is disputed and the supplier loses

### Proposed Behavior

#### Configuration

- **Per-milestone `insurance_bps`** - Configurable basis points (e.g., 200 = 2%) set at shipment creation
- Held back from the payout automatically when milestone is confirmed

#### On Successful Delivery (On-Time Confirmation, No Dispute)

- Insurance holdback is released to the supplier alongside (or after) the normal payout
- Can be released via `release_insurance_holdback()` function
- Acts as a "good faith" deposit that rewards timely, dispute-free delivery

#### On Dispute Resolution Against Supplier

- If a dispute is raised and resolved against the supplier (arbiter rejects the proof)
- The insurance holdback routes to the buyer instead of being refunded to supplier
- Called via `forfeit_insurance_to_buyer()` function

#### Query Visibility

- Held amounts are visible via queries:
  - `get_milestone_insurance_holdback(shipment_id, milestone_index)` - per milestone
  - `get_total_insurance_holdback(shipment_id)` - total across all milestones
  - Integrated with existing `get_shipment_escrow_balance()` query

### Example Calculation

```
Shipment: 1,000,000 tokens
Milestone 0: 25% = 250,000 tokens
Insurance BPS: 200 (2%)

On Confirmation:
- Gross Payment: 250,000
- Insurance Holdback: 5,000 (2% of 250,000)
- Net Immediate Payout to Supplier: 245,000

On-Time Delivery (No Dispute):
- Insurance Released to Supplier: +5,000
- Supplier Total: 250,000

Dispute Lost by Supplier:
- Insurance Forfeited to Buyer: +5,000
- Supplier Gets: 245,000 only
```

## Feature 2: Oracle Price Condition per Milestone

### Problem

Invoices denominated in one currency but settled in another (e.g., a USD-denominated invoice settled in XLM) currently have no way to gate milestone release on an oracle price condition being met.

### Proposed Behavior

#### Configuration

- **Per-milestone optional `price_condition`**: `(oracle_address, min_price)` set at creation
- `confirm_milestone` checks the configured oracle before allowing payout when a price condition is present
- Reverts if price condition is unmet (oracle price < min_price)

#### Milestone Confirmation Flow

1. Buyer calls `confirm_milestone()`
2. **If price condition is configured:**
   - Contract queries the oracle at `oracle_address`
   - Gets current price via `get_price(asset)`
   - Compares: `current_price >= min_price`
   - **If condition met:** Proceed with normal confirmation
   - **If condition NOT met:** Revert with "price condition not met"
3. **If no price condition:** Behaves exactly as today (backward compatible)

#### Oracle Approval

- Oracle address must be an **admin-approved address**
- Reuses existing admin governance for oracle whitelisting
- Uses the mock-oracle test pattern as the reference interface

#### Oracle Interface

```rust
pub trait PriceOracle {
    /// Get the current price for an asset
    /// Returns price in 7 decimals (1.0 = 10_000_000)
    fn get_price(env: Env, asset: Address) -> i128;
}
```

### Example Use Case

**Scenario:** USD-denominated invoice settled in XLM

```
Invoice: $10,000 USD
Payment Token: XLM
Current XLM Price: $0.12 USD per XLM
Required XLM: 83,333 XLM (10,000 / 0.12)

Buyer wants to ensure they only pay if XLM >= $0.10 USD

Configuration:
- oracle_address: <trusted price feed>
- min_price: 10_000_000 (= $0.10 in 7 decimals)

Milestone Confirmation:
- If XLM price drops to $0.08: Transaction REVERTS
- If XLM price is $0.12: Transaction SUCCEEDS
- If no price condition: Transaction SUCCEEDS regardless
```

## Combined Feature Usage

Both features can be used together:

```rust
// Create shipment with 1.5% insurance
let insurance_bps = 150;

// Set oracle price condition for milestone 0
client.set_milestone_price_condition(
    &buyer,
    &shipment_id,
    &0,
    &oracle_address,
    &min_price
);

// Confirmation flow:
// 1. Check oracle price >= min_price
// 2. If price OK, confirm milestone
// 3. Deduct insurance_bps from payout
// 4. Hold insurance until release/forfeit
```

## Test Coverage

The new test file `test_milestone_insurance_and_oracle.rs` includes 10 comprehensive tests:

### Insurance Holdback Tests (1-3, 9-10)

1. ✅ Basic insurance holdback released on-time confirmation
2. ✅ Insurance holdback forfeited on dispute loss
3. ✅ Insurance holdback across multiple milestones
4. ✅ Insurance query visibility alongside escrow balance
5. ✅ Edge case: Zero insurance BPS (no holdback)

### Oracle Price Condition Tests (4-7)

6. ✅ Oracle price condition blocks confirmation when price too low
7. ✅ Oracle price condition allows confirmation when price meets threshold
8. ✅ No price condition behaves normally (backward compatible)
9. ✅ Unapproved oracle rejected

### Combined Feature Test (8)

10. ✅ Combined insurance holdback + oracle price condition

## API Changes Required

### Storage Extensions

```rust
// Add to Milestone struct
pub struct Milestone {
    // ... existing fields ...

    /// Insurance holdback amount for this milestone (0 = none)
    pub insurance_holdback_amount: i128,

    /// Optional oracle price condition: (oracle_address, min_price)
    pub price_condition: Option<(Address, i128)>,
}

// Add to ShipmentOptions
pub struct ShipmentOptions {
    // ... existing fields ...

    /// Basis points of each milestone payment held as insurance (0 = disabled)
    pub insurance_bps: u32,
}
```

### New Contract Functions

```rust
// Insurance management
pub fn get_milestone_insurance_holdback(
    env: Env,
    shipment_id: String,
    milestone_index: u32
) -> i128;

pub fn get_total_insurance_holdback(
    env: Env,
    shipment_id: String
) -> i128;

pub fn release_insurance_holdback(
    env: Env,
    caller: Address,
    shipment_id: String,
    milestone_index: u32
);

pub fn forfeit_insurance_to_buyer(
    env: Env,
    shipment_id: String,
    milestone_index: u32
);

// Oracle price condition management
pub fn set_milestone_price_condition(
    env: Env,
    caller: Address,
    shipment_id: String,
    milestone_index: u32,
    oracle_address: Address,
    min_price: i128
);

pub fn approve_oracle(
    env: Env,
    admin: Address,
    oracle_address: Address
);
```

## Backward Compatibility

Both features are **fully backward compatible**:

1. **Insurance Holdback**:
   - Default `insurance_bps = 0` means no holdback (current behavior)
   - Existing shipments unaffected

2. **Oracle Price Condition**:
   - Default `price_condition = None` means no oracle check (current behavior)
   - Milestones without a price condition work exactly as before

## Security Considerations

### Insurance Holdback

- ✅ Insurance amounts are always < milestone payment (capped by BPS)
- ✅ Held in contract escrow, not externally
- ✅ Only released via explicit buyer action or dispute resolution
- ⚠️ Risk: Buyer could delay releasing insurance indefinitely
  - **Mitigation**: Add timeout parameter for auto-release after N ledgers

### Oracle Price Condition

- ✅ Oracle must be admin-approved (prevents malicious oracles)
- ✅ Price check is atomic with confirmation (no front-running)
- ⚠️ Risk: Oracle outage/stale data could block legitimate confirmations
  - **Mitigation**: Fallback to manual override by admin
- ⚠️ Risk: Oracle manipulation
  - **Mitigation**: Use trusted, decentralized oracle networks (Chainlink, Band, etc.)

## Implementation Priority

### Phase 1: Insurance Holdback

- Lower complexity
- No external dependencies
- High value for trade finance use cases

### Phase 2: Oracle Price Conditions

- Requires oracle integration
- Admin approval system for oracle whitelist
- Higher complexity but critical for cross-currency settlements

## Related Issues/Features

- Relates to existing `holdback_ledgers` feature (time-based hold)
- Complements `dispute_bond_amount` (buyer's skin in the game)
- Works with `late_penalty_bps_per_ledger` (delivery incentives)
- Oracle pattern similar to existing `test_oracle.rs` (proof verification)

## Next Steps

1. ✅ Write comprehensive test suite (DONE - 10 tests)
2. 🔨 Implement storage changes for `Milestone` and `ShipmentOptions`
3. 🔨 Implement insurance holdback logic in `confirm_milestone`
4. 🔨 Implement oracle price condition check in `confirm_milestone`
5. 🔨 Add query functions for insurance visibility
6. 🔨 Add admin functions for oracle approval
7. 🧪 Run full test suite and verify integration
8. 📚 Update contract documentation and API reference

---

**Status:** Specification Complete ✅ | Tests Written ✅ | Implementation Pending 🔨
