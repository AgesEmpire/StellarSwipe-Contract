//! v1 → v2 signal storage migration. Unmigrated records live in [`StorageKey::SignalsV1`];
//! canonical v2 data is in [`StorageKey::Signals`]. Re-running the migration is safe: only
//! ids with a v1 record are transformed; v1 is removed when written to v2.

use crate::categories;
use crate::categories::{RiskLevel, SignalCategory};
use crate::contests;
use crate::errors::AdminError;
use crate::events::emit_migration_progress;
use crate::types::{MigrationProgress, Signal, SignalAction, SignalStatus, SignalV1};
use crate::StorageKey;
use soroban_sdk::{Address, Env, Map, String, Vec};

const MAX_MIGRATION_BATCH: u32 = 256;

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
    }
}

fn get_v1_map(env: &Env) -> Map<u64, SignalV1> {
    env.storage()
        .instance()
        .get(&StorageKey::SignalsV1)
        .unwrap_or(Map::new(env))
}

fn save_v1_map(env: &Env, m: &Map<u64, SignalV1>) {
    env.storage()
        .instance()
        .set(&StorageKey::SignalsV1, m);
}

fn get_v2_map(env: &Env) -> Map<u64, Signal> {
    env.storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env))
}

fn save_v2_map(env: &Env, m: &Map<u64, Signal>) {
    env.storage()
        .instance()
        .set(&StorageKey::Signals, m);
}

fn get_migration_cursor(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&StorageKey::MigrationCursor)
        .unwrap_or(1u64)
}

fn set_migration_cursor(env: &Env, c: u64) {
    env.storage().instance().set(&StorageKey::MigrationCursor, &c);
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
fn count_v1_keys(_env: &Env, v1: &Map<u64, SignalV1>, max_id: u64) -> u32 {
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
    if count_v1_keys(env, &v1, counter) == 0 {
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
        set_migration_v1_target_total(
            env,
            count_v1_keys(env, &v1, counter),
        );
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
        if count_v1_keys(env, &v1, counter) == 0 {
            set_migration_cursor(env, max_id.saturating_add(1));
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
            total_volume: 0,
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
    env.storage().instance().set(&StorageKey::MigrationCursor, &1u64);
    env.storage().instance().remove(&StorageKey::MigrationV1TargetTotal);
}