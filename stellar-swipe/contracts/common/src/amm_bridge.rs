//! Stellar AMM bridge interface — price discovery, multi-source routing, slippage protection.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

pub const BPS_DENOMINATOR: i128 = 10_000;

/// Standard router entrypoints (compatible with `trade_executor::sdex`).
pub const FN_GET_BEST_ASK: &str = "get_best_ask";
pub const FN_SWAP: &str = "swap";

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AmmBridgeError {
    InvalidAmount = 1,
    NoLiquidity = 2,
    SlippageExceeded = 3,
    SourceUnavailable = 4,
    RouteNotFound = 5,
    ExecutionFailed = 6,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum AmmSourceKind {
    /// Soroban SDEX / aggregator router (`get_best_ask` + `swap`).
    SdexRouter,
    /// Stellar native AMM (classic or Soroban pool router).
    StellarAmm,
    /// Bridge wrapped-asset constant-product pool.
    BridgePool,
    /// Path-payment strict-send router.
    PathPayment,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmmSourceConfig {
    pub kind: AmmSourceKind,
    pub source_id: u32,
    pub router: Address,
    pub priority: u32,
    pub enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmmQuote {
    pub kind: AmmSourceKind,
    pub source_id: u32,
    pub available_in: i128,
    pub spot_price: i128,
    pub fee_bps: u32,
    pub max_slippage_bps: u32,
    /// Effective output for `probe_amount` after fees (price discovery).
    pub expected_out: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmmRouteSegment {
    pub kind: AmmSourceKind,
    pub source_id: u32,
    pub amount_in: i128,
    pub min_amount_out: i128,
    pub execution_price: i128,
    pub fee_amount: i128,
    pub estimated_slippage_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmmRoutePlan {
    pub amount_in: i128,
    pub amount_out: i128,
    pub average_price: i128,
    pub total_fees: i128,
    pub estimated_slippage_bps: u32,
    pub segments: Vec<AmmRouteSegment>,
}

/// Constant-product output: `amount_out = (amount_in * (BPS-fee) * reserve_out) / (reserve_in * BPS + amount_in * (BPS-fee))`
pub fn quote_constant_product(
    amount_in: i128,
    reserve_in: i128,
    reserve_out: i128,
    fee_bps: u32,
) -> Option<i128> {
    if amount_in <= 0 || reserve_in <= 0 || reserve_out <= 0 {
        return None;
    }
    let fee_mult = BPS_DENOMINATOR - fee_bps as i128;
    let amount_after_fee = amount_in.checked_mul(fee_mult)? / BPS_DENOMINATOR;
    let numerator = amount_after_fee.checked_mul(reserve_out)?;
    let denominator = reserve_in.checked_add(amount_after_fee)?;
    if denominator == 0 {
        return None;
    }
    Some(numerator / denominator)
}

/// Minimum acceptable output given max slippage (basis points).
pub fn min_amount_out_with_slippage(expected_out: i128, max_slippage_bps: u32) -> Option<i128> {
    if expected_out <= 0 {
        return None;
    }
    if max_slippage_bps >= 10_000 {
        return Some(0);
    }
    let num = (10_000u32).checked_sub(max_slippage_bps)? as i128;
    expected_out.checked_mul(num)?.checked_div(BPS_DENOMINATOR)
}

pub fn estimate_impact_slippage_bps(amount: i128, liquidity: i128, cap_bps: u32) -> u32 {
    if liquidity <= 0 || amount <= 0 {
        return u32::MAX;
    }
    let raw = (amount * cap_bps as i128 + liquidity - 1) / liquidity;
    raw as u32
}

fn apply_bps(value: i128, bps: u32) -> i128 {
    (value * bps as i128 + (BPS_DENOMINATOR - 1)) / BPS_DENOMINATOR
}

/// Build an `AmmQuote` from reserve-based pool state.
pub fn quote_from_pool_reserves(
    kind: AmmSourceKind,
    source_id: u32,
    amount_in: i128,
    reserve_in: i128,
    reserve_out: i128,
    fee_bps: u32,
    max_slippage_bps: u32,
) -> Result<AmmQuote, AmmBridgeError> {
    let expected_out = quote_constant_product(amount_in, reserve_in, reserve_out, fee_bps)
        .ok_or(AmmBridgeError::InvalidAmount)?;
    let spot_price = if amount_in > 0 {
        expected_out * BPS_DENOMINATOR / amount_in
    } else {
        0
    };
    Ok(AmmQuote {
        kind,
        source_id,
        available_in: reserve_in,
        spot_price,
        fee_bps,
        max_slippage_bps,
        expected_out,
    })
}

/// Rank quotes by best effective price (highest output per unit in).
pub fn rank_quotes_by_price(env: &Env, quotes: &Vec<AmmQuote>) -> Vec<AmmQuote> {
    let mut ranked = Vec::new(env);
    for q in quotes.iter() {
        ranked.push_back(q);
    }
    // Simple selection sort (small N venues).
    let len = ranked.len();
    for i in 0..len {
        let mut best_idx = i;
        for j in (i + 1)..len {
            let qi = ranked.get(i).unwrap();
            let qj = ranked.get(j).unwrap();
            if qj.expected_out * qi.available_in.max(1) > qi.expected_out * qj.available_in.max(1) {
                best_idx = j;
            }
        }
        if best_idx != i {
            let tmp = ranked.get(i).unwrap();
            let swap = ranked.get(best_idx).unwrap();
            ranked.set(i, swap);
            ranked.set(best_idx, tmp);
        }
    }
    ranked
}

/// Greedy multi-source route planner (lowest total cost first).
pub fn plan_multi_source_route(
    env: &Env,
    quotes: &Vec<AmmQuote>,
    amount_in: i128,
    reference_price: i128,
    max_slippage_bps: u32,
) -> Result<AmmRoutePlan, AmmBridgeError> {
    if amount_in <= 0 {
        return Err(AmmBridgeError::InvalidAmount);
    }
    if quotes.is_empty() {
        return Err(AmmBridgeError::RouteNotFound);
    }

    let mut remaining = amount_in;
    let mut segments = Vec::new(env);

    while remaining > 0 {
        let mut best: Option<(AmmRouteSegment, i128)> = None;

        for quote in quotes.iter() {
            let already = allocated_for(&segments, quote.kind, quote.source_id);
            let avail = quote.available_in - already;
            let alloc = core::cmp::min(remaining, avail);
            if alloc <= 0 {
                continue;
            }

            let impact =
                estimate_impact_slippage_bps(alloc, quote.available_in, quote.max_slippage_bps);
            if impact > max_slippage_bps {
                continue;
            }

            let out = alloc * quote.spot_price / BPS_DENOMINATOR;
            let price = if alloc > 0 {
                out * BPS_DENOMINATOR / alloc
            } else {
                quote.spot_price
            };
            let notional = alloc * price / BPS_DENOMINATOR;
            let fee = apply_bps(notional, quote.fee_bps);
            let min_out = min_amount_out_with_slippage(out, max_slippage_bps).unwrap_or(0);

            let segment = AmmRouteSegment {
                kind: quote.kind,
                source_id: quote.source_id,
                amount_in: alloc,
                min_amount_out: min_out,
                execution_price: price,
                fee_amount: fee,
                estimated_slippage_bps: impact,
            };

            match &best {
                Some((_, best_cost)) if notional + fee >= *best_cost => {}
                _ => best = Some((segment, notional + fee)),
            }
        }

        let Some((segment, _)) = best else {
            return Err(AmmBridgeError::NoLiquidity);
        };
        remaining -= segment.amount_in;
        segments.push_back(segment);
    }

    finalize_route_plan(env, segments, amount_in, reference_price, max_slippage_bps)
}

fn finalize_route_plan(
    _env: &Env,
    segments: Vec<AmmRouteSegment>,
    amount_in: i128,
    reference_price: i128,
    max_slippage_bps: u32,
) -> Result<AmmRoutePlan, AmmBridgeError> {
    let mut total_in = 0i128;
    let mut total_out = 0i128;
    let mut total_fees = 0i128;

    for seg in segments.iter() {
        total_in += seg.amount_in;
        total_out += seg.min_amount_out;
        total_fees += seg.fee_amount;
    }

    if total_in != amount_in {
        return Err(AmmBridgeError::NoLiquidity);
    }

    let average_price = if total_in > 0 {
        total_out * BPS_DENOMINATOR / total_in
    } else {
        0
    };

    let estimated_slippage_bps = if reference_price <= 0 || average_price <= reference_price {
        0
    } else {
        ((average_price - reference_price) * BPS_DENOMINATOR / reference_price) as u32
    };

    if estimated_slippage_bps > max_slippage_bps {
        return Err(AmmBridgeError::SlippageExceeded);
    }

    Ok(AmmRoutePlan {
        amount_in: total_in,
        amount_out: total_out,
        average_price,
        total_fees,
        estimated_slippage_bps,
        segments,
    })
}

fn allocated_for(segments: &Vec<AmmRouteSegment>, kind: AmmSourceKind, source_id: u32) -> i128 {
    let mut sum = 0i128;
    for seg in segments.iter() {
        if seg.kind == kind && seg.source_id == source_id {
            sum += seg.amount_in;
        }
    }
    sum
}

/// Returns fallback source ordering after removing failed sources.
pub fn build_fallback_chain(
    env: &Env,
    sources: &Vec<AmmSourceConfig>,
    failed_kinds: &Vec<(AmmSourceKind, u32)>,
) -> Vec<AmmSourceConfig> {
    let mut chain = Vec::new(env);
    for src in sources.iter() {
        if !src.enabled {
            continue;
        }
        let mut failed = false;
        for f in failed_kinds.iter() {
            if f.0 == src.kind && f.1 == src.source_id {
                failed = true;
                break;
            }
        }
        if !failed {
            chain.push_back(src);
        }
    }
    chain
}

pub fn emit_quote_discovered(env: &Env, signal_id: u64, quote: &AmmQuote) {
    let topics = (Symbol::new(env, "amm_quote_discovered"), signal_id);
    env.events().publish(topics, quote.clone());
}

pub fn emit_route_planned(env: &Env, signal_id: u64, plan: &AmmRoutePlan) {
    let topics = (Symbol::new(env, "amm_route_planned"), signal_id);
    env.events().publish(topics, plan.clone());
}

pub fn emit_fallback_used(env: &Env, signal_id: u64, kind: AmmSourceKind, source_id: u32) {
    let topics = (Symbol::new(env, "amm_fallback_used"), signal_id);
    env.events().publish(topics, (kind, source_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    #[test]
    fn constant_product_quote_basic() {
        let out = quote_constant_product(1_000, 100_000, 100_000, 30).unwrap();
        assert!(out > 0);
        assert!(out < 1_000);
    }

    #[test]
    fn slippage_floor_calculation() {
        assert_eq!(min_amount_out_with_slippage(10_000, 100), Some(9_900));
        assert_eq!(min_amount_out_with_slippage(10_000, 10_000), Some(0));
    }

    #[test]
    fn multi_source_route_splits_across_pools() {
        let env = Env::default();
        let mut quotes = Vec::new(&env);
        quotes.push_back(AmmQuote {
            kind: AmmSourceKind::BridgePool,
            source_id: 1,
            available_in: 50_000,
            spot_price: 100,
            fee_bps: 30,
            max_slippage_bps: 500,
            expected_out: 5_000,
        });
        quotes.push_back(AmmQuote {
            kind: AmmSourceKind::StellarAmm,
            source_id: 2,
            available_in: 40_000,
            spot_price: 100,
            fee_bps: 30,
            max_slippage_bps: 500,
            expected_out: 4_000,
        });

        let plan = plan_multi_source_route(&env, &quotes, 80_000, 100, 1_000).unwrap();
        assert_eq!(plan.amount_in, 80_000);
        assert!(!plan.segments.is_empty());
    }

    #[test]
    fn rejects_excessive_slippage() {
        let env = Env::default();
        let mut quotes = Vec::new(&env);
        quotes.push_back(AmmQuote {
            kind: AmmSourceKind::SdexRouter,
            source_id: 1,
            available_in: 100,
            spot_price: 200,
            fee_bps: 0,
            max_slippage_bps: 900,
            expected_out: 50,
        });
        assert!(plan_multi_source_route(&env, &quotes, 100, 100, 50).is_err());
    }

    #[test]
    fn fallback_chain_excludes_failed() {
        let env = Env::default();
        let mut sources = Vec::new(&env);
        sources.push_back(AmmSourceConfig {
            kind: AmmSourceKind::SdexRouter,
            source_id: 1,
            router: Address::generate(&env),
            priority: 1,
            enabled: true,
        });
        sources.push_back(AmmSourceConfig {
            kind: AmmSourceKind::BridgePool,
            source_id: 2,
            router: Address::generate(&env),
            priority: 2,
            enabled: true,
        });
        let mut failed = Vec::new(&env);
        failed.push_back((AmmSourceKind::SdexRouter, 1));
        let chain = build_fallback_chain(&env, &sources, &failed);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.get(0).unwrap().kind, AmmSourceKind::BridgePool);
    }
}
