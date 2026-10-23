#![cfg(test)]

extern crate std;

// Lending lifecycle tests: liquidity, borrowing, repayment, access control,
// and the pause circuit breaker.

use crate::test::test_common::{open_position, seed_liquidity, setup, Setup};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address};

fn debt_balance(s: &Setup, user: &Address) -> i128 {
    token::Client::new(&s.env, &s.debt_id).balance(user)
}

fn coll_balance(s: &Setup, user: &Address) -> i128 {
    token::Client::new(&s.env, &s.coll_id).balance(user)
}

// ============================================================
// LIQUIDITY
// ============================================================

#[test]
fn test_deposit_and_withdraw_roundtrip() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1, &s.d2], &[5_000, 3_000]);

    let (total_deposits, total_borrowed, _) = s.client().get_pool_info();
    assert_eq!(total_deposits, 8_000);
    assert_eq!(total_borrowed, 0);

    // Contract custody matches accounting.
    assert_eq!(debt_balance(&s, &s.id), 8_000);

    s.client().withdraw(&s.d2, &1_000);
    let (td, _, _) = s.client().get_pool_info();
    assert_eq!(td, 7_000);
}

#[test]
#[should_panic(expected = "insufficient depositor balance")]
fn test_withdraw_more_than_balance_panics() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[1_000]);
    s.client().withdraw(&s.d1, &1_500);
}

#[test]
#[should_panic(expected = "insufficient pool liquidity")]
fn test_withdraw_beyond_free_liquidity_panics() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 4_000, 2_000); // locks 2000 in the pool
    s.client().withdraw(&s.d1, &9_000); // only 8000 free
}

#[test]
#[should_panic(expected = "invalid amount")]
fn test_zero_deposit_rejected() {
    let s = setup();
    s.client().deposit(&s.d1, &0i128);
}

// ============================================================
// BORROW / REPAY
// ============================================================

#[test]
fn test_borrow_within_collateral_succeeds() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 1_500);

    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.collateral, 2_000);
    assert_eq!(pos.debt, 1_500);

    // HF fraction: num/den = (2000*8000)/(1500*10000) > 1 → healthy.
    let (num, den) = s.client().health_factor(&s.alice);
    assert!(num >= den);
}

#[test]
#[should_panic(expected = "insufficient collateral")]
fn test_borrow_beyond_threshold_panics() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    // HF would be (1000*8000)/(900*10000) < 1.
    open_position(&s, &s.alice, 1_000, 900);
}

#[test]
#[should_panic(expected = "insufficient pool liquidity")]
fn test_borrow_beyond_pool_liquidity_panics() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[1_000]);
    open_position(&s, &s.alice, 50_000, 1_001);
}

#[test]
fn test_repay_restores_capacity() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 1_500);

    s.client().repay(&s.alice, &500);
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.debt, 1_000);

    // Over-repayment is capped at outstanding debt.
    s.client().repay(&s.alice, &99_999);
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.debt, 0);

    let (_, tb, _) = s.client().get_pool_info();
    assert_eq!(tb, 0);
}

#[test]
fn test_withdraw_collateral_respects_health() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 1_000);

    // Pulling 600 keeps (1400*8000) >= 1000*10000 → allowed.
    s.client().withdraw_collateral(&s.alice, &600);
    assert_eq!(coll_balance(&s, &s.alice), 100_000_000_000 - 1_400);

    // Pulling more would undercollateralize.
    let res = s.client().try_withdraw_collateral(&s.alice, &500);
    assert!(res.is_err());
}

// ============================================================
// ACCESS CONTROL & INIT
// ============================================================

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_set_oracles() {
    let s = setup();
    let stranger = Address::generate(&s.env);
    s.client().set_oracles(
        &stranger,
        &Some(s.primary.clone()),
        &Some(s.secondary.clone()),
    );
}

#[test]
#[should_panic(expected = "unauthorized")]
fn test_non_admin_cannot_pause() {
    let s = setup();
    let stranger = Address::generate(&s.env);
    s.client().pause(&stranger);
}

#[test]
#[should_panic(expected = "already initialized")]
fn test_double_init_panics() {
    let s = setup();
    s.client().init(&s.admin);
}

#[test]
fn test_ops_fail_before_market_configured() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    let id = env.register(crate::LoanManagerContract, ());
    let client = crate::LoanManagerContractClient::new(&env, &id);
    // deposit without init/market → panics.
    let user = Address::generate(&env);
    let res = client.try_deposit(&user, &100i128);
    assert!(res.is_err(), "uninitialized contract must reject deposits");
}

// ============================================================
// PAUSE CIRCUIT BREAKER
// ============================================================

#[test]
fn test_pause_blocks_state_changes_but_not_views() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[5_000]);
    open_position(&s, &s.alice, 2_000, 1_000);

    s.client().pause(&s.admin);
    assert!(s.client().is_paused_view());

    // State-changing calls are halted…
    assert!(s.client().try_deposit(&s.d1, &100i128).is_err());
    assert!(s.client().try_borrow(&s.alice, &100i128).is_err());
    assert!(s.client().try_repay(&s.alice, &100i128).is_err());
    assert!(s.client().try_liquidate(&s.bob, &s.alice, &0i128).is_err());

    // …while read-only views keep working.
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.debt, 1_000);

    s.client().unpause(&s.admin);
    assert!(!s.client().is_paused_view());
    // d1's balance was fully consumed by the seed deposit; top up first.
    token::StellarAssetClient::new(&s.env, &s.debt_id).mint(&s.d1, &1_000);
    s.client().deposit(&s.d1, &100); // works again
}

// ============================================================
// ADDITIONAL COVERAGE
// ============================================================

#[test]
fn test_depositor_registry_is_deduplicated() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1, &s.d1, &s.d2], &[1_000, 500, 2_000]);

    let depositors = s.client().get_depositors();
    assert_eq!(depositors.len(), 2);
    assert_eq!(depositors.get(0).unwrap(), s.d1);
    assert_eq!(depositors.get(1).unwrap(), s.d2);
    assert_eq!(s.client().get_depositor_balance(&s.d1), 1_500);
    assert_eq!(s.client().get_depositor_balance(&s.d2), 2_000);
}

#[test]
fn test_full_withdraw_zeroes_depositor_balance() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[4_000]);

    s.client().withdraw(&s.d1, &4_000);
    assert_eq!(s.client().get_depositor_balance(&s.d1), 0);
    assert_eq!(debt_balance(&s, &s.d1), 4_000);
    assert_eq!(debt_balance(&s, &s.id), 0);

    let (td, _, _) = s.client().get_pool_info();
    assert_eq!(td, 0);
}

#[test]
#[should_panic(expected = "invalid amount")]
fn test_zero_withdraw_rejected() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[1_000]);
    s.client().withdraw(&s.d1, &0i128);
}

#[test]
#[should_panic(expected = "invalid amount")]
fn test_negative_borrow_rejected() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 0);
    s.client().borrow(&s.alice, &-1i128);
}

#[test]
fn test_borrow_exactly_at_threshold_succeeds() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    // HF = (1000*8000)/(800*10000) == 1 → exactly healthy.
    open_position(&s, &s.alice, 1_000, 800);

    let (num, den) = s.client().health_factor(&s.alice);
    assert_eq!(num, den);
    assert!(s.client().try_borrow(&s.alice, &1i128).is_err());
}

#[test]
fn test_borrow_transfers_tokens_and_updates_pool() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    let before = debt_balance(&s, &s.alice);

    open_position(&s, &s.alice, 5_000, 3_000);

    assert_eq!(debt_balance(&s, &s.alice), before + 3_000);
    assert_eq!(debt_balance(&s, &s.id), 7_000);
    assert_eq!(coll_balance(&s, &s.id), 5_000);

    let (td, tb, _) = s.client().get_pool_info();
    assert_eq!(td, 10_000);
    assert_eq!(tb, 3_000);
}

#[test]
#[should_panic(expected = "nothing to repay")]
fn test_repay_without_debt_panics() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 0);
    s.client().repay(&s.alice, &100);
}

#[test]
fn test_health_factor_without_debt_is_max() {
    let s = setup();
    open_position(&s, &s.alice, 2_000, 0);

    let (num, den) = s.client().health_factor(&s.alice);
    assert_eq!(num, i128::MAX);
    assert_eq!(den, 1);
}

#[test]
fn test_withdraw_all_collateral_after_full_repay() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 1_000);

    s.client().repay(&s.alice, &1_000);
    s.client().withdraw_collateral(&s.alice, &2_000);

    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.collateral, 0);
    assert_eq!(pos.debt, 0);
    assert_eq!(coll_balance(&s, &s.alice), 100_000_000_000);
}

#[test]
#[should_panic(expected = "insufficient collateral")]
fn test_withdraw_collateral_more_than_posted_panics() {
    let s = setup();
    open_position(&s, &s.alice, 1_000, 0);
    s.client().withdraw_collateral(&s.alice, &1_001);
}

// ============================================================
// ADDITIONAL TEST CASES
// ============================================================

#[test]
fn test_multiple_borrowers_independent_positions() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[20_000]);
    
    open_position(&s, &s.alice, 3_000, 2_000);
    open_position(&s, &s.bob, 4_000, 2_500);
    
    let pos_alice = s.client().get_position(&s.alice);
    let pos_bob = s.client().get_position(&s.bob);
    
    assert_eq!(pos_alice.collateral, 3_000);
    assert_eq!(pos_alice.debt, 2_000);
    assert_eq!(pos_bob.collateral, 4_000);
    assert_eq!(pos_bob.debt, 2_500);
    
    let (_, tb, _) = s.client().get_pool_info();
    assert_eq!(tb, 4_500);
}

#[test]
fn test_partial_collateral_withdrawal_with_debt() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 5_000, 2_000);
    
    // Withdraw collateral but maintain health factor
    s.client().withdraw_collateral(&s.alice, &1_000);
    
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.collateral, 4_000);
    assert_eq!(pos.debt, 2_000);
    
    let (num, den) = s.client().health_factor(&s.alice);
    assert!(num >= den);
}

#[test]
fn test_sequential_deposits_accumulate_correctly() {
    let s = setup();

    seed_liquidity(&s, &[&s.d1], &[1_000]);
    seed_liquidity(&s, &[&s.d1], &[2_000]);
    seed_liquidity(&s, &[&s.d1], &[3_000]);
    
    let balance = s.client().get_depositor_balance(&s.d1);
    assert_eq!(balance, 6_000);
    
    let (td, _, _) = s.client().get_pool_info();
    assert_eq!(td, 6_000);
}

#[test]
fn test_sequential_withdrawals_decrease_correctly() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    
    s.client().withdraw(&s.d1, &2_000);
    s.client().withdraw(&s.d1, &3_000);
    
    let balance = s.client().get_depositor_balance(&s.d1);
    assert_eq!(balance, 5_000);
    
    let (td, _, _) = s.client().get_pool_info();
    assert_eq!(td, 5_000);
}

#[test]
fn test_partial_repayment_improves_health_factor() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 1_500);
    
    let (num_before, den_before) = s.client().health_factor(&s.alice);
    
    s.client().repay(&s.alice, &500);
    
    let (num_after, den_after) = s.client().health_factor(&s.alice);
    
    // Health factor should improve: higher ratio after partial repayment
    assert!(num_after * den_before > num_before * den_after);
}

#[test]
fn test_borrow_additional_amount_on_existing_position() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 5_000, 1_000);
    
    // Borrow more against existing collateral
    s.client().borrow(&s.alice, &500);
    
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.collateral, 5_000);
    assert_eq!(pos.debt, 1_500);
    
    let (_, tb, _) = s.client().get_pool_info();
    assert_eq!(tb, 1_500);
}

#[test]
fn test_add_collateral_to_existing_position() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 2_000, 1_500);
    
    // Add more collateral to improve position
    token::StellarAssetClient::new(&s.env, &s.coll_id).mint(&s.alice, &1_000);
    s.client().deposit_collateral(&s.alice, &1_000);
    
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.collateral, 3_000);
    assert_eq!(pos.debt, 1_500);
}

#[test]
#[should_panic(expected = "invalid amount")]
fn test_zero_collateral_deposit_rejected() {
    let s = setup();
    s.client().deposit_collateral(&s.alice, &0i128);
}

#[test]
fn test_multiple_depositors_withdraw_independently() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1, &s.d2], &[5_000, 3_000]);
    
    s.client().withdraw(&s.d1, &2_000);
    
    assert_eq!(s.client().get_depositor_balance(&s.d1), 3_000);
    assert_eq!(s.client().get_depositor_balance(&s.d2), 3_000);
    
    let (td, _, _) = s.client().get_pool_info();
    assert_eq!(td, 6_000);
}

#[test]
fn test_repay_exactly_outstanding_debt() {
    let s = setup();
    seed_liquidity(&s, &[&s.d1], &[10_000]);
    open_position(&s, &s.alice, 3_000, 2_000);
    
    s.client().repay(&s.alice, &2_000);
    
    let pos = s.client().get_position(&s.alice);
    assert_eq!(pos.debt, 0);
    assert_eq!(pos.collateral, 3_000);
    
    let (_, tb, _) = s.client().get_pool_info();
    assert_eq!(tb, 0);
}
