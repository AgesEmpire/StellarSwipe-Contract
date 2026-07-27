#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, BytesN, Env, String, Symbol, Vec,
};

mod auth;
mod errors;
pub mod governance;
mod history;
mod multi_asset;
mod portfolio;
mod risk;
mod sdex;
pub mod signal_ordering;
mod storage;

use crate::storage::DataKey;
use errors::AutoTradeError;

const EXECUTION_RATE_LIMIT_SECONDS: u64 = 60;

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
    /// # Summary
    /// One-time contract initialization. Sets the admin address and initializes
    /// pause states and circuit breaker statistics.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    pub fn get_build_info(env: Env) -> soroban_sdk::Map<soroban_sdk::String, soroban_sdk::String> {
        let mut m = soroban_sdk::Map::new(&env);
        m.set(
            soroban_sdk::String::from_str(&env, "version"),
            soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION")),
        );
        m.set(
            soroban_sdk::String::from_str(&env, "source_hash"),
            soroban_sdk::String::from_str(&env, env!("STELLAR_SOURCE_HASH")),
        );
        m.set(
            soroban_sdk::String::from_str(&env, "git_commit"),
            soroban_sdk::String::from_str(&env, env!("STELLAR_GIT_COMMIT")),
        );
        m
    }

    ///
    /// # Returns
    /// Nothing. Panics if already initialized.
    pub fn initialize(env: Env, admin: Address) {
        admin::init_admin(&env, admin);
        shared::version::set_contract_version(&env, shared::version::AUTO_TRADE_VERSION);
    }

    // ── Issue #811: upgrade-safe contract versioning ─────────────────────────

    /// Returns this contract's stored version. Cross-contract callers can use
    /// this to enforce a minimum compatible version before invoking this
    /// contract (see `shared::version::validate_callee_version`).
    pub fn get_contract_version(env: Env) -> u32 {
        shared::version::get_contract_version(&env)
    }

    /// Admin-only: replace this contract's executable with `new_wasm_hash`
    /// (previously uploaded via `Deployer::upload_contract_wasm`) and record
    /// `new_version` as the contract's version.
    ///
    /// `new_version` must be strictly greater than the currently stored
    /// version, rejecting accidental or malicious downgrades.
    ///
    /// # Errors
    /// - [`AutoTradeError::Unauthorized`] — caller is not the admin.
    /// - [`AutoTradeError::IncompatibleContractVersion`] — `new_version` is
    ///   not strictly greater than the currently stored version.
    pub fn upgrade(
        env: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
        new_version: u32,
    ) -> Result<(), AutoTradeError> {
        admin::require_admin(&env, &caller)?;

        let current_version = shared::version::get_contract_version(&env);
        shared::version::guard_upgrade(current_version, new_version)
            .map_err(|_| AutoTradeError::IncompatibleContractVersion)?;

        env.deployer().update_current_contract_wasm(new_wasm_hash);
        shared::version::set_contract_version(&env, new_version);
        shared::version::emit_contract_upgraded(&env, current_version, new_version);
        Ok(())
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
    pub fn unpause_category(
        env: Env,
        caller: Address,
        category: String,
    ) -> Result<(), AutoTradeError> {
        admin::unpause_category(&env, &caller, category)
    }

    /// Set guardian address (admin only)
    pub fn set_guardian(
        env: Env,
        caller: Address,
        guardian: Address,
    ) -> Result<(), AutoTradeError> {
        admin::set_guardian(&env, &caller, guardian)
    }

    /// Revoke guardian (admin only)
    pub fn revoke_guardian(env: Env, caller: Address) -> Result<(), AutoTradeError> {
        admin::revoke_guardian(&env, &caller)
    }

    /// Propose admin transfer (current admin only)
    pub fn propose_admin_transfer(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), AutoTradeError> {
        admin::propose_admin_transfer(&env, &caller, new_admin)
    }

    /// Accept admin transfer (new admin only)
    pub fn accept_admin_transfer(env: Env, caller: Address) -> Result<(), AutoTradeError> {
        admin::accept_admin_transfer(&env, &caller)
    }

    /// Cancel pending admin transfer (current admin only)
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), AutoTradeError> {
        admin::cancel_admin_transfer(&env, &caller)
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
    pub fn get_oracle_circuit_breaker_state(env: Env) -> oracle::OracleCircuitBreakerState {
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
    pub fn get_oracle_whitelist(env: Env, asset_pair: u32) -> soroban_sdk::Vec<Address> {
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

    /// Create a new TWAP order covering a larger trade over a configurable time window.
    pub fn create_twap_order(
        env: Env,
        user: Address,
        pair: twap::AssetPair,
        total_amount: i128,
        duration_minutes: u32,
        num_segments: Option<u32>,
        window_minutes: Option<u32>,
    ) -> Result<u64, AutoTradeError> {
        twap::create_twap_order(
            &env,
            user,
            pair,
            total_amount,
            duration_minutes,
            num_segments,
            window_minutes,
        )
    }

    /// Execute all due TWAP segments for active running orders.
    pub fn execute_twap_segments(env: Env) -> soroban_sdk::Vec<u64> {
        twap::execute_twap_segments(&env)
    }

    /// Perform periodic strategy adjustment for a TWAP order based on volatility.
    pub fn adjust_twap_strategy(env: Env, order_id: u64) -> Result<(), AutoTradeError> {
        twap::adjust_twap_strategy(&env, order_id)
    }

    /// Cancel an active TWAP order and return a cancellation summary.
    pub fn cancel_twap_order(
        env: Env,
        order_id: u64,
        user: Address,
    ) -> Result<twap::CancellationSummary, AutoTradeError> {
        twap::cancel_twap_order(&env, order_id, user)
    }

    /// Retrieve a stored TWAP order.
    pub fn get_twap_order(env: Env, order_id: u64) -> Result<twap::TWAPOrder, AutoTradeError> {
        twap::get_twap_order(&env, order_id)
    }

    /// Retrieve all active TWAP orders.
    pub fn get_active_twap_orders(env: Env) -> soroban_sdk::Vec<twap::TWAPOrder> {
        twap::get_active_twap_orders(&env)
    }

    /// Set the contract-wide log level for structured log emission.
    pub fn set_log_level(
        env: Env,
        caller: Address,
        level: logging::LogLevel,
    ) -> Result<(), AutoTradeError> {
        logging::set_log_level(&env, &caller, level)
    }

    /// Get the current configured log level.
    pub fn get_log_level(env: Env) -> logging::LogLevel {
        logging::get_log_level(&env)
    }

    /// Write a structured log entry to the contract event stream.
    pub fn log_event(
        env: Env,
        level: logging::LogLevel,
        category: String,
        message: String,
        correlation_id: Option<String>,
    ) {
        logging::emit_log(&env, level, category, message, correlation_id);
    }

    /// Get running trade-outcome counters (attempts / filled / partially
    /// filled / failed) for a cheap on-chain success-rate signal.
    pub fn get_trade_metrics(env: Env) -> logging::TradeMetrics {
        logging::get_trade_metrics(&env)
    }

    /// Get the most recent structured log entries (oldest first, capped at 20).
    pub fn get_recent_logs(env: Env) -> soroban_sdk::Vec<logging::LogEntry> {
        logging::get_recent_logs(&env)
    }

    /// Submit a KYC verification request for a user.
    pub fn submit_kyc_verification(
        env: Env,
        user: Address,
        kyc_id: String,
        level: kyc::KYCLevel,
    ) -> Result<(), AutoTradeError> {
        kyc::submit_kyc_verification(&env, &user, kyc_id, level)
    }

    /// Admin verifies a user's KYC status.
    pub fn verify_kyc(
        env: Env,
        caller: Address,
        user: Address,
        verified: bool,
    ) -> Result<(), AutoTradeError> {
        kyc::verify_kyc(&env, &caller, &user, verified)
    }

    /// Get the stored KYC data for a user.
    pub fn get_kyc_data(env: Env, user: Address) -> Option<kyc::KYCData> {
        kyc::get_kyc_data(&env, &user)
    }

    /// Returns whether a user is currently KYC verified.
    pub fn is_kyc_verified(env: Env, user: Address) -> bool {
        kyc::is_kyc_verified(&env, &user)
    }

    /// Returns the user tier based on KYC verification level.
    pub fn get_user_tier(env: Env, user: Address) -> String {
        kyc::get_user_tier(&env, &user)
    }

    /// Set the circuit breaker configuration (admin only)
    pub fn set_circuit_breaker_config(
        env: Env,
        caller: Address,
        config: stellar_swipe_common::emergency::CircuitBreakerConfig,
    ) -> Result<(), AutoTradeError> {
        admin::set_cb_config(&env, &caller, config)
    }

    /// # Summary
    /// Execute a trade on behalf of a user based on a signal. Performs oracle
    /// circuit-breaker check, risk validation (stop-loss, position limits,
    /// daily trade limit), smart routing, and records the trade.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `user`: Address of the trader (must authorize).
    /// - `signal_id`: ID of the signal to trade on.
    /// - `order_type`: [`OrderType::Market`] or [`OrderType::Limit`].
    /// - `amount`: Amount to trade (must be > 0).
    ///
    /// # Returns
    /// [`TradeResult`] containing the executed trade details.
    ///
    /// # Errors
    /// - [`AutoTradeError::TradingPaused`] — trading category is paused.
    /// - [`AutoTradeError::OracleUnavailable`] — oracle circuit breaker is tripped.
    /// - [`AutoTradeError::InvalidAmount`] — amount <= 0.
    /// - [`AutoTradeError::SignalNotFound`] — signal_id does not exist.
    /// - [`AutoTradeError::SignalExpired`] — signal has expired.
    /// - [`AutoTradeError::Unauthorized`] — user is not authorized to trade.
    /// - [`AutoTradeError::InsufficientBalance`] — user has insufficient balance.
    /// - [`AutoTradeError::PositionLimitExceeded`] — trade would exceed position limit.
    /// - [`AutoTradeError::DailyTradeLimitExceeded`] — daily trade limit reached.
    ///
    /// # Example
    /// ```rust,ignore
    /// let result = client.execute_trade(&user, &signal_id, &OrderType::Market, &1_000_0000000i128);
    /// assert_eq!(result.trade.status, TradeStatus::Filled);
    /// ```
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

        let last_execution = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::LastExecution(user.clone(), signal_id))
            .unwrap_or(0);
        let now = env.ledger().timestamp();
        if now < last_execution.saturating_add(EXECUTION_RATE_LIMIT_SECONDS) {
            return Err(AutoTradeError::ExecutionRateLimited);
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
        env.storage()
            .persistent()
            .set(&DataKey::LastExecution(user.clone(), signal_id), &now);

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
}

mod test;
mod test_governance;
