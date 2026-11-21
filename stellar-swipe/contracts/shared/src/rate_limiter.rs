//! Generic fixed-window rate limiter shared across contracts (Issue #595).
//!
//! Previously, rate-limiting logic existed separately in `auto_trade::rate_limit`
//! and `common::rate_limit`, each implementing its own version of "is this
//! subject within `max_actions` for the current `window_secs`-second window."
//! This module consolidates that mechanism into one implementation: callers
//! identify a distinct use case with a `use_case: Symbol` key (e.g. one per
//! action type, or one per contract entrypoint) and a `subject: Address`
//! (typically the calling user), and supply their own `window_secs` /
//! `max_actions` configuration.
//!
//! Window semantics: a fixed window starting at the timestamp of the first
//! action in it. Once `window_secs` has elapsed since `window_start`, the
//! window resets (rather than sliding continuously), which is O(1) per check
//! — no per-action timestamp history to prune.

use soroban_sdk::{contracttype, Address, Env, Symbol};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitError {
    /// The subject has reached `max_actions` for this use case within the current window.
    Exceeded,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RateLimitWindow {
    pub window_start: u64,
    pub count: u32,
}

#[contracttype]
#[derive(Clone)]
enum RateLimiterKey {
    Window(Symbol, Address),
}

fn get_window(env: &Env, use_case: &Symbol, subject: &Address) -> Option<RateLimitWindow> {
    env.storage()
        .persistent()
        .get(&RateLimiterKey::Window(use_case.clone(), subject.clone()))
}

fn save_window(env: &Env, use_case: &Symbol, subject: &Address, window: &RateLimitWindow) {
    env.storage().persistent().set(
        &RateLimiterKey::Window(use_case.clone(), subject.clone()),
        window,
    );
}

/// Current action count for `subject` under `use_case` within the active window
/// (`0` if no window is active, i.e. never acted or the previous window expired).
pub fn current_count(env: &Env, use_case: &Symbol, subject: &Address, window_secs: u64) -> u32 {
    let now = env.ledger().timestamp();
    match get_window(env, use_case, subject) {
        Some(w) if now.saturating_sub(w.window_start) < window_secs => w.count,
        _ => 0,
    }
}

/// Check whether `subject` may perform one more action under `use_case`.
/// Returns `Ok(current_count)` if under `max_actions`, else `Err(Exceeded)`.
/// Does not mutate storage — call [`record`] after a successful, completed action.
pub fn check(
    env: &Env,
    use_case: &Symbol,
    subject: &Address,
    window_secs: u64,
    max_actions: u32,
) -> Result<u32, RateLimitError> {
    let count = current_count(env, use_case, subject, window_secs);
    if count >= max_actions {
        Err(RateLimitError::Exceeded)
    } else {
        Ok(count)
    }
}

/// Record one action for `subject` under `use_case`, rolling the window over if the
/// previous one has expired. Call after a successful [`check`].
pub fn record(env: &Env, use_case: &Symbol, subject: &Address, window_secs: u64) {
    let now = env.ledger().timestamp();
    let mut window = get_window(env, use_case, subject).unwrap_or(RateLimitWindow {
        window_start: now,
        count: 0,
    });

    if now.saturating_sub(window.window_start) >= window_secs {
        window.window_start = now;
        window.count = 0;
    }

    window.count = window.count.saturating_add(1);
    save_window(env, use_case, subject, &window);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl,
        testutils::{Address as _, Ledger},
        Symbol,
    };

    #[contract]
    struct RateLimiterHarness;

    #[contractimpl]
    impl RateLimiterHarness {}

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        let contract_id = env.register(RateLimiterHarness, ());
        let subject = Address::generate(&env);
        (env, contract_id, subject)
    }

    #[test]
    fn within_limit_calls_succeed() {
        let (env, contract_id, subject) = setup();
        env.as_contract(&contract_id, || {
            let use_case = Symbol::new(&env, "signal_submission");
            for _ in 0..5 {
                assert!(check(&env, &use_case, &subject, 3600, 5).is_ok());
                record(&env, &use_case, &subject, 3600);
            }
        });
    }

    #[test]
    fn over_limit_is_rejected() {
        let (env, contract_id, subject) = setup();
        env.as_contract(&contract_id, || {
            let use_case = Symbol::new(&env, "signal_submission");
            for _ in 0..5 {
                check(&env, &use_case, &subject, 3600, 5).unwrap();
                record(&env, &use_case, &subject, 3600);
            }
            assert_eq!(
                check(&env, &use_case, &subject, 3600, 5),
                Err(RateLimitError::Exceeded)
            );
        });
    }

    #[test]
    fn limit_resets_after_window_elapses() {
        let (env, contract_id, subject) = setup();
        env.as_contract(&contract_id, || {
            let use_case = Symbol::new(&env, "signal_submission");
            for _ in 0..5 {
                check(&env, &use_case, &subject, 3600, 5).unwrap();
                record(&env, &use_case, &subject, 3600);
            }
            assert!(check(&env, &use_case, &subject, 3600, 5).is_err());

            env.ledger()
                .set_timestamp(env.ledger().timestamp() + 3600 + 1);

            assert!(check(&env, &use_case, &subject, 3600, 5).is_ok());
        });
    }

    #[test]
    fn distinct_use_cases_are_independent() {
        let (env, contract_id, subject) = setup();
        env.as_contract(&contract_id, || {
            let a = Symbol::new(&env, "use_case_a");
            let b = Symbol::new(&env, "use_case_b");
            for _ in 0..3 {
                check(&env, &a, &subject, 3600, 3).unwrap();
                record(&env, &a, &subject, 3600);
            }
            assert!(check(&env, &a, &subject, 3600, 3).is_err());
            // A different use case for the same subject is unaffected.
            assert!(check(&env, &b, &subject, 3600, 3).is_ok());
        });
    }

    #[test]
    fn distinct_subjects_are_independent() {
        let (env, contract_id, subject_a) = setup();
        let subject_b = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let use_case = Symbol::new(&env, "withdraw");
            for _ in 0..3 {
                check(&env, &use_case, &subject_a, 3600, 3).unwrap();
                record(&env, &use_case, &subject_a, 3600);
            }
            assert!(check(&env, &use_case, &subject_a, 3600, 3).is_err());
            assert!(check(&env, &use_case, &subject_b, 3600, 3).is_ok());
        });
    }
}
