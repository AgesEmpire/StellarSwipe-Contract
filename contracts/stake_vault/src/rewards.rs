use soroban_sdk::{token, Address, Env};

pub const XLM: i128 = 10_000_000;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;

/// Decimal precision (in base-10 digits) assumed by every amount conversion in
/// this module. All on-chain amounts are stored as integer base units scaled by
/// `10^EXPECTED_DECIMALS`, so a token whose contract reports a different
/// precision cannot be mixed with these balances without silent value loss.
pub const EXPECTED_DECIMALS: u32 = 7;

/// Scale factor implied by [`EXPECTED_DECIMALS`] (`10^EXPECTED_DECIMALS`).
pub const DECIMAL_SCALE: i128 = XLM;

/// Pull `amount` base units of the token at `token` from `from` into `to`
/// using the Soroban token interface.
///
/// `transfer_from` requires the spender (`to`, i.e. this contract) to have been
/// authorized by `from` via `approve`, so the token contract enforces the
/// authorization check on our behalf. The call is rejected for non-positive
/// amounts to avoid no-op or sign-flipped transfers.
pub fn pull_tokens(env: &Env, token: &Address, from: &Address, to: &Address, amount: i128) -> Result<(), &'static str> {
    if amount <= 0 {
        return Err("invalid transfer: amount must be positive");
    }

    let client = token::Client::new(env, token);
    client.transfer_from(to, from, to, &amount);
    Ok(())
}

/// Push `amount` base units of the token at `token` from `from` to `to`
/// using the Soroban token interface.
///
/// `transfer` requires `from` to have authorized the call (the token contract
/// checks the invoker's authorization), so callers must ensure the contract is
/// the authorized spender of `from`'s balance. Non-positive amounts are
/// rejected before reaching the token contract.
pub fn push_tokens(env: &Env, token: &Address, from: &Address, to: &Address, amount: i128) -> Result<(), &'static str> {
    if amount <= 0 {
        return Err("invalid transfer: amount must be positive");
    }

    let client = token::Client::new(env, token);
    client.transfer(from, to, &amount);
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolStatus {
    pub balance: i128,
    pub estimated_days_remaining: u32,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolLow {
    pub balance: i128,
    pub days_remaining: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPool {
    pub balance: i128,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
    pub treasury_balance: i128,
}

/// Minimum balance policies enforced on partial withdrawals.
///
/// `user_minimum` is the reserve that must remain in a user's account after
/// any withdraw, while `strategy_minimum` is the reserve that must remain in
/// the strategy position. Both are inclusive lower bounds: a withdraw that
/// would leave a balance strictly below the relevant minimum is rejected.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimumBalancePolicy {
    pub user_minimum: i128,
    pub strategy_minimum: i128,
}

impl MinimumBalancePolicy {
    pub fn new(user_minimum: i128, strategy_minimum: i128) -> Self {
        Self {
            user_minimum,
            strategy_minimum,
        }
    }

    /// Validate a partial withdraw before any state is mutated.
    ///
    /// Returns `Ok(())` when the withdraw preserves both the user-level and
    /// strategy-level minimums, otherwise returns the offending rule so the
    /// caller can reject the request without touching balances.
    pub fn check_withdraw(
        &self,
        user_balance: i128,
        strategy_balance: i128,
        amount: i128,
    ) -> Result<(), MinimumBalanceViolation> {
        if amount <= 0 {
            return Err(MinimumBalanceViolation::NonPositiveAmount);
        }

        if amount > user_balance {
            return Err(MinimumBalanceViolation::InsufficientBalance);
        }

        let remaining_user = user_balance - amount;
        if remaining_user < self.user_minimum {
            return Err(MinimumBalanceViolation::BelowUserMinimum {
                remaining: remaining_user,
                minimum: self.user_minimum,
            });
        }

        let remaining_strategy = strategy_balance - amount;
        if remaining_strategy < self.strategy_minimum {
            return Err(MinimumBalanceViolation::BelowStrategyMinimum {
                remaining: remaining_strategy,
                minimum: self.strategy_minimum,
            });
        }

        Ok(())
    }
}

/// Reason a partial withdraw was rejected by the minimum balance policy.
#[derive(Clone, Debug, PartialEq)]
pub enum MinimumBalanceViolation {
    NonPositiveAmount,
    InsufficientBalance,
    BelowUserMinimum { remaining: i128, minimum: i128 },
    BelowStrategyMinimum { remaining: i128, minimum: i128 },
}

/// A time-based vesting plan for a single provider's incentive rewards.
///
/// `total_amount` is the full reward allocation. `start_time` is the ledger
/// timestamp at which vesting begins, `cliff` is the duration (in seconds)
/// that must elapse before any amount becomes releasable, and `duration` is
/// the total vesting window measured from `start_time`. `released` tracks the
/// amount already withdrawn so unvested balances stay inaccessible.
#[derive(Clone, Debug, PartialEq)]
pub struct VestingSchedule {
    pub total_amount: i128,
    pub start_time: u64,
    pub cliff: u64,
    pub duration: u64,
    pub released: i128,
}

impl VestingSchedule {
    pub fn new(total_amount: i128, start_time: u64, cliff: u64, duration: u64) -> Self {
        Self {
            total_amount,
            start_time,
            cliff,
            duration,
            released: 0,
        }
    }

    /// Deterministically compute the amount that has vested as of `now`.
    ///
    /// Before the cliff elapses nothing is vested. After the full duration
    /// the entire allocation is vested. In between, vesting is linear and
    /// integer-truncated so the result is fully deterministic.
    pub fn vested_amount(&self, now: u64) -> i128 {
        if now < self.start_time.saturating_add(self.cliff) {
            return 0;
        }

        let elapsed = now.saturating_sub(self.start_time);
        if self.duration == 0 || elapsed >= self.duration {
            return self.total_amount;
        }

        (self.total_amount * elapsed as i128) / self.duration as i128
    }

    /// Amount currently available to release: vested minus already released.
    /// Unvested balances remain inaccessible until scheduled release events.
    pub fn releasable_amount(&self, now: u64) -> i128 {
        (self.vested_amount(now) - self.released).max(0)
    }

    /// Release the currently vested amount, updating the released balance.
    /// Returns the amount released (0 when nothing is yet vested).
    pub fn release(&mut self, now: u64) -> i128 {
        let amount = self.releasable_amount(now);
        self.released += amount;
        amount
    }
}

/// Tracks the last applied deposit so repeated attempts for the same
/// transaction cannot duplicate balances or rewards.
#[derive(Clone, Debug, PartialEq)]
pub struct DepositRetryGuard {
    pub last_deposit_id: u64,
    pub last_deposit_amount: i128,
    pub applied: bool,
}

impl DepositRetryGuard {
    pub fn new() -> Self {
        DepositRetryGuard {
            last_deposit_id: 0,
            last_deposit_amount: 0,
            applied: false,
        }
    }
}

impl Default for DepositRetryGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata a token contract is expected to expose at `stake_vault` setup.
///
/// `address` is the token contract identifier, `decimals` its reported
/// precision, and `is_contract` whether the address actually resolves to a
/// deployed contract (as opposed to an account or an empty/invalid address).
#[derive(Clone, Debug, PartialEq)]
pub struct TokenMetadata {
    pub address: u32,
    pub decimals: u32,
    pub is_contract: bool,
}

/// Immutable token configuration captured at `stake_vault` initialization.
///
/// Once validated, the token address and its precision are frozen: there is no
/// setter, so changing them requires a governed upgrade/redeploy rather than a
/// runtime mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenConfig {
    pub address: u32,
    pub decimals: u32,
}

/// Validate a token contract before `stake_vault` initialization.
///
/// Rejects invalid/zero addresses, addresses that are not contracts, and
/// contracts whose reported decimals differ from [`EXPECTED_DECIMALS`]. On
/// success returns the frozen [`TokenConfig`] used for all amount conversions.
pub fn validate_token_metadata(metadata: &TokenMetadata) -> Result<TokenConfig, &'static str> {
    if metadata.address == 0 {
        return Err("invalid token: address must be non-zero");
    }

    if !metadata.is_contract {
        return Err("invalid token: address is not a contract");
    }

    if metadata.decimals != EXPECTED_DECIMALS {
        return Err("incompatible token: decimals do not match vault precision");
    }

    Ok(TokenConfig {
        address: metadata.address,
        decimals: metadata.decimals,
    })
}

/// Convert a whole-token amount into base units using checked arithmetic.
///
/// Rounding: this conversion is exact for whole tokens; any fractional part is
/// truncated toward zero (integer division), never rounded up. Overflow and
/// negative inputs are r

/* … truncated 9158 chars — edit only what you need near the top … */
