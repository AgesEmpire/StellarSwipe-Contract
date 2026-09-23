//! Storage layout migration guard (#1023).
//!
//! Upgrades can silently break state assumptions when storage layouts change.
//! This module enforces:
//!
//! 1. A `StorageLayoutVersion` key is written during `initialize` (version 1).
//! 2. Before accepting an upgrade, `guard_storage_upgrade` checks that the
//!    on-chain layout version is compatible with the new WASM's expected version.
//! 3. Required storage keys are validated to be present before the upgrade
//!    proceeds, preventing upgrades against partially-initialised state.
//! 4. On success the layout version is bumped and a `stgver` event is emitted.
//!
//! # Compatibility rule
//! An upgrade from layout version `current` to `next` is accepted when
//! `next == current + 1` (sequential migration only).  Skipping versions or
//! downgrading is rejected.

use soroban_sdk::{contracttype, symbol_short, Env, Symbol, Vec};

use shared::event_topics as topics;

// ── Storage key ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum StorageVersionKey {
    /// Current on-chain storage layout version (u32).
    LayoutVersion,
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// Initial storage layout version written at contract initialization.
pub const INITIAL_LAYOUT_VERSION: u32 = 1;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum StorageVersionError {
    /// The proposed layout version is not exactly `current + 1`.
    IncompatibleLayoutVersion,
    /// A required storage key is absent; upgrade cannot proceed safely.
    MissingRequiredKey,
    /// Contract has not been initialized (no layout version stored).
    NotInitialized,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Write the initial layout version during contract initialization.
pub fn init_storage_version(env: &Env) {
    env.storage()
        .instance()
        .set(&StorageVersionKey::LayoutVersion, &INITIAL_LAYOUT_VERSION);

    env.events().publish(
        (
            Symbol::new(env, "stake_vault"),
            topics::TOPIC_STORAGE_VERSION_SET(),
        ),
        INITIAL_LAYOUT_VERSION,
    );
}

/// Returns the current on-chain storage layout version, or `None` if unset.
pub fn get_layout_version(env: &Env) -> Option<u32> {
    env.storage()
        .instance()
        .get(&StorageVersionKey::LayoutVersion)
}

/// Guard an upgrade: validate that `next_layout_version == current + 1` and
/// that all `required_keys` are present in persistent storage.
///
/// On success, bumps the stored layout version and emits `stgver`.
/// On failure, emits `upgblk` and returns the appropriate error.
pub fn guard_storage_upgrade(
    env: &Env,
    next_layout_version: u32,
    required_keys: &Vec<Symbol>,
) -> Result<(), StorageVersionError> {
    let current: u32 = env
        .storage()
        .instance()
        .get(&StorageVersionKey::LayoutVersion)
        .ok_or_else(|| {
            emit_upgrade_blocked(env, 0, next_layout_version);
            StorageVersionError::NotInitialized
        })?;

    // Sequential-only migration: next must be exactly current + 1.
    if next_layout_version != current.saturating_add(1) {
        emit_upgrade_blocked(env, current, next_layout_version);
        return Err(StorageVersionError::IncompatibleLayoutVersion);
    }

    // Validate required keys are present.
    for i in 0..required_keys.len() {
        let key = required_keys.get(i).unwrap();
        if !env.storage().persistent().has(&key) {
            emit_upgrade_blocked(env, current, next_layout_version);
            return Err(StorageVersionError::MissingRequiredKey);
        }
    }

    // Commit new layout version.
    env.storage()
        .instance()
        .set(&StorageVersionKey::LayoutVersion, &next_layout_version);

    env.events().publish(
        (
            Symbol::new(env, "stake_vault"),
            topics::TOPIC_STORAGE_VERSION_SET(),
        ),
        (current, next_layout_version),
    );

    Ok(())
}

fn emit_upgrade_blocked(env: &Env, current: u32, proposed: u32) {
    env.events().publish(
        (
            Symbol::new(env, "stake_vault"),
            topics::TOPIC_UPGRADE_BLOCKED(),
        ),
        (current, proposed),
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, symbol_short, Env, Vec};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        env.mock_all_auths();
        let addr = env.register(TestContract, ());
        (env, addr)
    }

    #[test]
    fn init_sets_version_1() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            init_storage_version(&env);
            assert_eq!(get_layout_version(&env), Some(1));
        });
    }

    #[test]
    fn sequential_upgrade_succeeds() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            init_storage_version(&env);
            let empty: Vec<Symbol> = Vec::new(&env);
            guard_storage_upgrade(&env, 2, &empty).unwrap();
            assert_eq!(get_layout_version(&env), Some(2));
        });
    }

    #[test]
    fn skip_version_rejected() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            init_storage_version(&env);
            let empty: Vec<Symbol> = Vec::new(&env);
            assert_eq!(
                guard_storage_upgrade(&env, 3, &empty),
                Err(StorageVersionError::IncompatibleLayoutVersion)
            );
        });
    }

    #[test]
    fn downgrade_rejected() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            init_storage_version(&env);
            let empty: Vec<Symbol> = Vec::new(&env);
            assert_eq!(
                guard_storage_upgrade(&env, 1, &empty),
                Err(StorageVersionError::IncompatibleLayoutVersion)
            );
        });
    }

    #[test]
    fn missing_required_key_blocks_upgrade() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            init_storage_version(&env);
            let mut keys: Vec<Symbol> = Vec::new(&env);
            keys.push_back(symbol_short!("somekey"));
            assert_eq!(
                guard_storage_upgrade(&env, 2, &keys),
                Err(StorageVersionError::MissingRequiredKey)
            );
        });
    }

    #[test]
    fn present_required_key_allows_upgrade() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            init_storage_version(&env);
            // Write the required key into persistent storage.
            let key = symbol_short!("somekey");
            env.storage().persistent().set(&key, &true);

            let mut keys: Vec<Symbol> = Vec::new(&env);
            keys.push_back(key);
            guard_storage_upgrade(&env, 2, &keys).unwrap();
            assert_eq!(get_layout_version(&env), Some(2));
        });
    }

    #[test]
    fn not_initialized_returns_error() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let empty: Vec<Symbol> = Vec::new(&env);
            assert_eq!(
                guard_storage_upgrade(&env, 1, &empty),
                Err(StorageVersionError::NotInitialized)
            );
        });
    }
}
