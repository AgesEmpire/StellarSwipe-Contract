//! V1 → V2 portfolio state migration.
//!
//! V1 layout: `UserPositions(user) -> Vec<u64>` (open + closed mixed).
//! V2 layout: `UserOpenPositions(user) -> Vec<u64>` + `UserClosedPositions(user) -> Vec<u64>`.
//!
//! Call `migrate_portfolio_v1_to_v2(batch_size)` as admin after contract upgrade.
//! Idempotent: already-migrated users are skipped via `MigratedUser(user)` flag.

use crate::storage::DataKey;
use crate::{Position, PositionStatus};
use soroban_sdk::{symbol_short, Address, Env, Vec};

/// Migrate one user's positions from V1 to V2 layout.
/// Returns `(open_migrated, closed_migrated)`.
/// Panics if open position count after migration doesn't match V1.
pub fn migrate_user(env: &Env, user: &Address) -> (u32, u32) {
    // Already migrated — skip.
    if env
        .storage()
        .persistent()
        .has(&DataKey::MigratedUser(user.clone()))
    {
        return (0, 0);
    }

    let v1_ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));

    let mut open_ids: Vec<u64> = Vec::new(env);
    let mut closed_ids: Vec<u64> = Vec::new(env);

    for i in 0..v1_ids.len() {
        let id = v1_ids.get_unchecked(i);
        let pos: Position = env
            .storage()
            .persistent()
            .get(&DataKey::Position(id))
            .expect("position data missing during migration");
        match pos.status {
            PositionStatus::Open => open_ids.push_back(id),
            PositionStatus::Closed => closed_ids.push_back(id),
        }
    }

    let open_count = open_ids.len();
    let closed_count = closed_ids.len();

    // Sanity: open + closed must equal total V1 positions.
    assert_eq!(
        open_count + closed_count,
        v1_ids.len(),
        "position count mismatch: open+closed != v1 total"
    );

    env.storage()
        .persistent()
        .set(&DataKey::UserOpenPositions(user.clone()), &open_ids);
    env.storage()
        .persistent()
        .set(&DataKey::UserClosedPositions(user.clone()), &closed_ids);

    // Verify open count preserved after write.
    let written_open: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserOpenPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    assert_eq!(
        written_open.len(),
        open_count,
        "open position count mismatch after write"
    );

    // Mark migrated.
    env.storage()
        .persistent()
        .set(&DataKey::MigratedUser(user.clone()), &true);

    emit_migration_complete(env, user, open_count, closed_count);

    (open_count, closed_count)
}

/// Batch-migrate up to `batch_size` users from the pending queue.
/// Admin must have pre-populated `MigrationQueue` via `register_migration_users`.
pub fn migrate_batch(env: &Env, batch_size: u32) -> u32 {
    let mut queue: Vec<Address> = env
        .storage()
        .instance()
        .get(&DataKey::MigrationQueue)
        .unwrap_or_else(|| Vec::new(env));

    let to_process = batch_size.min(queue.len());
    let mut processed = 0u32;

    // Process from the tail to avoid shifting.
    for _ in 0..to_process {
        let last = queue.len() - 1;
        let user = queue.get_unchecked(last);
        queue.remove(last);
        migrate_user(env, &user);
        processed += 1;
    }

    env.storage()
        .instance()
        .set(&DataKey::MigrationQueue, &queue);

    processed
}

fn emit_migration_complete(env: &Env, user: &Address, open: u32, closed: u32) {
    env.events().publish(
        (symbol_short!("mig_done"), user.clone()),
        (open, closed),
    );
}
