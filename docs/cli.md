# Stellar CLI Cookbook

Copy-paste `stellar contract invoke` examples for common operations.
Replace `<CONTRACT_ID>`, `<SOURCE>`, and addresses with your values.
Struct and vector arguments use the `-- <name> '<json>'` encoding shown
below. Source of truth: the `pub fn` signatures in
`contracts/chainsetttle/src/lib.rs`.

Common flags used everywhere:

```bash
CID=<CONTRACT_ID>
SRC=my-account   # stellar keys identity, --source-account equivalent
NET=testnet
```

## Init

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- init --admin <ADMIN_ADDRESS>
```

## Create shipment

Milestones and options are JSON-encoded structs/vectors. Percentages must
sum to 100.

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- create-shipment \
  --shipment-id '"SHIP-001"' \
  --buyers '["<BUYER_ADDRESS>"]' \
  --supplier <SUPPLIER_ADDRESS> \
  --logistics <LOGISTICS_ADDRESS> \
  --arbiter <ARBITER_ADDRESS> \
  --token <TOKEN_ADDRESS> \
  --total-amount 1000000 \
  --milestones '[{"name":"M0","payment_percent":50,"proof_hash":"","status":{"Pending":null},"release_after_ledger":0,"proof_submitted_ledger":null,"dispute_opened_ledger":null,"deadline_ledger":0,"penalty_bps_per_ledger":0},{"name":"M1","payment_percent":50,"proof_hash":"","status":{"Pending":null},"release_after_ledger":0,"proof_submitted_ledger":null,"dispute_opened_ledger":null,"deadline_ledger":0,"penalty_bps_per_ledger":0}]' \
  --options '{"response_deadline":0,"penalty_bps":0,"milestone_mode":{"Parallel":null},"holdback_ledgers":0,"dispute_cooldown_ledgers":0,"late_penalty_bps_per_ledger":0,"auto_confirm_ledgers":0,"dispute_bond_amount":0,"dispute_bond_bps":0,"arbiter_fee_bps":0,"logistics_fee_bps":0,"supplier_collateral":0,"expires_at_ledger":null,"metadata_hash":null,"referrer":null,"buyer_cancel_fee_bps":0,"early_bonus_pool":0,"review_window_ledgers":null,"milestone_splits":[],"deadlines":[],"dispute_timeout_seconds":0,"default_resolution":{"Buyer":null},"backup_arbiter":null,"confirmation_cooldown_ledgers":null,"arbiter_panel":[],"jurisdiction":null,"grace_period_ledgers":0,"quality_grades":[],"milestone_quantities":[],"retainage_bps":0,"warranty_bps":0,"warranty_ledgers":0,"inspector":null,"inspected_milestones":[],"proof_submitters":[],"require_dual_attestation":false,"fund_from_vault":false,"milestone_suppliers":[],"refund_recipient":null}'
```

`u32`/`i128` numbers pass as bare values, `String` as `'"text"'`,
`Address` as the strkey, `Vec` as `[...]` (use `[]` for empty), `Option`
as a value or `null`, and union/enum variants as `{"Variant":null}`.

## Submit proof

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- submit-proof \
  --caller <SUPPLIER_ADDRESS> \
  --shipment-id '"SHIP-001"' \
  --milestone-index 0 \
  --proof-hash '"QmProofHash"' \
  --proof-type ipfs
```

## Confirm milestone

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- confirm-milestone \
  --buyer <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' \
  --milestone-index 0
```

## Dispute and resolve

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- raise-dispute \
  --buyer <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' \
  --milestone-index 0

stellar contract invoke --id $CID --source $SRC --network $NET \
  -- resolve-dispute \
  --arbiter <ARBITER_ADDRESS> \
  --shipment-id '"SHIP-001"' \
  --milestone-index 0 \
  --approve true \
  --reason null
```

`--reason` takes a `ResolutionReason` (`{"ProofValid":null}` etc.) or `null`
to default (`ProofValid` on approve, buyer-side default otherwise).

## Admin config

```bash
# Allow an escrow token
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- add-allowed-token --token <TOKEN_ADDRESS>

# Set the platform fee (basis points) and treasury
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- set-fee-config --admin <ADMIN_ADDRESS> --fee-bps 100 --treasury <TREASURY_ADDRESS>

# Cap total escrowed value for a token (0 = no cap)
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- set-tvl-cap --admin <ADMIN_ADDRESS> --token <TOKEN_ADDRESS> --cap 1000000000
```

Function names map to CLI commands with underscores replaced by dashes
(`confirm_milestone` -> `confirm-milestone`).
