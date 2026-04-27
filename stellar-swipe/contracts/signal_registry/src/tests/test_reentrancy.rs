#![cfg(test)]
//! Reentrancy guard tests for `unstake_tokens` (Issue #264).

use crate::errors::AdminError;
use crate::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, admin, client)
}

/// Simulate a reentrant call by manually setting the `UnstakeLock` flag in
/// temporary storage before calling `unstake_tokens`. The function must return
/// `ReentrancyDetected` without modifying any state.
#[test]
fn unstake_tokens_rejects_reentrant_call() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);

    // Stake enough to be eligible for unstaking.
    client.stake_tokens(&provider, &100_000_000i128).unwrap();

    // Simulate reentrancy: set the lock flag as if a reentrant call is in progress.
    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        let lock_key = Symbol::new(&env, "UnstakeLock");
        env.storage().temporary().set(&lock_key, &true);
    });

    // The call must be rejected with ReentrancyDetected.
    let err = client.try_unstake_tokens(&provider).unwrap_err().unwrap();
    assert_eq!(err, AdminError::ReentrancyDetected);

    // State must be unchanged: stake balance still present.
    env.as_contract(&contract_id, || {
        let lock_key = Symbol::new(&env, "UnstakeLock");
        // Clear the simulated lock so we can verify stake state.
        env.storage().temporary().remove(&lock_key);
    });

    // After clearing the simulated lock, a legitimate unstake succeeds.
    client.unstake_tokens(&provider).unwrap();
}

/// Verify the lock is cleared after a successful unstake (no lock leak).
#[test]
fn unstake_tokens_clears_lock_on_success() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);

    client.stake_tokens(&provider, &100_000_000i128).unwrap();
    client.unstake_tokens(&provider).unwrap();

    // Lock must not be set after a successful call.
    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        let lock_key = Symbol::new(&env, "UnstakeLock");
        let locked: bool = env
            .storage()
            .temporary()
            .get(&lock_key)
            .unwrap_or(false);
        assert!(!locked, "UnstakeLock was not cleared after successful unstake");
    });
}

/// Verify the lock is cleared after a failed unstake (no lock leak on error).
#[test]
fn unstake_tokens_clears_lock_on_error() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);

    // No stake — unstake will fail with InvalidParameter.
    let err = client.try_unstake_tokens(&provider).unwrap_err().unwrap();
    assert_eq!(err, AdminError::InvalidParameter);

    // Lock must not be set after a failed call.
    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        let lock_key = Symbol::new(&env, "UnstakeLock");
        let locked: bool = env
            .storage()
            .temporary()
            .get(&lock_key)
            .unwrap_or(false);
        assert!(!locked, "UnstakeLock was not cleared after failed unstake");
    });
}
