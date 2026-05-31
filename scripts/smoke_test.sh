#!/usr/bin/env bash
# Smoke test: create_shipment → submit_proof → confirm_milestone
set -euo pipefail

CONTRACT_ID="${STAGING_CONTRACT_ID}"
NETWORK="testnet"
SOURCE="staging-key"
SHIP_ID="SMOKE-$(date +%s)"

echo "==> [1/3] create_shipment"
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- create_shipment \
  --shipment_id "$SHIP_ID" \
  --buyers "[\"$(stellar keys address staging-key)\"]" \
  --supplier "$SUPPLIER_ADDRESS" \
  --logistics "$LOGISTICS_ADDRESS" \
  --arbiter "$ARBITER_ADDRESS" \
  --token "$USDC_SAC" \
  --total_amount 100000000 \
  --milestones '[{"name":"Dispatch","payment_percent":100,"proof_hash":"","status":"Pending","release_after_ledger":0,"proof_submitted_ledger":null,"dispute_opened_ledger":null}]' \
  --options '{"response_deadline":0,"penalty_bps":0,"milestone_mode":"Parallel","holdback_ledgers":0,"dispute_cooldown_ledgers":0,"late_penalty_bps_per_ledger":0,"auto_confirm_ledgers":0}'

echo "==> [2/3] submit_proof"
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- submit_proof \
  --caller "$SUPPLIER_ADDRESS" \
  --shipment_id "$SHIP_ID" \
  --milestone_index 0 \
  --proof_hash "bafybeismoke000000000000000000000000000000000000000000000000"

echo "==> [3/3] confirm_milestone"
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- confirm_milestone \
  --buyer "$(stellar keys address staging-key)" \
  --shipment_id "$SHIP_ID" \
  --milestone_index 0

echo "✅ Smoke tests passed for shipment $SHIP_ID"
