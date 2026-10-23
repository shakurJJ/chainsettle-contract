# Testing Guide

How the `chainsetttle` test suite is organized and how to add new tests.
Source of truth: `contracts/chainsetttle/src/lib.rs` and the
`contracts/chainsetttle/src/test_*.rs` modules.

## Module layout and registration

- Unit tests live next to the contract in `contracts/chainsetttle/src/`,
  one file per feature: `test_<feature>.rs` (for example
  `test_supplier_timeliness.rs`, `test_buyer_vault.rs`,
  `test_standing_orders.rs`).
- Shared fixtures live in `test_common.rs`.
- Every test module must be registered in `lib.rs` with a `mod` declaration
  alongside the other `mod test_*;` lines near the bottom of the file, e.g.
  `mod test_supplier_timeliness;`.
- Each test file starts with `#![cfg(test)]` and `extern crate std;`, then
  `use super::*;` plus the fixtures it needs from `crate::test_common`.
- Snapshot tests live outside the unit tree in `contracts/chainsetttle/tests/`
  (`test_snapshots.rs`, `test_dispute_snapshots.rs`, `test_ttl_regression.rs`)
  with expected outputs under `tests/snapshots/`.

## Shared fixtures (`test_common.rs`)

- `setup()` builds a `TestSetup { env, contract_id, token_id, buyer, buyer2,
  supplier, logistics, arbiter, treasury }`: a fresh `Env` with
  `mock_all_auths`, a registered contract, a Stellar asset token with funds
  minted to `buyer`/`buyer2`, and `init` already called.
- `build_milestones(&env)` returns the standard 3-milestone vector
  (25/50/25 percent, no deadlines).
- `single_buyer_vec(&env, &buyer)` wraps one buyer for `create_shipment`.
- `default_options(&env)` returns neutral `ShipmentOptions` (parallel mode,
  no holdback, no penalties, no cooldown).
- `create_standard_shipment(...)` creates a funded 1_000_000-unit shipment.
- `create_shipment_with_options(...)` is the same but takes caller-supplied
  `ShipmentOptions`.
- `submit_proof_for_milestone(&setup, &shipment_id, index)` submits a proof
  as the setup supplier.
- `try_submit_proof` / `try_correct_proof` return errors instead of panicking,
  for authorization-matrix tests.
- `capture_events(&env, f)` runs exactly one contract call `f` and returns
  its events; `has_event_topic(&env, &events, "topic")` checks the first
  topic. Use `Symbol::new(&env, ...)` names matching the contract's
  `env.events().publish` topics.

## Snapshot tests

- Snapshot tests render contract outputs (summaries, events) to files under
  `tests/snapshots/` and fail on mismatch.
- Regenerate after an intentional change with
  `UPDATE_SNAPSHOTS=1 cargo test` (run from the crate directory or workspace
  root), then review the git diff of `tests/snapshots/` before committing.

## Property tests

- `property_tests.rs` (and the proptest block in `storage.rs`) use the
  `proptest` crate to fuzz inputs such as milestone percentage splits
  (`prop_contract_accepts_valid_two_split`,
  `prop_contract_rejects_below_min_percent`).
- Run them with the normal `cargo test`; increase cases via
  `proptest::test_runner::Config` in the `proptest!` block when debugging.

## Pitfall: `env.events().all()` only covers the last invocation

`env.events().all()` reports events from the most recent top-level contract
invocation only. Reading it after several calls silently drops the earlier
calls' events. Always wrap exactly one client call in
`capture_events(&t.env, || { client.confirm_milestone(...); })` and inspect
the returned vector; with zero calls you get the previous call's events, with
several you get only the last call's.

## Adding a new test module

1. Create `contracts/chainsetttle/src/test_<feature>.rs` following the
   header pattern above.
2. Register `mod test_<feature>;` in `lib.rs`.
3. Cover the happy path, authorization failures (`#[should_panic(expected =
   "...")]` with the contract's exact panic string), and edge cases
   (zero/empty inputs, boundary ledgers).
4. Verify with `cargo test <module_name> -- --nocapture` and then the full
   `cargo test`.

## Commands

```bash
cargo test                          # all tests
cargo test <name> -- --nocapture    # one test, with output
UPDATE_SNAPSHOTS=1 cargo test       # regenerate snapshots
```
