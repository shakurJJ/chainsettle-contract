# ChainSettle — Frontend Integration Guide (React + Freighter)

This guide covers how to connect the Freighter wallet, construct and sign
Soroban transactions for each ChainSettle operation, and handle transaction
confirmation feedback in the React UI.

Related repos:
- Contract (you are here): `chainsetttle-contract`
- Frontend: [`chainsetttle-frontend`](https://github.com/shakurJJ/chainsetttle-frontend)
- Backend: [`chainsetttle-backend`](https://github.com/shakurJJ/chainsetttle-backend)

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Environment Variables](#environment-variables)
3. [Connecting Freighter Wallet](#connecting-freighter-wallet)
4. [Contract Invocation Pattern](#contract-invocation-pattern)
5. [Example: `create_shipment`](#example-create_shipment)
6. [Example: `confirm_milestone`](#example-confirm_milestone)
7. [Error Handling](#error-handling)
8. [Transaction Simulation Before Submission](#transaction-simulation-before-submission)
9. [Polling for Confirmation](#polling-for-confirmation)

---

## Prerequisites

```bash
npm install @stellar/stellar-sdk @stellar/freighter-api
```

| Package | Purpose |
|---------|---------|
| `@stellar/stellar-sdk` | Transaction building, XDR serialisation, Soroban contract client |
| `@stellar/freighter-api` | Freighter browser extension API |

---

## Environment Variables

```dotenv
VITE_SOROBAN_RPC_URL=https://soroban-testnet.stellar.org
VITE_HORIZON_URL=https://horizon-testnet.stellar.org
VITE_NETWORK_PASSPHRASE=Test SDF Network ; September 2015
VITE_CONTRACT_ID=CAAAA...YOUR_CONTRACT_ID_HERE
```

---

## Connecting Freighter Wallet

```typescript
import {
  isConnected,
  requestAccess,
  getPublicKey,
  signTransaction,
} from "@stellar/freighter-api";

/** Check that the Freighter extension is installed and connected. */
export async function connectFreighter(): Promise<string> {
  const connected = await isConnected();
  if (!connected) {
    throw new Error(
      "Freighter wallet is not installed. " +
      "Install it from https://freighter.app and reload."
    );
  }

  await requestAccess();                // prompts user to grant access if not already granted
  const publicKey = await getPublicKey();
  if (!publicKey) {
    throw new Error("No public key returned from Freighter.");
  }

  console.log("[Freighter] connected as", publicKey);
  return publicKey;
}
```

Keep the public key in React state and use it as the `buyer` / `supplier`
address in all subsequent contract calls.

```tsx
const [walletAddress, setWalletAddress] = React.useState<string | null>(null);

async function handleConnect() {
  try {
    const addr = await connectFreighter();
    setWalletAddress(addr);
  } catch (err) {
    console.error(err);
    alert((err as Error).message);
  }
}
```

---

## Contract Invocation Pattern

Every contract call follows four steps:

1. **Build** the `Operation` using `Contract.call()`.
2. **Simulate** the transaction against Soroban RPC to get the `auth` and
   `footprint` (storage access list).
3. **Assemble** the final transaction with the simulation result.
4. **Sign** via Freighter and **submit** to Soroban RPC.

```typescript
import {
  Contract,
  Networks,
  rpc as SorobanRpc,
  TransactionBuilder,
  BASE_FEE,
  xdr,
  Account,
} from "@stellar/stellar-sdk";

const RPC_URL           = import.meta.env.VITE_SOROBAN_RPC_URL as string;
const CONTRACT_ID       = import.meta.env.VITE_CONTRACT_ID     as string;
const NETWORK_PASSPHRASE = import.meta.env.VITE_NETWORK_PASSPHRASE as string;

const server = new SorobanRpc.Server(RPC_URL);

async function invokeContract(
  callerAddress: string,
  method: string,
  args: xdr.ScVal[],
): Promise<SorobanRpc.SorobanRpc.GetTransactionResponse> {
  // 1. Fetch current sequence number for the caller account.
  const account  = await server.getAccount(callerAddress);
  const contract = new Contract(CONTRACT_ID);

  // 2. Build an unsigned transaction.
  const tx = new TransactionBuilder(new Account(account.id, account.sequence), {
    fee:              BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(contract.call(method, ...args))
    .setTimeout(30)
    .build();

  // 3. Simulate to obtain auth entries and footprint (see dedicated section).
  const simResult = await server.simulateTransaction(tx);
  if (!SorobanRpc.Api.isSimulationSuccess(simResult)) {
    throw new Error(`Simulation failed: ${JSON.stringify(simResult)}`);
  }

  // 4. Assemble — merges auth + footprint into the transaction envelope.
  const assembled = SorobanRpc.assembleTransaction(tx, simResult).build();

  // 5. Sign via Freighter.
  const signedXdr = await signTransaction(assembled.toXDR(), {
    network: "TESTNET",        // or "PUBLIC" for mainnet
    networkPassphrase: NETWORK_PASSPHRASE,
    accountToSign: callerAddress,
  });

  // 6. Submit and poll.
  return submitAndWait(signedXdr);
}

async function submitAndWait(
  signedXdr: string,
): Promise<SorobanRpc.SorobanRpc.GetTransactionResponse> {
  const { hash } = await server.sendTransaction(
    TransactionBuilder.fromXDR(signedXdr, NETWORK_PASSPHRASE)
  );

  // Poll until confirmed or failed (Stellar closes a ledger ~every 5 seconds).
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 3_000));
    const result = await server.getTransaction(hash);
    if (result.status === SorobanRpc.Api.GetTransactionStatus.SUCCESS) return result;
    if (result.status === SorobanRpc.Api.GetTransactionStatus.FAILED)
      throw new Error(`Transaction ${hash} failed: ${JSON.stringify(result)}`);
  }
  throw new Error(`Transaction ${hash} timed out after 90 seconds.`);
}
```

---

## Example: `create_shipment`

```typescript
import { nativeToScVal, Address, xdr } from "@stellar/stellar-sdk";

interface MilestoneInput {
  name:            string;
  payment_percent: number;
}

interface CreateShipmentParams {
  shipmentId:   string;
  buyer:        string;            // Stellar address of primary buyer
  supplier:     string;
  logistics:    string;
  arbiter:      string;
  token:        string;            // SAC or custom token contract address
  totalAmount:  bigint;            // in token's smallest unit (e.g. stroops for XLM)
  milestones:   MilestoneInput[];
}

export async function createShipment(params: CreateShipmentParams) {
  const {
    shipmentId, buyer, supplier, logistics, arbiter, token,
    totalAmount, milestones,
  } = params;

  if (milestones.reduce((s, m) => s + m.payment_percent, 0) !== 100) {
    throw new Error("Milestone percentages must sum to 100.");
  }

  const args: xdr.ScVal[] = [
    nativeToScVal(shipmentId, { type: "string" }),

    // buyers: Vec<Address>
    xdr.ScVal.scvVec([Address.fromString(buyer).toScVal()]),

    Address.fromString(supplier).toScVal(),
    Address.fromString(logistics).toScVal(),
    Address.fromString(arbiter).toScVal(),
    Address.fromString(token).toScVal(),

    nativeToScVal(totalAmount, { type: "i128" }),

    // milestones: Vec<Milestone> — each Milestone is a map / contracttype
    xdr.ScVal.scvVec(
      milestones.map((m) =>
        xdr.ScVal.scvMap([
          new xdr.ScMapEntry({
            key: nativeToScVal("name",            { type: "symbol" }),
            val: nativeToScVal(m.name,            { type: "string" }),
          }),
          new xdr.ScMapEntry({
            key: nativeToScVal("payment_percent", { type: "symbol" }),
            val: nativeToScVal(m.payment_percent, { type: "u32" }),
          }),
          new xdr.ScMapEntry({
            key: nativeToScVal("proof_hash",      { type: "symbol" }),
            val: nativeToScVal("",                { type: "string" }),
          }),
          new xdr.ScMapEntry({
            key: nativeToScVal("status",          { type: "symbol" }),
            // MilestoneStatus::Pending = enum variant index 0
            val: xdr.ScVal.scvVec([nativeToScVal("Pending", { type: "symbol" })]),
          }),
          new xdr.ScMapEntry({
            key: nativeToScVal("release_after_ledger", { type: "symbol" }),
            val: nativeToScVal(0,                      { type: "u32" }),
          }),
          new xdr.ScMapEntry({
            key: nativeToScVal("proof_submitted_ledger", { type: "symbol" }),
            val: xdr.ScVal.scvVoid(),
          }),
          new xdr.ScMapEntry({
            key: nativeToScVal("dispute_opened_ledger", { type: "symbol" }),
            val: xdr.ScVal.scvVoid(),
          }),
        ])
      )
    ),

    // options: ShipmentOptions
    xdr.ScVal.scvMap([
      new xdr.ScMapEntry({
        key: nativeToScVal("response_deadline",          { type: "symbol" }),
        val: nativeToScVal(0,                            { type: "u32" }),
      }),
      new xdr.ScMapEntry({
        key: nativeToScVal("penalty_bps",                { type: "symbol" }),
        val: nativeToScVal(0,                            { type: "u32" }),
      }),
      new xdr.ScMapEntry({
        key: nativeToScVal("milestone_mode",             { type: "symbol" }),
        val: xdr.ScVal.scvVec([nativeToScVal("Parallel", { type: "symbol" })]),
      }),
      new xdr.ScMapEntry({
        key: nativeToScVal("holdback_ledgers",           { type: "symbol" }),
        val: nativeToScVal(0,                            { type: "u32" }),
      }),
      new xdr.ScMapEntry({
        key: nativeToScVal("dispute_cooldown_ledgers",   { type: "symbol" }),
        val: nativeToScVal(0,                            { type: "u32" }),
      }),
      new xdr.ScMapEntry({
        key: nativeToScVal("late_penalty_bps_per_ledger", { type: "symbol" }),
        val: nativeToScVal(0,                              { type: "u32" }),
      }),
      new xdr.ScMapEntry({
        key: nativeToScVal("auto_confirm_ledgers",       { type: "symbol" }),
        val: nativeToScVal(0,                            { type: "u32" }),
      }),
    ]),
  ];

  return invokeContract(buyer, "create_shipment", args);
}
```

---

## Example: `confirm_milestone`

```typescript
export async function confirmMilestone(
  buyer:          string,
  shipmentId:     string,
  milestoneIndex: number,
) {
  const args: xdr.ScVal[] = [
    Address.fromString(buyer).toScVal(),
    nativeToScVal(shipmentId,     { type: "string" }),
    nativeToScVal(milestoneIndex, { type: "u32"    }),
  ];
  return invokeContract(buyer, "confirm_milestone", args);
}
```

---

## Error Handling

Contract panics surface as Soroban `InvokeHostFunctionError` results. Map the
known error symbols to user-friendly messages:

```typescript
const CONTRACT_ERRORS: Record<string, string> = {
  ShipmentAlreadyExists:        "A shipment with this ID already exists.",
  ShipmentNotFound:             "Shipment not found. Check the ID.",
  Unauthorized:                 "You are not authorised to perform this action.",
  InvalidMilestoneIndex:        "Invalid milestone index.",
  InvalidMilestoneStatus:       "Milestone is not in the expected state.",
  ShipmentNotActive:            "Shipment is not active.",
  InvalidPercentages:           "Milestone percentages must sum to 100.",
  InvalidAmount:                "Amount must be greater than zero.",
  DisputeAlreadyOpen:           "A dispute is already open for this milestone.",
  DeadlineNotBreached:          "The deadline has not been breached yet.",
  FeeTooHigh:                   "Fee exceeds maximum (1000 bps).",
  PreviousMilestoneNotComplete: "Previous milestone must be completed first.",
  ContractPaused:               "Contract is currently paused by admin.",
  DisputeCooldownActive:        "Dispute cooldown period has not elapsed.",
  TransferDisallowed:           "Token transfer disallowed (check allowances).",
  CircuitBreakerTripped:        "Circuit breaker limit exceeded. Try again later.",
};

export function parseContractError(err: unknown): string {
  const msg = err instanceof Error ? err.message : String(err);

  for (const [key, label] of Object.entries(CONTRACT_ERRORS)) {
    if (msg.includes(key)) return label;
  }

  // User rejection comes from Freighter
  if (msg.toLowerCase().includes("user declined") || msg.includes("rejected")) {
    return "Transaction was rejected in your wallet.";
  }
  if (msg.toLowerCase().includes("insufficient balance")) {
    return "Insufficient token balance to fund this shipment.";
  }

  return `Transaction failed: ${msg}`;
}

// Usage in a React component:
// try {
//   await createShipment(params);
//   toast.success("Shipment created!");
// } catch (err) {
//   toast.error(parseContractError(err));
// }
```

---

## Transaction Simulation Before Submission

Always simulate before submitting. Simulation reveals:

- Resource fees (ledger reads/writes, instructions) so you can set an
  accurate fee instead of the default.
- Auth requirements — which addresses need to sign.
- Whether the transaction will succeed — catch errors cheaply before paying
  fees.

```typescript
export async function simulateOnly(
  callerAddress: string,
  method:        string,
  args:          xdr.ScVal[],
): Promise<SorobanRpc.Api.SimulateTransactionSuccessResponse> {
  const account  = await server.getAccount(callerAddress);
  const contract = new Contract(CONTRACT_ID);

  const tx = new TransactionBuilder(new Account(account.id, account.sequence), {
    fee:              BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(contract.call(method, ...args))
    .setTimeout(30)
    .build();

  const sim = await server.simulateTransaction(tx);
  if (!SorobanRpc.Api.isSimulationSuccess(sim)) {
    // Surface contract error before any wallet interaction.
    throw new Error(
      `Simulation error: ${
        (sim as SorobanRpc.Api.SimulateTransactionErrorResponse).error
      }`
    );
  }
  return sim;
}
```

Use `simulateOnly` for a "dry-run" preview:

```typescript
async function previewCreateShipment(params: CreateShipmentParams) {
  const args = buildCreateShipmentArgs(params); // same as createShipment above
  const sim  = await simulateOnly(params.buyer, "create_shipment", args);

  // Estimated fee in stroops.
  const fee = Number(sim.minResourceFee) + Number(BASE_FEE);
  console.log(`Estimated fee: ${fee} stroops (${fee / 1e7} XLM)`);
  return sim;
}
```

---

## Polling for Confirmation

After submission, poll `server.getTransaction(hash)` until status is
`SUCCESS` or `FAILED`. Display a loading state in the UI during this window
(Stellar closes ~every 5 seconds):

```tsx
type TxStatus = "idle" | "signing" | "pending" | "success" | "failed";

function ShipmentConfirmButton({ shipmentId, milestoneIndex }: Props) {
  const [status, setStatus] = React.useState<TxStatus>("idle");
  const [error,  setError]  = React.useState<string | null>(null);
  const walletAddress = useWalletAddress(); // from your wallet context

  async function handleConfirm() {
    setStatus("signing");
    setError(null);
    try {
      setStatus("pending");
      await confirmMilestone(walletAddress, shipmentId, milestoneIndex);
      setStatus("success");
    } catch (err) {
      setStatus("failed");
      setError(parseContractError(err));
    }
  }

  return (
    <div>
      <button onClick={handleConfirm} disabled={status === "pending" || status === "signing"}>
        {status === "signing"  && "Waiting for wallet…"}
        {status === "pending"  && "Confirming on-chain…"}
        {status === "success"  && "Confirmed!"}
        {status === "failed"   && "Retry"}
        {status === "idle"     && "Confirm Milestone"}
      </button>
      {error && <p style={{ color: "red" }}>{error}</p>}
    </div>
  );
}
```
