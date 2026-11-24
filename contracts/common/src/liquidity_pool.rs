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

/// Error returned by [`LiquidityPool::set_fee_bps`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeError {
    /// The requested fee is not a valid basis-point value (< 10_000).
    InvalidFee,
}

/// Default swap fee in basis points (0.30%).
pub const DEFAULT_FEE_BPS: u32 = 30;

/// Pool reserves and LP token supply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidityPool {
    /// Reserve of token A held by the pool.
    pub reserve_a: i128,
    /// Reserve of token B held by the pool.
    pub reserve_b: i128,
    /// Total supply of LP tokens outstanding.
    pub total_lp_supply: i128,
    /// Swap fee in basis points (e.g. `30` = 0.30%).
    pub fee_bps: u32,
    /// Accumulated swap fees collected for token A.
    pub accumulated_fee_a: i128,
    /// Accumulated swap fees collected for token B.
    pub accumulated_fee_b: i128,
}

impl LiquidityPool {
    /// Create an empty pool with the default swap fee.
    pub fn new() -> Self {
        Self {
            fee_bps: DEFAULT_FEE_BPS,
            ..Self::default()
        }
    }

    /// Return the configured swap fee in basis points.
    pub fn fee_bps(&self) -> u32 {
        self.fee_bps
    }

    /// Set the swap fee in basis points. Admin-only at the contract layer;
    /// this storage-agnostic helper validates the value.
    pub fn set_fee_bps(&mut self, fee_bps: u32) -> Result<(), FeeError> {
        if fee_bps >= 10_000 {
            return Err(FeeError::InvalidFee);
        }
        self.fee_bps = fee_bps;
        Ok(())
    }

    /// Return the accumulated fees collected for token A and token B.
    pub fn accumulated_fees(&self) -> (i128, i128) {
        (self.accumulated_fee_a, self.accumulated_fee_b)
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

    /// Execute a swap of `amount_in` of the input token, using the configured
    /// fee, and accumulate the collected fee for the input token.
    ///
    /// `a_to_b` selects the swap direction: `true` sells token A for token B,
    /// `false` sells token B for token A. The fee is deducted from the input
    /// before the swap calculation and tracked in `accumulated_fee_a` or
    /// `accumulated_fee_b` respectively. Reserves are updated in place.
    ///
    /// Returns the output amount of the token being bought.
    pub fn swap(&mut self, amount_in: i128, a_to_b: bool) -> Result<i128, SwapError> {
        let (reserve_in, reserve_out) = if a_to_b {
            (self.reserve_a, self.reserve_b)
        } else {
            (self.reserve_b, self.reserve_a)
        };

        let amount_out = self.get_amount_out(amount_in, reserve_in, reserve_out, self.fee_bps)?;

        // Fee collected on the input token.
        let fee = amount_in
            .checked_mul(self.fee_bps as i128)
            .ok_or(SwapError::Overflow)?
            / 10_000;
        let amount_in_after_fee = amount_in - fee;

        if a_to_b {
            self.reserve_a = self
                .reserve_a
                .checked_add(amount_in_after_fee)
                .ok_or(SwapError::Overflow)?;
            self.reserve_b = self
                .reserve_b
                .checked_sub(amount_out)
                .ok_or(SwapError::Overflow)?;
            self.accumulated_fee_a = self
                .accumulated_fee_a
                .checked_add(fee)
                .ok_or(SwapError::Overflow)?;
        } else {
            self.reserve_b = self
                .reserve_b
                .checked_add(amount_in_after_fee)
                .ok_or(SwapError::Overflow)?;
            self.reserve_a = self
                .reserve_a
                .checked_sub(amount_out)
                .ok_or(SwapError::Overflow)?;
            self.accumulated_fee_b = self
                .accumulated_fee_b
                .checked_add(fee)
                .ok_or(SwapError::Overflow)?;
        }

        Ok(amount_out)
    }
}

/// Integer square root via Newton's method.
fn integer_sqrt(n: i128) -> i128 {
    if n < 0 {
        return 0;
    }
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_pool() -> LiquidityPool {
        let mut pool = LiquidityPool::new();
        pool.deposit(1_000_000, 1_000_000).unwrap();
        pool
    }

    #[test]
    fn default_fee_is_30_bps() {
        let pool = LiquidityPool::new();
        assert_eq!(pool.fee_bps(), 30);
    }

    #[test]
    fn set_fee_bps_updates_storage() {
        let mut pool = LiquidityPool::new();
        pool.set_fee_bps(100).unwrap();
        assert_eq!(pool.fee_bps(), 100);
    }

    #[test]
    fn set_fee_bps_rejects_invalid() {
        let mut pool = LiquidityPool::new();
        assert_eq!(pool.set_fee_bps(10_000), Err(FeeError::InvalidFee));
        assert_eq!(pool.fee_bps(), 30);
    }

    #[test]
    fn fee_is_deducted_from_input() {
        let pool = seeded_pool();
        // 0.30% fee on 10_000 input leaves 9_970 for the formula.
        let out = pool
            .get_amount_out(10_000, 1_000_000, 1_000_000, 30)
            .unwrap();
        let expected = 1_000_000i128 * 9_970 / (1_000_000 + 9_970);
        assert_eq!(out, expected);
    }

    #[test]
    fn swap_accumulates_fees_per_token() {
        let mut pool = seeded_pool();
        pool.swap(10_000, true).unwrap();
        assert_eq!(pool.accumulated_fees(), (30, 0));
        pool.swap(10_000, false).unwrap();
        assert_eq!(pool.accumulated_fees(), (30, 30));
    }

    #[test]
    fn swap_uses_configured_fee() {
        let mut pool = seeded_pool();
        pool.set_fee_bps(100).unwrap();
        pool.swap(10_000, true).unwrap();
        assert_eq!(pool.accumulated_fees(), (100, 0));
    }
}
