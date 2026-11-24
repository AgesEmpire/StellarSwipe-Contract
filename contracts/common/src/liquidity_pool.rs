//! Liquidity pool deposit and withdrawal logic (issues #1505, #1506).
//!
//! Provides `deposit()` and `withdraw()` entry points that accept token A and
//! token B amounts, mint/burn LP tokens proportional to the depositor's pool
//! share, and update the pool reserves. The math is kept in a small,
//! storage-agnostic struct so it can be unit tested without a Soroban host.

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

/// Error returned by [`LiquidityPool::withdraw`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawError {
    /// The requested LP token amount is zero or negative.
    InvalidAmount,
    /// The pool has no LP tokens outstanding.
    EmptyPool,
    /// The caller does not hold enough LP tokens to burn.
    InsufficientLiquidity,
    /// The withdrawal would overflow the returned amounts.
    Overflow,
}

/// Error returned by [`LiquidityPool::get_amount_out`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapError {
    /// The input amount is zero or negative.
    InvalidAmount,
    /// One or both pool reserves are zero (no liquidity).
    ZeroLiquidity,
    /// The swap would overflow intermediate computations.
    Overflow,
    /// The input is too small to produce any output after fees.
    InsufficientOutput,
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

    /// Burn `lp_amount` LP tokens and withdraw the caller's proportional share
    /// of the pool reserves.
    ///
    /// The returned amounts are computed from the current reserves:
    /// `amount_a = lp_amount * reserve_a / total_lp_supply` and likewise for
    /// token B. Reserves and the LP supply are decremented in place on success.
    ///
    /// Returns the `(amount_a, amount_b)` withdrawn from the pool.
    pub fn withdraw(&mut self, lp_amount: i128) -> Result<(i128, i128), WithdrawError> {
        if lp_amount <= 0 {
            return Err(WithdrawError::InvalidAmount);
        }
        if self.total_lp_supply == 0 {
            return Err(WithdrawError::EmptyPool);
        }
        if lp_amount > self.total_lp_supply {
            return Err(WithdrawError::InsufficientLiquidity);
        }

        // Proportional amounts from current reserves.
        let amount_a = lp_amount
            .checked_mul(self.reserve_a)
            .ok_or(WithdrawError::Overflow)?
            / self.total_lp_supply;
        let amount_b = lp_amount
            .checked_mul(self.reserve_b)
            .ok_or(WithdrawError::Overflow)?
            / self.total_lp_supply;

        // Decrement reserves and LP supply only after all checks pass.
        self.reserve_a -= amount_a;
        self.reserve_b -= amount_b;
        self.total_lp_supply -= lp_amount;

        Ok((amount_a, amount_b))
    }

    /// Compute the output amount for swapping `amount_in` of the input token
    /// using the constant product formula `x * y = k` (issue #1507).
    ///
    /// The swap fee (in basis points, e.g. `30` = 0.30%) is deducted from the
    /// input before the formula is applied:
    ///
    /// ```text
    /// amount_in_after_fee = amount_in * (10_000 - fee_bps) / 10_000
    /// amount_out = reserve_out * amount_in_after_fee
    ///            / (reserve_in + amount_in_after_fee)
    /// ```
    ///
    /// `reserve_in` is the reserve of the token being sold and `reserve_out`
    /// the reserve of the token being bought. Returns an error for zero input,
    /// zero liquidity, overflow, or when the input is too small to yield any
    /// output after fees.
    pub fn get_amount_out(
        &self,
        amount_in: i128,
        reserve_in: i128,
        reserve_out: i128,
        fee_bps: u32,
    ) -> Result<i128, SwapError> {
        if amount_in <= 0 {
            return Err(SwapError::InvalidAmount);
        }
        if reserve_in <= 0 || reserve_out <= 0 {
            return Err(SwapError::ZeroLiquidity);
        }
        if fee_bps >= 10_000 {
            return Err(SwapError::InvalidAmount);
        }

        // Deduct the fee from the input before applying the formula.
        let amount_in_after_fee = amount_in
            .checked_mul((10_000 - fee_bps) as i128)
            .ok_or(SwapError::Overflow)?
            / 10_000;
        if amount_in_after_fee <= 0 {
            return Err(SwapError::InsufficientOutput);
        }

        // amount_out = reserve_out * amount_in_after_fee
        //            / (reserve_in + amount_in_after_fee)
        let numerator = reserve_out
            .checked_mul(amount_in_after_fee)
            .ok_or(SwapError::Overflow)?;
        let denominator = reserve_in
            .checked_add(amount_in_after_fee)
            .ok_or(SwapError::Overflow)?;
        let amount_out = numerator / denominator;

        if amount_out <= 0 {
            return Err(SwapError::InsufficientOutput);
        }

        Ok(amount_out)
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

    #[test]
    fn full_withdrawal_returns_all_reserves() {
        let mut pool = LiquidityPool::new();
        pool.deposit(1_000, 4_000).unwrap();
        let (amount_a, amount_b) = pool.withdraw(2_000).unwrap();
        assert_eq!(amount_a, 1_000);
        assert_eq!(amount_b, 4_000);
        assert_eq!(pool.reserve_a, 0);
        assert_eq!(pool.reserve_b, 0);
        assert_eq!(pool.total_lp_supply, 0);
    }

    #[test]
    fn amount_out_matches_constant_product_formula() {
        let pool = LiquidityPool::new();
        // 0.30% fee: 10_000 in -> 9_970 after fee.
        // out = 1_000_000 * 9_970 / (1_000_000 + 9_970) = 9_871 (floor).
        let out = pool
            .get_amount_out(10_000, 1_000_000, 1_000_000, 30)
            .unwrap();
        assert_eq!(out, 9_871);
    }

    #[test]
    fn amount_out_with_zero_fee() {
        let pool = LiquidityPool::new();
        // out = 2_000 * 1_000 / (1_000 + 1_000) = 1_000.
        let out = pool.get_amount_out(1_000, 1_000, 2_000, 0).unwrap();
        assert_eq!(out, 1_000);
    }

    #[test]
    fn amount_out_rejects_zero_input() {
        let pool = LiquidityPool::new();
        assert_eq!(
            pool.get_amount_out(0, 1_000, 1_000, 30),
            Err(SwapError::InvalidAmount)
        );
    }

    #[test]
    fn amount_out_rejects_zero_liquidity() {
        let pool = LiquidityPool::new();
        assert_eq!(
            pool.get_amount_out(1_000, 0, 1_000, 30),
            Err(SwapError::ZeroLiquidity)
        );
        assert_eq!(
            pool.get_amount_out(1_000, 1_000, 0, 30),
            Err(SwapError::ZeroLiquidity)
        );
    }

    #[test]
    fn amount_out_rejects_dust_input() {
        let pool = LiquidityPool::new();
        // 1 * 9_970 / 10_000 = 0 after fee -> insufficient output.
        assert_eq!(
            pool.get_amount_out(1, 1_000_000, 1_000_000, 30),
            Err(SwapError::InsufficientOutput)
        );
    }
}
