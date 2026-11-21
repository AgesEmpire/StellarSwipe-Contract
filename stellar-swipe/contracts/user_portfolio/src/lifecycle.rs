//! Portfolio lifecycle helpers:
//! - #1370 bounded number of active assets per portfolio
//! - #1371 revision-checked snapshot writes
//! - #1372 closure precondition (no unsettled trades)
//! - #1373 zero-balance position cleanup with settlement receipt

use soroban_sdk::{Address, Env, Vec};

use crate::storage::DataKey;
use crate::{exposure_cap, PortfolioError, Position, PositionStatus, TaxEvent};

/// Maximum number of distinct active assets one portfolio may hold (#1370).
pub const MAX_PORTFOLIO_ASSETS: u32 = 20;

pub fn active_assets(env: &Env, user: &Address) -> Vec<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::UserActiveAssets(user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn asset_count(env: &Env, user: &Address, asset_id: u32) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::UserAssetPositionCount(user.clone(), asset_id))
        .unwrap_or(0)
}

/// Fails with `AssetCapacityReached` if `asset_id` would occupy a new slot while full.
/// Assets that already hold an active position are always allowed.
pub fn check_asset_capacity(env: &Env, user: &Address, asset_id: u32) -> Result<(), PortfolioError> {
    if asset_count(env, user, asset_id) == 0
        && active_assets(env, user).len() >= MAX_PORTFOLIO_ASSETS
    {
        return Err(PortfolioError::AssetCapacityReached);
    }
    Ok(())
}

/// Register an open position for `asset_id`, occupying a slot if new.
pub fn track_asset_open(env: &Env, user: &Address, asset_id: u32) {
    let count = asset_count(env, user, asset_id);
    if count == 0 {
        let mut list = active_assets(env, user);
        list.push_back(asset_id);
        env.storage()
            .persistent()
            .set(&DataKey::UserActiveAssets(user.clone()), &list);
    }
    env.storage().persistent().set(
        &DataKey::UserAssetPositionCount(user.clone(), asset_id),
        &(count + 1),
    );
}

/// Release the position's asset slot (if any) and its remaining exposure.
/// Called whenever a position leaves the active set.
pub fn on_position_closed(env: &Env, user: &Address, position_id: u64, remaining: i128) {
    let asset_id: Option<u32> = env
        .storage()
        .persistent()
        .get(&DataKey::PositionAsset(position_id));
    let Some(asset_id) = asset_id else { return };

    if remaining > 0 {
        exposure_cap::remove_exposure(env, user, asset_id, remaining);
    }
    let count = asset_count(env, user, asset_id);
    let count_key = DataKey::UserAssetPositionCount(user.clone(), asset_id);
    if count <= 1 {
        env.storage().persistent().remove(&count_key);
        let list = active_assets(env, user);
        let mut next = Vec::new(env);
        for a in list.iter() {
            if a != asset_id {
                next.push_back(a);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::UserActiveAssets(user.clone()), &next);
    } else {
        env.storage().persistent().set(&count_key, &(count - 1));
    }
}

/// Reduce tracked exposure after a partial close (#1373).
pub fn on_partial_close(env: &Env, user: &Address, position_id: u64, closed: i128) {
    if let Some(asset_id) = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKey::PositionAsset(position_id))
    {
        exposure_cap::remove_exposure(env, user, asset_id, closed);
    }
}

/// Persist a settlement receipt for a position drained to zero (#1373).
pub fn record_settlement_receipt(
    env: &Env,
    user: &Address,
    position_id: u64,
    pos: &Position,
    closed_amount: i128,
    realized_pnl: i128,
) {
    let open_ts: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::PositionOpenedAt(position_id))
        .unwrap_or(0);
    let asset_pair: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::PositionAsset(position_id))
        .unwrap_or(0);
    let key = DataKey::UserTaxEvents(user.clone());
    let mut events: Vec<TaxEvent> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    events.push_back(TaxEvent {
        position_id,
        open_ts,
        close_ts: env.ledger().timestamp(),
        entry_price: pos.entry_price,
        exit_price: 0,
        amount: closed_amount,
        realized_pnl,
        asset_pair,
    });
    env.storage().persistent().set(&key, &events);
}

// ── #1371 snapshot revisions ────────────────────────────────────────────────

pub fn snapshot_revision(env: &Env, user: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::SnapshotRevision(user.clone()))
        .unwrap_or(0)
}

/// Verify `expected` matches the stored revision and advance it exactly once.
pub fn advance_snapshot_revision(
    env: &Env,
    user: &Address,
    expected: u64,
) -> Result<u64, PortfolioError> {
    let current = snapshot_revision(env, user);
    if current != expected {
        return Err(PortfolioError::StaleSnapshotRevision);
    }
    let next = current.checked_add(1).expect("revision overflow");
    env.storage()
        .persistent()
        .set(&DataKey::SnapshotRevision(user.clone()), &next);
    Ok(next)
}

// ── #1372 closure precondition ──────────────────────────────────────────────

/// True if any position of `user` is still `Open` or `Closing` (unsettled).
pub fn has_unsettled_trades(env: &Env, user: &Address) -> bool {
    let open: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserOpenPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    if !open.is_empty() {
        return true;
    }
    let all: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    for id in all.iter() {
        if let Some(p) = env
            .storage()
            .persistent()
            .get::<_, Position>(&DataKey::Position(id))
        {
            if p.status != PositionStatus::Closed {
                return true;
            }
        }
    }
    false
}

/// Delete active portfolio data. Settlement receipts (tax events) are preserved.
/// Caller must have verified `has_unsettled_trades` is false first.
pub fn delete_portfolio(env: &Env, user: &Address) {
    let s = env.storage().persistent();
    let timestamps: Vec<u64> = s
        .get(&DataKey::UserSnapshotTimestamps(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    for ts in timestamps.iter() {
        s.remove(&DataKey::PortfolioSnapshotEntry(user.clone(), ts));
    }
    s.remove(&DataKey::UserSnapshotTimestamps(user.clone()));
    s.remove(&DataKey::UserOpenPositions(user.clone()));
    s.remove(&DataKey::UserClosedPositions(user.clone()));
    s.remove(&DataKey::UserPositions(user.clone()));
    s.remove(&DataKey::UserActiveAssets(user.clone()));
    s.remove(&DataKey::SnapshotRevision(user.clone()));
}
