//! Cross-contract reentrancy guard (Issue #859).
//!
//! Provides a contract-wide reentrancy guard that can be used by any contract
//! performing cross-contract token transfers or other external calls. Modeled
//! after the existing guard in `signal_registry/src/reentrancy.rs` (Issue #781).
//!
//! A single boolean lock is stored in **temporary** storage so the lock
//! naturally expires at the end of the transaction and never persists (or
//! incurs rent) across ledgers.
//!
//! # Usage
//!
//! ```ignore
//! use shared::reentrancy;
//!
//! fn guarded_function(env: Env, ...) -> Result<(), MyError> {
//!     reentrancy::guarded(&env, || {
//!         // Persist all state *before* making the external call
//!         // (checks-effects-interactions).
//!         do_work(&env)?;
//!         make_external_call(&env)?;
//!         Ok(())
//!     })?;
//!     Ok(())
//! }
//! ```

use soroban_sdk::{contracterror, Env, Symbol};

/// Error returned when a reentrant call is detected.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ReentrancyError {
    ReentrancyDetected = 1,
}

fn lock_key(env: &Env) -> Symbol {
    Symbol::new(env, "SharedContractLock")
}

/// Returns `Err(ReentrancyError::ReentrancyDetected)` if the contract-wide
/// guard is currently held.
pub fn require_not_locked(env: &Env) -> Result<(), ReentrancyError> {
    let locked = env
        .storage()
        .temporary()
        .get::<_, bool>(&lock_key(env))
        .unwrap_or(false);
    if locked {
        return Err(ReentrancyError::ReentrancyDetected);
    }
    Ok(())
}

/// Runs `f` under the reentrancy guard. The lock is acquired before `f` runs
/// and released after it returns, regardless of whether `f` succeeded.
pub fn guarded<F, T, E>(env: &Env, f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
    E: From<ReentrancyError>,
{
    require_not_locked(env)?;
    let key = lock_key(env);
    env.storage().temporary().set(&key, &true);
    let result = f();
    env.storage().temporary().remove(&key);
    result
}

/// Test-only introspection into the guard's lock state.
#[cfg(any(test, feature = "testutils"))]
pub fn is_locked(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get::<_, bool>(&lock_key(env))
        .unwrap_or(false)
}

/// Test-only helper to simulate a reentrant call arriving while the guard is held.
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
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};

    #[contract]
    struct TestContract;

    #[contractimpl]
    impl TestContract {
        pub fn guarded_call(env: Env) -> Result<i32, ReentrancyError> {
            guarded(&env, || {
                // Simulate nested call attempt
                let nested: Result<i32, ReentrancyError> = guarded(&env, || Ok(1));
                assert_eq!(nested, Err(ReentrancyError::ReentrancyDetected));
                Ok(42)
            })
        }

        pub fn guarded_call_err(env: Env) -> Result<i32, ReentrancyError> {
            guarded(&env, || Err(ReentrancyError::ReentrancyDetected))?;
            Ok(1)
        }

        pub fn check_locked(env: Env) -> bool {
            is_locked(&env)
        }
    }

    #[test]
    fn guarded_blocks_nested_call() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestContract);
        let client = TestContractClient::new(&env, &contract_id);
        let result = client.guarded_call();
        assert_eq!(result, 42);
        assert!(!client.check_locked());
    }

    #[test]
    fn guard_releases_lock_on_error() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestContract);
        let client = TestContractClient::new(&env, &contract_id);
        let result = client.try_guarded_call_err();
        assert!(result.is_err());
        assert!(!client.check_locked());
    }

    #[test]
    fn guard_blocks_when_lock_held() {
        let env = Env::default();
        let contract_id = env.register_contract(None, TestContract);
        let client = TestContractClient::new(&env, &contract_id);

        env.as_contract(&contract_id, || force_lock(&env));
        let result = client.try_guarded_call();
        assert_eq!(result, Err(Ok(ReentrancyError::ReentrancyDetected)));
        env.as_contract(&contract_id, || force_unlock(&env));
        let result = client.guarded_call();
        assert_eq!(result, 42);
    }
}
