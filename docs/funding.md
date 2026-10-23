# Buyer Funding Options

Buyers have three ways to fund escrow, plus top-ups. All of them end in the
same escrowed shipment; they differ in when tokens move and whose
authorization is required. Source of truth:
`contracts/chainsetttle/src/lib.rs`.

## 1. Direct funding (default)

`create_shipment` pulls `total_amount` from the buyer(s) at creation time via
a token transfer. Use it for one-off shipments where the buyer holds the
tokens in their own account.

Top up later with `top_up_escrow(buyer, shipment_id, additional_amount)`
when a shipment needs more escrow (for example after extensions or added
scope).

## 2. Escrow vault

Pre-fund a personal vault, then draw from it per shipment:

- `vault_deposit(buyer, token, amount)` moves tokens into the buyer's vault.
- `vault_withdraw(buyer, token, amount)` pulls them back out.
- `get_vault_balance(buyer, token)` reads the balance.

Set `ShipmentOptions.fund_from_vault = true` on `create_shipment` to fund
that shipment from the vault instead of the buyer's wallet. Use the vault
when a buyer runs many shipments and wants to fund once, or when an
operator prefunds on a buyer's behalf. `fund_from_vault` cannot be combined
with allowance funding (`create_shipment_with_allowance` panics).

## 3. Standing orders

Recurring purchases without re-authorizing every shipment:

- `create_standing_order(buyer, params: StandingOrderParams)` registers a
  schedule and returns an `order_id`.
- `execute_standing_order(order_id)` creates the next shipment from the
  order and returns the new shipment id.
- `cancel_standing_order(buyer, order_id)` stops the schedule.
- `get_standing_order(order_id)` inspects it.

Use standing orders for repeat replenishment from the same supplier.

## 4. Allowance-based funding

`create_shipment_with_allowance(params: AllowanceShipmentParams)` lets a
third-party `spender` authorize the transfer from a funder (`params.from`,
who must be one of `params.buyers`). The spender authenticates
(`params.spender.require_auth()`), so a treasury, employer, or sponsor can
pay for a buyer's shipment without holding the buyer's keys. As noted above,
it cannot be combined with `fund_from_vault`.

## Interaction with spending limits and the TVL cap

- `set_buyer_spending_limit` / `get_buyer_spending_limit(buyer)` cap how much
  a buyer can put into escrow (limit plus window). Fund from any source
  counts toward the limit: check `get_buyer_spending_limit` before
  `create_shipment`, `top_up_escrow`, or `execute_standing_order` for
  capped buyers.
- `set_tvl_cap(admin, token, cap)` / `get_tvl_cap(token)` bound the total
  escrowed value per token (`get_tvl(token)` reads current usage). A funding
  call that would push escrowed value over the cap fails, whichever funding
  option is used.
- `get_escrow_balance(shipment_id)` and `get_total_escrowed_value(token)`
  report where the money currently sits.

## Which option to use

| Situation | Use |
| --- | --- |
| One-off purchase, buyer holds tokens | `create_shipment` |
| Many shipments, fund once up front | `vault_deposit` + `fund_from_vault` |
| Repeat scheduled purchases | `create_standing_order` / `execute_standing_order` |
| Third party pays for the buyer | `create_shipment_with_allowance` |
| Shipment needs more escrow later | `top_up_escrow` |
