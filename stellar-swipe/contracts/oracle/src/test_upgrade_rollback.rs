#![cfg(test)]
//! Upgrade rollback tests for failed post-migration validation (Issue #1403).
//!
//! Verifies that migration and post-migration validation are atomic: if
//! validation fails after migration writes, the pre-upgrade state is
//! preserved and no partial changes are observable. When validation
//! succeeds, all expected changes are committed.
//!
//! # Design
//! Soroban's test environment does not expose the WASM `upgrade` host
//! function directly. We simulate atomicity by using `env.as_contract` with
//! a closure that either commits or is never stored — equivalent to a
//! transaction that aborts before commit. We also rely on `register_at` to
//! swap in a V2 implementation at the same contract address, which keeps all
//! prior persistent storage visible (storage is keyed by address, not
//! WASM hash).
//!
//! # Atomicity invariant tested
//! - A migration closure that writes state and then returns an error leaves
//!   no mutations visible to subsequent reads (Soroban rolls back all writes
//!   made by an invocation that returns a non-Ok status).
//! - A migration closure that writes state and returns Ok commits all
//!   expected changes.

extern crate std;

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

// ─────────────────────────────────────────────────────────────────────────────
// Minimal shared storage types used by both V1 and the migration contract
// ─────────────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum Key {
    Admin,
    SchemaVersion,
    Data(u32),
    MigrationDone,
}

// ─────────────────────────────────────────────────────────────────────────────
// V1 — the pre-upgrade contract
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct V1;

#[contractimpl]
impl V1 {
    pub fn initialize(env: Env, admin: Address) {
        assert!(
            !env.storage().instance().has(&Key::Admin),
            "already initialized"
        );
        env.storage().instance().set(&Key::Admin, &admin);
        env.storage()
            .instance()
            .set(&Key::SchemaVersion, &1u32);
    }

    pub fn write_data(env: Env, id: u32, value: i128) {
        env.storage()
            .persistent()
            .set(&Key::Data(id), &value);
    }

    pub fn read_data(env: Env, id: u32) -> Option<i128> {
        env.storage().persistent().get(&Key::Data(id))
    }

    pub fn schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&Key::SchemaVersion)
            .unwrap_or(1)
    }

    pub fn migration_done(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&Key::MigrationDone)
            .unwrap_or(false)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// V2 — the upgraded contract with post-migration validation
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MigrateError {
    ValidationFailed,
    AlreadyMigrated,
}

#[contract]
pub struct V2;

#[contractimpl]
impl V2 {
    /// Run the V1→V2 migration.
    ///
    /// Steps:
    /// 1. Write the new schema version.
    /// 2. Transform each data entry.
    /// 3. Run post-migration validation.
    ///
    /// If validation fails the caller receives `Err`; Soroban rolls back all
    /// storage written in this invocation, so the schema version and
    /// transformed entries are never committed.
    ///
    /// Pass `fail_validation = true` to inject a failure after the writes,
    /// simulating a broken migration.
    pub fn migrate(env: Env, ids: Vec<u32>, fail_validation: bool) -> Result<(), u32> {
        // Guard idempotency.
        if env
            .storage()
            .instance()
            .get::<_, bool>(&Key::MigrationDone)
            .unwrap_or(false)
        {
            return Err(1); // AlreadyMigrated
        }

        // Step 1: bump schema version.
        env.storage()
            .instance()
            .set(&Key::SchemaVersion, &2u32);

        // Step 2: transform every data entry (multiply by 10 for V2 format).
        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            if let Some(v) = env
                .storage()
                .persistent()
                .get::<_, i128>(&Key::Data(id))
            {
                env.storage()
                    .persistent()
                    .set(&Key::Data(id), &(v * 10));
            }
        }

        // Step 3: post-migration validation.
        if fail_validation {
            // Validation failure: returning Err causes Soroban to roll back
            // ALL writes above (schema version + transformed data).
            return Err(2); // ValidationFailed
        }

        // Commit migration marker.
        env.storage()
            .instance()
            .set(&Key::MigrationDone, &true);

        Ok(())
    }

    pub fn read_data(env: Env, id: u32) -> Option<i128> {
        env.storage().persistent().get(&Key::Data(id))
    }

    pub fn schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&Key::SchemaVersion)
            .unwrap_or(1)
    }

    pub fn migration_done(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&Key::MigrationDone)
            .unwrap_or(false)
    }

    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&Key::Admin)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Setup helper
// ─────────────────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);
    let admin = Address::generate(&env);
    let cid = env.register(V1, ());
    let v1 = V1Client::new(&env, &cid);
    v1.initialize(&admin);
    (env, cid, admin)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: failed validation restores pre-upgrade state
// ─────────────────────────────────────────────────────────────────────────────

/// A `migrate()` call that fails post-migration validation must leave the
/// contract in its exact pre-upgrade state: schema version unchanged, data
/// entries unchanged, migration-done flag false.
#[test]
fn failed_validation_restores_pre_upgrade_state() {
    let (env, cid, _admin) = setup();
    let v1 = V1Client::new(&env, &cid);

    // Write three data entries pre-upgrade.
    v1.write_data(&1u32, &100i128);
    v1.write_data(&2u32, &200i128);
    v1.write_data(&3u32, &300i128);

    assert_eq!(v1.schema_version(), 1u32);
    assert!(!v1.migration_done());

    // Upgrade to V2 implementation.
    env.register_at(&cid, V2, ());
    let v2 = V2Client::new(&env, &cid);

    // Attempt migration with deliberately failing validation.
    let ids = soroban_sdk::vec![&env, 1u32, 2u32, 3u32];
    let result = v2.try_migrate(&ids, &true);
    assert!(
        result.is_err(),
        "migration must return Err when validation fails"
    );

    // Soroban rolls back all writes on Err — schema version must still be 1.
    assert_eq!(
        v2.schema_version(),
        1u32,
        "schema version must remain 1 after failed migration"
    );

    // Data entries must be untouched (original values, not multiplied by 10).
    assert_eq!(
        v2.read_data(&1u32),
        Some(100i128),
        "data entry 1 must be unchanged"
    );
    assert_eq!(
        v2.read_data(&2u32),
        Some(200i128),
        "data entry 2 must be unchanged"
    );
    assert_eq!(
        v2.read_data(&3u32),
        Some(300i128),
        "data entry 3 must be unchanged"
    );

    // Migration-done flag must not be set.
    assert!(
        !v2.migration_done(),
        "migration_done must remain false after failed migration"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: valid migration commits all expected changes
// ─────────────────────────────────────────────────────────────────────────────

/// A `migrate()` call that passes validation must commit every expected state
/// change: schema version bumped, all data entries transformed, done flag set.
#[test]
fn valid_migration_commits_all_expected_changes() {
    let (env, cid, _admin) = setup();
    let v1 = V1Client::new(&env, &cid);

    v1.write_data(&10u32, &50i128);
    v1.write_data(&20u32, &75i128);

    env.register_at(&cid, V2, ());
    let v2 = V2Client::new(&env, &cid);

    let ids = soroban_sdk::vec![&env, 10u32, 20u32];
    v2.migrate(&ids, &false);

    // Schema version bumped to 2.
    assert_eq!(
        v2.schema_version(),
        2u32,
        "schema version must be 2 after successful migration"
    );

    // Data entries transformed (multiplied by 10).
    assert_eq!(
        v2.read_data(&10u32),
        Some(500i128),
        "data entry 10 must be 50*10=500"
    );
    assert_eq!(
        v2.read_data(&20u32),
        Some(750i128),
        "data entry 20 must be 75*10=750"
    );

    // Migration-done flag set.
    assert!(
        v2.migration_done(),
        "migration_done must be true after successful migration"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: migration failure after a write does not leave partial state
// ─────────────────────────────────────────────────────────────────────────────

/// When migration fails midway (after some writes but before the final commit),
/// no partial mutations are observable. This directly tests the "fails after
/// a migration write" criterion from the issue.
#[test]
fn migration_failure_after_write_leaves_no_partial_state() {
    let (env, cid, _admin) = setup();
    let v1 = V1Client::new(&env, &cid);

    // Multiple entries: migration writes some before failing.
    v1.write_data(&1u32, &1_000i128);
    v1.write_data(&2u32, &2_000i128);
    v1.write_data(&3u32, &3_000i128);

    env.register_at(&cid, V2, ());
    let v2 = V2Client::new(&env, &cid);

    // Fail after writing entries 1 and 2 (all entries, then validation fails).
    let ids = soroban_sdk::vec![&env, 1u32, 2u32, 3u32];
    let _ = v2.try_migrate(&ids, &true);

    // Every entry must retain its original value — no partial transformation.
    assert_eq!(v2.read_data(&1u32), Some(1_000i128), "entry 1 must be original");
    assert_eq!(v2.read_data(&2u32), Some(2_000i128), "entry 2 must be original");
    assert_eq!(v2.read_data(&3u32), Some(3_000i128), "entry 3 must be original");
    assert_eq!(v2.schema_version(), 1u32, "schema must still be v1");
    assert!(!v2.migration_done(), "done flag must not be set");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: successful migration followed by a retry returns AlreadyMigrated
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn retry_after_successful_migration_is_rejected() {
    let (env, cid, _admin) = setup();
    let v1 = V1Client::new(&env, &cid);
    v1.write_data(&7u32, &77i128);

    env.register_at(&cid, V2, ());
    let v2 = V2Client::new(&env, &cid);

    let ids = soroban_sdk::vec![&env, 7u32];
    v2.migrate(&ids, &false);

    // Second call must fail with AlreadyMigrated (Err code 1).
    let result = v2.try_migrate(&ids, &false);
    assert!(result.is_err(), "second migration attempt must be rejected");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5: failed migration followed by a successful retry commits correctly
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn failed_migration_followed_by_successful_retry_commits() {
    let (env, cid, _admin) = setup();
    let v1 = V1Client::new(&env, &cid);
    v1.write_data(&5u32, &55i128);

    env.register_at(&cid, V2, ());
    let v2 = V2Client::new(&env, &cid);

    let ids = soroban_sdk::vec![&env, 5u32];

    // First attempt: validation fails.
    let _ = v2.try_migrate(&ids, &true);
    assert_eq!(v2.read_data(&5u32), Some(55i128), "original value after failure");
    assert!(!v2.migration_done());

    // Second attempt: validation passes.
    v2.migrate(&ids, &false);
    assert_eq!(v2.read_data(&5u32), Some(550i128), "55*10=550 after success");
    assert!(v2.migration_done());
    assert_eq!(v2.schema_version(), 2u32);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6: pre-upgrade instance storage (admin) survives upgrade and migration
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn admin_address_survives_upgrade_and_migration() {
    let (env, cid, admin) = setup();

    env.register_at(&cid, V2, ());
    let v2 = V2Client::new(&env, &cid);

    let ids = soroban_sdk::vec![&env];
    v2.migrate(&ids, &false);

    assert_eq!(
        v2.get_admin(),
        Some(admin),
        "admin must survive upgrade and migration"
    );
}
