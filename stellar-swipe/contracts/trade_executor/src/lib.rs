#![no_std]

mod errors;
 feature/copy-trade-balance-check
pub mod risk_gates;

use errors::{ContractError, InsufficientBalanceDetail};
use risk_gates::{
    check_position_limit, check_user_balance, DEFAULT_ESTIMATED_COPY_TRADE_FEE,
};

feature/position-limit-copy-trade
pub mod risk_gates;

use errors::ContractError;
use risk_gates::check_position_limit;
 main
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

/// Instance storage keys.
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    /// Contract implementing `get_open_position_count(user) -> u32` (UserPortfolio).
    UserPortfolio,
    /// When set to `true`, this user bypasses [`risk_gates::MAX_POSITIONS_PER_USER`].
    PositionLimitExempt(Address),
 feature/copy-trade-balance-check
    /// Overrides default estimated fee used in balance checks (`None` = use default constant).
    CopyTradeEstimatedFee,
    /// Last balance shortfall for `user` (cleared after a successful `execute_copy_trade`).
    LastInsufficientBalance(Address),

 main
}

/// Symbol invoked on the portfolio after a successful limit check (test / integration hook).
pub const RECORD_COPY_POSITION_FN: &str = "record_copy_position";

 feature/copy-trade-balance-check
#[contract]
pub struct TradeExecutorContract;

fn effective_estimated_fee(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::CopyTradeEstimatedFee)
        .unwrap_or(DEFAULT_ESTIMATED_COPY_TRADE_FEE)
}

#[contractimpl]
impl TradeExecutorContract {



pub mod sdex;

use errors::ContractError;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

use sdex::{execute_sdex_swap, min_received_from_slippage};

#[contracttype]
#[derive(Clone)]
enum StorageKey {
    Admin,
    SdexRouter,
}

 main
#[contract]
pub struct TradeExecutorContract;

#[contractimpl]
impl TradeExecutorContract {
  feature/position-limit-copy-trade

    /// One-time init; stores admin who may configure the SDEX router address.
main
main
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

 feature/copy-trade-balance-check

 feature/position-limit-copy-trade
 main
    /// Configure the portfolio contract used for open-position counts and copy-trade recording.
    pub fn set_user_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::UserPortfolio, &portfolio);
    }

    pub fn get_user_portfolio(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::UserPortfolio)
    }

 feature/copy-trade-balance-check
    /// Set the fee term used in `amount + estimated_fee` balance checks (admin). Use `0` for no fee cushion.
    pub fn set_copy_trade_estimated_fee(env: Env, fee: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if fee < 0 {
            panic!("fee must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::CopyTradeEstimatedFee, &fee);
    }

    pub fn get_copy_trade_estimated_fee(env: Env) -> i128 {
        effective_estimated_fee(&env)
    }

    /// Admin override: exempt `user` from the per-user position cap (or clear exemption).
    pub fn set_position_limit_exempt(env: Env, user: Address, exempt: bool) {

    /// Admin override: exempt `user` from the per-user position cap (or clear exemption).
    pub fn set_position_limit_exempt(env: Env, user: Address, exempt: bool) {

    /// Set the router contract invoked by [`sdex::execute_sdex_swap`].
    pub fn set_sdex_router(env: Env, router: Address) {
 main
main
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
feature/copy-trade-balance-check

feature/position-limit-copy-trade
main
        let key = StorageKey::PositionLimitExempt(user);
        if exempt {
            env.storage().instance().set(&key, &true);
        } else {
            env.storage().instance().remove(&key);
        }
    }

    pub fn is_position_limit_exempt(env: Env, user: Address) -> bool {
        let key = StorageKey::PositionLimitExempt(user);
        env.storage().instance().get(&key).unwrap_or(false)
    }

feature/copy-trade-balance-check
    /// Structured shortfall after the last `InsufficientBalance` from [`Self::execute_copy_trade`].
    pub fn get_insufficient_balance_detail(
        env: Env,
        user: Address,
    ) -> Option<InsufficientBalanceDetail> {
        let key = StorageKey::LastInsufficientBalance(user);
        env.storage().instance().get(&key)
    }

    /// Runs copy trade: balance check (incl. fee), position limit, then portfolio `record_copy_position`.
    pub fn execute_copy_trade(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        user.require_auth();

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }


    /// Runs copy trade: position limit check first, then portfolio `record_copy_position`.
    pub fn execute_copy_trade(env: Env, user: Address) -> Result<(), ContractError> {
        user.require_auth();

main
        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)
            .ok_or(ContractError::NotInitialized)?;

feature/copy-trade-balance-check
        let fee = effective_estimated_fee(&env);
        let bal_key = StorageKey::LastInsufficientBalance(user.clone());
        match check_user_balance(&env, &user, &token, amount, fee) {
            Ok(()) => {
                env.storage().instance().remove(&bal_key);
            }
            Err(detail) => {
                env.storage().instance().set(&bal_key, &detail);
                return Err(ContractError::InsufficientBalance);
            }
        }


main
        let exempt = {
            let key = StorageKey::PositionLimitExempt(user.clone());
            env.storage().instance().get(&key).unwrap_or(false)
        };

        check_position_limit(&env, &portfolio, &user, exempt)?;

        let sym = Symbol::new(&env, RECORD_COPY_POSITION_FN);
        let mut args = Vec::<Val>::new(&env);
        args.push_back(user.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &sym, args);

        Ok(())
feature/copy-trade-balance-check


        env.storage().instance().set(&StorageKey::SdexRouter, &router);
    }

    /// Read configured router (for off-chain tooling).
    pub fn get_sdex_router(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::SdexRouter)
    }

    /// Swap using a caller-supplied minimum output (already includes slippage tolerance).
    pub fn swap(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
    ) -> Result<i128, ContractError> {
        let router = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;
        execute_sdex_swap(
            &env,
            &router,
            &from_token,
            &to_token,
            amount,
            min_received,
        )
    }

    /// Swap with `min_received = amount * (10000 - max_slippage_bps) / 10000`.
    pub fn swap_with_slippage(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        max_slippage_bps: u32,
    ) -> Result<i128, ContractError> {
        let min_received =
            min_received_from_slippage(amount, max_slippage_bps).ok_or(ContractError::InvalidAmount)?;
        Self::swap(env, from_token, to_token, amount, min_received)
main
 main
    }
}

#[cfg(test)]
mod test;
