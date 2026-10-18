//! Checked decimal conversion helpers shared across contracts.
//!
//! Centralizes conversion between token precision (integer base units) and
//! contract fixed-point values with explicit rounding and overflow behavior.
//! All fallible operations return [`DecimalError`] instead of panicking.

use core::fmt;

/// Number of decimal places used by contract fixed-point values.
pub const CONTRACT_DECIMALS: u32 = 7;

/// Scaling factor for contract fixed-point values (10^CONTRACT_DECIMALS).
pub const CONTRACT_SCALE: i128 = 10_000_000;

/// Rounding mode applied when a conversion is not exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMode {
    /// Round toward zero (truncate).
    Down,
    /// Round away from zero.
    Up,
    /// Round to the nearest representable value, ties away from zero.
    Nearest,
}

/// Typed errors returned by checked decimal conversions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalError {
    /// The input was negative where a non-negative value is required.
    NegativeInput,
    /// The result does not fit in the target representation.
    Overflow,
    /// The requested token precision is not supported.
    UnsupportedPrecision,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecimalError::NegativeInput => write!(f, "negative input"),
            DecimalError::Overflow => write!(f, "conversion overflow"),
            DecimalError::UnsupportedPrecision => write!(f, "unsupported token precision"),
        }
    }
}

/// Convert a token amount in base units (with `token_decimals` precision)
/// into a contract fixed-point value, applying `mode` when rounding is needed.
///
/// Returns [`DecimalError::NegativeInput`] for negative amounts,
/// [`DecimalError::UnsupportedPrecision`] when `token_decimals` exceeds
/// [`CONTRACT_DECIMALS`], and [`DecimalError::Overflow`] when the scaled
/// result does not fit in `i128`.
pub fn to_contract_value(
    amount: i128,
    token_decimals: u32,
    mode: RoundingMode,
) -> Result<i128, DecimalError> {
    if amount < 0 {
        return Err(DecimalError::NegativeInput);
    }
    if token_decimals > CONTRACT_DECIMALS {
        return Err(DecimalError::UnsupportedPrecision);
    }

    let diff = CONTRACT_DECIMALS - token_decimals;
    let factor = pow10(diff).ok_or(DecimalError::Overflow)?;
    amount.checked_mul(factor).ok_or(DecimalError::Overflow)
}

/// Convert a contract fixed-point value into a token amount in base units
/// (with `token_decimals` precision), applying `mode` when rounding is needed.
///
/// Returns [`DecimalError::NegativeInput`] for negative values,
/// [`DecimalError::UnsupportedPrecision`] when `token_decimals` exceeds
/// [`CONTRACT_DECIMALS`], and [`DecimalError::Overflow`] when the result does
/// not fit in `i128`.
pub fn from_contract_value(
    value: i128,
    token_decimals: u32,
    mode: RoundingMode,
) -> Result<i128, DecimalError> {
    if value < 0 {
        return Err(DecimalError::NegativeInput);
    }
    if token_decimals > CONTRACT_DECIMALS {
        return Err(DecimalError::UnsupportedPrecision);
    }

    let diff = CONTRACT_DECIMALS - token_decimals;
    let divisor = pow10(diff).ok_or(DecimalError::Overflow)?;
    if divisor == 1 {
        return Ok(value);
    }

    let quotient = value / divisor;
    let remainder = value % divisor;
    if remainder == 0 {
        return Ok(quotient);
    }

    match mode {
        RoundingMode::Down => Ok(quotient),
        RoundingMode::Up => quotient.checked_add(1).ok_or(DecimalError::Overflow),
        RoundingMode::Nearest => {
            // Ties round away from zero; values are non-negative here.
            if remainder * 2 >= divisor {
                quotient.checked_add(1).ok_or(DecimalError::Overflow)
            } else {
                Ok(quotient)
            }
        }
    }
}

/// Compute 10^exp as an `i128`, returning `None` on overflow.
fn pow10(exp: u32) -> Option<i128> {
    let mut acc: i128 = 1;
    let mut i = 0;
    while i < exp {
        acc = acc.checked_mul(10)?;
        i += 1;
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_contract_value_scales_up() {
        assert_eq!(to_contract_value(1, 0, RoundingMode::Down), Ok(CONTRACT_SCALE));
        assert_eq!(to_contract_value(1, 6, RoundingMode::Down), Ok(10));
        assert_eq!(to_contract_value(0, 7, RoundingMode::Down), Ok(0));
    }

    #[test]
    fn to_contract_value_rejects_negative_and_precision() {
        assert_eq!(to_contract_value(-1, 0, RoundingMode::Down), Err(DecimalError::NegativeInput));
        assert_eq!(to_contract_value(1, 8, RoundingMode::Down), Err(DecimalError::UnsupportedPrecision));
    }

    #[test]
    fn to_contract_value_overflow() {
        assert_eq!(to_contract_value(i128::MAX, 0, RoundingMode::Down), Err(DecimalError::Overflow));
    }

    #[test]
    fn from_contract_value_rounding_modes() {
        // 15 / 10 = 1.5
        assert_eq!(from_contract_value(15, 6, RoundingMode::Down), Ok(1));
        assert_eq!(from_contract_value(15, 6, RoundingMode::Up), Ok(2));
        assert_eq!(from_contract_value(15, 6, RoundingMode::Nearest), Ok(2));
        // 14 / 10 = 1.4
        assert_eq!(from_contract_value(14, 6, RoundingMode::Nearest), Ok(1));
        // exact
        assert_eq!(from_contract_value(20, 6, RoundingMode::Nearest), Ok(2));
    }

    #[test]
    fn from_contract_value_rejects_negative_and_precision() {
        assert_eq!(from_contract_value(-1, 0, RoundingMode::Down), Err(DecimalError::NegativeInput));
        assert_eq!(from_contract_value(1, 8, RoundingMode::Down), Err(DecimalError::UnsupportedPrecision));
    }

    #[test]
    fn round_trip_is_lossless_for_supported_precision() {
        for decimals in 0..=CONTRACT_DECIMALS {
            let amount: i128 = 12345;
            let value = to_contract_value(amount, decimals, RoundingMode::Down).unwrap();
            let back = from_contract_value(value, decimals, RoundingMode::Down).unwrap();
            assert_eq!(back, amount);
        }
    }
}
