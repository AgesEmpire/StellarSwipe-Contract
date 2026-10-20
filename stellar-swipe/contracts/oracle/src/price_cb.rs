//! Single-update price-deviation circuit breaker (Issue #755).
//!
//! Guards against a single feed glitch or manipulated source pushing an
//! abnormal price spike by comparing each new price against the immediately
//! preceding accepted price.  When the deviation exceeds the configured
//! maximum, the breaker trips and price-dependent entrypoints reject calls
//! for the affected asset until an admin explicitly resets it.

use soroban_sdk::{symbol_short, Address, Env};
use stellar_swipe_common::AssetPair;

use crate::{errors::OracleError, storage};

// ── Storage keys (re-use types::StorageKey variants added for this issue) ─────

use crate::types::StorageKey;

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
/// - `Err(PriceDeviationBreakerTripped)` — the breaker is already tripped;
///   reject the update (no writes are made in this path, so nothing reverts).
pub fn check_and_trip(env: &Env, pair: &AssetPair, new_price: i128) -> Result<bool, OracleError> {
    // If already tripped, reject all new updates until admin resets.
    if is_tripped(env, pair) {
        return Err(OracleError::PriceDeviationBreakerTripped);
    }

    let max_bps = threshold_bps(env, pair);
    if max_bps == 0 {
        return Ok(false);
    }

    // Compare against the previous accepted price.
    let prev_price = match storage::get_price(env, pair) {
        Ok(p) if p > 0 => p,
        _ => return Ok(false), // No prior price — first update is always accepted.
    };

    let deviation_bps = ((new_price - prev_price).abs() * 10_000) / prev_price;
    if deviation_bps as u32 > max_bps {
        trip(env, pair, prev_price, new_price, deviation_bps);
        return Ok(true);
    }

    Ok(false)
}

/// Persist the tripped flag and emit the `dev_trip` event.
fn trip(env: &Env, pair: &AssetPair, prev_price: i128, new_price: i128, deviation_bps: i128) {
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

/// Admin: reset the circuit breaker after manual review.
pub fn reset(env: &Env, pair: &AssetPair, admin: Address) {
    env.storage()
        .instance()
        .set(&StorageKey::DeviationBreakerTripped(pair.clone()), &false);

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
