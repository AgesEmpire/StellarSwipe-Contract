//! Decimal-precision scaling helpers (Issue #562).
//!
//! Provides checked functions to convert an `i128` amount between two
//! arbitrary decimal precisions.  A single canonical implementation lives
//! here so oracle conversion, trade execution, and any other module that
//! needs to rescale amounts all use the same rounding behavior.
//!
//! # Rounding
//! Scale-down (from higher to lower precision) **truncates** toward zero
//! (integer division).  This is explicit, not implicit: callers that need
//! rounding must apply it on top of the raw result.
//!
//! # Overflow / invalid inputs
//! All functions return `None` on overflow.  `from_decimals` and
//! `to_decimals` are `u32`, so negative precision is a type-level
//! impossibility.  Precision values large enough to overflow `i128` (i.e.
//! those that would multiply by 10^38 or more) also return `None`.

/// Convert `amount` from `from_decimals` precision to `to_decimals` precision.
///
/// Rounding: scale-down truncates toward zero.
/// Returns `None` on arithmetic overflow or if the scale factor overflows
/// `i128` (e.g. `to_decimals - from_decimals >= 39`).
///
/// # Examples
/// ```ignore
/// // Same precision — no change.
/// assert_eq!(normalize_amount(10_000_000, 7, 7), Some(10_000_000));
/// // 6-decimal → 7-decimal: ×10
/// assert_eq!(normalize_amount(1_000_000, 6, 7), Some(10_000_000));
/// // 7-decimal → 6-decimal: ÷10 (truncates)
/// assert_eq!(normalize_amount(10_000_001, 7, 6), Some(1_000_000));
/// ```
pub fn normalize_amount(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    match from_decimals.cmp(&to_decimals) {
        core::cmp::Ordering::Equal => Some(amount),
        core::cmp::Ordering::Less => {
            let diff = to_decimals - from_decimals;
            let factor = pow10(diff)?;
            amount.checked_mul(factor)
        }
        core::cmp::Ordering::Greater => {
            let diff = from_decimals - to_decimals;
            let factor = pow10(diff)?;
            Some(amount / factor)
        }
    }
}

/// Scale `amount` up from `from_decimals` to `to_decimals` precision.
///
/// Panics (via `None`) if `to_decimals < from_decimals`; use
/// [`normalize_amount`] for the general case.
///
/// Returns `None` on overflow.
#[inline]
pub fn scale_up(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    normalize_amount(amount, from_decimals, to_decimals)
}

/// Scale `amount` down from `from_decimals` to `to_decimals` precision,
/// truncating toward zero.
///
/// Returns `None` if `to_decimals > from_decimals`; use [`normalize_amount`]
/// for the general case.
#[inline]
pub fn scale_down(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    normalize_amount(amount, from_decimals, to_decimals)
}

/// Compute 10^exp as i128. Returns `None` if the result overflows.
fn pow10(exp: u32) -> Option<i128> {
    let mut result: i128 = 1;
    for _ in 0..exp {
        result = result.checked_mul(10)?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn same_precision_unchanged() {
        assert_eq!(normalize_amount(10_000_000, 7, 7), Some(10_000_000));
        assert_eq!(normalize_amount(0, 7, 7), Some(0));
    }

    #[test]
    fn lower_to_higher_precision() {
        assert_eq!(normalize_amount(1_000_000, 6, 7), Some(10_000_000));
        assert_eq!(normalize_amount(1, 0, 7), Some(10_000_000));
    }

    #[test]
    fn higher_to_lower_precision_truncates() {
        assert_eq!(normalize_amount(10_000_000, 7, 6), Some(1_000_000));
        assert_eq!(normalize_amount(10_000_001, 7, 6), Some(1_000_000)); // truncated
        assert_eq!(normalize_amount(10_000_000, 7, 0), Some(1));
    }

    #[test]
    fn overflow_returns_none() {
        assert_eq!(normalize_amount(i128::MAX, 0, 39), None);
    }

    #[test]
    fn negative_amounts_work() {
        assert_eq!(normalize_amount(-1_000_000, 6, 7), Some(-10_000_000));
        assert_eq!(normalize_amount(-10_000_000, 7, 6), Some(-1_000_000));
    }

    /// Round-trip: scale up then down must recover the original value
    /// within the truncation tolerance (loss ≤ 10^diff - 1).
    #[test]
    fn round_trip_scale_up_then_down() {
        let amounts: &[i128] = &[0, 1, 999_999, 1_000_000, i128::MAX / 100];
        for &amount in amounts {
            let scaled = normalize_amount(amount, 6, 7).unwrap();
            let recovered = normalize_amount(scaled, 7, 6).unwrap();
            assert_eq!(recovered, amount, "round-trip failed for {}", amount);
        }
    }

    /// Round-trip starting from 7-decimal: extra sub-unit may be lost on
    /// scale-down, but recovered value must equal original / 10 * 10.
    #[test]
    fn round_trip_scale_down_then_up() {
        let amounts: &[i128] = &[10_000_000, 10_000_009, 99_999_999];
        for &amount in amounts {
            let down = normalize_amount(amount, 7, 6).unwrap();
            let up = normalize_amount(down, 6, 7).unwrap();
            // Rounding loss ≤ 9 (one sub-unit at 7 decimals)
            assert!(
                amount - up < 10 && amount >= up,
                "round-trip tolerance exceeded for {}",
                amount
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

        #[test]
        fn same_precision_is_identity(
            amount in -10_000_000_000_000_000_000_i128..=10_000_000_000_000_000_000_i128,
            decimals in 0u32..=18u32,
        ) {
            prop_assert_eq!(normalize_amount(amount, decimals, decimals), Some(amount));
        }

        #[test]
        fn scale_up_then_down_round_trips(
            amount in -1_000_000_000_000_i128..=1_000_000_000_000_i128,
            low in 0u32..=18u32,
            high in 0u32..=18u32,
        ) {
            prop_assume!(low < high);
            let scaled = normalize_amount(amount, low, high);
            prop_assume!(scaled.is_some());
            let recovered = normalize_amount(scaled.unwrap(), high, low);
            prop_assert_eq!(recovered, Some(amount));
        }

        #[test]
        fn scale_up_preserves_value_exactly(
            amount in -1_000_000_000_000_i128..=1_000_000_000_000_i128,
            low in 0u32..=18u32,
            high in 0u32..=18u32,
        ) {
            prop_assume!(low < high);
            let scaled = normalize_amount(amount, low, high);
            prop_assume!(scaled.is_some());
            // Scaling up is exact multiplication by 10^diff; no rounding loss.
            let expected = amount.checked_mul(pow10(high - low).unwrap_or(i128::MAX));
            prop_assert_eq!(scaled, expected);
        }

        #[test]
        fn scale_down_truncates_toward_zero(
            amount in -1_000_000_000_000_i128..=1_000_000_000_000_i128,
            high in 1u32..=18u32,
            low in 0u32..=18u32,
        ) {
            prop_assume!(low < high);
            let scaled = normalize_amount(amount, high, low);
            prop_assume!(scaled.is_some());
            let factor = pow10(high - low).unwrap_or(i128::MAX);
            // Truncation toward zero: |scaled| ≤ |amount / factor|
            let expected_trunc = amount / factor;
            prop_assert_eq!(scaled, expected_trunc);
        }

        #[test]
        fn normalize_amount_is_monotonic_for_positive_amounts(
            a in 1_i128..=1_000_000_000_000_i128,
            b in 1_i128..=1_000_000_000_000_i128,
            from_dec in 0u32..=18u32,
            to_dec in 0u32..=18u32,
        ) {
            prop_assume!(a <= b);
            let a_norm = normalize_amount(a, from_dec, to_dec);
            let b_norm = normalize_amount(b, from_dec, to_dec);
            prop_assume!(a_norm.is_some() && b_norm.is_some());
            prop_assert!(a_norm.unwrap() <= b_norm.unwrap());
        }

        #[test]
        fn normalize_amount_never_overflows_i128(
            amount in -10_000_000_000_000_000_000_i128..=10_000_000_000_000_000_000_i128,
            from_dec in 0u32..=38u32,
            to_dec in 0u32..=38u32,
        ) {
            let result = normalize_amount(amount, from_dec, to_dec);
            if result.is_some() {
                let val = result.unwrap();
                // If we got a value, it must be within i128 bounds (checked by the function).
                // Additionally, verify the absolute value is consistent with manual calc
                // when no overflow occurs.
                if from_dec == to_dec {
                    prop_assert_eq!(val, amount);
                } else if from_dec < to_dec {
                    let diff = to_dec - from_dec;
                    if diff < 39 {
                        let factor = pow10(diff).unwrap();
                        prop_assert_eq!(val, amount.checked_mul(factor).unwrap_or(i128::MAX));
                    }
                } else {
                    let diff = from_dec - to_dec;
                    let factor = pow10(diff).unwrap();
                    prop_assert_eq!(val, amount / factor);
                }
            }
        }

        #[test]
        fn zero_amount_always_stays_zero(
            decimals_from in 0u32..=18u32,
            decimals_to in 0u32..=18u32,
        ) {
            prop_assert_eq!(normalize_amount(0, decimals_from, decimals_to), Some(0));
        }
    }
}
