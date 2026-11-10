# ChainSettle — Contract

![CI](https://github.com/shakurJJ/chainsettle-contract/actions/workflows/ci.yml/badge.svg)
![codecov](https://codecov.io/gh/shakurJJ/chainsettle-contract/branch/main/graph/badge.svg)

**Milestone-based supply chain escrow on Stellar Soroban.** The buyer locks payment in escrow, and the contract releases it to the supplier as each delivery milestone is confirmed on-chain.

This is one of three ChainSettle repos:

| Repo | What |
| --- | --- |
| `chainsettle-contract` (this one) | Soroban smart contract (Rust) |
| `chainsetttle-frontend` | React + Freighter wallet UI |
| `chainsetttle-backend` | Node.js API, notifications, off-chain metadata |

## How it works

```
Buyer creates shipment      → funds locked in escrow
Supplier submits proof      → milestone 1 (e.g. dispatch, 25%)
Buyer confirms              → 25% released to supplier
Logistics submits proof     → milestone 2 (transit, 50%)
Buyer confirms              → 50% released
Supplier submits proof      → milestone 3 (delivery, 25%)
Buyer confirms              → final 25% released, shipment Completed

Buyer disputes a proof      → milestone frozen until the arbiter resolves it
```

One deployed contract serves any number of independent shipments.

### Roles

| Role | Can do |
| --- | --- |
| Buyer | Create and fund shipments, confirm milestones, raise disputes, cancel |
| Supplier | Submit proof, receive payouts |
| Logistics | Submit proof (e.g. in-transit milestones) |
| Arbiter | Resolve disputes |
| Admin | Configure fees, tokens, limits, pauses, roles and upgrades (set at `init`) |

Every state-changing call needs the acting party's signature (`require_auth`).

### Milestone states

```
Pending ─ submit_proof ─→ ProofSubmitted
  ├─ confirm_milestone ─→ Confirmed (paid)
  └─ raise_dispute ─→ Disputed
        ├─ resolve_dispute(approve)  ─→ Resolved (paid)
        ├─ resolve_dispute(reject)   ─→ Pending (resubmit)
        └─ resolve_dispute_timeout   ─→ Resolved (default outcome)
```

## Features

The core flow above is extended by optional, admin- or party-configured features, including:

- **Disputes:** partial disputes, evidence, mediation, arbiter pools and panels, appeals, arbiter slashing, finality delay
- **Payments:** advances, payee splits, retainage, warranties, quality grades, partial quantities, payout batching and currency preference
- **Risk controls:** supplier collateral and slashing (see [`docs/collateral.md`](docs/collateral.md)), exposure and TVL caps, buyer spending limits, depeg guard, blacklist with appeals
- **Fees:** fee tiers, VIP waivers, fee holidays, clean-completion rebates
- **Admin:** granular roles, multisig, timelock, global and per-operation pause, upgrades with `migrate`
- **Queries:** shipment summaries, cost estimates, completion percentage, reputation counters, paginated lists
- **Events:** every state change publishes an event. See [`docs/events.md`](docs/events.md) for each topic, its payload shape and the functions that emit it

For the full API, see the doc comments on each `pub fn` in [`lib.rs`](contracts/chainsetttle/src/lib.rs). Topic notes live in [`docs/`](docs/): [events](docs/events.md), [tokens](docs/tokens.md), [completion percentage](docs/completion-percentage.md), [error catalogue](docs/errors.md), [storage TTL and archival](docs/storage-ttl.md), [query and pagination reference](docs/queries.md), [compliance controls](docs/compliance.md) and [collateral and dispute bonds](docs/collateral.md).

## Project structure

```
contracts/chainsetttle/
├── src/lib.rs         contract, types and storage keys
├── src/storage.rs     storage helpers
├── src/test_*.rs      unit tests, one file per feature; shared fixtures in test_common.rs
├── tests/             snapshot tests (UPDATE_SNAPSHOTS=1 to regenerate)
└── Makefile           build / optimize / deploy targets
```

## Getting started

**Prerequisites:** Rust ≥ 1.84 with the `wasm32v1-none` target, and the [Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```bash
rustup target add wasm32v1-none
brew install stellar-cli            # or: cargo install --locked stellar-cli --features opt
```

**Build and test:**

```bash
cargo test                          # all tests
cargo test <name> -- --nocapture    # one test, with output

cd contracts/chainsetttle
make build                          # → target/wasm32v1-none/release/chainsetttle.wasm
make optimize                       # smaller .wasm for deployment (64 KB limit)
```

**Deploy to testnet:**

```bash
stellar keys generate --global my-account --network testnet
stellar keys fund my-account --network testnet
export STELLAR_ACCOUNT=my-account

cd contracts/chainsetttle && make deploy-testnet    # prints the contract ID

stellar contract invoke --id <CONTRACT_ID> --source my-account --network testnet \
  -- init --admin <YOUR_ADDRESS>
```

> Deploy to mainnet only after thorough testing and, ideally, a security audit. Mainnet USDC SAC: `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7EJKEF`

## Security notes

- **Escrow:** funds sit at the contract address and leave only through explicit payout and refund paths.
- **Validation:** milestone percentages must sum to 100. Amounts use integer math, so the last milestone absorbs rounding.
- **Storage TTL:** persistent entries get an extended TTL on write. Long-lived shipments can be kept alive by anyone calling `extend_shipment_ttl`.
- **Upgrades:** `upgrade` (single admin) or `propose_upgrade` / `approve_upgrade` (multisig), followed by `migrate`.

## Contributing

PRs welcome. Run `cargo fmt` and `cargo test` before submitting.

## License

MIT

## Handsoff notes

<!-- handsoff-issue-653 -->
- #653: feat: claimable balance query per address

<!-- handsoff-issue-667 -->
- #667: feat: per-shipment event sequence number for indexers
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

const TTL_INITIAL_LEDGERS: u32 = 17_280;
const TTL_MAX_LEDGERS: u32 = 120_960;
const MAX_TERM_LEDGERS: u32 = 10_000_000;

#[contracttype]
#[derive(Clone)]
pub enum ArbiterMembershipKey {
    Admin,
    MembershipTerm,
    Expiry(Address),
    Registered(Address),
    AuditSequence,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipStatus {
    pub arbiter: Address,
    pub registered: bool,
    pub expiry_ledger: u32,
    pub current_ledger: u32,
    pub active: bool,
    pub remaining_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TermChangedEvent {
    pub admin: Address,
    pub old_term: u32,
    pub new_term: u32,
    pub ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenewalEvent {
    pub arbiter: Address,
    pub old_expiry: u32,
    pub new_expiry: u32,
    pub term: u32,
    pub ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrationEvent {
    pub arbiter: Address,
    pub expiry_ledger: u32,
    pub term: u32,
    pub ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipAuditRecord {
    pub sequence: u32,
    pub action: Symbol,
    pub subject: Address,
    pub actor: Address,
    pub old_expiry: u32,
    pub new_expiry: u32,
    pub ledger: u32,
}

#[contract]
pub struct ArbiterMembershipContract;

#[contractimpl]
impl ArbiterMembershipContract {
    pub fn init_membership(
        env: Env,
        admin: Address,
        default_term_ledgers: u32,
    ) {
        admin.require_auth();
        validate_term(default_term_ledgers);

        let admin_key = ArbiterMembershipKey::Admin;
        let term_key = ArbiterMembershipKey::MembershipTerm;

        if env.storage().instance().has(&admin_key) {
            panic!("membership already initialized");
        }

        env.storage().instance().set(&admin_key, &admin);
        env.storage()
            .instance()
            .set(&term_key, &default_term_ledgers);

        emit_initialized(&env, &admin, default_term_ledgers);
    }

    pub fn set_membership_term(
        env: Env,
        admin: Address,
        new_term_ledgers: u32,
    ) {
        admin.require_auth();
        validate_admin(&env, &admin);
        validate_term(new_term_ledgers);

        let old_term = read_term(&env);

        env.storage()
            .instance()
            .set(
                &ArbiterMembershipKey::MembershipTerm,
                &new_term_ledgers,
            );

        emit_term_changed(
            &env,
            &admin,
            old_term,
            new_term_ledgers,
        );

        write_audit(
            &env,
            symbol_short!("term_set"),
            admin.clone(),
            admin,
            0,
            0,
        );
    }

    pub fn register_arbiter(
        env: Env,
        admin: Address,
        arbiter: Address,
    ) {
        admin.require_auth();
        validate_admin(&env, &admin);
        validate_address_not_admin(&env, &arbiter);

        let registered_key =
            ArbiterMembershipKey::Registered(arbiter.clone());

        if env.storage()
            .persistent()
            .get::<_, bool>(&registered_key)
            .unwrap_or(false)
        {
            panic!("arbiter already registered");
        }

        let term = read_term(&env);
        let now = env.ledger().sequence();
        let expiry = checked_add(now, term);

        env.storage()
            .persistent()
            .set(&registered_key, &true);

        env.storage()
            .persistent()
            .set(
                &ArbiterMembershipKey::Expiry(arbiter.clone()),
                &expiry,
            );

        extend_membership_ttl(&env, &arbiter);

        emit_registration(&env, &arbiter, expiry, term);

        write_audit(
            &env,
            symbol_short!("arb_reg"),
            arbiter,
            admin,
            0,
            expiry,
        );
    }

    pub fn renew_arbiter_membership(
        env: Env,
        arbiter: Address,
    ) -> u32 {
        arbiter.require_auth();

        let registered_key =
            ArbiterMembershipKey::Registered(arbiter.clone());

        if !env.storage()
            .persistent()
            .get::<_, bool>(&registered_key)
            .unwrap_or(false)
        {
            panic!("arbiter is not registered");
        }

        let term = read_term(&env);
        let now = env.ledger().sequence();

        let expiry_key =
            ArbiterMembershipKey::Expiry(arbiter.clone());

        let old_expiry = env.storage()
            .persistent()
            .get::<_, u32>(&expiry_key)
            .unwrap_or_else(|| panic!("membership expiry missing"));

        let base = if old_expiry > now {
            old_expiry
        } else {
            now
        };

        let new_expiry = checked_add(base, term);

        env.storage()
            .persistent()
            .set(&expiry_key, &new_expiry);

        extend_membership_ttl(&env, &arbiter);

        emit_renewal(
            &env,
            &arbiter,
            old_expiry,
            new_expiry,
            term,
        );

        write_audit(
            &env,
            symbol_short!("arb_renew"),
            arbiter.clone(),
            arbiter,
            old_expiry,
            new_expiry,
        );

        new_expiry
    }

    pub fn get_arbiter_membership(
        env: Env,
        arbiter: Address,
    ) -> MembershipStatus {
        let registered = env.storage()
            .persistent()
            .get::<_, bool>(
                &ArbiterMembershipKey::Registered(arbiter.clone()),
            )
            .unwrap_or(false);

        let expiry = env.storage()
            .persistent()
            .get::<_, u32>(
                &ArbiterMembershipKey::Expiry(arbiter.clone()),
            )
            .unwrap_or(0);

        let now = env.ledger().sequence();
        let active = registered && expiry > now;

        let remaining = if active {
            expiry - now
        } else {
            0
        };

        MembershipStatus {
            arbiter,
            registered,
            expiry_ledger: expiry,
            current_ledger: now,
            active,
            remaining_ledgers: remaining,
        }
    }

    pub fn is_arbiter_membership_active(
        env: Env,
        arbiter: Address,
    ) -> bool {
        is_active_internal(&env, &arbiter)
    }

    pub fn filter_active_arbiters(
        env: Env,
        candidates: Vec<Address>,
    ) -> Vec<Address> {
        let mut active = Vec::new(&env);

        for arbiter in candidates.iter() {
            if is_active_internal(&env, &arbiter) {
                active.push_back(arbiter);
            }
        }

        active
    }

    pub fn deactivate_arbiter(
        env: Env,
        admin: Address,
        arbiter: Address,
    ) {
        admin.require_auth();
        validate_admin(&env, &admin);

        let registered_key =
            ArbiterMembershipKey::Registered(arbiter.clone());

        if !env.storage()
            .persistent()
            .get::<_, bool>(&registered_key)
            .unwrap_or(false)
        {
            panic!("arbiter is not registered");
        }

        let expiry_key =
            ArbiterMembershipKey::Expiry(arbiter.clone());

        let old_expiry = env.storage()
            .persistent()
            .get::<_, u32>(&expiry_key)
            .unwrap_or(0);

        let now = env.ledger().sequence();

        env.storage()
            .persistent()
            .set(&expiry_key, &now);

        extend_membership_ttl(&env, &arbiter);

        env.events().publish(
            (symbol_short!("arb_deact"), arbiter.clone()),
            now,
        );

        write_audit(
            &env,
            symbol_short!("arb_deact"),
            arbiter,
            admin,
            old_expiry,
            now,
        );
    }
}

fn validate_term(term: u32) {
    if term == 0 {
        panic!("membership term must be positive");
    }

    if term > MAX_TERM_LEDGERS {
        panic!("membership term exceeds maximum");
    }
}

fn validate_admin(env: &Env, caller: &Address) {
    let admin = env.storage()
        .instance()
        .get::<_, Address>(&ArbiterMembershipKey::Admin)
        .unwrap_or_else(|| panic!("membership is not initialized"));

    if admin != *caller {
        panic!("unauthorized administrator");
    }
}

fn read_term(env: &Env) -> u32 {
    let term = env.storage()
        .instance()
        .get::<_, u32>(&ArbiterMembershipKey::MembershipTerm)
        .unwrap_or_else(|| panic!("membership term not configured"));

    validate_term(term);
    term
}

fn validate_address_not_admin(
    env: &Env,
    arbiter: &Address,
) {
    let admin = env.storage()
        .instance()
        .get::<_, Address>(&ArbiterMembershipKey::Admin)
        .unwrap_or_else(|| panic!("membership is not initialized"));

    if admin == *arbiter {
        panic!("administrator cannot be registered as arbiter");
    }
}

fn checked_add(left: u32, right: u32) -> u32 {
    left.checked_add(right)
        .unwrap_or_else(|| panic!("membership expiry overflow"))
}

fn is_active_internal(
    env: &Env,
    arbiter: &Address,
) -> bool {
    let registered = env.storage()
        .persistent()
        .get::<_, bool>(
            &ArbiterMembershipKey::Registered(arbiter.clone()),
        )
        .unwrap_or(false);

    if !registered {
        return false;
    }

    let expiry = env.storage()
        .persistent()
        .get::<_, u32>(
            &ArbiterMembershipKey::Expiry(arbiter.clone()),
        )
        .unwrap_or(0);

    expiry > env.ledger().sequence()
}

fn extend_membership_ttl(
    env: &Env,
    arbiter: &Address,
) {
    let registered_key =
        ArbiterMembershipKey::Registered(arbiter.clone());

    let expiry_key =
        ArbiterMembershipKey::Expiry(arbiter.clone());

    env.storage().persistent().extend_ttl(
        &registered_key,
        TTL_INITIAL_LEDGERS,
        TTL_MAX_LEDGERS,
    );

    env.storage().persistent().extend_ttl(
        &expiry_key,
        TTL_INITIAL_LEDGERS,
        TTL_MAX_LEDGERS,
    );
}

fn emit_initialized(
    env: &Env,
    admin: &Address,
    term: u32,
) {
    env.events().publish(
        (symbol_short!("arb_init"), admin.clone()),
        (term, env.ledger().sequence()),
    );
}

fn emit_term_changed(
    env: &Env,
    admin: &Address,
    old_term: u32,
    new_term: u32,
) {
    let event = TermChangedEvent {
        admin: admin.clone(),
        old_term,
        new_term,
        ledger: env.ledger().sequence(),
    };

    env.events().publish(
        (symbol_short!("term_change"),),
        event,
    );
}

fn emit_registration(
    env: &Env,
    arbiter: &Address,
    expiry: u32,
    term: u32,
) {
    let event = RegistrationEvent {
        arbiter: arbiter.clone(),
        expiry_ledger: expiry,
        term,
        ledger: env.ledger().sequence(),
    };

    env.events().publish(
        (symbol_short!("arb_register"),),
        event,
    );
}

fn emit_renewal(
    env: &Env,
    arbiter: &Address,
    old_expiry: u32,
    new_expiry: u32,
    term: u32,
) {
    let event = RenewalEvent {
        arbiter: arbiter.clone(),
        old_expiry,
        new_expiry,
        term,
        ledger: env.ledger().sequence(),
    };

    env.events().publish(
        (symbol_short!("arb_renew"),),
        event,
    );
}

fn write_audit(
    env: &Env,
    action: Symbol,
    subject: Address,
    actor: Address,
    old_expiry: u32,
    new_expiry: u32,
) {
    let seq_key = ArbiterMembershipKey::AuditSequence;

    let current = env.storage()
        .instance()
        .get::<_, u32>(&seq_key)
        .unwrap_or(0);

    let next = current.checked_add(1)
        .unwrap_or_else(|| panic!("audit sequence overflow"));

    let record = MembershipAuditRecord {
        sequence: next,
        action,
        subject,
        actor,
        old_expiry,
        new_expiry,
        ledger: env.ledger().sequence(),
    };

    env.storage().instance().set(&seq_key, &next);

    env.storage()
        .persistent()
        .set(&(seq_key.clone(), next), &record);

    env.storage().persistent().extend_ttl(
        &(seq_key, next),
        TTL_INITIAL_LEDGERS,
        TTL_MAX_LEDGERS,
    );

    env.events().publish(
        (symbol_short!("arb_audit"),),
        record,
    );
}

pub fn ensure_arbiter_active(
    env: &Env,
    arbiter: &Address,
) {
    if !is_active_internal(env, arbiter) {
        panic!("arbiter membership expired or inactive");
    }
}

pub fn select_active_panel(
    env: &Env,
    candidates: &Vec<Address>,
    required_count: u32,
) -> Vec<Address> {
    if required_count == 0 {
        panic!("panel size must be positive");
    }

    let mut active = Vec::new(env);

    for candidate in candidates.iter() {
        if is_active_internal(env, &candidate)
            && !active.contains(candidate.clone())
        {
            active.push_back(candidate);
        }
    }

    if active.len() < required_count {
        panic!("not enough active arbiters for panel");
    }

    let mut selected = Vec::new(env);
    let mut index = 0;

    while index < required_count {
        selected.push_back(active.get(index).unwrap());
        index += 1;
    }

    selected
}

#[cfg(test)]
mod test_arbiter_membership {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup() -> (
        Env,
        ArbiterMembershipContractClient<'static>,
        Address,
        Address,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(
            ArbiterMembershipContract,
            (),
        );

        let client =
            ArbiterMembershipContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let arbiter = Address::generate(&env);

        client.init_membership(&admin, &100);

        (env, client, admin, arbiter)
    }

    #[test]
    fn registration_starts_active_membership() {
        let (env, client, admin, arbiter) = setup();

        client.register_arbiter(&admin, &arbiter);

        let status = client.get_arbiter_membership(&arbiter);

        assert!(status.registered);
        assert!(status.active);

        assert_eq!(
            status.expiry_ledger,
            env.ledger().sequence() + 100,
        );

        assert_eq!(status.remaining_ledgers, 100);
    }

    #[test]
    fn membership_expires_at_expiry_ledger() {
        let (env, client, admin, arbiter) = setup();

        client.register_arbiter(&admin, &arbiter);

        env.ledger().set_sequence_number(
            env.ledger().sequence() + 100,
        );

        assert!(
            !client.is_arbiter_membership_active(&arbiter)
        );

        let status = client.get_arbiter_membership(&arbiter);

        assert!(!status.active);
        assert_eq!(status.remaining_ledgers, 0);
    }

    #[test]
    fn renewal_extends_expiry() {
        let (_env, client, admin, arbiter) = setup();

        client.register_arbiter(&admin, &arbiter);

        let old = client
            .get_arbiter_membership(&arbiter)
            .expiry_ledger;

        let new_expiry =
            client.renew_arbiter_membership(&arbiter);

        assert_eq!(new_expiry, old + 100);

        assert!(
            client.is_arbiter_membership_active(&arbiter)
        );
    }

    #[test]
    fn late_renewal_starts_from_current_ledger() {
        let (env, client, admin, arbiter) = setup();

        client.register_arbiter(&admin, &arbiter);

        let old_expiry = client
            .get_arbiter_membership(&arbiter)
            .expiry_ledger;

        env.ledger().set_sequence_number(old_expiry + 5);

        let new_expiry =
            client.renew_arbiter_membership(&arbiter);

        assert_eq!(new_expiry, old_expiry + 5 + 100);

        assert!(
            client.is_arbiter_membership_active(&arbiter)
        );
    }

    #[test]
    fn expired_arbiter_is_removed_from_candidates() {
        let (env, client, admin, arbiter) = setup();

        let second = Address::generate(&env);

        client.register_arbiter(&admin, &arbiter);
        client.register_arbiter(&admin, &second);

        let expiry = client
            .get_arbiter_membership(&arbiter)
            .expiry_ledger;

        env.ledger().set_sequence_number(expiry);

        let candidates = Vec::from_array(
            &env,
            [arbiter.clone(), second.clone()],
        );

        let active = client.filter_active_arbiters(&candidates);

        assert_eq!(active.len(), 1);
        assert_eq!(active.get(0).unwrap(), second);
    }

    #[test]
    fn query_reports_expiry_for_registered_arbiter() {
        let (env, client, admin, arbiter) = setup();

        client.register_arbiter(&admin, &arbiter);

        let status = client.get_arbiter_membership(&arbiter);

        assert_eq!(status.arbiter, arbiter);

        assert_eq!(
            status.current_ledger,
            env.ledger().sequence(),
        );

        assert!(
            status.expiry_ledger > status.current_ledger
        );
    }

    #[test]
    #[should_panic(expected = "membership term must be positive")]
    fn zero_term_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(
            ArbiterMembershipContract,
            (),
        );

        let client =
            ArbiterMembershipContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);

        client.init_membership(&admin, &0);
    }

    #[test]
    #[should_panic(expected = "arbiter is not registered")]
    fn unregistered_arbiter_cannot_renew() {
        let (_env, client, _admin, unknown) = setup();

        client.renew_arbiter_membership(&unknown);
    }

    #[test]
    #[should_panic(expected = "arbiter already registered")]
    fn duplicate_registration_is_rejected() {
        let (_env, client, admin, arbiter) = setup();

        client.register_arbiter(&admin, &arbiter);
        client.register_arbiter(&admin, &arbiter);
    }

    #[test]
    #[should_panic(expected = "membership already initialized")]
    fn initialization_cannot_run_twice() {
        let (_env, client, admin, _arbiter) = setup();

        client.init_membership(&admin, &100);
    }
}
