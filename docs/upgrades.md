# Upgrade and Migration Runbook

This runbook covers how to build and upload a new ChainSettle WASM artifact, execute a contract code upgrade under either single-admin or M-of-N multisig governance, run post-upgrade storage schema and paged shipment migrations, and verify contract state before and after an upgrade.

All entry points referenced below live in [`contracts/chainsetttle/src/lib.rs`](../contracts/chainsetttle/src/lib.rs), [`contracts/chainsetttle/src/paged_migration.rs`](../contracts/chainsetttle/src/paged_migration.rs), and [`contracts/chainsetttle/src/storage.rs`](../contracts/chainsetttle/src/storage.rs).

---

## Overview of Entry Points

| Function | Caller / Auth | Purpose |
| --- | --- | --- |
| `version(env) -> (u32, u32, u32)` | Anyone (read-only) | Returns the compiled contract version `(VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH)` (currently `(0, 1, 0)`). |
| `storage_schema_version(env) -> u32` | Anyone (read-only) | Returns `DataKeyExt4::StorageSchemaVersion` (`0` prior to `migrate`, `STORAGE_SCHEMA_VERSION = 1` after `migrate`). |
| `upgrade(env, admin, new_wasm_hash)` | Primary `Admin` (only when multisig is **not** configured) | Immediately swaps the contract WASM via `env.deployer().update_current_contract_wasm(new_wasm_hash)`. |
| `propose_upgrade(env, admin, new_wasm_hash) -> u64` | Any member of `MultiAdminConfig.admins` | Creates a persistent `UpgradeProposal` and records the proposer's approval (executes immediately if `threshold <= 1`). |
| `approve_upgrade(env, admin, proposal_id)` | Any member of `MultiAdminConfig.admins` | Adds an approval; executes the WASM swap automatically once `approvals.len() >= threshold`. |
| `cancel_upgrade(env, admin, proposal_id)` | Any member of `MultiAdminConfig.admins` (or primary `Admin`) | Cancels and deletes a pending `UpgradeProposal` before threshold is met. |
| `get_upgrade_proposal(env, proposal_id) -> Option<UpgradeProposal>` | Anyone (read-only) | Inspects a pending multisig upgrade proposal (`id`, `wasm_hash`, `approvals`, `proposed_at_ledger`). |
| `migrate(env, admin)` | Primary `Admin` | Advances `DataKeyExt4::StorageSchemaVersion` to `STORAGE_SCHEMA_VERSION` (`1`). Idempotency guard rejects repeat runs at the same version. |
| `start_migration(env, admin, target_version)` | Primary `Admin` | Initializes cursor-based paged V1 → V2 shipment migration state (`MigrationKey::MigrationState`). |
| `migrate_chunk(env, caller, batch_size) -> (u32, bool)` | Any authenticated `caller` | Migrates up to `batch_size` (`1..=50`) shipments from `V1Shipment` to V2 `Shipment`. Returns `(migrated_in_chunk, is_complete)`. |
| `get_migration_progress(env) -> (u32, u32, bool)` | Anyone (read-only) | Returns `(migrated_count, total_shipments, is_complete)`. |
| `migration_exists(env) -> bool` | Anyone (read-only) | Returns `true` if a `MigrationState` record is currently stored. |
| `abort_migration(env, admin)` | Primary `Admin` | Halts an in-progress paged migration (`in_progress = false`) so it can be restarted if needed. |
| `cleanup_migration(env, admin)` | Primary `Admin` | Removes `MigrationKey::MigrationState` once a paged migration is complete. |

---

## 1. Pre-Upgrade Checklist

Before swapping WASM on testnet or mainnet, record the baseline state of the deployed contract and build the new WASM binary:

1. **Build and optimize the release WASM:**
   ```bash
   cargo build --manifest-path contracts/chainsetttle/Cargo.toml \
     --target wasm32-unknown-unknown --release
   ```
2. **Upload (`install`) the new WASM to the network** to obtain its 32-byte hash (`<NEW_WASM_HASH>`):
   ```bash
   stellar contract install \
     --network <NETWORK> \
     --source <DEPLOYER_SECRET_OR_IDENTITY> \
     --wasm target/wasm32-unknown-unknown/release/chainsetttle.wasm
   ```
   Save the 64-character hex string output by `stellar contract install` as `NEW_WASM_HASH`.
3. **Snapshot current contract state:**
   ```bash
   # Code version tuple (major, minor, patch)
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- version

   # On-chain storage schema version (0 if migrate has never run, 1 after v1 migration)
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- storage_schema_version

   # Global counters (total_shipments, completed_count, disputed_count, total_volume_escrowed)
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_contract_stats

   # Governance mode (None = single-admin, Some(MultiAdminConfig) = M-of-N multisig)
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_multi_admin_config

   # Pause / freeze status
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- is_paused
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- is_emergency_frozen
   ```
4. **(Optional) Pause state-changing operations during the maintenance window:**
   If the upgrade includes a storage migration (`migrate` or paged V1 → V2 shipment migration), pause the contract first so no new shipments or milestone transitions interleave with the upgrade:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network $NETWORK --source $ADMIN \
     -- pause --admin $ADMIN_ADDRESS
   ```

---

## 2. Executing the Code Upgrade

ChainSettle supports two mutually exclusive upgrade paths depending on whether `initialize_multisig_admin` has been called on the contract.

### Path A: Single-Admin Upgrade (`upgrade`)

When `DataKey::MultiAdminConfig` is **not** set in instance storage, the primary `Admin` upgrades the contract in a single transaction:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  --source $ADMIN \
  -- upgrade \
  --admin $ADMIN_ADDRESS \
  --new_wasm_hash $NEW_WASM_HASH
```

**What happens on-chain:**
- Verifies `admin.require_auth()` and `assert_admin(&env, &admin)`.
- Verifies that `DataKey::MultiAdminConfig` does **not** exist in instance storage.
  > **Important:** Once `initialize_multisig_admin` has been configured, `upgrade` is permanently disabled and panics with `"upgrade multisig is configured; use propose_upgrade/approve_upgrade instead"`.
- Calls `env.deployer().update_current_contract_wasm(new_wasm_hash)`.
- Appends an `"upgrade"` entry to the admin audit log (`DataKey::AdminActionLog`) and emits the `contract_upgraded` event with payload `(admin, new_wasm_hash, ledger_sequence)`.

---

### Path B: M-of-N Multisig Upgrade (`propose_upgrade` / `approve_upgrade` / `cancel_upgrade`)

When `MultiAdminConfig` is configured (`admins: Vec<Address>`, `threshold: u32`), any WASM upgrade must collect `threshold` distinct approvals from members of `MultiAdminConfig.admins`.

#### Step 1 — Propose the upgrade (`propose_upgrade`)

Any address in `MultiAdminConfig.admins` submits the target WASM hash:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  --source $SIGNER_1 \
  -- propose_upgrade \
  --admin $SIGNER_1_ADDRESS \
  --new_wasm_hash $NEW_WASM_HASH
```

- Assigns a monotonically increasing `proposal_id: u64` (`DataKeyExt2::NextUpgradeProposalId`, starting at `1`).
- Stores `UpgradeProposal` under `DataKeyExt2::UpgradeProposal(proposal_id)` in persistent storage with `SIGNER_1_ADDRESS` pre-recorded as the first approval:
  ```rust
  pub struct UpgradeProposal {
      pub id: u64,
      pub wasm_hash: BytesN<32>,
      pub approvals: Vec<Address>,
      pub proposed_at_ledger: u32,
  }
  ```
- Emits `upgrade_proposed` with `(admin, new_wasm_hash)`.
- **Single-threshold fast path:** If `config.threshold <= 1`, `propose_upgrade` immediately removes the proposal from persistent storage, calls `update_current_contract_wasm`, and emits `contract_upgraded`.

#### Step 2 — Inspect the pending proposal (`get_upgrade_proposal`)

Other multisig signers should verify the `wasm_hash` before approving:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  -- get_upgrade_proposal \
  --proposal_id 1
```

#### Step 3 — Collect remaining approvals (`approve_upgrade`)

Each additional signer calls `approve_upgrade` with the `proposal_id`:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  --source $SIGNER_2 \
  -- approve_upgrade \
  --admin $SIGNER_2_ADDRESS \
  --proposal_id 1
```

- Panics with `"caller is not in the multisig admin set"` if `admin` is not a member of `MultiAdminConfig.admins`.
- Panics with `"admin has already approved this proposal"` if the same signer attempts to approve twice.
- Emits `upgrade_approved` with `(admin, proposal_id)`.
- As soon as `proposal.approvals.len() >= config.threshold`:
  1. Removes `DataKeyExt2::UpgradeProposal(proposal_id)` from persistent storage.
  2. Executes `env.deployer().update_current_contract_wasm(proposal.wasm_hash)`.
  3. Emits `contract_upgraded` with `(admin, proposal.wasm_hash, ledger_sequence)`.

#### Cancelling a pending proposal (`cancel_upgrade`)

If a wrong WASM hash was proposed or a vulnerability is discovered before threshold is reached, any member of `MultiAdminConfig.admins` can cancel and delete the proposal:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  --source $SIGNER_1 \
  -- cancel_upgrade \
  --admin $SIGNER_1_ADDRESS \
  --proposal_id 1
```

This deletes `DataKeyExt2::UpgradeProposal(proposal_id)` and emits `upgrade_cancelled` with `(admin, proposal_id)`.

---

## 3. Post-Upgrade Schema & State Migrations

Swapping the WASM binary updates contract code immediately on the next ledger, but existing ledger entries remain in their previous format until migration entry points are invoked.

### 3.1 Instance Schema Version Migration (`migrate`)

After upgrading to a release that bumps `STORAGE_SCHEMA_VERSION` (defined in [`src/constants.rs`](../contracts/chainsetttle/src/constants.rs), currently `1`), run `migrate`:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  --source $ADMIN \
  -- migrate \
  --admin $ADMIN_ADDRESS
```

- Requires primary `Admin` authorization (`assert_admin`).
- Reads `DataKeyExt4::StorageSchemaVersion` from instance storage (defaults to `0` if unset).
- If `current >= STORAGE_SCHEMA_VERSION`, panics with `"already at current schema version"` so a migration is never accidentally applied twice.
- Runs per-version instance migrations (`0 -> 1`), sets `DataKeyExt4::StorageSchemaVersion = STORAGE_SCHEMA_VERSION`, extends instance TTL, records `"migrate"` in the admin audit log, and emits `storage_migrated` with `(from_version, to_version, admin)`.

### 3.2 Cursor-Based Paged Shipment Migration (`start_migration` / `migrate_chunk`)

When upgrading from V1 shipment records (`storage::DataKey::V1Shipment(id)`) to V2 `Shipment` structs (`DataKey::Shipment(id)` with `grace_period_ledgers` and `supplier_payout_address`), migrating all shipments in a single transaction can exceed Soroban per-transaction CPU/ledger-footprint limits. ChainSettle provides a cursor-based paged migration in [`src/paged_migration.rs`](../contracts/chainsetttle/src/paged_migration.rs):

1. **Start the migration (Primary `Admin`):**
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network $NETWORK --source $ADMIN \
     -- start_migration \
     --admin $ADMIN_ADDRESS \
     --target_version 2
   ```
   Initializes `MigrationState` at `MigrationKey::MigrationState` with `current_cursor = 0`, `total_shipments = AllShipments.len()`, `migrated_count = 0`, and `in_progress = true`.

2. **Process chunks in a loop (Permissionless — any authenticated caller):**
   `batch_size` must be between `1` and `MAX_MIGRATION_BATCH_SIZE` (`50`).
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network $NETWORK --source $OPERATOR \
     -- migrate_chunk \
     --caller $OPERATOR_ADDRESS \
     --batch_size 50
   ```
   Each call converts up to `batch_size` shipments from `V1Shipment(id)` to V2 `Shipment(id)` via `storage::migrate_v1_to_v2` (defaulting `grace_period_ledgers: 0` and `supplier_payout_address: None`), removes the legacy `V1Shipment(id)` entry, advances `current_cursor`, and returns `(migrated_in_this_chunk, is_complete)`.

3. **Monitor progress (`get_migration_progress`):**
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network $NETWORK \
     -- get_migration_progress
   ```
   Returns `(migrated_count, total_shipments, is_complete)`. Repeat `migrate_chunk` until `is_complete` is `true`.

4. **Clean up migration state (`cleanup_migration`):**
   Once `is_complete == true`, remove the temporary `MigrationState` record:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network $NETWORK --source $ADMIN \
     -- cleanup_migration \
     --admin $ADMIN_ADDRESS
   ```
   *(If a migration needs to be halted mid-run, the admin can call `abort_migration(admin)` to set `in_progress = false`, which allows `start_migration` to be called again.)*

---

## 4. Post-Upgrade Verification Checklist

Run the following checks before unpausing the contract:

1. **Confirm code and schema versions:**
   ```bash
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- version
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- storage_schema_version
   ```
   Verify `version` matches the new release tuple and `storage_schema_version` equals `STORAGE_SCHEMA_VERSION` (`1`).
2. **Confirm global counters and configuration are intact:**
   ```bash
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_contract_stats
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_multi_admin_config
   ```
   Compare `get_contract_stats` against the pre-upgrade snapshot.
3. **Spot-check active shipments and escrow balances:**
   ```bash
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK \
     -- get_shipment --shipment_id 1
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK \
     -- get_escrow_balance --shipment_id 1
   ```
4. **Unpause the contract (if paused in Step 1.4):**
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network $NETWORK --source $ADMIN \
     -- unpause --admin $ADMIN_ADDRESS
   ```

---

## 5. Events Emitted During Upgrades & Migrations

| Event Topic | Emitted By | Payload |
| --- | --- | --- |
| `contract_upgraded` | `upgrade`, `propose_upgrade` (if `threshold <= 1`), `approve_upgrade` | `(admin: Address, new_wasm_hash: BytesN<32>, ledger: u32)` |
| `upgrade_proposed` | `propose_upgrade` | `(admin: Address, new_wasm_hash: BytesN<32>)` |
| `upgrade_approved` | `approve_upgrade` | `(admin: Address, proposal_id: u64)` |
| `upgrade_cancelled` | `cancel_upgrade` | `(admin: Address, proposal_id: u64)` |
| `storage_migrated` | `migrate` | `(from_version: u32, to_version: u32, admin: Address)` |
| `("chainsettle", "migration_started")` | `start_migration` | `(target_version: u32, total_shipments: u32)` |
| `("chainsettle", "migration_chunk")` | `migrate_chunk` | `(cursor: u32, processed: u32, total_migrated: u32)` |
| `("chainsettle", "migration_completed")` | `migrate_chunk` (final chunk) | `(total_migrated: u32, completed_at_ledger: u32)` |
| `("chainsettle", "migration_aborted")` | `abort_migration` | `migrated_count: u32` |
| `("chainsettle", "migration_cleaned")` | `cleanup_migration` | `()` |
