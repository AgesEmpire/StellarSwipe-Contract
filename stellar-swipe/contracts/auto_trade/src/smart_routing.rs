#![allow(dead_code)]

use soroban_sdk::{contracttype, Env, Symbol, Vec};

use crate::errors::AutoTradeError;
use crate::sdex::ExecutionResult;
use crate::storage::Signal;

const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidityVenue {
    Sdex,
    Pool,
    PathPayment,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueLiquidity {
    pub venue: LiquidityVenue,
    pub venue_id: u32,
    pub available_amount: i128,
    pub price: i128,
    pub fee_bps: u32,
    pub slippage_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteSegment {
    pub venue: LiquidityVenue,
    pub venue_id: u32,
    pub amount: i128,
    pub execution_price: i128,
    pub fee_amount: i128,
    pub estimated_slippage_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingPlan {
    pub requested_amount: i128,
    pub allocated_amount: i128,
    pub average_price: i128,
    pub total_fees: i128,
    pub total_cost: i128,
    pub estimated_slippage_bps: u32,
    pub segments: Vec<RouteSegment>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
struct VenueKey {
    venue: LiquidityVenue,
    venue_id: u32,
}

#[contracttype]
enum SmartRoutingKey {
    VenueIndex(u64),
    VenueQuote(u64, VenueKey),
    FailVenue(u64),
}

pub fn upsert_venue_liquidity(
    env: &Env,
    signal_id: u64,
    quote: VenueLiquidity,
) -> Result<(), AutoTradeError> {
    if quote.available_amount <= 0 || quote.price <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let key = venue_key(&quote);
    let mut index: Vec<VenueKey> = env
        .storage()
        .persistent()
        .get(&SmartRoutingKey::VenueIndex(signal_id))
        .unwrap_or_else(|| Vec::new(env));

    if !index.contains(key.clone()) {
        index.push_back(key.clone());
        env.storage()
            .persistent()
            .set(&SmartRoutingKey::VenueIndex(signal_id), &index);
    }

    env.storage()
        .persistent()
        .set(&SmartRoutingKey::VenueQuote(signal_id, key), &quote);

    Ok(())
}

pub fn get_venue_liquidity(env: &Env, signal_id: u64) -> Vec<VenueLiquidity> {
    let index: Vec<VenueKey> = env
        .storage()
        .persistent()
        .get(&SmartRoutingKey::VenueIndex(signal_id))
        .unwrap_or_else(|| Vec::new(env));
    let mut venues = Vec::new(env);

    for key in index.iter() {
        if let Some(quote) = env
            .storage()
            .persistent()
            .get::<_, VenueLiquidity>(&SmartRoutingKey::VenueQuote(signal_id, key))
        {
            venues.push_back(quote);
        }
    }

    venues
}

pub fn plan_best_execution(
    env: &Env,
    signal: &Signal,
    requested_amount: i128,
    max_slippage_bps: u32,
) -> Result<RoutingPlan, AutoTradeError> {
    if requested_amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let venues = get_venue_liquidity(env, signal.signal_id);
    if venues.is_empty() {
        return Err(AutoTradeError::RoutingPlanNotFound);
    }

    let mut remaining = requested_amount;
    let mut selected = Vec::new(env);

    while remaining > 0 {
        let mut best: Option<(RouteSegment, i128, i128)> = None;

        for venue in venues.iter() {
            let already_allocated = allocated_for(&selected, venue.venue, venue.venue_id);
            let remaining_liquidity = venue.available_amount - already_allocated;
            let allocation = core::cmp::min(remaining, remaining_liquidity);
            if allocation <= 0 {
                continue;
            }

            let slippage =
                compute_slippage_bps(allocation, venue.available_amount, venue.slippage_bps);
            if slippage > max_slippage_bps {
                continue;
            }

            let price_with_slippage =
                venue.price * (BPS_DENOMINATOR + slippage as i128) / BPS_DENOMINATOR;
            let notional = allocation * price_with_slippage;
            let fee_amount = apply_bps(notional, venue.fee_bps);
            let total_cost = notional + fee_amount;
            let segment = RouteSegment {
                venue: venue.venue,
                venue_id: venue.venue_id,
                amount: allocation,
                execution_price: price_with_slippage,
                fee_amount,
                estimated_slippage_bps: slippage,
            };

            match &best {
                Some((_, best_cost, best_amount))
                    if total_cost * *best_amount >= *best_cost * allocation => {}
                _ => best = Some((segment, total_cost, allocation)),
            }
        }

        let Some((best_segment, _, _)) = best else {
            return Err(AutoTradeError::InsufficientLiquidity);
        };

        remaining -= best_segment.amount;
        selected.push_back(best_segment);
    }

    finalize_plan(
        env,
        signal.price,
        requested_amount,
        selected,
        max_slippage_bps,
    )
}

pub fn execute_best_route(
    env: &Env,
    signal: &Signal,
    requested_amount: i128,
    max_slippage_bps: u32,
) -> Result<ExecutionResult, AutoTradeError> {
    let plan = plan_best_execution(env, signal, requested_amount, max_slippage_bps)?;
    execute_plan_atomically(env, signal.signal_id, &plan)
}

pub fn execute_plan_atomically(
    env: &Env,
    signal_id: u64,
    plan: &RoutingPlan,
) -> Result<ExecutionResult, AutoTradeError> {
    let fail_key: Option<VenueKey> = env
        .storage()
        .temporary()
        .get(&SmartRoutingKey::FailVenue(signal_id));

    for segment in plan.segments.iter() {
        let key = VenueKey {
            venue: segment.venue,
            venue_id: segment.venue_id,
        };
        let stored = env
            .storage()
            .persistent()
            .get::<_, VenueLiquidity>(&SmartRoutingKey::VenueQuote(signal_id, key.clone()))
            .ok_or(AutoTradeError::AtomicExecutionFailed)?;

        if stored.available_amount < segment.amount || stored.price <= 0 {
            return Err(AutoTradeError::AtomicExecutionFailed);
        }

        if fail_key.as_ref() == Some(&key) {
            return Err(AutoTradeError::AtomicExecutionFailed);
        }
    }

    for segment in plan.segments.iter() {
        let key = VenueKey {
            venue: segment.venue,
            venue_id: segment.venue_id,
        };
        let mut stored = env
            .storage()
            .persistent()
            .get::<_, VenueLiquidity>(&SmartRoutingKey::VenueQuote(signal_id, key.clone()))
            .ok_or(AutoTradeError::AtomicExecutionFailed)?;

        stored.available_amount -= segment.amount;
        env.storage()
            .persistent()
            .set(&SmartRoutingKey::VenueQuote(signal_id, key), &stored);
    }

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "smart_route_executed"), signal_id),
        plan.clone(),
    );

    Ok(ExecutionResult {
        executed_amount: plan.allocated_amount,
        executed_price: plan.average_price,
    })
}

#[cfg(test)]
pub fn set_execution_failure(env: &Env, signal_id: u64, venue: LiquidityVenue, venue_id: u32) {
    env.storage().temporary().set(
        &SmartRoutingKey::FailVenue(signal_id),
        &VenueKey { venue, venue_id },
    );
}

fn finalize_plan(
    env: &Env,
    reference_price: i128,
    requested_amount: i128,
    segments: Vec<RouteSegment>,
    max_slippage_bps: u32,
) -> Result<RoutingPlan, AutoTradeError> {
    let mut allocated_amount = 0i128;
    let mut total_notional = 0i128;
    let mut total_fees = 0i128;

    for segment in segments.iter() {
        allocated_amount += segment.amount;
        total_notional += segment.amount * segment.execution_price;
        total_fees += segment.fee_amount;
    }

    if allocated_amount != requested_amount {
        return Err(AutoTradeError::InsufficientLiquidity);
    }

    let average_price = if allocated_amount == 0 {
        0
    } else {
        total_notional / allocated_amount
    };

    let estimated_slippage_bps = if reference_price <= 0 || average_price <= reference_price {
        0
    } else {
        (((average_price - reference_price) * BPS_DENOMINATOR) / reference_price) as u32
    };

    if estimated_slippage_bps > max_slippage_bps {
        return Err(AutoTradeError::SlippageExceeded);
    }

    Ok(RoutingPlan {
        requested_amount,
        allocated_amount,
        average_price,
        total_fees,
        total_cost: total_notional + total_fees,
        estimated_slippage_bps,
        segments,
    })
}

fn venue_key(quote: &VenueLiquidity) -> VenueKey {
    VenueKey {
        venue: quote.venue,
        venue_id: quote.venue_id,
    }
}

fn compute_slippage_bps(amount: i128, liquidity: i128, max_slippage_bps: u32) -> u32 {
    if liquidity <= 0 {
        return u32::MAX;
    }

    let raw = (amount * max_slippage_bps as i128 + liquidity - 1) / liquidity;
    raw as u32
}

fn apply_bps(value: i128, bps: u32) -> i128 {
    (value * bps as i128 + (BPS_DENOMINATOR - 1)) / BPS_DENOMINATOR
}

fn allocated_for(segments: &Vec<RouteSegment>, venue: LiquidityVenue, venue_id: u32) -> i128 {
    let mut allocated = 0i128;
    for segment in segments.iter() {
        if segment.venue == venue && segment.venue_id == venue_id {
            allocated += segment.amount;
        }
    }
    allocated
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, testutils::Ledger as _, Address, Env};

    #[contract]
    struct TestContract;

    fn setup_env() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(TestContract, ());
        (env, contract_id)
    }

    fn signal(id: u64) -> Signal {
        Signal {
            signal_id: id,
            price: 100,
            expiry: 5_000,
            base_asset: 1,
        }
    }

    fn quote(
        venue: LiquidityVenue,
        venue_id: u32,
        available_amount: i128,
        price: i128,
        fee_bps: u32,
        slippage_bps: u32,
    ) -> VenueLiquidity {
        VenueLiquidity {
            venue,
            venue_id,
            available_amount,
            price,
            fee_bps,
            slippage_bps,
        }
    }

    #[test]
    fn chooses_best_price_across_venues() {
        let (env, contract_id) = setup_env();
        let signal = signal(7);

        env.as_contract(&contract_id, || {
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Sdex, 1, 100, 101, 5, 30),
            )
            .unwrap();
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Pool, 2, 100, 99, 10, 10),
            )
            .unwrap();

            let plan = plan_best_execution(&env, &signal, 50, 100).unwrap();

            assert_eq!(plan.allocated_amount, 50);
            assert_eq!(plan.segments.len(), 1);
            assert_eq!(plan.segments.get(0).unwrap().venue, LiquidityVenue::Pool);
        });
    }

    #[test]
    fn splits_across_multiple_venues_when_needed() {
        let (env, contract_id) = setup_env();
        let signal = signal(8);

        env.as_contract(&contract_id, || {
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Pool, 1, 40, 99, 5, 10),
            )
            .unwrap();
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Sdex, 2, 35, 100, 5, 10),
            )
            .unwrap();
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::PathPayment, 3, 50, 102, 5, 10),
            )
            .unwrap();

            let plan = plan_best_execution(&env, &signal, 90, 100).unwrap();

            assert_eq!(plan.allocated_amount, 90);
            assert_eq!(plan.segments.len(), 3);
            assert_eq!(plan.segments.get(0).unwrap().venue, LiquidityVenue::Pool);
            assert_eq!(plan.segments.get(1).unwrap().venue, LiquidityVenue::Sdex);
            assert_eq!(
                plan.segments.get(2).unwrap().venue,
                LiquidityVenue::PathPayment
            );
        });
    }

    #[test]
    fn rejects_routes_that_exceed_slippage() {
        let (env, contract_id) = setup_env();
        let signal = signal(9);

        env.as_contract(&contract_id, || {
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Sdex, 1, 100, 100, 0, 800),
            )
            .unwrap();

            let err = plan_best_execution(&env, &signal, 100, 50).unwrap_err();
            assert_eq!(err, AutoTradeError::InsufficientLiquidity);
        });
    }

    #[test]
    fn fails_when_total_liquidity_is_too_low() {
        let (env, contract_id) = setup_env();
        let signal = signal(10);

        env.as_contract(&contract_id, || {
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Sdex, 1, 20, 100, 0, 10),
            )
            .unwrap();
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Pool, 2, 20, 101, 0, 10),
            )
            .unwrap();

            let err = plan_best_execution(&env, &signal, 50, 100).unwrap_err();
            assert_eq!(err, AutoTradeError::InsufficientLiquidity);
        });
    }

    #[test]
    fn atomic_execution_rolls_back_on_failure() {
        let (env, contract_id) = setup_env();
        let signal = signal(11);

        env.as_contract(&contract_id, || {
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Sdex, 1, 60, 100, 0, 10),
            )
            .unwrap();
            upsert_venue_liquidity(
                &env,
                signal.signal_id,
                quote(LiquidityVenue::Pool, 2, 60, 101, 0, 10),
            )
            .unwrap();

            let plan = plan_best_execution(&env, &signal, 100, 100).unwrap();
            set_execution_failure(&env, signal.signal_id, LiquidityVenue::Pool, 2);

            let err = execute_plan_atomically(&env, signal.signal_id, &plan).unwrap_err();
            assert_eq!(err, AutoTradeError::AtomicExecutionFailed);

            let venues = get_venue_liquidity(&env, signal.signal_id);
            assert_eq!(venues.get(0).unwrap().available_amount, 60);
            assert_eq!(venues.get(1).unwrap().available_amount, 60);
        });
    }
}
