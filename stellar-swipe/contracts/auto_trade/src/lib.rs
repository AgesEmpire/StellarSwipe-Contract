#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, String, Vec};

mod admin;
mod auth;
mod errors;
mod exit_strategy;
mod history;
mod iceberg;
mod multi_asset;
mod portfolio;
mod portfolio_insurance;
mod referral;
mod risk;
mod risk_parity;
mod sdex;
mod storage;
mod strategies;

use crate::storage::DataKey;
use errors::AutoTradeError;
use stellar_swipe_common::emergency::{CAT_TRADING, PauseState};
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

    /// Pause a category (admin only)
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

    /// Get current pause states
    pub fn get_pause_states(env: Env) -> soroban_sdk::Map<String, PauseState> {
        admin::get_pause_states(&env)
    }

    /// Set circuit breaker configuration (admin only)
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

        if !sdex::has_sufficient_balance(&env, &user, &signal.base_asset, amount) {
            return Err(AutoTradeError::InsufficientBalance);
        }

        // Determine if this is a sell operation (simplified)
        let is_sell = false; // This should be determined from the signal or order details

        // Set current asset price for risk calculations
        risk::set_asset_price(&env, signal.base_asset, signal.price);

        // Perform risk checks
        let stop_loss_triggered = risk::validate_trade(
            &env,
            &user,
            signal.base_asset,
            amount,
            signal.price,
            is_sell,
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

    /// Get authorization config
    pub fn get_auth_config(env: Env, user: Address) -> Option<auth::AuthConfig> {
        auth::get_auth_config(&env, &user)
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

    // ── Exit Strategy (tiered TP + trailing stops) ─────────────────────────

    /// Create a custom exit strategy with explicit TP and stop-loss tiers.
    #[allow(clippy::too_many_arguments)]
    pub fn create_exit_strategy(
        env: Env,
        user: Address,
        signal_id: u64,
        entry_price: i128,
        position_size: i128,
        take_profit_tiers: soroban_sdk::Vec<exit_strategy::TakeProfitTier>,
        stop_loss_tiers: soroban_sdk::Vec<exit_strategy::StopLossTier>,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        exit_strategy::create_exit_strategy(
            &env,
            user,
            signal_id,
            entry_price,
            position_size,
            take_profit_tiers,
            stop_loss_tiers,
        )
    }

    /// Create a conservative preset exit strategy (3 TPs + 10% trail).
    pub fn create_conservative_exit(
        env: Env,
        user: Address,
        signal_id: u64,
        entry_price: i128,
        position_size: i128,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        exit_strategy::preset_conservative(&env, user, signal_id, entry_price, position_size)
    }

    /// Create a balanced preset exit strategy (2 TPs + tiered trails 10%/7%).
    pub fn create_balanced_exit(
        env: Env,
        user: Address,
        signal_id: u64,
        entry_price: i128,
        position_size: i128,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        exit_strategy::preset_balanced(&env, user, signal_id, entry_price, position_size)
    }

    /// Create an aggressive preset exit strategy (4 TPs + tiered trails 10%/7%/5%).
    pub fn create_aggressive_exit(
        env: Env,
        user: Address,
        signal_id: u64,
        entry_price: i128,
        position_size: i128,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        exit_strategy::preset_aggressive(&env, user, signal_id, entry_price, position_size)
    }

    /// Check current price against all tiers and auto-execute any triggered exits.
    pub fn check_exit_strategy(
        env: Env,
        strategy_id: u64,
        current_price: i128,
    ) -> Result<soroban_sdk::Vec<u64>, AutoTradeError> {
        exit_strategy::check_and_execute_exits(&env, strategy_id, current_price)
    }

    /// Get exit strategy state.
    pub fn get_exit_strategy(
        env: Env,
        strategy_id: u64,
    ) -> Result<exit_strategy::ExitStrategy, AutoTradeError> {
        exit_strategy::get_exit_strategy(&env, strategy_id)
    }

    /// Get all exit strategy IDs for a user.
    pub fn get_user_exit_strategies(
        env: Env,
        user: Address,
    ) -> soroban_sdk::Vec<u64> {
        exit_strategy::get_user_exit_strategies(&env, &user)
    }

    /// Manually adjust remaining position size (e.g. after partial manual close).
    pub fn adjust_exit_position(
        env: Env,
        user: Address,
        strategy_id: u64,
        new_size: i128,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        exit_strategy::adjust_position_size(&env, &user, strategy_id, new_size)
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
}

mod test;
