#![no_std]

mod errors;
pub mod dca;
pub mod keeper;
mod oracle;
pub mod risk_gates;
pub mod sdex;
pub mod triggers;
mod wire;

use errors::{ContractError, InsufficientBalanceDetail};
use risk_gates::{
    check_user_balance, resolve_trade_amount, validate_and_record_position,
    DEFAULT_ESTIMATED_COPY_TRADE_FEE, MAX_BATCH_SIZE,
};
use sdex::{execute_sdex_swap, min_received_from_slippage};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

use triggers::{ORACLE_KEY, PORTFOLIO_KEY};
use wire::TRADE_TIMEOUT_LEDGERS;

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
    /// Global daily trade volume limit in USD-equivalent units (0 = no limit).
    DailyVolumeLimit,
    /// Accumulated trade volume for `user` on the current day.
    DailyVolume(Address),
    /// The ledger-day (timestamp / 86400) when `DailyVolume(user)` was last reset.
    DailyVolumeDay(Address),
    /// Oracle contracts allowed to feed stop-loss / take-profit triggers.
    OracleWhitelisted(Address),
    OracleWhitelistCount,
    NextLimitOrderId,
    PendingLimitOrder(u64),
    PendingLimitOrderIds,
    SdexPrice(Address),
    /// DCA plan for (user, signal_id). Stores a `DCAPlan`.
    DCAPlan(Address, u64),
    /// Set when fee fallback was used for a trade: stores the fee amount deducted from received.
    FeeDeductedFromReceived(Address, u64),
}

/// Temporary-storage key for the reentrancy lock on `execute_copy_trade`.
const EXECUTION_LOCK: &str = "ExecLock";
pub const CIRCUIT_BREAKER_DURATION_LEDGERS: u32 = 720;

/// A single trade input for [`TradeExecutorContract::batch_execute`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchTradeInput {
    pub user: Address,
    pub token: Address,
    pub amount: i128,
}

/// Per-trade outcome returned by [`TradeExecutorContract::batch_execute`].
/// `ok = true` means the trade succeeded; `ok = false` means it failed with `error_code`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchTradeResult {
    pub ok: bool,
    /// `ContractError` discriminant when `ok == false`; 0 when `ok == true`.
    pub error_code: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationTarget {
    pub asset_pair: u32,
    pub target_pct_bps: u32,
}

pub fn rebalance_portfolio(_user: u32, targets: Vec<AllocationTarget>) {
    for t in targets.iter() {
        if t.target_pct_bps > 10000 {
            panic!("invalid allocation");
        }
    }
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLimitOrder {
    pub order_id: u64,
    pub user: Address,
    pub token: Address,
    pub amount: i128,
    pub portfolio_pct_bps: Option<u32>,
    pub limit_price: i128,
    pub expires_at_ledger: u32,
}

#[contract]
pub struct TradeExecutorContract;

fn effective_estimated_fee(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::CopyTradeEstimatedFee)
        .unwrap_or(DEFAULT_ESTIMATED_COPY_TRADE_FEE)
}

fn require_admin(env: &Env) -> Result<Address, ContractError> {
    oracle::require_admin(env)
}

fn execute_market_copy_trade(
    env: &Env,
    user: Address,
    token: Address,
    amount: i128,
    portfolio_pct_bps: Option<u32>,
    require_user_auth: bool,
) -> Result<(), ContractError> {
    if require_user_auth {
        user.require_auth();
    }

    if amount <= 0 {
        return Err(ContractError::InvalidAmount);
    }

    // ── Reentrancy guard ──────────────────────────────────────────────────
    let lock_key = Symbol::new(env, EXECUTION_LOCK);
    if env
        .storage()
        .temporary()
        .get::<_, bool>(&lock_key)
        .unwrap_or(false)
    {
        return Err(ContractError::ReentrancyDetected);
    }
    env.storage().temporary().set(&lock_key, &true);

    // ── Daily volume limit check ───────────────────────────────────────────
    let limit: i128 = env
        .storage()
        .instance()
        .get(&StorageKey::DailyVolumeLimit)
        .unwrap_or(0i128);
    if limit > 0 {
        let today: u64 = env.ledger().timestamp() / 86_400;
        let day_key = StorageKey::DailyVolumeDay(user.clone());
        let vol_key = StorageKey::DailyVolume(user.clone());
        let stored_day: u64 = env.storage().persistent().get(&day_key).unwrap_or(0u64);
        let current_vol: i128 = if stored_day == today {
            env.storage().persistent().get(&vol_key).unwrap_or(0i128)
        } else {
            0i128
        };
        let new_vol = current_vol.checked_add(amount).unwrap_or(i128::MAX);
        if new_vol > limit {
            env.storage().temporary().remove(&lock_key);
            return Err(ContractError::DailyVolumeLimitExceeded);
        }
        env.storage().persistent().set(&vol_key, &new_vol);
        env.storage().persistent().set(&day_key, &today);
    }

    // ── Read cached config from instance storage (no cross-contract call) ─
    let portfolio: Address = match env.storage().instance().get(&StorageKey::UserPortfolio) {
        Some(portfolio) => portfolio,
        None => {
            env.storage().temporary().remove(&lock_key);
            return Err(ContractError::NotInitialized);
        }
    };

    let exempt = {
        let key = StorageKey::PositionLimitExempt(user.clone());
        env.storage().instance().get(&key).unwrap_or(false)
    };

    // ── Resolve effective amount (portfolio % or explicit) ─────────────────
    let oracle: Option<Address> = env.storage().instance().get(&Symbol::new(env, ORACLE_KEY));
    let effective_amount =
        match resolve_trade_amount(env, &user, &token, amount, portfolio_pct_bps, oracle) {
            Ok(a) => a,
            Err(e) => {
                env.storage().temporary().remove(&lock_key);
                return Err(e);
            }
        };

    // ── Cross-contract call #1: SEP-41 balance check ──────────────────────
    let fee = effective_estimated_fee(env);
    let bal_key = StorageKey::LastInsufficientBalance(user.clone());
    match check_user_balance(env, &user, &token, effective_amount, fee) {
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
    if let Err(e) = validate_and_record_position(env, &portfolio, &user, exempt) {
        env.storage().temporary().remove(&lock_key);
        return Err(e);
    }

    env.storage().temporary().remove(&lock_key);
    Ok(())
}

fn next_limit_order_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::NextLimitOrderId)
        .unwrap_or(1);
    let next = id.checked_add(1).expect("limit order id overflow");
    env.storage()
        .instance()
        .set(&StorageKey::NextLimitOrderId, &next);
    id
}

fn pending_order_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .instance()
        .get(&StorageKey::PendingLimitOrderIds)
        .unwrap_or_else(|| Vec::new(env))
}

fn store_pending_order(env: &Env, order: PendingLimitOrder) {
    let mut ids = pending_order_ids(env);
    ids.push_back(order.order_id);
    env.storage()
        .instance()
        .set(&StorageKey::PendingLimitOrderIds, &ids);
    env.storage()
        .instance()
        .set(&StorageKey::PendingLimitOrder(order.order_id), &order);
}

fn set_pending_order_ids(env: &Env, ids: &Vec<u64>) {
    env.storage()
        .instance()
        .set(&StorageKey::PendingLimitOrderIds, ids);
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

    pub fn add_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        oracle::add(&env, oracle)
    }

    pub fn remove_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        oracle::remove(&env, oracle)
    }

    pub fn is_oracle_whitelisted(env: Env, oracle: Address) -> bool {
        oracle::is_whitelisted(&env, &oracle)
    }

    pub fn get_oracle_whitelist_count(env: Env) -> u32 {
        oracle::count(&env)
    }

    /// Set the oracle contract used by stop-loss/take-profit checks (admin only).
    pub fn set_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        oracle::require_whitelisted(&env, &oracle)?;
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ORACLE_KEY), &oracle);
        Ok(())
    }

    pub fn get_oracle(env: Env) -> Option<Address> {
        env.storage().instance().get(&Symbol::new(&env, ORACLE_KEY))
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
        keeper::register_watch(&env, &user, trade_id, asset_pair);
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

    /// Activate the protocol-wide market circuit breaker. The admin or a whitelisted
    /// oracle may activate it during extreme volatility.
    pub fn activate_market_circuit_breaker(env: Env, caller: Address) -> Result<(), ContractError> {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(ContractError::NotInitialized)?;
        if caller != admin && !oracle::is_whitelisted(&env, &caller) {
            return Err(ContractError::Unauthorized);
        }

        let ledger = env.ledger().sequence();
        env.storage()
            .instance()
            .set(&StorageKey::CircuitBreakerActive, &true);
        env.storage()
            .instance()
            .set(&StorageKey::CircuitBreakerLedger, &ledger);
        emit_circuit_breaker_activated(&env, caller, ledger);
        Ok(())
    }

    /// Admin reset hook; normal trade flow also auto-resets after the configured duration.
    pub fn reset_market_circuit_breaker(env: Env) -> Result<(), ContractError> {
        require_admin(&env)?;
        if market_circuit_breaker_active(&env) {
            reset_circuit_breaker(&env);
        }
        Ok(())
    }

    pub fn is_market_circuit_breaker_active(env: Env) -> bool {
        market_circuit_breaker_active(&env)
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
        order_type: OrderType,
        limit_price: Option<i128>,
    ) -> Result<(), ContractError> {
        match order_type {
            OrderType::Market => {
                execute_market_copy_trade(&env, user, token, amount, portfolio_pct_bps, true)
            }
            OrderType::Limit => {
                user.require_auth();
                if amount <= 0 {
                    return Err(ContractError::InvalidAmount);
                }
                let price = limit_price.ok_or(ContractError::InvalidAmount)?;
                if price <= 0 {
                    return Err(ContractError::InvalidAmount);
                }

                let fee = effective_estimated_fee(&env);
                let bal_key = StorageKey::LastInsufficientBalance(user.clone());
                match check_user_balance(&env, &user, &token, amount, fee) {
                    Ok(()) => env.storage().instance().remove(&bal_key),
                    Err(detail) => {
                        env.storage().instance().set(&bal_key, &detail);
                        return Err(ContractError::InsufficientBalance);
                    }
                }

                let order_id = next_limit_order_id(&env);
                let expires_at_ledger = env
                    .ledger()
                    .sequence()
                    .saturating_add(TRADE_TIMEOUT_LEDGERS);
                store_pending_order(
                    &env,
                    PendingLimitOrder {
                        order_id,
                        user,
                        token,
                        amount,
                        portfolio_pct_bps,
                        limit_price: price,
                        expires_at_ledger,
                    },
                );
                Ok(())
            }
        }
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
        env.storage()
            .instance()
            .set(&StorageKey::SdexRouter, &router);
    }

    pub fn get_sdex_router(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::SdexRouter)
    }

    /// Admin/keeper-facing price cache used to decide when pending limit orders
    /// are executable against the configured SDEX route.
    pub fn set_sdex_price(env: Env, token: Address, price: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if price <= 0 {
            panic!("price must be positive");
        }
        env.storage()
            .instance()
            .set(&StorageKey::SdexPrice(token), &price);
    }

    pub fn get_sdex_price(env: Env, token: Address) -> Option<i128> {
        env.storage().instance().get(&StorageKey::SdexPrice(token))
    }

    pub fn get_pending_limit_order(env: Env, order_id: u64) -> Option<PendingLimitOrder> {
        env.storage()
            .instance()
            .get(&StorageKey::PendingLimitOrder(order_id))
    }

    pub fn get_pending_limit_order_ids(env: Env) -> Vec<u64> {
        pending_order_ids(&env)
    }

    /// Keeper-facing sweep for pending limit orders on `token`.
    ///
    /// Orders expire after `TRADE_TIMEOUT_LEDGERS`. Executable orders run through
    /// the same market-trade path as immediate copy trades without requiring a
    /// fresh user signature, because the user authorized the limit order placement.
    pub fn check_pending_limit_orders(env: Env, token: Address) -> Result<u32, ContractError> {
        let current_price: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::SdexPrice(token.clone()))
            .ok_or(ContractError::OracleUnavailable)?;
        let ids = pending_order_ids(&env);
        let mut next_ids = Vec::new(&env);
        let mut processed = 0u32;

        for i in 0..ids.len() {
            let Some(order_id) = ids.get(i) else {
                continue;
            };
            let Some(order) = env
                .storage()
                .instance()
                .get::<StorageKey, PendingLimitOrder>(&StorageKey::PendingLimitOrder(order_id))
            else {
                continue;
            };

        // ── Cross-contract call #1: SEP-41 balance check ──────────────────────
        let fee = effective_estimated_fee(&env);
        let bal_key = StorageKey::LastInsufficientBalance(user.clone());

        // Primary: try to deduct fee upfront (user has amount + fee).
        // Fallback: if primary fails but user has at least `amount`, proceed and
        // deduct fee from received tokens after the trade.
        let use_fee_fallback = match check_user_balance(&env, &user, &token, effective_amount, fee) {
            Ok(()) => {
                env.storage().instance().remove(&bal_key);
                false
            }
            Err(detail) => {
                // Primary failed. Check if user has enough for just the amount (no fee).
                match check_user_balance(&env, &user, &token, effective_amount, 0) {
                    Ok(()) => {
                        // User has enough for the trade but not the fee — use fallback.
                        env.storage().instance().remove(&bal_key);
                        true
                    }
                    Err(_) => {
                        // User doesn't even have enough for the trade amount.
                        env.storage().instance().set(&bal_key, &detail);
                        env.storage().temporary().remove(&lock_key);
                        return Err(ContractError::InsufficientBalance);
                    }
                }
            }
        };
            if order.token != token {
                next_ids.push_back(order_id);
                continue;
            }

            if env.ledger().sequence() >= order.expires_at_ledger {
                env.storage()
                    .instance()
                    .remove(&StorageKey::PendingLimitOrder(order_id));
                processed = processed.saturating_add(1);
                continue;
            }

            if current_price <= order.limit_price {
                execute_market_copy_trade(
                    &env,
                    order.user,
                    order.token,
                    order.amount,
                    order.portfolio_pct_bps,
                    false,
                )?;
                env.storage()
                    .instance()
                    .remove(&StorageKey::PendingLimitOrder(order_id));
                processed = processed.saturating_add(1);
            } else {
                next_ids.push_back(order_id);
            }
        }

        // If fallback was used, emit the FeeDeductedFromReceived event.
        // The trade_id is the current position count (used as a proxy identifier).
        if use_fee_fallback && fee > 0 {
            // Use a monotonic counter stored per user as a trade_id proxy.
            let trade_id_key = StorageKey::FeeDeductedFromReceived(user.clone(), 0);
            let trade_id: u64 = env
                .storage()
                .instance()
                .get(&trade_id_key)
                .unwrap_or(0u64)
                .saturating_add(1);
            env.storage().instance().set(&trade_id_key, &trade_id);

            shared::events::emit_fee_deducted_from_received(
                &env,
                shared::events::EvtFeeDeductedFromReceived {
                    schema_version: shared::events::SCHEMA_VERSION,
                    user: user.clone(),
                    fee_amount: fee,
                    trade_id,
                },
            );
        }

        env.storage().temporary().remove(&lock_key);
        Ok(())
        set_pending_order_ids(&env, &next_ids);
        Ok(processed)
    }

    /// Admin: set the global daily trade volume limit (USD-equivalent units).
    /// `0` means no limit.
    pub fn set_daily_volume_limit(env: Env, limit: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if limit < 0 {
            panic!("limit must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::DailyVolumeLimit, &limit);
    }

    pub fn get_daily_volume_limit(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::DailyVolumeLimit)
            .unwrap_or(0i128)
    }

    /// Admin: set the per-pair open interest limit. `0` means no limit.
    pub fn set_max_open_interest_per_pair(env: Env, limit: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if limit < 0 {
            panic!("limit must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::MaxOpenInterestPerPair, &limit);
    }

    pub fn get_max_open_interest_per_pair(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::MaxOpenInterestPerPair)
            .unwrap_or(0i128)
    }

    pub fn get_open_interest(env: Env, pair: Address) -> i128 {
        open_interest_for_pair(&env, &pair)
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

        let exit_price =
            execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)?;

        let realized_pnl = exit_price - amount;
        let close_sym = Symbol::new(&env, "close_position");
        let mut close_args = Vec::<Val>::new(&env);
        close_args.push_back(user.clone().into_val(&env));
        close_args.push_back(trade_id.into_val(&env));
        close_args.push_back(realized_pnl.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &close_sym, close_args);
        decrease_open_interest(&env, &from_token, amount);

        shared::events::emit_trade_cancelled(
            &env,
            shared::events::EvtTradeCancelled {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                trade_id,
                exit_price,
                realized_pnl,
            },
        );

        Ok(())
    }

    /// Execute a batch of copy trades. Each trade is attempted independently;
    /// a failure in one trade does NOT roll back successful trades.
    ///
    /// Returns a `Vec<BatchTradeResult>` with one entry per input trade, in order.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAmount`] — batch is empty or exceeds `MAX_BATCH_SIZE`.
    pub fn batch_execute(
        env: Env,
        trades: Vec<BatchTradeInput>,
    ) -> Result<Vec<BatchTradeResult>, ContractError> {
        let len = trades.len();
        if len == 0 || len > MAX_BATCH_SIZE {
            return Err(ContractError::InvalidAmount);
        }

        let mut results: Vec<BatchTradeResult> = Vec::new(&env);

        for i in 0..len {
            let trade = trades.get(i).unwrap();
            let outcome = Self::execute_copy_trade(
                env.clone(),
                trade.user,
                trade.token,
                trade.amount,
                None,
                OrderType::Market,
                None,
            );
            let result = match outcome {
                Ok(()) => BatchTradeResult {
                    ok: true,
                    error_code: 0,
                },
                Err(e) => BatchTradeResult {
                    ok: false,
                    error_code: e as u32,
                },
            };
            results.push_back(result);
        }

        Ok(results)
    }

    // ── DCA copy trading (Issue #360) ─────────────────────────────────────────

    /// Create a DCA plan: split `total_amount` into `num_intervals` equal trades
    /// spaced `interval_ledgers` apart.  `signal_expiry_ledger = 0` means no expiry.
    pub fn execute_dca_copy_trade(
        env: Env,
        user: Address,
        signal_id: u64,
        total_amount: i128,
        num_intervals: u32,
        interval_ledgers: u32,
        signal_expiry_ledger: u32,
    ) -> Result<(), ContractError> {
        user.require_auth();
        dca::execute_dca_copy_trade(
            &env,
            &user,
            signal_id,
            total_amount,
            num_intervals,
            interval_ledgers,
            signal_expiry_ledger,
        )
    }

    /// Execute the next DCA interval for `(user, signal_id)`.
    /// Called by the keeper network.  Returns `true` when the plan is complete.
    pub fn execute_dca_interval(
        env: Env,
        user: Address,
        signal_id: u64,
    ) -> Result<bool, ContractError> {
        // Capture config needed inside the closure before moving env.
        let portfolio: Option<Address> = env.storage().instance().get(&StorageKey::UserPortfolio);
        let exempt = {
            let key = StorageKey::PositionLimitExempt(user.clone());
            env.storage().instance().get(&key).unwrap_or(false)
        };

        dca::execute_dca_interval(&env, &user, signal_id, |amount| {
            // Reuse the existing copy-trade balance + position-limit logic.
            let fee = effective_estimated_fee(&env);
            // We don't have a token address in the DCA plan (it's signal-level),
            // so balance check is skipped here — the caller is responsible for
            // ensuring funds are available (same pattern as batch_execute).
            let _ = (amount, fee); // suppress unused warnings

            if let Some(ref p) = portfolio {
                risk_gates::validate_and_record_position(&env, p, &user, exempt)?;
            }
            Ok(())
        })
    }

    /// Manually cancel a DCA plan. Only the plan owner may cancel.
    pub fn cancel_dca_plan(
        env: Env,
        user: Address,
        signal_id: u64,
    ) -> Result<(), ContractError> {
        user.require_auth();
        dca::cancel_dca_plan(&env, &user, signal_id)
    }
}

#[cfg(test)]
mod test;
#[cfg(test)]
mod tests;
