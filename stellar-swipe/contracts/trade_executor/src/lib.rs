#![no_std]

mod errors;
pub mod risk_gates;
pub mod triggers;
pub mod sdex;

use errors::{ContractError, InsufficientBalanceDetail};
use risk_gates::{
    check_user_balance, resolve_trade_amount, validate_and_record_position,
    DEFAULT_ESTIMATED_COPY_TRADE_FEE,
};
use sdex::{execute_sdex_swap, min_received_from_slippage};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

use triggers::{ORACLE_KEY, PORTFOLIO_KEY};

/// Instance storage keys.
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    /// Contract implementing `validate_and_record(user, max_positions) -> u32` (UserPortfolio).
    UserPortfolio,
    /// When set to `true`, this user bypasses the per-user position cap.
    PositionLimitExempt(Address),
    /// Oracle contract used by stop-loss/take-profit triggers (`get_price(asset_pair) -> i128`).
    Oracle,
    /// Portfolio contract used by stop-loss/take-profit close calls (`close_position(user, trade_id, pnl)`).
    StopLossPortfolio,
    /// Overrides default estimated fee used in balance checks (`None` = use default constant).
    CopyTradeEstimatedFee,
    /// Last balance shortfall for a user (cleared after a successful `execute_copy_trade`).
    LastInsufficientBalance(Address),
    SdexRouter,
}

/// Temporary-storage key for the reentrancy lock on `execute_copy_trade`.
const EXECUTION_LOCK: &str = "ExecLock";

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

    /// # Summary
    /// One-time contract initialization. Stores the admin address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    ///
    /// # Returns
    /// Nothing. Panics if already initialized.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    /// # Summary
    /// Configure the portfolio contract used for position validation and
    /// copy-trade recording. Admin auth required.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `portfolio`: Address of the UserPortfolio contract.
    ///
    /// # Returns
    /// Nothing. Panics if not initialized.
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

    /// Set the fee term used in `amount + estimated_fee` balance checks (admin).
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
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
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

    // ── Stop-loss / take-profit configuration ─────────────────────────────────

    /// Set the oracle contract used by stop-loss/take-profit checks (admin only).
    pub fn set_oracle(env: Env, oracle: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ORACLE_KEY), &oracle);
    }

    /// Set the portfolio contract used by stop-loss/take-profit close calls (admin only).
    pub fn set_stop_loss_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PORTFOLIO_KEY), &portfolio);
    }

    /// Register a stop-loss price for `(user, trade_id)`.
    pub fn set_stop_loss_price(env: Env, user: Address, trade_id: u64, stop_loss_price: i128) {
        user.require_auth();
        triggers::set_stop_loss(&env, &user, trade_id, stop_loss_price);
    }

    /// Check oracle price and trigger stop-loss if breached. Returns `true` when triggered.
    pub fn check_and_trigger_stop_loss(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_stop_loss(&env, user, trade_id, asset_pair)
    }

    /// Register a take-profit price for `(user, trade_id)`.
    pub fn set_take_profit_price(env: Env, user: Address, trade_id: u64, take_profit_price: i128) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
    }

    pub fn set_take_profit_price_with_pair(
        env: Env,
        user: Address,
        trade_id: u64,
        take_profit_price: i128,
        asset_pair: u32,
    ) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
        let _ = asset_pair; // watch registration reserved for future keeper indexer
    }

    pub fn check_and_trigger_take_profit(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_take_profit(&env, user, trade_id, asset_pair)
    }

    /// Structured shortfall after the last `InsufficientBalance` from [`Self::execute_copy_trade`].
    pub fn get_insufficient_balance_detail(
        env: Env,
        user: Address,
    ) -> Option<InsufficientBalanceDetail> {
        let key = StorageKey::LastInsufficientBalance(user);
        env.storage().instance().get(&key)
    }

    /// Execute a copy trade.
    ///
    /// ## Cross-contract call budget (Issue #306 optimization)
    /// | # | Callee            | Purpose                                      |
    /// |---|-------------------|----------------------------------------------|
    /// | 1 | SEP-41 token SAC  | Balance check (`token.balance(user)`)        |
    /// | 2 | UserPortfolio     | `validate_and_record(user, max_positions)`   |
    ///
    /// Previously 3 calls (balance + get_open_position_count + record_copy_position).
    /// Now 2 calls — calls #2 and #3 are batched into a single portfolio entrypoint.
    pub fn execute_copy_trade(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
        portfolio_pct_bps: Option<u32>,
    ) -> Result<(), ContractError> {
        // ── Auth ──────────────────────────────────────────────────────────────
        user.require_auth();

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        // ── Reentrancy guard ──────────────────────────────────────────────────
        let lock_key = Symbol::new(&env, EXECUTION_LOCK);
        if env
            .storage()
            .temporary()
            .get::<_, bool>(&lock_key)
            .unwrap_or(false)
        {
            return Err(ContractError::ReentrancyDetected);
        }
        env.storage().temporary().set(&lock_key, &true);

        // ── Read cached config from instance storage (no cross-contract call) ─
        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)
            .ok_or(ContractError::NotInitialized)?;

        let exempt = {
            let key = StorageKey::PositionLimitExempt(user.clone());
            env.storage().instance().get(&key).unwrap_or(false)
        };

        // ── Resolve effective amount (portfolio % or explicit) ─────────────────
        let oracle: Option<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, triggers::ORACLE_KEY));
        let effective_amount = match resolve_trade_amount(
            &env, &user, &token, amount, portfolio_pct_bps, oracle,
        ) {
            Ok(a) => a,
            Err(e) => {
                env.storage().temporary().remove(&lock_key);
                return Err(e);
            }
        };

        // ── Cross-contract call #1: SEP-41 balance check ──────────────────────
        let fee = effective_estimated_fee(&env);
        let bal_key = StorageKey::LastInsufficientBalance(user.clone());
        match check_user_balance(&env, &user, &token, effective_amount, fee) {
            Ok(()) => {
                env.storage().instance().remove(&bal_key);
            }
            Err(detail) => {
                env.storage().instance().set(&bal_key, &detail);
                env.storage().temporary().remove(&lock_key);
                return Err(ContractError::InsufficientBalance);
            }
        }

        // ── Cross-contract call #2: batched position-limit check + record ─────
        validate_and_record_position(&env, &portfolio, &user, exempt)?;

        env.storage().temporary().remove(&lock_key);
        Ok(())
    }

    // ── SDEX router configuration ─────────────────────────────────────────────

    /// Set the router contract invoked by [`sdex::execute_sdex_swap`].
    pub fn set_sdex_router(env: Env, router: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&StorageKey::SdexRouter, &router);
    }

    pub fn get_sdex_router(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::SdexRouter)
    }

    /// # Summary
    /// Execute a swap via the configured SDEX router with an explicit minimum
    /// received amount. Enforces slippage at the balance-delta level.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `from_token`: SEP-41 token to sell.
    /// - `to_token`: SEP-41 token to buy.
    /// - `amount`: Amount of `from_token` to sell (must be > 0).
    /// - `min_received`: Minimum acceptable amount of `to_token` (must be >= 0).
    ///
    /// # Returns
    /// Actual amount of `to_token` received.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — SDEX router not configured.
    /// - [`ContractError::InvalidAmount`] — amount <= 0 or min_received < 0.
    /// - [`ContractError::SlippageExceeded`] — actual received < min_received.
    ///
    /// # Example
    /// ```rust,ignore
    /// client.swap(&xlm_token, &usdc_token, &1_000_0000000i128, &990_0000000i128);
    /// ```
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
        execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)
    }

    /// # Summary
    /// Execute a swap with automatic slippage protection. Computes
    /// `min_received = amount * (10_000 - max_slippage_bps) / 10_000`
    /// and delegates to [`Self::swap`].
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `from_token`: SEP-41 token to sell.
    /// - `to_token`: SEP-41 token to buy.
    /// - `amount`: Amount of `from_token` to sell.
    /// - `max_slippage_bps`: Maximum acceptable slippage in basis points (e.g. `100` = 1%).
    ///
    /// # Returns
    /// Actual amount of `to_token` received.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAmount`] — amount <= 0 or slippage calculation overflows.
    /// - [`ContractError::NotInitialized`] — SDEX router not configured.
    /// - [`ContractError::SlippageExceeded`] — actual received < computed min_received.
    pub fn swap_with_slippage(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        max_slippage_bps: u32,
    ) -> Result<i128, ContractError> {
        let min_received = min_received_from_slippage(amount, max_slippage_bps)
            .ok_or(ContractError::InvalidAmount)?;
        Self::swap(env, from_token, to_token, amount, min_received)
    }

    // ── Manual position exit ──────────────────────────────────────────────────

    /// Cancel a copy trade manually: executes a SDEX swap to close the position,
    /// records exit in UserPortfolio, and emits `TradeCancelled`.
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

        let exists: bool = {
            let sym = Symbol::new(&env, "has_position");
            let mut args = Vec::<Val>::new(&env);
            args.push_back(user.clone().into_val(&env));
            args.push_back(trade_id.into_val(&env));
            env.invoke_contract(&portfolio, &sym, args)
        };
        if !exists {
            return Err(ContractError::TradeNotFound);
        }

        let router: Address = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;

        let exit_price = execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)?;

        let realized_pnl = exit_price - amount;
        let close_sym = Symbol::new(&env, "close_position");
        let mut close_args = Vec::<Val>::new(&env);
        close_args.push_back(user.clone().into_val(&env));
        close_args.push_back(trade_id.into_val(&env));
        close_args.push_back(realized_pnl.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &close_sym, close_args);

        shared::events::emit_trade_cancelled(
            &env,
            shared::events::EvtTradeCancelled {
                user: user.clone(),
                trade_id,
                exit_price,
                realized_pnl,
            },
        );

        Ok(())
    }
}

#[cfg(test)]
mod test;
#[cfg(test)]
mod tests {
    mod test_oracle_staleness;
}
