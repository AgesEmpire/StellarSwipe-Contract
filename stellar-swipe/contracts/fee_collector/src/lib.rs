#![no_std]

mod errors;
pub use errors::ContractError;

mod events;
pub use events::{DeprecatedCall, FeeRateUpdated, TreasuryWithdrawal, WithdrawalQueued};

pub mod distribution;
pub mod fee_cache;
pub use distribution::ProviderShare;

mod storage;
pub use storage::{
    get_admin, get_fee_rate, get_queued_withdrawal, get_treasury_balance, is_initialized,
    remove_queued_withdrawal, set_admin, set_fee_rate as set_fee_rate_storage, set_initialized,
    set_queued_withdrawal, set_treasury_balance, QueuedWithdrawal, StorageKey,
    MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS,
};

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Symbol, Vec};

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
            Some(q)
                if q.recipient == recipient && q.token == token && q.amount == amount =>
            {
                q
            }
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
        Ok(fee_cache::cached_fee_rate(&env))
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
        fee_cache::invalidate_fee_cache(&env);

        FeeRateUpdated {
            old_rate,
            new_rate: new_rate_bps,
            updated_by: admin,
        }
        .publish(&env);

        Ok(())
    }

    // ── Fee distribution ────────────────────────────────────────────────────

    /// Admin-only: configure provider fee shares (bps of epoch fees, sum <= 10_000).
    pub fn set_provider_shares(env: Env, providers: Vec<ProviderShare>) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        get_admin(&env).require_auth();
        distribution::set_providers(&env, &providers)
    }

    /// Admin-only: record a fee collected from a copy-trade execution.
    pub fn record_fee(env: Env, token: Address, amount: i128) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        get_admin(&env).require_auth();
        distribution::record_fee(&env, &token, amount)
    }

    /// Admin-only: close the epoch, crediting providers and retaining the rest in treasury.
    /// Returns the amount retained by treasury.
    pub fn close_epoch(env: Env, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        get_admin(&env).require_auth();
        distribution::close_epoch(&env, &token)
    }

    pub fn provider_balance(env: Env, provider: Address, token: Address) -> i128 {
        distribution::provider_balance(&env, &provider, &token)
    }

    pub fn epoch_fees(env: Env, token: Address) -> i128 {
        distribution::epoch_fees(&env, &token)
    }
}

// ── Deprecated entry-point compatibility shims ──────────────────────────────
//
// Legacy entry points kept so existing integrations keep working across
// upgrades. Each shim emits a `DeprecatedCall` event naming its replacement
// and routes to the current implementation. Remove after the deprecation window.

fn warn_deprecated(env: &Env, entry_point: Symbol, replacement: Symbol) {
    DeprecatedCall { entry_point, replacement }.publish(env);
}

#[contractimpl]
impl FeeCollector {
    /// Deprecated: use `fee_rate`.
    pub fn get_fee_rate(env: Env) -> Result<u32, ContractError> {
        warn_deprecated(&env, symbol_short!("get_fee"), symbol_short!("fee_rate"));
        Self::fee_rate(env)
    }

    /// Deprecated: use `set_fee_rate`.
    pub fn update_fee_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        warn_deprecated(&env, symbol_short!("upd_fee"), symbol_short!("set_fee"));
        Self::set_fee_rate(env, new_rate_bps)
    }

    /// Deprecated: use `treasury_balance`.
    pub fn get_treasury_balance(env: Env, token: Address) -> Result<i128, ContractError> {
        warn_deprecated(&env, symbol_short!("get_trsy"), symbol_short!("trsy_bal"));
        Self::treasury_balance(env, token)
    }
}
