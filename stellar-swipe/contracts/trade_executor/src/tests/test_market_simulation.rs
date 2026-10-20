#![cfg(test)]
//! Market simulation and stress tests for TradeExecutor (issue #638).
//!
//! Unlike `test.rs`'s `MockSdexRouter` (a fixed `amount_out`), `MockPriceRouter`
//! here models a price ratio in bps relative to 1:1, so a sequence of
//! `set_price_bps` calls can simulate a market price path (trending, mild
//! moves, flash crashes) across repeated swaps. Covers:
//! - A configurable market simulation environment.
//! - A property-style sweep over (amount, slippage_bps, market move) asserting
//!   the slippage/price-impact invariants hold across a wide input space —
//!   standing in for property-based testing without an external proptest
//!   dependency.
//! - A high-frequency stress run of sequential trades against the daily
//!   volume risk gate.

use crate::{
    errors::ContractError, risk_gates::DEFAULT_ESTIMATED_COPY_TRADE_FEE, sdex, OrderType,
    TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::Address as _,
    token::{self, StellarAssetClient},
    Address, Env, MuxedAddress,
};

// ── Mock price-feed router ───────────────────────────────────────────────────

#[contract]
pub struct MockPriceRouter;

#[contracttype]
#[derive(Clone)]
enum RouterKey {
    PriceBps,
}

#[contractimpl]
impl MockPriceRouter {
    /// Reports unlimited liquidity at price 0 so `check_liquidity`'s guard
    /// never blocks these tests — only the slippage path is under test.
    pub fn get_best_ask(_env: Env, _from_token: Address, _to_token: Address) -> (i128, i128) {
        (0, i128::MAX / 2)
    }

    pub fn set_price_bps(env: Env, bps: i128) {
        env.storage().instance().set(&RouterKey::PriceBps, &bps);
    }

    pub fn swap(
        env: Env,
        pull_from: Address,
        from_token: Address,
        to_token: Address,
        amount_in: i128,
        _min_out: i128,
        recipient: Address,
    ) -> i128 {
        let router = env.current_contract_address();
        let from_c = token::Client::new(&env, &from_token);
        from_c.transfer_from(&router, &pull_from, &router, &amount_in);

        let price_bps: i128 = env
            .storage()
            .instance()
            .get(&RouterKey::PriceBps)
            .unwrap_or(10_000);
        let amount_out = amount_in * price_bps / 10_000;

        let to_c = token::Client::new(&env, &to_token);
        let to_mux: MuxedAddress = recipient.into();
        to_c.transfer(&router, &to_mux, &amount_out);

        amount_out
    }
}

fn sac(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

/// Sets up executor + price router with deep liquidity on both sides.
fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_a = sac(&env);
    let token_b = sac(&env);

    let router_id = env.register(MockPriceRouter, ());
    let exec_id = env.register(TradeExecutorContract, ());
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_sdex_router(&router_id);

    StellarAssetClient::new(&env, &token_a).mint(&exec_id, &1_000_000_000_000);
    StellarAssetClient::new(&env, &token_b).mint(&router_id, &1_000_000_000_000);

    (env, exec_id, router_id, token_a, token_b)
}

fn set_price(env: &Env, router_id: &Address, bps: i128) {
    MockPriceRouterClient::new(env, router_id).set_price_bps(&bps);
}

/// Minimal deterministic xorshift PRNG so simulation paths are reproducible
/// without pulling in a `rand`/`proptest` dependency.
fn next_rand(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

// ── Market simulation: trending / crashing price paths ───────────────────────

/// A steadily improving price never trips slippage protection, across many
/// sequential swaps along the path.
#[test]
fn market_sim_trending_up_path_never_trips_slippage() {
    let (env, exec_id, router_id, token_a, token_b) = setup();
    let amount = 1_000_000i128;
    let max_slippage_bps = 100; // 1%

    let mut price_bps = 10_000i128;
    for step in 0..20 {
        set_price(&env, &router_id, price_bps);
        let out = env.as_contract(&exec_id, || {
            TradeExecutorContract::swap_with_slippage(
                env.clone(),
                token_a.clone(),
                token_b.clone(),
                amount,
                max_slippage_bps,
            )
        });
        assert!(
            out.is_ok(),
            "uptrend step {step} should never exceed slippage tolerance"
        );
        price_bps += 25; // price improves 0.25% each step
    }
}

/// A slow downtrend that stays within the configured slippage tolerance
/// succeeds at every step along the path.
#[test]
fn market_sim_mild_downtrend_within_tolerance_succeeds() {
    let (env, exec_id, router_id, token_a, token_b) = setup();
    let amount = 1_000_000i128;
    let max_slippage_bps = 500; // 5% tolerance

    let mut price_bps = 10_000i128;
    for step in 0..10 {
        price_bps -= 20; // -0.2% per step, well within the 5% tolerance
        set_price(&env, &router_id, price_bps);
        let out = env.as_contract(&exec_id, || {
            TradeExecutorContract::swap_with_slippage(
                env.clone(),
                token_a.clone(),
                token_b.clone(),
                amount,
                max_slippage_bps,
            )
        });
        assert!(
            out.is_ok(),
            "downtrend step {step} is within tolerance and must succeed"
        );
    }
}

/// A sudden flash-crash that breaches the slippage tolerance is rejected —
/// the trade must not execute at a price worse than the configured bound,
/// even though an earlier swap at the same tolerance succeeded.
#[test]
fn market_sim_flash_crash_exceeds_slippage_and_reverts() {
    let (env, exec_id, router_id, token_a, token_b) = setup();
    let amount = 1_000_000i128;
    let max_slippage_bps = 100; // 1% tolerance

    set_price(&env, &router_id, 10_000);
    let warmup = env.as_contract(&exec_id, || {
        TradeExecutorContract::swap_with_slippage(
            env.clone(),
            token_a.clone(),
            token_b.clone(),
            amount,
            max_slippage_bps,
        )
    });
    assert!(warmup.is_ok());

    // Flash crash: price collapses 40% in a single step.
    set_price(&env, &router_id, 6_000);
    let result = env.as_contract(&exec_id, || {
        TradeExecutorContract::swap_with_slippage(
            env.clone(),
            token_a.clone(),
            token_b.clone(),
            amount,
            max_slippage_bps,
        )
    });
    assert_eq!(result, Err(ContractError::SlippageExceeded));
}

// ── Property-style sweep: slippage / price-impact invariants ────────────────

/// Sweeps a deterministic pseudo-random grid of (amount, slippage_bps,
/// market move) combinations and asserts invariants that must hold for
/// *any* input:
/// - `min_received_from_slippage` never exceeds the requested amount and is
///   never negative.
/// - `swap_with_slippage` succeeds iff the simulated market output is at
///   least the computed minimum, and returns exactly that market output.
#[test]
fn slippage_property_sweep_across_amounts_and_market_moves() {
    let (env, exec_id, router_id, token_a, token_b) = setup();
    let mut seed: u64 = 0x1234_5678_9abc_def0;

    for _ in 0..200 {
        let amount = 1_000 + (next_rand(&mut seed) % 10_000_000) as i128;
        let max_slippage_bps = (next_rand(&mut seed) % 2_000) as u32; // 0–20%
                                                                      // Market move centered on 0, roughly -30%..+30%.
        let move_bps = (next_rand(&mut seed) % 6_000) as i128 - 3_000;
        let price_bps = 10_000 + move_bps;

        let min_received = sdex::min_received_from_slippage(amount, max_slippage_bps).unwrap();
        assert!(
            min_received <= amount,
            "min_received must never exceed the requested amount"
        );
        assert!(min_received >= 0, "min_received must never be negative");

        set_price(&env, &router_id, price_bps);
        let market_output = amount * price_bps / 10_000;

        let result = env.as_contract(&exec_id, || {
            TradeExecutorContract::swap_with_slippage(
                env.clone(),
                token_a.clone(),
                token_b.clone(),
                amount,
                max_slippage_bps,
            )
        });

        if market_output >= min_received {
            assert_eq!(
                result,
                Ok(market_output),
                "amount={amount} bps={max_slippage_bps} move={move_bps}: should succeed at the market output"
            );
        } else {
            assert_eq!(
                result,
                Err(ContractError::SlippageExceeded),
                "amount={amount} bps={max_slippage_bps} move={move_bps}: should be rejected"
            );
        }
    }
}

// ── Stress test: high-frequency sequential trades ───────────────────────────

#[contract]
pub struct UnlimitedPortfolio;

#[contractimpl]
impl UnlimitedPortfolio {
    pub fn validate_and_record(_env: Env, _user: Address, _max_positions: u32) -> u32 {
        0
    }
}

/// Fires a high-frequency burst of sequential market-order copy trades for a
/// single user and asserts the daily-volume risk gate holds exactly at its
/// boundary under rapid repeated invocation, not just a single isolated call.
#[test]
fn stress_high_frequency_trades_respect_daily_volume_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac(&env);
    let portfolio_id = env.register(UnlimitedPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    let per_trade = 1_000_000i128;
    let trade_count = 50i128;
    StellarAssetClient::new(&env, &token).mint(
        &user,
        &(per_trade * trade_count + trade_count * DEFAULT_ESTIMATED_COPY_TRADE_FEE),
    );

    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);
    exec.set_daily_volume_limit(&(per_trade * trade_count));

    for i in 0..trade_count {
        let nonce = (i + 1) as u64;
        let result = exec.try_execute_copy_trade(
            &user,
            &token,
            &per_trade,
            &None::<u32>,
            &OrderType::Market,
            &None,
            &nonce,
            &soroban_sdk::Bytes::from_array(&env, &[(i + 1) as u8; 32]),
            &(env.ledger().timestamp() + 86_400),
        );
        assert!(
            result.is_ok(),
            "trade {i} within the daily volume budget should succeed"
        );
    }

    // The next trade would push cumulative volume past the limit.
    let next_nonce = (trade_count + 1) as u64;
    let result = exec.try_execute_copy_trade(
        &user,
        &token,
        &per_trade,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &next_nonce,
        &soroban_sdk::Bytes::from_array(&env, &[(trade_count + 1) as u8; 32]),
        &(env.ledger().timestamp() + 86_400),
    );
    assert_eq!(result, Err(Ok(ContractError::DailyVolumeLimitExceeded)));
}
