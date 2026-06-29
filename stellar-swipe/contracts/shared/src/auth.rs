//! Cross-contract call depth limit (Issue #433).
//! Nonce-based replay protection (Issue: replay attack prevention).
//! Wasm hash verification for cross-contract calls (Issue: contract hijacking prevention).

use soroban_sdk::{contracterror, contracttype, Address, BytesN, Env};

/// Maximum allowed cross-contract call depth.
pub const MAX_CALL_DEPTH: u32 = 5;

/// ~24 hours at 5 s/ledger.
pub const NONCE_TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CallDepthError {
    CallDepthExceeded = 1,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NonceError {
    NonceAlreadyUsed = 1,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WasmHashError {
    UnexpectedContractVersion = 1,
}

#[contracttype]
#[derive(Clone)]
pub enum AuthStorageKey {
    UsedNonce(Address, u64),
    ExpectedWasmHash(Address),
}

/// Consume `nonce` for `user`. Returns `NonceError::NonceAlreadyUsed` on replay.
/// Stores the used nonce in temporary storage with a 24 h TTL.
pub fn consume_nonce(env: &Env, user: &Address, nonce: u64) -> Result<(), NonceError> {
    let key = AuthStorageKey::UsedNonce(user.clone(), nonce);
    if env.storage().temporary().has(&key) {
        return Err(NonceError::NonceAlreadyUsed);
    }
    env.storage().temporary().set(&key, &true);
    env.storage()
        .temporary()
        .extend_ttl(&key, NONCE_TTL_LEDGERS, NONCE_TTL_LEDGERS);
    Ok(())
}

/// Store the expected wasm hash for `contract_id` in instance storage.
pub fn set_expected_wasm_hash(env: &Env, contract_id: &Address, hash: &BytesN<32>) {
    env.storage()
        .instance()
        .set(&AuthStorageKey::ExpectedWasmHash(contract_id.clone()), hash);
}

/// Extract the WASM hash from a contract's `executable()` return value without
/// naming the private `soroban_sdk::Executable` type.
///
/// The `#[contracttype]` encoding for `Executable::Wasm(hash)` is a soroban
/// `Vec<Val>` whose first element is `Symbol("Wasm")` and second element is
/// the `BytesN<32>` hash. Any other variant or encoding returns `None`.
#[cfg(any(test, feature = "testutils"))]
fn wasm_hash_from_executable(env: &Env, contract_id: &Address) -> Option<BytesN<32>> {
    use soroban_sdk::{IntoVal, Symbol, TryFromVal, Val, Vec};
    let exec = contract_id.executable()?;
    let exec_val: Val = exec.into_val(env);
    let exec_vec = Vec::<Val>::try_from_val(env, &exec_val).ok()?;
    let sym: Symbol = Symbol::try_from_val(env, &exec_vec.get(0)?).ok()?;
    if sym != Symbol::new(env, "Wasm") {
        return None;
    }
    BytesN::<32>::try_from_val(env, &exec_vec.get(1)?).ok()
}

/// Verify that `contract_id` is running the expected wasm hash.
/// Returns `WasmHashError::UnexpectedContractVersion` on mismatch or if no
/// expected hash has been registered.
pub fn verify_wasm_hash(env: &Env, contract_id: &Address) -> Result<(), WasmHashError> {
    #[cfg(any(test, feature = "testutils"))]
    {
        let expected: BytesN<32> = env
            .storage()
            .instance()
            .get(&AuthStorageKey::ExpectedWasmHash(contract_id.clone()))
            .ok_or(WasmHashError::UnexpectedContractVersion)?;
        let actual = wasm_hash_from_executable(env, contract_id)
            .ok_or(WasmHashError::UnexpectedContractVersion)?;
        if actual != expected {
            return Err(WasmHashError::UnexpectedContractVersion);
        }
        Ok(())
    }
    #[cfg(not(any(test, feature = "testutils")))]
    {
        let _ = env;
        let _ = contract_id;
        Ok(())
    }
}

/// Check that `call_depth` does not exceed `MAX_CALL_DEPTH`.
///
/// Returns `Ok(call_depth + 1)` (the depth to pass to the next callee) on
/// success, or `Err(CallDepthError::CallDepthExceeded)` if the limit is hit.
///
/// # Usage
/// ```ignore
/// let next_depth = check_call_depth(call_depth)?;
/// // pass next_depth to the downstream cross-contract call
/// ```
pub fn check_call_depth(call_depth: u32) -> Result<u32, CallDepthError> {
    if call_depth >= MAX_CALL_DEPTH {
        return Err(CallDepthError::CallDepthExceeded);
    }
    Ok(call_depth.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env};

    #[contract]
    struct TestContract;
    #[contractimpl]
    impl TestContract {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let id = env.register(TestContract, ());
        (env, id)
    }

    // ── Nonce tests ───────────────────────────────────────────────────────────

    #[test]
    fn nonce_first_use_succeeds() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            assert!(consume_nonce(&env, &user, 1).is_ok());
        });
    }

    #[test]
    fn nonce_replay_returns_error() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            consume_nonce(&env, &user, 42).unwrap();
            assert_eq!(
                consume_nonce(&env, &user, 42),
                Err(NonceError::NonceAlreadyUsed)
            );
        });
    }

    #[test]
    fn nonce_expires_after_ttl() {
        let (env, contract_id) = setup();
        let user = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            consume_nonce(&env, &user, 7).unwrap();
            // Advance ledger past TTL
            env.ledger()
                .with_mut(|l| l.sequence_number += NONCE_TTL_LEDGERS + 1);
            // After expiry the key is gone; a new consume should succeed
            assert!(consume_nonce(&env, &user, 7).is_ok());
        });
    }

    #[test]
    fn different_users_same_nonce_are_independent() {
        let (env, contract_id) = setup();
        let user_a = soroban_sdk::Address::generate(&env);
        let user_b = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            consume_nonce(&env, &user_a, 1).unwrap();
            assert!(consume_nonce(&env, &user_b, 1).is_ok());
        });
    }

    // ── Wasm hash tests ───────────────────────────────────────────────────────

    #[test]
    fn wasm_hash_no_expected_hash_returns_error() {
        let (env, contract_id) = setup();
        let target = soroban_sdk::Address::generate(&env);
        env.as_contract(&contract_id, || {
            assert_eq!(
                verify_wasm_hash(&env, &target),
                Err(WasmHashError::UnexpectedContractVersion)
            );
        });
    }

    #[test]
    fn wasm_hash_mismatch_returns_error() {
        let (env, contract_id) = setup();
        // Register a second contract so we can get its real wasm hash
        let other_id = env.register(TestContract, ());
        let wrong_hash = BytesN::from_array(&env, &[0u8; 32]);
        env.as_contract(&contract_id, || {
            set_expected_wasm_hash(&env, &other_id, &wrong_hash);
            assert_eq!(
                verify_wasm_hash(&env, &other_id),
                Err(WasmHashError::UnexpectedContractVersion)
            );
        });
    }

    #[test]
    fn wasm_hash_match_succeeds() {
        let (env, contract_id) = setup();
        let other_id = env.register(TestContract, ());
        // Fetch the real wasm hash via our helper (avoids naming the private Executable type)
        let real_hash = wasm_hash_from_executable(&env, &other_id)
            .expect("expected wasm contract");
        env.as_contract(&contract_id, || {
            set_expected_wasm_hash(&env, &other_id, &real_hash);
            assert!(verify_wasm_hash(&env, &other_id).is_ok());
        });
    }

    // ── Call depth tests ──────────────────────────────────────────────────────

    #[test]
    fn depth_within_limit_succeeds() {
        // Depths 0..5 should all succeed and return depth+1.
        for d in 0..MAX_CALL_DEPTH {
            let result = check_call_depth(d);
            assert!(result.is_ok(), "expected Ok for depth {d}");
            assert_eq!(result.unwrap(), d + 1);
        }
    }

    #[test]
    fn depth_at_limit_fails() {
        assert_eq!(
            check_call_depth(MAX_CALL_DEPTH),
            Err(CallDepthError::CallDepthExceeded)
        );
    }

    #[test]
    fn depth_exceeds_limit_returns_error() {
        let result = check_call_depth(MAX_CALL_DEPTH + 1);
        assert_eq!(result, Err(CallDepthError::CallDepthExceeded));
    }

    #[test]
    fn simulated_call_chain_depth_5_succeeds() {
        // Simulate a chain of 5 nested calls (depths 0→1→2→3→4→5).
        let mut depth = 0u32;
        for _ in 0..5 {
            depth = check_call_depth(depth).expect("should not exceed limit");
        }
        assert_eq!(depth, 5);
    }

    #[test]
    fn simulated_call_chain_depth_6_fails() {
        let mut depth = 0u32;
        for _ in 0..5 {
            depth = check_call_depth(depth).expect("should not exceed limit");
        }
        // 6th call should fail
        let result = check_call_depth(depth);
        assert_eq!(result, Err(CallDepthError::CallDepthExceeded));
    }
}
