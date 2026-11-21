//! Checked-arithmetic wrapper for financial amounts (issue #599).
//!
//! `Amount` deliberately does not implement `core::ops::{Add, Sub, Mul, Div}`,
//! so writing `amount_a + amount_b` on it is a compile error rather than a
//! runtime panic or silent wraparound. All arithmetic goes through the
//! `checked_*` methods below, which return `Err(AmountError)` instead of
//! panicking or wrapping silently.
//!
//! As a defense-in-depth backstop for amounts still represented as raw
//! `i128` during migration, crates handling financial amounts (fee_collector,
//! user_portfolio) set `clippy::arithmetic_side_effects = "deny"` in their
//! `[lints.clippy]` table (see issue #599), which flags any direct `+`/`-`/`*`/`/`
//! on integers in that crate.
//!
//! Convention: any new financial amount arithmetic should be expressed in
//! terms of `Amount` and its `checked_*` methods rather than raw `i128` math.

use soroban_sdk::contracttype;

/// A checked-arithmetic wrapper around a Stellar amount (7-decimal `i128`).
///
/// Has no `Add`/`Sub`/`Mul`/`Div` impls on purpose: only `checked_add`,
/// `checked_sub`, `checked_mul`, `checked_mul_rate`, and `checked_div` are
/// available, all returning `Result<Amount, AmountError>`.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(i128);

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmountError {
    Overflow,
    DivisionByZero,
}

impl Amount {
    pub const ZERO: Amount = Amount(0);

    pub fn new(value: i128) -> Self {
        Amount(value)
    }

    pub fn value(self) -> i128 {
        self.0
    }

    pub fn checked_add(self, other: Amount) -> Result<Amount, AmountError> {
        self.0
            .checked_add(other.0)
            .map(Amount)
            .ok_or(AmountError::Overflow)
    }

    pub fn checked_sub(self, other: Amount) -> Result<Amount, AmountError> {
        self.0
            .checked_sub(other.0)
            .map(Amount)
            .ok_or(AmountError::Overflow)
    }

    pub fn checked_mul(self, other: Amount) -> Result<Amount, AmountError> {
        self.0
            .checked_mul(other.0)
            .map(Amount)
            .ok_or(AmountError::Overflow)
    }

    /// Multiply by a `numerator / denominator` rate (e.g. a basis-points fee
    /// or a price ratio) without an intermediate overflow.
    pub fn checked_mul_rate(
        self,
        numerator: i128,
        denominator: i128,
    ) -> Result<Amount, AmountError> {
        if denominator == 0 {
            return Err(AmountError::DivisionByZero);
        }
        let scaled = self.0.checked_mul(numerator).ok_or(AmountError::Overflow)?;
        scaled
            .checked_div(denominator)
            .map(Amount)
            .ok_or(AmountError::Overflow)
    }

    pub fn checked_div(self, other: Amount) -> Result<Amount, AmountError> {
        if other.0 == 0 {
            return Err(AmountError::DivisionByZero);
        }
        self.0
            .checked_div(other.0)
            .map(Amount)
            .ok_or(AmountError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_add_ok() {
        assert_eq!(
            Amount::new(5).checked_add(Amount::new(3)),
            Ok(Amount::new(8))
        );
    }

    #[test]
    fn checked_add_overflow() {
        assert_eq!(
            Amount::new(i128::MAX).checked_add(Amount::new(1)),
            Err(AmountError::Overflow)
        );
    }

    #[test]
    fn checked_sub_overflow() {
        assert_eq!(
            Amount::new(i128::MIN).checked_sub(Amount::new(1)),
            Err(AmountError::Overflow)
        );
    }

    #[test]
    fn checked_mul_overflow() {
        assert_eq!(
            Amount::new(i128::MAX).checked_mul(Amount::new(2)),
            Err(AmountError::Overflow)
        );
    }

    #[test]
    fn checked_div_by_zero() {
        assert_eq!(
            Amount::new(10).checked_div(Amount::ZERO),
            Err(AmountError::DivisionByZero)
        );
    }

    #[test]
    fn checked_mul_rate_applies_fee_bps() {
        // 10_000_000 (100 units at 7dp) * 50 bps / 10_000 = 50_000 (0.5 units)
        assert_eq!(
            Amount::new(10_000_000).checked_mul_rate(50, 10_000),
            Ok(Amount::new(50_000))
        );
    }

    #[test]
    fn checked_mul_rate_division_by_zero() {
        assert_eq!(
            Amount::new(100).checked_mul_rate(1, 0),
            Err(AmountError::DivisionByZero)
        );
    }

    // Deliberately-unchecked arithmetic for lint verification (issue #599):
    // uncommenting the line below must trigger `clippy::arithmetic_side_effects`
    // in this crate because `[lints.clippy] arithmetic_side_effects = "deny"`
    // is set in Cargo.toml.
    //
    // #[test]
    // fn deliberately_unchecked_add_should_be_flagged_by_clippy() {
    //     let a: i128 = i128::MAX;
    //     let b: i128 = 1;
    //     let _ = a + b; // clippy::arithmetic_side_effects fires here
    // }
}
