//! v1 → v2 signal storage migration. Unmigrated records live in [`StorageKey::SignalsV1`];
//! canonical v2 data is in [`StorageKey::Signals`]. Re-running the migration is safe: only
//! ids with a v1 record are transformed; v1 is removed when written to v2.

use crate::categories;
use crate::categories::{RiskLevel, SignalCategory};
use crate::contests;
use crate::errors::AdminError;
use crate::events::{
    emit_migration_layout_rejected, emit_migration_progress, emit_migration_verification_failed,
    emit_migration_verified,
};
use crate::types::{MigrationProgress, Signal, SignalAction, SignalStatus, SignalV1};
use crate::StorageKey;
use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};

const MAX_MIGRATION_BATCH: u32 = 256;

// ═══════════════════════════════════════════════════════════════════
// Issue #812: storage layout migration guards
// ═══════════════════════════════════════════════════════════════════
//
// `migrate_signals_v1_to_v2` assumes a specific on-chain storage shape: a
// `SignalsV1` map of legacy `SignalV1` records being drained into a
// `Signals` map of current `Signal` (v2) records. That assumption is only
// safe to act on when the contract's recorded schema version actually says
// "pre-migration v1 layout". Running the migration logic against any other
// schema (e.g. a hypothetical future v3 layout that reuses these storage
// keys differently) could silently corrupt state. `verify_storage_layout`
// is called as the very first step of `migrate_signals_v1_to_v2`, before
// any storage is read for mutation, and produces a deterministic,
// auditable error (`AdminError::IncompatibleStorageLayout` + an emitted
// event) instead of proceeding when the layout doesn't match.

/// Schema version for the legacy `SignalV1` layout (pre-migration).
pub const SIGNAL_SCHEMA_V1: u32 = 1;
/// Schema version for the current `Signal` (v2) layout.
pub const SIGNAL_SCHEMA_V2: u32 = 2;

/// Read the on-chain storage schema version.
///
/// Defaults to [`SIGNAL_SCHEMA_V1`] when unset: contracts deployed before
/// this guard existed never wrote this key, and — being pre-migration —
/// that is in fact their real schema. Freshly-initialized contracts set
/// this explicitly to [`SIGNAL_SCHEMA_V2`] in `initialize()` since they
/// start with no legacy `SignalsV1` rows at all.
pub fn get_schema_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::SchemaVersion)
        .unwrap_or(SIGNAL_SCHEMA_V1)
}

pub fn set_schema_version(env: &Env, version: u32) {
    env.storage()
        .instance()
        .set(&StorageKey::SchemaVersion, &version);
}

/// Guard checked before any migration logic runs (Issue #812).
///
/// Returns `Ok(())` when the stored schema version is a layout this
/// migration code understands:
/// - [`SIGNAL_SCHEMA_V1`] — the expected pre-migration layout; the caller
///   proceeds with the actual v1→v2 migration.
/// - [`SIGNAL_SCHEMA_V2`] — already fully migrated; the caller's subsequent
///   scan finds no v1 rows and is a safe, idempotent no-op.
///
/// Any other value — storage corruption, or a schema this migration code
/// predates (e.g. a future v3) — is rejected deterministically with
/// [`AdminError::IncompatibleStorageLayout`] and an audit event, without
/// touching any migration state.
pub fn verify_storage_layout(env: &Env) -> Result<(), AdminError> {
    let version = get_schema_version(env);
    if version == SIGNAL_SCHEMA_V1 || version == SIGNAL_SCHEMA_V2 {
        Ok(())
    } else {
        emit_migration_layout_rejected(env, version);
        Err(AdminError::IncompatibleStorageLayout)
    }
}

fn v1_to_v2(_env: &Env, v1: &SignalV1) -> Signal {
    let rationale_hash = v1.rationale.clone();
    Signal {
        id: v1.id,
        provider: v1.provider.clone(),
        asset_pair: v1.asset_pair.clone(),
        action: v1.action.clone(),
        price: v1.price,
        rationale: v1.rationale.clone(),
        timestamp: v1.timestamp,
        expiry: v1.expiry,
        status: v1.status.clone(),
        executions: v1.executions,
        successful_executions: v1.successful_executions,
        total_volume: v1.total_volume,
        total_roi: v1.total_roi,
        category: v1.category.clone(),
        tags: v1.tags.clone(),
        risk_level: v1.risk_level.clone(),
        is_collaborative: v1.is_collaborative,
        submitted_at: v1.timestamp,
        rationale_hash,
        confidence: 50,
        adoption_count: 0,
        ai_validation_score: None,
        avg_copier_roi_bps: 0,
        copier_closed_count: 0,
        warning_emitted: false,
        benchmark_return_bps: None,
        alpha_bps: None,
    }
}

fn get_v1_map(env: &Env) -> Map<u64, SignalV1> {
    env.storage()
        .instance()
        .get(&StorageKey::SignalsV1)
        .unwrap_or(Map::new(env))
}

fn save_v1_map(env: &Env, m: &Map<u64, SignalV1>) {
    env.storage().instance().set(&StorageKey::SignalsV1, m);
}

fn get_v2_map(env: &Env) -> Map<u64, Signal> {
    env.storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env))
}

fn save_v2_map(env: &Env, m: &Map<u64, Signal>) {
    env.storage().instance().set(&StorageKey::Signals, m);
}

fn get_migration_cursor(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&StorageKey::MigrationCursor)
        .unwrap_or(1u64)
}

fn set_migration_cursor(env: &Env, c: u64) {
    env.storage()
        .instance()
        .set(&StorageKey::MigrationCursor, &c);
}

fn get_migration_v1_target_total(env: &Env) -> Option<u32> {
    env.storage()
        .instance()
        .get(&StorageKey::MigrationV1TargetTotal)
}

fn set_migration_v1_target_total(env: &Env, n: u32) {
    env.storage()
        .instance()
        .set(&StorageKey::MigrationV1TargetTotal, &n);
}

/// Counts legacy rows with id in 1..=max_id. Bounded by the instance signal counter.
fn count_v1_keys(v1: &Map<u64, SignalV1>, max_id: u64) -> u32 {
    if max_id == 0 {
        return 0;
    }
    let mut c: u32 = 0;
    let mut i: u64 = 1;
    while i <= max_id {
        if v1.get(i).is_some() {
            c = c.saturating_add(1);
        }
        i = i.saturating_add(1);
    }
    c
}

// ═══════════════════════════════════════════════════════════════════
// Issue #597: Post-migration invariant checksum verification
// ═══════════════════════════════════════════════════════════════════
//
// `migrate_signals_v1_to_v2` moves records in bounded batches; nothing
// previously confirmed that the migrated v2 data actually reconciles with
// the v1 data it replaced. This snapshots aggregate invariants (record
// count + sum of `total_volume`) over the v1 scope when migration starts,
// recomputes the same aggregates over the migrated v2 scope once v1 is
// fully drained, and records whether they reconcile — automatically, as
// part of this same migration helper, not a separate manual step.

/// Aggregate invariants captured over a signal map, scoped to ids `1..=max_id`.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationSnapshot {
    pub record_count: u32,
    pub total_volume_sum: i128,
}

/// Result of reconciling a pre-migration v1 snapshot against the
/// post-migration v2 snapshot over the same id scope.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationVerification {
    pub verified: bool,
    pub pre: MigrationSnapshot,
    pub post: MigrationSnapshot,
}

/// Sum of `total_volume` across legacy rows with id in 1..=max_id.
fn sum_v1_total_volume(v1: &Map<u64, SignalV1>, max_id: u64) -> i128 {
    let mut sum: i128 = 0;
    let mut i: u64 = 1;
    while i <= max_id {
        if let Some(v) = v1.get(i) {
            sum = sum.saturating_add(v.total_volume);
        }
        i = i.saturating_add(1);
    }
    sum
}

/// Snapshot the v1 scope (record count + total_volume sum) over ids `1..=max_id`.
fn snapshot_v1(v1: &Map<u64, SignalV1>, max_id: u64) -> MigrationSnapshot {
    MigrationSnapshot {
        record_count: count_v1_keys(v1, max_id),
        total_volume_sum: sum_v1_total_volume(v1, max_id),
    }
}

/// Snapshot the migrated v2 scope (record count + total_volume sum) over ids `1..=max_id`.
fn snapshot_v2(v2: &Map<u64, Signal>, max_id: u64) -> MigrationSnapshot {
    let mut record_count: u32 = 0;
    let mut total_volume_sum: i128 = 0;
    let mut i: u64 = 1;
    while i <= max_id {
        if let Some(s) = v2.get(i) {
            record_count = record_count.saturating_add(1);
            total_volume_sum = total_volume_sum.saturating_add(s.total_volume);
        }
        i = i.saturating_add(1);
    }
    MigrationSnapshot {
        record_count,
        total_volume_sum,
    }
}

/// Pure reconciliation: compares a pre-migration and post-migration snapshot
/// of the same scope and reports whether they reconcile.
fn reconcile(pre: MigrationSnapshot, post: MigrationSnapshot) -> MigrationVerification {
    let verified = pre == post;
    MigrationVerification {
        verified,
        pre,
        post,
    }
}

fn get_migration_pre_snapshot(env: &Env) -> Option<MigrationSnapshot> {
    env.storage()
        .instance()
        .get(&StorageKey::MigrationPreSnapshot)
}

fn set_migration_pre_snapshot(env: &Env, snap: &MigrationSnapshot) {
    env.storage()
        .instance()
        .set(&StorageKey::MigrationPreSnapshot, snap);
}

fn set_migration_verification(env: &Env, v: &MigrationVerification) {
    env.storage()
        .instance()
        .set(&StorageKey::MigrationVerification, v);
}

/// Last recorded post-migration verification result, if any migration has
/// completed since this contract version was deployed.
pub fn get_migration_verification(env: &Env) -> Option<MigrationVerification> {
    env.storage()
        .instance()
        .get(&StorageKey::MigrationVerification)
}

fn add_to_category_index(env: &Env, id: u64, category: SignalCategory) {
    let mut cat_map: Map<SignalCategory, Vec<u64>> = env
        .storage()
        .instance()
        .get(&StorageKey::ActiveSignalsByCategory)
        .unwrap_or(Map::new(env));
    let mut cat_list = cat_map.get(category.clone()).unwrap_or(Vec::new(env));
    let mut found = false;
    for j in 0..cat_list.len() {
        if cat_list.get(j).unwrap() == id {
            found = true;
            break;
        }
    }
    if !found {
        cat_list.push_back(id);
    }
    cat_map.set(category, cat_list);
    env.storage()
        .instance()
        .set(&StorageKey::ActiveSignalsByCategory, &cat_map);
}

/// Migrate at most `batch_size` v1 signal records into v2, scanning by signal id
/// from the saved cursor. Idempotent: re-running with no v1 rows is a no-op (aside from events).
pub fn migrate_signals_v1_to_v2(
    env: &Env,
    _admin: &Address,
    batch_size: u32,
) -> Result<(), AdminError> {
    // Issue #812: storage layout check runs before any migration logic —
    // including before the batch_size validation below — touches state.
    verify_storage_layout(env)?;

    if batch_size == 0 || batch_size > MAX_MIGRATION_BATCH {
        return Err(AdminError::InvalidParameter);
    }

    let counter: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::SignalCounter)
        .unwrap_or(0u64);
    if counter == 0 {
        emit_migration_progress(
            env,
            MigrationProgress {
                migrated_count: 0,
                total_count: 0,
            },
        );
        return Ok(());
    }

    let v1 = get_v1_map(env);
    if count_v1_keys(&v1, counter) == 0 {
        set_migration_cursor(env, counter.saturating_add(1));
        let tt = get_migration_v1_target_total(env).unwrap_or(0);
        emit_migration_progress(
            env,
            MigrationProgress {
                migrated_count: 0,
                total_count: tt,
            },
        );
        return Ok(());
    }

    if get_migration_v1_target_total(env).is_none() {
        set_migration_v1_target_total(env, count_v1_keys(&v1, counter));
        // Pre-migration invariant snapshot (issue #597), captured once at the
        // very start of this migration run, over the same id scope used for
        // the post-migration check below.
        set_migration_pre_snapshot(env, &snapshot_v1(&v1, counter));
    }
    let target_total = get_migration_v1_target_total(env).unwrap_or(0);

    let mut v1 = v1;
    let mut v2 = get_v2_map(env);
    let mut cur = get_migration_cursor(env);
    if cur < 1 {
        cur = 1;
    }

    let end_scan = cur.saturating_add((batch_size as u64).saturating_sub(1));
    let max_id = counter;
    let scan_to = if end_scan > max_id { max_id } else { end_scan };
    let mut batch_migrated: u32 = 0;

    let mut id = cur;
    while id <= scan_to {
        if let Some(v1_sig) = v1.get(id) {
            if v1_sig.id == id {
                let s2 = v1_to_v2(env, &v1_sig);
                v2.set(id, s2.clone());
                v1.remove(id);
                if s2.status == SignalStatus::Active {
                    add_to_category_index(env, id, s2.category.clone());
                }
                categories::increment_tag_popularity(env, &s2.tags);
                let _ = contests::auto_enter_signal(env, &s2);
                batch_migrated = batch_migrated.saturating_add(1);
            }
        }
        id = id.saturating_add(1);
    }

    save_v1_map(env, &v1);
    save_v2_map(env, &v2);
    set_migration_cursor(env, scan_to.saturating_add(1));
    if scan_to >= max_id {
        if count_v1_keys(&v1, counter) == 0 {
            set_migration_cursor(env, max_id.saturating_add(1));

            // Issue #812: v1 is now fully drained — advance the recorded
            // schema version so a future migration call takes the
            // already-migrated (no-op) path via `verify_storage_layout`.
            set_schema_version(env, SIGNAL_SCHEMA_V2);

            // Post-migration invariant verification (issue #597): v1 for this
            // scope is now fully drained, so reconcile the pre-migration
            // snapshot against the migrated v2 data over the same id range.
            // Runs automatically here, as part of this same migration call —
            // not a separate manual step. A mismatch does not panic (the
            // already-migrated data is left in place for inspection); it is
            // recorded via `MigrationVerification.verified = false` and an
            // event, flagging the migration as requiring manual review/rollback.
            if let Some(pre) = get_migration_pre_snapshot(env) {
                let post = snapshot_v2(&v2, max_id);
                let verification = reconcile(pre, post);
                set_migration_verification(env, &verification);
                if verification.verified {
                    emit_migration_verified(env, verification.post.clone());
                } else {
                    emit_migration_verification_failed(env, verification.clone());
                }
            }
        }
    }

    emit_migration_progress(
        env,
        MigrationProgress {
            migrated_count: batch_migrated,
            total_count: target_total,
        },
    );
    Ok(())
}

/// Test helper: only compiled for unit tests. Seeds v1, clears v2, resets migration metadata.
#[cfg(test)]
pub(crate) fn test_seed_v1_signals(env: &Env, count: u64) {
    use soroban_sdk::testutils::Address as _;
    if count == 0 {
        return;
    }
    let p = Address::generate(env);
    let mut m: Map<u64, SignalV1> = Map::new(env);
    let now = 1_000u64;
    let mut i: u64 = 1;
    while i <= count {
        let v = SignalV1 {
            id: i,
            provider: p.clone(),
            asset_pair: String::from_str(env, "XLM-USDC"),
            action: SignalAction::Buy,
            price: 100_000_000i128,
            rationale: String::from_str(env, "test rationale"),
            timestamp: now,
            expiry: now + 86_400,
            status: SignalStatus::Active,
            executions: 0,
            successful_executions: 0,
            total_volume: (i as i128) * 100,
            total_roi: 0,
            category: SignalCategory::SWING,
            tags: Vec::new(env),
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
        };
        m.set(i, v);
        i = i.saturating_add(1);
    }
    env.storage().instance().set(&StorageKey::SignalsV1, &m);
    let empty: Map<u64, Signal> = Map::new(env);
    env.storage().instance().set(&StorageKey::Signals, &empty);
    env.storage()
        .instance()
        .set(&StorageKey::SignalCounter, &count);
    env.storage()
        .instance()
        .set(&StorageKey::MigrationCursor, &1u64);
    env.storage()
        .instance()
        .remove(&StorageKey::MigrationV1TargetTotal);
}

#[cfg(test)]
mod migration_invariant_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        env.mock_all_auths();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    /// Clean migration: pre- and post-migration snapshots reconcile, and the
    /// verification result reflects that (issue #597).
    #[test]
    fn clean_migration_reconciles() {
        with_contract(|env| {
            let admin = Address::generate(env);
            test_seed_v1_signals(env, 37);

            // 37 records, batches of 10 -> 4 calls to fully drain v1.
            for _ in 0..4 {
                migrate_signals_v1_to_v2(env, &admin, 10).unwrap();
            }

            let verification = get_migration_verification(env).expect("verification recorded");
            assert!(
                verification.verified,
                "expected clean migration to reconcile: {verification:?}"
            );
            assert_eq!(verification.pre.record_count, 37);
            assert_eq!(verification.post.record_count, 37);
            assert_eq!(
                verification.pre.total_volume_sum,
                verification.post.total_volume_sum
            );
            // Sum of (i * 100) for i in 1..=37
            let expected_sum: i128 = (1..=37i128).map(|i| i * 100).sum();
            assert_eq!(verification.pre.total_volume_sum, expected_sum);
        });
    }

    /// Deliberately broken migration: a v1 record is corrupted (its
    /// total_volume changed) after the pre-migration snapshot was captured
    /// but before it gets migrated, simulating a migration-time data bug.
    /// The post-migration verification must detect the mismatch rather than
    /// silently reporting success (issue #597).
    #[test]
    fn deliberately_broken_migration_is_detected() {
        with_contract(|env| {
            let admin = Address::generate(env);
            test_seed_v1_signals(env, 20);

            // First batch captures the pre-migration snapshot over all 20
            // records (target_total/pre-snapshot are only set once, on the
            // first call), and migrates ids 1..=5.
            migrate_signals_v1_to_v2(env, &admin, 5).unwrap();

            // Corrupt an unmigrated v1 record's total_volume before it gets
            // its turn to migrate.
            let mut v1: Map<u64, SignalV1> = env
                .storage()
                .instance()
                .get(&StorageKey::SignalsV1)
                .unwrap();
            let mut tampered = v1.get(15).expect("record 15 not yet migrated");
            tampered.total_volume = tampered.total_volume.saturating_add(999_999);
            v1.set(15, tampered);
            env.storage().instance().set(&StorageKey::SignalsV1, &v1);

            // Drain the rest.
            for _ in 0..3 {
                migrate_signals_v1_to_v2(env, &admin, 5).unwrap();
            }

            let verification = get_migration_verification(env).expect("verification recorded");
            assert!(
                !verification.verified,
                "expected corrupted migration to be flagged as mismatched"
            );
            assert_eq!(
                verification.pre.record_count,
                verification.post.record_count
            );
            assert_ne!(
                verification.pre.total_volume_sum, verification.post.total_volume_sum,
                "corruption should have shifted the total_volume sum"
            );
            assert_eq!(
                verification.post.total_volume_sum - verification.pre.total_volume_sum,
                999_999
            );
        });
    }
}

// ═══════════════════════════════════════════════════════════════════
// Issue #812: storage layout migration guard regression tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod storage_layout_guard_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        env.mock_all_auths();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    /// A legacy (pre-guard) deployment never wrote `SchemaVersion`, so it
    /// defaults to `SIGNAL_SCHEMA_V1` — the guard must accept this and let
    /// migration proceed normally.
    #[test]
    fn migration_succeeds_with_default_legacy_schema_version() {
        with_contract(|env| {
            let admin = Address::generate(env);
            test_seed_v1_signals(env, 5);

            assert_eq!(get_schema_version(env), SIGNAL_SCHEMA_V1);
            assert!(migrate_signals_v1_to_v2(env, &admin, 10).is_ok());

            // Fully drained in one batch -> schema advances to v2.
            assert_eq!(get_schema_version(env), SIGNAL_SCHEMA_V2);
        });
    }

    /// A contract already fully migrated (schema == v2) takes the no-op
    /// path — the guard does not reject it, and re-running is safe.
    #[test]
    fn migration_is_noop_when_already_at_current_schema() {
        with_contract(|env| {
            let admin = Address::generate(env);
            set_schema_version(env, SIGNAL_SCHEMA_V2);

            assert!(migrate_signals_v1_to_v2(env, &admin, 10).is_ok());
            assert_eq!(get_schema_version(env), SIGNAL_SCHEMA_V2);
        });
    }

    /// An unrecognized schema version — simulating storage corruption or a
    /// future incompatible layout this migration code predates — must be
    /// rejected deterministically, before any migration state is touched.
    #[test]
    fn migration_rejects_incompatible_schema_version() {
        with_contract(|env| {
            let admin = Address::generate(env);
            test_seed_v1_signals(env, 5);
            // Simulate an incompatible/unknown on-chain schema.
            set_schema_version(env, 99);

            let result = migrate_signals_v1_to_v2(env, &admin, 10);
            assert_eq!(result, Err(AdminError::IncompatibleStorageLayout));

            // No migration state was touched: cursor still at the seeded
            // default, v1 map still fully populated, v2 map still empty.
            let v1: Map<u64, SignalV1> = env
                .storage()
                .instance()
                .get(&StorageKey::SignalsV1)
                .unwrap();
            assert_eq!(v1.len(), 5);
            let v2: Map<u64, Signal> = env.storage().instance().get(&StorageKey::Signals).unwrap();
            assert_eq!(v2.len(), 0);
            assert_eq!(get_schema_version(env), 99);
        });
    }

    /// `verify_storage_layout` is a pure precondition check independent of
    /// any contract state beyond the schema version key itself.
    #[test]
    fn verify_storage_layout_accepts_known_versions_only() {
        with_contract(|env| {
            set_schema_version(env, SIGNAL_SCHEMA_V1);
            assert!(verify_storage_layout(env).is_ok());

            set_schema_version(env, SIGNAL_SCHEMA_V2);
            assert!(verify_storage_layout(env).is_ok());

            set_schema_version(env, 42);
            assert_eq!(
                verify_storage_layout(env),
                Err(AdminError::IncompatibleStorageLayout)
            );
        });
    }

    /// `initialize()` sets a fresh contract's schema straight to v2 (it has
    /// no legacy v1 rows to migrate), so migration is a safe no-op even
    /// though `SignalsV1`/`Signals` were never seeded.
    #[test]
    fn fresh_initialize_sets_current_schema_version() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(crate::SignalRegistry, ());
        let client = crate::SignalRegistryClient::new(&env, &cid);
        client.initialize(&admin);

        env.as_contract(&cid, || {
            assert_eq!(get_schema_version(&env), SIGNAL_SCHEMA_V2);
        });
    }
}
