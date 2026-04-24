#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};
use crate::errors::AutoTradeError;

pub type Asset = u32;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPair {
    pub base: Asset,
    pub quote: Asset,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiquidityVenue {
    SDEX,
    LiquidityPool(u32),
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArbitrageType {
    Simple,
    Triangular,
    Complex,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpportunityStatus {
    Detected,
    Executed,
    Failed,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbLeg {
    pub venue: LiquidityVenue,
    pub asset_in: Asset,
    pub asset_out: Asset,
    pub amount_in: i128,
    pub expected_amount_out: i128,
    pub fee_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitrageOpportunity {
    pub opportunity_id: u64,
    pub arb_type: ArbitrageType,
    pub path: Vec<ArbLeg>,
    pub expected_profit: i128,
    pub expected_profit_pct: u32,
    pub required_capital: i128,
    pub execution_deadline: u64,
    pub status: OpportunityStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitrageStats {
    pub total_opportunities_detected: u64,
    pub total_executed: u64,
    pub total_failed: u64,
    pub total_profit: i128,
    pub avg_profit_pct: u32,
    pub success_rate: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitrageExecutedEvent {
    pub opportunity_id: u64,
    pub user: Address,
    pub expected_profit: i128,
    pub actual_profit: i128,
    pub trade_ids: Vec<u64>,
}

#[contracttype]
pub enum ArbStorageKey {
    ArbitrageStats(Address),
    NextOpportunityId,
}

const PRECISION: i128 = 10_000_000;
const MAX_SINGLE_ARB: i128 = 100_000_000_000;

// Common assets for triangular
const USDC: Asset = 1;
const XLM: Asset = 2;
const BTC: Asset = 3;

fn min(a: i128, b: i128) -> i128 {
    if a < b { a } else { b }
}

fn generate_arb_id(env: &Env) -> u64 {
    let mut id: u64 = env.storage().persistent().get(&ArbStorageKey::NextOpportunityId).unwrap_or(1);
    env.storage().persistent().set(&ArbStorageKey::NextOpportunityId, &(id + 1));
    id
}

fn current_time(env: &Env) -> u64 {
    env.ledger().timestamp()
}

pub fn monitor_arbitrage_opportunities(
    env: &Env,
    asset_pair: AssetPair,
) -> Result<Vec<ArbitrageOpportunity>, AutoTradeError> {
    let mut opportunities = Vec::new(env);

    let sdex_price = get_sdex_price(env, &asset_pair)?;
    let pool_prices = get_all_pool_prices(env, &asset_pair)?;

    for pool in pool_prices.iter() {
        let pool_id = pool.0;
        let pool_price = pool.1;

        let price_diff = if pool_price > sdex_price {
            pool_price - sdex_price
        } else {
            sdex_price - pool_price
        };
        
        // Avoid division by zero
        if sdex_price == 0 {
            continue;
        }

        let diff_pct = (price_diff * 10000) / sdex_price;
        let total_fee_bps = 40; // 0.4%

        if diff_pct > total_fee_bps {
            let profit_pct = (diff_pct - total_fee_bps) as u32;
            let opportunity = if pool_price > sdex_price {
                create_simple_arbitrage(
                    env,
                    LiquidityVenue::SDEX,
                    LiquidityVenue::LiquidityPool(pool_id),
                    asset_pair.clone(),
                    sdex_price,
                    pool_price,
                    profit_pct,
                )?
            } else {
                create_simple_arbitrage(
                    env,
                    LiquidityVenue::LiquidityPool(pool_id),
                    LiquidityVenue::SDEX,
                    asset_pair.clone(),
                    pool_price,
                    sdex_price,
                    profit_pct,
                )?
            };

            opportunities.push_back(opportunity);
        }
    }

    let triangular_opps = find_triangular_arbitrage(env, asset_pair.base)?;
    for opp in triangular_opps.iter() {
        opportunities.push_back(opp);
    }

    Ok(opportunities)
}

fn create_simple_arbitrage(
    env: &Env,
    buy_venue: LiquidityVenue,
    sell_venue: LiquidityVenue,
    pair: AssetPair,
    buy_price: i128,
    sell_price: i128,
    profit_pct: u32,
) -> Result<ArbitrageOpportunity, AutoTradeError> {
    let capital = calculate_optimal_arb_capital(env, buy_price, sell_price, &buy_venue, &sell_venue)?;

    // Handle zero buy price
    if buy_price == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let leg1 = ArbLeg {
        venue: buy_venue.clone(),
        asset_in: pair.quote,
        asset_out: pair.base,
        amount_in: capital,
        expected_amount_out: (capital * 10000) / buy_price,
        fee_bps: get_venue_fee_bps(&buy_venue),
    };

    let leg2 = ArbLeg {
        venue: sell_venue.clone(),
        asset_in: pair.base,
        asset_out: pair.quote,
        amount_in: leg1.expected_amount_out,
        expected_amount_out: (leg1.expected_amount_out * sell_price) / 10000,
        fee_bps: get_venue_fee_bps(&sell_venue),
    };

    let expected_profit = leg2.expected_amount_out - capital;

    // Use Vec to construct path
    let mut path = Vec::new(env);
    path.push_back(leg1);
    path.push_back(leg2);

    Ok(ArbitrageOpportunity {
        opportunity_id: generate_arb_id(env),
        arb_type: ArbitrageType::Simple,
        path,
        expected_profit,
        expected_profit_pct: profit_pct,
        required_capital: capital,
        execution_deadline: current_time(env) + 30, // 30 seconds
        status: OpportunityStatus::Detected,
    })
}

pub fn find_triangular_arbitrage(
    env: &Env,
    base_asset: Asset,
) -> Result<Vec<ArbitrageOpportunity>, AutoTradeError> {
    let mut opportunities = Vec::new(env);

    let mut intermediaries = Vec::new(env);
    intermediaries.push_back(USDC);
    intermediaries.push_back(XLM);
    intermediaries.push_back(BTC);

    for intermediate in intermediaries.iter() {
        if intermediate == base_asset {
            continue;
        }

        for quote_asset in get_tradeable_assets(env)?.iter() {
            if quote_asset == base_asset || quote_asset == intermediate {
                continue;
            }

            let price1 = get_best_price(env, AssetPair { base: base_asset, quote: intermediate })?;
            let price2 = get_best_price(env, AssetPair { base: intermediate, quote: quote_asset })?;
            let price3 = get_best_price(env, AssetPair { base: quote_asset, quote: base_asset })?;

            let start_amount = 1000 * PRECISION;
            let after_leg1 = (start_amount * price1.price) / PRECISION;
            let after_leg2 = (after_leg1 * price2.price) / PRECISION;
            let after_leg3 = (after_leg2 * price3.price) / PRECISION;

            let total_fees = (start_amount * 90) / 10000;
            let net_amount = after_leg3 - total_fees;

            if net_amount > start_amount {
                let profit_pct = ((net_amount - start_amount) * 10000) / start_amount;

                let mut path = Vec::new(env);
                path.push_back(ArbLeg {
                    venue: price1.venue,
                    asset_in: base_asset,
                    asset_out: intermediate,
                    amount_in: start_amount,
                    expected_amount_out: after_leg1,
                    fee_bps: 30,
                });
                path.push_back(ArbLeg {
                    venue: price2.venue,
                    asset_in: intermediate,
                    asset_out: quote_asset,
                    amount_in: after_leg1,
                    expected_amount_out: after_leg2,
                    fee_bps: 30,
                });
                path.push_back(ArbLeg {
                    venue: price3.venue,
                    asset_in: quote_asset,
                    asset_out: base_asset,
                    amount_in: after_leg2,
                    expected_amount_out: after_leg3,
                    fee_bps: 30,
                });

                let opportunity = ArbitrageOpportunity {
                    opportunity_id: generate_arb_id(env),
                    arb_type: ArbitrageType::Triangular,
                    path,
                    expected_profit: net_amount - start_amount,
                    expected_profit_pct: profit_pct as u32,
                    required_capital: start_amount,
                    execution_deadline: current_time(env) + 30,
                    status: OpportunityStatus::Detected,
                };

                opportunities.push_back(opportunity);
            }
        }
    }

    Ok(opportunities)
}

pub fn execute_arbitrage_atomic(
    env: &Env,
    user: Address,
    opportunity: ArbitrageOpportunity,
) -> Result<Vec<u64>, AutoTradeError> {
    if current_time(env) > opportunity.execution_deadline {
        return Err(AutoTradeError::ArbitrageOpportunityExpired); // Defined in errors
    }

    let first_leg = opportunity.path.first().ok_or(AutoTradeError::InvalidPairsConfig)?;
    let user_balance = get_asset_balance(env, user.clone(), first_leg.asset_in)?;
    if user_balance < opportunity.required_capital {
        return Err(AutoTradeError::InsufficientBalance);
    }

    let mut trade_ids = Vec::new(env);
    let mut current_amount = opportunity.required_capital;

    for leg in opportunity.path.iter() {
        let trade_id = execute_venue_trade(
            env,
            user.clone(),
            leg.venue,
            leg.asset_in,
            leg.asset_out,
            current_amount,
        )?;

        trade_ids.push_back(trade_id);

        let trade_result = get_trade_result(env, trade_id)?;
        current_amount = trade_result.amount_out;
    }

    let final_amount = current_amount;
    if final_amount <= opportunity.required_capital {
        return Err(AutoTradeError::ArbitrageUnprofitable); // Rollback tx
    }

    let actual_profit = final_amount - opportunity.required_capital;

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "arbitrage_executed"), user.clone()),
        ArbitrageExecutedEvent {
            opportunity_id: opportunity.opportunity_id,
            user,
            expected_profit: opportunity.expected_profit,
            actual_profit,
            trade_ids: trade_ids.clone(),
        },
    );

    Ok(trade_ids)
}

pub fn calculate_optimal_arb_capital(
    env: &Env,
    buy_price: i128,
    sell_price: i128,
    buy_venue: &LiquidityVenue,
    sell_venue: &LiquidityVenue,
) -> Result<i128, AutoTradeError> {
    let buy_liquidity = get_venue_liquidity(env, buy_venue)?;
    let sell_liquidity = get_venue_liquidity(env, sell_venue)?;

    let max_capital = min(buy_liquidity, sell_liquidity);

    let mut optimal_amount = 0i128;
    let mut max_profit = 0i128;

    let mut amount = 1000i128;
    
    // Prevent division by zero
    if buy_price == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    while amount <= max_capital {
        let buy_slippage = calculate_slippage(env, buy_venue, amount)?;
        let sell_slippage = calculate_slippage(env, sell_venue, amount)?;

        let effective_buy_price = buy_price * (10000 + buy_slippage as i128) / 10000;
        let effective_sell_price = sell_price * (10000 - sell_slippage as i128) / 10000;

        // Prevent division by zero
        if effective_buy_price == 0 {
            break;
        }

        let gross_profit = (effective_sell_price - effective_buy_price) * amount / effective_buy_price;
        let fees = (amount * 40) / 10000;
        let net_profit = gross_profit - fees;

        if net_profit > max_profit {
            max_profit = net_profit;
            optimal_amount = amount;
        } else {
            break;
        }

        amount += 1000;
    }

    Ok(optimal_amount)
}

pub fn protect_against_mev(
    env: &Env,
    opportunity: &ArbitrageOpportunity,
) -> Result<(), AutoTradeError> {
    if opportunity.required_capital > MAX_SINGLE_ARB {
        return Err(AutoTradeError::ArbTooLarge);
    }

    if opportunity.execution_deadline - current_time(env) > 60 {
        return Err(AutoTradeError::FrontRunningRisk);
    }

    let pending_txs = get_pending_transactions_for_assets(env, &opportunity.path)?;
    if !pending_txs.is_empty() {
        return Err(AutoTradeError::FrontRunningRisk);
    }

    Ok(())
}

pub fn update_arbitrage_stats(
    env: &Env,
    user: Address,
    opportunity: &ArbitrageOpportunity,
    actual_profit: i128,
) -> Result<(), AutoTradeError> {
    let mut stats = env
        .storage()
        .persistent()
        .get(&ArbStorageKey::ArbitrageStats(user.clone()))
        .unwrap_or(ArbitrageStats {
            total_opportunities_detected: 0,
            total_executed: 0,
            total_failed: 0,
            total_profit: 0,
            avg_profit_pct: 0,
            success_rate: 0,
        });

    stats.total_executed += 1;
    stats.total_profit += actual_profit;

    let total_attempts = stats.total_executed + stats.total_failed;
    stats.success_rate = ((stats.total_executed * 10000) / total_attempts) as u32;

    if opportunity.required_capital > 0 {
        let profit_pct = ((actual_profit * 10000) / opportunity.required_capital) as u32;
        stats.avg_profit_pct =
            ((stats.avg_profit_pct * (stats.total_executed - 1) as u32) + profit_pct)
                / stats.total_executed as u32;
    }

    env.storage().persistent().set(&ArbStorageKey::ArbitrageStats(user), &stats);

    Ok(())
}

// ==========================================
// Mocks for internal external dependencies
// ==========================================

fn get_sdex_price(_env: &Env, _asset_pair: &AssetPair) -> Result<i128, AutoTradeError> {
    Ok(10000)
}

fn get_all_pool_prices(env: &Env, _asset_pair: &AssetPair) -> Result<Vec<(u32, i128)>, AutoTradeError> {
    let mut vec = Vec::new(env);
    vec.push_back((1, 10050));
    Ok(vec)
}

fn get_venue_fee_bps(venue: &LiquidityVenue) -> u32 {
    match venue {
        LiquidityVenue::SDEX => 10,
        LiquidityVenue::LiquidityPool(_) => 30,
    }
}

fn get_tradeable_assets(env: &Env) -> Result<Vec<Asset>, AutoTradeError> {
    let mut vec = Vec::new(env);
    vec.push_back(1);
    vec.push_back(2);
    vec.push_back(3);
    Ok(vec)
}

struct PriceInfo {
    venue: LiquidityVenue,
    price: i128,
}

fn get_best_price(_env: &Env, _pair: AssetPair) -> Result<PriceInfo, AutoTradeError> {
    Ok(PriceInfo { venue: LiquidityVenue::SDEX, price: 10000 })
}

fn get_asset_balance(_env: &Env, _user: Address, _asset: Asset) -> Result<i128, AutoTradeError> {
    Ok(100_000_000_000)
}

fn execute_venue_trade(
    _env: &Env,
    _user: Address,
    _venue: LiquidityVenue,
    _asset_in: Asset,
    _asset_out: Asset,
    _amount_in: i128,
) -> Result<u64, AutoTradeError> {
    Ok(1)
}

struct TradeResult {
    amount_out: i128,
}

fn get_trade_result(_env: &Env, _trade_id: u64) -> Result<TradeResult, AutoTradeError> {
    Ok(TradeResult { amount_out: 10500 })
}

fn get_venue_liquidity(_env: &Env, _venue: &LiquidityVenue) -> Result<i128, AutoTradeError> {
    Ok(1_000_000_000)
}

fn calculate_slippage(_env: &Env, _venue: &LiquidityVenue, _amount: i128) -> Result<u32, AutoTradeError> {
    Ok(10)
}

fn get_pending_transactions_for_assets(env: &Env, _path: &Vec<ArbLeg>) -> Result<Vec<u64>, AutoTradeError> {
    Ok(Vec::new(env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_optimal_arb_capital() {
        let env = Env::default();
        let capital = calculate_optimal_arb_capital(
            &env,
            10000,
            10100,
            &LiquidityVenue::SDEX,
            &LiquidityVenue::LiquidityPool(1),
        ).unwrap();
        assert!(capital > 0);
    }
}
