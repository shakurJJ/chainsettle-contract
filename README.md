# ChainSettle — Contract Repo

[![CI](https://github.com/shakurJJ/chainsettle-contract/actions/workflows/ci.yml/badge.svg)](https://github.com/shakurJJ/chainsettle-contract/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/shakurJJ/chainsettle-contract/branch/main/graph/badge.svg)](https://codecov.io/gh/shakurJJ/chainsettle-contract)

> **Milestone-based supply chain escrow on Stellar Soroban**

ChainSettle is a Soroban smart contract that locks buyer payment in escrow and automatically releases funds to the supplier as each delivery milestone is confirmed on-chain. No middlemen, no delayed wire transfers, no trust required.

This is **Repo 1 of 3** in the ChainSettle project:

| Repo | Description |
|------|-------------|
| `chainsetttle-contract` ← you are here | Soroban smart contract (Rust) |
| `chainsetttle-frontend` | React + Freighter wallet UI |
| `chainsetttle-backend` | Node.js API, notifications, off-chain metadata |

---

## Table of Contents

- [How It Works](#how-it-works)
- [Architecture](#architecture)
- [Data Structures](#data-structures)
- [Contract Functions](#contract-functions)
- [Events](#events)
- [Error Codes](#error-codes)
- [Project Structure](#project-structure)
- [Prerequisites](#prerequisites)
- [Setup & Installation](#setup--installation)
- [Running Tests](#running-tests)
- [Building](#building)
- [Deploying to Testnet](#deploying-to-testnet)
- [Deploying to Mainnet](#deploying-to-mainnet)
- [Security Considerations](#security-considerations)
  - See detailed security model: [docs/SECURITY.md](docs/SECURITY.md)
- [Roadmap](#roadmap)

---

## How It Works

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

## Architecture

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

### Roles

| Role | Address | Permissions |
|------|---------|-------------|
| **Buyer** | Locks USDC, confirms milestones, raises disputes, cancels shipment | Most actions |
| **Supplier** | Submits proof for dispatch and delivery milestones | `submit_proof` only |
| **Logistics** | Submits proof for in-transit milestones | `submit_proof` only |
| **Arbiter** | Resolves disputes — approves or rejects supplier proof | `resolve_dispute` only |
| **Admin** | Contract deployer, set at `init` | Future: upgrade, pause |

---

## Data Structures

### `Milestone`

```rust
pub struct Milestone {
    pub name: String,            // e.g. "Goods Dispatched"
    pub payment_percent: u32,    // 0-100, all milestones must sum to 100
    pub proof_hash: String,      // IPFS CID set by supplier/logistics
    pub status: MilestoneStatus, // Pending | ProofSubmitted | Confirmed | Disputed | Resolved
}
```

### `Shipment`

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
}
```

### `MilestoneStatus` state machine

```
Pending
  └─ submit_proof()  ──→ ProofSubmitted
       ├─ confirm_milestone() ──→ Confirmed  (payment released)
       └─ raise_dispute()     ──→ Disputed
               ├─ resolve_dispute(approve=true)  ──→ Resolved (payment released)
               └─ resolve_dispute(approve=false) ──→ Pending  (supplier resubmits)
```

---

## Contract Functions

All functions require the relevant party to sign the transaction (Soroban auth).

### `init(admin: Address)`
Initialises the contract. Called once by the deployer right after deployment.

### `create_shipment(...) → String`
Creates a new shipment, validates milestone percentages sum to 100,
and transfers `total_amount` USDC from the buyer into escrow.

```
Parameters:
  shipment_id   String    — unique ID for this shipment
  buyer         Address   — funds source + milestone approver
  supplier      Address   — payment recipient
  logistics     Address   — in-transit proof submitter
  arbiter       Address   — dispute resolver
  token         Address   — USDC Stellar Asset Contract address
  total_amount  i128      — total USDC to lock (in stroops)
  milestones    Vec<Milestone> — ordered list, percentages must sum to 100

Returns: shipment_id (same as input, for confirmation)
```

### `submit_proof(caller, shipment_id, milestone_index, proof_hash)`
Supplier or logistics submits an IPFS hash as proof for a milestone.
Milestone must be in `Pending` status. Moves status to `ProofSubmitted`.

### `confirm_milestone(buyer, shipment_id, milestone_index)`
Buyer confirms a `ProofSubmitted` milestone. Automatically calculates
and transfers the milestone's payment percentage to the supplier.
If all milestones are confirmed, shipment status becomes `Completed`.

### `raise_dispute(buyer, shipment_id, milestone_index)`
Buyer disputes a `ProofSubmitted` milestone. Freezes the milestone in
`Disputed` state — no payment can be released until arbiter resolves.

### `resolve_dispute(arbiter, shipment_id, milestone_index, approve: bool)`
Arbiter resolves a `Disputed` milestone.
- `approve = true` → releases payment, status → `Resolved`
- `approve = false` → resets status → `Pending` (supplier must resubmit)

### `cancel_shipment(buyer, shipment_id)`
Cancels the shipment if no milestones have been confirmed yet.
Returns all locked funds to the buyer.

### `get_shipment(shipment_id) → Shipment` *(read-only)*
Returns the full shipment record.

### `get_milestone(shipment_id, milestone_index) → Milestone` *(read-only)*
Returns a single milestone.

### `get_escrow_balance(shipment_id) → i128` *(read-only)*
Returns the amount of USDC still locked in escrow.

---

## Events

The contract emits the following events (subscribe via Horizon or RPC):

| Event name | Payload | When |
|---|---|---|
| `shipment_created` | `shipment_id` | New shipment created |
| `proof_submitted` | `(shipment_id, milestone_index)` | Proof submitted for a milestone |
| `milestone_confirmed` | `(shipment_id, milestone_index, payment_amount)` | Milestone confirmed, payment released |
| `dispute_raised` | `(shipment_id, milestone_index)` | Buyer disputes a milestone |
| `dispute_resolved` | `(shipment_id, milestone_index, approved)` | Arbiter resolves dispute |
| `shipment_cancelled` | `(shipment_id, refund_amount)` | Shipment cancelled |

The backend service (`chainsetttle-backend`) listens for these events and
sends push notifications to the relevant parties.

---

## Error Codes

| Code | Meaning |
|------|---------|
| 1 | `ShipmentAlreadyExists` — shipment ID already in use |
| 2 | `ShipmentNotFound` — shipment ID not found |
| 3 | `Unauthorized` — caller does not have permission |
| 4 | `InvalidMilestoneIndex` — index out of range |
| 5 | `InvalidMilestoneStatus` — wrong state for this action |
| 6 | `ShipmentNotActive` — shipment is completed or cancelled |
| 7 | `InvalidPercentages` — milestone percentages don't sum to 100 |
| 8 | `InvalidAmount` — amount must be > 0 |
| 9 | `DisputeAlreadyOpen` — dispute already exists for this milestone |

---

## Project Structure

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

### Test File Organization

Tests are split by domain for clarity and to reduce merge conflicts:

| File | Purpose | Example Tests |
|------|---------|---|
| `test_common.rs` | Shared setup & utilities | `setup()`, `build_milestones()`, `create_standard_shipment()` |
| `test_shipment.rs` | Shipment lifecycle | `test_create_shipment_success`, `test_full_shipment_lifecycle`, `test_cancel_shipment` |
| `test_dispute.rs` | Dispute resolution | `test_raise_dispute`, `test_resolve_dispute`, `test_dispute_cooldown_enforced` |
| `test_admin.rs` | Admin controls | `test_pause_blocks_create_shipment`, `test_blacklist_removal_restores_participation` |
| `test_query.rs` | Read-only queries | `test_get_completion_percentage_*` |

All shared fixtures and helper functions are centralized in `test_common.rs` to avoid duplication.

---

## Prerequisites

Install the following before you begin:

### 1. Rust + wasm32 target

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32v1-none
```

Requires **Rust v1.84.0 or higher**.

### 2. Stellar CLI

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

### 3. A Stellar testnet account (for deployment)

```bash
stellar keys generate --global my-account --network testnet
stellar keys fund my-account --network testnet
```

---

## Setup & Installation

```bash
# Clone the repo
git clone https://github.com/your-org/chainsetttle-contract.git
cd chainsetttle-contract

# Check all dependencies compile
cargo check
```

---

## Running Tests

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

## Building

```bash
# Build contract to .wasm
make build
# → target/wasm32v1-none/release/chainsetttle.wasm

# Optimize .wasm for production (smaller size = lower fees)
make optimize
# → target/wasm32v1-none/release/chainsetttle.optimized.wasm
```

Soroban contracts have a **64KB max size**. The `optimize` step uses
`stellar contract optimize` to strip unused symbols and shrink the binary.

---

## Deploying to Testnet

```bash
# Set your account name (created in Prerequisites step)
export STELLAR_ACCOUNT=my-account

# Deploy (uses optimized .wasm)
make deploy-testnet
```

You'll get back a **contract ID** like:
```
CXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

Save this — you'll need it to initialize the contract and in your frontend/backend configs.

### Initialize after deployment

After deploying, call `init` once to set the admin:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source my-account \
  --network testnet \
  -- init \
  --admin <YOUR_ADDRESS>
```

### Create a test shipment via CLI

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

## Deploying to Mainnet

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

## Security Considerations

- **Authorization**: Every state-changing function calls `require_auth()` on the relevant party. No one can act on behalf of another address without their signature.
- **Escrow isolation**: Funds are held by the contract address itself, not a separate wallet. The contract can only release funds via the explicit `transfer` calls in `confirm_milestone` and `resolve_dispute`.
- **Milestone ordering**: Milestones can be confirmed in any order. For sequential enforcement (e.g. must confirm dispatch before transit), you would add a check in `submit_proof` that the previous milestone is `Confirmed` — this is left as an optional extension.
- **Percentage validation**: The contract validates that all milestone percentages sum exactly to 100 at shipment creation. Rounding is integer-based — for amounts where `total * percent / 100` doesn't divide evenly, the final milestone may receive a slightly different amount. Consider adjusting percentages accordingly.
- **TTL / State Archival**: Persistent storage entries are given an extended TTL (~1 year) at creation. Long-lived shipments should call `extend_ttl` via the backend before entries archive.
- **No upgradability (MVP)**: This scaffold has no upgrade mechanism. For production, consider implementing Soroban's `upgrade` pattern.

For a detailed threat analysis and security model, see [docs/SECURITY.md](docs/SECURITY.md).


---

## Roadmap

- [x] Core escrow + milestone logic
- [x] Dispute resolution via arbiter
- [x] USDC token transfers via Stellar Asset Contract
- [x] Full unit test suite
- [ ] Sequential milestone enforcement (optional)
- [ ] Multi-token support (XLM, EURC)
- [ ] Partial cancellation (after some milestones confirmed)
- [ ] Contract upgrade mechanism
- [ ] Mainnet deployment + verification
- [ ] Integration with `chainsetttle-backend` event listener
- [ ] Integration with `chainsetttle-frontend` Freighter wallet

---

## Contributing

Pull requests welcome. Please run `cargo fmt` and `cargo test` before submitting.

## License

MIT
