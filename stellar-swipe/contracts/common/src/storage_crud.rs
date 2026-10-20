//! Generic CRUD helpers that eliminate per-contract storage boilerplate.
//!
//! Each contract crate previously duplicated the same `get / set / has /
//! remove` pattern for every stored type, choosing between instance,
//! persistent, and temporary storage inline. This module centralises those
//! four primitives so that new contracts can adopt them by default and
//! existing contracts can migrate incrementally (Issue #579).
//!
//! # Usage pattern
//!
//! ```ignore
//! use stellar_swipe_common::storage_crud::{crud_get_or, crud_set, StorageTier};
//!
//! pub fn get_fee_rate(env: &Env) -> u32 {
//!     crud_get_or(env, StorageTier::Instance, &StorageKey::FeeRate, DEFAULT_FEE_RATE_BPS)
//! }
//!
//! pub fn set_fee_rate(env: &Env, rate: u32) {
//!     crud_set(env, StorageTier::Instance, &StorageKey::FeeRate, &rate);
//! }
//! ```
//!
//! # Storage-tier semantics (unchanged from direct SDK calls)
//!
//! | Tier       | Lifespan                             | Typical use                     |
//! |------------|--------------------------------------|---------------------------------|
//! | Instance   | Tied to contract instance TTL        | Config, counters, admin address |
//! | Persistent | Survives instance expiry; pays rent  | User balances, per-key records  |
//! | Temporary  | Cleared at end of transaction        | Reentrancy guards, nonces       |

use soroban_sdk::{Env, IntoVal, TryFromVal, Val};

/// Selects which Soroban storage tier an operation targets.
#[derive(Clone, Copy)]
pub enum StorageTier {
    /// Contract-instance storage — lifespan tied to instance TTL.
    Instance,
    /// Persistent storage — survives instance expiry; accrues rent.
    Persistent,
    /// Temporary storage — cleared at transaction end; cheapest tier.
    Temporary,
}

/// Read a value from storage, returning `None` if the key is absent.
///
/// Equivalent to `env.storage().<tier>().get(&key)`.
pub fn crud_get<K, V>(env: &Env, tier: StorageTier, key: &K) -> Option<V>
where
    K: IntoVal<Env, Val>,
    V: TryFromVal<Env, Val>,
{
    match tier {
        StorageTier::Instance => env.storage().instance().get(key),
        StorageTier::Persistent => env.storage().persistent().get(key),
        StorageTier::Temporary => env.storage().temporary().get(key),
    }
}

/// Read a value from storage, returning `default` if the key is absent.
///
/// Equivalent to `env.storage().<tier>().get(&key).unwrap_or(default)`.
pub fn crud_get_or<K, V>(env: &Env, tier: StorageTier, key: &K, default: V) -> V
where
    K: IntoVal<Env, Val>,
    V: TryFromVal<Env, Val>,
{
    crud_get(env, tier, key).unwrap_or(default)
}

/// Read a value from storage, returning `V::default()` if the key is absent.
///
/// Equivalent to `env.storage().<tier>().get(&key).unwrap_or_default()`.
pub fn crud_get_or_default<K, V>(env: &Env, tier: StorageTier, key: &K) -> V
where
    K: IntoVal<Env, Val>,
    V: TryFromVal<Env, Val> + Default,
{
    crud_get(env, tier, key).unwrap_or_default()
}

/// Write `value` to storage under `key`.
///
/// Equivalent to `env.storage().<tier>().set(&key, &value)`.
pub fn crud_set<K, V>(env: &Env, tier: StorageTier, key: &K, value: &V)
where
    K: IntoVal<Env, Val>,
    V: IntoVal<Env, Val>,
{
    match tier {
        StorageTier::Instance => env.storage().instance().set(key, value),
        StorageTier::Persistent => env.storage().persistent().set(key, value),
        StorageTier::Temporary => env.storage().temporary().set(key, value),
    }
}

/// Return `true` if `key` exists in storage.
///
/// Equivalent to `env.storage().<tier>().has(&key)`.
pub fn crud_has<K>(env: &Env, tier: StorageTier, key: &K) -> bool
where
    K: IntoVal<Env, Val>,
{
    match tier {
        StorageTier::Instance => env.storage().instance().has(key),
        StorageTier::Persistent => env.storage().persistent().has(key),
        StorageTier::Temporary => env.storage().temporary().has(key),
    }
}

/// Delete `key` from storage. A no-op if the key does not exist.
///
/// Equivalent to `env.storage().<tier>().remove(&key)`.
pub fn crud_remove<K>(env: &Env, tier: StorageTier, key: &K)
where
    K: IntoVal<Env, Val>,
{
    match tier {
        StorageTier::Instance => env.storage().instance().remove(key),
        StorageTier::Persistent => env.storage().persistent().remove(key),
        StorageTier::Temporary => env.storage().temporary().remove(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, contracttype, Env};

    #[contracttype]
    #[derive(Clone, PartialEq, Debug)]
    enum TestKey {
        A,
        B(u32),
    }

    #[contract]
    struct CrudHarness;

    #[contractimpl]
    impl CrudHarness {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let id = env.register(CrudHarness, ());
        (env, id)
    }

    #[test]
    fn instance_get_returns_none_when_missing() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let val: Option<u32> = crud_get(&env, StorageTier::Instance, &TestKey::A);
            assert_eq!(val, None);
        });
    }

    #[test]
    fn instance_set_and_get_roundtrip() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            crud_set(&env, StorageTier::Instance, &TestKey::A, &42u32);
            let val: Option<u32> = crud_get(&env, StorageTier::Instance, &TestKey::A);
            assert_eq!(val, Some(42u32));
        });
    }

    #[test]
    fn persistent_set_and_get_roundtrip() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            crud_set(&env, StorageTier::Persistent, &TestKey::B(7), &100i128);
            let val: Option<i128> = crud_get(&env, StorageTier::Persistent, &TestKey::B(7));
            assert_eq!(val, Some(100i128));
        });
    }

    #[test]
    fn crud_get_or_returns_default_when_missing() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let val = crud_get_or(&env, StorageTier::Instance, &TestKey::A, 99u32);
            assert_eq!(val, 99u32);
        });
    }

    #[test]
    fn crud_has_reflects_presence() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            assert!(!crud_has(&env, StorageTier::Instance, &TestKey::A));
            crud_set(&env, StorageTier::Instance, &TestKey::A, &1u32);
            assert!(crud_has(&env, StorageTier::Instance, &TestKey::A));
        });
    }

    #[test]
    fn crud_remove_deletes_key() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            crud_set(&env, StorageTier::Persistent, &TestKey::B(3), &55u32);
            assert!(crud_has(&env, StorageTier::Persistent, &TestKey::B(3)));
            crud_remove(&env, StorageTier::Persistent, &TestKey::B(3));
            assert!(!crud_has(&env, StorageTier::Persistent, &TestKey::B(3)));
        });
    }

    #[test]
    fn temporary_tier_roundtrip() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            crud_set(&env, StorageTier::Temporary, &TestKey::A, &true);
            let val: Option<bool> = crud_get(&env, StorageTier::Temporary, &TestKey::A);
            assert_eq!(val, Some(true));
            crud_remove(&env, StorageTier::Temporary, &TestKey::A);
            assert!(!crud_has(&env, StorageTier::Temporary, &TestKey::A));
        });
    }
}
