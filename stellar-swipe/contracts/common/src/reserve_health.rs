//! Reserve health monitoring event stream (issue #1034).
//!
//! Balance-affecting pool operations call [`publish_reserve_health`], which
//! computes a health ratio in basis points, maps it to a deterministic
//! [`ReserveHealthBand`], and emits a structured event. Off-chain consumers get
//! the ratio, the band and both raw legs, so they never need to read contract
//! state to interpret the event.
//!
//! Thresholds (health ratio = reserves * 10_000 / liabilities, in bps):
//!
//! | Band       | Ratio (bps)        |
//! |------------|--------------------|
//! | Critical   | < 10_000           |
//! | Warning    | 10_000 ..< 12_000  |
//! | Healthy    | 12_000 ..< 15_000  |
//! | Overfunded | >= 15_000          |

use soroban_sdk::{contracttype, symbol_short, Env};

/// Ratio at or above which reserves fully cover liabilities.
pub const CRITICAL_THRESHOLD_BPS: i128 = 10_000;
/// Ratio at or above which the pool leaves the warning band.
pub const WARNING_THRESHOLD_BPS: i128 = 12_000;
/// Ratio at or above which the pool is considered overfunded.
pub const OVERFUNDED_THRESHOLD_BPS: i128 = 15_000;

/// Deterministic health band derived from the reserve ratio.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReserveHealthBand {
    /// Reserves do not cover liabilities.
    Critical = 0,
    /// Coverage is thin and trending toward critical.
    Warning = 1,
    /// Coverage is within the target range.
    Healthy = 2,
    /// Coverage exceeds the target range.
    Overfunded = 3,
}

/// Structured payload published on every reserve health update.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveHealthEvent {
    /// Total reserves backing the pool.
    pub reserves: i128,
    /// Total outstanding liabilities.
    pub liabilities: i128,
    /// Health ratio in basis points.
    pub ratio_bps: i128,
    /// Band the ratio falls into.
    pub band: ReserveHealthBand,
    /// Band reported by the previous update. Only meaningful when
    /// `has_previous` is true.
    pub previous_band: ReserveHealthBand,
    /// False on the first update for a pool, when there is no prior band.
    pub has_previous: bool,
    /// True when this update crossed a band boundary.
    pub crossed: bool,
    /// Ledger timestamp of the update.
    pub timestamp: u64,
}

/// Health ratio in basis points. Zero liabilities means fully covered, which is
/// reported as [`OVERFUNDED_THRESHOLD_BPS`] rather than a division by zero.
pub fn health_ratio_bps(reserves: i128, liabilities: i128) -> i128 {
    if liabilities <= 0 {
        return OVERFUNDED_THRESHOLD_BPS;
    }
    reserves.saturating_mul(10_000) / liabilities
}

/// Maps a ratio to its band. Boundaries are inclusive on the lower edge.
pub fn band_for_ratio(ratio_bps: i128) -> ReserveHealthBand {
    if ratio_bps < CRITICAL_THRESHOLD_BPS {
        ReserveHealthBand::Critical
    } else if ratio_bps < WARNING_THRESHOLD_BPS {
        ReserveHealthBand::Warning
    } else if ratio_bps < OVERFUNDED_THRESHOLD_BPS {
        ReserveHealthBand::Healthy
    } else {
        ReserveHealthBand::Overfunded
    }
}

/// Builds and emits the reserve health event under topic `("reserve", "health")`.
///
/// Returns the emitted payload so callers can persist the band for the next
/// call's `previous_band`.
pub fn publish_reserve_health(
    env: &Env,
    reserves: i128,
    liabilities: i128,
    previous_band: Option<ReserveHealthBand>,
) -> ReserveHealthEvent {
    let ratio_bps = health_ratio_bps(reserves, liabilities);
    let band = band_for_ratio(ratio_bps);
    let crossed = match previous_band {
        Some(prev) => prev != band,
        None => true,
    };

    let event = ReserveHealthEvent {
        reserves,
        liabilities,
        ratio_bps,
        band,
        previous_band: previous_band.unwrap_or(band),
        has_previous: previous_band.is_some(),
        crossed,
        timestamp: env.ledger().timestamp(),
    };

    env.events()
        .publish((symbol_short!("reserve"), symbol_short!("health")), event.clone());

    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn ratio_is_deterministic() {
        assert_eq!(health_ratio_bps(100, 100), 10_000);
        assert_eq!(health_ratio_bps(150, 100), 15_000);
        assert_eq!(health_ratio_bps(50, 100), 5_000);
        // No liabilities is treated as fully covered, not a divide by zero.
        assert_eq!(health_ratio_bps(0, 0), OVERFUNDED_THRESHOLD_BPS);
    }

    #[test]
    fn band_boundaries_are_inclusive_on_the_lower_edge() {
        assert_eq!(band_for_ratio(9_999), ReserveHealthBand::Critical);
        assert_eq!(band_for_ratio(10_000), ReserveHealthBand::Warning);
        assert_eq!(band_for_ratio(11_999), ReserveHealthBand::Warning);
        assert_eq!(band_for_ratio(12_000), ReserveHealthBand::Healthy);
        assert_eq!(band_for_ratio(14_999), ReserveHealthBand::Healthy);
        assert_eq!(band_for_ratio(15_000), ReserveHealthBand::Overfunded);
    }

    #[test]
    fn first_publish_is_always_a_crossing() {
        let env = Env::default();
        let ev = publish_reserve_health(&env, 130, 100, None);
        assert!(!ev.has_previous);
        assert_eq!(ev.band, ReserveHealthBand::Healthy);
        assert!(ev.crossed);
        assert_eq!(ev.ratio_bps, 13_000);
    }

    #[test]
    fn crossing_flag_tracks_band_changes() {
        let env = Env::default();
        let first = publish_reserve_health(&env, 130, 100, None);
        // Same band -> not a crossing.
        let same = publish_reserve_health(&env, 140, 100, Some(first.band));
        assert!(!same.crossed);
        // Drop into warning -> crossing.
        let dropped = publish_reserve_health(&env, 110, 100, Some(same.band));
        assert_eq!(dropped.band, ReserveHealthBand::Warning);
        assert!(dropped.crossed);
    }

    #[test]
    fn payload_is_self_describing_for_off_chain_consumers() {
        let env = Env::default();
        let ev = publish_reserve_health(&env, 90, 100, None);
        // Everything a consumer needs is on the payload; no state read required.
        assert_eq!(ev.reserves, 90);
        assert_eq!(ev.liabilities, 100);
        assert_eq!(ev.ratio_bps, 9_000);
        assert_eq!(ev.band, ReserveHealthBand::Critical);
        assert_eq!(ev.timestamp, env.ledger().timestamp());
    }
}
