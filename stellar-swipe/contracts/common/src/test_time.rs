//! Shared test harness for simulating Stellar ledger time advancement.
//!
//! This module provides a clean, reusable API for advancing the simulated ledger
//! timestamp in Soroban test environments. Use it instead of ad-hoc
//! `env.ledger().set_timestamp(...)` calls to make time-dependent tests
//! readable and consistent across contracts.
//!
//! # Example
//!
//! ```rust,ignore
//! use stellar_swipe_common::test_time::{advance_time, set_time, current_time};
//! use soroban_sdk::testutils::Ledger;
//!
//! let env = Env::default();
//! env.mock_all_auths();
//! env.ledger().set_timestamp(1_000);
//!
//! // Advance by a duration
//! advance_time(&env, 3600);          // +1 hour
//! assert_eq!(current_time(&env), 4_600);
//!
//! // Jump to an absolute timestamp
//! set_time(&env, 100_000);
//! assert_eq!(current_time(&env), 100_000);
//!
//! // Multiple checkpoints in one test (e.g. vesting schedule)
//! advance_time(&env, 30 * 86_400);   // +30 days  (cliff)
//! advance_time(&env, 60 * 86_400);   // +60 more days
//! ```

use soroban_sdk::testutils::Ledger as LedgerTestUtils;
use soroban_sdk::Env;

/// Returns the current simulated ledger timestamp (seconds since epoch).
pub fn current_time(env: &Env) -> u64 {
    env.ledger().timestamp()
}

/// Advance the simulated ledger clock by `seconds` from the current timestamp.
///
/// Panics if the resulting timestamp would overflow `u64`.
pub fn advance_time(env: &Env, seconds: u64) {
    let next = current_time(env)
        .checked_add(seconds)
        .expect("timestamp overflow");
    env.ledger().set_timestamp(next);
}

/// Set the simulated ledger clock to an absolute `timestamp`.
///
/// `timestamp` must be ≥ the current ledger timestamp; moving the clock
/// backwards is not realistic and will panic.
pub fn set_time(env: &Env, timestamp: u64) {
    assert!(
        timestamp >= current_time(env),
        "set_time: cannot move clock backwards ({} < {})",
        timestamp,
        current_time(env),
    );
    env.ledger().set_timestamp(timestamp);
}

/// Convenience: advance by exactly `days` calendar days (86 400 seconds each).
pub fn advance_days(env: &Env, days: u64) {
    advance_time(env, days * 86_400);
}

/// Convenience: advance by exactly `hours`.
pub fn advance_hours(env: &Env, hours: u64) {
    advance_time(env, hours * 3_600);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger as LedgerTestUtils;
    use soroban_sdk::Env;

    fn make_env(start: u64) -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(start);
        env
    }

    // ── timelock test suite (migrated from governance::timelock tests) ────────

    /// Simulates "timelock delay has passed" by advancing time past the delay.
    #[test]
    fn test_timelock_delay_elapsed_via_harness() {
        let env = make_env(0);
        let delay: u64 = 2 * 86_400; // 2-day delay

        // Queue a hypothetical action at t=0
        let queued_at = current_time(&env);
        let execution_available = queued_at + delay;

        // Before delay elapses: action not yet available
        assert!(current_time(&env) < execution_available);

        // Advance past the delay
        advance_time(&env, delay + 1);

        // Action is now available
        assert!(current_time(&env) >= execution_available);
    }

    /// Multiple time checkpoints within one test (mimics a vesting schedule).
    #[test]
    fn test_multiple_time_checkpoints() {
        let env = make_env(1_000);

        // Checkpoint 1: cliff at +30 days
        let cliff = 30 * 86_400;
        advance_days(&env, 30);
        assert_eq!(current_time(&env), 1_000 + cliff);

        // Checkpoint 2: partial vesting at +60 more days
        advance_days(&env, 60);
        assert_eq!(current_time(&env), 1_000 + cliff + 60 * 86_400);

        // Checkpoint 3: full vesting at +2 more years
        advance_days(&env, 2 * 365);
        assert_eq!(
            current_time(&env),
            1_000 + cliff + 60 * 86_400 + 2 * 365 * 86_400
        );
    }

    // ── signal expiry test suite (migrated) ───────────────────────────────────

    /// Confirms that a signal created with a given TTL is live before expiry
    /// and conceptually expired after.
    #[test]
    fn test_signal_expiry_via_harness() {
        let env = make_env(10_000);
        let ttl: u64 = 86_400; // 1 day
        let expiry = current_time(&env) + ttl;

        // Before expiry
        assert!(current_time(&env) <= expiry);

        // Advance to just before expiry
        advance_time(&env, ttl - 1);
        assert!(current_time(&env) < expiry);

        // Advance past expiry
        advance_time(&env, 2);
        assert!(current_time(&env) > expiry);
    }

    #[test]
    fn test_set_time_absolute() {
        let env = make_env(0);
        set_time(&env, 999_999);
        assert_eq!(current_time(&env), 999_999);
    }

    #[test]
    fn test_advance_hours() {
        let env = make_env(0);
        advance_hours(&env, 24);
        assert_eq!(current_time(&env), 86_400);
    }

    #[test]
    #[should_panic(expected = "cannot move clock backwards")]
    fn test_set_time_backwards_panics() {
        let env = make_env(1_000);
        set_time(&env, 500); // should panic
    }
}
