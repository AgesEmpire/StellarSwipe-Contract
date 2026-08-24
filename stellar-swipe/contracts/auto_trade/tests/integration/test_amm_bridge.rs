//! AMM bridge integration tests with mock routers.

extern crate std;

use auto_trade::amm_bridge;
use auto_trade::amm_bridge::mock_router::{MockAmmRouter, MockAmmRouterClient};
use auto_trade::smart_routing;
use auto_trade::smart_routing::{LiquidityVenue, VenueLiquidity};
use auto_trade::{AutoTradeContract, Signal};
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{symbol_short, Address, Env};
use stellar_swipe_common::amm_bridge::{AmmSourceConfig, AmmSourceKind};

fn signal(id: u64) -> Signal {
    Signal {
        signal_id: id,
        price: 100,
        expiry: 9_999,
        base_asset: 1,
    }
}

fn venue(venue: LiquidityVenue, venue_id: u32, available: i128, price: i128) -> VenueLiquidity {
    VenueLiquidity {
        venue,
        venue_id,
        available_amount: available,
        price,
        fee_bps: 30,
        slippage_bps: 500,
    }
}

#[test]
fn discover_quotes_from_multiple_venues() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let signal_id = 42u64;

    env.as_contract(&contract, || {
        smart_routing::upsert_venue_liquidity(
            &env,
            signal_id,
            venue(LiquidityVenue::Sdex, 1, 50_000, 100),
        )
        .unwrap();
        smart_routing::upsert_venue_liquidity(
            &env,
            signal_id,
            venue(LiquidityVenue::Pool, 2, 80_000, 98),
        )
        .unwrap();
        assert_eq!(amm_bridge::discover_quotes(&env, signal_id, 1_000).len(), 2);
    });
}

#[test]
fn router_quote_merged_into_discovery() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let router_id = env.register(MockAmmRouter, ());
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let signal_id = 7u64;

    MockAmmRouterClient::new(&env, &router_id).set_best_ask(&105, &500_000);

    env.as_contract(&contract, || {
        amm_bridge::set_signal_token_pair(&env, signal_id, from, to);
        amm_bridge::register_amm_source(
            &env,
            AmmSourceConfig {
                kind: AmmSourceKind::SdexRouter,
                source_id: 10,
                router: router_id,
                priority: 1,
                enabled: true,
            },
        )
        .unwrap();
        assert_eq!(
            amm_bridge::discover_quotes(&env, signal_id, 10_000).len(),
            1
        );
    });
}

#[test]
fn plan_route_across_two_pools() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let signal_id = 99u64;

    env.as_contract(&contract, || {
        smart_routing::upsert_venue_liquidity(
            &env,
            signal_id,
            venue(LiquidityVenue::Pool, 1, 40_000, 100),
        )
        .unwrap();
        smart_routing::upsert_venue_liquidity(
            &env,
            signal_id,
            venue(LiquidityVenue::Pool, 2, 60_000, 100),
        )
        .unwrap();
        let plan = amm_bridge::plan_amm_route(&env, &signal(signal_id), 80_000, 1_000).unwrap();
        assert_eq!(plan.amount_in, 80_000);
        assert!(plan.segments.len() >= 1);
    });
}

#[test]
fn slippage_protection_rejects_bad_plan() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());

    env.as_contract(&contract, || {
        smart_routing::upsert_venue_liquidity(&env, 1, venue(LiquidityVenue::Sdex, 1, 100, 500))
            .unwrap();
        assert!(amm_bridge::plan_amm_route(&env, &signal(1), 100, 50).is_err());
    });
}

#[test]
fn execute_uses_smart_route_first() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 5u64;

    env.as_contract(&contract, || {
        smart_routing::upsert_venue_liquidity(
            &env,
            signal_id,
            venue(LiquidityVenue::Sdex, 1, 100_000, 100),
        )
        .unwrap();
        let key = (user.clone(), symbol_short!("liquidity"));
        env.storage().temporary().set(&key, &100_000i128);

        let result =
            amm_bridge::execute_swap_with_fallback(&env, &user, &signal(signal_id), 50_000, 1_000)
                .unwrap();
        assert_eq!(result.executed_amount, 50_000);
    });
}

#[test]
fn fallback_to_sdex_when_no_venues() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let user = Address::generate(&env);
    let signal_id = 3u64;

    env.as_contract(&contract, || {
        let key = (symbol_short!("liquidity"), signal_id);
        env.storage().temporary().set(&key, &25_000i128);

        let result =
            amm_bridge::execute_swap_with_fallback(&env, &user, &signal(signal_id), 30_000, 500)
                .unwrap();
        assert_eq!(result.executed_amount, 25_000);
    });
}

#[test]
fn router_fallback_when_primary_venue_fails() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let router_id = env.register(MockAmmRouter, ());
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let user = Address::generate(&env);
    let signal_id = 11u64;

    MockAmmRouterClient::new(&env, &router_id).set_best_ask(&100, &100_000);
    MockAmmRouterClient::new(&env, &router_id).set_amount_out(&49_000);

    env.as_contract(&contract, || {
        smart_routing::upsert_venue_liquidity(
            &env,
            signal_id,
            venue(LiquidityVenue::Sdex, 1, 10_000, 100),
        )
        .unwrap();
        smart_routing::set_execution_failure(&env, signal_id, LiquidityVenue::Sdex, 1);

        amm_bridge::set_signal_token_pair(&env, signal_id, from, to);
        amm_bridge::register_amm_source(
            &env,
            AmmSourceConfig {
                kind: AmmSourceKind::SdexRouter,
                source_id: 10,
                router: router_id,
                priority: 1,
                enabled: true,
            },
        )
        .unwrap();

        let key = (symbol_short!("liquidity"), signal_id);
        env.storage().temporary().set(&key, &100_000i128);

        let result =
            amm_bridge::execute_swap_with_fallback(&env, &user, &signal(signal_id), 50_000, 1_000);
        assert!(result.is_ok());
    });
}

#[test]
fn unavailable_router_skips_to_next_source() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let contract = env.register(AutoTradeContract, ());
    let bad_router = env.register(MockAmmRouter, ());
    let good_router = env.register(MockAmmRouter, ());
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let user = Address::generate(&env);
    let signal_id = 12u64;

    MockAmmRouterClient::new(&env, &bad_router).set_fail_swap(&true);
    MockAmmRouterClient::new(&env, &good_router).set_best_ask(&100, &100_000);
    MockAmmRouterClient::new(&env, &good_router).set_amount_out(&48_000);

    env.as_contract(&contract, || {
        amm_bridge::set_signal_token_pair(&env, signal_id, from, to);
        amm_bridge::register_amm_source(
            &env,
            AmmSourceConfig {
                kind: AmmSourceKind::StellarAmm,
                source_id: 1,
                router: bad_router,
                priority: 1,
                enabled: true,
            },
        )
        .unwrap();
        amm_bridge::register_amm_source(
            &env,
            AmmSourceConfig {
                kind: AmmSourceKind::StellarAmm,
                source_id: 2,
                router: good_router,
                priority: 2,
                enabled: true,
            },
        )
        .unwrap();

        let key = (symbol_short!("liquidity"), signal_id);
        env.storage().temporary().set(&key, &100_000i128);

        let result =
            amm_bridge::execute_swap_with_fallback(&env, &user, &signal(signal_id), 50_000, 1_000);
        assert!(result.is_ok());
    });
}
