# ChainSettle — Backend Integration Guide (Node.js)

This guide explains how to set up a Node.js event listener for ChainSettle
Soroban contract events, parse each event type, and trigger downstream
notifications or database updates.

Related repos:
- Contract (you are here): `chainsetttle-contract`
- Backend: [`chainsetttle-backend`](https://github.com/shakurJJ/chainsetttle-backend)
- Frontend: [`chainsetttle-frontend`](https://github.com/shakurJJ/chainsetttle-frontend)

---

## Table of Contents

1. [Overview](#overview)
2. [Prerequisites](#prerequisites)
3. [Environment Variables](#environment-variables)
4. [Subscribing to Contract Events via Horizon](#subscribing-to-contract-events-via-horizon)
5. [Parsing Event Payloads](#parsing-event-payloads)
6. [Retry and Reconnect Strategy](#retry-and-reconnect-strategy)
7. [Complete Runnable Example](#complete-runnable-example)

---

## Overview

ChainSettle events are native **Soroban contract events** written to the
Stellar ledger. The canonical way to stream them is via the **Horizon**
`/events` endpoint (available from Horizon v2.26+), which supports
server-sent events (SSE) and cursor-based pagination.

```
Soroban contract  →  Stellar ledger  →  Horizon /events SSE  →  Node.js listener
```

The listener should:

1. Open a long-lived SSE connection to Horizon's `/events` endpoint filtered
   by the contract address.
2. Decode each `ContractEvent` from XDR.
3. Route on the event topic symbol (`shipment_created`, `milestone_confirmed`,
   `shipment_cancelled`, …).
4. Persist the event data and trigger notifications (email, webhook, push).

---

## Prerequisites

```bash
npm install @stellar/stellar-sdk eventsource
```

| Package | Purpose |
|---------|---------|
| `@stellar/stellar-sdk` | XDR decoding, address helpers |
| `eventsource` | Native SSE client for Node.js |

---

## Environment Variables

Create a `.env` file (never commit it):

```dotenv
# Stellar network: "testnet" or "mainnet"
STELLAR_NETWORK=testnet

# Horizon base URL
HORIZON_URL=https://horizon-testnet.stellar.org

# Soroban RPC URL (used for simulation, not streaming)
SOROBAN_RPC_URL=https://soroban-testnet.stellar.org

# ChainSettle contract ID on the target network
CONTRACT_ID=CAAAA...YOUR_CONTRACT_ID_HERE

# Starting cursor; set to "now" to only receive future events or a specific
# paging_token from a previous run to replay from a checkpoint.
EVENT_CURSOR=now

# Number of events to fetch per Horizon page (max 200)
EVENT_LIMIT=100
```

---

## Subscribing to Contract Events via Horizon

Horizon's `/events` endpoint filters events by contract address and streams
them as newline-delimited JSON.

```typescript
import EventSource from "eventsource";
import * as StellarSdk from "@stellar/stellar-sdk";

const HORIZON_URL   = process.env.HORIZON_URL!;
const CONTRACT_ID   = process.env.CONTRACT_ID!;
const EVENT_CURSOR  = process.env.EVENT_CURSOR ?? "now";
const EVENT_LIMIT   = process.env.EVENT_LIMIT  ?? "100";

function buildHorizonEventsUrl(): string {
  const url = new URL(`${HORIZON_URL}/events`);
  url.searchParams.set("filter[contract_ids]", CONTRACT_ID);
  url.searchParams.set("cursor",               EVENT_CURSOR);
  url.searchParams.set("limit",                EVENT_LIMIT);
  return url.toString();
}

function openEventStream(
  onEvent: (raw: HorizonEventRecord) => void,
  onError: (err: Event) => void
): EventSource {
  const es = new EventSource(buildHorizonEventsUrl());
  es.onmessage = (msg) => {
    if (msg.data === '"hello"' || msg.data === '"byebye"') return;
    onEvent(JSON.parse(msg.data) as HorizonEventRecord);
  };
  es.onerror = onError;
  return es;
}
```

---

## Parsing Event Payloads

### Type Definitions

```typescript
/** Raw Horizon event record returned over SSE. */
interface HorizonEventRecord {
  id:            string;            // paging_token — use as cursor checkpoint
  paging_token:  string;
  type:          "contract";
  ledger:        number;
  ledger_closed_at: string;         // ISO-8601
  contract_id:   string;
  topic:         string[];          // XDR-encoded topic values (base64)
  value:         string;            // XDR-encoded data value (base64)
}

// Decoded event topic[0] is always the event name Symbol.
// Decoded event topic[1] is the shipment_id String (for shipment events).

interface ShipmentCreatedData {
  buyer:        string;   // Stellar address (G… or C…)
  supplier:     string;
  logistics:    string;
  arbiter:      string;
  token:        string;
  total_amount: bigint;
  ledger:       number;
}

interface MilestoneConfirmedData {
  milestone_index: number;
  payment:         bigint;
  fee_amount:      bigint;
  penalty_deducted: bigint;
  supplier:        string;
  ledger:          number;
}

interface ShipmentCancelledData {
  refunded_amount: bigint;
  cancelled_by:    string;
  ledger:          number;
}
```

### Decoding XDR Topics and Data

```typescript
import { xdr, StrKey } from "@stellar/stellar-sdk";

function decodeSymbol(base64: string): string {
  const scVal = xdr.ScVal.fromXDR(base64, "base64");
  return scVal.sym().toString();
}

function decodeString(base64: string): string {
  const scVal = xdr.ScVal.fromXDR(base64, "base64");
  return Buffer.from(scVal.str()).toString("utf-8");
}

function decodeAddress(base64: string): string {
  const scVal = xdr.ScVal.fromXDR(base64, "base64");
  const addrObj = scVal.address();
  if (addrObj.switch() === xdr.ScAddressType.scAddressTypeAccount()) {
    return StrKey.encodeEd25519PublicKey(
      addrObj.accountId().ed25519()
    );
  }
  return StrKey.encodeContract(addrObj.contractId());
}

function decodeTupleVec(base64: string): xdr.ScVal[] {
  const scVal = xdr.ScVal.fromXDR(base64, "base64");
  return scVal.vec()!;
}

function decodeI128(scv: xdr.ScVal): bigint {
  const i128 = scv.i128();
  const hi = BigInt(i128.hi().toString());
  const lo = BigInt(i128.lo().toString());
  return (hi << 64n) | lo;
}
```

### Event Router

```typescript
async function handleEvent(raw: HorizonEventRecord): Promise<void> {
  const [topicName, topicShipmentId] = raw.topic;
  const eventName  = decodeSymbol(topicName);
  const shipmentId = decodeString(topicShipmentId);

  switch (eventName) {
    case "shipment_created":
      await onShipmentCreated(shipmentId, raw);
      break;
    case "milestone_confirmed":
      await onMilestoneConfirmed(shipmentId, raw);
      break;
    case "shipment_cancelled":
      await onShipmentCancelled(shipmentId, raw);
      break;
    case "dispute_raised":
      await onDisputeRaised(shipmentId, raw);
      break;
    case "dispute_resolved":
      await onDisputeResolved(shipmentId, raw);
      break;
    default:
      console.log(`[ChainSettle] unhandled event: ${eventName}`);
  }
}

async function onShipmentCreated(
  shipmentId: string,
  raw: HorizonEventRecord
): Promise<void> {
  // Data tuple: (buyer, supplier, logistics, arbiter, token, total_amount, ledger)
  const vals = decodeTupleVec(raw.value);
  const data: ShipmentCreatedData = {
    buyer:        decodeAddress(vals[0].toXDR("base64")),
    supplier:     decodeAddress(vals[1].toXDR("base64")),
    logistics:    decodeAddress(vals[2].toXDR("base64")),
    arbiter:      decodeAddress(vals[3].toXDR("base64")),
    token:        decodeAddress(vals[4].toXDR("base64")),
    total_amount: decodeI128(vals[5]),
    ledger:       Number(vals[6].u32()),
  };
  console.log("[shipment_created]", shipmentId, data);
  // TODO: persist to DB, send buyer/supplier confirmation email/notification
}

async function onMilestoneConfirmed(
  shipmentId: string,
  raw: HorizonEventRecord
): Promise<void> {
  // Data tuple: (milestone_index, payment, fee_amount, penalty_deducted, supplier, ledger)
  const vals = decodeTupleVec(raw.value);
  const data: MilestoneConfirmedData = {
    milestone_index:  Number(vals[0].u32()),
    payment:          decodeI128(vals[1]),
    fee_amount:       decodeI128(vals[2]),
    penalty_deducted: decodeI128(vals[3]),
    supplier:         decodeAddress(vals[4].toXDR("base64")),
    ledger:           Number(vals[5].u32()),
  };
  console.log("[milestone_confirmed]", shipmentId, data);
  // TODO: notify supplier of payment release, update shipment progress in DB
}

async function onShipmentCancelled(
  shipmentId: string,
  raw: HorizonEventRecord
): Promise<void> {
  // Data tuple: (refunded_amount, cancelled_by, ledger)
  const vals = decodeTupleVec(raw.value);
  const data: ShipmentCancelledData = {
    refunded_amount: decodeI128(vals[0]),
    cancelled_by:    decodeAddress(vals[1].toXDR("base64")),
    ledger:          Number(vals[2].u32()),
  };
  console.log("[shipment_cancelled]", shipmentId, data);
  // TODO: mark shipment cancelled in DB, notify supplier of cancellation
}

async function onDisputeRaised(
  shipmentId: string,
  raw: HorizonEventRecord
): Promise<void> {
  const milestoneIndex = Number(
    xdr.ScVal.fromXDR(raw.value, "base64").u32()
  );
  console.log("[dispute_raised]", shipmentId, { milestoneIndex });
  // TODO: alert arbiter, flag shipment as disputed in DB
}

async function onDisputeResolved(
  shipmentId: string,
  raw: HorizonEventRecord
): Promise<void> {
  const vals    = decodeTupleVec(raw.value);
  const milestoneIndex = Number(vals[0].u32());
  const approved       = vals[1].bool();
  console.log("[dispute_resolved]", shipmentId, { milestoneIndex, approved });
  // TODO: update dispute state in DB, notify parties
}
```

---

## Retry and Reconnect Strategy

Soroban event streams may drop under network instability. Implement
exponential back-off with jitter and checkpoint the last seen `paging_token`
so you can resume without replaying from the beginning.

```typescript
const MAX_RETRY_MS   = 30_000;
const BASE_DELAY_MS  = 1_000;

let lastCursor = process.env.EVENT_CURSOR ?? "now";
let retries    = 0;
let es: EventSource | null = null;

function connect(): void {
  if (es) {
    es.close();
  }

  // Always rebuild the URL with the latest checkpoint cursor.
  const url = new URL(`${HORIZON_URL}/events`);
  url.searchParams.set("filter[contract_ids]", CONTRACT_ID);
  url.searchParams.set("cursor",               lastCursor);
  url.searchParams.set("limit",                EVENT_LIMIT);

  es = new EventSource(url.toString());

  es.onmessage = async (msg) => {
    if (msg.data === '"hello"' || msg.data === '"byebye"') return;

    const raw = JSON.parse(msg.data) as HorizonEventRecord;

    try {
      await handleEvent(raw);
      // Advance cursor only after successful processing.
      lastCursor = raw.paging_token;
      retries    = 0;
      // Persist lastCursor to durable storage (Redis, DB) here.
    } catch (err) {
      console.error("[ChainSettle] event handler error:", err);
      // Do NOT advance cursor on error; we will re-process on reconnect.
    }
  };

  es.onerror = (_err) => {
    console.warn("[ChainSettle] SSE error — reconnecting…");
    es?.close();

    const delay = Math.min(
      BASE_DELAY_MS * 2 ** retries + Math.random() * 500,
      MAX_RETRY_MS
    );
    retries++;
    console.log(`[ChainSettle] retrying in ${Math.round(delay)}ms (attempt ${retries})`);
    setTimeout(connect, delay);
  };
}

// Start listening.
connect();
```

---

## Complete Runnable Example

Save as `listener.ts` in the `chainsetttle-backend` repo and run with
`ts-node listener.ts` (or compile first with `tsc`).

```typescript
import * as dotenv from "dotenv";
dotenv.config();

import EventSource from "eventsource";
import { xdr, StrKey } from "@stellar/stellar-sdk";

const HORIZON_URL  = process.env.HORIZON_URL  ?? "https://horizon-testnet.stellar.org";
const CONTRACT_ID  = process.env.CONTRACT_ID  ?? "";
const EVENT_LIMIT  = process.env.EVENT_LIMIT  ?? "100";

if (!CONTRACT_ID) throw new Error("CONTRACT_ID is not set in .env");

// ─── helpers ───────────────────────────────────────────────────────────────

function decodeSymbol(b64: string): string {
  return xdr.ScVal.fromXDR(b64, "base64").sym().toString();
}
function decodeString(b64: string): string {
  return Buffer.from(xdr.ScVal.fromXDR(b64, "base64").str()).toString("utf-8");
}
function decodeAddress(scv: xdr.ScVal): string {
  const a = scv.address();
  return a.switch() === xdr.ScAddressType.scAddressTypeAccount()
    ? StrKey.encodeEd25519PublicKey(a.accountId().ed25519())
    : StrKey.encodeContract(a.contractId());
}
function decodeI128(scv: xdr.ScVal): bigint {
  const i = scv.i128();
  return (BigInt(i.hi().toString()) << 64n) | BigInt(i.lo().toString());
}

// ─── router ────────────────────────────────────────────────────────────────

interface RawEvent {
  paging_token: string;
  topic:        string[];
  value:        string;
}

function handleEvent(raw: RawEvent): void {
  const eventName  = decodeSymbol(raw.topic[0]);
  const shipmentId = raw.topic[1] ? decodeString(raw.topic[1]) : "";
  const vals       = xdr.ScVal.fromXDR(raw.value, "base64").vec() ?? [];

  switch (eventName) {
    case "shipment_created":
      console.log("[shipment_created]", {
        shipmentId,
        buyer:        decodeAddress(vals[0]),
        supplier:     decodeAddress(vals[1]),
        logistics:    decodeAddress(vals[2]),
        arbiter:      decodeAddress(vals[3]),
        token:        decodeAddress(vals[4]),
        total_amount: decodeI128(vals[5]).toString(),
        ledger:       vals[6].u32(),
      });
      break;

    case "milestone_confirmed":
      console.log("[milestone_confirmed]", {
        shipmentId,
        milestone_index:  vals[0].u32(),
        payment:          decodeI128(vals[1]).toString(),
        fee_amount:       decodeI128(vals[2]).toString(),
        penalty_deducted: decodeI128(vals[3]).toString(),
        supplier:         decodeAddress(vals[4]),
        ledger:           vals[5].u32(),
      });
      break;

    case "shipment_cancelled":
      console.log("[shipment_cancelled]", {
        shipmentId,
        refunded_amount: decodeI128(vals[0]).toString(),
        cancelled_by:    decodeAddress(vals[1]),
        ledger:          vals[2].u32(),
      });
      break;

    default:
      console.log(`[${eventName}]`, shipmentId);
  }
}

// ─── reconnect loop ────────────────────────────────────────────────────────

let cursor  = "now";
let retries = 0;

function connect(): void {
  const url = new URL(`${HORIZON_URL}/events`);
  url.searchParams.set("filter[contract_ids]", CONTRACT_ID);
  url.searchParams.set("cursor", cursor);
  url.searchParams.set("limit",  EVENT_LIMIT);

  const es = new EventSource(url.toString());

  es.onmessage = (msg) => {
    if (msg.data === '"hello"' || msg.data === '"byebye"') return;
    const raw = JSON.parse(msg.data) as RawEvent;
    handleEvent(raw);
    cursor  = raw.paging_token;
    retries = 0;
  };

  es.onerror = () => {
    es.close();
    const delay = Math.min(1000 * 2 ** retries++, 30_000);
    console.warn(`[ChainSettle] reconnecting in ${delay}ms`);
    setTimeout(connect, delay);
  };

  console.log(`[ChainSettle] listening on ${CONTRACT_ID} (cursor: ${cursor})`);
}

connect();
```

Test this example against the testnet contract by pointing `CONTRACT_ID` at
the deployed ChainSettle testnet contract ID and running:

```bash
CONTRACT_ID=C... HORIZON_URL=https://horizon-testnet.stellar.org ts-node listener.ts
```

You should see events appear as you call `create_shipment`, `confirm_milestone`,
etc. via the frontend or Stellar CLI.
