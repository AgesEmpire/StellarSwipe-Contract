//! StakeVault storage migration: V1 → V2
//!
//! V1 stored stakes as `Map<Address, i128>` under key `StakesV1`.
//! V2 stores stakes as `Map<Address, StakeInfoV2>` under key `StakesV2`,
//! adding `locked_until` and `last_updated` fields.
//!
//! # Idempotency
//! Each provider is written to V2 only once. Re-running the migration
//! skips already-migrated providers and providers in `pending_recovery`.
//! `MigrationState.batch_number` increments on every call for correlation.
//!
//! # Checksum
//! After writing each entry, the contract reads it back and asserts
//! `new_balance == old_balance`. A mismatch halts the current batch,
//! records the provider in `pending_recovery`, and emits `MigrationError`.
//!
//! # Recovery
//! Admin calls `recover_migration_entry` to set the verified V2 balance for a
//! provider in `pending_recovery`, removing it from recovery and adding it to
//! `migrated`. Migration is `complete` only when all V1 providers are in
//! `migrated` (none remain in `pending_recovery`).

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
    /// Providers successfully migrated to V2.
    pub migrated: Vec<Address>,
    pub total_v1_providers: u32,
    /// True only when all V1 providers are in `migrated` (none in `pending_recovery`).
    pub complete: bool,
    /// Monotonically increasing per-call counter for event correlation.
    pub batch_number: u32,
    /// Providers that failed checksum verification; require `recover_migration_entry`.
    pub pending_recovery: Vec<Address>,
}

/// Per-call result summary.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MigrationBatchResult {
    pub migrated_this_batch: u32,
    pub total_migrated: u32,
    pub complete: bool,
    pub batch_number: u32,
    pub pending_recovery_count: u32,
}

/// Result of a successful recovery operation.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MigrationRecoveryResult {
    pub provider: Address,
    pub corrected_balance: i128,
    pub remaining_recovery: u32,
    pub migration_complete: bool,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum MigrationError {
    /// Caller is not the admin authorized to run or recover the migration.
    Unauthorized,
    /// Checksum readback after writing a V2 entry did not match the V1 balance.
    BalanceMismatch {
        provider: Address,
        old: i128,
        new: i128,
    },
    /// Migration is already complete; no further batches can be run.
    AlreadyComplete,
    /// Provider is not in `pending_recovery`; nothing to recover.
    NotInRecovery,
}

impl MigrationError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            MigrationError::Unauthorized => {
                "caller is not the admin authorized to run or recover the migration"
            }
            MigrationError::BalanceMismatch { .. } => {
                "checksum readback after writing a V2 entry did not match the V1 balance"
            }
            MigrationError::AlreadyComplete => {
                "migration is already complete; no further batches can be run"
            }
            MigrationError::NotInRecovery => {
                "provider is not in pending_recovery; nothing to recover"
            }
        }
    }
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
            batch_number: 0,
            pending_recovery: Vec::new(env),
        })
}

fn is_in_vec(vec: &Vec<Address>, target: &Address) -> bool {
    for i in 0..vec.len() {
        if vec.get(i).unwrap() == *target {
            return true;
        }
    }
    false
}

fn remove_from_vec(env: &Env, vec: &Vec<Address>, target: &Address) -> Vec<Address> {
    let mut result = Vec::new(env);
    for i in 0..vec.len() {
        let addr = vec.get(i).unwrap();
        if addr != *target {
            result.push_back(addr);
        }
    }
    result
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

fn emit_batch_start(env: &Env, batch_number: u32, pending_count: u32, recovery_count: u32) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("mig_start"),),
        (batch_number, pending_count, recovery_count),
    );
}

fn emit_batch_progress(
    env: &Env,
    batch_number: u32,
    migrated_this_batch: u32,
    total_migrated: u32,
    total_v1: u32,
    pending_recovery_count: u32,
) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("mig_prog"),),
        (
            batch_number,
            migrated_this_batch,
            total_migrated,
            total_v1,
            pending_recovery_count,
        ),
    );
}

fn emit_migration_complete(env: &Env, total_migrated: u32) {
    #[allow(deprecated)]
    env.events()
        .publish((symbol_short!("mig_done"),), (total_migrated,));
}

fn emit_recovery(env: &Env, provider: Address, corrected_balance: i128, remaining_recovery: u32) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("mig_rec"), provider),
        (corrected_balance, remaining_recovery),
    );
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Migrate up to `batch_size` providers from V1 storage to V2.
///
/// Must be called by `admin`. Halts on any balance mismatch — the failing
/// provider is added to `pending_recovery` so subsequent batches skip it
/// and migration can continue past the bad entry. Call
/// `recover_migration_entry` to resolve stuck providers.
///
/// Safe to call multiple times — already-migrated and pending-recovery
/// providers are both skipped, so partial batches resume cleanly.
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

    // Snapshot total early so it is persisted even on early-exit paths.
    let total_v1 = v1.len();
    state.total_v1_providers = total_v1;

    // Build pending list: V1 providers not yet migrated and not in pending_recovery.
    let mut pending: Vec<Address> = Vec::new(env);
    for key in v1.keys() {
        if !is_in_vec(&state.migrated, &key) && !is_in_vec(&state.pending_recovery, &key) {
            pending.push_back(key);
        }
    }

    state.batch_number += 1;
    emit_batch_start(
        env,
        state.batch_number,
        pending.len(),
        state.pending_recovery.len(),
    );

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

        // Checksum: read back and verify balance was written correctly.
        let written = v2.get(provider.clone()).unwrap();
        if written.balance != old_balance {
            emit_error(env, provider.clone(), old_balance, written.balance);
            // Park the failing provider so future batches skip it.
            state.pending_recovery.push_back(provider.clone());
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

    // Complete only when every V1 provider is migrated and recovery queue is clear.
    let all_accounted = state.migrated.len() + state.pending_recovery.len() >= total_v1;
    state.complete = all_accounted && state.pending_recovery.is_empty();

    save_v2(env, &v2);
    save_state(env, &state);

    let batch_number = state.batch_number;
    let total_migrated = state.migrated.len();
    let pending_recovery_count = state.pending_recovery.len();
    let complete = state.complete;

    emit_batch_progress(
        env,
        batch_number,
        migrated_this_batch,
        total_migrated,
        total_v1,
        pending_recovery_count,
    );
    if complete {
        emit_migration_complete(env, total_migrated);
    }

    Ok(MigrationBatchResult {
        migrated_this_batch,
        total_migrated,
        complete,
        batch_number,
        pending_recovery_count,
    })
}

/// Resolve a provider stuck in `pending_recovery` after a checksum mismatch.
///
/// Admin supplies `verified_balance` after independently auditing V1 data.
/// The provider is removed from `pending_recovery`, written to V2 with the
/// given balance, and added to `migrated`. If this was the last pending
/// recovery and all V1 providers are accounted for, migration is marked
/// complete and `mig_done` is emitted.
pub fn recover_migration_entry(
    env: &Env,
    admin: &Address,
    provider: Address,
    verified_balance: i128,
) -> Result<MigrationRecoveryResult, MigrationError> {
    admin.require_auth();

    let mut state = get_state(env);

    if !is_in_vec(&state.pending_recovery, &provider) {
        return Err(MigrationError::NotInRecovery);
    }

    let mut v2 = get_v2(env);
    let now = env.ledger().timestamp();

    v2.set(
        provider.clone(),
        StakeInfoV2 {
            balance: verified_balance,
            locked_until: 0,
            last_updated: now,
        },
    );

    state.pending_recovery = remove_from_vec(env, &state.pending_recovery, &provider);
    state.migrated.push_back(provider.clone());

    let total_v1 = state.total_v1_providers;
    let all_accounted = state.migrated.len() + state.pending_recovery.len() >= total_v1;
    state.complete = all_accounted && state.pending_recovery.is_empty();

    let remaining_recovery = state.pending_recovery.len();
    let migration_complete = state.complete;
    let total_migrated = state.migrated.len();

    save_v2(env, &v2);
    save_state(env, &state);

    emit_recovery(env, provider.clone(), verified_balance, remaining_recovery);
    if migration_complete {
        emit_migration_complete(env, total_migrated);
    }

    Ok(MigrationRecoveryResult {
        provider,
        corrected_balance: verified_balance,
        remaining_recovery,
        migration_complete,
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

/// Inspect the current migration progress (batch_number, migrated count, pending_recovery).
pub fn get_migration_state(env: &Env) -> MigrationState {
    get_state(env)
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

    /// Each migration call needs its own contract frame so `require_auth()` is not
    /// invoked twice on the same authorized frame.
    fn run_migrate(
        env: &Env,
        contract_addr: &Address,
        admin: &Address,
        batch_size: u32,
    ) -> Result<MigrationBatchResult, MigrationError> {
        env.as_contract(contract_addr, || {
            migrate_stakes_v1_to_v2(env, admin, batch_size)
        })
    }

    /// Seed 50 providers into V1 and migrate them in two batches.
    /// Verifies every balance is preserved exactly and batch_number increments.
    #[test]
    fn test_migrate_50_providers_balance_preservation() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        let admin = Address::generate(&env);
        let mut v1: Map<Address, i128> = Map::new(&env);

        let mut providers = Vec::new(&env);
        for i in 0..50u32 {
            let p = Address::generate(&env);
            let balance = (i as i128 + 1) * 1_000_000;
            v1.set(p.clone(), balance);
            providers.push_back(p);
        }
        env.as_contract(&contract_addr, || seed_v1_stakes(&env, v1.clone()));

        // Batch 1: migrate 30
        let r1 = run_migrate(&env, &contract_addr, &admin, 30).unwrap();
        assert_eq!(r1.migrated_this_batch, 30);
        assert_eq!(r1.batch_number, 1);
        assert_eq!(r1.pending_recovery_count, 0);
        assert!(!r1.complete);

        // Batch 2: migrate remaining 20
        let r2 = run_migrate(&env, &contract_addr, &admin, 30).unwrap();
        assert_eq!(r2.migrated_this_batch, 20);
        assert_eq!(r2.batch_number, 2);
        assert!(r2.complete);
        assert_eq!(r2.total_migrated, 50);

        // Verify every balance
        for i in 0..50u32 {
            let p = providers.get(i).unwrap();
            let expected = (i as i128 + 1) * 1_000_000;
            let balance = env.as_contract(&contract_addr, || get_v2_balance(&env, &p));
            assert_eq!(balance, Some(expected));
        }
    }

    #[test]
    fn test_idempotent_second_run() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        let admin = Address::generate(&env);
        let mut v1: Map<Address, i128> = Map::new(&env);
        let p = Address::generate(&env);
        v1.set(p.clone(), 500_000_000);
        env.as_contract(&contract_addr, || seed_v1_stakes(&env, v1));

        run_migrate(&env, &contract_addr, &admin, 10).unwrap();

        // Second call should return AlreadyComplete
        let err = run_migrate(&env, &contract_addr, &admin, 10).unwrap_err();
        assert_eq!(err, MigrationError::AlreadyComplete);
    }

    #[test]
    fn test_recover_migration_entry_without_pending_recovery_is_not_in_recovery() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);

        let err = env
            .as_contract(&contract_addr, || {
                recover_migration_entry(&env, &admin, provider, 100)
            })
            .unwrap_err();
        assert_eq!(err, MigrationError::NotInRecovery);
    }

    #[test]
    fn error_messages_are_non_empty_and_distinct() {
        let env = setup();
        let provider = Address::generate(&env);
        let samples = [
            MigrationError::Unauthorized,
            MigrationError::BalanceMismatch {
                provider,
                old: 1,
                new: 2,
            },
            MigrationError::AlreadyComplete,
            MigrationError::NotInRecovery,
        ];
        for err in samples.iter() {
            assert!(!err.message().is_empty());
        }
        for i in 0..samples.len() {
            for j in (i + 1)..samples.len() {
                assert_ne!(
                    samples[i].message(),
                    samples[j].message(),
                    "expected distinct messages for {:?} and {:?}",
                    samples[i],
                    samples[j]
                );
            }
        }
    }
}
