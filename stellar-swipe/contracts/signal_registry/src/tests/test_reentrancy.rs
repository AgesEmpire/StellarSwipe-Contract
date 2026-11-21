#![cfg(test)]
//! Reentrancy guard tests (Issue #264, extended by the cross-contract audit
//! in Issue #781).
//!
//! `unstake_tokens` and `ban_provider` both run under the shared,
//! contract-wide lock in [`crate::reentrancy`]. These tests exercise the
//! lock in isolation (fast, no cross-contract registration needed); the
//! adversarial end-to-end scenario — a malicious `StakeVault` attempting to
//! reenter `ban_provider` mid-call — is covered by the integration test
//! `test_signal_registry_reentrancy` in `contracts/integration_tests`.

use crate::errors::AdminError;
use crate::{reentrancy, SignalRegistry, SignalRegistryClient};
use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env, String};

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

/// Minimal `StakeVault` stand-in exposing only `get_stake`, always returning
/// zero so `ban_provider`'s slash sub-call is skipped. Sufficient for
/// exercising the guard/lock mechanics around `ban_provider` without pulling
/// in the real `stake_vault` crate as a dev-dependency of `signal_registry`.
#[contract]
struct ZeroStakeVaultStub;

#[contractimpl]
impl ZeroStakeVaultStub {
    pub fn get_stake(_env: Env, _staker: Address) -> i128 {
        0
    }
}

fn zero_stake_vault(env: &Env) -> Address {
    #[allow(deprecated)]
    env.register_contract(None, ZeroStakeVaultStub)
}

// ── unstake_tokens ──────────────────────────────────────────────────────────

/// Simulate a reentrant call by manually holding the shared guard before
/// calling `unstake_tokens`. The function must return `ReentrancyDetected`
/// without modifying any state.
#[test]
fn unstake_tokens_rejects_reentrant_call() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);

    // Stake enough to be eligible for unstaking.
    client.stake_tokens(&provider, &100_000_000i128);

    // Simulate reentrancy: hold the lock as if a nested call is in progress.
    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || reentrancy::force_lock(&env));

    // The call must be rejected with ReentrancyDetected.
    let err = client.try_unstake_tokens(&provider).unwrap_err().unwrap();
    assert_eq!(err, AdminError::ReentrancyDetected);

    // Clear the simulated lock; a legitimate unstake then succeeds.
    env.as_contract(&contract_id, || reentrancy::force_unlock(&env));
    client.unstake_tokens(&provider);
}

/// Verify the lock is cleared after a successful unstake (no lock leak).
#[test]
fn unstake_tokens_clears_lock_on_success() {
    let (env, _, client) = setup();
    let provider = Address::generate(&env);

    client.stake_tokens(&provider, &100_000_000i128);
    client.unstake_tokens(&provider);

    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        assert!(
            !reentrancy::is_locked(&env),
            "guard was not released after successful unstake"
        );
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

    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        assert!(
            !reentrancy::is_locked(&env),
            "guard was not released after failed unstake"
        );
    });
}

// ── ban_provider ─────────────────────────────────────────────────────────

/// Simulate a reentrant call arriving during `ban_provider`: the guard must
/// reject it with `ReentrancyDetected`, and — because `apply_ban` persists
/// its effects before any external call — the provider must not be left
/// half-banned.
#[test]
fn ban_provider_rejects_reentrant_call() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let stake_vault = zero_stake_vault(&env);
    let reason = String::from_str(&env, "ipfs://evidence");

    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || reentrancy::force_lock(&env));

    let err = client
        .try_ban_provider(&admin, &provider, &reason, &stake_vault)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AdminError::ReentrancyDetected);

    // Rejected before any effect ran: provider is not banned.
    assert!(!client.is_provider_banned(&provider));

    env.as_contract(&contract_id, || reentrancy::force_unlock(&env));
}

/// Verify the lock is cleared after a successful ban (no lock leak), and
/// that the ban itself took effect.
#[test]
fn ban_provider_clears_lock_on_success() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let stake_vault = zero_stake_vault(&env);
    let reason = String::from_str(&env, "ipfs://evidence");

    client.ban_provider(&admin, &provider, &reason, &stake_vault);
    assert!(client.is_provider_banned(&provider));

    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        assert!(
            !reentrancy::is_locked(&env),
            "guard was not released after successful ban_provider"
        );
    });
}

/// The guard is contract-wide: holding it blocks *any* guarded entrypoint,
/// not just the one that acquired it. This is what stops a malicious
/// `stake_vault` invoked from `ban_provider` from reentering via a
/// different guarded function (e.g. `unstake_tokens`).
#[test]
fn guard_blocks_across_entrypoints() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let stake_vault = zero_stake_vault(&env);
    let reason = String::from_str(&env, "ipfs://evidence");
    client.stake_tokens(&provider, &100_000_000i128);

    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || reentrancy::force_lock(&env));

    assert_eq!(
        client.try_unstake_tokens(&provider).unwrap_err().unwrap(),
        AdminError::ReentrancyDetected
    );
    assert_eq!(
        client
            .try_ban_provider(&admin, &provider, &reason, &stake_vault)
            .unwrap_err()
            .unwrap(),
        AdminError::ReentrancyDetected
    );

    env.as_contract(&contract_id, || reentrancy::force_unlock(&env));
}
