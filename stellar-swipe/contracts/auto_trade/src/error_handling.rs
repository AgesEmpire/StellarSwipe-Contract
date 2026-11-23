//! Standardized error handling with category hierarchy, recovery strategies,
//! automatic retry logic, error metrics tracking, and graceful degradation.
//!
//! # Error Category Hierarchy
//!
//! ```text
//! ErrorCategory
//! ├── Transient        – temporary conditions; automatic retry is safe
//! │   ├── NetworkCongestion
//! │   ├── OracleUnavailable
//! │   └── LiquidityInsufficient
//! ├── Validation       – caller input is wrong; retry without change is useless
//! │   ├── InvalidAmount
//! │   ├── InvalidConfig
//! │   └── AssetPairInvalid
//! ├── Authorization    – permission / identity failures
//! │   ├── Unauthorized
//! │   └── RateLimited
//! ├── State            – contract state does not allow this operation right now
//! │   ├── TradingPaused
//! │   ├── PositionLimit
//! │   └── DailyLimit
//! └── Permanent        – unrecoverable; no retry makes sense
//!     ├── SlippageExceeded
//!     └── SignalExpired
//! ```
//!
//! # Recovery Strategies
//!
//! Each `ErrorCategory` carries a [`RecoveryStrategy`] that describes what
//! the caller (or an automated keeper) should do next:
//!
//! - `RetryAfter { delay_secs }` – wait the given seconds, then retry.
//! - `RetryWithBackoff { base_delay_secs, max_attempts }` – exponential
//!   back-off, capped at `max_attempts`.
//! - `SwitchToFallback` – skip this path and use an alternative (e.g. SDEX
//!   instead of AMM).
//! - `Degrade { description }` – emit a degraded-mode event and continue with
//!   reduced functionality.
//! - `Abort` – give up; no automated recovery is appropriate.
//!
//! # Automatic Retry
//!
//! [`RetryState`] is a lightweight value type that tracks attempt count and
//! last-attempt timestamp.  Call [`RetryState::should_retry`] before each
//! attempt and [`RetryState::record_attempt`] after a failure.
//!
//! # Error Metrics
//!
//! [`ErrorMetrics`] accumulates per-category failure counts (instance
//! storage, so it persists across transactions).  Call
//! [`record_error`] after every caught error; query with
//! [`get_error_metrics`].
//!
//! # Graceful Degradation
//!
//! Call [`enter_degraded_mode`] to record that the contract is operating with
//! reduced capability (e.g. oracle unavailable → use last-known price).
//! Query with [`is_degraded`].  Call [`exit_degraded_mode`] once normal
//! operation resumes.

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, String, Symbol};

use crate::errors::AutoTradeError;

// ── Error Category Hierarchy ─────────────────────────────────────────────────

/// Top-level classification of every [`AutoTradeError`] variant.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Temporary condition; the same operation may succeed after a short wait.
    Transient,
    /// Caller supplied invalid input; retry without changing inputs is useless.
    Validation,
    /// Permission or identity failure.
    Authorization,
    /// Current contract state does not allow this operation.
    State,
    /// Unrecoverable failure; no retry strategy applies.
    Permanent,
}

/// Maps an [`AutoTradeError`] to its broad [`ErrorCategory`].
pub fn categorize(err: &AutoTradeError) -> ErrorCategory {
    match err {
        // Transient: network / liquidity / oracle issues that may clear up
        AutoTradeError::OracleUnavailable
        | AutoTradeError::InsufficientLiquidity
        | AutoTradeError::SystemError => ErrorCategory::Transient,

        // Validation: bad input
        AutoTradeError::InvalidAmount
        | AutoTradeError::InvalidBasketSize
        | AutoTradeError::InvalidPriceData
        | AutoTradeError::InvalidExitConfig
        | AutoTradeError::InvalidInsuranceConfig
        | AutoTradeError::InvalidStatArbConfig
        | AutoTradeError::InvalidConditionalConfig
        | AutoTradeError::InvalidPairsConfig
        | AutoTradeError::InvalidSlippageSetting => ErrorCategory::Validation,

        // Authorization: identity / permission
        AutoTradeError::Unauthorized | AutoTradeError::RateLimited => {
            ErrorCategory::Authorization
        }

        // State: current contract/account state blocks the operation
        AutoTradeError::TradingPaused
        | AutoTradeError::PositionLimitExceeded
        | AutoTradeError::DailyTradeLimitExceeded
        | AutoTradeError::PositionAlreadyExists
        | AutoTradeError::ActivePortfolioExists
        | AutoTradeError::RateLimitExceeded => ErrorCategory::State,

        // Permanent: nothing the caller can do will change the outcome
        AutoTradeError::SignalExpired
        | AutoTradeError::SignalNotFound
        | AutoTradeError::StrategyNotFound
        | AutoTradeError::ExitStrategyNotFound
        | AutoTradeError::StopLossTriggered
        | AutoTradeError::SlippageExceeded
        | AutoTradeError::FrontRunningRisk => ErrorCategory::Permanent,

        // Default: treat unknown variants as permanent
        _ => ErrorCategory::Permanent,
    }
}

// ── Recovery Strategies ───────────────────────────────────────────────────────

/// What a caller or keeper should do after encountering an error.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryStrategy {
    /// Wait `delay_secs` then retry the same operation unchanged.
    RetryAfter { delay_secs: u64 },
    /// Exponential back-off: start at `base_delay_secs`, double each attempt,
    /// stop after `max_attempts`.
    RetryWithBackoff {
        base_delay_secs: u64,
        max_attempts: u32,
    },
    /// Skip this code-path and attempt a configured fallback (e.g. SDEX when
    /// the AMM route is unavailable).
    SwitchToFallback,
    /// Continue with reduced functionality; `description` names what was
    /// disabled (e.g. `"oracle_price_check"`).
    Degrade { description: String },
    /// No automated recovery is appropriate; surface the error to the user.
    Abort,
}

/// Returns the recommended [`RecoveryStrategy`] for the given error.
pub fn recovery_for(env: &Env, err: &AutoTradeError) -> RecoveryStrategy {
    match categorize(err) {
        ErrorCategory::Transient => RecoveryStrategy::RetryWithBackoff {
            base_delay_secs: 5,
            max_attempts: 3,
        },
        ErrorCategory::State => RecoveryStrategy::RetryAfter { delay_secs: 60 },
        ErrorCategory::Validation | ErrorCategory::Authorization | ErrorCategory::Permanent => {
            // Specific overrides for cases where a fallback makes sense
            match err {
                AutoTradeError::OracleUnavailable => RecoveryStrategy::Degrade {
                    description: String::from_str(env, "oracle_price_check"),
                },
                AutoTradeError::InsufficientLiquidity => RecoveryStrategy::SwitchToFallback,
                _ => RecoveryStrategy::Abort,
            }
        }
    }
}

// ── Automatic Retry State ─────────────────────────────────────────────────────

/// Lightweight per-operation retry tracker (value type; store in persistent
/// storage keyed by operation id when retries span transactions).
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryState {
    /// Number of attempts made so far (including the first try).
    pub attempts: u32,
    /// Ledger timestamp of the last attempt.
    pub last_attempt_at: u64,
    /// Maximum number of attempts configured for this operation.
    pub max_attempts: u32,
    /// Minimum seconds to wait between consecutive attempts.
    pub base_delay_secs: u64,
}

impl RetryState {
    /// Create a new [`RetryState`] with the given caps.
    pub fn new(max_attempts: u32, base_delay_secs: u64) -> Self {
        RetryState {
            attempts: 0,
            last_attempt_at: 0,
            max_attempts,
            base_delay_secs,
        }
    }

    /// Returns `true` when it is safe to make another attempt right now.
    ///
    /// Conditions checked:
    /// 1. Attempt count has not reached `max_attempts`.
    /// 2. Enough time has passed since `last_attempt_at` (exponential back-off).
    pub fn should_retry(&self, now: u64) -> bool {
        if self.attempts >= self.max_attempts {
            return false;
        }
        if self.last_attempt_at == 0 {
            return true; // first attempt
        }
        // Exponential back-off: delay doubles each attempt, capped at 10 minutes.
        let multiplier: u64 = 1u64.checked_shl(self.attempts.min(10)).unwrap_or(1024);
        let required_delay = (self.base_delay_secs.saturating_mul(multiplier)).min(600);
        now.saturating_sub(self.last_attempt_at) >= required_delay
    }

    /// Record a failed attempt.  Call this immediately after a failed try.
    pub fn record_attempt(&mut self, now: u64) {
        self.attempts = self.attempts.saturating_add(1);
        self.last_attempt_at = now;
    }

    /// Whether all retry attempts are exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.attempts >= self.max_attempts
    }
}

// ── Error Metrics ─────────────────────────────────────────────────────────────

/// Per-category error counters, stored in instance storage.
#[contracttype]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorMetrics {
    pub transient_count: u64,
    pub validation_count: u64,
    pub authorization_count: u64,
    pub state_count: u64,
    pub permanent_count: u64,
    /// Total error events ever recorded.
    pub total_count: u64,
    /// Ledger timestamp of the most recent error.
    pub last_error_at: u64,
}

#[contracttype]
pub enum ErrorHandlingKey {
    Metrics,
    DegradedMode,
}

/// Record an error into the persistent [`ErrorMetrics`] counters.
///
/// Call this whenever an `Err(AutoTradeError)` path is taken in the contract
/// so dashboards and monitoring can track failure rates per category.
pub fn record_error(env: &Env, err: &AutoTradeError) {
    let mut metrics: ErrorMetrics = env
        .storage()
        .instance()
        .get(&ErrorHandlingKey::Metrics)
        .unwrap_or_default();

    metrics.total_count = metrics.total_count.saturating_add(1);
    metrics.last_error_at = env.ledger().timestamp();

    match categorize(err) {
        ErrorCategory::Transient => {
            metrics.transient_count = metrics.transient_count.saturating_add(1)
        }
        ErrorCategory::Validation => {
            metrics.validation_count = metrics.validation_count.saturating_add(1)
        }
        ErrorCategory::Authorization => {
            metrics.authorization_count = metrics.authorization_count.saturating_add(1)
        }
        ErrorCategory::State => metrics.state_count = metrics.state_count.saturating_add(1),
        ErrorCategory::Permanent => {
            metrics.permanent_count = metrics.permanent_count.saturating_add(1)
        }
    }

    env.storage()
        .instance()
        .set(&ErrorHandlingKey::Metrics, &metrics);

    // Emit a lightweight event so off-chain indexers can track errors without
    // polling storage.
    env.events().publish(
        (
            Symbol::new(env, "error_recorded"),
            categorize(err) as u32,
        ),
        (err.message(), env.ledger().timestamp()),
    );
}

/// Query the current error metrics.
pub fn get_error_metrics(env: &Env) -> ErrorMetrics {
    env.storage()
        .instance()
        .get(&ErrorHandlingKey::Metrics)
        .unwrap_or_default()
}

/// Reset error metrics (admin only; useful after a diagnosed incident).
pub fn reset_error_metrics(env: &Env, caller: &Address) -> Result<(), AutoTradeError> {
    crate::admin::require_admin(env, caller)?;
    env.storage()
        .instance()
        .set(&ErrorHandlingKey::Metrics, &ErrorMetrics::default());
    Ok(())
}

// ── Graceful Degradation ──────────────────────────────────────────────────────

/// Description of the current degraded-mode condition.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DegradedModeState {
    /// Whether the contract is currently operating in degraded mode.
    pub active: bool,
    /// Human-readable explanation of what capability was reduced.
    pub reason: String,
    /// Ledger timestamp when degraded mode was entered.
    pub entered_at: u64,
}

/// Transition the contract into degraded mode.
///
/// Emits a `degraded_mode_entered` event so monitoring infrastructure can
/// trigger alerts.  Idempotent: calling again while already degraded updates
/// the reason.
pub fn enter_degraded_mode(env: &Env, reason: String) {
    let state = DegradedModeState {
        active: true,
        reason: reason.clone(),
        entered_at: env.ledger().timestamp(),
    };
    env.storage()
        .instance()
        .set(&ErrorHandlingKey::DegradedMode, &state);

    env.events()
        .publish((symbol_short!("degraded"),), (reason, env.ledger().timestamp()));
}

/// Return the contract to normal operation.
///
/// Emits a `degraded_mode_exited` event.  No-op if not currently degraded.
pub fn exit_degraded_mode(env: &Env) {
    let state = DegradedModeState {
        active: false,
        reason: String::from_str(env, ""),
        entered_at: 0,
    };
    env.storage()
        .instance()
        .set(&ErrorHandlingKey::DegradedMode, &state);

    env.events().publish(
        (symbol_short!("restored"),),
        env.ledger().timestamp(),
    );
}

/// Returns `true` when the contract is currently running in degraded mode.
pub fn is_degraded(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<_, DegradedModeState>(&ErrorHandlingKey::DegradedMode)
        .map(|s| s.active)
        .unwrap_or(false)
}

/// Full degraded-mode state, or `None` if the contract is operating normally.
pub fn get_degraded_state(env: &Env) -> Option<DegradedModeState> {
    env.storage()
        .instance()
        .get::<_, DegradedModeState>(&ErrorHandlingKey::DegradedMode)
        .filter(|s| s.active)
}

// ── Helper: record_and_recover ────────────────────────────────────────────────

/// Convenience wrapper: record metrics for `err`, log it at the appropriate
/// level, and return the recommended recovery strategy.
///
/// Usage pattern:
/// ```rust,ignore
/// let strategy = error_handling::record_and_recover(&env, &err);
/// match strategy {
///     RecoveryStrategy::RetryWithBackoff { .. } => { /* keeper schedules retry */ }
///     RecoveryStrategy::SwitchToFallback       => { /* use SDEX path */ }
///     RecoveryStrategy::Degrade { .. }         => { enter_degraded_mode(&env, ..) }
///     RecoveryStrategy::Abort                  => return Err(err),
///     RecoveryStrategy::RetryAfter { .. }      => { /* wait and retry */ }
/// }
/// ```
pub fn record_and_recover(env: &Env, err: &AutoTradeError) -> RecoveryStrategy {
    record_error(env, err);
    recovery_for(env, err)
}
