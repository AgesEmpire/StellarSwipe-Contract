#![cfg(test)]

use super::*;
use crate::risk;
use crate::storage;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger as _},
    Env,
};

fn setup_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1000);
    env
}

fn setup_signal(_env: &Env, signal_id: u64, expiry: u64) -> storage::Signal {
    storage::Signal {
        signal_id,
        price: 100,
        expiry,
        base_asset: 1,
    }
}

#[test]
fn test_execute_trade_invalid_amount() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let res =
            AutoTradeContract::execute_trade(env.clone(), user.clone(), 1, OrderType::Market, 0);

        assert_eq!(res, Err(AutoTradeError::InvalidAmount));
    });
}

#[test]
fn test_execute_trade_signal_not_found() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            999,
            OrderType::Market,
            100,
        );

        assert_eq!(res, Err(AutoTradeError::SignalNotFound));
    });
}

#[test]
fn test_execute_trade_signal_expired() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() - 1);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            100,
        );

        assert_eq!(res, Err(AutoTradeError::SignalExpired));
    });
}

#[test]
fn test_execute_trade_unauthorized() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            100,
        );

        assert_eq!(res, Err(AutoTradeError::Unauthorized));
    });
}

#[test]
fn test_execute_trade_insufficient_balance() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &50i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            100,
        );

        assert_eq!(res, Err(AutoTradeError::InsufficientBalance));
    });
}

#[test]
fn test_execute_trade_market_full_fill() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &500i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &500i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            400,
        )
        .unwrap();

        assert_eq!(res.trade.executed_amount, 400);
        assert_eq!(res.trade.executed_price, 100);
        assert_eq!(res.trade.status, TradeStatus::Filled);
    });
}

#[test]
fn test_execute_trade_market_partial_fill() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 2;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &500i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &100i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            300,
        )
        .unwrap();

        assert_eq!(res.trade.executed_amount, 100);
        assert_eq!(res.trade.executed_price, 100);
        assert_eq!(res.trade.status, TradeStatus::PartiallyFilled);
    });
}

#[test]
fn test_execute_trade_limit_filled() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 3;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &500i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("price"), signal_id), &90i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Limit,
            200,
        )
        .unwrap();

        assert_eq!(res.trade.executed_amount, 200);
        assert_eq!(res.trade.executed_price, 100);
        assert_eq!(res.trade.status, TradeStatus::Filled);
    });
}

#[test]
fn test_execute_trade_limit_not_filled() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 4;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &500i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("price"), signal_id), &150i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Limit,
            200,
        )
        .unwrap();

        assert_eq!(res.trade.executed_amount, 0);
        assert_eq!(res.trade.executed_price, 0);
        assert_eq!(res.trade.status, TradeStatus::Failed);
    });
}

#[test]
fn test_get_trade_existing() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &500i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &500i128);
    });

    env.as_contract(&contract_id, || {
        let _ = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            400,
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let trade = AutoTradeContract::get_trade(env.clone(), user.clone(), signal_id).unwrap();

        assert_eq!(trade.executed_amount, 400);
    });
}

#[test]
fn test_get_trade_non_existing() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 999;

    env.as_contract(&contract_id, || {
        let trade = AutoTradeContract::get_trade(env.clone(), user.clone(), signal_id);

        assert!(trade.is_none());
    });
}

// ========================================
// Risk Management Tests
// ========================================

#[test]
fn test_get_default_risk_config() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let config = AutoTradeContract::get_risk_config(env.clone(), user.clone());

        assert_eq!(config.max_position_pct, 20);
        assert_eq!(config.daily_trade_limit, 10);
        assert_eq!(config.stop_loss_pct, 15);
    });
}

#[test]
fn test_set_custom_risk_config() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let custom_config = risk::RiskConfig {
            max_position_pct: 30,
            daily_trade_limit: 15,
            stop_loss_pct: 10,
        };

        AutoTradeContract::set_risk_config(env.clone(), user.clone(), custom_config.clone());

        let retrieved = AutoTradeContract::get_risk_config(env.clone(), user.clone());
        assert_eq!(retrieved, custom_config);
    });
}

#[test]
fn test_position_limit_allows_first_trade() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &1000i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &1000i128);

        // First trade should be allowed
        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            1000,
        );

        assert!(res.is_ok());
    });
}

#[test]
fn test_get_user_positions() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &1000i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &500i128);

        // Execute a trade
        let _ = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            400,
        )
        .unwrap();

        // Check positions
        let positions = AutoTradeContract::get_user_positions(env.clone(), user.clone());
        assert!(positions.contains_key(1));

        let position = positions.get(1).unwrap();
        assert_eq!(position.amount, 400);
        assert_eq!(position.entry_price, 100);
    });
}

#[test]
fn test_stop_loss_check() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        // Setup a position with entry price 100
        risk::update_position(&env, &user, 1, 1000, 100);

        let config = risk::RiskConfig::default(); // 15% stop loss

        // Price at 90 (10% drop) - should NOT trigger
        let triggered = risk::check_stop_loss(&env, &user, 1, 90, &config);
        assert!(!triggered);

        // Price at 80 (20% drop) - should trigger
        let triggered = risk::check_stop_loss(&env, &user, 1, 80, &config);
        assert!(triggered);
    });
}

#[test]
fn test_get_trade_history_paginated() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    // Setup (max_position_pct: 100 so multiple buys in same asset pass risk checks)
    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        risk::set_risk_config(
            &env,
            &user,
            &risk::RiskConfig {
                max_position_pct: 100,
                daily_trade_limit: 10,
                stop_loss_pct: 15,
            },
        );
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &5000i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &5000i128);
    });

    // Execute 5 trades in separate frames (avoids "frame is already authorized")
    for _ in 0..5 {
        env.as_contract(&contract_id, || {
            let _ = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                100,
            )
            .unwrap();
        });
    }

    // Query history (no auth required)
    env.as_contract(&contract_id, || {
        let history = AutoTradeContract::get_trade_history(env.clone(), user.clone(), 0, 10);
        assert_eq!(history.len(), 5);

        let page2 = AutoTradeContract::get_trade_history(env.clone(), user.clone(), 2, 2);
        assert_eq!(page2.len(), 2);
    });
}

#[test]
fn test_get_trade_history_empty() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let history = AutoTradeContract::get_trade_history(env.clone(), user.clone(), 0, 20);
        assert_eq!(history.len(), 0);
    });
}

#[test]
fn test_get_portfolio() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user(&env, &user);
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &1000i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &500i128);

        let _ = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            400,
        )
        .unwrap();

        let portfolio = AutoTradeContract::get_portfolio(env.clone(), user.clone());
        assert_eq!(portfolio.assets.len(), 1);
        assert_eq!(portfolio.assets.get(0).unwrap().amount, 400);
        assert_eq!(portfolio.assets.get(0).unwrap().asset_id, 1);
    });
}

#[test]
fn test_portfolio_value_calculation() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        // Set up positions and prices
        risk::set_asset_price(&env, 1, 100);
        risk::set_asset_price(&env, 2, 200);

        risk::update_position(&env, &user, 1, 1000, 100);
        risk::update_position(&env, &user, 2, 500, 200);

        let total_value = risk::calculate_portfolio_value(&env, &user);
        // (1000 * 100 / 100) + (500 * 200 / 100) = 1000 + 1000 = 2000
        assert_eq!(total_value, 2000);
    });
}

// ========================================
// Authorization Tests
// ========================================

#[test]
fn test_grant_authorization_success() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let res = AutoTradeContract::grant_authorization(env.clone(), user.clone(), 500_0000000, 30);
        assert!(res.is_ok());

        let config = AutoTradeContract::get_auth_config(env.clone(), user.clone()).unwrap();
        assert_eq!(config.authorized, true);
        assert_eq!(config.max_trade_amount, 500_0000000);
        assert_eq!(config.expires_at, 1000 + (30 * 86400));
    });
}

#[test]
fn test_grant_authorization_zero_amount() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let res = AutoTradeContract::grant_authorization(env.clone(), user.clone(), 0, 30);
        assert_eq!(res, Err(AutoTradeError::InvalidAmount));
    });
}

#[test]
fn test_revoke_authorization() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 1000_0000000, 30)
            .unwrap();
        AutoTradeContract::revoke_authorization(env.clone(), user.clone()).unwrap();

        let config = AutoTradeContract::get_auth_config(env.clone(), user.clone());
        assert!(config.is_none());
    });
}

#[test]
fn test_trade_under_limit_succeeds() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 500_0000000, 30)
            .unwrap();
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &1000_0000000i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &1000_0000000i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            400_0000000,
        );
        assert!(res.is_ok());
    });
}

#[test]
fn test_trade_over_limit_fails() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 500_0000000, 30)
            .unwrap();
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &1000_0000000i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            600_0000000,
        );
        assert_eq!(res, Err(AutoTradeError::Unauthorized));
    });
}

#[test]
fn test_revoked_authorization_blocks_trade() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 1000_0000000, 30)
            .unwrap();
        AutoTradeContract::revoke_authorization(env.clone(), user.clone()).unwrap();

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            100_0000000,
        );
        assert_eq!(res, Err(AutoTradeError::Unauthorized));
    });
}

#[test]
fn test_expired_authorization_blocks_trade() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 100000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        // Grant with 1 day duration
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 1000_0000000, 1)
            .unwrap();

        // Fast forward time beyond expiry
        env.ledger().set_timestamp(1000 + 86400 + 1);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            100_0000000,
        );
        assert_eq!(res, Err(AutoTradeError::Unauthorized));
    });
}

#[test]
fn test_multiple_authorization_grants_latest_applies() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 500_0000000, 30)
            .unwrap();
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 1000_0000000, 60)
            .unwrap();

        let config = AutoTradeContract::get_auth_config(env.clone(), user.clone()).unwrap();
        assert_eq!(config.max_trade_amount, 1000_0000000);
        assert_eq!(config.expires_at, 1000 + (60 * 86400));
    });
}

#[test]
fn test_authorization_at_exact_limit() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 1;
    let signal = setup_signal(&env, signal_id, env.ledger().timestamp() + 1000);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        AutoTradeContract::grant_authorization(env.clone(), user.clone(), 500_0000000, 30)
            .unwrap();
        env.storage()
            .temporary()
            .set(&(user.clone(), symbol_short!("balance")), &1000_0000000i128);
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &1000_0000000i128);

        let res = AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            signal_id,
            OrderType::Market,
            500_0000000,
        );
        assert!(res.is_ok());
    });
}

// ========================================
// Rate Limit Tests
// ========================================

mod rate_limit_tests {
    use super::*;
    use crate::rate_limit::{BridgeRateLimits, ViolationType};
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    fn setup_with_rate_limits(env: &Env) -> (soroban_sdk::Address, soroban_sdk::Address) {
        let contract_id = env.register(AutoTradeContract, ());
        let admin = soroban_sdk::Address::generate(env);
        env.as_contract(&contract_id, || {
            AutoTradeContract::init_rate_limit_admin(env.clone(), admin.clone());
            // Tight limits for testing
            AutoTradeContract::set_rate_limits(
                env.clone(),
                BridgeRateLimits {
                    per_user_hourly_transfers: 3,
                    per_user_hourly_volume: 1_000,
                    per_user_daily_transfers: 10,
                    per_user_daily_volume: 5_000,
                    global_hourly_capacity: 100,
                    global_daily_volume: 50_000,
                    min_transfer_amount: 10,
                    cooldown_between_transfers: 0, // no cooldown for most tests
                },
            )
            .unwrap();
        });
        (contract_id, admin)
    }

    fn setup_signal_and_user(
        env: &Env,
        contract_id: &soroban_sdk::Address,
    ) -> (soroban_sdk::Address, u64) {
        let user = soroban_sdk::Address::generate(env);
        let signal_id = 42u64;
        env.as_contract(contract_id, || {
            storage::set_signal(
                env,
                signal_id,
                &storage::Signal {
                    signal_id,
                    price: 100,
                    expiry: env.ledger().timestamp() + 10_000,
                    base_asset: 1,
                },
            );
            storage::authorize_user(env, &user);
            env.storage()
                .temporary()
                .set(&(user.clone(), soroban_sdk::symbol_short!("balance")), &10_000i128);
            env.storage()
                .temporary()
                .set(&(soroban_sdk::symbol_short!("liquidity"), signal_id), &10_000i128);
        });
        (user, signal_id)
    }

    #[test]
    fn test_hourly_transfer_count_limit() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        // 3 trades should succeed (limit = 3)
        for _ in 0..3 {
            env.as_contract(&contract_id, || {
                AutoTradeContract::execute_trade(
                    env.clone(),
                    user.clone(),
                    signal_id,
                    OrderType::Market,
                    50,
                )
                .unwrap();
            });
        }

        // 4th should fail
        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            );
            assert_eq!(res, Err(AutoTradeError::HourlyTransferLimitExceeded));
        });
    }

    #[test]
    fn test_hourly_volume_limit() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        // First trade of 900 is fine (volume limit = 1000)
        env.as_contract(&contract_id, || {
            AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                900,
            )
            .unwrap();
        });

        // Second trade of 200 would exceed 1000 volume
        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                200,
            );
            assert_eq!(res, Err(AutoTradeError::HourlyVolumeLimitExceeded));
        });
    }

    #[test]
    fn test_below_minimum_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                5, // below min_transfer_amount = 10
            );
            assert_eq!(res, Err(AutoTradeError::BelowMinTransfer));
        });
    }

    #[test]
    fn test_cooldown_between_transfers() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        // Set cooldown to 60 seconds
        env.as_contract(&contract_id, || {
            AutoTradeContract::set_rate_limits(
                env.clone(),
                BridgeRateLimits {
                    per_user_hourly_transfers: 10,
                    per_user_hourly_volume: 100_000,
                    per_user_daily_transfers: 50,
                    per_user_daily_volume: 500_000,
                    global_hourly_capacity: 1000,
                    global_daily_volume: 5_000_000,
                    min_transfer_amount: 10,
                    cooldown_between_transfers: 60,
                },
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            )
            .unwrap();
        });

        // Immediate second trade should fail
        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            );
            assert_eq!(res, Err(AutoTradeError::CooldownNotElapsed));
        });

        // After cooldown passes it should succeed
        env.ledger().set_timestamp(10_000 + 61);
        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            );
            assert!(res.is_ok());
        });
    }

    #[test]
    fn test_whitelist_bypasses_rate_limits() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        // Whitelist the user
        env.as_contract(&contract_id, || {
            AutoTradeContract::add_to_whitelist(env.clone(), user.clone()).unwrap();
            assert!(AutoTradeContract::is_whitelisted(env.clone(), user.clone()));
        });

        // Should be able to exceed the hourly limit of 3
        for _ in 0..5 {
            env.as_contract(&contract_id, || {
                AutoTradeContract::execute_trade(
                    env.clone(),
                    user.clone(),
                    signal_id,
                    OrderType::Market,
                    50,
                )
                .unwrap();
            });
        }
    }

    #[test]
    fn test_penalty_blocks_user() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        // Apply a violation (1st = 1 hour penalty)
        env.as_contract(&contract_id, || {
            AutoTradeContract::record_violation(
                env.clone(),
                user.clone(),
                ViolationType::HourlyCountExceeded,
            )
            .unwrap();
        });

        // Trade should be blocked
        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            );
            assert_eq!(res, Err(AutoTradeError::RateLimitPenalty));
        });

        // After penalty expires it should work
        env.ledger().set_timestamp(10_000 + 3601);
        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            );
            assert!(res.is_ok());
        });
    }

    #[test]
    fn test_progressive_penalties() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, _) = setup_signal_and_user(&env, &contract_id);

        let expected_durations: &[(u32, u64)] = &[
            (1, 3600),
            (2, 3600),
            (3, 86400),
            (4, 86400),
            (5, 86400),
            (6, 604800),
        ];

        for (expected_count, expected_duration) in expected_durations {
            env.as_contract(&contract_id, || {
                AutoTradeContract::record_violation(
                    env.clone(),
                    user.clone(),
                    ViolationType::HourlyCountExceeded,
                )
                .unwrap();
                let history =
                    AutoTradeContract::get_user_rate_history(env.clone(), user.clone());
                assert_eq!(history.violation_count, *expected_count);
                assert_eq!(
                    history.penalty_until,
                    env.ledger().timestamp() + expected_duration
                );
            });
        }
    }

    #[test]
    fn test_global_capacity_limit() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);

        // Set global capacity to 2
        env.as_contract(&contract_id, || {
            AutoTradeContract::set_rate_limits(
                env.clone(),
                BridgeRateLimits {
                    per_user_hourly_transfers: 100,
                    per_user_hourly_volume: 1_000_000,
                    per_user_daily_transfers: 500,
                    per_user_daily_volume: 5_000_000,
                    global_hourly_capacity: 2,
                    global_daily_volume: 10_000_000,
                    min_transfer_amount: 10,
                    cooldown_between_transfers: 0,
                },
            )
            .unwrap();
        });

        // Two different users fill global capacity
        for _ in 0..2 {
            let u = soroban_sdk::Address::generate(&env);
            let sid = 99u64;
            env.as_contract(&contract_id, || {
                storage::set_signal(
                    &env,
                    sid,
                    &storage::Signal {
                        signal_id: sid,
                        price: 100,
                        expiry: env.ledger().timestamp() + 10_000,
                        base_asset: 1,
                    },
                );
                storage::authorize_user(&env, &u);
                env.storage()
                    .temporary()
                    .set(&(u.clone(), soroban_sdk::symbol_short!("balance")), &10_000i128);
                env.storage()
                    .temporary()
                    .set(&(soroban_sdk::symbol_short!("liquidity"), sid), &10_000i128);
                AutoTradeContract::execute_trade(
                    env.clone(),
                    u.clone(),
                    sid,
                    OrderType::Market,
                    50,
                )
                .unwrap();
            });
        }

        // Third user should hit global capacity
        let u3 = soroban_sdk::Address::generate(&env);
        let sid3 = 100u64;
        env.as_contract(&contract_id, || {
            storage::set_signal(
                &env,
                sid3,
                &storage::Signal {
                    signal_id: sid3,
                    price: 100,
                    expiry: env.ledger().timestamp() + 10_000,
                    base_asset: 1,
                },
            );
            storage::authorize_user(&env, &u3);
            env.storage()
                .temporary()
                .set(&(u3.clone(), soroban_sdk::symbol_short!("balance")), &10_000i128);
            env.storage()
                .temporary()
                .set(&(soroban_sdk::symbol_short!("liquidity"), sid3), &10_000i128);
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                u3.clone(),
                sid3,
                OrderType::Market,
                50,
            );
            assert_eq!(res, Err(AutoTradeError::GlobalCapacityExceeded));
        });
    }

    #[test]
    fn test_dynamic_adjustment_low_load() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);

        // global_hourly_capacity = 100, current load = 0 → 0% → relax
        env.as_contract(&contract_id, || {
            let before = AutoTradeContract::get_rate_limits(env.clone());
            AutoTradeContract::adjust_rate_limits(env.clone()).unwrap();
            let after = AutoTradeContract::get_rate_limits(env.clone());
            assert_eq!(
                after.per_user_hourly_transfers,
                (before.per_user_hourly_transfers + 1).min(20)
            );
        });
    }

    #[test]
    fn test_dynamic_adjustment_high_load() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        // Set capacity to 1 and execute 1 trade → 100% load
        env.as_contract(&contract_id, || {
            AutoTradeContract::set_rate_limits(
                env.clone(),
                BridgeRateLimits {
                    per_user_hourly_transfers: 10,
                    per_user_hourly_volume: 100_000,
                    per_user_daily_transfers: 50,
                    per_user_daily_volume: 500_000,
                    global_hourly_capacity: 1,
                    global_daily_volume: 5_000_000,
                    min_transfer_amount: 10,
                    cooldown_between_transfers: 0,
                },
            )
            .unwrap();
            AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            let before = AutoTradeContract::get_rate_limits(env.clone());
            AutoTradeContract::adjust_rate_limits(env.clone()).unwrap();
            let after = AutoTradeContract::get_rate_limits(env.clone());
            assert!(after.per_user_hourly_transfers <= before.per_user_hourly_transfers);
            assert!(after.cooldown_between_transfers >= before.cooldown_between_transfers);
        });
    }

    #[test]
    fn test_remove_from_whitelist_restores_limits() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (contract_id, _admin) = setup_with_rate_limits(&env);
        let (user, signal_id) = setup_signal_and_user(&env, &contract_id);

        env.as_contract(&contract_id, || {
            AutoTradeContract::add_to_whitelist(env.clone(), user.clone()).unwrap();
            AutoTradeContract::remove_from_whitelist(env.clone(), user.clone()).unwrap();
            assert!(!AutoTradeContract::is_whitelisted(env.clone(), user.clone()));
        });

        // Exhaust hourly limit
        for _ in 0..3 {
            env.as_contract(&contract_id, || {
                AutoTradeContract::execute_trade(
                    env.clone(),
                    user.clone(),
                    signal_id,
                    OrderType::Market,
                    50,
                )
                .unwrap();
            });
        }

        env.as_contract(&contract_id, || {
            let res = AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                50,
            );
            assert_eq!(res, Err(AutoTradeError::HourlyTransferLimitExceeded));
        });
    }
}
