//! Safe arithmetic helpers for deterministic rounding and overflow safeguards (Issue #861).
//!
//! Provides checked arithmetic operations with explicit rounding rules so fee
//! splitting, reward distribution, and protocol parameter math are deterministic
//! and resistant to overflow or precision issues.
//!
//! # Rounding Modes
//!
//! - `Floor` (truncation toward zero) — user-favorable; the user is never
//!   charged more than their exact pro-rata share.
//! - `Ceil` (round up) — protocol-favorable; used when the protocol must
//!   collect at minimum its expected share.
//! - `Round` (round to nearest, ties away from zero) — balanced; used when
//!   neither side should consistently absorb rounding error.

use soroban_sdk::contracttype;

/// Rounding mode for division operations.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RoundingMode {
    /// Truncate toward zero (user-favorable).
    Floor = 0,
    /// Round up (protocol-favorable).
    Ceil = 1,
    /// Round to nearest, ties away from zero.
    Round = 2,
}

/// Compute `numerator / denominator` with the specified rounding mode.
/// Returns `None` on division by zero.
pub fn checked_div(numerator: i128, denominator: i128, mode: RoundingMode) -> Option<i128> {
    if denominator == 0 {
        return None;
    }
    let result = numerator.checked_div(denominator)?;
    let remainder = numerator.checked_rem(denominator)?;
    match mode {
        RoundingMode::Floor => Some(result),
        RoundingMode::Ceil => {
            if remainder != 0 && ((numerator > 0) == (denominator > 0)) {
                result.checked_add(1)
            } else {
                Some(result)
            }
        }
        RoundingMode::Round => {
            let abs_remainder = remainder.abs();
            let abs_denominator = denominator.abs();
            let halfway = abs_denominator / 2;
            if abs_remainder > halfway
                || (abs_remainder == halfway && (abs_remainder * 2) == abs_denominator)
            {
                if (numerator > 0) == (denominator > 0) {
                    result.checked_add(1)
                } else {
                    result.checked_sub(1)
                }
            } else {
                Some(result)
            }
        }
    }
}

/// Compute `value * bps / 10_000` using floor rounding (truncation toward zero).
/// Returns `None` on overflow or if bps > 10_000.
pub fn checked_bps_floor(value: i128, bps: u32) -> Option<i128> {
    if bps > 10_000 {
        return None;
    }
    let numerator = value.checked_mul(bps as i128)?;
    checked_div(numerator, 10_000, RoundingMode::Floor)
}

/// Compute `value * bps / 10_000` using ceiling rounding (round up).
/// Returns `None` on overflow or if bps > 10_000.
pub fn checked_bps_ceil(value: i128, bps: u32) -> Option<i128> {
    if bps > 10_000 {
        return None;
    }
    let numerator = value.checked_mul(bps as i128)?;
    checked_div(numerator, 10_000, RoundingMode::Ceil)
}

/// Compute `value * percentage / 100` using floor rounding.
/// Returns `None` on overflow or if percentage > 100.
pub fn checked_pct_floor(value: i128, pct: u32) -> Option<i128> {
    if pct > 100 {
        return None;
    }
    let numerator = value.checked_mul(pct as i128)?;
    checked_div(numerator, 100, RoundingMode::Floor)
}

/// Compute `value * percentage / 100` using ceiling rounding.
/// Returns `None` on overflow or if percentage > 100.
pub fn checked_pct_ceil(value: i128, pct: u32) -> Option<i128> {
    if pct > 100 {
        return None;
    }
    let numerator = value.checked_mul(pct as i128)?;
    checked_div(numerator, 100, RoundingMode::Ceil)
}

/// Compute `a * b / c` with floor rounding, checking all intermediate steps.
/// Returns `None` on overflow or division by zero.
pub fn checked_mul_div_floor(a: i128, b: i128, c: i128) -> Option<i128> {
    let product = a.checked_mul(b)?;
    checked_div(product, c, RoundingMode::Floor)
}

/// Compute `a * b / c` with ceiling rounding, safe against overflow.
pub fn checked_mul_div_ceil(a: i128, b: i128, c: i128) -> Option<i128> {
    let product = a.checked_mul(b)?;
    checked_div(product, c, RoundingMode::Ceil)
}

/// Safely compute `value + (value * bps / 10_000)` with floor rounding.
/// Returns `None` on overflow or invalid bps.
pub fn checked_add_bps_floor(value: i128, bps: u32) -> Option<i128> {
    let fee = checked_bps_floor(value, bps)?;
    value.checked_add(fee)
}

/// Safely compute `value - (value * bps / 10_000)` with floor rounding.
/// Returns `None` on overflow, underflow, or invalid bps.
pub fn checked_sub_bps_floor(value: i128, bps: u32) -> Option<i128> {
    let fee = checked_bps_floor(value, bps)?;
    value.checked_sub(fee)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_floor_truncates() {
        assert_eq!(checked_div(100, 3, RoundingMode::Floor), Some(33));
        assert_eq!(checked_div(-100, 3, RoundingMode::Floor), Some(-33));
    }

    #[test]
    fn div_ceil_rounds_up() {
        assert_eq!(checked_div(100, 3, RoundingMode::Ceil), Some(34));
        assert_eq!(checked_div(100, 5, RoundingMode::Ceil), Some(20));
    }

    #[test]
    fn div_round_rounds_nearest() {
        assert_eq!(checked_div(100, 3, RoundingMode::Round), Some(33));
        assert_eq!(checked_div(101, 3, RoundingMode::Round), Some(34));
        assert_eq!(checked_div(100, 2, RoundingMode::Round), Some(50));
    }

    #[test]
    fn div_by_zero_returns_none() {
        assert_eq!(checked_div(100, 0, RoundingMode::Floor), None);
    }

    #[test]
    fn bps_floor_basic() {
        assert_eq!(checked_bps_floor(1_000_000, 500), Some(50_000));
        assert_eq!(checked_bps_floor(1_000_000, 10_000), Some(1_000_000));
    }

    #[test]
    fn bps_floor_truncates() {
        assert_eq!(checked_bps_floor(100, 333), Some(3));
    }

    #[test]
    fn bps_floor_overflow_returns_none() {
        assert_eq!(checked_bps_floor(i128::MAX, 10_001), None);
    }

    #[test]
    fn bps_ceil_basic() {
        assert_eq!(checked_bps_ceil(1_000_000, 500), Some(50_000));
        assert_eq!(checked_bps_ceil(100, 333), Some(4));
    }

    #[test]
    fn pct_floor_basic() {
        assert_eq!(checked_pct_floor(1_000, 50), Some(500));
        assert_eq!(checked_pct_floor(1_000, 100), Some(1_000));
    }

    #[test]
    fn pct_floor_overflow_returns_none() {
        assert_eq!(checked_pct_floor(1_000, 101), None);
    }

    #[test]
    fn mul_div_floor() {
        assert_eq!(checked_mul_div_floor(100, 30, 100), Some(30));
        assert_eq!(checked_mul_div_floor(10, 33, 100), Some(3));
    }

    #[test]
    fn mul_div_ceil() {
        assert_eq!(checked_mul_div_ceil(100, 30, 100), Some(30));
        assert_eq!(checked_mul_div_ceil(10, 33, 100), Some(4));
    }

    #[test]
    fn add_bps_floor() {
        assert_eq!(checked_add_bps_floor(10_000, 500), Some(10_500));
        assert_eq!(checked_add_bps_floor(0, 500), Some(0));
    }

    #[test]
    fn sub_bps_floor() {
        assert_eq!(checked_sub_bps_floor(10_000, 500), Some(9_500));
        assert_eq!(checked_sub_bps_floor(0, 500), Some(0));
    }

    #[test]
    fn extreme_values_no_overflow() {
        assert_eq!(
            checked_bps_floor(i128::MAX / 2, 1),
            Some(i128::MAX / 2 / 10_000)
        );
        assert_eq!(
            checked_div(i128::MAX, 1, RoundingMode::Floor),
            Some(i128::MAX)
        );
    }

    #[test]
    fn zero_values() {
        assert_eq!(checked_bps_floor(0, 500), Some(0));
        assert_eq!(checked_div(0, 100, RoundingMode::Floor), Some(0));
        assert_eq!(checked_mul_div_floor(0, 100, 100), Some(0));
    }

    #[test]
    fn negative_values() {
        assert_eq!(checked_bps_floor(-1000, 500), Some(-50));
        assert_eq!(checked_bps_ceil(-1000, 500), Some(-50));
        assert_eq!(checked_div(-100, 3, RoundingMode::Ceil), Some(-33));
        assert_eq!(checked_div(-100, 3, RoundingMode::Floor), Some(-33));
    }
}
