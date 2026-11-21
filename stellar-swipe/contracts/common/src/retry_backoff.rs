//! Shared retry-with-backoff helper for cross-contract calls (Issue #699).
//!
//! # On-Chain Execution Model
//!
//! Soroban contracts execute within a single ledger entry — block times are
//! **not** available for wall-clock sleep.  Retries therefore **cannot** happen
//! inside a single invocation.  Instead, this helper is designed to be called
//! **across separate invocations/transactions**:
//!
//! 1. The caller determines whether a transient failure occurred.
//! 2. It stores/passes a [`RetryState`] and calls [`should_retry`] which,
//!    when retries remain, computes a `retry_delay_ledgers` — the number of
//!    **ledger sequences** the caller should wait before the next attempt.
//! 3. The off-chain keeper or cron job observes the delay and re-invokes the
//!    contract after that many ledgers have passed.
//!
//! This pattern matches Soroban's atomic execution model: each invocation is
//! an independent transaction.
//!
//! # Exponential Backoff
//!
//! The delay follows an exponential progression:
//! `base_delay_ledgers × 2^(attempt - 1)`, clamped to `max_delay_ledgers`.
//!
//! | Attempt | Base=1 → delay | Base=5 → delay |
//! |---------|----------------|----------------|
//! | 1       | 1             | 5              |
//! | 2       | 2             | 10             |
//! | 3       | 4             | 20             |
//! | 4       | 8             | 40             |
//! | 5       | 16            | 80             |
//!
//! # Usage
//!
//! ```rust,ignore
//! use stellar_swipe_common::retry_backoff::{RetryConfig, RetryState, should_retry};
//!
//! const MAX_ATTEMPTS: u32 = 5;
//! const BASE_DELAY_LEDGERS: u32 = 3;
//!
//! fn my_cross_contract_call(env: &Env, state: &RetryState) -> Result<(), MyError> {
//!     if let Some(delay) = should_retry(state, MAX_ATTEMPTS, BASE_DELAY_LEDGERS) {
//!         // Persist state.retry_count + 1 and instruct caller to retry after `delay` ledgers.
//!         return Err(MyError::Transient(delay));
//!     }
//!     // No more retries — propagate the error permanently.
//!     Err(MyError::Permanent)
//! }
//! ```

use soroban_sdk::contracttype;

/// Configuration for the retry policy.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (including the initial attempt?).
    /// Convention: `max_attempts = 1` means no retry; the operation is
    /// attempted once total.
    pub max_attempts: u32,
    /// Base delay in ledger numbers for the exponential backoff formula.
    /// `delay = base_delay_ledgers × 2^(attempt - 1)`
    pub base_delay_ledgers: u32,
    /// Optional cap on the per-attempt delay (prevents unbounded wait).
    /// `None` means no cap.
    pub max_delay_ledgers: Option<u32>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ledgers: 5,
            max_delay_ledgers: Some(200),
        }
    }
}

/// Mutable retry state that should be persisted between invocations.
#[contracttype]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetryState {
    /// How many times the operation has been **attempted** so far.
    /// Starts at 0 (not yet attempted) and is incremented on each try.
    pub attempt: u32,
}

/// Whether the caller should retry the operation.
///
/// # Returns
///
/// - `Some(delay_ledgers)` — the caller should wait this many ledger
///   sequences before attempting again.  The caller is responsible for
///   persisting a `RetryState { attempt: state.attempt + 1 }` for the
///   next round.
/// - `None` — no more retries remain; the caller should treat the
///   operation as permanently failed.
///
/// # Panics
///
/// Panics if `config.max_attempts == 0` (meaningless configuration).
pub fn should_retry(state: &RetryState, config: &RetryConfig) -> Option<u32> {
    assert!(config.max_attempts > 0, "max_attempts must be > 0");

    // If we've already exhausted all attempts, do not retry.
    if state.attempt >= config.max_attempts {
        return None;
    }

    // Compute exponential backoff: base × 2^(attempt - 1)
    // The first attempt (attempt=0) yields delay=base (zero delay before
    // first try is fine — the first try is immediate).
    // After the first failure (attempt=1), delay = base × 2^0 = base.
    // After the second failure (attempt=2), delay = base × 2^1 = 2×base.
    // etc.
    let exponent = state.attempt.saturating_sub(1); // 0 on first failure → 2^0 = 1
    let multiplier = 1u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let raw_delay = config.base_delay_ledgers.saturating_mul(multiplier);

    let delay = match config.max_delay_ledgers {
        Some(cap) => core::cmp::min(raw_delay, cap),
        None => raw_delay,
    };

    Some(delay)
}

/// Convenience: create a new [`RetryState`] with `attempt` incremented by 1.
pub fn next_retry_state(state: &RetryState) -> RetryState {
    RetryState {
        attempt: state.attempt.saturating_add(1),
    }
}

/// Returns `true` if the state still has remaining attempts according to
/// `config` (i.e. `should_retry` would return `Some`).
pub fn has_remaining_attempts(state: &RetryState, config: &RetryConfig) -> bool {
    state.attempt < config.max_attempts
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_attempt_returns_delay() {
        let state = RetryState { attempt: 0 };
        let config = RetryConfig {
            max_attempts: 3,
            base_delay_ledgers: 5,
            max_delay_ledgers: None,
        };
        let delay = should_retry(&state, &config);
        assert_eq!(delay, Some(5));
    }

    #[test]
    fn test_backoff_progression() {
        let config = RetryConfig {
            max_attempts: 5,
            base_delay_ledgers: 1,
            max_delay_ledgers: None,
        };
        assert_eq!(should_retry(&RetryState { attempt: 1 }, &config), Some(1));
        assert_eq!(should_retry(&RetryState { attempt: 2 }, &config), Some(2));
        assert_eq!(should_retry(&RetryState { attempt: 3 }, &config), Some(4));
        assert_eq!(should_retry(&RetryState { attempt: 4 }, &config), Some(8));
    }

    #[test]
    fn test_exhausted_attempts_return_none() {
        let config = RetryConfig {
            max_attempts: 3,
            base_delay_ledgers: 5,
            max_delay_ledgers: None,
        };
        assert_eq!(should_retry(&RetryState { attempt: 3 }, &config), None);
    }

    #[test]
    fn test_max_delay_cap_applied() {
        let config = RetryConfig {
            max_attempts: 10,
            base_delay_ledgers: 10,
            max_delay_ledgers: Some(50),
        };
        assert_eq!(should_retry(&RetryState { attempt: 4 }, &config), Some(50));
        assert_eq!(should_retry(&RetryState { attempt: 2 }, &config), Some(20));
    }

    #[test]
    fn test_next_retry_state_increments() {
        let state = RetryState { attempt: 0 };
        let next = next_retry_state(&state);
        assert_eq!(next.attempt, 1);
        let next = next_retry_state(&next);
        assert_eq!(next.attempt, 2);
    }

    #[test]
    fn test_has_remaining_attempts() {
        let config = RetryConfig {
            max_attempts: 3,
            base_delay_ledgers: 1,
            max_delay_ledgers: None,
        };
        assert!(has_remaining_attempts(&RetryState { attempt: 0 }, &config));
        assert!(has_remaining_attempts(&RetryState { attempt: 2 }, &config));
        assert!(!has_remaining_attempts(&RetryState { attempt: 3 }, &config));
    }

    #[test]
    fn test_attempt_saturating_sub_does_not_underflow() {
        let config = RetryConfig {
            max_attempts: 1,
            base_delay_ledgers: 10,
            max_delay_ledgers: None,
        };
        assert_eq!(should_retry(&RetryState { attempt: 0 }, &config), Some(10));
        assert_eq!(should_retry(&RetryState { attempt: 1 }, &config), None);
    }

    #[test]
    fn test_attempt_counting() {
        let config = RetryConfig {
            max_attempts: 3,
            base_delay_ledgers: 2,
            max_delay_ledgers: None,
        };
        let mut state = RetryState { attempt: 0 };
        assert!(should_retry(&state, &config).is_some());
        state = next_retry_state(&state);
        assert!(should_retry(&state, &config).is_some());
        state = next_retry_state(&state);
        assert!(should_retry(&state, &config).is_some());
        state = next_retry_state(&state);
        assert!(should_retry(&state, &config).is_none());
    }

    #[test]
    #[should_panic(expected = "max_attempts must be > 0")]
    fn test_zero_max_attempts_panics() {
        let config = RetryConfig {
            max_attempts: 0,
            base_delay_ledgers: 1,
            max_delay_ledgers: None,
        };
        should_retry(&RetryState::default(), &config);
    }

    #[test]
    fn test_default_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.base_delay_ledgers, 5);
        assert_eq!(config.max_delay_ledgers, Some(200));
    }
}
