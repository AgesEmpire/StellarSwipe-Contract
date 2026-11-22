//! Hysteresis for oracle deviation circuit breakers (Issue #1409).
//!
//! The single-threshold breaker in `price_cb` toggles the moment deviation
//! crosses one value, which lets a feed oscillate around that boundary and
//! flip rapidly between tripped and cleared. Hysteresis solves this by
//! requiring a *lower* threshold to be sustained before the breaker can
//! recover: a feed trips at `trigger_bps` and cannot clear until it shows
//! `healthy_rounds` consecutive updates within `recovery_bps`, where
//! `recovery_bps < trigger_bps`.
//!
//! # Thresholds
//! - **Trigger** (`trigger_bps`): deviation from the last accepted price at
//!   which the breaker trips. Equivalent to the single threshold in `price_cb`.
//! - **Recovery** (`recovery_bps`): tighter bound a feed must sustain before
//!   it can exit the tripped state. Must be strictly less than `trigger_bps`
//!   (validated at configuration time). A value of 0 is accepted for trigger
//!   only when recovery is also 0 (both disabled).
//!
//! # Recovery condition
//! `healthy_rounds` consecutive observations, each within `recovery_bps` of
//! the last accepted price, are required to clear the breaker. Any deviating
//! update resets the streak count without tripping again (since we are already
//! tripped). The observation that completes the streak is the first one stored.
//!
//! # Storage
//! All state is per asset pair and keyed under [`HysteresisKey`]. The config
//! and streak counter are stored in instance storage (small, frequently
//! accessed). Trip state is a boolean under the same namespace.

use soroban_sdk::{contracttype, symbol_short, Env};
use stellar_swipe_common::AssetPair;

use crate::errors::OracleError;
use crate::storage;

// ── Default constants ─────────────────────────────────────────────────────────

/// Default consecutive healthy-observation count required for recovery.
pub const DEFAULT_HEALTHY_ROUNDS: u32 = 3;

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum HysteresisKey {
    /// Hysteresis configuration (trigger + recovery thresholds + healthy rounds).
    Config(AssetPair),
    /// Whether the hysteresis breaker is currently tripped for this pair.
    Tripped(AssetPair),
    /// Count of consecutive within-recovery-threshold observations since trip.
    RecoveryStreak(AssetPair),
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Hysteresis circuit-breaker configuration for one asset pair.
///
/// # Validation
/// `recovery_bps` must be strictly less than `trigger_bps` unless both are
/// zero (disabled). `healthy_rounds` must be at least 1 when thresholds are
/// non-zero. These invariants are enforced by [`validate_config`].
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HysteresisConfig {
    /// Deviation (basis points, 100 = 1%) above which the breaker trips.
    /// 0 disables the check entirely.
    pub trigger_bps: u32,
    /// Tighter bound (basis points) a feed must sustain before recovery.
    /// Must be strictly less than `trigger_bps` when non-zero.
    pub recovery_bps: u32,
    /// Consecutive within-`recovery_bps` updates required to clear the breaker.
    pub healthy_rounds: u32,
}

impl Default for HysteresisConfig {
    fn default() -> Self {
        Self {
            trigger_bps: 500,
            recovery_bps: 200,
            healthy_rounds: DEFAULT_HEALTHY_ROUNDS,
        }
    }
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate a [`HysteresisConfig`] before persisting it.
///
/// # Errors
/// - [`OracleError::InvalidPrice`] is reused here as a generic "bad config"
///   sentinel when `recovery_bps >= trigger_bps` (non-zero) or when
///   `healthy_rounds == 0` with non-zero thresholds.
///
/// A dedicated config-error variant would be cleaner but would require a
/// protocol-level error addition. Using `InvalidPrice` keeps the existing
/// error surface intact; the doc comment makes the intent clear.
pub fn validate_config(cfg: &HysteresisConfig) -> Result<(), OracleError> {
    if cfg.trigger_bps == 0 && cfg.recovery_bps == 0 {
        // Both disabled — healthy_rounds is irrelevant.
        return Ok(());
    }
    if cfg.trigger_bps == 0 {
        // recovery without trigger makes no sense.
        return Err(OracleError::InvalidPrice);
    }
    if cfg.recovery_bps >= cfg.trigger_bps {
        return Err(OracleError::InvalidPrice);
    }
    if cfg.healthy_rounds == 0 {
        return Err(OracleError::InvalidPrice);
    }
    Ok(())
}

// ── Admin API ─────────────────────────────────────────────────────────────────

/// Persist a validated hysteresis configuration for `pair`.
///
/// # Errors
/// Forwards [`validate_config`] errors.
pub fn set_config(env: &Env, pair: &AssetPair, cfg: HysteresisConfig) -> Result<(), OracleError> {
    validate_config(&cfg)?;
    env.storage()
        .instance()
        .set(&HysteresisKey::Config(pair.clone()), &cfg);
    Ok(())
}

/// Read the hysteresis configuration for `pair`.
/// Returns `None` when no configuration has been set.
pub fn get_config(env: &Env, pair: &AssetPair) -> Option<HysteresisConfig> {
    env.storage()
        .instance()
        .get(&HysteresisKey::Config(pair.clone()))
}

// ── State queries ─────────────────────────────────────────────────────────────

/// Whether the hysteresis breaker is currently tripped for `pair`.
pub fn is_tripped(env: &Env, pair: &AssetPair) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&HysteresisKey::Tripped(pair.clone()))
        .unwrap_or(false)
}

/// Count of consecutive within-recovery-threshold observations since the
/// breaker last tripped.
pub fn recovery_streak(env: &Env, pair: &AssetPair) -> u32 {
    env.storage()
        .instance()
        .get(&HysteresisKey::RecoveryStreak(pair.clone()))
        .unwrap_or(0)
}

// ── Core logic ────────────────────────────────────────────────────────────────

/// Guard: return `Err(PriceDeviationBreakerTripped)` if the hysteresis breaker
/// is currently tripped for `pair`.
pub fn guard_tripped(env: &Env, pair: &AssetPair) -> Result<(), OracleError> {
    if is_tripped(env, pair) {
        Err(OracleError::PriceDeviationBreakerTripped)
    } else {
        Ok(())
    }
}

/// Evaluate `new_price` against the hysteresis breaker for `pair`.
///
/// Returns:
/// - `Ok(true)`  — update is blocked (breaker tripped or still in recovery).
///   The caller must not store the new price.
/// - `Ok(false)` — update is accepted.
///
/// When no configuration is set for `pair`, returns `Ok(false)` immediately
/// (no-op). The first update for a feed with no prior price is always
/// accepted.
///
/// # Side effects
/// May set/clear the `Tripped` flag and update the `RecoveryStreak` counter.
/// Emits `hyst_trip` on trip and `hyst_recov` on recovery.
pub fn check_and_trip(env: &Env, pair: &AssetPair, new_price: i128) -> Result<bool, OracleError> {
    let cfg = match get_config(env, pair) {
        Some(c) if c.trigger_bps > 0 => c,
        _ => return Ok(false), // disabled or not configured
    };

    let prev_price = match storage::get_price(env, pair) {
        Ok(p) if p > 0 => p,
        _ => return Ok(is_tripped(env, pair)), // No prior price — first update always accepted
    };

    let deviation_bps = ((new_price - prev_price).abs() * 10_000) / prev_price;
    let within_recovery = deviation_bps <= cfg.recovery_bps as i128;
    let within_trigger = deviation_bps <= cfg.trigger_bps as i128;

    if is_tripped(env, pair) {
        if !within_recovery {
            // Deviating update resets the streak; breaker stays tripped.
            set_streak(env, pair, 0);
            return Ok(true);
        }
        let streak = recovery_streak(env, pair) + 1;
        if streak < cfg.healthy_rounds {
            set_streak(env, pair, streak);
            return Ok(true);
        }
        // Streak complete — clear the breaker; this observation is accepted.
        clear(env, pair);
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("hyst_recov")),
            (pair.clone(), new_price, cfg.recovery_bps, cfg.healthy_rounds),
        );
        return Ok(false);
    }

    if !within_trigger {
        trip(env, pair, prev_price, new_price, deviation_bps, &cfg);
        return Ok(true);
    }

    Ok(false)
}

/// Admin: reset the hysteresis breaker unconditionally.
///
/// Emits `hyst_reset` for audit.
pub fn reset(env: &Env, pair: &AssetPair) {
    clear(env, pair);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("hyst_reset")),
        pair.clone(),
    );
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn trip(env: &Env, pair: &AssetPair, prev: i128, new: i128, deviation_bps: i128, cfg: &HysteresisConfig) {
    set_streak(env, pair, 0);
    env.storage()
        .instance()
        .set(&HysteresisKey::Tripped(pair.clone()), &true);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("hyst_trip")),
        (pair.clone(), prev, new, deviation_bps, cfg.recovery_bps),
    );
}

fn clear(env: &Env, pair: &AssetPair) {
    env.storage()
        .instance()
        .set(&HysteresisKey::Tripped(pair.clone()), &false);
    set_streak(env, pair, 0);
}

fn set_streak(env: &Env, pair: &AssetPair, streak: u32) {
    env.storage()
        .instance()
        .set(&HysteresisKey::RecoveryStreak(pair.clone()), &streak);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env, String};
    use stellar_swipe_common::{Asset, AssetPair};

    fn pair(env: &Env) -> AssetPair {
        AssetPair {
            base: Asset {
                code: String::from_str(env, "XLM"),
                issuer: None,
            },
            quote: Asset {
                code: String::from_str(env, "USDC"),
                issuer: None,
            },
        }
    }

    fn cfg(trigger: u32, recovery: u32, rounds: u32) -> HysteresisConfig {
        HysteresisConfig {
            trigger_bps: trigger,
            recovery_bps: recovery,
            healthy_rounds: rounds,
        }
    }

    // Helper: seed an accepted price directly into storage without going through
    // the full oracle pipeline (avoids uninitialized contract dependencies).
    fn seed_price(env: &Env, pair: &AssetPair, price: i128) {
        use crate::storage::PriceFeed;
        use crate::storage::StorageKey;
        env.storage()
            .persistent()
            .set(&StorageKey::Feed(pair.clone()), &PriceFeed { price, timestamp: env.ledger().timestamp() });
    }

    // ── Config validation ─────────────────────────────────────────────────────

    #[test]
    fn valid_config_accepted() {
        assert!(validate_config(&cfg(500, 200, 3)).is_ok());
        assert!(validate_config(&cfg(0, 0, 0)).is_ok()); // disabled
    }

    #[test]
    fn recovery_gte_trigger_rejected() {
        assert!(validate_config(&cfg(500, 500, 3)).is_err());
        assert!(validate_config(&cfg(500, 600, 3)).is_err());
    }

    #[test]
    fn zero_healthy_rounds_rejected_when_thresholds_nonzero() {
        assert!(validate_config(&cfg(500, 200, 0)).is_err());
    }

    #[test]
    fn trigger_zero_with_nonzero_recovery_rejected() {
        assert!(validate_config(&cfg(0, 200, 3)).is_err());
    }

    // ── Trigger / recovery oscillation ───────────────────────────────────────

    #[test]
    fn update_within_trigger_does_not_trip() {
        let env = Env::default();
        let p = pair(&env);
        set_config(&env, &p, cfg(500, 200, 3)).unwrap(); // 5% trigger, 2% recovery
        seed_price(&env, &p, 1_000_000);

        // 3% move — within trigger.
        let blocked = check_and_trip(&env, &p, 1_030_000).unwrap();
        assert!(!blocked);
        assert!(!is_tripped(&env, &p));
    }

    #[test]
    fn update_exceeding_trigger_trips_breaker() {
        let env = Env::default();
        let p = pair(&env);
        set_config(&env, &p, cfg(500, 200, 3)).unwrap();
        seed_price(&env, &p, 1_000_000);

        // 10% move — exceeds 5% trigger.
        let blocked = check_and_trip(&env, &p, 1_100_000).unwrap();
        assert!(blocked);
        assert!(is_tripped(&env, &p));
    }

    #[test]
    fn recovery_requires_sustained_healthy_rounds() {
        let env = Env::default();
        let p = pair(&env);
        set_config(&env, &p, cfg(500, 200, 3)).unwrap(); // 3 healthy rounds
        seed_price(&env, &p, 1_000_000);

        // Trip it.
        check_and_trip(&env, &p, 1_100_000).unwrap();
        assert!(is_tripped(&env, &p));

        // Within recovery (< 2%), but not enough rounds yet.
        for _ in 0..2 {
            let blocked = check_and_trip(&env, &p, 1_010_000).unwrap(); // 1%
            assert!(blocked, "still blocked before 3 rounds");
            assert!(is_tripped(&env, &p));
        }

        // Third consecutive within-recovery-threshold update: clears the breaker.
        let blocked = check_and_trip(&env, &p, 1_010_000).unwrap();
        assert!(!blocked, "third healthy round must clear the breaker");
        assert!(!is_tripped(&env, &p));
    }

    #[test]
    fn deviating_update_during_recovery_resets_streak() {
        let env = Env::default();
        let p = pair(&env);
        set_config(&env, &p, cfg(500, 200, 3)).unwrap();
        seed_price(&env, &p, 1_000_000);

        // Trip.
        check_and_trip(&env, &p, 1_100_000).unwrap();

        // Two healthy rounds.
        check_and_trip(&env, &p, 1_010_000).unwrap();
        check_and_trip(&env, &p, 1_010_000).unwrap();
        assert_eq!(recovery_streak(&env, &p), 2);

        // A deviating update (outside recovery bound but we are already tripped).
        check_and_trip(&env, &p, 1_040_000).unwrap(); // 4% — resets streak
        assert_eq!(recovery_streak(&env, &p), 0);
        assert!(is_tripped(&env, &p));
    }

    #[test]
    fn oscillation_around_trigger_does_not_prematurely_recover() {
        // Demonstrate that the separate recovery threshold prevents rapid toggling.
        let env = Env::default();
        let p = pair(&env);
        // Trigger at 5%, recovery at 2%, 3 rounds needed.
        set_config(&env, &p, cfg(500, 200, 3)).unwrap();
        seed_price(&env, &p, 1_000_000);

        // Trip at 6%.
        check_and_trip(&env, &p, 1_060_000).unwrap();
        assert!(is_tripped(&env, &p));

        // Now oscillate between 4% (within trigger, outside recovery):
        // each such observation resets the streak — breaker never clears.
        for _ in 0..10 {
            let blocked = check_and_trip(&env, &p, 1_040_000).unwrap(); // 4% from baseline
            assert!(blocked, "oscillation at 4% must not clear a 2%-recovery breaker");
        }
        assert!(is_tripped(&env, &p));
    }

    // ── Admin reset ───────────────────────────────────────────────────────────

    #[test]
    fn admin_reset_clears_breaker_immediately() {
        let env = Env::default();
        let p = pair(&env);
        set_config(&env, &p, cfg(500, 200, 3)).unwrap();
        seed_price(&env, &p, 1_000_000);

        check_and_trip(&env, &p, 1_100_000).unwrap();
        assert!(is_tripped(&env, &p));

        reset(&env, &p);
        assert!(!is_tripped(&env, &p));
        assert_eq!(recovery_streak(&env, &p), 0);
    }

    // ── Disabled (both thresholds zero) ───────────────────────────────────────

    #[test]
    fn disabled_config_allows_any_update() {
        let env = Env::default();
        let p = pair(&env);
        set_config(&env, &p, cfg(0, 0, 0)).unwrap();
        seed_price(&env, &p, 1_000_000);

        let blocked = check_and_trip(&env, &p, 10_000_000).unwrap(); // 900% move
        assert!(!blocked);
    }

    // ── No config present acts as disabled ────────────────────────────────────

    #[test]
    fn no_config_allows_any_update() {
        let env = Env::default();
        let p = pair(&env);
        // No set_config call.
        seed_price(&env, &p, 1_000_000);

        let blocked = check_and_trip(&env, &p, 50_000_000).unwrap();
        assert!(!blocked);
    }
}
