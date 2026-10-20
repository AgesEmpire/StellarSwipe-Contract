#![allow(dead_code)]

//! Structured logging and trade-outcome metrics (issue #636).
//!
//! Emitted events (topic `log_entry`, payload [`LogEntry`]):
//! - category `"trade"`, message `"execute_trade_started"` — Info, on entry
//!   to `execute_trade`, before any validation.
//! - category `"trade"`, message `"execute_trade_blocked"` — Warn, when
//!   trading is paused or the oracle circuit breaker is tripped.
//! - category `"trade"`, message one of `"trade_filled"` /
//!   `"trade_partially_filled"` / `"trade_failed"` — Info/Warn/Critical,
//!   once the trade's final [`crate::TradeStatus`] is known.
//! - category `"simulation"`, message = the simulation's failure reason
//!   (e.g. `"slippage_exceeded"`, `"insufficient_liquidity"`) — Warn, any
//!   time `simulate_copy_trade` predicts a trade would not succeed.
//!
//! [`TradeMetrics`] is a running counter (attempts / filled / partially
//! filled / failed) updated on every `execute_trade` outcome, queryable via
//! `get_trade_metrics` for a cheap on-chain success-rate signal.

use crate::admin::require_admin;
use crate::errors::AutoTradeError;
use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    // Named `Critical` rather than `Error`: a variant literally named `Error`
    // collides with the associated type `Error` that soroban's #[contracttype]
    // macro generates for value conversion, which rustc rejects as ambiguous.
    Critical,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub schema_version: u32,
    pub timestamp: u64,
    pub level: LogLevel,
    pub category: String,
    pub message: String,
    pub correlation_id: Option<String>,
}

#[contracttype]
pub enum LoggingStorageKey {
    Config,
    RecentLogs,
    Metrics,
}

/// Running trade-outcome counters, updated once per `execute_trade` call.
#[contracttype]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TradeMetrics {
    pub total_attempts: u64,
    pub total_filled: u64,
    pub total_partially_filled: u64,
    pub total_failed: u64,
}

pub fn get_trade_metrics(env: &Env) -> TradeMetrics {
    env.storage()
        .instance()
        .get(&LoggingStorageKey::Metrics)
        .unwrap_or_default()
}

/// Record the outcome of a completed `execute_trade` call and emit a
/// matching structured log entry (Info for a full fill, Warn for a partial
/// fill, Critical for a failure).
pub fn record_trade_outcome(env: &Env, status: &crate::TradeStatus) {
    let mut metrics = get_trade_metrics(env);
    metrics.total_attempts += 1;

    let (level, message) = match status {
        crate::TradeStatus::Filled => {
            metrics.total_filled += 1;
            (LogLevel::Info, "trade_filled")
        }
        crate::TradeStatus::PartiallyFilled => {
            metrics.total_partially_filled += 1;
            (LogLevel::Warn, "trade_partially_filled")
        }
        crate::TradeStatus::Failed => {
            metrics.total_failed += 1;
            (LogLevel::Critical, "trade_failed")
        }
        crate::TradeStatus::Pending => return,
    };

    env.storage()
        .instance()
        .set(&LoggingStorageKey::Metrics, &metrics);

    emit_log(
        env,
        level,
        String::from_str(env, "trade"),
        String::from_str(env, message),
        None,
    );
}

pub fn set_log_level(env: &Env, caller: &Address, level: LogLevel) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;
    env.storage()
        .instance()
        .set(&LoggingStorageKey::Config, &level);
    Ok(())
}

pub fn get_log_level(env: &Env) -> LogLevel {
    env.storage()
        .instance()
        .get(&LoggingStorageKey::Config)
        .unwrap_or(LogLevel::Info)
}

pub fn is_info_logging_enabled(env: &Env) -> bool {
    matches!(get_log_level(env), LogLevel::Debug | LogLevel::Info)
}

pub fn emit_log(
    env: &Env,
    level: LogLevel,
    category: String,
    message: String,
    correlation_id: Option<String>,
) {
    let configured = get_log_level(env);
    if !should_log(&configured, &level) {
        return;
    }

    let entry = LogEntry {
        schema_version: 1,
        timestamp: env.ledger().timestamp(),
        level: level.clone(),
        category: category.clone(),
        message: message.clone(),
        correlation_id: correlation_id.clone(),
    };

    env.events()
        .publish((Symbol::new(env, "log_entry"),), entry.clone());

    let mut logs: Vec<LogEntry> = env
        .storage()
        .instance()
        .get(&LoggingStorageKey::RecentLogs)
        .unwrap_or_else(|| Vec::new(env));

    if logs.len() >= 20 {
        logs.remove(0);
    }
    logs.push_back(entry);
    env.storage()
        .instance()
        .set(&LoggingStorageKey::RecentLogs, &logs);
}

/// Returns the most recent log entries (oldest first), capped at 20.
pub fn get_recent_logs(env: &Env) -> Vec<LogEntry> {
    env.storage()
        .instance()
        .get(&LoggingStorageKey::RecentLogs)
        .unwrap_or_else(|| Vec::new(env))
}

fn should_log(configured: &LogLevel, event_level: &LogLevel) -> bool {
    matches!(
        (configured, event_level),
        (LogLevel::Debug, _)
            | (LogLevel::Info, LogLevel::Info)
            | (LogLevel::Info, LogLevel::Warn)
            | (LogLevel::Info, LogLevel::Critical)
            | (LogLevel::Warn, LogLevel::Warn)
            | (LogLevel::Warn, LogLevel::Critical)
            | (LogLevel::Critical, LogLevel::Critical)
    )
}
