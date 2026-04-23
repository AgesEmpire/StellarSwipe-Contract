#![no_std]

mod errors;
pub use errors::ContractError;

mod events;
pub use events::{FeeRateUpdated, FeesClaimed, TreasuryWithdrawal, WithdrawalQueued};

mod rebates;

mod storage;
pub use storage::{
    get_admin, get_fee_rate, get_monthly_trade_volume, get_oracle_contract, get_pending_fees,
    get_queued_withdrawal, get_treasury_balance, is_initialized, remove_monthly_trade_volume,
    remove_queued_withdrawal, set_admin, set_fee_rate as set_fee_rate_storage, set_initialized,
    set_monthly_trade_volume, set_oracle_contract as set_oracle_contract_storage, set_pending_fees,
    set_queued_withdrawal, set_treasury_balance, MonthlyTradeVolume, QueuedWithdrawal, StorageKey,
    MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS,
};

use soroban_sdk::{contract, contractimpl, token, Address, Env};
 refactor/157-shared-constants
use stellar_swipe_common::SECONDS_PER_DAY;

use stellar_swipe_common::Asset;
 main

#[cfg(test)]
mod test;

#[contract]
pub struct FeeCollector;

// ── Rounding helpers ──────────────────────────────────────────────────────────
//
// All fee arithmetic uses these two functions so the rounding strategy is
// centralised and easy to audit.
//
// • `fee_amount_floor` — truncates (rounds DOWN).  Used when charging the user.
//   The user never pays more than their exact fractional share.
//
// • `fee_amount_ceil`  — rounds UP.  Reserved for future provider-reward splits
//   where the protocol should not lose sub-unit amounts to rounding.
//
// Neither function can produce unwithdrawable dust:
//   - floor: the foregone sub-unit stays with the user (never enters the contract).
//   - ceil:  the extra sub-unit is collected from the user and fully credited to
//            the recipient, so the contract balance is always exactly the sum of
//            all credited amounts.

/// `amount * rate_bps / 10_000`, truncated (user-favorable).
pub(crate) fn fee_amount_floor(amount: i128, rate_bps: u32) -> Option<i128> {
    amount
        .checked_mul(rate_bps as i128)?
        .checked_div(10_000)
}

/// `ceil(amount * rate_bps / 10_000)` — rounds UP (protocol-favorable).
/// Use for provider reward splits to avoid sub-unit loss.
pub(crate) fn fee_amount_ceil(amount: i128, rate_bps: u32) -> Option<i128> {
    let numerator = amount.checked_mul(rate_bps as i128)?;
    // ceil(a/b) = (a + b - 1) / b  for positive a, b
    numerator.checked_add(10_000 - 1)?.checked_div(10_000)
}

#[contractimpl]
impl FeeCollector {
    pub fn initialize(env: Env, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();
        if is_initialized(&env) {
            return Err(ContractError::AlreadyInitialized);
        }
        set_admin(&env, &admin);
        set_initialized(&env);
        Ok(())
    }

    pub fn set_oracle_contract(env: Env, oracle_contract: Address) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        set_oracle_contract_storage(&env, &oracle_contract);
        Ok(())
    }

    pub fn fee_rate_for_user(env: Env, user: Address) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(rebates::get_fee_rate_for_user(&env, &user))
    }

    pub fn monthly_trade_volume(env: Env, user: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(rebates::get_active_volume_usd(&env, &user))
    }

    pub fn treasury_balance(env: Env, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_treasury_balance(&env, &token))
    }

    pub fn queue_withdrawal(
        env: Env,
        recipient: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        if amount > get_treasury_balance(&env, &token) {
            return Err(ContractError::InsufficientTreasuryBalance);
        }
        let queued_at = env.ledger().timestamp();
        set_queued_withdrawal(
            &env,
            &QueuedWithdrawal {
                recipient: recipient.clone(),
                token: token.clone(),
                amount,
                queued_at,
            },
        );
        WithdrawalQueued {
            recipient: recipient.clone(),
            token: token.clone(),
            amount,
            available_at: queued_at + SECONDS_PER_DAY,
        }
        .publish(&env);
        Ok(())
    }

    pub fn withdraw_treasury_fees(
        env: Env,
        recipient: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        let queued = match get_queued_withdrawal(&env) {
            Some(q) if q.recipient == recipient && q.token == token && q.amount == amount => q,
            _ => return Err(ContractError::WithdrawalNotQueued),
        };

        if env.ledger().timestamp() < queued.queued_at + SECONDS_PER_DAY {
            return Err(ContractError::TimelockNotElapsed);
        }

        if amount > get_treasury_balance(&env, &token) {
            return Err(ContractError::InsufficientTreasuryBalance);
        }

        let new_balance = get_treasury_balance(&env, &token)
            .checked_sub(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;

        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        set_treasury_balance(&env, &token, new_balance);
        remove_queued_withdrawal(&env);

        TreasuryWithdrawal {
            recipient: recipient.clone(),
            token: token.clone(),
            amount,
            remaining_balance: new_balance,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the current fee rate in basis points.
    pub fn fee_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_fee_rate(&env))
    }

    /// Admin-only: update the fee rate (in basis points).
    /// Validates: MIN_FEE_RATE_BPS <= new_rate_bps <= MAX_FEE_RATE_BPS.
    /// Change takes effect on the next trade — no retroactive application.
    pub fn set_fee_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        if new_rate_bps > MAX_FEE_RATE_BPS {
            return Err(ContractError::FeeRateTooHigh);
        }
        if new_rate_bps < MIN_FEE_RATE_BPS {
            return Err(ContractError::FeeRateTooLow);
        }

        let old_rate = get_fee_rate(&env);
        set_fee_rate_storage(&env, new_rate_bps);

        FeeRateUpdated {
            old_rate,
            new_rate: new_rate_bps,
            updated_by: admin,
        }
        .publish(&env);

        Ok(())
    }

    pub fn collect_fee(
        env: Env,
        trader: Address,
        token: Address,
        trade_amount: i128,
        trade_asset: Asset,
    ) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        trader.require_auth();

        if trade_amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let fee_rate = rebates::get_fee_rate_for_user(&env, &trader);
        // Rounding strategy: fee charged to the user truncates (rounds DOWN).
        // This is user-favorable: the user never pays more than their exact share.
        // Any sub-unit remainder stays with the user, not the protocol.
        // Consequence: at most (fee_rate / 10_000) units of dust per trade are
        // foregone by the protocol — this is intentional and not withdrawable dust.
        // See docs/security/fee_rounding_analysis.md for full analysis.
        let fee_amount = fee_amount_floor(trade_amount, fee_rate)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if fee_amount <= 0 {
            return Err(ContractError::FeeRoundedToZero);
        }

        token::Client::new(&env, &token).transfer(
            &trader,
            &env.current_contract_address(),
            &fee_amount,
        );

        let updated_treasury_balance = get_treasury_balance(&env, &token)
            .checked_add(fee_amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_treasury_balance(&env, &token, updated_treasury_balance);

        rebates::record_trade_volume(&env, &trader, &trade_asset, trade_amount)?;

        Ok(fee_amount)
    }

    /// Claim all pending fee earnings for a provider and token.
    /// Returns the amount claimed (0 if no pending balance).
    pub fn claim_fees(env: Env, provider: Address, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        provider.require_auth();

        let amount = get_pending_fees(&env, &provider, &token);

        if amount > 0 {
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &provider,
                &amount,
            );
            set_pending_fees(&env, &provider, &token, 0);
        }

        FeesClaimed {
            provider: provider.clone(),
            token: token.clone(),
            amount,
        }
        .publish(&env);

        Ok(amount)
    }
}
