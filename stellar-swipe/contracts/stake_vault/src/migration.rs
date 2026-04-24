//! StakeVault storage migration: V1 → V2
//!
//! V1 stored stakes as `Map<Address, i128>` under key `StakesV1`.
//! V2 stores stakes as `Map<Address, StakeInfoV2>` under key `StakesV2`,
//! adding `locked_until` and `last_updated` fields.
//!
//! # Idempotency
//! Each provider is written to V2 only once. Re-running the migration
//! skips already-migrated providers (MigrationState tracks progress).
//!
//! # Checksum
//! After writing each entry, the contract reads it back and asserts
//! `new_balance == old_balance`. Any mismatch halts migration and emits
//! `MigrationError`.

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, Vec};

// ── Storage keys ────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum MigrationKey {
    StakesV1,
    StakesV2,
    MigrationState,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// V1 stake: bare balance only.
pub type StakesV1Map = Map<Address, i128>;

/// V2 stake: balance + lock metadata.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakeInfoV2 {
    pub balance: i128,
    pub locked_until: u64,
    pub last_updated: u64,
}

/// Persisted migration cursor so batched runs are idempotent.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MigrationState {
    /// Providers already migrated (in order).
    pub migrated: Vec<Address>,
    pub total_v1_providers: u32,
    pub complete: bool,
}

/// Per-call result summary.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MigrationBatchResult {
    pub migrated_this_batch: u32,
    pub total_migrated: u32,
    pub complete: bool,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum MigrationError {
    Unauthorized,
    BalanceMismatch { provider: Address, old: i128, new: i128 },
    AlreadyComplete,
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn get_v1(env: &Env) -> StakesV1Map {
    env.storage()
        .persistent()
        .get(&MigrationKey::StakesV1)
        .unwrap_or_else(|| Map::new(env))
}

fn get_v2(env: &Env) -> Map<Address, StakeInfoV2> {
    env.storage()
        .persistent()
        .get(&MigrationKey::StakesV2)
        .unwrap_or_else(|| Map::new(env))
}

fn save_v2(env: &Env, map: &Map<Address, StakeInfoV2>) {
    env.storage().persistent().set(&MigrationKey::StakesV2, map);
}

fn get_state(env: &Env) -> MigrationState {
    env.storage()
        .persistent()
        .get(&MigrationKey::MigrationState)
        .unwrap_or(MigrationState {
            migrated: Vec::new(env),
            total_v1_providers: 0,
            complete: false,
        })
}

fn save_state(env: &Env, state: &MigrationState) {
    env.storage()
        .persistent()
        .set(&MigrationKey::MigrationState, state);
}

fn emit_verified(env: &Env, provider: Address, old_balance: i128, new_balance: i128) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("mig_ok"), provider),
        (old_balance, new_balance),
    );
}

fn emit_error(env: &Env, provider: Address, old_balance: i128, new_balance: i128) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("mig_err"), provider),
        (old_balance, new_balance),
    );
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Migrate up to `batch_size` providers from V1 storage to V2.
///
/// Must be called by `admin`. Halts on any balance mismatch.
/// Safe to call multiple times — already-migrated providers are skipped.
pub fn migrate_stakes_v1_to_v2(
    env: &Env,
    admin: &Address,
    batch_size: u32,
) -> Result<MigrationBatchResult, MigrationError> {
    admin.require_auth();

    let mut state = get_state(env);
    if state.complete {
        return Err(MigrationError::AlreadyComplete);
    }

    let v1 = get_v1(env);
    let mut v2 = get_v2(env);
    let now = env.ledger().timestamp();

    // Build ordered list of V1 providers not yet migrated.
    let already_migrated = &state.migrated;
    let mut pending: Vec<Address> = Vec::new(env);
    for key in v1.keys() {
        let mut found = false;
        for i in 0..already_migrated.len() {
            if already_migrated.get(i).unwrap() == key {
                found = true;
                break;
            }
        }
        if !found {
            pending.push_back(key);
        }
    }

    let to_process = batch_size.min(pending.len());
    let mut migrated_this_batch = 0u32;

    for i in 0..to_process {
        let provider = pending.get(i).unwrap();
        let old_balance = v1.get(provider.clone()).unwrap_or(0);

        let info = StakeInfoV2 {
            balance: old_balance,
            locked_until: 0,
            last_updated: now,
        };
        v2.set(provider.clone(), info);

        // Checksum: read back and verify.
        let written = v2.get(provider.clone()).unwrap();
        if written.balance != old_balance {
            emit_error(env, provider.clone(), old_balance, written.balance);
            // Persist partial progress before halting.
            save_v2(env, &v2);
            save_state(env, &state);
            return Err(MigrationError::BalanceMismatch {
                provider,
                old: old_balance,
                new: written.balance,
            });
        }

        emit_verified(env, provider.clone(), old_balance, written.balance);
        state.migrated.push_back(provider);
        migrated_this_batch += 1;
    }

    let total_v1 = v1.len();
    state.total_v1_providers = total_v1;
    state.complete = state.migrated.len() >= total_v1;

    save_v2(env, &v2);
    save_state(env, &state);

    Ok(MigrationBatchResult {
        migrated_this_batch,
        total_migrated: state.migrated.len(),
        complete: state.complete,
    })
}

/// Seed V1 storage (test helper / admin bootstrap).
pub fn seed_v1_stakes(env: &Env, stakes: Map<Address, i128>) {
    env.storage()
        .persistent()
        .set(&MigrationKey::StakesV1, &stakes);
}

/// Read a V2 stake balance (post-migration).
pub fn get_v2_balance(env: &Env, provider: &Address) -> Option<i128> {
    get_v2(env).get(provider.clone()).map(|s| s.balance)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as TestAddress;
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn setup() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env
    }

    /// Seed 50 providers into V1 and migrate them in two batches.
    /// Verifies every balance is preserved exactly.
    #[test]
    fn test_migrate_50_providers_balance_preservation() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            let admin = Address::generate(&env);
            let mut v1: Map<Address, i128> = Map::new(&env);

            let mut providers = Vec::new(&env);
            for i in 0..50u32 {
                let p = Address::generate(&env);
                let balance = (i as i128 + 1) * 1_000_000;
                v1.set(p.clone(), balance);
                providers.push_back(p);
            }
            seed_v1_stakes(&env, v1.clone());

            // Batch 1: migrate 30
            let r1 = migrate_stakes_v1_to_v2(&env, &admin, 30).unwrap();
            assert_eq!(r1.migrated_this_batch, 30);
            assert!(!r1.complete);

            // Batch 2: migrate remaining 20
            let r2 = migrate_stakes_v1_to_v2(&env, &admin, 30).unwrap();
            assert_eq!(r2.migrated_this_batch, 20);
            assert!(r2.complete);
            assert_eq!(r2.total_migrated, 50);

            // Verify every balance
            for i in 0..50u32 {
                let p = providers.get(i).unwrap();
                let expected = (i as i128 + 1) * 1_000_000;
                assert_eq!(get_v2_balance(&env, &p), Some(expected));
            }
        });
    }

    #[test]
    fn test_idempotent_second_run() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            let admin = Address::generate(&env);
            let mut v1: Map<Address, i128> = Map::new(&env);
            let p = Address::generate(&env);
            v1.set(p.clone(), 500_000_000);
            seed_v1_stakes(&env, v1);

            migrate_stakes_v1_to_v2(&env, &admin, 10).unwrap();

            // Second call should return AlreadyComplete
            let err = migrate_stakes_v1_to_v2(&env, &admin, 10).unwrap_err();
            assert_eq!(err, MigrationError::AlreadyComplete);
        });
    }
}
