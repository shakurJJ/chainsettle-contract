# Multi-Party Shipments

A shipment always has one supplier, one logistics provider and one arbiter,
but it can involve many more parties. This guide covers every way to add
them: co-buyers, consortium suppliers, substitute suppliers, observers,
delegates and payees. For each one it says who can act and what
authorization is needed. Source of truth:
`contracts/chainsetttle/src/lib.rs`.

CLI examples use the same variables as [cli.md](cli.md):

```bash
CID=<CONTRACT_ID>
SRC=my-account   # stellar keys identity, --source-account equivalent
NET=testnet
```

## 1. Multiple buyers and the co-buyer

**Multiple buyers.** `create_shipment` takes a `buyers` vector. Every
address in it must authorize creation (`require_auth` on each buyer).
After that, any listed buyer counts as "the buyer" for buyer-only calls
such as `confirm_milestone`, `raise_dispute`, `cancel_shipment` and
`set_milestone_payees`. The first buyer (`buyers[0]`) is the primary buyer
and receives refunds unless a refund recipient is set.

**Co-buyer (dual control).** `set_co_buyer(caller, shipment_id, co_buyer)`
names a second approver for joint milestone confirmation.

- Who: any registered buyer, with that buyer's auth.
- When: the shipment is `Active` and nothing has been released or advanced
  yet. It can be set only once and cannot be changed afterwards.
- Effect: when the shipment value is above the admin-set
  `set_joint_confirmation_threshold` (0 = off), `confirm_milestone` releases
  funds only after both a buyer (or delegate) and the co-buyer have
  confirmed. Follow progress with
  `get_joint_confirmation_status(shipment_id, milestone_index)`.
- The co-buyer authorizes its own `confirm_milestone` call. It cannot
  dispute, cancel or change the shipment.

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- set-co-buyer --caller <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' --co-buyer <CO_BUYER_ADDRESS>

stellar contract invoke --id $CID --source co-buyer --network $NET \
  -- confirm-milestone --buyer <CO_BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' --milestone-index 0
```

Query with `get_co_buyer(shipment_id)`.

## 2. Consortium suppliers

A consortium shipment assigns each milestone to a different supplier. Set
`ShipmentOptions.milestone_suppliers` on `create_shipment` to one address
per milestone. The vector length must match the milestone count.

- Who: set by the buyers at creation, with the usual creation auth.
- Effect: each milestone supplier is the designated proof submitter for its
  milestone (unless `proof_submitters` is set) and receives that
  milestone's payout. The shipment is indexed under every member supplier.
- Query: `is_consortium_shipment(shipment_id)` and
  `get_milestone_supplier(shipment_id, milestone_index)`.

```bash
stellar contract invoke --id $CID --source $SRC --network $NET \
  -- create-shipment ... \
  --options '{ ..., "milestone_suppliers": ["<SUPPLIER_A>", "<SUPPLIER_B>"] }'
```

## 3. Substitute suppliers

If a milestone with a deadline goes overdue while still `Pending`, the
buyer can hand it to another supplier. This is a two-step flow:

1. `propose_substitute_supplier(buyer, shipment_id, milestone_index, new_supplier)`:
   a registered buyer, with buyer auth. `new_supplier` must differ from the
   shipment supplier.
2. `approve_substitute_supplier(arbiter, shipment_id, milestone_index)`:
   the shipment arbiter, with arbiter auth.

Both calls need the shipment to be `Active`, the milestone to be `Pending`,
and the milestone `deadline_ledger` to have passed. Once approved, the
substitute submits proof and receives the payout for that milestone only.
It is returned by `get_milestone_supplier`.

```bash
stellar contract invoke --id $CID --source buyer --network $NET \
  -- propose-substitute-supplier --buyer <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' --milestone-index 1 --new-supplier <NEW_SUPPLIER>

stellar contract invoke --id $CID --source arbiter --network $NET \
  -- approve-substitute-supplier --arbiter <ARBITER_ADDRESS> \
  --shipment-id '"SHIP-001"' --milestone-index 1
```

## 4. Observers

Observers are a read-only list for off-chain event routing. They have no
on-chain authority.

- `add_shipment_observer(buyer, shipment_id, observer_address)`: a registered
  buyer, with buyer auth. Adding an address twice has no effect.
- `remove_shipment_observer(buyer, shipment_id, observer_address)`: same rules.
- `get_shipment_observers(shipment_id)`: anyone.

```bash
stellar contract invoke --id $CID --source buyer --network $NET \
  -- add-shipment-observer --buyer <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' --observer-address <OBSERVER_ADDRESS>
```

## 5. Confirmation delegates

A delegate can confirm milestones for the buyer.

- `authorize_delegate(buyer, shipment_id, delegate, per_tx_cap)`: a registered
  buyer, with buyer auth, on an `Active` shipment. `per_tx_cap` must be
  greater than zero. A new call replaces the previous delegate.
- `revoke_delegate(buyer, shipment_id)`: a registered buyer, with buyer auth.
  It takes effect immediately.
- `get_delegate(shipment_id)`: anyone.

The delegate calls `confirm_milestone` with its own address in the `buyer`
argument and its own auth. It cannot dispute, cancel or change the
shipment.

> **Note:** `per_tx_cap` is stored in `DelegateConfig` and returned by
> `get_delegate`, but `confirm_milestone` does not currently check it. Do
> not rely on it as an on-chain limit until it is enforced.


```bash
stellar contract invoke --id $CID --source buyer --network $NET \
  -- authorize-delegate --buyer <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' --delegate <DELEGATE_ADDRESS> --per-tx-cap 250000

stellar contract invoke --id $CID --source delegate --network $NET \
  -- confirm-milestone --buyer <DELEGATE_ADDRESS> \
  --shipment-id '"SHIP-001"' --milestone-index 0
```

## 6. Payees

There are two ways to send a supplier's payout somewhere else.

**Milestone payee splits.**
`set_milestone_payees(buyer, shipment_id, milestone_index, payees)` splits one
milestone's payout across several addresses (`MilestonePayee { payee, percent }`).

- Who: a registered buyer, with buyer auth.
- When: the shipment is `Active` and the milestone is still `Pending`.
- Percents must sum to 100. An empty vector removes the split.
- Query: `get_milestone_payees(shipment_id, milestone_index)`.

```bash
stellar contract invoke --id $CID --source buyer --network $NET \
  -- set-milestone-payees --buyer <BUYER_ADDRESS> \
  --shipment-id '"SHIP-001"' --milestone-index 0 \
  --payees '[{"payee":"<SUPPLIER_ADDRESS>","percent":70},{"payee":"<SUBCONTRACTOR>","percent":30}]'
```

**Payout assignment (invoice factoring).** Sends every later supplier payout
on the shipment to a financier. This overrides payee splits and payout
batching.

- `assign_payout(supplier, shipment_id, financier)`: the supplier and the
  financier must both authorize. The shipment must be `Active` with no open
  dispute, and the financier must not be blacklisted. Only one assignment
  per shipment.
- `transfer_payout_assignment(financier, shipment_id, new_financier)`: the
  current and the new financier must both authorize.
- `release_payout_assignment(financier, shipment_id)`: the current financier
  only. The supplier cannot revoke an assignment.
- Query: `get_payout_assignment(shipment_id)`.

```bash
stellar contract invoke --id $CID --source supplier --network $NET \
  -- assign-payout --supplier <SUPPLIER_ADDRESS> \
  --shipment-id '"SHIP-001"' --financier <FINANCIER_ADDRESS>
```

## Permission matrix

"Buyer" means any address in the shipment's `buyers` vector. Every row needs
the acting party's own `require_auth`.

| Function | Who can call | Extra auth | Conditions |
| --- | --- | --- | --- |
| `create_shipment` (`buyers`, `milestone_suppliers`) | buyers | every buyer; supplier too if `supplier_collateral > 0` | — |
| `set_co_buyer` | buyer | — | `Active`, nothing released or advanced, set once |
| `confirm_milestone` | buyer, co-buyer or delegate | — | milestone has proof; joint confirmation above threshold |
| `propose_substitute_supplier` | buyer | — | `Active`, milestone `Pending` and overdue |
| `approve_substitute_supplier` | shipment arbiter | — | same as above, and a proposal exists |
| `add_shipment_observer` / `remove_shipment_observer` | buyer | — | — |
| `authorize_delegate` | buyer | — | `Active`, `per_tx_cap > 0` |
| `revoke_delegate` | buyer | — | — |
| `set_milestone_payees` | buyer | — | `Active`, milestone `Pending`, percents sum to 100 |
| `assign_payout` | supplier | financier | `Active`, no open dispute, not yet assigned |
| `transfer_payout_assignment` | current financier | new financier | — |
| `release_payout_assignment` | current financier | — | — |

What each added party can do afterwards:

| Party | Confirm | Dispute / cancel | Submit proof | Receives funds |
| --- | --- | --- | --- | --- |
| Additional buyer | yes | yes | no | refunds only as `buyers[0]` |
| Co-buyer | yes (joint) | no | no | no |
| Consortium supplier | no | no | its milestone | its milestone's payout |
| Substitute supplier | no | no | its milestone | its milestone's payout |
| Observer | no | no | no | no |
| Delegate | yes | no | no | no |
| Milestone payee | no | no | no | its share of the milestone |
| Financier (assignee) | no | no | no | all later supplier payouts |
