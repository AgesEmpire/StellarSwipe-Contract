#![no_std]

mod errors;
pub use errors::ContractError;

mod events;
pub use events::{FeeRateUpdated, TreasuryWithdrawal, WithdrawalQueued};

mod storage;
pub use storage::{
    get_admin, get_epoch_fees, get_fee_rate, get_provider_balance, get_queued_withdrawal,
    get_treasury_balance, is_initialized, remove_queued_withdrawal, set_admin, set_epoch_fees,
    set_fee_rate as set_fee_rate_storage, set_initialized, set_provider_balance,
    set_queued_withdrawal, set_treasury_balance, QueuedWithdrawal, StorageKey, MAX_FEE_RATE_BPS,
    MIN_FEE_RATE_BPS,
};

use soroban_sdk::{contract, contractimpl, token, Address, Env, Vec};

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
            available_at: queued_at + 86400,
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

        if env.ledger().timestamp() < queued.queued_at + 86400 {
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

    /// Records a fee collected from a copy-trade execution into the current epoch accumulator.
    /// `fee_amount` is in stroops and must be > 0.
    pub fn collect_trade_fee(
        env: Env,
        token: Address,
        fee_amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        if fee_amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        let current = get_epoch_fees(&env, &token);
        let new_total = current
            .checked_add(fee_amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_epoch_fees(&env, &token, new_total);
        Ok(())
    }

    /// Closes the current epoch: distributes provider shares and retains the remainder
    /// as treasury balance.
    ///
    /// `providers`      – ordered list of provider addresses
    /// `shares_bps`     – parallel list of basis-point allocations (must sum ≤ 10_000)
    ///
    /// Each provider's credited balance is incremented by:
    ///   floor(total_epoch_fees * share_bps / 10_000)
    ///
    /// Treasury retains: total_epoch_fees - sum(provider_shares)
    /// Epoch accumulator is reset to 0 after distribution.
    pub fn close_epoch(
        env: Env,
        token: Address,
        providers: Vec<Address>,
        shares_bps: Vec<u32>,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        if providers.len() != shares_bps.len() {
            return Err(ContractError::InvalidAmount);
        }

        let total_fees = get_epoch_fees(&env, &token);
        if total_fees == 0 {
            return Ok(());
        }

        let mut distributed: i128 = 0;
        let len = providers.len();
        let mut i: u32 = 0;
        while i < len {
            let provider = providers.get(i).unwrap();
            let bps = shares_bps.get(i).unwrap() as i128;
            let share = total_fees
                .checked_mul(bps)
                .ok_or(ContractError::ArithmeticOverflow)?
                / 10_000;
            let prev = get_provider_balance(&env, &provider, &token);
            let next = prev
                .checked_add(share)
                .ok_or(ContractError::ArithmeticOverflow)?;
            set_provider_balance(&env, &provider, &token, next);
            distributed = distributed
                .checked_add(share)
                .ok_or(ContractError::ArithmeticOverflow)?;
            i += 1;
        }

        // Treasury retains the remainder (rounding dust stays here)
        let treasury_share = total_fees
            .checked_sub(distributed)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let prev_treasury = get_treasury_balance(&env, &token);
        set_treasury_balance(
            &env,
            &token,
            prev_treasury
                .checked_add(treasury_share)
                .ok_or(ContractError::ArithmeticOverflow)?,
        );

        // Reset epoch accumulator
        set_epoch_fees(&env, &token, 0);

        Ok(())
    }

    /// Returns the credited (undistributed) balance for a provider.
    pub fn provider_balance(
        env: Env,
        provider: Address,
        token: Address,
    ) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_provider_balance(&env, &provider, &token))
    }
}
