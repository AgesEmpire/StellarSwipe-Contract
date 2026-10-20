//! Contract-level rate limiting for join / registration operations (Issue #1030).
//!
//! High-frequency join attempts (joining a pool, registering as a provider,
//! enrolling in a contest, …) can strain contract resources and drive state
//! churn.  Unlike [`crate::rate_limit`], which throttles a **single user's**
//! actions, this limiter caps the **aggregate** volume of join operations for a
//! given asset within a fixed time window, regardless of who initiates them.
//!
//! ## Deterministic windows
//!
//! Windows are aligned to the ledger clock: the active window id is
//! `timestamp / window_secs`.  Every node evaluating the same ledger sees the
//! same window id and the same counter, so the limiter is fully deterministic
//! and needs no per-action timestamp history (O(1) per check).
//!
//! ## Asset-specific counters
//!
//! Each asset (identified by its `Symbol` code, e.g. `symbol_short!("XLM")`)
//! has its own independent counter.  Exhausting the budget for one asset never
//! blocks joins for another.
//!
//! Storage layout:
//!   JoinRateLimitKey::Config          -> JoinRateLimitConfig   (instance)
//!   JoinRateLimitKey::Bucket(Symbol)  -> JoinBucket            (persistent)

#![allow(dead_code)]

use crate::constants::SECONDS_PER_HOUR;
use soroban_sdk::{contracttype, Env, Symbol};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default window length: one hour.
pub const DEFAULT_JOIN_WINDOW_SECS: u64 = SECONDS_PER_HOUR;
/// Default maximum join operations per asset per window.
pub const DEFAULT_MAX_JOINS_PER_WINDOW: u32 = 100;

// ── Types ────────────────────────────────────────────────────────────────────

/// Failure modes surfaced to callers when a join is rate limited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum JoinRateLimitError {
    /// The per-asset budget for the current window has been reached.
    Exceeded = 1,
    /// Join operations are globally disabled (`max_joins_per_window == 0`).
    Disabled = 2,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRateLimitConfig {
    /// Fixed window length in seconds. Values below 1 are treated as 1.
    pub window_secs: u64,
    /// Maximum join operations allowed per asset within a single window.
    /// `0` disables all join operations.
    pub max_joins_per_window: u32,
}

impl JoinRateLimitConfig {
    fn effective_window(&self) -> u64 {
        self.window_secs.max(1)
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct JoinBucket {
    /// Aligned window id (`timestamp / window_secs`) the count belongs to.
    pub window_id: u64,
    /// Join operations recorded so far in `window_id`.
    pub count: u32,
}

#[contracttype]
#[derive(Clone)]
enum JoinRateLimitKey {
    Config,
    Bucket(Symbol),
}

// ── Config ───────────────────────────────────────────────────────────────────

pub fn default_config() -> JoinRateLimitConfig {
    JoinRateLimitConfig {
        window_secs: DEFAULT_JOIN_WINDOW_SECS,
        max_joins_per_window: DEFAULT_MAX_JOINS_PER_WINDOW,
    }
}

pub fn get_config(env: &Env) -> JoinRateLimitConfig {
    env.storage()
        .instance()
        .get(&JoinRateLimitKey::Config)
        .unwrap_or_else(default_config)
}

/// Persist a new limiter configuration. `window_secs` is floored at 1 to keep
/// window-id arithmetic well defined.
pub fn set_config(env: &Env, mut config: JoinRateLimitConfig) {
    config.window_secs = config.effective_window();
    env.storage()
        .instance()
        .set(&JoinRateLimitKey::Config, &config);
}

// ── Window helpers ───────────────────────────────────────────────────────────

/// Aligned id of the window that `now` falls into for the active config.
pub fn current_window_id(env: &Env) -> u64 {
    let window = get_config(env).effective_window();
    env.ledger().timestamp() / window
}

fn get_bucket(env: &Env, asset: &Symbol) -> Option<JoinBucket> {
    env.storage()
        .persistent()
        .get(&JoinRateLimitKey::Bucket(asset.clone()))
}

fn save_bucket(env: &Env, asset: &Symbol, bucket: &JoinBucket) {
    env.storage()
        .persistent()
        .set(&JoinRateLimitKey::Bucket(asset.clone()), bucket);
}

/// Join operations recorded for `asset` within the current window
/// (`0` once the window rolls over).
pub fn current_count(env: &Env, asset: &Symbol) -> u32 {
    let wid = current_window_id(env);
    match get_bucket(env, asset) {
        Some(b) if b.window_id == wid => b.count,
        _ => 0,
    }
}

// ── Core API ─────────────────────────────────────────────────────────────────

/// Check whether one more join for `asset` is permitted in the current window.
/// Does not mutate storage. Emits `join_rate_limited` and returns an error when
/// the budget is exhausted or joins are disabled.
pub fn check(env: &Env, asset: &Symbol) -> Result<u32, JoinRateLimitError> {
    let config = get_config(env);
    if config.max_joins_per_window == 0 {
        emit_join_rate_limited(env, asset, 0, 0);
        return Err(JoinRateLimitError::Disabled);
    }
    let count = current_count(env, asset);
    if count >= config.max_joins_per_window {
        emit_join_rate_limited(env, asset, count, config.max_joins_per_window);
        return Err(JoinRateLimitError::Exceeded);
    }
    Ok(count)
}

/// Record one join for `asset`, rolling the window over if it has elapsed.
/// Call after a successful [`check`].
pub fn record(env: &Env, asset: &Symbol) {
    let wid = current_window_id(env);
    let mut bucket = get_bucket(env, asset).unwrap_or(JoinBucket {
        window_id: wid,
        count: 0,
    });
    if bucket.window_id != wid {
        bucket.window_id = wid;
        bucket.count = 0;
    }
    bucket.count = bucket.count.saturating_add(1);
    save_bucket(env, asset, &bucket);
}

/// Atomically [`check`] then [`record`] a join for `asset`.
/// Returns the count *before* this join on success.
pub fn try_consume(env: &Env, asset: &Symbol) -> Result<u32, JoinRateLimitError> {
    let count = check(env, asset)?;
    record(env, asset);
    Ok(count)
}

// ── Event ────────────────────────────────────────────────────────────────────

fn emit_join_rate_limited(env: &Env, asset: &Symbol, count: u32, limit: u32) {
    let topics = (Symbol::new(env, "join_rate_limited"), asset.clone());
    env.events()
        .publish(topics, (count, limit, current_window_id(env)));
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl, symbol_short,
        testutils::{Events, Ledger},
        Env, Symbol,
    };

    #[contract]
    struct JoinLimitHarness;

    #[contractimpl]
    impl JoinLimitHarness {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let contract_id = env.register(JoinLimitHarness, ());
        (env, contract_id)
    }

    #[test]
    fn within_limit_calls_succeed() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 5,
                },
            );
            let xlm = symbol_short!("XLM");
            for _ in 0..5 {
                assert!(try_consume(&env, &xlm).is_ok());
            }
        });
    }

    #[test]
    fn over_limit_is_rejected_with_code() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 3,
                },
            );
            let xlm = symbol_short!("XLM");
            for _ in 0..3 {
                try_consume(&env, &xlm).unwrap();
            }
            assert_eq!(check(&env, &xlm), Err(JoinRateLimitError::Exceeded));
            assert!(!env.events().all().is_empty());
        });
    }

    #[test]
    fn deterministic_window_rolls_over_on_alignment() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            env.ledger().set_timestamp(10_000);
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 2,
                },
            );
            let xlm = symbol_short!("XLM");
            let start_window = current_window_id(&env);
            try_consume(&env, &xlm).unwrap();
            try_consume(&env, &xlm).unwrap();
            assert!(check(&env, &xlm).is_err());

            // Advance into the next aligned window.
            env.ledger().set_timestamp((start_window + 1) * 3600);
            assert_eq!(current_window_id(&env), start_window + 1);
            assert_eq!(current_count(&env, &xlm), 0);
            assert!(try_consume(&env, &xlm).is_ok());
        });
    }

    #[test]
    fn same_window_advance_does_not_reset() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            env.ledger().set_timestamp(3_600);
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 2,
                },
            );
            let xlm = symbol_short!("XLM");
            try_consume(&env, &xlm).unwrap();
            try_consume(&env, &xlm).unwrap();
            // Still inside the same window (3600 <= t < 7200).
            env.ledger().set_timestamp(7_199);
            assert!(check(&env, &xlm).is_err());
        });
    }

    #[test]
    fn distinct_assets_are_independent() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 2,
                },
            );
            let xlm = symbol_short!("XLM");
            let usdc = symbol_short!("USDC");
            try_consume(&env, &xlm).unwrap();
            try_consume(&env, &xlm).unwrap();
            assert!(check(&env, &xlm).is_err());
            assert!(check(&env, &usdc).is_ok());
        });
    }

    #[test]
    fn config_update_applies_immediately() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let xlm = symbol_short!("XLM");
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 1,
                },
            );
            try_consume(&env, &xlm).unwrap();
            assert!(check(&env, &xlm).is_err());

            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 5,
                },
            );
            assert!(check(&env, &xlm).is_ok());
        });
    }

    #[test]
    fn zero_budget_disables_joins() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 0,
                },
            );
            let xlm = symbol_short!("XLM");
            assert_eq!(check(&env, &xlm), Err(JoinRateLimitError::Disabled));
        });
    }

    #[test]
    fn zero_window_secs_is_floored_to_one() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 0,
                    max_joins_per_window: 1,
                },
            );
            assert_eq!(get_config(&env).window_secs, 1);
            let xlm = symbol_short!("XLM");
            env.ledger().set_timestamp(100);
            try_consume(&env, &xlm).unwrap();
            assert!(check(&env, &xlm).is_err());
            env.ledger().set_timestamp(101);
            assert!(check(&env, &xlm).is_ok());
        });
    }

    #[test]
    fn default_config_matches_documented_values() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            let cfg = get_config(&env);
            assert_eq!(cfg.window_secs, DEFAULT_JOIN_WINDOW_SECS);
            assert_eq!(cfg.max_joins_per_window, DEFAULT_MAX_JOINS_PER_WINDOW);
        });
    }

    #[test]
    fn uses_symbol_new_assets_too() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            set_config(
                &env,
                JoinRateLimitConfig {
                    window_secs: 3600,
                    max_joins_per_window: 1,
                },
            );
            let long_asset = Symbol::new(&env, "yXLM");
            try_consume(&env, &long_asset).unwrap();
            assert!(check(&env, &long_asset).is_err());
        });
    }
}
