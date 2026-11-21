#![cfg(test)]
//! Issue #1123: provider stake lockup schedule.

use crate::{
    lock_provider, read_provider_lockup, require_lockup_released, StakeVaultError,
    DEFAULT_PROVIDER_LOCKUP_LEDGERS,
};
use soroban_sdk::{
    contract,
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

#[contract]
struct TestContract;

fn setup(sequence: u32) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.sequence_number = sequence);
    let contract_id = env.register(TestContract, ());
    let provider = Address::generate(&env);
    (env, contract_id, provider)
}

#[test]
fn lockup_blocks_through_end_and_releases_after() {
    let (env, contract_id, provider) = setup(5);
    env.as_contract(&contract_id, || {
        lock_provider(&env, &provider, 100);
        let lockup = read_provider_lockup(&env, &provider).unwrap();
        assert_eq!(lockup.start_ledger, 5);
        assert_eq!(lockup.end_ledger, 105);

        // The closing boundary is inclusive: still locked at end_ledger.
        env.ledger().with_mut(|l| l.sequence_number = 105);
        assert_eq!(
            require_lockup_released(&env, &provider),
            Err(StakeVaultError::StakeLocked)
        );

        // Allowed exactly one ledger later, and the window is cleared.
        env.ledger().with_mut(|l| l.sequence_number = 106);
        assert!(require_lockup_released(&env, &provider).is_ok());
        assert!(read_provider_lockup(&env, &provider).is_none());
    });
}

#[test]
fn relock_extends_window_without_moving_start() {
    let (env, contract_id, provider) = setup(5);
    env.as_contract(&contract_id, || {
        lock_provider(&env, &provider, 100); // 5..105
        env.ledger().with_mut(|l| l.sequence_number = 50);
        lock_provider(&env, &provider, 100); // end = max(105, 150)
        let lockup = read_provider_lockup(&env, &provider).unwrap();
        assert_eq!(lockup.start_ledger, 5);
        assert_eq!(lockup.end_ledger, 150);
    });
}

#[test]
fn expired_lockup_restarts_from_current_ledger() {
    let (env, contract_id, provider) = setup(5);
    env.as_contract(&contract_id, || {
        lock_provider(&env, &provider, 100); // 5..105
        env.ledger().with_mut(|l| l.sequence_number = 200);
        lock_provider(&env, &provider, 10); // fresh 200..210
        let lockup = read_provider_lockup(&env, &provider).unwrap();
        assert_eq!(lockup.start_ledger, 200);
        assert_eq!(lockup.end_ledger, 210);
    });
}

#[test]
fn zero_length_lockup_is_a_noop() {
    let (env, contract_id, provider) = setup(5);
    env.as_contract(&contract_id, || {
        lock_provider(&env, &provider, 0);
        assert!(read_provider_lockup(&env, &provider).is_none());
        assert!(require_lockup_released(&env, &provider).is_ok());
    });
}

#[test]
fn default_lockup_length_is_the_documented_constant() {
    let (env, contract_id, _) = setup(1);
    env.as_contract(&contract_id, || {
        assert_eq!(
            crate::default_lockup_ledgers(&env),
            DEFAULT_PROVIDER_LOCKUP_LEDGERS
        );
    });
}
