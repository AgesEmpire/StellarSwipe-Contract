//! Portfolio risk exposure and concentration metrics (Issue #879).
//!
//! Pure, allocation-free helpers that turn a user's per-asset exposures into
//! risk metrics: total exposure, the largest single-asset share, and a
//! Herfindahl-Hirschman concentration index. The metrics are derived from the
//! exposures already tracked by [`crate::exposure_cap`], so they stay correct
//! after any state transition that updates exposure — there is no separate
//! cached copy to fall out of sync.
//!
//! Shares and the index are expressed in basis points (10_000 bps = 100%).
//! Enforcement of a per-asset cap remains in `exposure_cap`; the concentration
//! limit here is a portfolio-level check layered on top of it.

use soroban_sdk::{contracttype, Env, Vec};

/// Basis-point denominator (100%).
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Risk metrics derived from a user's per-asset exposures.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskMetrics {
    /// Sum of all per-asset exposures.
    pub total_exposure: i128,
    /// Exposure of the single largest position.
    pub largest_exposure: i128,
    /// Largest position as a share of the total, in basis points.
    pub largest_share_bps: u32,
    /// Herfindahl-Hirschman index in basis points: 10_000 for a single asset,
    /// 10_000 / n for n equally sized positions. Higher means more concentrated.
    pub concentration_bps: u32,
    /// Number of non-zero positions.
    pub position_count: u32,
}

impl RiskMetrics {
    /// An empty portfolio: no exposure, no concentration.
    pub fn empty() -> Self {
        RiskMetrics {
            total_exposure: 0,
            largest_exposure: 0,
            largest_share_bps: 0,
            concentration_bps: 0,
            position_count: 0,
        }
    }
}

/// `part / total` expressed in basis points.
///
/// Both operands are halved until the numerator fits, so a portfolio near
/// `i128::MAX` still yields a meaningful share instead of a saturated one.
fn share_bps(mut part: i128, mut total: i128) -> i128 {
    const LIMIT: i128 = i128::MAX / BPS_DENOMINATOR;
    while part > LIMIT {
        part /= 2;
        total /= 2;
    }
    if total <= 0 {
        return 0;
    }
    part * BPS_DENOMINATOR / total
}

/// Computes risk metrics from a user's per-asset exposures.
///
/// Negative and zero exposures are ignored — only non-zero long exposure
/// contributes to concentration. An empty (or all-zero) portfolio yields
/// [`RiskMetrics::empty`], never a division by zero.
pub fn compute(_env: &Env, exposures: &Vec<i128>) -> RiskMetrics {
    let mut total: i128 = 0;
    let mut largest: i128 = 0;
    let mut count: u32 = 0;

    for exposure in exposures.iter() {
        if exposure <= 0 {
            continue;
        }
        total = total.saturating_add(exposure);
        if exposure > largest {
            largest = exposure;
        }
        count += 1;
    }

    if total == 0 {
        return RiskMetrics::empty();
    }

    let mut hhi: i128 = 0;
    for exposure in exposures.iter() {
        if exposure <= 0 {
            continue;
        }
        let share = share_bps(exposure, total);
        hhi = hhi.saturating_add(share * share / BPS_DENOMINATOR);
    }

    RiskMetrics {
        total_exposure: total,
        largest_exposure: largest,
        largest_share_bps: share_bps(largest, total) as u32,
        concentration_bps: hhi.min(BPS_DENOMINATOR) as u32,
        position_count: count,
    }
}

/// Returns `true` when no single position exceeds `max_share_bps` of the
/// portfolio. An empty portfolio is always within limit.
pub fn is_within_concentration_limit(metrics: &RiskMetrics, max_share_bps: u32) -> bool {
    metrics.largest_share_bps <= max_share_bps
}

/// Largest exposure that may be added to `asset_exposure` without pushing it
/// past `max_share_bps` of the resulting portfolio. Returns 0 when the position
/// is already at or over the limit.
pub fn headroom(total_exposure: i128, asset_exposure: i128, max_share_bps: u32) -> i128 {
    let max_bps = max_share_bps as i128;
    if max_bps == 0 || max_bps >= BPS_DENOMINATOR {
        return if max_bps == 0 { 0 } else { i128::MAX };
    }
    // asset + x <= max_bps/10_000 * (total + x)  =>  x <= (max_bps*total - 10_000*asset) / (10_000 - max_bps)
    let numerator = max_bps.saturating_mul(total_exposure) - BPS_DENOMINATOR * asset_exposure;
    if numerator <= 0 {
        return 0;
    }
    numerator / (BPS_DENOMINATOR - max_bps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{vec, Env};

    #[test]
    fn empty_portfolio_has_no_risk() {
        let env = Env::default();
        let metrics = compute(&env, &vec![&env]);
        assert_eq!(metrics, RiskMetrics::empty());
        assert!(is_within_concentration_limit(&metrics, 0));
    }

    #[test]
    fn zero_exposures_are_ignored() {
        let env = Env::default();
        let metrics = compute(&env, &vec![&env, 0i128, 0i128]);
        assert_eq!(metrics.total_exposure, 0);
        assert_eq!(metrics.position_count, 0);
    }

    #[test]
    fn single_position_is_fully_concentrated() {
        let env = Env::default();
        let metrics = compute(&env, &vec![&env, 1_000i128]);
        assert_eq!(metrics.total_exposure, 1_000);
        assert_eq!(metrics.largest_share_bps, 10_000);
        assert_eq!(metrics.concentration_bps, 10_000);
        assert_eq!(metrics.position_count, 1);
    }

    #[test]
    fn equal_positions_split_concentration() {
        let env = Env::default();
        let metrics = compute(&env, &vec![&env, 250i128, 250i128, 250i128, 250i128]);
        assert_eq!(metrics.largest_share_bps, 2_500);
        assert_eq!(metrics.concentration_bps, 2_500); // 10_000 / 4
        assert_eq!(metrics.position_count, 4);
    }

    #[test]
    fn skewed_portfolio_trips_the_limit() {
        let env = Env::default();
        let metrics = compute(&env, &vec![&env, 900i128, 50i128, 50i128]);
        assert_eq!(metrics.largest_share_bps, 9_000);
        assert!(!is_within_concentration_limit(&metrics, 5_000));
        assert!(is_within_concentration_limit(&metrics, 9_000));
    }

    #[test]
    fn negative_exposures_do_not_contribute() {
        let env = Env::default();
        let metrics = compute(&env, &vec![&env, 100i128, -100i128]);
        assert_eq!(metrics.total_exposure, 100);
        assert_eq!(metrics.position_count, 1);
    }

    #[test]
    fn headroom_respects_the_limit() {
        // 50% limit, 100 total, 20 already in the asset:
        // 20 + x <= 0.5 * (100 + x)  =>  x <= 60
        assert_eq!(headroom(100, 20, 5_000), 60);
        // Already over the limit.
        assert_eq!(headroom(100, 80, 5_000), 0);
        // Degenerate limits.
        assert_eq!(headroom(100, 0, 0), 0);
        assert_eq!(headroom(100, 0, 10_000), i128::MAX);
    }

    #[test]
    fn large_exposures_do_not_overflow() {
        let env = Env::default();
        let big = i128::MAX / 4;
        let metrics = compute(&env, &vec![&env, big, big]);
        // Shares are scaled down before dividing at this magnitude, so allow a
        // one-bps rounding difference rather than a saturated garbage value.
        assert!(metrics.largest_share_bps.abs_diff(5_000) <= 1);
        assert!(metrics.concentration_bps <= 10_000);
    }
}
