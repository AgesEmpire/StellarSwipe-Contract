//! AMM bridge integration for `auto_trade`: price discovery, routing, fallback execution.

use soroban_sdk::{contracttype, Address, Env, IntoVal, Symbol, Vec};

use stellar_swipe_common::amm_bridge::{
    build_fallback_chain, emit_fallback_used, emit_quote_discovered, emit_route_planned,
    min_amount_out_with_slippage, plan_multi_source_route, rank_quotes_by_price, AmmQuote,
    AmmRoutePlan, AmmSourceConfig, AmmSourceKind, FN_GET_BEST_ASK,
};

use crate::errors::AutoTradeError;
use crate::sdex::{execute_market_order, ExecutionResult};
use crate::smart_routing::{self, LiquidityVenue, VenueLiquidity};
use crate::storage::Signal;

#[contracttype]
enum AmmBridgeKey {
    SourceRegistry,
    SignalTokenFrom(u64),
    SignalTokenTo(u64),
    FailedSource(u64, AmmSourceKind, u32),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenPairConfig {
    pub from_token: Address,
    pub to_token: Address,
}

fn venue_to_kind(venue: LiquidityVenue) -> AmmSourceKind {
    match venue {
        LiquidityVenue::Sdex => AmmSourceKind::SdexRouter,
        LiquidityVenue::Pool => AmmSourceKind::StellarAmm,
        LiquidityVenue::PathPayment => AmmSourceKind::PathPayment,
    }
}

fn kind_to_venue(kind: AmmSourceKind) -> LiquidityVenue {
    match kind {
        AmmSourceKind::SdexRouter => LiquidityVenue::Sdex,
        AmmSourceKind::BridgePool | AmmSourceKind::StellarAmm => LiquidityVenue::Pool,
        AmmSourceKind::PathPayment => LiquidityVenue::PathPayment,
    }
}

pub fn register_amm_source(env: &Env, config: AmmSourceConfig) -> Result<(), AutoTradeError> {
    if config.source_id == 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let mut registry: Vec<AmmSourceConfig> = env
        .storage()
        .persistent()
        .get(&AmmBridgeKey::SourceRegistry)
        .unwrap_or_else(|| Vec::new(env));

    let mut replaced = false;
    for i in 0..registry.len() {
        let existing = registry.get(i).unwrap();
        if existing.kind == config.kind && existing.source_id == config.source_id {
            registry.set(i, config.clone());
            replaced = true;
            break;
        }
    }
    if !replaced {
        registry.push_back(config);
    }

    env.storage()
        .persistent()
        .set(&AmmBridgeKey::SourceRegistry, &registry);
    Ok(())
}

pub fn get_amm_sources(env: &Env) -> Vec<AmmSourceConfig> {
    env.storage()
        .persistent()
        .get(&AmmBridgeKey::SourceRegistry)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_signal_token_pair(env: &Env, signal_id: u64, from_token: Address, to_token: Address) {
    env.storage()
        .persistent()
        .set(&AmmBridgeKey::SignalTokenFrom(signal_id), &from_token);
    env.storage()
        .persistent()
        .set(&AmmBridgeKey::SignalTokenTo(signal_id), &to_token);
}

pub fn get_signal_token_pair(env: &Env, signal_id: u64) -> Option<TokenPairConfig> {
    let from_token: Address = env
        .storage()
        .persistent()
        .get(&AmmBridgeKey::SignalTokenFrom(signal_id))?;
    let to_token: Address = env
        .storage()
        .persistent()
        .get(&AmmBridgeKey::SignalTokenTo(signal_id))?;
    Some(TokenPairConfig {
        from_token,
        to_token,
    })
}

fn query_router_best_ask(
    env: &Env,
    router: &Address,
    from_token: &Address,
    to_token: &Address,
) -> Option<(i128, i128)> {
    let sym = Symbol::new(env, FN_GET_BEST_ASK);
    match env.try_invoke_contract::<(i128, i128), soroban_sdk::Error>(
        router,
        &sym,
        (from_token.clone(), to_token.clone()).into_val(env),
    ) {
        Ok(Ok(result)) => Some(result),
        _ => None,
    }
}

/// Price discovery: merge stored venue quotes with on-chain router quotes.
pub fn discover_quotes(env: &Env, signal_id: u64, probe_amount: i128) -> Vec<AmmQuote> {
    let mut quotes = Vec::new(env);

    for venue in smart_routing::get_venue_liquidity(env, signal_id).iter() {
        if let Ok(q) = quote_from_venue(&venue, probe_amount) {
            emit_quote_discovered(env, signal_id, &q);
            quotes.push_back(q);
        }
    }

    if let Some(pair) = get_signal_token_pair(env, signal_id) {
        for source in get_amm_sources(env).iter() {
            if !source.enabled {
                continue;
            }
            if let Some((price, qty)) =
                query_router_best_ask(env, &source.router, &pair.from_token, &pair.to_token)
            {
                if qty <= 0 || price <= 0 {
                    continue;
                }
                let expected_out =
                    probe_amount * price / stellar_swipe_common::amm_bridge::BPS_DENOMINATOR;
                let q = AmmQuote {
                    kind: source.kind,
                    source_id: source.source_id,
                    available_in: qty,
                    spot_price: price,
                    fee_bps: 0,
                    max_slippage_bps: 500,
                    expected_out: core::cmp::min(expected_out, qty),
                };
                emit_quote_discovered(env, signal_id, &q);
                quotes.push_back(q);
            }
        }
    }

    rank_quotes_by_price(env, &quotes)
}

fn quote_from_venue(
    venue: &VenueLiquidity,
    probe_amount: i128,
) -> Result<AmmQuote, AutoTradeError> {
    let kind = venue_to_kind(venue.venue);
    let alloc = core::cmp::min(probe_amount, venue.available_amount);
    if alloc <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    let expected_out = alloc * venue.price / stellar_swipe_common::amm_bridge::BPS_DENOMINATOR;
    Ok(AmmQuote {
        kind,
        source_id: venue.venue_id,
        available_in: venue.available_amount,
        spot_price: venue.price,
        fee_bps: venue.fee_bps,
        max_slippage_bps: venue.slippage_bps,
        expected_out,
    })
}

pub fn plan_amm_route(
    env: &Env,
    signal: &Signal,
    amount: i128,
    max_slippage_bps: u32,
) -> Result<AmmRoutePlan, AutoTradeError> {
    let quotes = discover_quotes(env, signal.signal_id, amount);
    if quotes.is_empty() {
        return Err(AutoTradeError::RoutingPlanNotFound);
    }
    plan_multi_source_route(env, &quotes, amount, signal.price, max_slippage_bps)
        .map_err(map_bridge_error)
        .map(|plan| {
            emit_route_planned(env, signal.signal_id, &plan);
            plan
        })
}

fn map_bridge_error(err: stellar_swipe_common::amm_bridge::AmmBridgeError) -> AutoTradeError {
    use stellar_swipe_common::amm_bridge::AmmBridgeError;
    match err {
        AmmBridgeError::InvalidAmount => AutoTradeError::InvalidAmount,
        AmmBridgeError::NoLiquidity | AmmBridgeError::SourceUnavailable => {
            AutoTradeError::InsufficientLiquidity
        }
        AmmBridgeError::SlippageExceeded => AutoTradeError::SlippageExceeded,
        AmmBridgeError::RouteNotFound => AutoTradeError::RoutingPlanNotFound,
        AmmBridgeError::ExecutionFailed => AutoTradeError::AtomicExecutionFailed,
    }
}

fn mark_source_failed(env: &Env, signal_id: u64, kind: AmmSourceKind, source_id: u32) {
    env.storage().temporary().set(
        &AmmBridgeKey::FailedSource(signal_id, kind, source_id),
        &true,
    );
    smart_routing::set_execution_failure(env, signal_id, kind_to_venue(kind), source_id);
}

fn is_source_failed(env: &Env, signal_id: u64, kind: AmmSourceKind, source_id: u32) -> bool {
    env.storage()
        .temporary()
        .get(&AmmBridgeKey::FailedSource(signal_id, kind, source_id))
        .unwrap_or(false)
}

fn execute_amm_plan(
    env: &Env,
    signal_id: u64,
    plan: &AmmRoutePlan,
) -> Result<ExecutionResult, AutoTradeError> {
    for segment in plan.segments.iter() {
        if is_source_failed(env, signal_id, segment.kind, segment.source_id) {
            return Err(AutoTradeError::AtomicExecutionFailed);
        }
    }

    for segment in plan.segments.iter() {
        execute_segment(env, signal_id, &segment)?;
    }

    Ok(ExecutionResult {
        executed_amount: plan.amount_in,
        executed_price: plan.average_price,
    })
}

fn execute_segment(
    env: &Env,
    signal_id: u64,
    segment: &stellar_swipe_common::amm_bridge::AmmRouteSegment,
) -> Result<(), AutoTradeError> {
    let venue = kind_to_venue(segment.kind);
    if smart_routing::debit_venue_liquidity(
        env,
        signal_id,
        venue,
        segment.source_id,
        segment.amount_in,
    )
    .is_ok()
    {
        return Ok(());
    }

    if let Some(source) = find_source_config(env, segment.kind, segment.source_id) {
        if let Some(pair) = get_signal_token_pair(env, signal_id) {
            let min_out = segment.min_amount_out;
            let out = invoke_router_swap(
                env,
                &source.router,
                &pair.from_token,
                &pair.to_token,
                segment.amount_in,
                min_out,
            )?;
            if out < min_out {
                return Err(AutoTradeError::SlippageExceeded);
            }
            return Ok(());
        }
    }

    Err(AutoTradeError::AtomicExecutionFailed)
}

fn find_source_config(env: &Env, kind: AmmSourceKind, source_id: u32) -> Option<AmmSourceConfig> {
    for src in get_amm_sources(env).iter() {
        if src.kind == kind && src.source_id == source_id && src.enabled {
            return Some(src);
        }
    }
    None
}

fn invoke_router_swap(
    env: &Env,
    router: &Address,
    from_token: &Address,
    to_token: &Address,
    amount_in: i128,
    min_out: i128,
) -> Result<i128, AutoTradeError> {
    let sym = Symbol::new(env, stellar_swipe_common::amm_bridge::FN_SWAP);
    let pull_from = env.current_contract_address();
    let recipient = pull_from.clone();

    env.try_invoke_contract::<i128, soroban_sdk::Error>(
        router,
        &sym,
        (
            pull_from,
            from_token.clone(),
            to_token.clone(),
            amount_in,
            min_out,
            recipient,
        )
            .into_val(env),
    )
    .map_err(|_| AutoTradeError::AtomicExecutionFailed)?
    .map_err(|_| AutoTradeError::AtomicExecutionFailed)
}

/// Primary entry: smart route → AMM bridge plan → per-source fallback → SDEX stub.
pub fn execute_swap_with_fallback(
    env: &Env,
    user: &Address,
    signal: &Signal,
    amount: i128,
    max_slippage_bps: u32,
) -> Result<ExecutionResult, AutoTradeError> {
    if amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    match smart_routing::execute_best_route(env, signal, amount, max_slippage_bps) {
        Ok(result) => return Ok(result),
        Err(AutoTradeError::RoutingPlanNotFound) | Err(AutoTradeError::InsufficientLiquidity) => {}
        Err(AutoTradeError::AtomicExecutionFailed) | Err(AutoTradeError::SlippageExceeded) => {
            // Continue to AMM bridge fallback chain.
        }
        Err(err) => return Err(err),
    }

    match plan_amm_route(env, signal, amount, max_slippage_bps) {
        Ok(plan) => match execute_amm_plan(env, signal.signal_id, &plan) {
            Ok(result) => return Ok(result),
            Err(AutoTradeError::AtomicExecutionFailed) | Err(AutoTradeError::SlippageExceeded) => {
                for segment in plan.segments.iter() {
                    mark_source_failed(env, signal.signal_id, segment.kind, segment.source_id);
                    emit_fallback_used(env, signal.signal_id, segment.kind, segment.source_id);
                }
            }
            Err(err) => return Err(err),
        },
        Err(AutoTradeError::RoutingPlanNotFound) | Err(AutoTradeError::InsufficientLiquidity) => {}
        Err(err) => return Err(err),
    }

    let mut failed = Vec::new(env);
    for src in get_amm_sources(env).iter() {
        if is_source_failed(env, signal.signal_id, src.kind, src.source_id) {
            failed.push_back((src.kind, src.source_id));
        }
    }
    let chain = build_fallback_chain(env, &get_amm_sources(env), &failed);

    for src in chain.iter() {
        if let Some(pair) = get_signal_token_pair(env, signal.signal_id) {
            if let Some((price, qty)) =
                query_router_best_ask(env, &src.router, &pair.from_token, &pair.to_token)
            {
                if qty >= amount && price > 0 {
                    let expected_out =
                        amount * price / stellar_swipe_common::amm_bridge::BPS_DENOMINATOR;
                    let min_out =
                        min_amount_out_with_slippage(expected_out, max_slippage_bps).unwrap_or(0);
                    if invoke_router_swap(
                        env,
                        &src.router,
                        &pair.from_token,
                        &pair.to_token,
                        amount,
                        min_out,
                    )
                    .is_ok()
                    {
                        emit_fallback_used(env, signal.signal_id, src.kind, src.source_id);
                        return Ok(ExecutionResult {
                            executed_amount: amount,
                            executed_price: price,
                        });
                    }
                }
            }
        }
        mark_source_failed(env, signal.signal_id, src.kind, src.source_id);
        emit_fallback_used(env, signal.signal_id, src.kind, src.source_id);
    }

    execute_market_order(env, user, signal, amount)
}

#[cfg(any(test, feature = "testutils"))]
pub mod mock_router {
    use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};

    #[contract]
    pub struct MockAmmRouter;

    #[contractimpl]
    impl MockAmmRouter {
        pub fn get_best_ask(env: Env, _from: Address, _to: Address) -> (i128, i128) {
            env.storage()
                .instance()
                .get(&symbol_short!("ask"))
                .unwrap_or((100, 1_000_000))
        }

        pub fn set_best_ask(env: Env, price: i128, qty: i128) {
            env.storage()
                .instance()
                .set(&symbol_short!("ask"), &(price, qty));
        }

        pub fn set_amount_out(env: Env, out: i128) {
            env.storage().instance().set(&symbol_short!("amtout"), &out);
        }

        pub fn set_fail_swap(env: Env, fail: bool) {
            env.storage().instance().set(&symbol_short!("fail"), &fail);
        }

        pub fn swap(
            env: Env,
            pull_from: Address,
            from_token: Address,
            _to_token: Address,
            amount_in: i128,
            min_out: i128,
            _recipient: Address,
        ) -> i128 {
            let fail: bool = env
                .storage()
                .instance()
                .get(&symbol_short!("fail"))
                .unwrap_or(false);
            if fail {
                panic!("swap failed");
            }

            let router = env.current_contract_address();
            let from_c = token::Client::new(&env, &from_token);
            from_c.transfer_from(&router, &pull_from, &router, &amount_in);

            let out: i128 = env
                .storage()
                .instance()
                .get(&symbol_short!("amtout"))
                .unwrap_or(amount_in);
            if out < min_out {
                panic!("slippage");
            }
            out
        }
    }
}
