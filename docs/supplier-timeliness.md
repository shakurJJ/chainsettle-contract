# Supplier On-Time Milestone Rate (#631)

Buyers care about delivery timeliness. In addition to the reputation counters
returned by `get_reputation`, the contract tracks how many milestones each
supplier completed before their deadline versus late.

## How it works

- When a milestone with a `deadline_ledger` is confirmed, the supplier's
  timeliness counters are updated: `on_time` when the proof was submitted on
  or before the deadline, `late` otherwise.
- Milestones without a deadline (`deadline_ledger == 0`) are ignored.
- Counters update on confirmation only: `confirm_milestone` (held and direct
  payout paths), `batch_confirm_milestones`, `confirm_milestone_graded`, and
  `claim_auto_confirmation`. Releases (`release_held_payment`) never
  double-count.
- The confirming party is authenticated by the underlying confirmation call
  (`require_auth` on the buyer / delegate / co-buyer path); the confirmation
  already appends a `milestone_confirmed` entry to the shipment audit log.

## Reading the counters

```rust
let t = client.get_supplier_timeliness(&supplier);
```

Returns a `SupplierTimeliness { on_time, late }` struct. A new supplier
starts at `(0, 0)`. The counter entry has its TTL extended on every write
(`extend_ttl(TTL_INITIAL_LEDGERS, TTL_MAX_LEDGERS)`).

## Events

Each recorded milestone emits a `timeliness_recorded` event with topics
`(timeliness_recorded, supplier)` and data `(on_time, late)`.

## Storage

Counters live under `DataKeyExt4::SupplierTimeliness(Address)`, keeping the
50-variant XDR limit of the older `DataKey` enums untouched.
