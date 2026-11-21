use soroban_sdk::{symbol_short, Address, Env, Symbol};

use crate::admin;
use crate::errors::AutoTradeError;
use crate::storage::{
    self, clear_loss_streak_paused, get_loss_streak_config, get_loss_streak_counter,
    is_loss_streak_paused, set_loss_streak_counter, set_loss_streak_paused, LossStreakConfig,
    LossStreakCounter,
};
use crate::TradeStatus;

fn emit_loss_streak_pause(env: &Env, user: &Address, threshold: u32) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "loss_streak_pause"),
            user.clone(),
            threshold,
        ),
        (),
    );
}

pub fn check_loss_streak_paused(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    if is_loss_streak_paused(env, user) {
        return Err(AutoTradeError::TradingPaused);
    }
    Ok(())
}

pub fn record_trade_outcome(env: &Env, user: &Address, status: &TradeStatus) {
    let now = env.ledger().timestamp();

    match status {
        TradeStatus::Filled | TradeStatus::PartiallyFilled => {
            let counter = LossStreakCounter {
                consecutive_losses: 0,
                updated_at: now,
            };
            set_loss_streak_counter(env, user, &counter);
            clear_loss_streak_paused(env, user);
        }
        TradeStatus::Failed => {
            let mut counter = get_loss_streak_counter(env, user);
            counter.consecutive_losses = counter.consecutive_losses.saturating_add(1);
            counter.updated_at = now;
            set_loss_streak_counter(env, user, &counter);

            let config = get_loss_streak_config(env);
            if counter.consecutive_losses >= config.threshold {
                set_loss_streak_paused(env, user);
                emit_loss_streak_pause(env, user, config.threshold);

                crate::logging::emit_log(
                    env,
                    crate::logging::LogLevel::Critical,
                    soroban_sdk::String::from_str(env, "loss_streak"),
                    soroban_sdk::String::from_str(env, "auto_paused"),
                    None,
                );
            }
        }
        TradeStatus::Pending => {}
    }
}

pub fn set_loss_streak_threshold(
    env: &Env,
    caller: &Address,
    threshold: u32,
) -> Result<(), AutoTradeError> {
    if threshold == 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let admin = admin::get_admin(env).ok_or(AutoTradeError::Unauthorized)?;
    if caller != &admin {
        return Err(AutoTradeError::Unauthorized);
    }
    let config = storage::LossStreakConfig { threshold };
    storage::set_loss_streak_config(env, &config);
    Ok(())
}

pub fn resume_after_loss_streak(env: &Env, user: &Address) -> Result<(), AutoTradeError> {
    if !is_loss_streak_paused(env, user) {
        return Err(AutoTradeError::NotPaused);
    }
    clear_loss_streak_paused(env, user);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::init_admin;
    use crate::AutoTradeContract;
    use soroban_sdk::{
        contract,
        testutils::{Address as _, Events as _, Ledger as _},
        Address, Env, Symbol, TryFromVal, Val,
    };

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1000);
        let contract_id = env.register(AutoTradeContract, ());
        let admin = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init_admin(&env, admin.clone());
        });
        (env, contract_id, admin)
    }

    #[test]
    fn test_loss_streak_increments_on_failure() {
        let (env, _cid, _admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 0);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 1);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 2);
        });
    }

    #[test]
    fn test_successful_trade_resets_counter() {
        let (env, _cid, _admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 3);
            record_trade_outcome(&env, &user, &TradeStatus::Filled);
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 0);
        });
    }

    #[test]
    fn test_partial_fill_resets_counter() {
        let (env, _cid, _admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::PartiallyFilled);
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 0);
        });
    }

    #[test]
    fn test_threshold_triggers_pause_and_event() {
        let (env, _cid, admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_loss_streak_threshold(&env, &admin, 3).unwrap();
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            assert!(!is_loss_streak_paused(&env, &user));
            let _existing = env.events().all();
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            assert!(is_loss_streak_paused(&env, &user));
            let events = env.events().all();
            let found = events.iter().any(|event| {
                let topics: soroban_sdk::Vec<Val> = event.1.clone();
                if topics.len() < 3 {
                    return false;
                }
                let sym = Symbol::try_from_val(&env, &topics.get(0).unwrap())
                    .unwrap_or(Symbol::new(&env, ""));
                sym == Symbol::new(&env, "loss_streak_pause")
            });
            assert!(found, "Expected loss_streak_pause event");
        });
    }

    #[test]
    fn test_check_blocks_paused_user() {
        let (env, _cid, admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_loss_streak_threshold(&env, &admin, 2).unwrap();
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            assert!(is_loss_streak_paused(&env, &user));
            assert_eq!(
                check_loss_streak_paused(&env, &user),
                Err(AutoTradeError::TradingPaused)
            );
        });
    }

    #[test]
    fn test_resume_clears_pause() {
        let (env, _cid, admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_loss_streak_threshold(&env, &admin, 2).unwrap();
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            assert!(is_loss_streak_paused(&env, &user));
            resume_after_loss_streak(&env, &user).unwrap();
            assert!(!is_loss_streak_paused(&env, &user));
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 2);
            assert_eq!(check_loss_streak_paused(&env, &user), Ok(()));
        });
    }

    #[test]
    fn test_resume_fails_if_not_paused() {
        let (env, _cid, _admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            assert_eq!(
                resume_after_loss_streak(&env, &user),
                Err(AutoTradeError::NotPaused)
            );
        });
    }

    #[test]
    fn test_set_threshold_zero_rejected() {
        let (env, _cid, admin) = setup();
        env.as_contract(&_cid, || {
            assert_eq!(
                set_loss_streak_threshold(&env, &admin, 0),
                Err(AutoTradeError::InvalidAmount)
            );
        });
    }

    #[test]
    fn test_successful_trade_clears_pause() {
        let (env, _cid, admin) = setup();
        let user = Address::generate(&env);
        env.as_contract(&_cid, || {
            set_loss_streak_threshold(&env, &admin, 2).unwrap();
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            record_trade_outcome(&env, &user, &TradeStatus::Failed);
            assert!(is_loss_streak_paused(&env, &user));
            record_trade_outcome(&env, &user, &TradeStatus::Filled);
            assert!(!is_loss_streak_paused(&env, &user));
            let counter = get_loss_streak_counter(&env, &user);
            assert_eq!(counter.consecutive_losses, 0);
        });
    }

    #[test]
    fn test_default_threshold_is_five() {
        let env = Env::default();
        let contract_id = env.register(AutoTradeContract, ());
        env.as_contract(&contract_id, || {
            let config = get_loss_streak_config(&env);
            assert_eq!(config.threshold, 5);
        });
    }
}
