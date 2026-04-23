#![no_std]

mod errors;
pub use errors::ContractError;

mod events;
pub use events::{FeeRateUpdated, FeesBurned, FeesClaimed, TreasuryWithdrawal, WithdrawalQueued};

mod rebates;

mod storage;
pub use storage::{
    get_admin, get_burn_rate, get_fee_rate, get_monthly_trade_volume, get_oracle_contract,
    get_pending_fees, get_queued_withdrawal, get_treasury_balance, is_initialized,
    remove_monthly_trade_volume, remove_queued_withdrawal, set_admin,
    set_burn_rate as set_burn_rate_storage, set_fee_rate as set_fee_rate_storage, set_initialized,
    set_monthly_trade_volume, set_oracle_contract as set_oracle_contract_storage, set_pending_fees,
    set_queued_withdrawal, set_treasury_balance, MonthlyTradeVolume, QueuedWithdrawal, StorageKey,
    MAX_BURN_RATE_BPS, MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS,
};

use soroban_sdk::{contract, contractimpl, token, Address, Env};

use stellar_swipe_common::Asset;
use stellar_swipe_common::SECONDS_PER_DAY;

#[cfg(test)]
mod test;

#[contract]
pub struct FeeCollector;

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
            available_at: queued_at
                .checked_add(SECONDS_PER_DAY)
                .ok_or(ContractError::ArithmeticOverflow)?,
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

        if env.ledger().timestamp()
            < queued.queued_at
                .checked_add(SECONDS_PER_DAY)
                .ok_or(ContractError::ArithmeticOverflow)?
        {
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

    /// Returns the current burn rate in basis points (default: 1000 = 10%).
    pub fn burn_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_burn_rate(&env))
    }

    /// Admin-only: set the percentage of collected fees to burn (in basis points).
    /// Max is 10_000 (100%). Change takes effect on the next fee collection.
    pub fn set_burn_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if new_rate_bps > MAX_BURN_RATE_BPS {
            return Err(ContractError::BurnRateTooHigh);
        }
        set_burn_rate_storage(&env, new_rate_bps);
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
        // ROUNDING STRATEGY: user-paid fee truncates (rounds down) — user-favorable.
        // Integer division in Rust truncates toward zero, so the user is never overcharged.
        // Any sub-stroope remainder is intentionally discarded; it does not accumulate in
        // the contract because it is never credited to any balance.
        let fee_amount = trade_amount
            .checked_mul(fee_rate as i128)
            .and_then(|amount| amount.checked_div(10_000))
            .ok_or(ContractError::ArithmeticOverflow)?;

        if fee_amount <= 0 {
            return Err(ContractError::FeeRoundedToZero);
        }

        token::Client::new(&env, &token).transfer(
            &trader,
            &env.current_contract_address(),
            &fee_amount,
        );

        // ROUNDING STRATEGY: burn slice truncates (rounds down) — provider-favorable.
        // burn_amount + distributable == fee_amount exactly (no dust):
        //   distributable = fee_amount - burn_amount
        // Because burn_amount is truncated, distributable is effectively rounded up,
        // ensuring every stroop of fee_amount is either burned or credited to the treasury.
        let burn_rate = get_burn_rate(&env);
        let burn_amount = fee_amount
            .checked_mul(burn_rate as i128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(ContractError::ArithmeticOverflow)?;
        // distributable = fee_amount - burn_amount: no remainder, no dust possible.
        let distributable = fee_amount
            .checked_sub(burn_amount)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if burn_amount > 0 {
            token::Client::new(&env, &token).burn(&env.current_contract_address(), &burn_amount);
            FeesBurned {
                amount: burn_amount,
                token: token.clone(),
            }
            .publish(&env);
        }

        let updated_treasury_balance = get_treasury_balance(&env, &token)
            .checked_add(distributable)
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
