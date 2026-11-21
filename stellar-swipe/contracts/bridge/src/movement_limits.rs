//! Per-asset aggregate bridge movement limits (Issue #1417).
//!
//! An admin can cap the total amount of one asset that moves through the
//! bridge (lock/mint and burn/unlock alike) within a fixed ledger-time window.
//! Windows are aligned buckets: `window_start = (now / window_seconds) *
//! window_seconds`, so rollover is deterministic and independent of when the
//! first movement in a window happened.
//!
//! The check is split into [`check_movement`] (pure, no writes) and
//! [`commit_movement`] so callers can run every limit check first and only
//! persist accounting once the whole movement is known to succeed. A limit
//! breach therefore fails before any bridge state or token balance changes.

use soroban_sdk::{contracttype, Env, String, Symbol};

use crate::BridgeError;

/// Admin-configured aggregate movement limit for one asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMovementLimit {
    /// Maximum aggregate amount allowed per window. Always `> 0`.
    pub limit: i128,
    /// Window length in seconds. Always `> 0`.
    pub window_seconds: u64,
}

/// Accumulated movement for an asset in the current window.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMovementWindow {
    /// Aligned start of the window this total belongs to.
    pub window_start: u64,
    /// Total amount moved in this window.
    pub total_moved: i128,
}

/// Storage keys. Variant names are unique across the crate so they cannot
/// collide with other `contracttype` key enums.
#[contracttype]
pub enum MovementLimitKey {
    AssetMovementLimit(String),
    AssetMovementWindow(String),
}

fn window_bucket(now: u64, window_seconds: u64) -> u64 {
    (now / window_seconds) * window_seconds
}

pub fn get_limit(env: &Env, asset: &String) -> Option<AssetMovementLimit> {
    env.storage()
        .persistent()
        .get(&MovementLimitKey::AssetMovementLimit(asset.clone()))
}

/// Current-window usage for `asset`, rolled over to the current bucket so a
/// stale stored window reads as zero. Returns a zeroed window when no limit
/// is configured.
pub fn get_usage(env: &Env, asset: &String) -> AssetMovementWindow {
    let now = env.ledger().timestamp();
    match get_limit(env, asset) {
        Some(limit) => current_window(env, asset, &limit, now),
        None => AssetMovementWindow {
            window_start: now,
            total_moved: 0,
        },
    }
}

fn current_window(
    env: &Env,
    asset: &String,
    limit: &AssetMovementLimit,
    now: u64,
) -> AssetMovementWindow {
    let bucket = window_bucket(now, limit.window_seconds);
    let stored: Option<AssetMovementWindow> = env
        .storage()
        .persistent()
        .get(&MovementLimitKey::AssetMovementWindow(asset.clone()));
    match stored {
        Some(w) if w.window_start == bucket => w,
        _ => AssetMovementWindow {
            window_start: bucket,
            total_moved: 0,
        },
    }
}

/// Set or update the limit for `asset`.
///
/// * Changing only `limit` keeps the volume already moved in the current
///   window, so a lowered limit takes effect immediately.
/// * Changing `window_seconds` starts a fresh window at the current aligned
///   bucket, because the old total belongs to a different bucket layout.
pub fn set_limit(
    env: &Env,
    asset: String,
    limit: i128,
    window_seconds: u64,
) -> Result<(), BridgeError> {
    if limit <= 0 || window_seconds == 0 {
        return Err(BridgeError::InvalidAmount);
    }
    let now = env.ledger().timestamp();
    let previous = get_limit(env, &asset);
    let new_limit = AssetMovementLimit {
        limit,
        window_seconds,
    };

    let window = match previous {
        Some(ref prev) if prev.window_seconds == window_seconds => {
            current_window(env, &asset, prev, now)
        }
        _ => AssetMovementWindow {
            window_start: window_bucket(now, window_seconds),
            total_moved: 0,
        },
    };

    env.storage()
        .persistent()
        .set(&MovementLimitKey::AssetMovementLimit(asset.clone()), &new_limit);
    env.storage()
        .persistent()
        .set(&MovementLimitKey::AssetMovementWindow(asset.clone()), &window);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge"),
            Symbol::new(env, "asset_limit_set"),
            asset,
        ),
        (limit, window_seconds, window.window_start, window.total_moved),
    );
    Ok(())
}

/// Remove the limit for `asset`; movement of that asset becomes unlimited
/// by this mechanism (other bridge limits still apply).
pub fn remove_limit(env: &Env, asset: String) {
    env.storage()
        .persistent()
        .remove(&MovementLimitKey::AssetMovementLimit(asset.clone()));
    env.storage()
        .persistent()
        .remove(&MovementLimitKey::AssetMovementWindow(asset.clone()));

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge"),
            Symbol::new(env, "asset_limit_removed"),
            asset,
        ),
        env.ledger().timestamp(),
    );
}

/// Pure check: returns the window that would be stored if `amount` moves now,
/// or `None` when the asset has no limit. Uses checked accumulation; an
/// overflow is treated as crossing the limit. Writes nothing.
pub fn check_movement(
    env: &Env,
    asset: &String,
    amount: i128,
) -> Result<Option<AssetMovementWindow>, BridgeError> {
    let limit = match get_limit(env, asset) {
        Some(l) => l,
        None => return Ok(None),
    };
    let mut window = current_window(env, asset, &limit, env.ledger().timestamp());
    let new_total = window
        .total_moved
        .checked_add(amount)
        .ok_or(BridgeError::AssetMovementLimitExceeded)?;
    if new_total > limit.limit {
        return Err(BridgeError::AssetMovementLimitExceeded);
    }
    window.total_moved = new_total;
    Ok(Some(window))
}

/// Persist a window previously returned by [`check_movement`].
pub fn commit_movement(env: &Env, asset: &String, window: &AssetMovementWindow) {
    env.storage()
        .persistent()
        .set(&MovementLimitKey::AssetMovementWindow(asset.clone()), window);
}
