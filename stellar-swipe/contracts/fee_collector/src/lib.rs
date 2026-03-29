#![no_std]

 feature/reputation-score
use soroban_sdk::{contract, contractimpl, contracterror, contracttype, Address, Env, Symbol, symbol_short, token};

use stellar_swipe_common::assets::Asset;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FeeCollectorError {
    TradeTooSmall = 1,
    ArithmeticOverflow = 2,
    InvalidAmount = 3,
    Unauthorized = 4,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeConfig {
    pub max_fee_per_trade: i128, // 100 XLM equivalent
    pub min_fee_per_trade: i128, // 0.01 XLM equivalent
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum StorageKey {
    ProviderPendingFees(Address, Address),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Admin,
    FeeConfig,
}

#[contract]
pub struct FeeCollectorContract;

#[contractimpl]
impl FeeCollectorContract {
    /// Initialize the contract with admin and default fee config
    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);

        // Default config: 100 XLM = 100 * 10^7 stroops = 1_000_000_000
        // 0.01 XLM = 0.01 * 10^7 = 100_000
        let default_config = FeeConfig {
            max_fee_per_trade: 1_000_000_000, // 100 XLM
            min_fee_per_trade: 100_000,       // 0.01 XLM
        };
        env.storage().instance().set(&DataKey::FeeConfig, &default_config);
    }

    /// Set fee config (admin only)
    pub fn set_fee_config(env: Env, caller: Address, config: FeeConfig) -> Result<(), FeeCollectorError> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        if config.min_fee_per_trade <= 0 || config.max_fee_per_trade <= config.min_fee_per_trade {
            return Err(FeeCollectorError::InvalidAmount);
        }

        env.storage().instance().set(&DataKey::FeeConfig, &config);
        Ok(())
    }

    /// Get current fee config
    pub fn get_fee_config(env: Env) -> FeeConfig {
        env.storage().instance().get(&DataKey::FeeConfig).unwrap()
    }

    /// Collect fee with cap and floor applied
    /// Returns the clamped fee amount
    pub fn collect_fee(env: Env, trade_amount: i128, calculated_fee: i128) -> Result<i128, FeeCollectorError> {
        if trade_amount <= 0 || calculated_fee < 0 {
            return Err(FeeCollectorError::InvalidAmount);
        }

        let config = Self::get_fee_config(env);

        // Check if trade amount is too small to cover minimum fee
        if trade_amount < config.min_fee_per_trade {
            return Err(FeeCollectorError::TradeTooSmall);
        }

        // Clamp the fee between min and max
        let clamped_fee = if calculated_fee < config.min_fee_per_trade {
            config.min_fee_per_trade
        } else if calculated_fee > config.max_fee_per_trade {
            config.max_fee_per_trade
        } else {
            calculated_fee
        };

        // Ensure the clamped fee doesn't exceed the trade amount
        if clamped_fee > trade_amount {
            return Err(FeeCollectorError::TradeTooSmall);
        }

        Ok(clamped_fee)
    }

    /// Claim pending fees for a provider and token
    pub fn claim_fees(env: Env, provider: Address, token: Address) -> i128 {
        provider.require_auth();

        let key = StorageKey::ProviderPendingFees(provider.clone(), token.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);

        if amount > 0 {
            // Transfer tokens from contract to provider
            let token_client = token::Client::new(&env, &token);
            token_client.transfer(&env.current_contract_address(), &provider, &amount);

            // Reset pending balance to 0
            env.storage().persistent().set(&key, &0);
        }

        // Emit FeesClaimed event
        env.events().publish(
            (symbol_short!("fees"), symbol_short!("claimed")),
            (provider, amount, token),
        );

        amount
    }
}
=======
mod errors;
pub use errors::ContractError;

mod events;
pub use events::{TreasuryWithdrawal, WithdrawalQueued};

mod storage;
pub use storage::{
    get_admin, get_queued_withdrawal, get_treasury_balance, is_initialized, remove_queued_withdrawal,
    set_admin, set_initialized, set_queued_withdrawal, set_treasury_balance, QueuedWithdrawal,
    StorageKey,
};

use soroban_sdk::{contract, contractimpl, token, Address, Env};

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
}
 main
