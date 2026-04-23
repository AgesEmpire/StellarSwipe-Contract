#![no_std]

mod errors;
pub mod triggers;
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
 feature/take-profit-trigger
    /// Oracle contract used by stop-loss/take-profit triggers (`get_price(asset_pair) -> i128`).
    Oracle,
    /// Portfolio contract used by stop-loss/take-profit close calls (`close_position(user, trade_id, pnl)`).

    /// Oracle contract used by stop-loss trigger (`get_price(asset_pair) -> i128`).
    Oracle,
    /// Portfolio contract used by stop-loss trigger (`close_position(user, trade_id, pnl)`).
 main
    StopLossPortfolio,
 feature/copy-trade-balance-check
    /// Overrides default estimated fee used in balance checks (`None` = use default constant).
    CopyTradeEstimatedFee,
    /// Last balance shortfall for `user` (cleared after a successful `execute_copy_trade`).
    LastInsufficientBalance(Address),

 main
}

/// Symbol invoked on the portfolio after a successful limit check (test / integration hook).
pub const RECORD_COPY_POSITION_FN: &str = "record_copy_position";

/// Temporary-storage key for the reentrancy lock on `execute_copy_trade`.
const EXECUTION_LOCK: &str = "ExecLock";

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

 feature/take-profit-trigger
    // ── Stop-loss / take-profit configuration ─────────────────────────────────

    /// Set the oracle contract used by stop-loss/take-profit checks (admin only).

    // ── Stop-loss configuration ───────────────────────────────────────────────

    /// Set the oracle contract used by stop-loss checks (admin only).
 main
    pub fn set_oracle(env: Env, oracle: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, triggers::ORACLE_KEY), &oracle);
    }

 feature/take-profit-trigger
    /// Set the portfolio contract used by stop-loss/take-profit close calls (admin only).

    /// Set the portfolio contract used by stop-loss close calls (admin only).
 main
    pub fn set_stop_loss_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, triggers::PORTFOLIO_KEY), &portfolio);
    }

 feature/take-profit-trigger
    /// Register a stop-loss price for `(user, trade_id)`.

    /// Register a stop-loss price for `(user, trade_id)`.  Callable by the user or a keeper.
 main
    pub fn set_stop_loss_price(env: Env, user: Address, trade_id: u64, stop_loss_price: i128) {
        user.require_auth();
        triggers::set_stop_loss(&env, &user, trade_id, stop_loss_price);
    }

 feature/take-profit-trigger
    /// Check oracle price and trigger stop-loss if breached. Returns `true` when triggered.

    /// Check oracle price and trigger stop-loss if breached.  Returns `true` when triggered.
    /// Callable by an off-chain keeper or on-chain oracle callback.
 main
    pub fn check_and_trigger_stop_loss(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_stop_loss(&env, user, trade_id, asset_pair)
    }

 feature/take-profit-trigger
    /// Register a take-profit price for `(user, trade_id)`.
    pub fn set_take_profit_price(env: Env, user: Address, trade_id: u64, take_profit_price: i128) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
    }

    /// Check oracle price and trigger take-profit if breached. Returns `true` when triggered.
    /// Stop-loss takes priority if both would trigger simultaneously.
    pub fn check_and_trigger_take_profit(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_take_profit(&env, user, trade_id, asset_pair)
    }


 main
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
        let lock_key = Symbol::new(&env, EXECUTION_LOCK);
        if env.storage().temporary().get::<_, bool>(&lock_key).unwrap_or(false) {
            return Err(ContractError::ReentrancyDetected);
        }
        env.storage().temporary().set(&lock_key, &true);

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

        env.storage().temporary().remove(&Symbol::new(&env, EXECUTION_LOCK));
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

    // ── Manual position exit ──────────────────────────────────────────────────

    /// Cancel a copy trade manually: executes a SDEX swap to close the position,
    /// records exit in UserPortfolio, and emits `TradeCancelled`.
    ///
    /// Returns `Unauthorized` if `caller != user`, `TradeNotFound` if the position
    /// does not exist. If the SDEX swap fails the position remains open.
    pub fn cancel_copy_trade(
        env: Env,
        caller: Address,
        user: Address,
        trade_id: u64,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
    ) -> Result<(), ContractError> {
        caller.require_auth();
        if caller != user {
            return Err(ContractError::Unauthorized);
        }

        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)
            .ok_or(ContractError::NotInitialized)?;

        // Verify the position exists for this user.
        let exists: bool = {
            let sym = Symbol::new(&env, "has_position");
            let mut args = Vec::<Val>::new(&env);
            args.push_back(user.clone().into_val(&env));
            args.push_back(trade_id.into_val(&env));
            env.invoke_contract::<bool>(&portfolio, &sym, args)
        };
        if !exists {
            return Err(ContractError::TradeNotFound);
        }

        let router: Address = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;

        // Execute SDEX swap to close the position. If this fails, position stays open.
        let exit_price = execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)?;

        // Compute realized P&L and close position in UserPortfolio.
        let realized_pnl = exit_price - amount;
        let close_sym = Symbol::new(&env, "close_position_by_executor");
        let executor = env.current_contract_address();
        let mut close_args = Vec::<Val>::new(&env);
        close_args.push_back(executor.clone().into_val(&env));
        close_args.push_back(user.clone().into_val(&env));
        close_args.push_back(trade_id.into_val(&env));
        close_args.push_back(realized_pnl.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &close_sym, close_args);

        stellar_swipe_common::EvtTradeCancelled {
            user: user.clone(),
            trade_id,
            exit_price,
            realized_pnl,
        }
        .publish(&env);

        Ok(())
    }
}

#[cfg(test)]
mod test;
