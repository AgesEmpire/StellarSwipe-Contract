//! Single-update price-deviation circuit breaker (Issue #755).
//!
//! Guards against a single feed glitch or manipulated source pushing an
//! abnormal price spike by comparing each new price against the immediately
//! preceding accepted price.  When the deviation exceeds the configured
//! maximum, the breaker trips and price-dependent entrypoints reject calls
//! for the affected asset until it recovers.
//!
//! # Recovery (Issue #1310)
//! A tripped breaker clears only when one of these conditions is met:
//! - an authorized admin reset, or
//! - [`RECOVERY_ROUNDS`] consecutive updates, each within the threshold of
//!   the last accepted (pre-trip) price.
//!
//! Recovery updates are counted but not stored, so a halted feed cannot
//! affect prices until the breaker clears; the update completing the streak
//! is the first one accepted. Any deviating update restarts the streak, so a
//! single marginal observation cannot toggle the feed between states.

use soroban_sdk::{contracttype, symbol_short, Address, Env};
use stellar_swipe_common::AssetPair;

use crate::{errors::OracleError, storage};

// ── Storage keys (re-use types::StorageKey variants added for this issue) ─────

use crate::types::StorageKey;

/// Consecutive within-threshold updates required to clear a tripped breaker.
pub const RECOVERY_ROUNDS: u32 = 3;

#[contracttype]
#[derive(Clone)]
enum CbKey {
    RecoveryStreak(AssetPair),
}

fn set_streak(env: &Env, pair: &AssetPair, streak: u32) {
    env.storage()
        .instance()
        .set(&CbKey::RecoveryStreak(pair.clone()), &streak);
}

/// Consecutive valid updates observed since the breaker tripped.
pub fn recovery_streak(env: &Env, pair: &AssetPair) -> u32 {
    env.storage()
        .instance()
        .get(&CbKey::RecoveryStreak(pair.clone()))
        .unwrap_or(0)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn is_tripped(env: &Env, pair: &AssetPair) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&StorageKey::DeviationBreakerTripped(pair.clone()))
        .unwrap_or(false)
}

fn threshold_bps(env: &Env, pair: &AssetPair) -> u32 {
    env.storage()
        .instance()
        .get::<_, u32>(&StorageKey::DeviationThreshold(pair.clone()))
        .unwrap_or(0)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Called before accepting a new price update.
///
/// If a threshold is configured and the deviation from the previous accepted
/// price exceeds it, the breaker is tripped (flag persisted + event emitted)
/// and `Ok(true)` is returned so the caller rejects the update without storing
/// it. The tripping call must *succeed* (return `Ok`) so the trip flag write
/// survives: Soroban rolls back all storage writes of a contract call that
/// returns an error, which would silently undo the trip.
///
/// Returns:
/// - `Ok(true)` — the update deviated past the threshold; breaker tripped,
///   update rejected (caller must not store the price).
/// - `Ok(false)` — the update is acceptable and should be stored.
///
/// While tripped, updates feed the recovery streak (see module docs) and
/// return `Ok(true)` until the streak completes, at which point the breaker
/// clears and `Ok(false)` accepts that update.
pub fn check_and_trip(env: &Env, pair: &AssetPair, new_price: i128) -> Result<bool, OracleError> {
    let max_bps = threshold_bps(env, pair);

    // Compare against the previous accepted price.
    let prev_price = match storage::get_price(env, pair) {
        Ok(p) if p > 0 => p,
        _ => return Ok(is_tripped(env, pair)), // No prior price — first update is always accepted.
    };

    let deviation_bps = ((new_price - prev_price).abs() * 10_000) / prev_price;
    let within = max_bps == 0 || deviation_bps <= max_bps as i128;

    if is_tripped(env, pair) {
        if !within {
            set_streak(env, pair, 0);
            return Ok(true);
        }
        let streak = recovery_streak(env, pair) + 1;
        if streak < RECOVERY_ROUNDS {
            set_streak(env, pair, streak);
            return Ok(true);
        }
        clear(env, pair);
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("dev_recov")),
            (pair.clone(), new_price),
        );
        return Ok(false);
    }

    if !within {
        trip(env, pair, prev_price, new_price, deviation_bps);
        return Ok(true);
    }

    Ok(false)
}

/// Persist the tripped flag and emit the `dev_trip` event.
fn trip(env: &Env, pair: &AssetPair, prev_price: i128, new_price: i128, deviation_bps: i128) {
    set_streak(env, pair, 0);
    env.storage()
        .instance()
        .set(&StorageKey::DeviationBreakerTripped(pair.clone()), &true);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("dev_trip")),
        (pair.clone(), prev_price, new_price, deviation_bps),
    );
}

/// Returns `PriceDeviationBreakerTripped` if the breaker is currently tripped
/// for this pair.  Price-dependent entrypoints call this before returning a price.
pub fn guard_tripped(env: &Env, pair: &AssetPair) -> Result<(), OracleError> {
    if is_tripped(env, pair) {
        Err(OracleError::PriceDeviationBreakerTripped)
    } else {
        Ok(())
    }
}

/// Admin: configure the maximum allowed single-update deviation in basis points.
/// Pass `0` to disable the check.
pub fn set_threshold(env: &Env, pair: &AssetPair, max_deviation_bps: u32) {
    env.storage().instance().set(
        &StorageKey::DeviationThreshold(pair.clone()),
        &max_deviation_bps,
    );
}

fn clear(env: &Env, pair: &AssetPair) {
    env.storage()
        .instance()
        .set(&StorageKey::DeviationBreakerTripped(pair.clone()), &false);
    set_streak(env, pair, 0);
}

/// Admin: reset the circuit breaker after manual review.
pub fn reset(env: &Env, pair: &AssetPair, admin: Address) {
    clear(env, pair);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("dev_reset")),
        (pair.clone(), admin),
    );
}

/// Returns whether the breaker is currently tripped.
pub fn is_breaker_tripped(env: &Env, pair: &AssetPair) -> bool {
    is_tripped(env, pair)
}

/// Returns the configured threshold in basis points (0 = disabled).
pub fn get_threshold(env: &Env, pair: &AssetPair) -> u32 {
    threshold_bps(env, pair)
}
