use soroban_sdk::{Address, Env, Symbol};

use crate::errors::AutoTradeError;
use crate::storage::{
    self, get_daily_cap_config, get_daily_execution_counter, set_daily_execution_counter,
    DailyExecutionCounter,
};

/// Rolling 24-hour window in seconds.
const WINDOW_SECONDS: u64 = 86400;

/// Emit a `daily_execution_cap_blocked` event.
fn emit_daily_cap_blocked(env: &Env, user: &Address, max_executions: u32) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "daily_execution_cap_blocked"),
            user.clone(),
            max_executions,
        ),
        (),
    );
}

/// Check whether the user has exceeded their daily execution cap.
///
/// Called before trade execution.  Resets the rolling-window counter when
/// the 24-hour window has elapsed (ledger-time-based), avoiding an unbounded
/// scan of historical executions.
pub fn check_daily_execution_cap(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    let config = get_daily_cap_config(env, user);
    let mut counter = get_daily_execution_counter(env, user);
    let now = env.ledger().timestamp();

    // Reset if the 24-hour window has elapsed (persist the reset)
    if counter.window_start == 0 || now >= counter.window_start.saturating_add(WINDOW_SECONDS) {
        counter = DailyExecutionCounter {
            count: 0,
            window_start: now,
        };
        set_daily_execution_counter(env, user, &counter);
    }

    if counter.count >= config.max_executions {
        emit_daily_cap_blocked(env, user, config.max_executions);
        return Err(AutoTradeError::DailyTradeLimitExceeded);
    }

    Ok(())
}

/// Record an auto-trade execution attempt, incrementing the rolling counter.
///
/// Called after a trade is executed (regardless of outcome) so the counter
/// reflects all attempts, not just successful fills.
pub fn record_execution(env: &Env, user: &Address) {
    let mut counter = get_daily_execution_counter(env, user);
    let now = env.ledger().timestamp();

    // Reset if the 24-hour window has elapsed
    if counter.window_start == 0 || now >= counter.window_start.saturating_add(WINDOW_SECONDS) {
        counter = DailyExecutionCounter {
            count: 0,
            window_start: now,
        };
    }

    counter.count = counter.count.saturating_add(1);
    set_daily_execution_counter(env, user, &counter);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::init_admin;
    use crate::storage::{get_daily_cap_config, set_daily_cap_config, DailyCapConfig};
    use crate::AutoTradeContract;
    use soroban_sdk::{
        contract,
        testutils::{Address as _, Events as _, Ledger as _},
        Address, Env, Symbol, TryFromVal, Val,
    };

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1000);
        let contract_id = env.register(AutoTradeContract, ());
        let admin = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init_admin(&env, admin.clone());
        });
        (env, contract_id)
    }

    #[test]
    fn test_executions_under_cap_allowed() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            // Default cap is 10, counter starts at 0 → should be allowed
            assert!(check_daily_execution_cap(&env, &user).is_ok());

            // Record 5 executions
            for _ in 0..5 {
                record_execution(&env, &user);
            }

            // Still under the cap
            assert!(check_daily_execution_cap(&env, &user).is_ok());
        });
    }

    #[test]
    fn test_execution_at_cap_is_rejected() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_daily_cap_config(&env, &user, &DailyCapConfig { max_executions: 3 });

            for _ in 0..3 {
                assert!(check_daily_execution_cap(&env, &user).is_ok());
                record_execution(&env, &user);
            }

            assert_eq!(
                check_daily_execution_cap(&env, &user),
                Err(AutoTradeError::DailyTradeLimitExceeded)
            );
        });
    }

    #[test]
    fn test_cap_block_emits_event() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_daily_cap_config(&env, &user, &DailyCapConfig { max_executions: 1 });

            assert!(check_daily_execution_cap(&env, &user).is_ok());
            record_execution(&env, &user);

            let result = check_daily_execution_cap(&env, &user);
            assert_eq!(result, Err(AutoTradeError::DailyTradeLimitExceeded));

            let events = env.events().all();
            let found = events.iter().any(|event| {
                let topics: soroban_sdk::Vec<Val> = event.1.clone();
                if topics.len() < 1 {
                    return false;
                }
                let sym = Symbol::try_from_val(&env, &topics.get(0).unwrap())
                    .unwrap_or(Symbol::new(&env, ""));
                sym == Symbol::new(&env, "daily_execution_cap_blocked")
            });
            assert!(found, "Expected daily_execution_cap_blocked event");
        });
    }

    #[test]
    fn test_window_rollover_allows_execution() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_daily_cap_config(&env, &user, &DailyCapConfig { max_executions: 2 });

            // Use up both slots
            for _ in 0..2 {
                assert!(check_daily_execution_cap(&env, &user).is_ok());
                record_execution(&env, &user);
            }

            assert_eq!(
                check_daily_execution_cap(&env, &user),
                Err(AutoTradeError::DailyTradeLimitExceeded)
            );

            // Advance ledger past the 24-hour window
            env.ledger().set_timestamp(1000 + WINDOW_SECONDS + 1);

            // Window has rolled over → execution should be allowed again
            assert!(check_daily_execution_cap(&env, &user).is_ok());
        });
    }

    #[test]
    fn test_window_rollover_resets_count() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_daily_cap_config(&env, &user, &DailyCapConfig { max_executions: 3 });

            // Record 2 executions
            for _ in 0..2 {
                record_execution(&env, &user);
            }

            let counter_before = get_daily_execution_counter(&env, &user);
            assert_eq!(counter_before.count, 2);

            // Advance past the window
            env.ledger().set_timestamp(1000 + WINDOW_SECONDS + 1);

            // Check triggers a reset
            assert!(check_daily_execution_cap(&env, &user).is_ok());

            let counter_after = get_daily_execution_counter(&env, &user);
            assert_eq!(
                counter_after.count, 0,
                "Counter should reset to 0 after window rollover"
            );
            assert_eq!(
                counter_after.window_start,
                1000 + WINDOW_SECONDS + 1,
                "window_start should be updated to current time"
            );
        });
    }

    #[test]
    fn test_default_cap_config() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            let config = get_daily_cap_config(&env, &user);
            assert_eq!(config.max_executions, 10);
        });
    }

    #[test]
    fn test_set_custom_cap_config() {
        let (env, _cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_daily_cap_config(&env, &user, &DailyCapConfig { max_executions: 25 });
            let config = get_daily_cap_config(&env, &user);
            assert_eq!(config.max_executions, 25);
        });
    }
}
