#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

mod auth;
mod errors;
mod history;
mod multi_asset;
mod portfolio;
mod risk;
mod sdex;
mod storage;
mod strategies;

use crate::storage::DataKey;
use errors::AutoTradeError;

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
    /// Execute a trade on behalf of a user based on a signal
    pub fn execute_trade(
        env: Env,
        user: Address,
        signal_id: u64,
        order_type: OrderType,
        amount: i128,
    ) -> Result<TradeResult, AutoTradeError> {
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
                0,
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
}

mod test;
