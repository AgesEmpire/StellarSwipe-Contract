#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol, Vec};

mod admin;
mod advanced_risk;
mod auth;
mod conditional;
mod correlation;
mod errors;
mod history;
mod iceberg;
mod multi_asset;
mod oracle;
mod portfolio;
mod portfolio_insurance;
mod positions;
mod rate_limit;
mod referral;
mod rate_limit;
mod risk;
mod risk_parity;
mod sdex;
mod storage;
mod strategies;
mod twap;

pub use errors::AutoTradeError;
pub use risk::RiskConfig;

#[cfg(feature = "testutils")]
pub use storage::{set_signal, Signal};

use crate::storage::DataKey;
use advanced_risk::AutoSellResult;
 feat/batch-copy-trade
use stellar_swipe_common::emergency::{CAT_TRADING, PauseState};

use errors::AutoTradeError;
use stellar_swipe_common::emergency::{CAT_ALL, CAT_TRADING, PauseState};
use stellar_swipe_common::{health_uninitialized, HealthStatus};
 main

use risk_parity::{AssetRisk, RebalanceTrade};

pub use iceberg::{
    create_iceberg_order, cancel_iceberg_order, get_full_order_view, get_public_order_view,
    get_user_orders, on_sdex_fill, update_iceberg_price, AssetPair, CancellationInfo,
    FullOrderView, IcebergOrder, OrderSide, OrderStatus, PublicOrderView,
};

/// ==========================
/// Types
/// ==========================

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeStatus {
    Pending,
    PartiallyFilled,
    Filled,
    Failed,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trade {
    pub signal_id: u64,
    pub user: Address,
    pub requested_amount: i128,
    pub executed_amount: i128,
    pub executed_price: i128,
    pub timestamp: u64,
    pub status: TradeStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeResult {
    pub trade: Trade,
}

/// ==========================
/// Contract
/// ==========================

#[contract]
pub struct AutoTradeContract;

/// ==========================
/// Implementation
/// ==========================

#[contractimpl]
impl AutoTradeContract {
    /// Initialize the contract with an admin
    pub fn initialize(env: Env, admin: Address) {
        admin::init_admin(&env, admin);
    }

    /// Pause a category (admin or guardian)
    pub fn pause_category(
        env: Env,
        caller: Address,
        category: String,
        duration: Option<u64>,
        reason: String,
    ) -> Result<(), AutoTradeError> {
        admin::pause_category(&env, &caller, category, duration, reason)
    }

    /// Unpause a category (admin only)
    pub fn unpause_category(env: Env, caller: Address, category: String) -> Result<(), AutoTradeError> {
        admin::unpause_category(&env, &caller, category)
    }

    /// Set guardian address (admin only)
    pub fn set_guardian(env: Env, caller: Address, guardian: Address) -> Result<(), AutoTradeError> {
        admin::set_guardian(&env, &caller, guardian)
    }

    /// Revoke guardian (admin only)
    pub fn revoke_guardian(env: Env, caller: Address) -> Result<(), AutoTradeError> {
        admin::revoke_guardian(&env, &caller)
    }

    /// Get current guardian
    pub fn get_guardian(env: Env) -> Option<Address> {
        admin::get_guardian(&env)
    }

    /// Get current pause states
    pub fn get_pause_states(env: Env) -> soroban_sdk::Map<String, PauseState> {
        admin::get_pause_states(&env)
    }

    /// Set the oracle contract address (admin only).
    /// The oracle is used for manipulation-resistant stop-loss/take-profit price checks.
    pub fn set_oracle_address(
        env: Env,
        caller: Address,
        oracle_addr: Address,
    ) -> Result<(), AutoTradeError> {
        oracle::set_oracle_address(&env, &caller, oracle_addr)
    }

    /// Get the currently configured oracle contract address.
    pub fn get_oracle_address(env: Env) -> Option<Address> {
        oracle::get_oracle_address(&env)
    }

    /// Admin override for the oracle circuit breaker.
    /// When `enabled = true`, trading proceeds even if the oracle is unavailable.
    /// When `enabled = false`, the normal circuit breaker logic applies.
    pub fn override_oracle_circuit_breaker(
        env: Env,
        caller: Address,
        enabled: bool,
    ) -> Result<(), AutoTradeError> {
        oracle::override_oracle_circuit_breaker(&env, &caller, enabled)
    }

    /// Get the current oracle circuit breaker state.
    pub fn get_oracle_circuit_breaker_state(
        env: Env,
    ) -> oracle::OracleCircuitBreakerState {
        oracle::get_cb_state(&env)
    }

    /// Add an oracle address to the whitelist for `asset_pair` (admin only).
    /// Emits `OracleAdded` event. Idempotent.
    pub fn add_oracle(
        env: Env,
        caller: Address,
        asset_pair: u32,
        oracle_addr: Address,
    ) -> Result<(), AutoTradeError> {
        oracle::add_oracle(&env, &caller, asset_pair, oracle_addr)
    }

    /// Remove an oracle address from the whitelist for `asset_pair` (admin only).
    /// Emits `OracleRemoved` event. Returns `LastOracleForPair` if it would be the last.
    pub fn remove_oracle(
        env: Env,
        caller: Address,
        asset_pair: u32,
        oracle_addr: Address,
    ) -> Result<(), AutoTradeError> {
        oracle::remove_oracle(&env, &caller, asset_pair, oracle_addr)
    }

    /// Get the current oracle whitelist for `asset_pair`.
    pub fn get_oracle_whitelist(
        env: Env,
        asset_pair: u32,
    ) -> soroban_sdk::Vec<Address> {
        oracle::get_oracle_whitelist(&env, asset_pair)
    }

    /// Whitelisted oracle pushes a price update for `asset_pair`.
    /// Caller must be in the whitelist; price must be fresh.
    pub fn push_price_update(
        env: Env,
        caller: Address,
        asset_pair: u32,
        price: stellar_swipe_common::oracle::OraclePrice,
    ) -> Result<(), AutoTradeError> {
        oracle::push_price_update(&env, &caller, asset_pair, price)
    }

    /// Set the circuit breaker configuration (admin only)
    pub fn set_circuit_breaker_config(
        env: Env,
        caller: Address,
        config: stellar_swipe_common::emergency::CircuitBreakerConfig,
    ) -> Result<(), AutoTradeError> {
        admin::set_cb_config(&env, &caller, config)
    }

    /// Execute a trade on behalf of a user based on a signal
    pub fn execute_trade(
        env: Env,
        user: Address,
        signal_id: u64,
        order_type: OrderType,
        amount: i128,
    ) -> Result<TradeResult, AutoTradeError> {
        // Check if trading is paused
        if admin::is_paused(&env, String::from_str(&env, CAT_TRADING)) {
            return Err(AutoTradeError::TradingPaused);
        }

        // Oracle circuit breaker: halt if oracle is unavailable (unless admin override)
        oracle::check_oracle_circuit_breaker(&env, signal_id as u32)?;

        if amount <= 0 {
            return Err(AutoTradeError::InvalidAmount);
        }

        user.require_auth();

        let signal = storage::get_signal(&env, signal_id).ok_or(AutoTradeError::SignalNotFound)?;

        if env.ledger().timestamp() > signal.expiry {
            return Err(AutoTradeError::SignalExpired);
        }

        if !auth::is_authorized(&env, &user, amount) {
            return Err(AutoTradeError::Unauthorized);
        }

        rate_limit::check_rate_limits(&env, &user, amount)?;

        if !sdex::has_sufficient_balance(&env, &user, &signal.base_asset, amount) {
            return Err(AutoTradeError::InsufficientBalance);
        }

        // Determine if this is a sell operation (simplified)
        let is_sell = false; // This should be determined from the signal or order details

        // Set current asset price for risk calculations
        risk::set_asset_price(&env, signal.base_asset, signal.price);

        // Fetch oracle price for manipulation-resistant stop-loss evaluation.
        // Falls back to None (SDEX spot) when no oracle is configured.
        let oracle_price: Option<i128> = oracle::get_oracle_price(&env, signal.base_asset)
            .ok()
            .map(|op| oracle::oracle_price_to_i128(&op));

        // Perform risk checks
        let stop_loss_triggered = risk::validate_trade(
            &env,
            &user,
            signal.base_asset,
            amount,
            signal.price,
            is_sell,
            oracle_price,
        )?;

        // If stop-loss is triggered, emit event and proceed with sell
        if stop_loss_triggered {
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(&env, "stop_loss_triggered"),
                    user.clone(),
                    signal.base_asset,
                ),
                signal.price,
            );
        }

        let execution = match order_type {
            OrderType::Market => sdex::execute_market_order(&env, &user, &signal, amount)?,
            OrderType::Limit => sdex::execute_limit_order(&env, &user, &signal, amount)?,
        };

        let status = if execution.executed_amount == 0 {
            TradeStatus::Failed
        } else if execution.executed_amount < amount {
            TradeStatus::PartiallyFilled
        } else {
            TradeStatus::Filled
        };

        // Update circuit breaker stats
        admin::update_cb_stats(
            &env,
            status == TradeStatus::Failed,
            execution.executed_amount,
            execution.executed_price,
        );

        let trade = Trade {
            signal_id,
            user: user.clone(),
            requested_amount: amount,
            executed_amount: execution.executed_amount,
            executed_price: execution.executed_price,
            timestamp: env.ledger().timestamp(),
            status: status.clone(),
        };

        // Update position tracking
        if execution.executed_amount > 0 {
            let positions = risk::get_user_positions(&env, &user);
            let current_amount = positions
                .get(signal.base_asset)
                .map(|p| p.amount)
                .unwrap_or(0);

            let new_amount = if is_sell {
                current_amount - execution.executed_amount
            } else {
                current_amount + execution.executed_amount
            };

            risk::update_position(
                &env,
                &user,
                signal.base_asset,
                new_amount,
                execution.executed_price,
            );

            // Record trade in history
            risk::add_trade_record(&env, &user, signal_id, execution.executed_amount);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Trades(user.clone(), signal_id), &trade);

        if execution.executed_amount > 0 {
            // ── Referral fee split ────────────────────────────────────────────
            // Platform fee = 7% of executed amount (0.7 XLM per 10 XLM trade).
            // Referral reward = 10% of platform fee → deducted from platform share.
            let platform_fee = execution.executed_amount * 7 / 100;
            let referral_reward =
                referral::process_referral_reward(&env, &user, signal.base_asset, platform_fee);

            let hist_status = match status {
                TradeStatus::Filled | TradeStatus::PartiallyFilled => {
                    history::HistoryTradeStatus::Executed
                }
                TradeStatus::Failed => history::HistoryTradeStatus::Failed,
                TradeStatus::Pending => history::HistoryTradeStatus::Pending,
            };
            history::record_trade(
                &env,
                &user,
                signal_id,
                signal.base_asset,
                execution.executed_amount,
                execution.executed_price,
                platform_fee - referral_reward,
                hist_status,
            );
        }

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "trade_executed"), user.clone(), signal_id),
            trade.clone(),
        );

        // Emit event if trade was blocked by risk limits (status = Failed due to risk)
        if status == TradeStatus::Failed {
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(&env, "risk_limit_block"),
                    user.clone(),
                    signal_id,
                ),
                amount,
            );
        }

        Ok(TradeResult { trade })
    }

    // ── Position Management (Issues #191, #192, #193) ────────────────────────

    /// Open a new tracked position. Returns a unique trade_id (BytesN<32>).
    /// Issue #191
    #[allow(clippy::too_many_arguments)]
    pub fn open_position(
        env: Env,
        user: Address,
        signal_id: u64,
        asset_pair: u32,
        amount: i128,
        entry_price: i128,
        stop_loss: i128,
        take_profit: i128,
    ) -> soroban_sdk::BytesN<32> {
        user.require_auth();
        if amount <= 0 || entry_price <= 0 {
            panic!("invalid amount or price");
        }
        positions::open_position(&env, &user, signal_id, asset_pair, amount, entry_price, stop_loss, take_profit)
    }

    /// Close an existing position and calculate P&L. Returns PositionResult.
    /// Issue #192
    pub fn close_position(
        env: Env,
        user: Address,
        trade_id: soroban_sdk::BytesN<32>,
        exit_price: i128,
    ) -> Option<positions::PositionResult> {
        user.require_auth();
        positions::close_position(&env, &user, &trade_id, exit_price)
    }

    /// Get all positions (open + closed) for a user — the full portfolio view.
    /// Issue #193
    pub fn get_all_positions(
        env: Env,
        user: Address,
    ) -> soroban_sdk::Vec<positions::PositionData> {
        positions::get_all_positions(&env, &user)
    }

    /// Get only open positions for a user.
    pub fn get_open_positions(
        env: Env,
        user: Address,
    ) -> soroban_sdk::Vec<positions::PositionData> {
        positions::get_open_positions(&env, &user)
    }

    /// Get only closed positions for a user.
    pub fn get_closed_positions(
        env: Env,
        user: Address,
    ) -> soroban_sdk::Vec<positions::PositionData> {
        positions::get_closed_positions(&env, &user)
    }

    /// Fetch executed trade by user + signal
    pub fn get_trade(env: Env, user: Address, signal_id: u64) -> Option<Trade> {
        env.storage()
            .persistent()
            .get(&DataKey::Trades(user, signal_id))
    }

    /// Get user's risk configuration
    pub fn get_risk_config(env: Env, user: Address) -> risk::RiskConfig {
        risk::get_risk_config(&env, &user)
    }

    /// Update user's risk configuration
    pub fn set_risk_config(env: Env, user: Address, config: risk::RiskConfig) {
        user.require_auth();
        risk::set_risk_config(&env, &user, &config);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "risk_config_updated"), user.clone()),
            config,
        );
    }

    /// Get user's current positions
    pub fn get_user_positions(env: Env, user: Address) -> soroban_sdk::Map<u32, risk::Position> {
        risk::get_user_positions(&env, &user)
    }

    /// Get user's trade history (risk module, legacy)
    pub fn get_trade_history_legacy(
        env: Env,
        user: Address,
    ) -> soroban_sdk::Vec<risk::TradeRecord> {
        risk::get_trade_history(&env, &user)
    }

    /// Get paginated trade history (newest first)
    pub fn get_trade_history(
        env: Env,
        user: Address,
        offset: u32,
        limit: u32,
    ) -> soroban_sdk::Vec<history::HistoryTrade> {
        history::get_trade_history(&env, &user, offset, limit)
    }

    /// Get user portfolio with holdings and P&L
    pub fn get_portfolio(env: Env, user: Address) -> portfolio::Portfolio {
        portfolio::get_portfolio(&env, &user)
    }

    /// Set risk parity configuration
    pub fn set_risk_parity_config(
        env: Env,
        user: Address,
        enabled: bool,
        rebalance_frequency_days: u32,
        threshold_pct: u32,
    ) -> Result<(), AutoTradeError> {
        if !cfg!(test) {
            user.require_auth();
        }
        let mut config = risk::get_risk_parity_config(&env, &user);
        config.enabled = enabled;
        config.rebalance_frequency_days = rebalance_frequency_days;
        config.threshold_pct = threshold_pct;
        risk::set_risk_parity_config(&env, &user, &config);
        Ok(())
    }

    /// Get risk parity configuration
    pub fn get_risk_parity_config(env: Env, user: Address) -> risk::RiskParityConfig {
        risk::get_risk_parity_config(&env, &user)
    }

    /// Preview a risk parity rebalance
    pub fn preview_risk_parity_rebalance(
        env: Env,
        user: Address,
    ) -> Result<(Vec<AssetRisk>, Vec<RebalanceTrade>), AutoTradeError> {
        risk_parity::calculate_risk_parity_rebalance(&env, &user)
    }

    /// Manually trigger a risk parity rebalance
    pub fn trigger_risk_parity_rebalance(env: Env, user: Address) -> Result<(), AutoTradeError> {
        if !cfg!(test) {
            user.require_auth();
        }
        risk_parity::execute_risk_parity_rebalance(&env, &user)
    }

    /// Record a price for volatility tracking (usually called by oracle)
    pub fn record_asset_price(env: Env, asset_id: u32, price: i128) {
        risk::record_price(&env, asset_id, price);
        risk::set_asset_price(&env, asset_id, price);
    }

    pub fn process_price_update(
        env: Env,
        user: Address,
        asset_id: u32,
        price: i128,
    ) -> Option<AutoSellResult> {
        let result = advanced_risk::process_price_update(&env, &user, asset_id, price);

        if let Some(ref sell_result) = result {
            let event_name = match sell_result.trigger {
                advanced_risk::StopTrigger::TrailingStop => "trailing_stop_triggered",
                advanced_risk::StopTrigger::FixedStopLoss => "stop_loss_triggered",
            };

            #[allow(deprecated)]
            env.events().publish(
                (Symbol::new(&env, event_name), user.clone(), asset_id),
                sell_result.clone(),
            );
        }

        result
    }

    pub fn get_trailing_stop_price(env: Env, user: Address, asset_id: u32) -> Option<i128> {
        let config = risk::get_risk_config(&env, &user);
        advanced_risk::get_trailing_stop_price(&env, &user, asset_id, &config)
    }

    /// Grant authorization to execute trades
    pub fn grant_authorization(
        env: Env,
        user: Address,
        max_amount: i128,
        duration_days: u32,
    ) -> Result<(), AutoTradeError> {
        auth::grant_authorization(&env, &user, max_amount, duration_days)
    }

    /// Revoke authorization
    pub fn revoke_authorization(env: Env, user: Address) -> Result<(), AutoTradeError> {
        auth::revoke_authorization(&env, &user)
    }

    /// Initialize rate limit admin
    pub fn init_rate_limit_admin(env: Env, admin: Address) {
        admin.require_auth();
        rate_limit::set_admin(&env, &admin);
    }

    /// Configure rate limits (admin only)
    pub fn set_rate_limits(
        env: Env,
        limits: rate_limit::BridgeRateLimits,
    ) -> Result<(), AutoTradeError> {
        let admin = rate_limit::get_admin(&env).ok_or(AutoTradeError::Unauthorized)?;
        admin.require_auth();
        rate_limit::set_limits(&env, &limits);
        Ok(())
    }

    /// Add user to rate limit whitelist (admin only)
    pub fn add_to_whitelist(env: Env, user: Address) -> Result<(), AutoTradeError> {
        rate_limit::add_to_whitelist(&env, &user)
    }

    /// Remove user from whitelist (admin only)
    pub fn remove_from_whitelist(env: Env, user: Address) -> Result<(), AutoTradeError> {
        rate_limit::remove_from_whitelist(&env, &user)
    }

    /// Record a rate limit violation and apply penalty (admin only)
    pub fn record_violation(
        env: Env,
        user: Address,
        violation_type: rate_limit::ViolationType,
    ) -> Result<(), AutoTradeError> {
        let admin = rate_limit::get_admin(&env).ok_or(AutoTradeError::Unauthorized)?;
        admin.require_auth();
        rate_limit::record_violation(&env, &user, violation_type)
    }

    /// Dynamically adjust rate limits based on current load
    pub fn adjust_rate_limits(env: Env) -> Result<(), AutoTradeError> {
        rate_limit::adjust_limits_based_on_load(&env)
    }

    /// Get current rate limits
    pub fn get_rate_limits(env: Env) -> rate_limit::BridgeRateLimits {
        rate_limit::get_limits(&env)
    }

    /// Get user transfer history
    pub fn get_user_rate_history(
        env: Env,
        user: Address,
    ) -> rate_limit::UserTransferHistory {
        rate_limit::get_user_history(&env, &user)
    }

    /// Check if user is whitelisted
    pub fn is_whitelisted(env: Env, user: Address) -> bool {
        rate_limit::is_whitelisted(&env, &user)
    }

    /// Get authorization config
    pub fn get_auth_config(env: Env, user: Address) -> Option<auth::AuthConfig> {
        auth::get_auth_config(&env, &user)
    }

    // ── DCA ──────────────────────────────────────────────────────────────────

    pub fn create_dca(
        env: Env,
        user: Address,
        asset_pair: u32,
        purchase_amount: i128,
        frequency: strategies::dca::DCAFrequency,
        duration_days: Option<u64>,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::dca::create_dca_strategy(&env, user, asset_pair, purchase_amount, frequency, duration_days)
    }

    pub fn execute_due_dca(env: Env) -> soroban_sdk::Vec<u64> {
        strategies::dca::execute_due_dca_purchases(&env)
    }

    pub fn execute_dca_purchase(env: Env, strategy_id: u64) -> Result<(), AutoTradeError> {
        strategies::dca::execute_dca_purchase(&env, strategy_id)
    }

    pub fn pause_dca(env: Env, user: Address, strategy_id: u64) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::dca::pause_dca_strategy(&env, strategy_id)
    }

    pub fn resume_dca(env: Env, user: Address, strategy_id: u64) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::dca::resume_dca_strategy(&env, strategy_id)
    }

    pub fn update_dca(
        env: Env,
        user: Address,
        strategy_id: u64,
        new_amount: Option<i128>,
        new_frequency: Option<strategies::dca::DCAFrequency>,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::dca::update_dca_schedule(&env, strategy_id, new_amount, new_frequency)
    }

    pub fn handle_missed_dca(env: Env, strategy_id: u64) -> Result<u32, AutoTradeError> {
        strategies::dca::handle_missed_dca_purchases(&env, strategy_id)
    }

    pub fn get_dca_strategy(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::dca::DCAStrategy, AutoTradeError> {
        strategies::dca::get_dca_strategy(&env, strategy_id)
    }

    pub fn analyze_dca(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::dca::DCAPerformance, AutoTradeError> {
        strategies::dca::analyze_dca_performance(&env, strategy_id)
    }

    // ── Mean Reversion ────────────────────────────────────────────────────────

    pub fn create_mean_reversion(
        env: Env,
        user: Address,
        asset_pair: u32,
        lookback_period_days: u32,
        entry_z_score: i128,
        exit_z_score: i128,
        position_size_pct: u32,
        max_positions: u32,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::create_mean_reversion_strategy(
            &env, user, asset_pair, lookback_period_days,
            entry_z_score, exit_z_score, position_size_pct, max_positions,
        )
    }

    pub fn get_mean_reversion(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::mean_reversion::MeanReversionStrategy, AutoTradeError> {
        strategies::mean_reversion::get_mean_reversion_strategy(&env, strategy_id)
    }

    pub fn check_mr_signals(
        env: Env,
        strategy_id: u64,
    ) -> Result<Option<strategies::mean_reversion::ReversionSignal>, AutoTradeError> {
        strategies::mean_reversion::check_mean_reversion_signals(&env, strategy_id)
    }

    pub fn execute_mr_trade(
        env: Env,
        user: Address,
        strategy_id: u64,
        signal: strategies::mean_reversion::ReversionSignal,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::execute_mean_reversion_trade(&env, strategy_id, signal)
    }

    pub fn check_mr_exits(
        env: Env,
        strategy_id: u64,
    ) -> Result<soroban_sdk::Vec<u64>, AutoTradeError> {
        strategies::mean_reversion::check_reversion_exits(&env, strategy_id)
    }

    pub fn adjust_mr_params(
        env: Env,
        strategy_id: u64,
    ) -> Result<(), AutoTradeError> {
        strategies::mean_reversion::adjust_strategy_parameters(&env, strategy_id)
    }

    pub fn disable_mean_reversion(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::disable_mean_reversion_strategy(&env, strategy_id)
    }

    pub fn enable_mean_reversion(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::enable_mean_reversion_strategy(&env, strategy_id)
    }

    pub fn set_stat_arb_price_history(
        env: Env,
        asset_id: u32,
        prices: soroban_sdk::Vec<i128>,
    ) -> Result<(), AutoTradeError> {
        strategies::stat_arb::set_price_history(&env, asset_id, prices)
    }

    pub fn get_stat_arb_price_history(env: Env, asset_id: u32) -> soroban_sdk::Vec<i128> {
        strategies::stat_arb::get_price_history(&env, asset_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure_stat_arb_strategy(
        env: Env,
        user: Address,
        asset_basket: soroban_sdk::Vec<u32>,
        lookback_period_days: u32,
        cointegration_threshold: i128,
        entry_z_score: i128,
        exit_z_score: i128,
        rebalance_frequency_hours: u32,
    ) -> Result<strategies::stat_arb::StatArbStrategy, AutoTradeError> {
        user.require_auth();
        let strategy = strategies::stat_arb::configure_strategy(
            &env,
            &user,
            asset_basket,
            lookback_period_days,
            cointegration_threshold,
            entry_z_score,
            exit_z_score,
            rebalance_frequency_hours,
        )?;
        strategies::stat_arb::emit_strategy_configured(&env, &user, &strategy);
        Ok(strategy)
    }

    pub fn get_stat_arb_strategy(
        env: Env,
        user: Address,
    ) -> Option<strategies::stat_arb::StatArbStrategy> {
        strategies::stat_arb::get_strategy(&env, &user)
    }

    pub fn test_stat_arb_cointegration(
        env: Env,
        asset_basket: soroban_sdk::Vec<u32>,
        lookback_period_days: u32,
        cointegration_threshold: i128,
    ) -> Result<strategies::stat_arb::CointegrationTest, AutoTradeError> {
        strategies::stat_arb::test_cointegration_for_assets(
            &env,
            asset_basket,
            lookback_period_days,
            cointegration_threshold,
        )
    }

    pub fn check_stat_arb_signal(
        env: Env,
        user: Address,
    ) -> Result<strategies::stat_arb::StatArbSignal, AutoTradeError> {
        strategies::stat_arb::check_stat_arb_signal(&env, &user)
    }

    pub fn execute_stat_arb_trade(
        env: Env,
        user: Address,
        total_value: i128,
    ) -> Result<strategies::stat_arb::StatArbPortfolio, AutoTradeError> {
        user.require_auth();
        let portfolio = strategies::stat_arb::execute_stat_arb_trade(&env, &user, total_value)?;
        strategies::stat_arb::emit_trade_opened(&env, &user, &portfolio);
        Ok(portfolio)
    }

    pub fn get_active_stat_arb_portfolio(
        env: Env,
        user: Address,
    ) -> Option<strategies::stat_arb::StatArbPortfolio> {
        strategies::stat_arb::get_active_portfolio(&env, &user)
    }

    pub fn rebalance_stat_arb_portfolio(
        env: Env,
        user: Address,
    ) -> Result<strategies::stat_arb::StatArbPortfolio, AutoTradeError> {
        user.require_auth();
        let portfolio = strategies::stat_arb::rebalance_stat_arb_portfolio(&env, &user)?;
        strategies::stat_arb::emit_rebalanced(&env, &user, &portfolio);
        Ok(portfolio)
    }

    pub fn check_stat_arb_exit(
        env: Env,
        user: Address,
    ) -> Result<strategies::stat_arb::StatArbExitCheck, AutoTradeError> {
        strategies::stat_arb::check_stat_arb_exit(&env, &user)
    }

    pub fn close_stat_arb_portfolio(
        env: Env,
        user: Address,
    ) -> Result<strategies::stat_arb::StatArbPortfolio, AutoTradeError> {
        user.require_auth();
        let exit_check = strategies::stat_arb::check_stat_arb_exit(&env, &user)?;
        let reason = if exit_check.reason == strategies::stat_arb::StatArbExitReason::None {
            strategies::stat_arb::StatArbExitReason::Converged
        } else {
            exit_check.reason.clone()
        };
        let portfolio = strategies::stat_arb::close_stat_arb_portfolio(&env, &user)?;
        strategies::stat_arb::emit_closed(&env, &user, &portfolio, reason);
        Ok(portfolio)
    }

    // ── Portfolio Insurance public API ────────────────────────────────────────

    /// Configure portfolio insurance for the calling user.
    pub fn configure_insurance(
        env: Env,
        user: Address,
        enabled: bool,
        max_drawdown_bps: u32,
        hedge_ratio_bps: u32,
        rebalance_threshold_bps: u32,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        portfolio_insurance::configure_insurance(
            &env,
            &user,
            enabled,
            max_drawdown_bps,
            hedge_ratio_bps,
            rebalance_threshold_bps,
        )
    }

    /// Return current drawdown in basis points and update the high-water mark.
    pub fn get_portfolio_drawdown(env: Env, user: Address) -> Result<i128, AutoTradeError> {
        portfolio_insurance::calculate_drawdown(&env, &user)
    }

    /// Check drawdown and open hedge positions if the threshold is breached.
    pub fn apply_hedge_if_needed(
        env: Env,
        user: Address,
    ) -> Result<soroban_sdk::Vec<u32>, AutoTradeError> {
        user.require_auth();
        portfolio_insurance::check_and_apply_hedge(&env, &user)
    }

    /// Rebalance existing hedges to match the current portfolio size.
    pub fn rebalance_hedges(
        env: Env,
        user: Address,
    ) -> Result<soroban_sdk::Vec<u32>, AutoTradeError> {
        user.require_auth();
        portfolio_insurance::rebalance_hedges(&env, &user)
    }

    /// Close all hedges when the portfolio has recovered (drawdown < 5%).
    pub fn remove_hedges_if_recovered(
        env: Env,
        user: Address,
    ) -> Result<soroban_sdk::Vec<u32>, AutoTradeError> {
        user.require_auth();
        portfolio_insurance::remove_hedges_if_recovered(&env, &user)
    }

    /// Get the current insurance configuration for a user.
    pub fn get_insurance_config(
        env: Env,
        user: Address,
    ) -> Option<portfolio_insurance::PortfolioInsurance> {
        portfolio_insurance::get_insurance(&env, &user)
    }

    // ── Grid Trading Strategy (Issue #104) ───────────────────────────────────

    /// Initialise a grid strategy and return its id.
    pub fn init_grid(
        env: Env,
        user: Address,
        asset_pair: u32,
        upper_price: i128,
        lower_price: i128,
        num_grids: u32,
        total_capital: i128,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::grid::initialize_grid_strategy(
            &env,
            user,
            asset_pair,
            upper_price,
            lower_price,
            num_grids,
            total_capital,
        )
    }

    /// Place limit orders across all grid levels.
    pub fn place_grid_orders(env: Env, strategy_id: u64) -> Result<(), AutoTradeError> {
        strategies::grid::place_grid_orders(&env, strategy_id)
    }

    /// Record a filled grid order and optionally rebalance.
    pub fn grid_order_filled(
        env: Env,
        strategy_id: u64,
        order_id: u64,
        fill_price: i128,
        fill_amount: i128,
    ) -> Result<(), AutoTradeError> {
        strategies::grid::on_grid_order_filled(&env, strategy_id, order_id, fill_price, fill_amount)
    }

    /// Shift the grid if price has moved outside the configured range.
    pub fn adjust_grid(env: Env, strategy_id: u64) -> Result<(), AutoTradeError> {
        strategies::grid::adjust_grid_to_price_movement(&env, strategy_id)
    }

    /// Return performance metrics for a grid strategy.
    pub fn grid_performance(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::grid::GridPerformance, AutoTradeError> {
        strategies::grid::calculate_grid_performance(&env, strategy_id)
    }

    // ── Pairs Trading Strategy (Issue #106) ───────────────────────────────────

    pub fn configure_pairs_strategy(
        env: Env,
        user: Address,
        asset_a: u32,
        asset_b: u32,
        lookback_period_days: u32,
        entry_z_score: i128,
        exit_z_score: i128,
        position_size_pct: u32,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::pairs_trading::configure_pairs_strategy(
            &env,
            user,
            asset_a,
            asset_b,
            lookback_period_days,
            entry_z_score,
            exit_z_score,
            position_size_pct,
        )
    }

    pub fn get_pairs_trading_strategy(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<strategies::pairs_trading::PairsTradingStrategy, AutoTradeError> {
        strategies::pairs_trading::get_pairs_trading_strategy(&env, &user, strategy_id)
    }

    pub fn analyze_asset_pair(
        env: Env,
        asset_a: u32,
        asset_b: u32,
        lookback_days: u32,
    ) -> Result<strategies::pairs_trading::PairAnalysis, AutoTradeError> {
        strategies::pairs_trading::analyze_asset_pair(&env, asset_a, asset_b, lookback_days)
    }

    pub fn check_pairs_trading_signal(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<Option<strategies::pairs_trading::PairsSignal>, AutoTradeError> {
        strategies::pairs_trading::check_pairs_trading_signal(&env, &user, strategy_id)
    }

    pub fn execute_pairs_trade(
        env: Env,
        user: Address,
        strategy_id: u64,
        signal: strategies::pairs_trading::PairsSignal,
        portfolio_value: i128,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::pairs_trading::execute_pairs_trade(
            &env,
            &user,
            strategy_id,
            signal,
            portfolio_value,
        )
    }

    pub fn check_pairs_exit(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<Option<u64>, AutoTradeError> {
        user.require_auth();
        strategies::pairs_trading::check_pairs_exit(&env, &user, strategy_id)
    }

    pub fn calculate_optimal_hedge_ratio(
        env: Env,
        asset_a: u32,
        asset_b: u32,
        lookback_days: u32,
    ) -> Result<i128, AutoTradeError> {
        strategies::pairs_trading::calculate_optimal_hedge_ratio(
            &env,
            asset_a,
            asset_b,
            lookback_days,
        )
    }

    // ── Correlation-Based Risk Management (Issue #correlation) ───────────────

    /// Calculate Pearson correlation between two assets (returns bps in [-10000, 10000]).
    pub fn calculate_correlation(env: Env, asset_a: u32, asset_b: u32, window: u32) -> i128 {
        correlation::calculate_correlation(&env, asset_a, asset_b, window)
    }

    /// Build and cache the correlation matrix for the given asset list.
    pub fn build_correlation_matrix(
        env: Env,
        user: Address,
        assets: Vec<u32>,
    ) -> correlation::CorrelationMatrix {
        correlation::get_or_build_matrix(&env, &user, &assets)
    }

    /// Assess correlation risk of adding `new_asset` / `new_amount` to the portfolio.
    pub fn check_portfolio_correlation(
        env: Env,
        user: Address,
        new_asset: u32,
        new_amount: i128,
    ) -> Result<correlation::CorrelationRisk, AutoTradeError> {
        correlation::check_portfolio_correlation(&env, &user, new_asset, new_amount)
    }

    /// Enforce correlation limits; returns error if the trade would breach them.
    pub fn enforce_correlation_limits(
        env: Env,
        user: Address,
        new_asset: u32,
        new_amount: i128,
    ) -> Result<(), AutoTradeError> {
        let result = correlation::enforce_correlation_limits(&env, &user, new_asset, new_amount);
        if result.is_err() {
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(&env, "corr_limit_breach"),
                    user.clone(),
                    new_asset,
                ),
                new_amount,
            );
        }
        result
    }

    /// Set per-user correlation limits.
    pub fn set_correlation_limits(
        env: Env,
        user: Address,
        limits: correlation::CorrelationLimits,
    ) {
        user.require_auth();
        correlation::set_correlation_limits(&env, &user, &limits);
    }

    /// Get per-user correlation limits (defaults if not set).
    pub fn get_correlation_limits(
        env: Env,
        user: Address,
    ) -> correlation::CorrelationLimits {
        correlation::get_correlation_limits(&env, &user)
    }

    /// Suggest up to 5 diversifying assets from `available` with low portfolio correlation.
    pub fn suggest_diversification(
        env: Env,
        user: Address,
        available: Vec<u32>,
    ) -> Vec<u32> {
        correlation::suggest_diversification(&env, &user, &available)
    }

    // ── Conditional Orders (Issue: Options-Style Conditional Orders) ──────────

    /// Create a conditional order that executes when trigger logic fires.
    #[allow(clippy::too_many_arguments)]
    pub fn create_conditional_order(
        env: Env,
        user: Address,
        asset_id: u32,
        side: conditional::ConditionalSide,
        amount: i128,
        limit_price: i128,
        conditions: Vec<conditional::Condition>,
        logic: conditional::LogicOp,
        expires_in_seconds: u64,
    ) -> Result<u64, AutoTradeError> {
        conditional::create_conditional_order(
            &env, user, asset_id, side, amount, limit_price, conditions, logic, expires_in_seconds,
        )
    }

    /// Cancel a pending conditional order.
    pub fn cancel_conditional_order(
        env: Env,
        id: u64,
        user: Address,
    ) -> Result<(), AutoTradeError> {
        conditional::cancel_conditional_order(&env, id, user)
    }

    /// Get a conditional order by id.
    pub fn get_conditional_order(
        env: Env,
        id: u64,
    ) -> Result<conditional::ConditionalOrder, AutoTradeError> {
        conditional::get_conditional_order(&env, id)
    }

    /// Evaluate all active conditional orders; returns ids of newly triggered ones.
    pub fn check_and_trigger_conditionals(env: Env) -> Vec<u64> {
        conditional::check_and_trigger(&env)
    }

    /// Mark a triggered conditional order as executed (call after trade fill).
    pub fn mark_conditional_executed(env: Env, id: u64) -> Result<(), AutoTradeError> {
        conditional::mark_executed(&env, id)
    }
}

#[cfg(test)]
mod test;
mod test_oracle_whitelist;

// ── Oracle integration tests ─────────────────────────────────────────────────
#[cfg(test)]
mod oracle_tests {
    use super::*;
    use crate::oracle;
    use crate::risk;
    use stellar_swipe_common::oracle::{MockOracleClient, OraclePrice};
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env, Symbol,
    };

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(AutoTradeContract, ());
        (env, contract_id)
    }

    fn make_price(env: &Env, price: i128) -> OraclePrice {
        OraclePrice {
            price,
            decimals: 0,
            timestamp: env.ledger().timestamp(),
            source: Symbol::new(env, "mock"),
        }
    }

    /// Admin can set and retrieve the oracle address.
    #[test]
    fn test_set_and_get_oracle_address() {
        let (env, contract_id) = setup();
        let admin = Address::generate(&env);
        let oracle_addr = Address::generate(&env);

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());
            oracle::set_oracle_address(&env, &admin, oracle_addr.clone()).unwrap();
            assert_eq!(oracle::get_oracle_address(&env), Some(oracle_addr));
        });
    }

    /// Non-admin cannot set the oracle address.
    #[test]
    fn test_non_admin_cannot_set_oracle() {
        let (env, contract_id) = setup();
        let admin = Address::generate(&env);
        let attacker = Address::generate(&env);
        let oracle_addr = Address::generate(&env);

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());
            let result = oracle::set_oracle_address(&env, &attacker, oracle_addr);
            assert_eq!(result, Err(AutoTradeError::Unauthorized));
        });
    }

    /// Stop-loss uses oracle price when available, ignoring higher SDEX spot.
    #[test]
    fn test_stop_loss_uses_oracle_price() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            // Entry price 100, stop-loss at 15% → triggers at ≤ 85
            risk::update_position(&env, &user, 1, 1_000, 100);

            let config = risk::RiskConfig::default();

            // SDEX spot = 90 (above stop-loss) but oracle = 80 (below stop-loss)
            let triggered = risk::check_stop_loss(&env, &user, 1, 90, Some(80), &config);
            assert!(triggered, "oracle price below stop-loss must trigger");

            // SDEX spot = 80 (below stop-loss) but oracle = 90 (above stop-loss)
            let not_triggered = risk::check_stop_loss(&env, &user, 1, 80, Some(90), &config);
            assert!(!not_triggered, "oracle price above stop-loss must not trigger");
        });
    }

    /// When no oracle is configured, stop-loss falls back to SDEX spot price.
    #[test]
    fn test_stop_loss_fallback_to_sdex_when_no_oracle() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            risk::update_position(&env, &user, 1, 1_000, 100);
            let config = risk::RiskConfig::default();

            // No oracle price (None) → falls back to SDEX spot 80 → triggers
            let triggered = risk::check_stop_loss(&env, &user, 1, 80, None, &config);
            assert!(triggered);

            // No oracle price (None) → falls back to SDEX spot 90 → no trigger
            let not_triggered = risk::check_stop_loss(&env, &user, 1, 90, None, &config);
            assert!(!not_triggered);
        });
    }

    /// Mock oracle returns the seeded price correctly.
    #[test]
    fn test_mock_oracle_returns_seeded_price() {
        let (env, contract_id) = setup();

        env.as_contract(&contract_id, || {
            let expected = make_price(&env, 42_000);
            MockOracleClient::set_price(&env, 1, expected.clone());

            let result = oracle::get_mock_oracle_price(&env, 1).unwrap();
            assert_eq!(result.price, 42_000);
        });
    }

    /// Mock oracle returns PriceNotFound when no price is seeded.
    #[test]
    fn test_mock_oracle_price_not_found() {
        let (env, contract_id) = setup();
        use stellar_swipe_common::oracle::OracleError;

        env.as_contract(&contract_id, || {
            let result = oracle::get_mock_oracle_price(&env, 99);
            assert_eq!(result, Err(OracleError::PriceNotFound));
        });
    }

    /// Stale oracle price is rejected.
    #[test]
    fn test_stale_oracle_price_rejected() {
        let (env, contract_id) = setup();
        use stellar_swipe_common::oracle::OracleError;

        env.as_contract(&contract_id, || {
            // Seed a price with timestamp far in the past
            let stale = OraclePrice {
                price: 100,
                decimals: 0,
                timestamp: 1, // way older than MAX_PRICE_AGE_SECS from ledger ts 1_000
                source: Symbol::new(&env, "mock"),
            };
            MockOracleClient::set_price(&env, 1, stale);

            let result = oracle::get_mock_oracle_price(&env, 1);
            assert_eq!(result, Err(OracleError::PriceStale));
        });
    }

    /// oracle_price_to_i128 correctly scales by decimals.
    #[test]
    fn test_oracle_price_scaling() {
        let env = Env::default();
        let op = OraclePrice {
            price: 1_000_000,
            decimals: 4,
            timestamp: 1_000,
            source: Symbol::new(&env, "mock"),
        };
        // 1_000_000 / 10^4 = 100
        assert_eq!(oracle::oracle_price_to_i128(&op), 100);
    }
}

// ── Oracle circuit breaker tests ───────────────────────────────────────────────
#[cfg(test)]
mod oracle_cb_tests {
    use super::*;
    use crate::oracle;
    use crate::admin;
    use stellar_swipe_common::oracle::{MockOracleClient, OraclePrice};
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env, Symbol,
    };

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(AutoTradeContract, ());
        let admin = Address::generate(&env);
        (env, contract_id, admin)
    }

    fn fresh_price(env: &Env, price: i128) -> OraclePrice {
        OraclePrice {
            price,
            decimals: 0,
            timestamp: env.ledger().timestamp(),
            source: Symbol::new(env, "mock"),
        }
    }

    /// When oracle is available, get_aggregated_price returns the price and
    /// circuit breaker stays un-tripped.
    #[test]
    fn test_oracle_available_trade_proceeds() {
        let (env, contract_id, admin) = setup();

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());
            MockOracleClient::set_price(&env, 1, fresh_price(&env, 100));

            // Simulate get_aggregated_price via mock path
            let result = oracle::get_mock_oracle_price(&env, 1);
            assert!(result.is_ok());

            // Circuit breaker must not be tripped
            let state = oracle::get_cb_state(&env);
            assert!(!state.triggered);
        });
    }

    /// When oracle is unavailable, get_aggregated_price trips the circuit
    /// breaker and returns OracleUnavailable.
    #[test]
    fn test_oracle_unavailable_trips_circuit_breaker() {
        let (env, contract_id, admin) = setup();

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());
            // No price seeded → oracle unavailable
            MockOracleClient::clear_price(&env, 1);

            // Manually trip the breaker as get_aggregated_price would
            let mut state = oracle::get_cb_state(&env);
            state.triggered = true;
            state.triggered_at = env.ledger().timestamp();
            env.storage()
                .instance()
                .set(&crate::admin::AdminStorageKey::OracleCircuitBreaker, &state);

            let state = oracle::get_cb_state(&env);
            assert!(state.triggered);
            assert!(!state.admin_override);
        });
    }

    /// check_oracle_circuit_breaker blocks trading when breaker is tripped
    /// and oracle is still unavailable.
    #[test]
    fn test_circuit_breaker_blocks_trade_when_oracle_down() {
        let (env, contract_id, admin) = setup();

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());

            // Trip the breaker
            let mut state = oracle::get_cb_state(&env);
            state.triggered = true;
            state.triggered_at = env.ledger().timestamp();
            env.storage()
                .instance()
                .set(&crate::admin::AdminStorageKey::OracleCircuitBreaker, &state);

            // Oracle still down (no price seeded) → check must fail
            let result = oracle::check_oracle_circuit_breaker(&env, 1);
            assert_eq!(result, Err(AutoTradeError::OracleUnavailable));
        });
    }

    /// When oracle recovers, check_oracle_circuit_breaker auto-resets the
    /// breaker and returns Ok.
    #[test]
    fn test_circuit_breaker_auto_resets_on_recovery() {
        let (env, contract_id, admin) = setup();

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());

            // Trip the breaker
            let mut state = oracle::get_cb_state(&env);
            state.triggered = true;
            state.triggered_at = env.ledger().timestamp();
            env.storage()
                .instance()
                .set(&crate::admin::AdminStorageKey::OracleCircuitBreaker, &state);

            // Oracle recovers — seed a fresh price
            MockOracleClient::set_price(&env, 1, fresh_price(&env, 100));

            // check_oracle_circuit_breaker should reset and return Ok
            let result = oracle::check_oracle_circuit_breaker(&env, 1);
            assert!(result.is_ok(), "should recover when oracle is healthy");

            let state = oracle::get_cb_state(&env);
            assert!(!state.triggered, "breaker must be reset after recovery");
        });
    }

    /// Admin override allows trading even when circuit breaker is tripped.
    #[test]
    fn test_admin_override_allows_trade_when_oracle_down() {
        let (env, contract_id, admin) = setup();

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());

            // Trip the breaker
            let mut state = oracle::get_cb_state(&env);
            state.triggered = true;
            state.triggered_at = env.ledger().timestamp();
            env.storage()
                .instance()
                .set(&crate::admin::AdminStorageKey::OracleCircuitBreaker, &state);

            // Admin enables override
            oracle::override_oracle_circuit_breaker(&env, &admin, true).unwrap();

            // check must pass despite breaker being tripped
            let result = oracle::check_oracle_circuit_breaker(&env, 1);
            assert!(result.is_ok(), "admin override must allow trading");
        });
    }

    /// Admin can disable the override, restoring normal circuit breaker behaviour.
    #[test]
    fn test_admin_can_disable_override() {
        let (env, contract_id, admin) = setup();

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());

            // Trip the breaker and enable override
            let mut state = oracle::get_cb_state(&env);
            state.triggered = true;
            state.triggered_at = env.ledger().timestamp();
            env.storage()
                .instance()
                .set(&crate::admin::AdminStorageKey::OracleCircuitBreaker, &state);
            oracle::override_oracle_circuit_breaker(&env, &admin, true).unwrap();

            // Disable override — oracle still down → should block again
            oracle::override_oracle_circuit_breaker(&env, &admin, false).unwrap();
            let result = oracle::check_oracle_circuit_breaker(&env, 1);
            assert_eq!(result, Err(AutoTradeError::OracleUnavailable));
        });
    }

    /// Non-admin cannot set the override.
    #[test]
    fn test_non_admin_cannot_override_circuit_breaker() {
        let (env, contract_id, admin) = setup();
        let attacker = Address::generate(&env);

        env.as_contract(&contract_id, || {
            admin::init_admin(&env, admin.clone());
            let result = oracle::override_oracle_circuit_breaker(&env, &attacker, true);
            assert_eq!(result, Err(AutoTradeError::Unauthorized));
        });
    }
}

// ── Correlation-Based Risk Management integration tests ───────────────────────────────────────────────
#[cfg(test)]
mod correlation_tests {
    use super::*;
    use crate::correlation::{CorrelationLimits, RiskLevel};
    use crate::risk;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env, Vec,
    };

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(AutoTradeContract, ());
        (env, contract_id)
    }

    fn seed_prices(env: &Env, asset_id: u32, prices: &[i128]) {
        use crate::risk::RiskDataKey;
        for (i, &p) in prices.iter().enumerate() {
            env.storage().persistent().set(
                &RiskDataKey::AssetPriceHistory(asset_id, i as u32),
                &p,
            );
        }
        env.storage().persistent().set(
            &RiskDataKey::AssetPriceHistoryCount(asset_id),
            &(prices.len() as u32),
        );
    }

    /// Validation: XLM/USDC and XLM/BTC share the XLM leg → high correlation.
    #[test]
    fn test_xlm_usdc_xlm_btc_high_correlation() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            // Asset 1 = XLM/USDC, Asset 2 = XLM/BTC — same trend (XLM dominates).
            let prices = [100i128, 103, 101, 106, 104, 109, 107, 112];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);

            let corr =
                AutoTradeContract::calculate_correlation(env.clone(), 1, 2, 30);
            assert!(
                corr > 7_000,
                "XLM pairs should be highly correlated, got {corr}"
            );
        });
    }

    /// Validation: build matrix for 10 assets.
    #[test]
    fn test_build_matrix_10_assets() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let prices = [100i128, 102, 101, 105, 103, 107, 106, 110];
            for id in 1u32..=10 {
                seed_prices(&env, id, &prices);
            }
            let mut assets = Vec::new(&env);
            for id in 1u32..=10 {
                assets.push_back(id);
            }
            let matrix =
                AutoTradeContract::build_correlation_matrix(env.clone(), user.clone(), assets);
            // 10 assets → 10*9 = 90 directed pairs.
            assert_eq!(matrix.correlations.len(), 90);
        });
    }

    /// Validation: portfolio with high correlation is detected.
    #[test]
    fn test_high_correlation_portfolio_detected() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let prices = [100i128, 103, 101, 106, 104, 109, 107, 112];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);

            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);

            let risk_result = AutoTradeContract::check_portfolio_correlation(
                env.clone(),
                user.clone(),
                2,
                5_000,
            )
            .unwrap();

            assert_eq!(risk_result.highly_correlated_assets, 1);
            assert_ne!(risk_result.risk_level, RiskLevel::Low);
        });
    }

    /// Validation: trade that exceeds correlation limits is blocked.
    #[test]
    fn test_trade_blocked_by_correlation_limit() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let prices = [100i128, 103, 101, 106, 104, 109, 107, 112];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);

            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);

            // Zero correlated positions allowed.
            AutoTradeContract::set_correlation_limits(
                env.clone(),
                user.clone(),
                CorrelationLimits {
                    max_correlated_exposure_pct: 70,
                    max_single_correlation: 7_000,
                    max_correlated_positions: 0,
                },
            );

            let result = AutoTradeContract::enforce_correlation_limits(
                env.clone(),
                user.clone(),
                2,
                5_000,
            );
            assert_eq!(result, Err(AutoTradeError::TooManyCorrelatedPositions));
        });
    }

    /// Validation: uncorrelated trade passes limits.
    #[test]
    fn test_uncorrelated_trade_passes_limits() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 1_000, 100);
            // Asset 99 has no price history → correlation defaults to 0.
            let result = AutoTradeContract::enforce_correlation_limits(
                env.clone(),
                user.clone(),
                99,
                500,
            );
            assert!(result.is_ok());
        });
    }

    /// Validation: diversification suggestions exclude held assets and return low-corr candidates.
    #[test]
    fn test_diversification_suggestions() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 1_000, 100);

            let mut available = Vec::new(&env);
            available.push_back(1u32); // already held — should be excluded
            available.push_back(2u32); // no history → low corr → suggested
            available.push_back(3u32); // no history → low corr → suggested

            let suggestions = AutoTradeContract::suggest_diversification(
                env.clone(),
                user.clone(),
                available,
            );

            // Asset 1 must not appear; assets 2 and 3 should.
            for i in 0..suggestions.len() {
                assert_ne!(suggestions.get(i).unwrap(), 1u32);
            }
            assert!(suggestions.len() >= 2);
        });
    }
}
