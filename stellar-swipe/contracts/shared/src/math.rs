//! Decimal-precision normalization helpers (Issue #387).
//!
//! All internal calculations use 7-decimal precision (Stellar standard).
//! `normalize_amount` converts an amount between two decimal precisions
//! without precision loss.

/// Convert `amount` from `from_decimals` precision to `to_decimals` precision.
///
/// # Examples
/// ```
/// // Same precision — no change.
/// assert_eq!(normalize_amount(1_000_000_0, 7, 7), Some(1_000_000_0));
/// // 6-decimal → 7-decimal: multiply by 10.
/// assert_eq!(normalize_amount(1_000_000, 6, 7), Some(10_000_000));
/// // 7-decimal → 6-decimal: divide by 10.
/// assert_eq!(normalize_amount(10_000_000, 7, 6), Some(1_000_000));
/// ```
///
/// Returns `None` on overflow.
pub fn normalize_amount(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    match from_decimals.cmp(&to_decimals) {
        core::cmp::Ordering::Equal => Some(amount),
        core::cmp::Ordering::Less => {
            // Scale up: multiply by 10^(to - from)
            let diff = to_decimals - from_decimals;
            let factor = pow10(diff)?;
            amount.checked_mul(factor)
        }
        core::cmp::Ordering::Greater => {
            // Scale down: divide by 10^(from - to)
            let diff = from_decimals - to_decimals;
            let factor = pow10(diff)?;
            Some(amount / factor)
        }
    }
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

    #[test]
    fn same_precision_unchanged() {
        assert_eq!(normalize_amount(1_000_000_0, 7, 7), Some(1_000_000_0));
        assert_eq!(normalize_amount(0, 7, 7), Some(0));
    }

    #[test]
    fn lower_to_higher_precision() {
        // 6 → 7: ×10
        assert_eq!(normalize_amount(1_000_000, 6, 7), Some(10_000_000));
        // 0 → 7: ×10^7
        assert_eq!(normalize_amount(1, 0, 7), Some(10_000_000));
    }

    #[test]
    fn higher_to_lower_precision() {
        // 7 → 6: ÷10
        assert_eq!(normalize_amount(10_000_000, 7, 6), Some(1_000_000));
        // 7 → 0: ÷10^7
        assert_eq!(normalize_amount(10_000_000, 7, 0), Some(1));
    }

    #[test]
    fn overflow_returns_none() {
        // i128::MAX scaled up should overflow
        assert_eq!(normalize_amount(i128::MAX, 0, 39), None);
    }

    #[test]
    fn negative_amounts_work() {
        assert_eq!(normalize_amount(-1_000_000, 6, 7), Some(-10_000_000));
        assert_eq!(normalize_amount(-10_000_000, 7, 6), Some(-1_000_000));
    }
}
