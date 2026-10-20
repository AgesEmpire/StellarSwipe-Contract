//! Cross-contract reentrancy guard (Issue #781).
//!
//! `signal_registry` calls out to other Soroban contracts (e.g. `StakeVault`
//! via [`crate::providers::ban_provider`]) from a handful of state-changing
//! entrypoints. Soroban permits a callee to invoke back into the caller
//! within the same transaction, so any entrypoint that writes storage on
//! either side of an external call is a reentrancy candidate.
//!
//! The guard follows the pattern already established for `unstake_tokens`
//! (Issue #264): a boolean flag in *temporary* storage, set for the duration
//! of the guarded call and cleared before returning on every path (success
//! or error). Temporary storage is used because the lock only needs to
//! survive for the lifetime of the current transaction — it must not persist
//! (or cost rent) across ledgers.
//!
//! A single, contract-wide lock is used rather than one lock per entrypoint.
//! This matches the classic `nonReentrant` modifier pattern (e.g. OpenZeppelin's
//! `ReentrancyGuard`): it is simpler to reason about, and it also blocks
//! *cross-entrypoint* reentrancy (e.g. a malicious callee invoked from
//! `ban_provider` trying to call back into a different guarded function)
//! which a per-function lock would miss.
//!
//! Guarded entrypoints must persist all storage effects *before* making the
//! external call (checks-effects-interactions) — the lock is defense in
//! depth, not a substitute for that ordering. See the call-site comments in
//! `providers.rs` and `lib.rs` for the per-site risk assessment.

use crate::errors::AdminError;
use soroban_sdk::{Env, Symbol};

fn lock_key(env: &Env) -> Symbol {
    Symbol::new(env, "XContractLock")
}

/// Returns `Err(AdminError::ReentrancyDetected)` if the contract-wide guard
/// is currently held (i.e. this call is nested inside another guarded call).
pub fn require_not_locked(env: &Env) -> Result<(), AdminError> {
    let locked = env
        .storage()
        .temporary()
        .get::<_, bool>(&lock_key(env))
        .unwrap_or(false);
    if locked {
        return Err(AdminError::ReentrancyDetected);
    }
    Ok(())
}

/// Runs `f` under the reentrancy guard. The lock is acquired *before* `f`
/// runs and released after it returns, regardless of whether `f` succeeded —
/// so a failed guarded call never leaves the lock held for the rest of the
/// transaction.
///
/// Callers are responsible for persisting all storage effects inside `f`
/// before performing the external call, per checks-effects-interactions.
pub fn guarded<F, T>(env: &Env, f: F) -> Result<T, AdminError>
where
    F: FnOnce() -> Result<T, AdminError>,
{
    require_not_locked(env)?;
    let key = lock_key(env);
    env.storage().temporary().set(&key, &true);
    let result = f();
    env.storage().temporary().remove(&key);
    result
}

/// Test-only introspection into the guard's lock state, so tests can assert
/// on lock/unlock behaviour without depending on the private storage key.
#[cfg(any(test, feature = "testutils"))]
pub fn is_locked(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get::<_, bool>(&lock_key(env))
        .unwrap_or(false)
}

/// Test-only helper to simulate a reentrant call arriving while the guard is
/// already held (i.e. as if a nested cross-contract call were in progress).
#[cfg(any(test, feature = "testutils"))]
pub fn force_lock(env: &Env) {
    env.storage().temporary().set(&lock_key(env), &true);
}

/// Test-only helper to clear a simulated lock.
#[cfg(any(test, feature = "testutils"))]
pub fn force_unlock(env: &Env) {
    env.storage().temporary().remove(&lock_key(env));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_blocks_nested_call() {
        let env = Env::default();
        let contract_id = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&contract_id, || {
            let outcome = guarded(&env, || {
                // Attempt a nested guarded call while the outer one is active.
                let nested = guarded(&env, || Ok::<_, AdminError>(1));
                assert_eq!(nested, Err(AdminError::ReentrancyDetected));
                Ok::<_, AdminError>(())
            });
            assert!(outcome.is_ok());

            // Lock must be released after the outer call completes.
            assert!(require_not_locked(&env).is_ok());
        });
    }

    #[test]
    fn guarded_releases_lock_on_error() {
        let env = Env::default();
        let contract_id = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&contract_id, || {
            let outcome = guarded(&env, || Err::<(), AdminError>(AdminError::InvalidParameter));
            assert_eq!(outcome, Err(AdminError::InvalidParameter));
            assert!(require_not_locked(&env).is_ok());
        });
    }
}
