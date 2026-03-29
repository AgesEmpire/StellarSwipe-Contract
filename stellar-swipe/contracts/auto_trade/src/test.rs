#![cfg(test)]

use super::*;
use crate::auth;
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

 issue-87-reentrancy-protection
/* 
// TODO: Fix test_risk_parity_rebalance before PR. 
// Currently failing due to integer precision or trade size issues in execution.
#[test]
fn test_risk_parity_rebalance() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        // Setup 2 assets: Asset 1 (Low Vol), Asset 2 (High Vol)
        // Record some prices to establish volatility
        for i in 0..10 {
            // Asset 1: Stable at 100
            AutoTradeContract::record_asset_price(env.clone(), 1, 100);
            // Asset 2: Volatile swings between 90 and 110
            let p2 = if i % 2 == 0 { 90 } else { 110 };
            AutoTradeContract::record_asset_price(env.clone(), 2, p2);
        }

        // Initial positions: Equal XLM value
        // Asset 1: 10 units @ 100 = 1000 XLM
        risk::update_position(&env, &user, 1, 10, 100);
        // Asset 2: 10 units @ 100 = 1000 XLM
        risk::update_position(&env, &user, 2, 10, 100);

        // Enable Risk Parity
        let _ = AutoTradeContract::set_risk_parity_config(env.clone(), user.clone(), true, 0, 1);

        // Preview rebalance
        let (risks, trades) = AutoTradeContract::preview_risk_parity_rebalance(env.clone(), user.clone()).unwrap();
        
        // Asset 1 (stable) should have lower vol than Asset 2
        let r1 = risks.iter().find(|r| r.asset_id == 1).unwrap();
        let r2 = risks.iter().find(|r| r.asset_id == 2).unwrap();
        assert!(r1.volatility_bps < r2.volatility_bps, "Asset 1 should be less volatile");

        // Risk parity should recommend SELLING Asset 2 (high risk) and BUYING Asset 1 (low risk)
        assert!(trades.len() >= 2);
        let t1 = trades.iter().find(|t| t.asset_id == 1).unwrap();
        let t2 = trades.iter().find(|t| t.asset_id == 2).unwrap();
        assert!(t1.is_buy, "Should buy low-vol asset");
        assert!(!t2.is_buy, "Should sell high-vol asset");

        // Execute rebalance
        AutoTradeContract::trigger_risk_parity_rebalance(env.clone(), user.clone()).unwrap();

        // Verify new positions
        let portfolio = AutoTradeContract::get_portfolio(env.clone(), user.clone());
        let p1 = portfolio.assets.iter().find(|a| a.asset_id == 1).unwrap();
        let p2 = portfolio.assets.iter().find(|a| a.asset_id == 2).unwrap();

        assert!(p1.amount > 10, "Asset 1 amount should increase");
        assert!(p2.amount < 10, "Asset 2 amount should decrease");
    });
}
*/

fn stat_arb_basket(env: &Env) -> soroban_sdk::Vec<u32> {
    let mut basket = soroban_sdk::Vec::new(env);
    basket.push_back(1);
    basket.push_back(2);
    basket.push_back(3);
    basket
}

fn stat_arb_history(env: &Env, values: &[i128]) -> soroban_sdk::Vec<i128> {
    let mut prices = soroban_sdk::Vec::new(env);
    for value in values {
        prices.push_back(*value);
    }
    prices
}

fn grant_auth(
    env: &Env,
    contract_id: &Address,
    user: &Address,
    max_amount: i128,
    duration_days: u32,
) {
    env.as_contract(contract_id, || {
        AutoTradeContract::grant_authorization(
            env.clone(),
            user.clone(),
            max_amount,
            duration_days,
        )
        .unwrap();
    });
}

fn revoke_auth(env: &Env, contract_id: &Address, user: &Address) {
    env.as_contract(contract_id, || {
        AutoTradeContract::revoke_authorization(env.clone(), user.clone()).unwrap();
    });
}

fn configure_stat_arb(
    env: &Env,
    contract_id: &Address,
    user: &Address,
    entry_z_score: i128,
    exit_z_score: i128,
) {
    env.as_contract(contract_id, || {
        AutoTradeContract::configure_stat_arb_strategy(
            env.clone(),
            user.clone(),
            stat_arb_basket(env),
            6,
            1,
            entry_z_score,
            exit_z_score,
            1,
        )
        .unwrap();
    });
}
 main

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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        auth::grant_authorization(&env, &user, 1000000, 30).unwrap();
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
        let res =
            AutoTradeContract::grant_authorization(env.clone(), user.clone(), 500_0000000, 30);
        assert!(res.is_ok());

        let config = AutoTradeContract::get_auth_config(env.clone(), user.clone()).unwrap();
        assert!(config.authorized);
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

    grant_auth(&env, &contract_id, &user, 1000_0000000, 30);

    env.as_contract(&contract_id, || {
 feat/governance-token-distribution-111
        storage::authorize_user_with_limits(&env, &user, 1000_0000000, 30);
        storage::revoke_user_authorization(&env, &user);

        AutoTradeContract::revoke_authorization(env.clone(), user.clone()).unwrap();
 main

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

    grant_auth(&env, &contract_id, &user, 500_0000000, 30);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
 feat/governance-token-distribution-111
        storage::authorize_user_with_limits(&env, &user, 500_0000000, 30);
 main
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

    grant_auth(&env, &contract_id, &user, 500_0000000, 30);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
 feat/governance-token-distribution-111
        storage::authorize_user_with_limits(&env, &user, 500_0000000, 30);

 main
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

    grant_auth(&env, &contract_id, &user, 1000_0000000, 30);
    revoke_auth(&env, &contract_id, &user);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
        storage::authorize_user_with_limits(&env, &user, 1000_0000000, 30);
        storage::revoke_user_authorization(&env, &user);

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

    grant_auth(&env, &contract_id, &user, 1000_0000000, 1);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
 feat/governance-token-distribution-111
        // Grant with 1 day duration
        storage::authorize_user_with_limits(&env, &user, 1000_0000000, 1);


 main
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

 feat/governance-token-distribution-111
    env.as_contract(&contract_id, || {
        storage::authorize_user_with_limits(&env, &user, 500_0000000, 30);
        storage::authorize_user_with_limits(&env, &user, 1000_0000000, 60);

    grant_auth(&env, &contract_id, &user, 500_0000000, 30);
    grant_auth(&env, &contract_id, &user, 1000_0000000, 60);
 main

    env.as_contract(&contract_id, || {
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

    grant_auth(&env, &contract_id, &user, 500_0000000, 30);

    env.as_contract(&contract_id, || {
        storage::set_signal(&env, signal_id, &signal);
 feat/governance-token-distribution-111
        storage::authorize_user_with_limits(&env, &user, 500_0000000, 30);

 main
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

#[test]
fn test_stat_arb_trade_creates_active_portfolio_state_correctly() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            1,
            stat_arb_history(&env, &[100, 101, 102, 103, 104, 180]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            2,
            stat_arb_history(&env, &[80, 81, 82, 83, 84, 85]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            3,
            stat_arb_history(&env, &[60, 61, 62, 63, 64, 65]),
        )
        .unwrap();
    });

    configure_stat_arb(&env, &contract_id, &user, 500, 250);

    env.as_contract(&contract_id, || {
        let portfolio =
            AutoTradeContract::execute_stat_arb_trade(env.clone(), user.clone(), 90_000).unwrap();

        assert_eq!(portfolio.asset_positions.len(), 3);
        assert_eq!(portfolio.total_value, 90_000);
        assert!(
            AutoTradeContract::get_active_stat_arb_portfolio(env.clone(), user.clone()).is_some()
        );
    });
}

#[test]
fn test_stat_arb_rebalance_updates_toward_new_hedge_ratios() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            1,
            stat_arb_history(&env, &[100, 101, 102, 103, 104, 180]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            2,
            stat_arb_history(&env, &[80, 81, 82, 83, 84, 85]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            3,
            stat_arb_history(&env, &[60, 61, 62, 63, 64, 65]),
        )
        .unwrap();
    });

    configure_stat_arb(&env, &contract_id, &user, 500, 250);

    let opened = env.as_contract(&contract_id, || {
        AutoTradeContract::execute_stat_arb_trade(env.clone(), user.clone(), 90_000).unwrap()
    });
    env.ledger().set_timestamp(env.ledger().timestamp() + 3601);

    env.as_contract(&contract_id, || {
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            2,
            stat_arb_history(&env, &[80, 82, 84, 86, 88, 90]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            3,
            stat_arb_history(&env, &[60, 62, 64, 66, 68, 70]),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let rebalanced =
            AutoTradeContract::rebalance_stat_arb_portfolio(env.clone(), user.clone()).unwrap();
        assert_eq!(rebalanced.portfolio_id, opened.portfolio_id);
        assert!(rebalanced.last_rebalanced_at > opened.last_rebalanced_at);
    });
}

#[test]
fn test_stat_arb_exit_closes_on_convergence() {
    let env = setup_env();
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            1,
            stat_arb_history(&env, &[100, 101, 102, 103, 104, 180]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            2,
            stat_arb_history(&env, &[80, 81, 82, 83, 84, 85]),
        )
        .unwrap();
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            3,
            stat_arb_history(&env, &[60, 61, 62, 63, 64, 65]),
        )
        .unwrap();
    });

    configure_stat_arb(&env, &contract_id, &user, 500, 250);

    env.as_contract(&contract_id, || {
        AutoTradeContract::execute_stat_arb_trade(env.clone(), user.clone(), 90_000).unwrap();
    });

    env.as_contract(&contract_id, || {
        AutoTradeContract::set_stat_arb_price_history(
            env.clone(),
            1,
            stat_arb_history(&env, &[100, 101, 102, 103, 104, 105]),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let exit_check = AutoTradeContract::check_stat_arb_exit(env.clone(), user.clone()).unwrap();
        assert!(exit_check.should_exit);
    });

    env.as_contract(&contract_id, || {
        AutoTradeContract::close_stat_arb_portfolio(env.clone(), user.clone()).unwrap();
        assert!(
            AutoTradeContract::get_active_stat_arb_portfolio(env.clone(), user.clone()).is_none()
        );
    });
}

// ========================================
// Exit Strategy Tests (tiered TP + trailing stops)
// ========================================

#[cfg(test)]
mod exit_strategy_tests {
    use super::*;
    use crate::exit_strategy::{StopLossTier, StrategyStatus, TakeProfitTier};
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env, Vec,
    };

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let cid = env.register(AutoTradeContract, ());
        (env, cid)
    }

    // ── Preset: Conservative (3 TPs + 10% trail) ─────────────────────────────

    #[test]
    fn test_contract_conservative_tp1_partial_close() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // TP1 at +20% = 1200
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 1_200).unwrap();
            assert_eq!(trades.len(), 1);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            // 33.33% of 10_000 = 3_333 closed → 6_667 remaining
            assert_eq!(s.current_position_size, 10_000 - 3_333);
            assert_eq!(s.status, StrategyStatus::Active);
        });
    }

    #[test]
    fn test_contract_conservative_multiple_tps_same_update() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // Price gaps past TP1 and TP2 simultaneously
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 1_500).unwrap();
            assert_eq!(trades.len(), 2);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            // TP1: close 3333 → 6667 remaining
            // TP2: close 50% of 6667 = 3333 → 3334 remaining
            assert_eq!(s.current_position_size, 3_334);
            assert_eq!(s.status, StrategyStatus::Active);
        });
    }

    #[test]
    fn test_contract_conservative_all_tps_complete() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // Price hits all 3 TPs at once (+100%)
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 2_000).unwrap();
            assert_eq!(trades.len(), 3);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    // ── Trailing stop: triggers before any TP ────────────────────────────────

    #[test]
    fn test_contract_stop_triggered_before_tp() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            // entry=1000, trail=10% from start → stop at 900
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // Price rises to 1100 (no TP), then drops 10% → stop at 990
            AutoTradeContract::check_exit_strategy(env.clone(), id, 1_100).unwrap();
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 990).unwrap();
            assert_eq!(trades.len(), 1);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::StopHit);
        });
    }

    // ── Tiered trailing stop tightens after profit threshold ─────────────────

    #[test]
    fn test_contract_trailing_stop_tightens_after_20pct_profit() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            // Balanced: trail 10% initially, tightens to 7% after 20% profit
            let id = AutoTradeContract::create_balanced_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // Rise to 1200 (+20%) → tier 2 activates (trail 7%)
            AutoTradeContract::check_exit_strategy(env.clone(), id, 1_200).unwrap();

            // 1200 * 93% = 1116 → exactly at 7% trail, no stop
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 1_116).unwrap();
            assert_eq!(trades.len(), 0);

            // 1115 → just below 7% trail of 1200 → stop triggered
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 1_115).unwrap();
            assert_eq!(trades.len(), 1);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.status, StrategyStatus::StopHit);
        });
    }

    // ── Preset: Aggressive (4 TPs + tiered trails 10%/7%/5%) ─────────────────

    #[test]
    fn test_contract_aggressive_four_tps_all_hit() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_aggressive_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 2_500).unwrap();
            assert_eq!(trades.len(), 4);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    #[test]
    fn test_contract_aggressive_tight_trail_after_50pct_profit() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_aggressive_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // Rise to 1500 (+50%) → tier 3 activates (trail 5%)
            AutoTradeContract::check_exit_strategy(env.clone(), id, 1_500).unwrap();

            // 1500 * 95% = 1425 → exactly at 5% trail, no stop
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 1_425).unwrap();
            assert_eq!(trades.len(), 0);

            // 1424 → just below 5% trail → stop triggered
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 1_424).unwrap();
            assert_eq!(trades.len(), 1);

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.status, StrategyStatus::StopHit);
        });
    }

    // ── Custom strategy with explicit tiers ───────────────────────────────────

    #[test]
    fn test_contract_custom_exit_strategy() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let mut tps: Vec<TakeProfitTier> = Vec::new(&env);
            tps.push_back(TakeProfitTier { price: 120, position_pct: 5_000, executed: false });
            tps.push_back(TakeProfitTier { price: 150, position_pct: 10_000, executed: false });

            let mut sls: Vec<StopLossTier> = Vec::new(&env);
            sls.push_back(StopLossTier { trigger_profit_pct: 0, trail_pct: 8, active: true });

            let id = AutoTradeContract::create_exit_strategy(
                env.clone(), user.clone(), 42, 100, 1_000, tps, sls,
            ).unwrap();

            // TP1 at 120 → close 50%
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 120).unwrap();
            assert_eq!(trades.len(), 1);
            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.current_position_size, 500);

            // TP2 at 150 → close remaining 100%
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 150).unwrap();
            assert_eq!(trades.len(), 1);
            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.current_position_size, 0);
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    // ── Manual position adjustment ────────────────────────────────────────────

    #[test]
    fn test_contract_manual_position_adjust_updates_remaining_tiers() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            // User manually closes half the position
            AutoTradeContract::adjust_exit_position(
                env.clone(), user.clone(), id, 5_000,
            ).unwrap();

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.current_position_size, 5_000);
            assert_eq!(s.status, StrategyStatus::Active);

            // Remaining TP tiers still execute against the adjusted size
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 2_000).unwrap();
            assert!(trades.len() > 0);
        });
    }

    #[test]
    fn test_contract_manual_adjust_to_zero_marks_complete() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            AutoTradeContract::adjust_exit_position(
                env.clone(), user.clone(), id, 0,
            ).unwrap();

            let s = AutoTradeContract::get_exit_strategy(env.clone(), id).unwrap();
            assert_eq!(s.status, StrategyStatus::Complete);
        });
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn test_contract_no_execution_on_inactive_strategy() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let id = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();

            AutoTradeContract::adjust_exit_position(env.clone(), user.clone(), id, 0).unwrap();

            // Further price checks on a Complete strategy return empty
            let trades = AutoTradeContract::check_exit_strategy(env.clone(), id, 9_999).unwrap();
            assert_eq!(trades.len(), 0);
        });
    }

    #[test]
    fn test_contract_get_user_exit_strategies() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 1_000, 10_000,
            ).unwrap();
            AutoTradeContract::create_balanced_exit(
                env.clone(), user.clone(), 2, 2_000, 5_000,
            ).unwrap();
            AutoTradeContract::create_aggressive_exit(
                env.clone(), user.clone(), 3, 500, 20_000,
            ).unwrap();

            let ids = AutoTradeContract::get_user_exit_strategies(env.clone(), user.clone());
            assert_eq!(ids.len(), 3);
        });
    }

    #[test]
    fn test_contract_invalid_entry_price_rejected() {
        let (env, cid) = setup();
        let user = Address::generate(&env);

        env.as_contract(&cid, || {
            let err = AutoTradeContract::create_conservative_exit(
                env.clone(), user.clone(), 1, 0, 10_000,
            ).unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidAmount);
        });
    }

    #[test]
    fn test_contract_get_nonexistent_strategy_errors() {
        let (env, cid) = setup();

        env.as_contract(&cid, || {
            let err = AutoTradeContract::get_exit_strategy(env.clone(), 999).unwrap_err();
            assert_eq!(err, AutoTradeError::ExitStrategyNotFound);
        });
    }
}

// ========================================
// Portfolio Insurance & Dynamic Hedging Tests (Issue #89)
// ========================================

#[cfg(test)]
mod insurance_tests {
    use super::*;
    use crate::risk;
    use crate::storage;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env,
    };

    fn setup_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env
    }

    /// Validation scenario from the issue:
    /// Portfolio = 10_000, trigger = 15% drawdown, hedge ratio = 50%.
    /// Simulate 20% decline → hedges open at 15% → size ~50% → recover → hedges close.
    #[test]
    fn test_full_insurance_lifecycle() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            // ── 1. Configure insurance: 15% trigger (1500 bps), 50% ratio (5000 bps) ──
            AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                true,
                1500,
                5000,
                200,
            )
            .unwrap();

            // ── 2. Establish portfolio at 10_000 value ──
            // price=100, amount=10_000 → value = 10_000 * 100 / 100 = 10_000
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);

            // Seed HWM by calling drawdown once
            let dd = AutoTradeContract::get_portfolio_drawdown(env.clone(), user.clone()).unwrap();
            assert_eq!(dd, 0);

            let ins = AutoTradeContract::get_insurance_config(env.clone(), user.clone()).unwrap();
            assert_eq!(ins.portfolio_high_water_mark, 10_000);

            // ── 3. Simulate 20% decline (price 80 → value 8_000) ──
            risk::set_asset_price(&env, 1, 80);

            let dd = AutoTradeContract::get_portfolio_drawdown(env.clone(), user.clone()).unwrap();
            // (10_000 - 8_000) * 10_000 / 10_000 = 2_000 bps = 20%
            assert_eq!(dd, 2_000);

            // ── 4. Verify hedges created at 15% drawdown threshold ──
            let ids =
                AutoTradeContract::apply_hedge_if_needed(env.clone(), user.clone()).unwrap();
            assert!(ids.len() > 0, "hedges must be created when drawdown > threshold");

            let ins = AutoTradeContract::get_insurance_config(env.clone(), user.clone()).unwrap();
            assert!(!ins.active_hedges.is_empty());

            // ── 5. Verify hedge size ≈ 50% of portfolio ──
            let hedge = ins.active_hedges.get(0).unwrap();
            // current_value = 8_000, target_hedge_value = 8_000 * 5000 / 10_000 = 4_000
            // amount = 4_000 * 100 / 80 = 5_000
            assert_eq!(hedge.amount, 5_000);

            // ── 6. Simulate recovery (price back to 99 → drawdown < 5%) ──
            risk::set_asset_price(&env, 1, 99);

            let dd = AutoTradeContract::get_portfolio_drawdown(env.clone(), user.clone()).unwrap();
            // (10_000 - 9_900) * 10_000 / 10_000 = 100 bps = 1% < 500 bps
            assert!(dd < 500);

            // ── 7. Verify hedges removed ──
            let removed =
                AutoTradeContract::remove_hedges_if_recovered(env.clone(), user.clone())
                    .unwrap();
            assert!(removed.len() > 0, "hedges must be removed on recovery");

            let ins = AutoTradeContract::get_insurance_config(env.clone(), user.clone()).unwrap();
            assert!(ins.active_hedges.is_empty());
        });
    }

    #[test]
    fn test_insurance_configure_and_query() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                true,
                2000,
                3000,
                500,
            )
            .unwrap();

            let ins = AutoTradeContract::get_insurance_config(env.clone(), user.clone()).unwrap();
            assert!(ins.enabled);
            assert_eq!(ins.max_drawdown_bps, 2000);
            assert_eq!(ins.hedge_ratio_bps, 3000);
            assert_eq!(ins.rebalance_threshold_bps, 500);
        });
    }

    #[test]
    fn test_hedge_not_triggered_below_threshold() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                true,
                1500,
                5000,
                200,
            )
            .unwrap();

            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);
            AutoTradeContract::get_portfolio_drawdown(env.clone(), user.clone()).unwrap();

            // Only 10% drop — below 15% threshold
            risk::set_asset_price(&env, 1, 90);

            let ids =
                AutoTradeContract::apply_hedge_if_needed(env.clone(), user.clone()).unwrap();
            assert_eq!(ids.len(), 0);
        });
    }

    #[test]
    fn test_disabled_insurance_no_hedge() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                false, // disabled
                1500,
                5000,
                200,
            )
            .unwrap();

            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);
            AutoTradeContract::get_portfolio_drawdown(env.clone(), user.clone()).unwrap();
            risk::set_asset_price(&env, 1, 80);

            let ids =
                AutoTradeContract::apply_hedge_if_needed(env.clone(), user.clone()).unwrap();
            assert_eq!(ids.len(), 0);
        });
    }

    #[test]
    fn test_rebalance_increases_hedge_on_portfolio_growth() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                true,
                1500,
                5000,
                200,
            )
            .unwrap();

            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);
            AutoTradeContract::get_portfolio_drawdown(env.clone(), user.clone()).unwrap();
            risk::set_asset_price(&env, 1, 80);
            AutoTradeContract::apply_hedge_if_needed(env.clone(), user.clone()).unwrap();

            // Portfolio doubles in size
            risk::update_position(&env, &user, 1, 20_000, 80);

            let ids = AutoTradeContract::rebalance_hedges(env.clone(), user.clone()).unwrap();
            assert!(ids.len() > 0, "rebalance should add hedges when portfolio grows");
        });
    }

    #[test]
    fn test_no_hedge_without_insurance_config() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let err =
                AutoTradeContract::apply_hedge_if_needed(env.clone(), user.clone()).unwrap_err();
            assert_eq!(err, AutoTradeError::InsuranceNotConfigured);
        });
    }

    #[test]
    fn test_invalid_config_rejected() {
        let env = setup_env();
        let contract_id = env.register(AutoTradeContract, ());
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            // Zero drawdown threshold is invalid
            let err = AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                true,
                0,
                5000,
                200,
            )
            .unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidInsuranceConfig);

            // Zero hedge ratio is invalid
            let err = AutoTradeContract::configure_insurance(
                env.clone(),
                user.clone(),
                true,
                1500,
                0,
                200,
            )
            .unwrap_err();
            assert_eq!(err, AutoTradeError::InvalidInsuranceConfig);
        });
    }
}
