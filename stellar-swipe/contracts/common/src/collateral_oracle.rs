//! Oracle price-freshness validation for collateral / health checks (Issue #1031).
//!
//! Collateral valuation that trusts a stale oracle price can overvalue assets
//! and destabilise positions.  This module gates every collateral calculation
//! behind an explicit freshness window: a price whose timestamp is missing or
//! older than the configured window is rejected with a contract error *before*
//! any ratio math runs, so health-check logic only ever proceeds with valid,
//! current data.
//!
//! Freshness reuses the same semantics as [`crate::oracle::validate_freshness`]
//! (`timestamp == 0` or `now - timestamp > window` ⇒ stale) but the window is
//! independently configurable here because collateral checks are more
//! risk-sensitive than, say, informational quotes.
//!
//! Storage layout:
//!   CollateralOracleKey::Config -> CollateralOracleConfig  (instance)

#![allow(dead_code)]

use crate::oracle::{validate_price_bounds, OracleError, OraclePrice, MAX_PRICE_AGE_SECS};
use soroban_sdk::{contracttype, Env, Symbol};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default minimum collateralisation ratio: 150% (15 000 bps).
pub const DEFAULT_MIN_COLLATERAL_RATIO_BPS: u32 = 15_000;

/// Basis-point denominator (100% = 10 000 bps).
const BPS_DENOMINATOR: i128 = 10_000;

/// Oracle price feeds must report at most 18 decimals (matches the protocol's
/// token-metadata sanity bound); anything larger is treated as a corrupt feed.
const MAX_PRICE_DECIMALS: u32 = 18;

// ── Types ────────────────────────────────────────────────────────────────────

/// Explicit failure modes for collateral valuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CollateralError {
    /// The oracle price is older than the freshness window, or its timestamp
    /// is zero (never published). Risky calculations are skipped.
    PriceStale = 1,
    /// No usable price was supplied by the oracle.
    PriceUnavailable = 2,
    /// The price is outside `[MIN_ORACLE_PRICE, MAX_ORACLE_PRICE]` or reports
    /// an implausible number of decimals.
    InvalidPrice = 3,
    /// Collateral valuation overflowed i128 range.
    Arithmetic = 4,
}

impl From<OracleError> for CollateralError {
    fn from(e: OracleError) -> Self {
        match e {
            OracleError::PriceStale => CollateralError::PriceStale,
            OracleError::PriceNotFound | OracleError::NotConfigured | OracleError::CallFailed => {
                CollateralError::PriceUnavailable
            }
            OracleError::PriceBelowMin | OracleError::PriceAboveMax => {
                CollateralError::InvalidPrice
            }
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollateralOracleConfig {
    /// Maximum accepted price age, in seconds, for collateral checks.
    pub max_price_age_secs: u64,
    /// Minimum collateralisation ratio, in basis points, for a position to be
    /// considered healthy.
    pub min_collateral_ratio_bps: u32,
}

impl CollateralOracleConfig {
    fn effective_window(&self) -> u64 {
        self.max_price_age_secs.max(1)
    }
}

/// Outcome of a collateral health evaluation over a *validated, fresh* price.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollateralHealth {
    /// Collateral value in debt units (`quantity * price`, de-scaled by decimals).
    pub collateral_value: i128,
    /// Outstanding debt value in the same units.
    pub debt_value: i128,
    /// Collateralisation ratio in basis points, saturated at `u32::MAX`.
    pub ratio_bps: u32,
    /// `true` when `ratio_bps >= min_collateral_ratio_bps`.
    pub healthy: bool,
}

#[contracttype]
#[derive(Clone)]
enum CollateralOracleKey {
    Config,
}

// ── Config ───────────────────────────────────────────────────────────────────

pub fn default_config() -> CollateralOracleConfig {
    CollateralOracleConfig {
        max_price_age_secs: MAX_PRICE_AGE_SECS,
        min_collateral_ratio_bps: DEFAULT_MIN_COLLATERAL_RATIO_BPS,
    }
}

pub fn get_freshness_config(env: &Env) -> CollateralOracleConfig {
    env.storage()
        .instance()
        .get(&CollateralOracleKey::Config)
        .unwrap_or_else(default_config)
}

/// Persist a new collateral-oracle configuration. `max_price_age_secs` is
/// floored at 1 second.
pub fn set_freshness_config(env: &Env, mut config: CollateralOracleConfig) {
    config.max_price_age_secs = config.effective_window();
    env.storage()
        .instance()
        .set(&CollateralOracleKey::Config, &config);
}

// ── Freshness gate ───────────────────────────────────────────────────────────

/// Enforce the freshness window on `price` before it is accepted for a
/// collateral calculation. Emits `collateral_price_rejected` and returns
/// [`CollateralError::PriceStale`] when the price is missing or too old.
pub fn require_fresh_price(
    env: &Env,
    price: &OraclePrice,
    max_age_secs: u64,
) -> Result<(), CollateralError> {
    let window = max_age_secs.max(1);
    let now = env.ledger().timestamp();
    if price.timestamp == 0 || now.saturating_sub(price.timestamp) > window {
        emit_price_rejected(env, &price.source, price.timestamp, now);
        return Err(CollateralError::PriceStale);
    }
    Ok(())
}

// ── Health evaluation ────────────────────────────────────────────────────────

/// Evaluate collateral health for a position.
///
/// The price is checked for freshness first; a stale price returns
/// [`CollateralError::PriceStale`] and **no valuation is attempted**. Only once
/// the price is confirmed fresh and within bounds does the ratio math run.
///
/// * `collateral_qty` – raw collateral quantity (same scale as on-ledger balance).
/// * `debt_value` – outstanding debt already expressed in debt units; a
///   non-positive value means "no debt" (always healthy).
pub fn evaluate_collateral_health(
    env: &Env,
    price: &OraclePrice,
    collateral_qty: i128,
    debt_value: i128,
    min_ratio_bps: u32,
    max_age_secs: u64,
) -> Result<CollateralHealth, CollateralError> {
    require_fresh_price(env, price, max_age_secs)?;

    if price.decimals > MAX_PRICE_DECIMALS {
        return Err(CollateralError::InvalidPrice);
    }
    validate_price_bounds(price).map_err(CollateralError::from)?;
    if collateral_qty < 0 {
        return Err(CollateralError::InvalidPrice);
    }

    let scale = 10i128.pow(price.decimals);
    let collateral_value = collateral_qty
        .checked_mul(price.price)
        .and_then(|v| v.checked_div(scale))
        .ok_or(CollateralError::Arithmetic)?;

    if debt_value <= 0 {
        return Ok(CollateralHealth {
            collateral_value,
            debt_value: 0,
            ratio_bps: u32::MAX,
            healthy: true,
        });
    }

    let ratio_i128 = collateral_value
        .checked_mul(BPS_DENOMINATOR)
        .and_then(|v| v.checked_div(debt_value))
        .ok_or(CollateralError::Arithmetic)?;
    let ratio_bps = if ratio_i128 < 0 {
        0
    } else if ratio_i128 > u32::MAX as i128 {
        u32::MAX
    } else {
        ratio_i128 as u32
    };

    Ok(CollateralHealth {
        collateral_value,
        debt_value,
        ratio_bps,
        healthy: ratio_bps >= min_ratio_bps,
    })
}

/// Convenience wrapper using the stored [`CollateralOracleConfig`].
pub fn evaluate_with_config(
    env: &Env,
    price: &OraclePrice,
    collateral_qty: i128,
    debt_value: i128,
) -> Result<CollateralHealth, CollateralError> {
    let cfg = get_freshness_config(env);
    evaluate_collateral_health(
        env,
        price,
        collateral_qty,
        debt_value,
        cfg.min_collateral_ratio_bps,
        cfg.max_price_age_secs,
    )
}

// ── Event ────────────────────────────────────────────────────────────────────

fn emit_price_rejected(env: &Env, source: &Symbol, price_ts: u64, now: u64) {
    let topics = (
        Symbol::new(env, "collateral_price_rejected"),
        source.clone(),
    );
    env.events().publish(topics, (price_ts, now));
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{MAX_ORACLE_PRICE, MIN_ORACLE_PRICE};
    use soroban_sdk::{contract, contractimpl, symbol_short, testutils::Ledger, Env};

    #[contract]
    struct CollateralHarness;

    #[contractimpl]
    impl CollateralHarness {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let id = env.register(CollateralHarness, ());
        env.ledger().set_timestamp(1_000_000);
        (env, id)
    }

    fn price_at(ts: u64, value: i128, decimals: u32) -> OraclePrice {
        OraclePrice {
            price: value,
            decimals,
            timestamp: ts,
            source: symbol_short!("band"),
        }
    }

    #[test]
    fn stale_price_is_rejected_before_any_math() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let stale = price_at(now - MAX_PRICE_AGE_SECS - 1, 1_000_000, 6);
            let res =
                evaluate_collateral_health(&env, &stale, 1_000, 500, 15_000, MAX_PRICE_AGE_SECS);
            assert_eq!(res, Err(CollateralError::PriceStale));
        });
    }

    #[test]
    fn zero_timestamp_is_stale() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let never = price_at(0, 1_000_000, 6);
            assert_eq!(
                require_fresh_price(&env, &never, MAX_PRICE_AGE_SECS),
                Err(CollateralError::PriceStale)
            );
        });
    }

    #[test]
    fn exactly_at_window_edge_is_fresh() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let edge = price_at(now - MAX_PRICE_AGE_SECS, 1_000_000, 6);
            assert!(require_fresh_price(&env, &edge, MAX_PRICE_AGE_SECS).is_ok());
        });
    }

    #[test]
    fn fresh_healthy_position() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            // price 2.0 (decimals 6), 1000 units collateral => value 2000, debt 1000 => 200%
            let price = price_at(now - 10, 2_000_000, 6);
            let h =
                evaluate_collateral_health(&env, &price, 1_000, 1_000, 15_000, MAX_PRICE_AGE_SECS)
                    .unwrap();
            assert_eq!(h.collateral_value, 2_000);
            assert_eq!(h.ratio_bps, 20_000);
            assert!(h.healthy);
        });
    }

    #[test]
    fn fresh_undercollateralised_position() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            // price 1.0, 1000 collateral => value 1000, debt 1000 => 100% < 150%
            let price = price_at(now - 10, 1_000_000, 6);
            let h =
                evaluate_collateral_health(&env, &price, 1_000, 1_000, 15_000, MAX_PRICE_AGE_SECS)
                    .unwrap();
            assert_eq!(h.ratio_bps, 10_000);
            assert!(!h.healthy);
        });
    }

    #[test]
    fn no_debt_is_always_healthy() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let price = price_at(now, 1_000_000, 6);
            let h =
                evaluate_collateral_health(&env, &price, 1, 0, 15_000, MAX_PRICE_AGE_SECS).unwrap();
            assert!(h.healthy);
            assert_eq!(h.ratio_bps, u32::MAX);
        });
    }

    #[test]
    fn price_below_min_is_invalid() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let bad = price_at(now, MIN_ORACLE_PRICE - 1, 6);
            assert_eq!(
                evaluate_collateral_health(&env, &bad, 1_000, 1_000, 15_000, MAX_PRICE_AGE_SECS),
                Err(CollateralError::InvalidPrice)
            );
        });
    }

    #[test]
    fn price_above_max_is_invalid() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let bad = price_at(now, MAX_ORACLE_PRICE + 1, 6);
            assert_eq!(
                evaluate_collateral_health(&env, &bad, 1, 1, 15_000, MAX_PRICE_AGE_SECS),
                Err(CollateralError::InvalidPrice)
            );
        });
    }

    #[test]
    fn implausible_decimals_are_invalid() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let bad = price_at(now, 1_000, 30);
            assert_eq!(
                evaluate_collateral_health(&env, &bad, 1, 1, 15_000, MAX_PRICE_AGE_SECS),
                Err(CollateralError::InvalidPrice)
            );
        });
    }

    #[test]
    fn tighter_custom_window_rejects_borderline_price() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let now = env.ledger().timestamp();
            let price = price_at(now - 120, 1_000_000, 6);
            // Default 300s window: fresh.
            assert!(require_fresh_price(&env, &price, MAX_PRICE_AGE_SECS).is_ok());
            // Tighter 60s window: stale.
            assert_eq!(
                require_fresh_price(&env, &price, 60),
                Err(CollateralError::PriceStale)
            );
        });
    }

    #[test]
    fn config_round_trips_and_defaults() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            assert_eq!(get_freshness_config(&env), default_config());
            set_freshness_config(
                &env,
                CollateralOracleConfig {
                    max_price_age_secs: 0,
                    min_collateral_ratio_bps: 12_000,
                },
            );
            let cfg = get_freshness_config(&env);
            assert_eq!(cfg.max_price_age_secs, 1);
            assert_eq!(cfg.min_collateral_ratio_bps, 12_000);
        });
    }

    #[test]
    fn evaluate_with_config_uses_stored_window() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_freshness_config(
                &env,
                CollateralOracleConfig {
                    max_price_age_secs: 30,
                    min_collateral_ratio_bps: 15_000,
                },
            );
            let now = env.ledger().timestamp();
            let price = price_at(now - 45, 1_000_000, 6);
            assert_eq!(
                evaluate_with_config(&env, &price, 1_000, 1_000),
                Err(CollateralError::PriceStale)
            );
        });
    }
}
