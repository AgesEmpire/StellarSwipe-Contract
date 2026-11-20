//! Checked cross-network asset amount conversion (Issue #1299).
//!
//! Dust policy: conversions are exact. Scaling down is accepted only when the
//! remainder is zero; any discarded dust is an error. Scaling up uses checked
//! arithmetic, so an amount that cannot be represented on the target scale is
//! rejected before mint/release. Zero is always representable.

use crate::BridgeError;
use soroban_sdk::Env;

/// Largest supported decimal-scale difference. Larger gaps are rejected
/// instead of attempting a potentially unbounded power computation.
pub const MAX_DECIMAL_DIFFERENCE: u32 = 18;

/// Convert `amount` from `from_decimals` to `to_decimals` with checked
/// arithmetic and exact (no-silent-dust) scaling.
pub fn convert_asset_amount(
    _env: &Env,
    amount: i128,
    from_decimals: u32,
    to_decimals: u32,
) -> Result<i128, BridgeError> {
    if amount < 0 {
        return Err(BridgeError::AmountConversionFailed);
    }
    if amount == 0 || from_decimals == to_decimals {
        return Ok(amount);
    }

    let difference = from_decimals.abs_diff(to_decimals);
    if difference > MAX_DECIMAL_DIFFERENCE {
        return Err(BridgeError::AmountConversionFailed);
    }
    let factor = 10i128
        .checked_pow(difference)
        .ok_or(BridgeError::AmountConversionFailed)?;

    if to_decimals > from_decimals {
        amount
            .checked_mul(factor)
            .ok_or(BridgeError::AmountConversionFailed)
    } else {
        // Exact division: a non-zero remainder would be silently truncated.
        let remainder = amount % factor;
        if remainder != 0 {
            return Err(BridgeError::AmountConversionFailed);
        }
        amount
            .checked_div(factor)
            .ok_or(BridgeError::AmountConversionFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn equal_decimals_and_zero_amounts() {
        let env = Env::default();
        assert_eq!(convert_asset_amount(&env, 1_234, 8, 8), Ok(1_234));
        assert_eq!(convert_asset_amount(&env, 0, 6, 18), Ok(0));
        assert_eq!(convert_asset_amount(&env, 0, 18, 6), Ok(0));
    }

    #[test]
    fn scale_up_uses_checked_arithmetic() {
        let env = Env::default();
        assert_eq!(
            convert_asset_amount(&env, 12, 6, 8),
            Ok(1_200)
        );
        // i128::MAX / 100 + 1 cannot be scaled up without overflow.
        assert_eq!(
            convert_asset_amount(&env, i128::MAX / 100 + 1, 6, 8),
            Err(BridgeError::AmountConversionFailed)
        );
        assert_eq!(
            convert_asset_amount(&env, 1, 0, MAX_DECIMAL_DIFFERENCE + 1),
            Err(BridgeError::AmountConversionFailed)
        );
    }

    #[test]
    fn scale_down_rejects_remainder_and_accepts_exact_values() {
        let env = Env::default();
        assert_eq!(
            convert_asset_amount(&env, 1_299, 8, 6),
            Err(BridgeError::AmountConversionFailed)
        );
        assert_eq!(convert_asset_amount(&env, 1_200, 8, 6), Ok(12));
        // A non-zero amount that would round down to zero is dust.
        assert_eq!(
            convert_asset_amount(&env, 1, 8, 6),
            Err(BridgeError::AmountConversionFailed)
        );
    }
}
