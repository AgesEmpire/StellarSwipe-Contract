#![cfg(test)]
//! Performance regression test for auto_trade execute_trade hot path.

use auto_trade::{
    authorize_user_with_limits, set_signal, AutoTradeContract, OrderType, Signal, TradeStatus,
};
use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env};
use stellar_swipe_common::budget_regression::measure_and_emit;
use stellar_swipe_common::perf::{regression_budget_limit, BASELINE_AUTO_TRADE_INSTRUCTIONS};

const TRADE_AMOUNT: i128 = 1_000;

#[test]
fn test_execute_trade_latency_regression() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::initialize(env.clone(), admin);
        auto_trade::rate_limit::set_limits(
            &env,
            &auto_trade::rate_limit::BridgeRateLimits {
                per_user_hourly_transfers: 10_000,
                per_user_hourly_volume: i128::MAX,
                per_user_daily_transfers: 100_000,
                per_user_daily_volume: i128::MAX,
                global_hourly_capacity: 100_000,
                global_daily_volume: i128::MAX,
                min_transfer_amount: 1,
                cooldown_between_transfers: 0,
            },
        );
        set_signal(
            &env,
            1,
            &Signal {
                signal_id: 1,
                price: 100,
                expiry: env.ledger().timestamp() + 86_400,
                base_asset: 1,
            },
        );
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), 1u64), &1_000_000_000i128);
        authorize_user_with_limits(&env, &user, 1_000_000_000i128, 30);
        env.storage().temporary().set(
            &(user.clone(), symbol_short!("balance")),
            &1_000_000_000i128,
        );
    });

    env.as_contract(&contract_id, || {
        AutoTradeContract::execute_trade(env.clone(), user, 1, OrderType::Market, TRADE_AMOUNT)
            .expect("trade should succeed");
    });

    let instructions = env.cost_estimate().budget().cpu_instruction_cost();
    assert!(
        instructions <= regression_budget_limit(),
        "execute_trade used {instructions} instructions"
    );
    assert!(
        instructions <= BASELINE_AUTO_TRADE_INSTRUCTIONS * 3,
        "execute_trade {instructions} exceeds 3x baseline"
    );
    measure_and_emit("auto_trade.execute_auto_trade", BASELINE_AUTO_TRADE_INSTRUCTIONS, instructions);
}

#[test]
fn test_rate_limit_recorded_after_trade() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let contract_id = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        AutoTradeContract::initialize(env.clone(), admin);
        auto_trade::rate_limit::set_limits(
            &env,
            &auto_trade::rate_limit::BridgeRateLimits {
                per_user_hourly_transfers: 10_000,
                per_user_hourly_volume: i128::MAX,
                per_user_daily_transfers: 100_000,
                per_user_daily_volume: i128::MAX,
                global_hourly_capacity: 100_000,
                global_daily_volume: i128::MAX,
                min_transfer_amount: 1,
                cooldown_between_transfers: 0,
            },
        );
        set_signal(
            &env,
            1,
            &Signal {
                signal_id: 1,
                price: 100,
                expiry: env.ledger().timestamp() + 86_400,
                base_asset: 1,
            },
        );
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), 1u64), &1_000_000_000i128);
        authorize_user_with_limits(&env, &user, 1_000_000_000i128, 30);
        env.storage().temporary().set(
            &(user.clone(), symbol_short!("balance")),
            &1_000_000_000i128,
        );
    });

    env.as_contract(&contract_id, || {
        AutoTradeContract::execute_trade(
            env.clone(),
            user.clone(),
            1,
            OrderType::Market,
            TRADE_AMOUNT,
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        auto_trade::rate_limit::check_rate_limits(&env, &user, TRADE_AMOUNT)
            .expect("second trade should pass limits");
    });
}
