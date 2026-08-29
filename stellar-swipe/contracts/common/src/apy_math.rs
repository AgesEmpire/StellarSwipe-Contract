//! Safe arithmetic guardrails for APY and reward-rate calculations (issue #1024).
//!
//! APY and reward math multiplies a principal by a rate and by a time fraction,
//! then — for compounding — repeats that step once per period.  Written naively
//! on raw `i128` this can overflow or silently drift.  This module centralises
//! the bounds checks and overflow-checked arithmetic so that callers never
//! apply an unvalidated rate to a deposit or claim amount:
//!
//! * [`validate_apy_bps`] rejects out-of-range rates *before* any arithmetic.
//! * [`accrue_simple`] and [`accrue_compound`] perform every multiplication and
//!   division through the checked [`Amount`] helpers, returning
//!   [`ApyMathError`] instead of panicking or wrapping.
//! * [`accrue_compound`] additionally caps its "repeated multiplication" loop at
//!   [`MAX_COMPOUND_PERIODS`] so an oversized period count fails cleanly.

use crate::checked_amount::{Amount, AmountError};
use soroban_sdk::contracttype;

/// Basis-points denominator: `10_000` bps == 100%.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Seconds in a 365-day year, used as the APY accrual period.
pub const SECONDS_PER_YEAR: u64 = 365 * 24 * 60 * 60;

/// Hard upper bound on an accepted APY rate: 1_000% (`100_000` bps).
///
/// A rate above this is treated as misconfiguration and rejected rather than
/// applied to real balances.
pub const MAX_APY_BPS: u32 = 100_000;

/// Maximum number of compounding periods a single [`accrue_compound`] call may
/// iterate over.  Bounds the repeated-multiplication loop so it cannot be
/// driven toward overflow or excessive cost by an oversized period count.
pub const MAX_COMPOUND_PERIODS: u32 = 1_200;

/// Failure modes for the APY / reward calculations in this module.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApyMathError {
    /// The supplied rate exceeds [`MAX_APY_BPS`].
    RateOutOfBounds,
    /// An intermediate or final value overflowed `i128`.
    Overflow,
    /// A division with a zero denominator was attempted.
    DivisionByZero,
    /// A principal or time-period argument was outside the valid range.
    InvalidInput,
}

impl From<AmountError> for ApyMathError {
    fn from(e: AmountError) -> Self {
        match e {
            AmountError::Overflow => ApyMathError::Overflow,
            AmountError::DivisionByZero => ApyMathError::DivisionByZero,
        }
    }
}

/// Validate an APY rate expressed in basis points.
///
/// Returns the rate unchanged when `rate_bps <= MAX_APY_BPS`, otherwise
/// [`ApyMathError::RateOutOfBounds`].  Call this before the rate reaches any
/// deposit or claim calculation.
pub fn validate_apy_bps(rate_bps: u32) -> Result<u32, ApyMathError> {
    if rate_bps > MAX_APY_BPS {
        return Err(ApyMathError::RateOutOfBounds);
    }
    Ok(rate_bps)
}

/// Simple (non-compounding) reward accrual:
/// `principal * rate_bps / 10_000 * elapsed_secs / SECONDS_PER_YEAR`.
///
/// * `principal` must be `>= 0`.
/// * `rate_bps` is validated against [`MAX_APY_BPS`].
/// * Every multiplication and division is overflow-checked; division rounds
///   toward zero.
pub fn accrue_simple(
    principal: i128,
    rate_bps: u32,
    elapsed_secs: u64,
) -> Result<i128, ApyMathError> {
    if principal < 0 {
        return Err(ApyMathError::InvalidInput);
    }
    validate_apy_bps(rate_bps)?;

    // annual_reward = principal * rate_bps / BPS_DENOMINATOR
    let annual = Amount::new(principal).checked_mul_rate(rate_bps as i128, BPS_DENOMINATOR)?;
    // accrued = annual_reward * elapsed_secs / SECONDS_PER_YEAR
    let accrued = annual.checked_mul_rate(elapsed_secs as i128, SECONDS_PER_YEAR as i128)?;
    Ok(accrued.value())
}

/// Compounding reward accrual over `periods` equal periods of `period_secs`
/// each, re-investing the growth after every period.
///
/// Per-period growth is
/// `balance * rate_bps / 10_000 * period_secs / SECONDS_PER_YEAR`, computed at
/// full `i128` precision each iteration (no pre-floored per-period rate) to
/// keep drift minimal.  The loop is capped at [`MAX_COMPOUND_PERIODS`] and
/// every step is overflow-checked, so a large `periods` value fails cleanly
/// instead of wrapping.
///
/// Returns the *reward* portion only: the final balance minus the original
/// `principal`.
pub fn accrue_compound(
    principal: i128,
    rate_bps: u32,
    periods: u32,
    period_secs: u64,
) -> Result<i128, ApyMathError> {
    if principal < 0 || period_secs == 0 || periods > MAX_COMPOUND_PERIODS {
        return Err(ApyMathError::InvalidInput);
    }
    validate_apy_bps(rate_bps)?;

    let start = Amount::new(principal);
    let mut balance = start;
    for _ in 0..periods {
        let annual = balance.checked_mul_rate(rate_bps as i128, BPS_DENOMINATOR)?;
        let growth = annual.checked_mul_rate(period_secs as i128, SECONDS_PER_YEAR as i128)?;
        balance = balance.checked_add(growth)?;
    }
    Ok(balance.checked_sub(start)?.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_apy_bps ──────────────────────────────────────────────────

    #[test]
    fn validate_accepts_zero_and_max() {
        assert_eq!(validate_apy_bps(0), Ok(0));
        assert_eq!(validate_apy_bps(MAX_APY_BPS), Ok(MAX_APY_BPS));
    }

    #[test]
    fn validate_rejects_above_max() {
        assert_eq!(
            validate_apy_bps(MAX_APY_BPS + 1),
            Err(ApyMathError::RateOutOfBounds)
        );
        assert_eq!(
            validate_apy_bps(u32::MAX),
            Err(ApyMathError::RateOutOfBounds)
        );
    }

    // ── accrue_simple ─────────────────────────────────────────────────────

    #[test]
    fn simple_zero_principal_or_rate_or_time_yields_zero() {
        assert_eq!(accrue_simple(0, 500, SECONDS_PER_YEAR), Ok(0));
        assert_eq!(accrue_simple(10_000_000, 0, SECONDS_PER_YEAR), Ok(0));
        assert_eq!(accrue_simple(10_000_000, 500, 0), Ok(0));
    }

    #[test]
    fn simple_full_year_at_five_percent() {
        // 1_000 units (7dp) at 5% for one year == 50 units.
        assert_eq!(
            accrue_simple(10_000_000, 500, SECONDS_PER_YEAR),
            Ok(500_000)
        );
    }

    #[test]
    fn simple_half_year_is_half_the_reward() {
        assert_eq!(
            accrue_simple(10_000_000, 500, SECONDS_PER_YEAR / 2),
            Ok(250_000)
        );
    }

    #[test]
    fn simple_rejects_negative_principal() {
        assert_eq!(accrue_simple(-1, 500, 1), Err(ApyMathError::InvalidInput));
    }

    #[test]
    fn simple_rejects_out_of_bounds_rate() {
        assert_eq!(
            accrue_simple(10_000_000, MAX_APY_BPS + 1, SECONDS_PER_YEAR),
            Err(ApyMathError::RateOutOfBounds)
        );
    }

    #[test]
    fn simple_near_limit_principal_overflows_cleanly() {
        assert_eq!(
            accrue_simple(i128::MAX, MAX_APY_BPS, SECONDS_PER_YEAR),
            Err(ApyMathError::Overflow)
        );
    }

    #[test]
    fn simple_large_but_representable_principal_is_ok() {
        // ~10 billion units at 7 decimals: large, yet every intermediate
        // product stays well inside i128 even at the maximum rate.
        let principal = 100_000_000_000_000_000i128;
        let out = accrue_simple(principal, MAX_APY_BPS, SECONDS_PER_YEAR);
        assert_eq!(out, Ok(principal * MAX_APY_BPS as i128 / BPS_DENOMINATOR));
    }

    // ── accrue_compound ───────────────────────────────────────────────────

    #[test]
    fn compound_zero_rate_yields_zero_reward() {
        assert_eq!(
            accrue_compound(10_000_000, 0, 12, SECONDS_PER_YEAR / 12),
            Ok(0)
        );
    }

    #[test]
    fn compound_beats_simple_over_same_horizon() {
        let principal = 1_000_000_000i128;
        let simple = accrue_simple(principal, 1_000, SECONDS_PER_YEAR).unwrap();
        let compounded = accrue_compound(principal, 1_000, 12, SECONDS_PER_YEAR / 12).unwrap();
        assert!(compounded > simple);
    }

    #[test]
    fn compound_rejects_zero_period_length() {
        assert_eq!(
            accrue_compound(10_000_000, 500, 12, 0),
            Err(ApyMathError::InvalidInput)
        );
    }

    #[test]
    fn compound_rejects_period_count_over_cap() {
        assert_eq!(
            accrue_compound(10_000_000, 500, MAX_COMPOUND_PERIODS + 1, 3_600),
            Err(ApyMathError::InvalidInput)
        );
    }

    #[test]
    fn compound_at_cap_is_accepted() {
        let out = accrue_compound(10_000_000, 500, MAX_COMPOUND_PERIODS, 3_600);
        assert!(out.is_ok());
    }

    #[test]
    fn compound_rejects_out_of_bounds_rate() {
        assert_eq!(
            accrue_compound(10_000_000, MAX_APY_BPS + 1, 1, SECONDS_PER_YEAR),
            Err(ApyMathError::RateOutOfBounds)
        );
    }

    #[test]
    fn compound_near_limit_principal_overflows_cleanly() {
        assert_eq!(
            accrue_compound(i128::MAX, MAX_APY_BPS, 2, SECONDS_PER_YEAR),
            Err(ApyMathError::Overflow)
        );
    }
}
