ChainSettle — Contract Repo
![CI](https://github.com/shakurJJ/chainsettle-contract/actions/workflows/ci.yml/badge.svg)
![codecov](https://codecov.io/gh/shakurJJ/chainsettle-contract/branch/main/graph/badge.svg)

> **Milestone-based supply chain escrow on Stellar Soroban**
> ChainSettle is a Soroban smart contract that locks buyer payment in escrow and automatically releases funds to the supplier as each delivery milestone is confirmed on-chain. No middlemen, no delayed wire transfers, no trust required.
> This is Repo 1 of 3 in the ChainSettle project:
> Repo Description
> `chainsetttle-contract` ← you are here Soroban smart contract (Rust)
> `chainsetttle-frontend` React + Freighter wallet UI

## `chainsetttle-backend` Node.js API, notifications, off-chain metadata

Table of Contents
How It Works
Architecture
Data Structures
Contract Functions
Allowed token list
Invoice hash
rebalance_milestones
Milestone Payee Splits
Partial Disputes & Escalation Checks

Milestone Completion Percentage

Dispute Evidence Submission

Advance Payment Lifecycle
Milestone Amendment History Tracking
Supplier Payout Batching
Admin Succession & Emergency Recovery
Governance Timelock
Multisig Admin Governance
Contract Upgrades & Migration
Events
Error Codes
Project Structure
Prerequisites
Setup & Installation
Running Tests
Building
Deploying to Testnet
Deploying to Mainnet
Security Considerations
See detailed security model: docs/SECURITY.md
Roadmap

---

How It Works

```
Buyer creates shipment → USDC locked in contract escrow
         ↓
Supplier dispatches goods → submits IPFS proof hash (Milestone 1)
         ↓
Buyer confirms → 25% of USDC released to supplier automatically
         ↓
Logistics confirms transit → submits proof (Milestone 2)
         ↓
Buyer confirms → 50% released
         ↓
Goods delivered → supplier submits proof (Milestone 3)
         ↓
Buyer confirms → final 25% released → Shipment Completed ✓

If buyer disputes any proof → milestone frozen → Arbiter resolves
```

The contract is deployed once. Multiple independent shipments can be
created by different buyers using the same contract.

---

Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                  ChainSettle Contract (Soroban)               │
│                                                              │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────────┐ │
│  │  Shipment    │   │  Milestone   │   │   USDC Escrow    │ │
│  │  Registry    │   │  State       │   │   (SAC Transfer) │ │
│  │  (Persistent │   │  Machine     │   │                  │ │
│  │   Storage)   │   │              │   │                  │ │
│  └──────────────┘   └──────────────┘   └──────────────────┘ │
└──────────────────────────────────────────────────────────────┘
         ↑                    ↑                    ↑
    Buyer / Supplier     Buyer confirms      Token SAC contract
    call contract fns    or disputes         (USDC on Stellar)
```

Roles
Role Address Permissions
Buyer Locks USDC, confirms milestones, raises disputes, cancels shipment Most actions
Supplier Submits proof for dispatch and delivery milestones `submit_proof` only
Logistics Submits proof for in-transit milestones `submit_proof` only
Arbiter Resolves disputes — approves or rejects supplier proof `resolve_dispute` only
Admin Contract deployer, set at `init` Future: upgrade, pause

---

Data Structures
`Milestone`

```rust
pub struct Milestone {
    pub name: String,            // e.g. "Goods Dispatched"
    pub payment_percent: u32,    // 0-100, all milestones must sum to 100
    pub proof_hash: String,      // IPFS CID set by supplier/logistics
    pub status: MilestoneStatus, // Pending | ProofSubmitted | Confirmed | Disputed | Resolved
}
```

`Shipment`

```rust
pub struct Shipment {
    pub id: String,              // unique buyer-defined ID e.g. "SHIP-2026-001"
    pub buyer: Address,
    pub supplier: Address,
    pub logistics: Address,
    pub arbiter: Address,
    pub token: Address,          // Stellar Asset Contract for USDC
    pub total_amount: i128,      // locked in escrow (smallest unit)
    pub released_amount: i128,   // how much has been paid out so far
    pub milestones: Vec<Milestone>,
    pub status: ShipmentStatus,  // Active | Completed | Cancelled
    pub created_at: u32,         // ledger sequence number
    pub dispute_timeout_seconds: u64,   // auto-resolution deadline in seconds (0 = disabled)
    pub default_resolution: Resolution, // Supplier or Buyer outcome on timeout
}
```

`ReputationScore`
`get_reputation(supplier)` returns cumulative supplier outcome counters. It is
not a single weighted score; applications can use these counters to calculate
their own supplier rating:

```rust
pub struct ReputationScore {
    pub completed: u32,  // shipments completed successfully
    pub disputed: u32,   // disputes raised against the supplier
    pub cancelled: u32,  // shipments cancelled or expired
}
```

The counters are updated automatically:
`completed` increases once when all milestones in a shipment reach a
completed state, including successful dispute resolution.
`disputed` increases when a buyer raises a full or partial dispute. Resolving
the dispute does not remove the historical count.
`cancelled` increases when a shipment is cancelled by the buyer or supplier,
or when the buyer claims a deadline refund.
All counters start at `0` for a supplier with no reputation record and are
monotonically increasing (with saturating `u32` arithmetic).
Interpreting supplier reputation
`get_reputation` intentionally exposes raw on-chain facts instead of imposing
one platform-wide rating formula. A frontend, marketplace, or backend can
choose how much weight to give each outcome, while different applications can
use the same contract data for different purposes.
Important details for scoring integrations:
`completed` is incremented once per shipment, not once per milestone. A
shipment completed after an approved dispute still counts as completed.
`disputed` is incremented once whenever a dispute is opened against one of
the supplier's milestones. A shipment with disputes on several milestones
can therefore contribute more than one disputed count.
`cancelled` is incremented once when the shipment becomes cancelled or
expired. This includes buyer cancellation, eligible supplier cancellation,
and a successful deadline refund claim.
A dispute that is later rejected or automatically resolved does not erase
the historical `disputed` counter. Applications should treat it as an
activity or risk signal rather than proof that the supplier lost the
dispute.
The counters do not include payment volume, delivery speed, milestone
size, dispute outcome, or the identity of the party that raised the dispute.
Fetch shipment records separately when those dimensions are required.
For example, an application may calculate a simple historical completion rate
from the returned counters:

```
outcomes = completed + disputed + cancelled
completion_rate = completed / outcomes       // when outcomes > 0
```

This is only an application-level metric, not a value returned or enforced by
the contract. Because `disputed` counts milestone dispute events while the
other fields count shipment outcomes, the ratio should be labelled clearly
and should not be presented as an official ChainSettle score. A safer UI can
show the three counters separately and display `Not enough history` until a
minimum number of completed shipments has been reached.
Example read-only response:

```json
{
  "completed": 18,
  "disputed": 2,
  "cancelled": 1
}
```

The example represents 18 completed shipments, two disputed milestones, and
one cancelled or expired shipment. It does not mean that 18 of 21 shipments
were completed, because the disputed value is not a shipment-level count.
The getter requires no supplier signature and does not create a reputation
record. A supplier with no stored record returns the default value with all
fields set to zero. Reputation storage uses a persistent entry with a
renewable Soroban TTL, so indexers should periodically refresh their cached
value and should not assume that an absent record proves the supplier has
never used the contract.
`MilestoneStatus` state machine

```
Pending
  └─ submit_proof()  ──→ ProofSubmitted
       ├─ confirm_milestone() ──→ Confirmed  (payment released)
       ├─ raise_dispute()     ──→ Disputed (full 100%)
       │       ├─ resolve_dispute(approve=true)  ──→ Resolved (payment released)
       │       ├─ resolve_dispute(approve=false) ──→ Pending  (supplier resubmits)
       │       └─ resolve_dispute_timeout()      ──→ Resolved (default_resolution)
       └─ raise_partial_dispute() ──→ Disputed (uncontested released)
               ├─ resolve_dispute(approve=true)  ──→ Resolved (contested paid to supplier)
               ├─ resolve_dispute(approve=false) ──→ Resolved (contested refunded to buyer)
               └─ resolve_dispute_timeout()      ──→ Resolved (default_resolution)
```

---

Contract Functions
All functions require the relevant party to sign the transaction (Soroban auth).
`init(admin: Address)`
Initialises the contract. Called once by the deployer right after deployment.
`create_shipment(...) → String`
Creates a new shipment, validates milestone percentages sum to 100,
and transfers `total_amount` USDC from the buyer into escrow.

```
Parameters:
  shipment_id   String    — unique ID for this shipment
  buyer         Address   — funds source + milestone approver
  supplier      Address   — payment recipient
  logistics     Address   — in-transit proof submitter
  arbiter       Address   — dispute resolver
  token         Address   — Stellar Asset Contract address used for escrow
  total_amount  i128      — total USDC to lock (in stroops)
  milestones    Vec<Milestone> — ordered list, percentages must sum to 100

Returns: shipment_id (same as input, for confirmation)
```

Allowed token list
The `token` parameter on `create_shipment` is checked against an admin-managed allowlist (`DataKey::AllowedTokens`). By default the list is empty, which means **open mode**: any Stellar Asset Contract (SAC) address is accepted. Once the admin adds at least one token, `create_shipment` only accepts tokens on that list — a non-listed token panics with `"token is not in the approved whitelist"`.
Function Who Effect
`add_allowed_token(token)` Admin only Approves a SAC address for use as `create_shipment`'s `token` parameter
`remove_allowed_token(token)` Admin only Revokes approval; existing shipments that already use the token are unaffected
`get_allowed_tokens() → Vec<Address>` Anyone (read-only) Returns the current allowlist (empty = open mode)
The allowlist gates shipment creation only. After a shipment is created, all payouts (`confirm_milestone`, dispute resolution, cancellation refunds, etc.) always use the token address stored on that shipment — they never re-check the allowlist.
`submit_proof(caller, shipment_id, milestone_index, proof_hash, proof_type)`
Supplier or logistics submits proof for a milestone (`proof_hash` is the payload reference, e.g. an IPFS CID; `proof_type` is a short symbol naming the content scheme, e.g. `ipfs`, `sha256`, or `url`).
Milestone must be in `Pending` status. Moves status to `ProofSubmitted`.
Invoice hash
An invoice hash is an optional, immutable `BytesN<32>` commitment (e.g. a SHA-256 digest of an off-chain invoice PDF) that the supplier can attach to a milestone for audit and reconciliation. It does not move funds; it only records a cryptographic reference next to the milestone after proof is on-chain.
Function Who When
`attach_invoice_hash(caller, shipment_id, milestone_index, invoice_hash)` Supplier only Shipment `Active`, milestone past `Pending` (proof already submitted), and no hash set yet
`get_invoice_hash(shipment_id, milestone_index) → Option<BytesN<32>>` Anyone (read-only) Returns the stored hash, or `None` if none was attached
Rules:
Only the shipment's supplier may call `attach_invoice_hash` (buyer, logistics, arbiter, and strangers are rejected).
Proof must already be submitted for that milestone — attaching while still `Pending` panics with `"proof must be submitted before attaching invoice hash"`.
The hash is immutable: a second attach for the same milestone panics with `"invoice hash already set and is immutable"`.
Emits `invoice_hash_attached` with `(milestone_index, invoice_hash, caller)`.
Proof content-type whitelist
The buyer can constrain which `proof_type` values are valid for each milestone before anyone submits proof. This is enforced inside `submit_proof`: if a non-empty whitelist is stored for that milestone, the supplied `proof_type` must appear in the list or the call panics (`proof type not in whitelist`). If no whitelist was configured, or the buyer cleared it with an empty list, any `proof_type` is accepted.
Function Who When
`set_proof_whitelist(buyer, shipment_id, milestone_index, allowed_types)` Buyer Shipment `Active`, milestone still `Pending` (before first submission)
`get_proof_whitelist(shipment_id, milestone_index) → Vec<Symbol>` Anyone (read-only) Returns allowed types; empty vector means “any type”
`get_milestone_proof_type(shipment_id, milestone_index) → Option<Symbol>` Anyone (read-only) Type recorded at submission, or `None` if not submitted yet
Example: the buyer allows only IPFS proofs on milestone 0:

```
set_proof_whitelist(buyer, "SHIP-001", 0, [ipfs])
submit_proof(supplier, "SHIP-001", 0, "<cid>", ipfs)   // ok
submit_proof(supplier, "SHIP-001", 0, "<hash>", sha256) // fails — not whitelisted
```

Pass an empty `allowed_types` vector to `set_proof_whitelist` to remove the restriction for that milestone.
`confirm_milestone(buyer, shipment_id, milestone_index)`
Buyer confirms a `ProofSubmitted` milestone. Automatically calculates
and transfers the milestone's payment percentage to the supplier.
If all milestones are confirmed, shipment status becomes `Completed`.
`claim_auto_confirmation(shipment_id, milestone_index)`
Anyone can trigger automatic confirmation after the milestone's inactivity
window has elapsed. The window is measured in ledger sequences from the
ledger where proof was submitted:
`current_ledger >= proof_submitted_ledger + effective_window`.
The effective window is selected in this order:
`review_window_ledgers` (per-shipment override), `auto_confirm_ledgers`
(per-shipment default), then the global value configured by
`set_auto_confirm_threshold`. A value of `0` disables auto-confirmation;
`review_window_ledgers = Some(0)` explicitly disables it for that shipment.

The shipment must still be `Active` and the milestone must be
`ProofSubmitted`. A successful claim changes the milestone to `Confirmed`,
clears its proof-submission ledger, releases the milestone payment to the
supplier through the normal fee, advance-deduction, and penalty logic, and
updates the shipment's confirmation tracking. If it confirms the final
milestone, the shipment becomes `Completed`. Because anyone can submit the
claim, keepers, indexers, or other third parties can trigger it without the
buyer's signature. The buyer cannot manually confirm or dispute the milestone
after the auto-confirm window has expired.

`set_auto_confirm_threshold(admin, window_ledgers)`
Admin-only setter for the global default inactivity window, measured in
ledgers after proof submission. `0` disables the global fallback.
`get_auto_confirm_threshold() → u32` returns the configured global value.
`batch_confirm_milestones(buyer, shipment_id, milestone_indices)`
Confirms multiple milestones in a single transaction. Functionally equivalent
to calling `confirm_milestone` once per index, but saves transaction fees and
reduces the number of on-chain round-trips when several milestones are ready
to approve at once.

```
Parameters:
  buyer              Address   — must be the shipment's registered buyer
  shipment_id        String    — shipment to operate on
  milestone_indices  Vec<u32>  — ordered list of milestone indices to confirm
```

Behavior:
The call is atomic — if any index is invalid or any milestone is not in
`ProofSubmitted` status, the entire transaction is reverted and no payments
are released.
All indices are validated before any state is mutated, so partial application
never occurs.
Each confirmed milestone triggers its own `milestone_confirmed` event and
releases that milestone's proportional payment to the supplier (same fee and
advance-deduction logic as `confirm_milestone`).
Passing an empty `milestone_indices` list is a no-op — the call returns
without error or side-effects.
If all milestones end up confirmed after the batch, shipment status
automatically transitions to `Completed` (same as `confirm_milestone`).
vs. `confirm_milestone`:
`confirm_milestone` `batch_confirm_milestones`
Milestones per call 1 Many
Atomicity Single milestone All-or-nothing across the batch
Partial failure N/A Reverts entire batch
Gas / fee cost Per milestone One transaction for all
Example — confirm all three milestones at once after all proofs are in:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source buyer-account \
  --network testnet \
  -- batch_confirm_milestones \
  --buyer <BUYER_ADDRESS> \
  --shipment_id "SHIP-001" \
  --milestone_indices '[0, 1, 2]'
```

`raise_dispute(buyer, shipment_id, milestone_index)`
Buyer disputes 100% of a `ProofSubmitted` or `ConfirmedHeld` milestone's payment. Freezes the milestone in `Disputed` state — no payment can be released until arbiter resolves.
`withdraw_dispute(buyer, shipment_id, milestone_index)`
Buyer-initiated withdrawal of an already-raised dispute. This is a buyer-side rollback for a milestone that was moved into `MilestoneStatus::Disputed` but should not remain disputed. It is intended for the case where the buyer reconsiders the dispute, wants to preserve the supplier's original proof, and wants to continue through the normal proof/confirmation workflow instead of waiting for arbiter resolution.

Who may call it:

- Only the shipment's registered buyer may invoke it.
- Any other caller, including the supplier, logistics provider, or an unrelated address, is rejected with `unauthorized`.
- The shipment must still be `Active`; a shipment that has already moved beyond active handling cannot be used to withdraw a dispute.

What the function changes on success:

- It validates the milestone index and confirms the milestone is still in `Disputed` state.
- It reverts the milestone back to `ProofSubmitted`.
- It clears `dispute_opened_ledger` to remove the dispute start timestamp.
- It decreases the shipment's `open_dispute_count`.
- It removes the dispute from the active-disputes list.
- It does not delete the supplier's proof or change the milestone's underlying evidence; the original submitted proof remains available for ordinary buyer confirmation or for a later re-dispute.

Why this matters:

- The buyer is effectively abandoning the dispute without forcing a resolution outcome.
- The milestone returns to the same state it was in before the dispute was raised, so the normal approval path can resume.
- No funds are released by this function itself; the dispute is simply canceled and the payment remains in escrow until the buyer confirms the milestone or raises a new dispute.

Important restriction:

- `withdraw_dispute()` is only valid while the milestone is still `Disputed`.
- Once the case leaves that state because an arbiter has begun or completed resolution, the function panics with `milestone is not in disputed status`.
- In other words, it is a narrow rollback for an open dispute only; it cannot reverse a dispute that has moved into review or been resolved.

`raise_partial_dispute(buyer, shipment_id, milestone_index, contested_percent: u32)`
Buyer contests only a specified percentage (`contested_percent` from `1` to `99`) of a milestone's value. The uncontested portion `(100 - contested_percent)%` is immediately calculated and released to the supplier, while the contested portion is held in escrow in `Disputed` state pending arbiter resolution. Panics if an approved advance exists for the milestone.
`resolve_dispute(arbiter, shipment_id, milestone_index, approve: bool)`
Arbiter resolves a `Disputed` milestone (full or partial).
For full disputes:
`approve = true` → releases payment to supplier (minus arbiter fee), status → `Resolved`
`approve = false` → refunds buyer / rejects proof, status → `Pending` (supplier must resubmit)
For partial disputes:
`approve = true` → releases contested portion to supplier (minus arbiter fee), status → `Resolved`
`approve = false` → refunds contested portion to buyer (minus arbiter fee), status → `Resolved`
`resolve_dispute_timeout(shipment_id, milestone_index)`
Anyone may call this once the shipment's `dispute_timeout_seconds` has elapsed since the dispute opened without arbiter resolution. Applies the shipment's `default_resolution`: `Supplier` releases the contested amount to the supplier (minus arbiter fee); `Buyer` refunds it to the buyer. See [Dispute Auto-Resolution Timeout](#dispute-auto-resolution-timeout) below for full details.
`check_escalation(shipment_id, milestone_index)`
Checks whether an open dispute on a milestone has exceeded the admin-configured escalation threshold without arbiter resolution. Emits `dispute_escalated` event if the threshold is met or exceeded. Callable by anyone.
`set_escalation_threshold(admin, threshold_ledgers: u32)`
Admin-only. Sets the dispute escalation threshold in ledgers (`0` = disabled). `get_escalation_threshold() → u32` returns the current threshold.
`cancel_shipment(buyer, shipment_id)`
Cancels the shipment if no milestones have been confirmed yet.
Returns all locked funds to the buyer.
`top_up_escrow(buyer, shipment_id, additional_amount)`
Buyer adds more funds to an already active shipment's escrow.
Use this when the shipment scope or expected payout increases after
creation, for example:
the order was expanded and the buyer wants to cover the extra cost
milestone payments need to be increased before the shipment finishes
the buyer wants to add a buffer so later confirmations do not run short
The call is buyer-only and only works while the shipment is `Active`.
It transfers the additional USDC into the contract, increases the
shipment's `total_amount`, and leaves the milestone percentages unchanged.
That means every milestone keeps the same percentage split, but each
milestone's absolute payout becomes larger because the escrow base grew.

---

Partial Disputes & Escalation Checks
Partial Disputes Mechanics
In real-world supply chain transactions, delivered goods may be partially damaged, incomplete, or slightly off-specification without justifying a total transaction freeze. ChainSettle supports partial disputes via `raise_partial_dispute()`:
1–99% Contested Split: The buyer specifies `contested_percent` (must be between `1` and `99`). The contract calculates:
Uncontested share: `(100 - contested_percent)%` of milestone value.
Contested share: `contested_percent%` of milestone value.
Immediate Uncontested Payout: The uncontested portion is immediately released to the supplier at dispute-raising time (net of standard platform fees), updating the shipment's `released_amount` and reducing total escrow balance (`TotalEscrowed`).
Advance Request Constraint: If an approved advance request already exists on the milestone, `raise_partial_dispute()` panics with `"partial dispute not allowed when an approved advance exists for this milestone"`. Buyers must use full `raise_dispute()` when an advance has been approved.
Resolution Path (`resolve_dispute`):
Arbiter Approves (`approve = true`): Supplier receives the contested portion minus the arbiter fee. Dispute bond is returned to the buyer. Milestone status transitions to `Resolved`.
Arbiter Rejects (`approve = false`): Buyer receives a refund of the contested portion minus the arbiter fee. Milestone status transitions to `Resolved` (unlike full disputes where rejection resets status to `Pending`, partial disputes finalize at `Resolved` because the uncontested portion was already paid out).
Dispute Escalation & Escalation Threshold
To ensure disputes do not stall indefinitely without arbiter action, ChainSettle provides a dispute escalation check mechanism:
Configuring the Threshold: The contract admin configures the global escalation threshold using `set_escalation_threshold(admin, threshold_ledgers)`. Setting `threshold_ledgers = 0` disables escalation checks (default).
Checking Escalation (`check_escalation`): Anyone (buyer, supplier, arbiter, or automated backend services) can invoke `check_escalation(shipment_id, milestone_index)`.
Ledger Sequence Comparison: `check_escalation()` inspects the target milestone:
Verifies the milestone is in `MilestoneStatus::Disputed`.
Compares the current ledger sequence (`env.ledger().sequence()`) against `opened_ledger + threshold` (where `opened_ledger` is recorded in `dispute_opened_ledger` when the dispute was raised).
If `current_ledger >= opened_ledger + threshold`, the contract emits a `dispute_escalated` event containing `(milestone_index, opened_ledger, current_ledger)`.
Off-Chain Handling: The backend service (`chainsetttle-backend`) monitors `dispute_escalated` events to alert platform administrators, notify the assigned arbiter, or trigger automated resolution escalation workflows.
Dispute Auto-Resolution Timeout
If an arbiter fails to act on an open dispute, the dispute can stall a milestone indefinitely and freeze its payment. Each shipment configures a time-based auto-resolution fallback that lets *anyone* finalize the dispute once a deadline passes, so the buyer is never permanently blocked by an unresponsive arbiter.

Configuring the timeout
The two relevant fields are set at `create_shipment` time on the `Shipment` record (see `Shipment` struct above) and are immutable afterwards — there is no admin setter to change them:
`dispute_timeout_seconds: u64` — the deadline, in **seconds**, measured from the ledger timestamp recorded when the dispute was raised. `0` (the default) disables auto-resolution, meaning the dispute stays frozen until the arbiter resolves it.
`default_resolution: Resolution` — the outcome applied on timeout: `Supplier` releases the disputed funds to the supplier, `Buyer` refunds them to the buyer.
When a buyer calls `raise_dispute` or `raise_partial_dispute`, the contract records the open time in the `DisputeOpenedAt` storage entry (this is a wall-clock timestamp in seconds — separate from the escalation threshold, which works in ledgers).

Who can call it
`resolve_dispute_timeout(shipment_id, milestone_index)` is **permissionless** — any account (buyer, supplier, arbiter, bot, or backend service) may invoke it once the window has elapsed. The caller does not need to authenticate as the arbiter. It is gated by the emergency pause circuit breaker like the rest of the shipment lifecycle.

When it fires
The call validates several preconditions and panics if any fail:
- the shipment must be `Active`,
- the milestone index must exist and the milestone must be in `Disputed` status,
- `dispute_timeout_seconds` must be non-zero (otherwise `"dispute timeout not configured for this shipment"`),
- the current ledger timestamp must be at least `opened_at + dispute_timeout_seconds` (otherwise `"DisputeTimeoutNotReached"`),
- the opened-at timestamp must have been recorded (otherwise `"dispute opened timestamp not recorded"`).
If the arbiter resolves the dispute first, the milestone leaves `Disputed` status and `resolve_dispute_timeout` is no longer callable for it — arbiter action always takes priority.

Default outcome
The function resolves only the **contested** portion of the milestone payment:
- Full disputes (`raise_dispute`): the entire milestone gross payment is contested.
- Partial disputes (`raise_partial_dispute`): only `contested_percent%` is contested (the uncontested share was already released to the supplier at dispute time), so the payout/refund is based on that slice.
`Resolution::Supplier`:
- The arbiter (platform) fee is deducted from the contested amount via the standard fee logic,
- the rate-limiting circuit breaker and per-address outflow checks run before any transfer,
- the net amount is transferred to the supplier, and the **gross** contested amount is added to `shipment.released_amount`.
`Resolution::Buyer`:
- The full contested amount is transferred to the primary buyer (`buyers[0]`) — no fee is deducted,
- `released_amount` is not incremented, since nothing reaches the supplier.
Either way, the contract's `TotalEscrowed` balance for the shipment's token is decremented by the contested amount.

Side effects
After applying the outcome:
- the milestone status moves to `Resolved`,
- `open_dispute_count` is decremented and `last_dispute_resolved_ledger` is recorded,
- the entry is removed from the `ActiveDisputes` list and the temporary `DisputeOpenedAt` and `DisputeContestedPercent` storage entries are cleared,
- two events are emitted: `dispute_auto_resolved` (`(milestone_index, resolution)` where `resolution` is `supplier` or `buyer`) and `dispute_resolved`,
- if this was the last outstanding milestone, the shipment transitions to `Completed`, a `shipment_completed` event fires, and the supplier's `ReputationScore.completed` counter is incremented.

Example — auto-resolve a stalled dispute after the timeout:
```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- resolve_dispute_timeout \
  --shipment_id "SHIP-001" \
  --milestone_index 1
```
`get_shipment(shipment_id) → Shipment` (read-only)
Returns the full shipment record.
`get_milestone(shipment_id, milestone_index) → Milestone` (read-only)
Returns a single milestone.
`get_reputation(supplier) → ReputationScore` (read-only)
Returns the supplier's cumulative `completed`, `disputed`, and `cancelled`
shipment counters. The call requires no authorization and returns zeroes when
the supplier has no recorded activity; it does not calculate or persist a
separate weighted rating.
`get_shipment_risk(shipment_id) → ShipmentRisk` (read-only)
Returns a risk snapshot for the shipment based on milestone deadlines and
dispute history.
```
Fields:
  late_milestones     u32 — milestones past their deadline that are not yet
                            Confirmed or Resolved
  disputed_milestones u32 — milestones currently in Disputed or Resolved state
  total_milestones    u32 — total milestones defined for the shipment
```
Inputs that feed the risk score:
- `shipment_id` — identifies the shipment to evaluate
- Current ledger sequence — compared against each milestone's stored deadline
- Milestone status — determines whether a milestone counts as late or disputed

How to interpret the returned value:
- `late_milestones / total_milestones` indicates schedule risk (higher = more
  milestones are behind deadline).
- `disputed_milestones / total_milestones` indicates dispute exposure (higher =
  a larger share of milestones have been contested or resolved through dispute).
- A shipment with `late_milestones = 0` and `disputed_milestones = 0` is
  currently on schedule with no dispute history.
`get_escrow_balance(shipment_id) → i128` (read-only)
Returns the amount of USDC still locked in escrow.

Milestone Completion Percentage
`get_completion_percentage(shipment_id) → u32` (read-only)
Returns how far the shipment has settled, as a whole number in `0..=100`, derived
from the milestone payment weights. Full reference with worked examples:
docs/completion-percentage.md.
Milestone payment weights are the share of `total_amount` that settling a
milestone releases from escrow. They come from `Milestone.payment_percent`
(whole percents that `create_shipment` requires to sum to exactly `100`), or from
`ShipmentOptions.milestone_splits` when supplied (basis points that must sum to
exactly `10_000`, taking precedence over `payment_percent`).
`milestone_gross_payment()` converts a weight into money, and this is the only
formula used to move escrow out for a milestone:

| Weight source | Gross payment for milestone `i` |
|---|---|
| `milestone_splits` (bps) | `total_amount * splits_bps(i) / 10_000` |
| `payment_percent` (fallback) | `total_amount * payment_percent(i) / 100` |

Because the weights always cover the whole shipment, `Σ gross(i) = total_amount`,
so the settled money ratio *is* the settled weight ratio:

```text
settled = released_amount + total_advanced_amount
pct     = clamp(settled * 100 / total_amount, 0, 100)     // integer division
```

`released_amount` grows by one milestone weight (minus any late-delivery penalty)
each time that weight leaves escrow — `confirm_milestone`,
`batch_confirm_milestones`, `release_held_payment`, `claim_auto_confirmation`, the
uncontested share of `raise_partial_dispute`, and dispute resolutions that pay the
supplier or refund a contested partial share. `total_advanced_amount` holds
approved advances (a fraction of a single milestone weight) and is cleared as that
milestone is confirmed, so an advance is never counted twice.
With the standard 25 / 50 / 25 weights the query therefore walks
`0 → 25 → 75 → 100`. Notable properties:

| Rule | Effect on the reading |
|---|---|
| Integer division | fractional weights floor (a `3_333` bps milestone reads `33`) |
| Clamped to `[0, 100]` | top-ups, penalties or rebalances can never return out-of-range |
| Fees (platform, logistics, arbiter, referral) | no effect — deducted from the payout, not from the weight |
| `holdback_ledgers > 0` | a `ConfirmedHeld` milestone counts only after `release_held_payment` |
| Late-delivery penalty | credits `gross(i) - penalty`, so a late shipment can end below `100` |
| Partial dispute refunded to buyer | still counted — the query measures escrow settled, not supplier earnings |
| Full dispute rejected (milestone back to `Pending`) | not counted; that weight can still settle later |
| `top_up_escrow` | raises the denominator, diluting an in-flight percentage |
| `cancel_shipment` / `supplier_cancel` / `claim_deadline_refund` | freeze the reading at the settled share |
| Unknown/archived `shipment_id` | panics with `"shipment not found"` |

It is the exact complement of `get_escrow_balance`, which returns the unsettled
remainder `total_amount - released_amount - total_advanced_amount`.

`get_total_escrowed_value(token) → i128` (read-only)
Returns the aggregate amount of a given token currently locked in escrow across
all shipments on the contract. The value is scoped per-token: `token` is the
Stellar Asset Contract address used at shipment creation, and the contract keeps
one independent counter per token (`DataKey::TotalEscrowed(token)`), so a
multi-token deployment (e.g. USDC and EURC) never mixes balances. Returns `0`
when no shipment has ever been created with that token (no record exists) or
when every shipment using it has fully paid out.

The counter is incremented by `total_amount` on `create_shipment` and decremented
whenever funds leave escrow — milestone confirmations (`confirm_milestone`,
`batch_confirm_milestones`, `release_held_payment`, `claim_auto_confirmation`),
advance approvals (`approve_advance`), partial-dispute uncontested payouts,
cancellations (`cancel_shipment`, `supplier_cancel`), deadline refund claims,
dispute timeouts (`resolve_dispute_timeout`), and emergency recovery.

Relation to `get_escrow_balance()`: `get_escrow_balance(shipment_id)` is scoped
per-shipment — it returns the unlocked balance of one shipment, denominated in
that shipment's own token (`total_amount - released_amount -
total_advanced_amount`). `get_total_escrowed_value(token)` is scoped per-token —
it aggregates locked funds across every shipment created with that token.
Summing `get_escrow_balance()` over all shipments sharing a token approximates
the aggregate, with two known divergences: `top_up_escrow` raises a shipment's
per-shipment balance without updating the aggregate counter, and
dispute-resolution payouts (`resolve_dispute`) move funds without decrementing
it. Indexers should treat the aggregate as an approximation of locked supply and
reconcile against per-shipment balances when exact accounting is required.

Shipment Listing & Queries
These read-only functions let callers enumerate shipments stored on the contract. No authorization is required.

`list_shipments(cursor, limit, status_filter) → (Vec<String>, Option<u32>)` (read-only)

Returns a page of shipment IDs, with cursor-based pagination so callers can walk the full list incrementally without loading everything at once.

| Parameter | Type | Description |
|---|---|---|
| `cursor` | `Option<u32>` | Index to start reading from. Pass `None` (or `0`) to begin at the first shipment. |
| `limit` | `u32` | Maximum number of IDs to return per page. Capped at `50` (`LIST_SHIPMENTS_MAX_PAGE`); larger values are silently clamped. |
| `status_filter` | `Option<ShipmentStatus>` | When `Some(status)`, only IDs of shipments in that status are returned (`Active`, `Completed`, or `Cancelled`). `None` returns IDs across all statuses. |

Return value — a tuple `(ids, next_cursor)`:
- `ids` — the shipment IDs for this page (up to `limit`).
- `next_cursor` — `Some(index)` when more results exist; pass this value as `cursor` in the next call to fetch the following page. `None` means the last page has been reached.

Pagination example — walk all active shipments in pages of 10:

```bash
# First page
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- list_shipments \
  --cursor null \
  --limit 10 \
  --status_filter '{"Active": null}'

# If next_cursor = 10, fetch the next page
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- list_shipments \
  --cursor 10 \
  --limit 10 \
  --status_filter '{"Active": null}'
```

`get_shipments_by_supplier(supplier) → Vec<String>` (read-only)

Returns all shipment IDs where `supplier` is the registered supplier. The list is maintained automatically as shipments are created — callers receive every shipment the address has ever participated in as a supplier, regardless of status.

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_shipments_by_supplier \
  --supplier <SUPPLIER_ADDRESS>
```

`get_shipments_by_buyer(buyer) → Vec<String>` (read-only)

Returns all shipment IDs where `buyer` is the registered buyer. Equivalent to `get_shipments_by_supplier` but scoped to the buyer role.

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_shipments_by_buyer \
  --buyer <BUYER_ADDRESS>
```

`get_shipment_count(address) → u32` (read-only)

Returns the total number of distinct shipments associated with `address` in either the buyer or supplier role. If the address holds both roles on the same shipment (buyer and supplier are the same address), that shipment is counted only once — duplicates are deduplicated before the count is returned.

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_shipment_count \
  --address <ADDRESS>
```

`get_contract_stats() → ContractStats` (read-only)

Returns contract-level aggregate statistics. No authorization is required. All counters default to `0` if no shipments have been created yet.

```rust
pub struct ContractStats {
    pub total_shipments: u64,       // total number of shipments ever created
    pub total_volume: i128,         // cumulative USDC locked across all shipments (in stroops)
    pub total_disputes: u64,        // total number of disputes ever raised
    pub completed_shipments: u64,   // total number of shipments that reached Completed status
}
```

Field descriptions:

| Field | Type | Description |
|---|---|---|
| `total_shipments` | `u64` | Incremented once each time `create_shipment` succeeds. |
| `total_volume` | `i128` | Sum of `total_amount` across all created shipments, in the token's smallest unit (stroops for USDC). |
| `total_disputes` | `u64` | Incremented each time `raise_dispute` or `raise_partial_dispute` is called successfully. |
| `completed_shipments` | `u64` | Incremented when all milestones in a shipment reach a settled state and the shipment transitions to `Completed`. |

Example:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_contract_stats
```

Example response:

```json
{
  "total_shipments": 42,
  "total_volume": "420000000000000",
  "total_disputes": 5,
  "completed_shipments": 38
}
```

---

Milestone Deadline Extensions
Suppliers can ask for more ledger time on an active shipment milestone,
and buyers decide whether to accept that new deadline. The flow is
single-request: each milestone can have only one pending extension
request at a time.
`request_extension(caller, shipment_id, milestone_index, extra_ledgers)`
— supplier-only. Creates a pending request to add `extra_ledgers` to
the milestone deadline and emits `extension_requested`. The shipment
must be active, the milestone index must exist, and the call fails if
that milestone already has a pending extension request.
`approve_extension(buyer, shipment_id, milestone_index)` — buyer-only.
Approves the pending request, clears it, and writes the effective
milestone deadline. If a deadline already exists, the contract adds
the requested extra ledgers to it; otherwise, it starts from the
current ledger sequence and adds the requested extra ledgers. The call
emits `extension_approved`.
`deny_extension(buyer, shipment_id, milestone_index)` — buyer-only.
Denies the pending request, clears it, leaves the current deadline
unchanged, and emits `extension_denied`.
`get_milestone_deadline(shipment_id, milestone_index) → u32`
(read-only) — returns the stored effective deadline ledger for the
milestone, or `0` when no deadline has been set.
Extension request, approval, and denial calls are paused by the emergency
circuit breaker; `get_milestone_deadline` remains available while paused.

Milestone Amendment History Tracking
Buyer and supplier can mutually agree to change a **Pending** milestone's payment percentage and/or name after a shipment is created — for example, to fix a typo in the milestone name or rebalance value between milestones before any proof is submitted. Changes only take effect once **both parties agree on the exact same terms**.

**`propose_amendment(caller, shipment_id, milestone_index, new_percent, new_name)`** — buyer or supplier only.

- The shipment must be `Active` and the target milestone must be `Pending` — amendments are not allowed once proof has been submitted.
- Proposals are stored in temporary storage keyed to the milestone, tracking `new_percent`, `new_name`, and a `buyer_agreed` / `supplier_agreed` flag for each side.
- Calling this function records the caller's agreement to the given `new_percent` / `new_name`. If a pending proposal already exists with **different** terms, it is reset and both agreement flags are cleared — meaning changing your proposed terms effectively restarts consent from scratch, even if the other party had already agreed to the old terms.
- Emits `amendment_proposed` on every call, regardless of whether it completes the agreement.

**Mutual consent — what happens when both sides agree:**

Once `buyer_agreed` and `supplier_agreed` are both `true` for the same terms, the amendment executes automatically in that same call:

1. Validates that the new percentage still keeps all milestones summing to 100% — panics with `"milestone percentages must sum to 100"` otherwise.
2. Updates the milestone's `payment_percent` and `name`.
3. Persists the updated shipment and clears the temporary proposal.
4. Appends an entry to that milestone's amendment log (see below).
5. Records an `amendment_accepted` entry in the contract's admin audit trail.

There is no separate `approve_amendment` call — the second matching `propose_amendment` call *is* the approval.

**No explicit rejection:** there is currently no `reject_amendment` or `cancel_amendment` function. A party can effectively withdraw by proposing different terms (which resets both flags), but a first mover's agreement can't be explicitly rescinded without someone proposing something new.

**`get_amendment_log(shipment_id, milestone_index) → Vec<AmendmentEntry>`** — read-only, callable by anyone.

Returns the append-only, chronological history of amendments **actually applied** to a milestone (not proposals — only agreements that both parties completed). Each entry:

```rust
pub struct AmendmentEntry {
    pub proposer: Address,      // who made the triggering call that completed the amendment
    pub old_payment_percent: u32,
    pub new_payment_percent: u32,
    pub ledger: u32,
}
```

The log is capped at 20 entries per milestone; once full, the oldest entry is evicted (FIFO) to make room for the newest. Unlike the admin audit log, this is scoped per-milestone and only records completed amendments, not every proposal attempt.
Emergency pause (circuit breaker)
Admin-only kill switch that halts state-changing calls across every
shipment without touching any stored data. Locked funds stay in escrow
untouched while paused — this only blocks new actions, it never moves
or seizes funds itself.
`pause(admin)` — sets the paused flag and emits `contract_paused`.
`unpause(admin)` — clears the paused flag and emits `contract_unpaused`.
`is_paused() → bool` (read-only) — returns the current state.
While paused, every function that guards on `assert_not_paused` panics
with `"contract is paused"`. That covers the full shipment lifecycle:
`create_shipment`, `submit_proof`, `confirm_milestone`,
`raise_dispute`/`raise_partial_dispute`, `resolve_dispute`, `resolve_dispute_timeout`,
`cancel_shipment`/`supplier_cancel`, escrow top-ups, advances,
extensions, amendments, transfers, and claims. Read-only getters,
admin configuration setters (fees, thresholds, whitelists, etc.), and
admin succession (`nominate_admin`/`accept_admin`) are not gated by
pause and keep working normally.
Only the current admin can call `pause`/`unpause` — same
`assert_admin` check used everywhere else. To resume normal operation,
the admin simply calls `unpause`; no other recovery steps are needed.
Rate-limiting circuit breaker
A sliding-window rate limiter that caps total token outflow over a rolling
period of ledger sequences. It is separate from the emergency pause and
protects against abnormal drain scenarios (e.g. exploit, compromised key)
without halting the entire contract.
`set_circuit_breaker(admin, limit: i128, window_ledgers: u32)`
Admin-only. Configures the circuit breaker parameters and immediately
resets the current window counter to zero.
Parameter Description
`limit` Maximum cumulative outflow (in token smallest units) allowed within one window. Pass `0` to disable the circuit breaker entirely.
`window_ledgers` Duration of the tracking window in ledger sequences. After this many ledgers have elapsed since the window start, the counter resets automatically.
When the breaker is active, every payment that moves tokens out of escrow
(`confirm_milestone`, `resolve_dispute`, `resolve_dispute_timeout`, partial dispute uncontested
release, advance payouts, claim payouts, and amendments that release
funds) is checked against the window:
If the current ledger has moved past `window_start + window_ledgers`,
the window resets — outflow counter returns to zero and the window
start updates to the current ledger.
The pending payment is added to the running `window_outflow` counter.
If the new total would exceed `limit`, the call panics with
`"circuit breaker triggered: outflow limit exceeded"` and no state
is mutated.
Because the check runs before the payment executes, a triggered
breaker leaves all funds exactly where they were — no partial transfers
occur.
`get_circuit_breaker() → (i128, u32, u32, i128)` (read-only)
Returns the current circuit breaker configuration and window state:
`(limit, window_ledgers, window_start, window_outflow)`. When `limit`
is `0`, the circuit breaker is disabled and the other fields are
irrelevant.
Example — set a 100,000 USDC per 1,000-ledger cap:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source admin-account \
  --network testnet \
  -- set_circuit_breaker \
  --admin <ADMIN_ADDRESS> \
  --limit 10000000000000 \
  --window_ledgers 1000
```

Platform fee configuration
The admin can configure a platform fee deducted from each milestone payment.
`set_fee_config(admin, fee_bps, treasury)` — admin-only. Sets the platform
fee rate and treasury address.
`get_fee_config() → Option<FeeConfig>` (read-only) — returns the current
configuration, or `None` if no fee has been configured (effective rate = `0` bps).

| Parameter  | Type      | Description                                                              |
| ---------- | --------- | ------------------------------------------------------------------------ |
| `fee_bps`  | `u32`     | Fee rate in basis points (0–1000, where 100 = 1%). Capped at 1000 (10%). |
| `treasury` | `Address` | Wallet address that receives the collected platform fees.                |

The fee is deducted from each milestone payout at confirmation or dispute
resolution time. If no fee config has been set, the effective fee rate is
`0` bps and no platform fees are charged.

Per-shipment fee override
An admin can override the fee rate for a single shipment without changing
the global `FeeConfig` or the buyer's volume-based tier. This is useful for
negotiated rates, promotional pricing, or correcting fee tiers on a
case-by-case basis.

`set_shipment_fee_override(admin, shipment_id, fee_bps)`
Admin-only. Sets a per-shipment fee override. The shipment must be `Active`
or the call panics with `"shipment is not active"`. The override persists
until explicitly cleared or the shipment leaves the `Active` state.

`clear_shipment_fee_override(admin, shipment_id)`
Admin-only. Removes the per-shipment override. After clearing, the shipment
reverts to the normal fee resolution path.

`get_shipment_fee_override(shipment_id) → Option<u32>` (read-only)
Returns the current override in basis points, or `None` when no override
has been set.

Fee precedence
When the contract calculates the fee for a milestone payout, it resolves the
effective `fee_bps` in this order:

1. **Shipment-level override** — if `set_shipment_fee_override` was called
   for this shipment, that value is used.
2. **Locked tier bps** — when the shipment was created, the buyer's lifetime
   volume was evaluated against the configured fee tiers and the best matching
   `fee_bps` was locked onto the shipment (`ShipmentFeeBps`). This locked
   value is used if no override exists.
3. **Global FeeConfig** — if neither an override nor a locked tier exists,
   the contract falls back to `FeeConfig.fee_bps`.
4. **Zero** — if no `FeeConfig` has ever been set, the effective rate is
   `0` bps.

The locked tier value is immutable for the lifetime of the shipment: later
volume changes do not alter fees mid-flight. `get_fee_tier(address)` always
reflects the address's *current* volume against the live tier table, which
is useful for dashboards and for previewing the rate the next shipment
would lock, but it does not change fees on existing shipments.

---

Long-hold escrow rebate (#300)
The long-hold escrow rebate returns part of the platform fee to the supplier
when a milestone is paid out after the escrow has been held open long enough.
This rewards escrows that sit in the contract for a long time (long-hold) with a
cheaper effective fee when their funds are finally released.

`set_long_hold_rebate(admin, threshold_ledgers, rebate_bps)` — admin-only.
Configures the rebate and emits a `long_hold_rebate_set` event.
`get_long_hold_rebate() → (u32, u32)` (read-only) — returns the current
`(threshold_ledgers, rebate_bps)` configuration, or `(0, 0)` if the rebate has
never been configured (disabled by default).

| Parameter | Type | Description |
|---|---|---|
| `threshold_ledgers` | `u32` | Minimum age of the shipment, measured in ledger sequences, before the rebate can apply. A shipment's age is `current_ledger_sequence − Shipment.created_at`. `0` disables the rebate. |
| `rebate_bps` | `u32` | The share of the platform fee returned to the supplier, in basis points. `1 bps = 0.01%`; `10_000 bps = 100%`. `0` disables the rebate. |

A basis point (bps) is one-hundredth of a percent: `1 bps = 0.01%`, and
`10_000 bps = 100%`.

**How it works**

1. The rebate is applied on the immediate-release path of `confirm_milestone`
   (i.e. a milestone with no holdback window).
2. The platform fee is first computed on the milestone payout via
   `deduct_fee_for_shipment`: `fee_amount = payout × fee_bps / 10_000`.
3. The rebate only applies when **both** `threshold_ledgers > 0` **and**
   `rebate_bps > 0` are set, and only when a fee was actually charged
   (`fee_amount > 0`).
4. If the shipment's ledger age is at least `threshold_ledgers`, the contract
   computes `rebate = fee_amount × rebate_bps / 10_000` and adds it back to the
   supplier's net payment, reducing the effective platform fee by the same amount.

> **Ledger threshold, not time.** `threshold_ledgers` is expressed purely in
> ledgers (the Stellar ledger sequence number), not seconds/minutes. Because the
> exact wall-clock duration of a ledger depends on the network, the threshold is
> defined in ledger terms; on Stellar a ledger closes roughly every 5 seconds,
> so `1000` ledgers ≈ ~83 minutes, but this is an approximation and should not be
> relied on for time-sensitive accounting.

**Worked example** — `set_long_hold_rebate(admin, 1000, 1000)`

Assume the platform fee is configured at `fee_bps = 100` (1%) and a single
milestone is worth `1_000_000` units.

| Step | Computation | Result |
|---|---|---|
| 1. Platform fee on payout | `fee_amount = 1_000_000 × 100 / 10_000` | `10_000` |
| 2. Supplier net before rebate | `1_000_000 − 10_000` | `990_000` |
| 3. Shipment age | `3200 − 2000` (current ledger − created) | `1200` |
| 4. Threshold met? | `1200 ≥ 1000` | yes |
| 5. Rebate | `10_000 × 1000 / 10_000` | `1_000` |
| 6. Supplier net after rebate | `990_000 + 1_000` | `991_000` |
| 7. Effective platform fee | `10_000 − 1_000` | `9_000` (0.9%) |

So the supplier keeps an extra `1_000` (the rebated share of the fee), and the
collectible platform fee drops from `1%` to `0.9%` for that milestone.

---

Referral fee program
The referral fee mechanism lets an admin reward a referrer when a shipment
completes successfully. The fee is paid out of the protocol fee that would
otherwise go to the platform treasury, so the referrer receives a share of
that fee instead of the full amount being retained by the contract.
`set_referral_fee_bps(admin, bps: u32)` — admin-only. Sets the referral
fee as a basis-point value between `0` and `10_000`.
`get_referral_fee_bps() → u32` (read-only) — returns the current
referral fee setting, defaulting to `500` bps (`5%`) if no value has
been configured yet.
A basis point (bps) is one-hundredth of a percent: `1 bps = 0.01%`, and
`10_000 bps = 100%`. The contract computes the referral payout as a
fraction of the total protocol fee using the configured bps value, so a
setting of `500` bps means the referrer receives `5%` of that fee on
shipment completion.

Volume-based fee tiers
Buyers can receive a reduced platform fee once their lifetime shipment volume
crosses admin-configured thresholds. Each `FeeTier` entry is a pair:

| Field                 | Type   | Meaning                                                                    |
| --------------------- | ------ | -------------------------------------------------------------------------- |
| `min_lifetime_volume` | `i128` | Minimum cumulative volume (token base units) the address must have accrued |
| `fee_bps`             | `u32`  | Fee in basis points applied when that threshold is met                     |

`set_fee_tiers(admin, tiers: Vec<FeeTier>)` — admin-only. Stores up to 5 tier
entries on instance storage. Prefer listing higher `min_lifetime_volume` values
first; at lookup time the contract still picks the **lowest** `fee_bps` among
every tier whose threshold the address meets.
`get_fee_tier(address) → u32` (read-only) — returns the effective fee bps for
`address`:

1. Load the address's `LifetimeVolume`.
2. Among configured tiers where `volume >= min_lifetime_volume`, take the
   smallest `fee_bps`.
3. If no tier matches (or no tiers are configured), fall back to
   `FeeConfig.fee_bps`, or `0` if fee config was never set.

Per-address application: when a shipment is created, the primary buyer's
resolved tier bps is locked onto that shipment (`ShipmentFeeBps`) so later
volume changes do not alter fees mid-flight. Subsequent payouts for that
shipment use the locked value; `get_fee_tier` always reflects the address's
_current_ volume against the live tier table (useful for dashboards and for
previewing the rate the next shipment would lock).

`set_max_concurrent_disputes(admin, limit: u32)`
Admin-only. Sets how many milestones on a single shipment can be under
dispute at the same time. Defaults to `1` if never called. `raise_dispute`
and `raise_partial_dispute` check the shipment's open dispute count
against this cap and panic with `"DisputeAlreadyOpen"` once it's reached.
This is what stops a buyer from disputing several milestones of the same
shipment all at once. Without a cap, one shipment could rack up an
unbounded number of simultaneous disputes, tying up several payment
releases at once and dumping all of that resolution work on one arbiter
at the same time.

---

Advance Payment Lifecycle
The advance payment flow lets a supplier request early access to a
portion of a milestone's payment before submitting proof. This is useful
when the supplier needs working capital (e.g. to purchase materials or
cover logistics costs) before the milestone can be fulfilled.
`set_max_advance_percent(admin, percent: u32)` — admin-only. Sets the
maximum percentage of a milestone's payment that a supplier may request
as an advance. `percent` must not exceed `100`. Defaults to `30` if the
admin has never called this function.
`get_max_advance_percent() → u32` (read-only) — returns the current
advance percentage cap, defaulting to `30`.
`request_advance(caller, shipment_id, milestone_index, advance_percent)`
— supplier-only. Requests an advance draw of up to `advance_percent` of
the milestone's payment value. The milestone must be in `Pending` status.
`advance_percent` is capped by the admin-configured `max_advance_percent`;
if it exceeds the cap, the call panics with `"AdvanceExceedsMax"`.
A milestone may have only one pending advance request at a time. If an
earlier request was already approved, a second request panics with
`"AdvanceAlreadyApproved"`. The buyer can later deny a pending request
simply by not approving it — there is no explicit `deny_advance` function;
a new `request_advance` overwrites the previous unapproved request.
Emits `advance_requested` on success.
`approve_advance(buyer, shipment_id, milestone_index)` — buyer-only.
Approves a pending advance request for the given milestone. The contract
immediately transfers the advance amount (`milestone_payment * advance_percent / 100`)
from escrow to the supplier. The shipment's `total_advanced_amount` is
incremented, and the total escrowed balance is decremented accordingly.
After approval, the advance record is marked `approved` so the same
request cannot be approved twice. The advance amount is deducted from
the milestone's payout when the milestone is later confirmed (see
`confirm_milestone` / `batch_confirm_milestones`), so the supplier
receives only the remaining balance at that point. Emits
`advance_approved` on success.
Deduction on confirmation:
When the buyer confirms a milestone that has an approved advance, the
confirmation flow calls `consume_advance_for_milestone` which reads the
stored `AdvanceRequest`, returns the already-advanced amount, removes
the request from storage, and adjusts the shipment's
`total_advanced_amount`. The net payment to the supplier during
confirmation is therefore `milestone_payment - advance_amount` (minus
any applicable protocol fees). If the milestone has no approved advance,
the full milestone payment is released as normal.
Constraints:

- Buyer raises a partial dispute: If an approved advance exists for a
  milestone, `raise_partial_dispute` panics with `"partial dispute not
allowed when an approved advance exists for this milestone"`. The
  buyer must use the full `raise_dispute` flow instead.
- Milestone status: The milestone must be `Pending` when the advance is
  requested. Once proof has been submitted (`ProofSubmitted`), the
  advance window is closed and the supplier must wait for confirmation.
- Escrow coverage: The advance is paid from escrowed funds, so the
  shipment must have sufficient unlocked balance. The total advanced
  amount across all milestones is tracked in
  `shipment.total_advanced_amount` and is excluded from the recoverable
  escrow balance used by `emergency_recover`, `supplier_cancel`, and
  the `get_escrow_balance` view.
  Example lifecycle:

```bash
# 1. Supplier requests a 20% advance on milestone 0
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source supplier-account \
  --network testnet \
  -- request_advance \
  --caller <SUPPLIER_ADDRESS> \
  --shipment_id "SHIP-001" \
  --milestone_index 0 \
  --advance_percent 20

# 2. Buyer approves the advance — funds are transferred immediately
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source buyer-account \
  --network testnet \
  -- approve_advance \
  --buyer <BUYER_ADDRESS> \
  --shipment_id "SHIP-001" \
  --milestone_index 0

# 3. Later, buyer confirms the milestone — the advance is deducted
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source buyer-account \
  --network testnet \
  -- confirm_milestone \
  --caller <BUYER_ADDRESS> \
  --shipment_id "SHIP-001" \
  --milestone_index 0
```

The supplier receives the advance amount at step 2 and only the remaining
balance at step 3. If the milestone payment is 100 USDC with a 20 %
advance, the supplier gets 20 USDC at step 2 and roughly 80 USDC (minus
fees) at step 3.
Supplier Address Whitelisting
The supplier-address whitelist lets an admin operate a closed or curated
supplier network. It is enforced only when a new shipment is created:
when the list is non-empty, the shipment's `supplier` must be on it or
`create_shipment` panics with `"unauthorized"`. It does not grant an
address any additional permissions and does not restrict buyers, logistics
providers, or arbiters. Existing shipments are unaffected if their supplier
is later added to or removed from the list.
The whitelist is open by default. An empty list means every supplier is
allowed; adding the first address switches the contract to restricted mode.
Removing the last address switches it back to open mode.
Function Who Behaviour
`add_to_whitelist(admin, address)` Admin only Adds `address` as an approved supplier. Adding an address already on the list is a no-op.
`remove_from_whitelist(admin, address)` Admin only Removes `address` from the approved suppliers. It does not blacklist the address.
`is_whitelisted(address) → bool` Anyone (read-only) Returns `true` when the address is listed, or when the list is empty (open mode).
Interaction with address blacklisting
Whitelisting is an allowlist for the supplier role; blacklisting is a
denylist for every role in a new shipment (buyer, supplier, logistics, or
arbiter). Blacklisting takes precedence: a supplier that is both whitelisted
and blacklisted cannot be used to create a shipment. Conversely, removing an
address from the blacklist only removes that denial; the supplier must still
be on a non-empty whitelist to be eligible. Neither control changes an
already-created shipment.

### Reputation-Triggered Auto-Blacklisting

In addition to manual blacklisting (`blacklist_address()`), the contract supports automatic blacklisting based on a supplier's reputation counters. 

The admin can configure thresholds for cancelled and disputed shipments. When a supplier's reputation counters exceed these thresholds, they are automatically treated as blacklisted and cannot act as a supplier in new shipments.

Function | Who | Behaviour
--- | --- | ---
`set_auto_blacklist_rule(admin, max_cancelled, max_disputed)` | Admin only | Sets the auto-blacklist thresholds. If a supplier's cancelled count exceeds `max_cancelled` or their disputed count exceeds `max_disputed`, they are automatically blacklisted. Setting a threshold to `0` disables that specific check. Setting both to `0` disables auto-blacklisting entirely (default).
`get_auto_blacklist_rule() → AutoBlacklistRule` | Anyone (read-only) | Returns the current `max_cancelled` and `max_disputed` thresholds.

**Interaction with manual blacklisting:**
Auto-blacklisting operates in addition to the manual blacklist. If a supplier triggers the auto-blacklist rule, they are blocked from being assigned to new shipments just as if they had been manually blacklisted. A manual blacklist entry (`blacklist_address()`) takes precedence and will permanently block an address regardless of its reputation counters. Since reputation counters are monotonically increasing, once a supplier hits the auto-blacklist threshold, they remain blocked unless the admin increases the threshold or disables the rule.

This supplier whitelist is separate from the approved-token whitelist and the
per-milestone proof content-type whitelist.
Arbiter Pool & Assignment
ChainSettle supports an active pool of trusted arbiters, allowing for automatic assignment to distribute the dispute resolution workload.
`add_arbiter_to_pool(admin, arbiter: Address)`: Admin-only. Adds an arbiter address to the active pool.
`remove_arbiter_from_pool(admin, arbiter: Address)`: Admin-only. Removes an arbiter from the active pool.
`get_arbiter_pool() → Vec<Address>` (read-only): Returns the current list of active arbiters in the pool.
When creating a shipment, a buyer must specify an arbiter. By querying `get_arbiter_pool()`, a frontend or backend service can automatically select an arbiter using a round-robin assignment strategy. This ensures that disputes are evenly distributed among all trusted arbiters in the pool rather than overloading a single resolver.

Arbiter Rotation
When both the buyer and supplier lose confidence in the assigned arbiter, they can agree to replace them using `propose_arbiter_rotation()`. Unlike `recuse_arbiter()` (which is an arbiter-initiated voluntary step-down), rotation is initiated and controlled entirely by the two trading parties.

`propose_arbiter_rotation(caller, shipment_id, new_arbiter)`

Either the buyer or supplier calls this to nominate a replacement arbiter. The proposal is stored in temporary storage keyed to the shipment.

Mutual-consent model — both parties must agree on the **same** address:
- The first caller records their agreement (`buyer_agreed` or `supplier_agreed`) and emits `arbiter_rotation_proposed`.
- The second party calls `propose_arbiter_rotation` with the **same** `new_arbiter` address, which sets their flag and triggers automatic execution in that same transaction.
- If either party calls the function with a **different** `new_arbiter` than the one currently stored, the pending proposal is reset and both agreement flags are cleared — changing the nominated address effectively restarts the consent process from scratch.

When both `buyer_agreed` and `supplier_agreed` are `true`:
1. The shipment's `arbiter` field is updated to the new address immediately.
2. The temporary proposal is removed from storage.
3. An `arbiter_rotated` event is emitted and the change is recorded in the admin audit trail.

```bash
# Buyer proposes a new arbiter
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source buyer-account \
  --network testnet \
  -- propose_arbiter_rotation \
  --caller <BUYER_ADDRESS> \
  --shipment_id "SHIP-001" \
  --new_arbiter <NEW_ARBITER_ADDRESS>

# Supplier agrees with the same address — rotation executes immediately
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source supplier-account \
  --network testnet \
  -- propose_arbiter_rotation \
  --caller <SUPPLIER_ADDRESS> \
  --shipment_id "SHIP-001" \
  --new_arbiter <NEW_ARBITER_ADDRESS>
```

How it differs from `recuse_arbiter()`:

| | `propose_arbiter_rotation()` | `recuse_arbiter()` |
|---|---|---|
| Who initiates | Buyer **and** supplier (must both agree) | Current arbiter only |
| Trigger | Parties lose confidence in the arbiter | Arbiter voluntarily steps down |
| New arbiter | Explicitly nominated by the parties | Auto-assigned from the arbiter pool (round-robin) |
| Requires pool | No | Yes — panics with `"no available arbiter"` if pool is empty |
| Authorization | Buyer or supplier signature | Arbiter's own signature |

Both functions require the shipment to be `Active` and are blocked by the emergency pause.

Supplier Payout Batching
By default, every milestone payment (confirmation, dispute resolution,
advance approval) transfers tokens to the supplier immediately in the same
transaction that releases them. A supplier can instead opt into batched
mode, where payments accumulate in a per-supplier on-chain balance and are
withdrawn later in a single transaction.
`set_payout_mode(supplier, batched: bool)`
Supplier-only. `batched = true` switches the supplier to batched mode;
`batched = false` (the default) switches back to immediate mode. Emits
`payout_mode_set`. Only affects payments made after the switch — it does
not retroactively move an already-transferred payment into the pending
balance, and it does not by itself move an already-accumulated pending
balance back to immediate transfers.
`claim_payout(supplier, token)`
Supplier-only. Transfers the supplier's entire accumulated pending balance
for `token` in one call and resets it to zero. Panics with `"no pending
payout"` if the balance is zero. The balance is zeroed before the token
transfer (checks-effects-interactions), so a failed or reentrant transfer
cannot double-spend the same balance. Blocked while the contract is
paused. Emits `payout_claimed`.
`get_pending_payout(supplier) → i128` (read-only)
Returns the supplier's current accumulated, unclaimed payout balance.
Returns `0` if the supplier is in immediate mode or has no pending funds.
How it works:
When a milestone payment is released to a supplier (`confirm_milestone`,
`batch_confirm_milestones`, dispute resolution payouts, and approved
advances), the contract checks that supplier's payout mode:
in **immediate mode** (default), the net payment is transferred to the
supplier's wallet right away, exactly as before this feature existed.
in **batched mode**, the net payment is added to the supplier's pending
balance instead of being transferred, and the supplier must call
`claim_payout` later to receive the funds.
Payout batching only applies to the default single-supplier payout path.
If a milestone has per-payee splits configured (`MilestonePayees`), each
payee is always paid immediately regardless of the primary supplier's
payout mode.
Example — a supplier opts into batching, accrues two milestone payments,
then claims them together:
```bash
stellar contract invoke --id <CONTRACT_ID> --source supplier-account \
  --network testnet -- set_payout_mode --supplier <SUPPLIER_ADDRESS> --batched true

# ... buyer confirms milestone 0 and milestone 1; funds accumulate on-chain ...

stellar contract invoke --id <CONTRACT_ID> --source supplier-account \
  --network testnet -- get_pending_payout --supplier <SUPPLIER_ADDRESS>

stellar contract invoke --id <CONTRACT_ID> --source supplier-account \
  --network testnet -- claim_payout --supplier <SUPPLIER_ADDRESS> --token <TOKEN_ADDRESS>
```
---
Admin Action Audit Trail
The contract maintains an immutable audit log of all administrative actions
for transparency and governance accountability. Every admin function call
(pause, fee changes, blacklist operations, etc.) is automatically recorded
with context about who performed the action and when.
`get_admin_log() → Vec<AuditEntry>` (read-only)
Returns the complete history of admin actions performed on the contract.
Each `AuditEntry` contains:

```rust
pub struct AuditEntry {
    pub action: Symbol,      // Admin function called (e.g. "pause", "set_fee_config")
    pub caller: Address,     // Admin address who performed the action
    pub ledger: u32,         // Ledger sequence number (timestamp proxy)
    pub detail: Symbol,      // Context detail (e.g. "contract_paused", "fee_config_updated")
}
```

Logged actions include:
Action Triggered by Detail
`pause` `pause()` `contract_paused`
`unpause` `unpause()` `contract_unpaused`
`set_fee_config` `set_fee_config()` `fee_config_updated`
`set_max_concurrent_disputes` `set_max_concurrent_disputes()` `limit_updated`
`set_min_milestone_percent` `set_min_milestone_percent()` `percent_updated`
`set_max_advance_percent` `set_max_advance_percent()` `percent_updated`
`set_nft_hook_enabled` `set_nft_hook_enabled()` `nft_hook_updated`
`blacklist_address` `blacklist_address()` `address_blacklisted`
`remove_from_blacklist` `remove_from_blacklist()` `address_whitelisted`
`add_allowed_token` `add_allowed_token()` `allowed_token_added`
`remove_allowed_token` `remove_allowed_token()` `allowed_token_removed`
`add_arbiter_to_pool` `add_arbiter_to_pool()` `arbiter_added`
`remove_arbiter_from_pool` `remove_arbiter_from_pool()` `arbiter_removed`
Usage example:

```bash
# Query the admin audit log via CLI
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_admin_log
```

Why this matters:
Transparency: Anyone can verify when contract parameters were changed
and by whom
Governance: Off-chain tools can monitor the log and alert stakeholders
to admin actions
Forensics: If a dispute arises about contract configuration, the audit
trail provides a tamper-proof record
Accountability: Admin actions are permanently recorded on-chain, making
unilateral changes visible to all participants
The audit log uses persistent storage and is retained for the lifetime of
the contract. Unlike events (which may be pruned by Horizon), these entries
remain queryable indefinitely.

---

Admin Succession & Emergency Recovery
Two-step admin handover
Admin control transfers to a new address through a nominate/accept flow
rather than a single-step reassignment, so a typo'd or unreachable address
can never accidentally lock out contract administration:
`nominate_admin(current_admin, nominee: Address)` — Admin-only. Records
`nominee` as the pending admin and emits `admin_nominated`. This does
not change who the admin is; the current admin keeps full authority
until the nominee actively accepts.
`accept_admin(nominee: Address)` — Must be called by the nominee itself.
Fails with `"no pending nomination"` if none exists, or `"unauthorized"`
if called by any address other than the pending nominee. On success, it
sets `nominee` as the new admin, clears the pending nomination, and emits
`admin_transferred` with the old and new admin addresses.
`revoke_nomination(current_admin)` — Admin-only. Clears any pending
nomination before it's accepted (e.g. if the wrong address was nominated
or the handover is called off) and emits `nomination_revoked`.
Because the nominee must sign `accept_admin` themselves, admin rights can
never be transferred to an address that can't authorize transactions, and
the outgoing admin retains control for the entire window until acceptance.
`emergency_recover(admin, shipment_id: String)`
A last-resort, admin-only escape hatch for shipments whose escrowed funds
would otherwise be stuck indefinitely — for example, a shipment where the
buyer or supplier has gone permanently unresponsive and normal completion,
cancellation, or dispute paths can't be reached. Calling it:
Is blocked while the contract is paused (subject to `assert_not_paused`,
same as the rest of the shipment lifecycle) and requires the admin's
authorization.
Only works on a shipment that is currently `Active`; it panics with
`"shipment is not active"` otherwise.
Only becomes callable once `RECOVERY_THRESHOLD_LEDGERS` have elapsed since
the shipment's `created_at` ledger — panicking with
`"recovery threshold not reached"` before then. This threshold is a long,
fixed cooldown (currently `12,614,400` ledgers), so it's meant for
genuinely abandoned shipments, not routine intervention.
Transfers whatever remains locked in escrow (`total_amount - released_amount`) from the contract to the admin address, then marks the
shipment `Cancelled` and updates the token's total-escrowed accounting
accordingly.
Emits an `emergency_recovery` event containing the recovered amount and
the admin address, keyed to the `shipment_id`.
Because recovered funds go to the admin rather than back to the buyer,
this function is meant purely as a break-glass measure for abandoned
escrow, not as a way to resolve normal disputes — those should go through
`raise_dispute` / `raise_partial_dispute` and `resolve_dispute` instead.

---
Governance Timelock
The governance timelock makes selected high-impact parameter changes visible
on-chain before they take effect. It is controlled by the current contract
admin and measures its delay in ledger sequences.

`set_timelock_duration(admin, ledgers: u32)` — Admin-only. Sets the delay
applied to newly proposed parameter changes. The default is `0`, which allows
a proposal to be executed in the same ledger. Changing the duration does not
alter the effective ledger of an already pending proposal.

The timelock proposal flow supports these parameter symbols:

| Parameter symbol | Change applied at execution |
| --- | --- |
| `fee_config` | Updates the configured platform fee rate (`fee_bps`) only; it does not change the treasury address. |
| `max_shipment_value` | Updates the global maximum shipment value. |
| `circuit_breaker_limit` | Updates the circuit breaker's token-outflow limit. |

All three functions below are admin-only and require the admin address to
authorize the transaction:

1. **Propose:** Call `propose_param_change(admin, param, new_value)`. The
   contract records the proposed value and sets its effective ledger to the
   current ledger plus the configured timelock duration. A new proposal for
   the same `param` replaces its existing pending proposal.
2. **Wait:** Monitor the `param_change_proposed` event and wait until the
   current ledger sequence is at least the recorded effective ledger. Calling
   `execute_param_change` before then fails with `TimelockNotExpired`.
3. **Execute:** Call `execute_param_change(admin, param)` once the delay has
   elapsed. The contract applies the proposed value, removes the pending
   proposal, and emits `param_change_executed`.
4. **Cancel (optional):** Call `cancel_param_change(admin, param)` before
   execution to remove the pending proposal. It emits `param_change_cancelled`;
   a later execution attempt fails with `NoPendingParamChange`.

`propose_param_change` accepts only the listed parameter symbols at execution
time. The direct admin setters remain separate contract entry points; use the
proposal workflow when a change must be subject to the configured timelock.
---
Multisig Admin Governance
ChainSettle supports multi-signature admin governance to prevent unilateral contract
changes. Once enabled, sensitive admin operations require approval from multiple
trusted keys, and the legacy single-key `upgrade()` path is disabled.

`initialize_multisig_admin(admin, admins, threshold)`
Replaces single-admin mode with a multisig scheme. Only the current single admin
can call this. It stores the set of registered admin keys (`admins`) and the
minimum number of distinct approvals required (`threshold`). After this call:

- Any registered admin can propose or approve admin actions.
- The single-key `upgrade()` function panics; upgrades must use
  `propose_upgrade` / `approve_upgrade` instead.
- `threshold` must be between `1` and `admins.len()`. A threshold of `1`
  executes immediately on first proposal.

`propose_admin_action(admin, action_id, operation, params)`
Registers an approval from `admin` for the action identified by `action_id`.
The caller must be one of the registered multisig admins. Each admin can approve
a given `action_id` at most once. When the number of collected approvals reaches
`threshold`, the contract executes the action and clears the pending approvals.

`get_pending_admin_actions(action_id) → Vec<Address>`
Returns the list of admin addresses that have already approved the specified
`action_id`. Useful for off-chain dashboards or wallets to display how many
more approvals are needed before the action executes.

Workflow example — enable multisig and execute a protected action:

```bash
# 1. Current single admin enables multisig with 2-of-3 control
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source current-admin-account \
  --network testnet \
  -- initialize_multisig_admin \
  --admin <CURRENT_ADMIN> \
  --admins '["<ADMIN_A>","<ADMIN_B>","<ADMIN_C>"]' \
  --threshold 2

# 2. Any registered admin proposes an action (e.g. set_fee_config)
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source admin-a-account \
  --network testnet \
  -- propose_admin_action \
  --admin <ADMIN_A> \
  --action_id "fee-update-2026-01" \
  --operation set_fee_config \
  --params '{"fee_bps": 300, "treasury": "<TREASURY_ADDR>"}'

# 3. A second admin approves the same action_id
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source admin-b-account \
  --network testnet \
  -- propose_admin_action \
  --admin <ADMIN_B> \
  --action_id "fee-update-2026-01" \
  --operation set_fee_config \
  --params '{"fee_bps": 300, "treasury": "<TREASURY_ADDR>"}'
# Threshold (2) is now met → action executes automatically

# 4. Query pending approvals for an action
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_pending_admin_actions \
  --action_id "fee-update-2026-01"
```

Security notes:

- `initialize_multisig_admin` is irreversible on-chain. To change the admin set
  or threshold, the multisig itself must propose and approve a new
  `initialize_multisig_admin` call.
- The current single admin retains full control until `initialize_multisig_admin`
  is called. After that, every sensitive admin action requires the configured
  threshold of distinct admin signatures.
- `get_pending_admin_actions` is read-only and does not require authorization.

---

Contract Upgrades & Migration
ChainSettle supports in-place WebAssembly (WASM) contract upgrades on Soroban using the contract deployer's `update_current_contract_wasm` interface. In-place upgrades preserve the existing contract ID, token escrow balances, state indices, and historical shipment records.

`upgrade(admin: Address, new_wasm_hash: BytesN<32>)`
Replaces the contract's executing WASM code in-place with the compiled WebAssembly module corresponding to `new_wasm_hash`.

- **Who can call:** Single contract Admin only (`admin.require_auth()` + `admin == stored_admin`).
- **Behavior:** Updates the WASM bytecode in-place and emits a `contract_upgraded` event with `(new_wasm_hash, ledger_sequence)`.
- **Multisig Protection:** If multi-admin governance (`DataKey::MultiAdminConfig`) has been initialized via `initialize_multisig_admin`, single-key `upgrade()` is automatically disabled and panics with `"upgrade multisig is configured; use propose_upgrade/approve_upgrade instead"`.

Multisig Upgrade Governance
When multi-admin governance is enabled, upgrades must be proposed and approved by a threshold of registered admin keys:

- `propose_upgrade(admin: Address, new_wasm_hash: BytesN<32>) → u64` — Any registered multisig admin key can propose an upgrade. The proposer's approval is recorded immediately. Returns a unique `proposal_id`.
- `approve_upgrade(admin: Address, proposal_id: u64)` — Registered admin keys approve a pending proposal. Once the approval count reaches the configured threshold (`config.threshold`), the contract automatically executes `update_current_contract_wasm` and emits `upgrade_executed`.
- `cancel_upgrade(admin: Address, proposal_id: u64)` — Registered admin key cancels a pending proposal. Emits `upgrade_cancelled`.
- `get_upgrade_proposal(proposal_id: u64) → Option<UpgradeProposal>` (read-only) — Queries details and collected approvals for a pending upgrade proposal.

`migrate(env: Env)`
Post-upgrade state migration entrypoint.

- **Who can call:** Public / post-upgrade execution hook.
- **What it does post-upgrade:** Called once immediately after a WASM bytecode upgrade to execute state schema transformations, re-key storage entries (e.g. migrating `V1_*` storage keys to `V2_*` schema), or set new storage defaults.
- **Idempotency:** Designed to be safe to invoke post-upgrade without side-effects when no data model changes are required (currently operates as an idempotent stub for the active contract version).

---

Events
The contract emits the following events (subscribe via Horizon or RPC):

Event name	Payload	When
`shipment_created`	`shipment_id`	New shipment created
`proof_submitted`	`(shipment_id, milestone_index)`	Proof submitted for a milestone
`milestone_confirmed`	`(shipment_id, milestone_index, payment_amount)`	Milestone confirmed, payment released
`dispute_raised`	`(shipment_id, milestone_index)`	Buyer disputes a milestone
`partial_uncontested_released`	`(shipment_id)` topic, `(milestone_index, uncontested_amount, fee_amount)` data	Uncontested portion released immediately at partial dispute time
`dispute_resolved`	`(shipment_id, milestone_index, approved)`	Arbiter resolves dispute
`dispute_escalated`	`(shipment_id)` topic, `(milestone_index, opened_ledger, current_ledger)` data	Dispute open duration surpassed escalation threshold
`dispute_evidence_submitted`	`(shipment_id)` topic, `(milestone_index, caller, evidence_hash, evidence_type)` data	Evidence submitted for a disputed milestone
`escalation_threshold_set`	`threshold_ledgers`	Admin configured escalation threshold
`shipment_cancelled`	`(shipment_id, refund_amount)`	Shipment cancelled
`milestones_rebalanced`	`(buyer, new_percents)`	Buyer rebalanced milestone percentages
`nft_hook_config_updated`	`(admin, enabled, ledger_sequence)`	Admin toggled the NFT mint hook via `set_nft_hook_enabled`
`circuit_breaker_set`	`(limit, window_ledgers)`	Admin configured rate-limiting circuit breaker
`long_hold_rebate_set`	`(threshold_ledgers, rebate_bps)`	Admin configured the long-hold escrow rebate
`nft_mint_hook`	`(shipment_id)` topic, `(buyer, supplier, total_amount, ledger_sequence, metadata_hash)` data	Final milestone completed while the NFT mint hook is enabled
`contract_upgraded`	`(new_wasm_hash, ledger_sequence)`	Single-key WASM upgrade executed
`upgrade_proposed`	`(new_wasm_hash, admin, 1)`	Multisig WASM upgrade proposed
`upgrade_approved`	`(admin, approvals_count)`	Multisig WASM upgrade approved by an admin
`upgrade_cancelled`	`admin`	Multisig WASM upgrade proposal cancelled
`upgrade_executed`	`new_wasm_hash`	Multisig WASM upgrade executed after threshold reached
`arbiter_rotation_proposed`	`(shipment_id)` topic, `new_arbiter` data	Buyer or supplier proposed an arbiter rotation (emitted on every call, even before both parties agree)
`arbiter_rotated`	`(shipment_id)` topic, `new_arbiter` data	Both parties agreed — arbiter replaced with `new_arbiter`
`arbiter_recused`	`(shipment_id)` topic, `(old_arbiter, new_arbiter)` data	Arbiter voluntarily stepped down; replacement auto-assigned from pool

Event name Payload When
`shipment_created` `shipment_id` New shipment created
`proof_submitted` `(shipment_id, milestone_index)` Proof submitted for a milestone
`milestone_confirmed` `(shipment_id, milestone_index, payment_amount)` Milestone confirmed, payment released
`dispute_raised` `(shipment_id, milestone_index)` Buyer disputes a milestone
`partial_uncontested_released` `(shipment_id)` topic, `(milestone_index, uncontested_amount, fee_amount)` data Uncontested portion released immediately at partial dispute time
`dispute_resolved` `(shipment_id, milestone_index, approved)` Arbiter resolves dispute
`dispute_escalated` `(shipment_id)` topic, `(milestone_index, opened_ledger, current_ledger)` data Dispute open duration surpassed escalation threshold
`escalation_threshold_set` `threshold_ledgers` Admin configured escalation threshold
`shipment_cancelled` `(shipment_id, refund_amount)` Shipment cancelled
`milestones_rebalanced` `(buyer, new_percents)` Buyer rebalanced milestone percentages
`nft_hook_config_updated` `(admin, enabled, ledger_sequence)` Admin toggled the NFT mint hook via `set_nft_hook_enabled`
`circuit_breaker_set` `(limit, window_ledgers)` Admin configured rate-limiting circuit breaker
`nft_mint_hook` `(shipment_id)` topic, `(buyer, supplier, total_amount, ledger_sequence, metadata_hash)` data Final milestone completed while the NFT mint hook is enabled
`contract_upgraded` `(new_wasm_hash, ledger_sequence)` Single-key WASM upgrade executed
`upgrade_proposed` `(new_wasm_hash, admin, 1)` Multisig WASM upgrade proposed
`upgrade_approved` `(admin, approvals_count)` Multisig WASM upgrade approved by an admin
`upgrade_cancelled` `admin` Multisig WASM upgrade proposal cancelled
`upgrade_executed` `new_wasm_hash` Multisig WASM upgrade executed after threshold reached
`arbiter_rotation_proposed` `(shipment_id)` topic, `new_arbiter` data Buyer or supplier proposed an arbiter rotation (emitted on every call, even before both parties agree)
`arbiter_rotated` `(shipment_id)` topic, `new_arbiter` data Both parties agreed — arbiter replaced with `new_arbiter`
`arbiter_recused` `(shipment_id)` topic, `(old_arbiter, new_arbiter)` data Arbiter voluntarily stepped down; replacement auto-assigned from pool

The backend service (`chainsetttle-backend`) listens for these events and
sends push notifications to the relevant parties.

---

Error Codes
Code Meaning
1 `ShipmentAlreadyExists` — shipment ID already in use
2 `ShipmentNotFound` — shipment ID not found
3 `Unauthorized` — caller does not have permission
4 `InvalidMilestoneIndex` — index out of range
5 `InvalidMilestoneStatus` — wrong state for this action
6 `ShipmentNotActive` — shipment is completed or cancelled
7 `InvalidPercentages` — milestone percentages don't sum to 100
8 `InvalidAmount` — amount must be > 0
9 `DisputeAlreadyOpen` — dispute already exists for this milestone
18 `MaxShipmentValueExceeded` — `total_amount` exceeds the admin-configured cap
19 `CircuitBreakerTripped` — payment would exceed the rate-limiting outflow cap

---

Project Structure

```
chainsetttle-contract/
├── Cargo.toml                         ← Rust workspace config
├── Cargo.lock
├── .gitignore
├── README.md                          ← this file
└── contracts/
    └── chainsetttle/
        ├── Cargo.toml                 ← contract package config
        ├── Makefile                   ← build / deploy shortcuts
        └── src/
            ├── lib.rs                 ← main contract logic
            ├── test.rs                ← test module orchestrator
            ├── test_common.rs         ← shared test setup, fixtures, helpers
            ├── test_shipment.rs       ← shipment lifecycle tests (create, confirm, cancel)
            ├── test_dispute.rs        ← dispute workflow tests (raise, resolve, cooldown)
            ├── test_admin.rs          ← admin control tests (pause, blacklist, settings)
            ├── test_query.rs          ← read-only query tests (completion %)
            ├── constants.rs           ← contract constants
            ├── storage.rs             ← storage layer
            ├── admin.rs               ← admin functions
            ├── benchmarks.rs          ← performance benchmarks
            └── [other test files]     ← edge cases, stress tests, advanced features
```

Test File Organization
Tests are split by domain for clarity and to reduce merge conflicts:

File	Purpose	Example Tests
`test_common.rs`	Shared setup & utilities	`setup()`, `build_milestones()`, `create_standard_shipment()`
`test_shipment.rs`	Shipment lifecycle	`test_create_shipment_success`, `test_full_shipment_lifecycle`, `test_cancel_shipment`
`test_dispute.rs`	Dispute resolution	`test_raise_dispute`, `test_resolve_dispute`, `test_dispute_cooldown_enforced`
`test_admin.rs`	Admin controls	`test_pause_blocks_create_shipment`, `test_blacklist_removal_restores_participation`
`test_query.rs`	Read-only queries	`test_get_completion_percentage_*` (weight derivation, bps splits, advances, holdback, refunds, top-ups, fees)

File Purpose Example Tests
`test_common.rs` Shared setup & utilities `setup()`, `build_milestones()`, `create_standard_shipment()`
`test_shipment.rs` Shipment lifecycle `test_create_shipment_success`, `test_full_shipment_lifecycle`, `test_cancel_shipment`
`test_dispute.rs` Dispute resolution `test_raise_dispute`, `test_resolve_dispute`, `test_dispute_cooldown_enforced`
`test_admin.rs` Admin controls `test_pause_blocks_create_shipment`, `test_blacklist_removal_restores_participation`
`test_query.rs` Read-only queries `test_get_completion_percentage_*`

All shared fixtures and helper functions are centralized in `test_common.rs` to avoid duplication.

---

Prerequisites
Install the following before you begin:

1. Rust + wasm32 target

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32v1-none
```

Requires Rust v1.84.0 or higher. 2. Stellar CLI

```bash
# macOS
brew install stellar-cli

# Linux / WSL
cargo install --locked stellar-cli --features opt
```

Verify installation:

```bash
stellar --version
```

3. A Stellar testnet account (for deployment)

```bash
stellar keys generate --global my-account --network testnet
stellar keys fund my-account --network testnet
```

---

Setup & Installation

```bash
# Clone the repo
git clone https://github.com/your-org/chainsetttle-contract.git
cd chainsetttle-contract

# Check all dependencies compile
cargo check
```

---

Running Tests

```bash
# Run all unit tests
cargo test

# Run tests with output (useful for debugging)
cargo test -- --nocapture

# Run a specific test
cargo test test_full_shipment_lifecycle

# Run with logs enabled
cargo test --features testutils
```

Expected output:

```
running 7 tests
test test::test_cancel_shipment ... ok
test test::test_create_shipment_success ... ok
test test::test_full_shipment_lifecycle ... ok
test test::test_raise_and_resolve_dispute_approve ... ok
test test::test_raise_and_resolve_dispute_reject ... ok
test test::test_unauthorized_confirm_milestone ... ok
test test::test_create_shipment_invalid_percentages ... ok
```

---

Building

```bash
# Build contract to .wasm
make build
# → target/wasm32v1-none/release/chainsetttle.wasm

# Optimize .wasm for production (smaller size = lower fees)
make optimize
# → target/wasm32v1-none/release/chainsetttle.optimized.wasm
```

Soroban contracts have a 64KB max size. The `optimize` step uses
`stellar contract optimize` to strip unused symbols and shrink the binary.

---

Deploying to Testnet

```bash
# Set your account name (created in Prerequisites step)
export STELLAR_ACCOUNT=my-account

# Deploy (uses optimized .wasm)
make deploy-testnet
```

You'll get back a contract ID like:

```
CXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

Save this — you'll need it to initialize the contract and in your frontend/backend configs.
Initialize after deployment
After deploying, call `init` once to set the admin:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source my-account \
  --network testnet \
  -- init \
  --admin <YOUR_ADDRESS>
```

Create a test shipment via CLI

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source my-account \
  --network testnet \
  -- create_shipment \
  --shipment_id "SHIP-001" \
  --buyer <BUYER_ADDRESS> \
  --supplier <SUPPLIER_ADDRESS> \
  --logistics <LOGISTICS_ADDRESS> \
  --arbiter <ARBITER_ADDRESS> \
  --token <USDC_SAC_ADDRESS> \
  --total_amount 1000000000 \
  --milestones '[{"name":"Dispatch","payment_percent":25,"proof_hash":"","status":"Pending"},{"name":"Transit","payment_percent":50,"proof_hash":"","status":"Pending"},{"name":"Delivered","payment_percent":25,"proof_hash":"","status":"Pending"}]'
```

---

Deploying to Mainnet

> ⚠️ Only deploy to Mainnet after thorough testing and ideally a security audit.

```bash
# Fund a mainnet account first (you need XLM for fees)
stellar contract deploy \
  --wasm target/wasm32v1-none/release/chainsetttle.optimized.wasm \
  --source my-account \
  --network mainnet
```

> **USDC SAC address on Mainnet:**
> `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7EJKEF`

---

Security Considerations
Authorization: Every state-changing function calls `require_auth()` on the relevant party. No one can act on behalf of another address without their signature.
Escrow isolation: Funds are held by the contract address itself, not a separate wallet. The contract can only release funds via the explicit `transfer` calls in `confirm_milestone` and `resolve_dispute`.
Milestone ordering: Milestones can be confirmed in any order. For sequential enforcement (e.g. must confirm dispatch before transit), you would add a check in `submit_proof` that the previous milestone is `Confirmed` — this is left as an optional extension.
Percentage validation: The contract validates that all milestone percentages sum exactly to 100 at shipment creation. Rounding is integer-based — for amounts where `total * percent / 100` doesn't divide evenly, the final milestone may receive a slightly different amount. Consider adjusting percentages accordingly.
TTL / State Archival: Persistent storage entries are given an extended TTL (~1 year) at creation. Long-lived shipments should call `extend_ttl` via the backend before entries archive.
Contract Upgrades: The contract supports in-place WASM code upgrades via Soroban's deployer mechanism (`upgrade` for single-admin mode and `propose_upgrade`/`approve_upgrade` for multi-admin governance). Post-upgrade data migration is handled via the `migrate` entrypoint.
For a detailed threat analysis and security model, see docs/SECURITY.md.

---

Roadmap
[x] Core escrow + milestone logic
[x] Dispute resolution via arbiter
[x] USDC token transfers via Stellar Asset Contract
[x] Full unit test suite
[x] Sequential milestone enforcement (#162)
[x] Multi-token support (XLM, EURC)
[x] Partial cancellation (after some milestones confirmed) (#163)
[x] Contract upgrade mechanism
[ ] Mainnet deployment + verification
[ ] Integration with `chainsetttle-backend` event listener
[ ] Integration with `chainsetttle-frontend` Freighter wallet

---

Contributing
Pull requests welcome. Please run `cargo fmt` and `cargo test` before submitting.
License
MIT


### Shipment Party Reassignment

The contract allows updating the active buyer or supplier on a shipment using `transfer_buyer()` and `transfer_supplier()`.

- **Authorization**: Only the current role holder (or authorized contract admin/super-arbiter, depending on configuration) can initiate the transfer to a new address.
- **In-flight Milestones & Disputes**: Active milestones and ongoing disputes remain tied to the shipment state, but authorization for approving deliverables, withdrawing payouts, or voting on dispute resolutions transfers immediately to the new account.
- **Events Emitted**:
  - `BuyerTransferred(shipment_id, old_buyer, new_buyer)`
  - `SupplierTransferred(shipment_id, old_supplier, new_supplier)`
