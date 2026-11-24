//! Liquidity pool deposit logic (issue #1505).
//!
//! Provides a `deposit()` entry point that accepts token A and token B
//! amounts, mints LP tokens proportional to the depositor's pool share, and
//! updates the pool reserves. The math is kept in a small, storage-agnostic
//! struct so it can be unit tested without a Soroban host.

use serde::{Deserialize, Serialize};

/// Error returned by [`LiquidityPool::deposit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositError {
    /// One or both deposited amounts are zero or negative.
    InvalidAmount,
    /// The deposit would overflow the pool reserves or LP supply.
    Overflow,
    /// The deposit is too small to mint any LP tokens.
    ZeroLiquidityMinted,
}

/// Pool reserves and LP token supply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidityPool {
    /// Reserve of token A held by the pool.
    pub reserve_a: i128,
    /// Reserve of token B held by the pool.
    pub reserve_b: i128,
    /// Total supply of LP tokens outstanding.
    pub total_lp_supply: i128,
}

impl LiquidityPool {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Deposit `amount_a` of token A and `amount_b` of token B, minting LP
    /// tokens proportional to the depositor's share of the pool.
    ///
    /// For the first deposit the minted LP tokens equal the geometric mean of
    /// the two amounts (`sqrt(amount_a * amount_b)`), establishing the initial
    /// share. Subsequent deposits mint LP tokens proportional to the smaller
    /// of the two ratios relative to the existing reserves, so a depositor can
    /// never gain more than their fair share. Reserves and the LP supply are
    /// updated in place on success.
    ///
    /// Returns the number of LP tokens minted.
    pub fn deposit(&mut self, amount_a: i128, amount_b: i128) -> Result<i128, DepositError> {
        if amount_a <= 0 || amount_b <= 0 {
            return Err(DepositError::InvalidAmount);
        }

        let minted = if self.total_lp_supply == 0 {
            // Initial deposit: LP tokens = sqrt(amount_a * amount_b).
            let product = amount_a
                .checked_mul(amount_b)
                .ok_or(DepositError::Overflow)?;
            integer_sqrt(product)
        } else {
            // Proportional mint: min(amount_a * supply / reserve_a,
            //                       amount_b * supply / reserve_b).
            let share_a = amount_a
                .checked_mul(self.total_lp_supply)
                .ok_or(DepositError::Overflow)?
                / self.reserve_a;
            let share_b = amount_b
                .checked_mul(self.total_lp_supply)
                .ok_or(DepositError::Overflow)?
                / self.reserve_b;
            share_a.min(share_b)
        };

        if minted <= 0 {
            return Err(DepositError::ZeroLiquidityMinted);
        }

        // Update reserves and LP supply only after all checks pass.
        self.reserve_a = self
            .reserve_a
            .checked_add(amount_a)
            .ok_or(DepositError::Overflow)?;
        self.reserve_b = self
            .reserve_b
            .checked_add(amount_b)
            .ok_or(DepositError::Overflow)?;
        self.total_lp_supply = self
            .total_lp_supply
            .checked_add(minted)
            .ok_or(DepositError::Overflow)?;

        Ok(minted)
    }
}

/// Integer square root via Newton's method, for non-negative inputs.
fn integer_sqrt(value: i128) -> i128 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_deposit_mints_geometric_mean() {
        let mut pool = LiquidityPool::new();
        let minted = pool.deposit(1_000, 4_000).unwrap();
        assert_eq!(minted, 2_000);
        assert_eq!(pool.reserve_a, 1_000);
        assert_eq!(pool.reserve_b, 4_000);
        assert_eq!(pool.total_lp_supply, 2_000);
    }

    #[test]
    fn subsequent_deposit_mints_proportionally() {
        let mut pool = LiquidityPool::new();
        pool.deposit(1_000, 4_000).unwrap();
        // Doubling both reserves should mint the same LP supply again.
        let minted = pool.deposit(1_000, 4_000).unwrap();
        assert_eq!(minted, 2_000);
        assert_eq!(pool.reserve_a, 2_000);
        assert_eq!(pool.reserve_b, 8_000);
        assert_eq!(pool.total_lp_supply, 4_000);
    }

    #[test]
    fn proportional_deposit_uses_smaller_ratio() {
        let mut pool = LiquidityPool::new();
        pool.deposit(1_000, 4_000).unwrap();
        // amount_a ratio = 500/1000 = 0.5, amount_b ratio = 4000/4000 = 1.0.
        let minted = pool.deposit(500, 4_000).unwrap();
        assert_eq!(minted, 1_000);
        assert_eq!(pool.reserve_a, 1_500);
        assert_eq!(pool.reserve_b, 8_000);
        assert_eq!(pool.total_lp_supply, 3_000);
    }

    #[test]
    fn rejects_zero_amounts() {
        let mut pool = LiquidityPool::new();
        assert_eq!(pool.deposit(0, 100), Err(DepositError::InvalidAmount));
        assert_eq!(pool.deposit(100, 0), Err(DepositError::InvalidAmount));
        assert_eq!(pool.reserve_a, 0);
        assert_eq!(pool.reserve_b, 0);
        assert_eq!(pool.total_lp_supply, 0);
    }

    #[test]
    fn rejects_dust_deposit() {
        let mut pool = LiquidityPool::new();
        pool.deposit(1_000, 4_000).unwrap();
        // 1 * 2000 / 1000 = 2, 1 * 2000 / 4000 = 0 -> min is 0.
        assert_eq!(pool.deposit(1, 1), Err(DepositError::ZeroLiquidityMinted));
        assert_eq!(pool.reserve_a, 1_000);
        assert_eq!(pool.reserve_b, 4_000);
        assert_eq!(pool.total_lp_supply, 2_000);
    }
}
