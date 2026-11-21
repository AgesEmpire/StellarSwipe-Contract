/// Shared double-initialization guard (issue #584).
///
/// Provides a standardized initialized-flag storage key and guard check
/// reusable by any contract crate, eliminating duplicated inline patterns.
///
/// # Usage
/// ```ignore
/// use shared::initializable;
///
/// fn initialize(env: &Env, ...) -> Result<(), MyError> {
///     initializable::guard_not_initialized(env)?;
///     // ... perform init ...
///     initializable::mark_initialized(env);
///     Ok(())
/// }
/// ```
use soroban_sdk::{contracttype, Env};

#[contracttype]
#[derive(Clone)]
pub enum InitializableKey {
    Initialized,
}

/// Returns `true` if the contract has already been initialized.
pub fn is_initialized(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&InitializableKey::Initialized)
        .unwrap_or(false)
}

/// Persist the initialized flag so future calls to [`is_initialized`] return `true`.
pub fn mark_initialized(env: &Env) {
    env.storage()
        .instance()
        .set(&InitializableKey::Initialized, &true);
}

/// Returns `Err(true)` (a sentinel) if the contract is already initialized,
/// `Ok(())` otherwise. Callers translate the sentinel into their own error type.
///
/// Using a concrete boolean sentinel rather than a generic keeps the shared
/// crate free of contract-specific error enums.
pub fn guard_not_initialized(env: &Env) -> Result<(), bool> {
    if is_initialized(env) {
        Err(true)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl, Address, Env};

    #[contract]
    struct TestContract;

    #[contractimpl]
    impl TestContract {
        pub fn init(env: Env) -> bool {
            guard_not_initialized(&env).is_err()
        }
        pub fn mark(env: Env) {
            mark_initialized(&env);
        }
        pub fn initialized(env: Env) -> bool {
            is_initialized(&env)
        }
    }

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(TestContract, ());
        (env, id)
    }

    #[test]
    fn fresh_contract_is_not_initialized() {
        let (env, id) = setup();
        let client = TestContractClient::new(&env, &id);
        assert!(!client.initialized());
    }

    #[test]
    fn after_mark_is_initialized() {
        let (env, id) = setup();
        let client = TestContractClient::new(&env, &id);
        client.mark();
        assert!(client.initialized());
    }

    #[test]
    fn guard_succeeds_before_init() {
        let (env, id) = setup();
        let client = TestContractClient::new(&env, &id);
        // init() returns true when guard returns Err (already initialized) — should be false here.
        assert!(!client.init());
    }

    #[test]
    fn guard_fails_after_init() {
        let (env, id) = setup();
        let client = TestContractClient::new(&env, &id);
        client.mark();
        assert!(client.init());
    }
}
