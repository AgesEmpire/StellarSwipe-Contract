#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Vec, Symbol};

use crate::errors::AutoTradeError;
use crate::risk;

pub const PRECISION: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionStatus {
    Open,
    Closed,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairsPosition {
    pub position_id: u64,
    pub entry_ratio: i128,
    pub entry_z_score: i128,
    pub entry_time: u64,
    pub long_asset: u32,
    pub short_asset: u32,
    pub long_amount: i128,
    pub short_amount: i128,
    pub status: PositionStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairsTradingStrategy {
    pub user: Address,
    pub asset_a: u32,
    pub asset_b: u32,
    pub lookback_period_days: u32,
    pub entry_z_score: i128,
    pub exit_z_score: i128,
    pub position_size_pct: u32, // out of 10000
    /// `position_id == 0` means no active position.
    pub active_position: PairsPosition,
    pub historical_ratio_mean: i128,
    pub historical_ratio_std_dev: i128,
    pub correlation_coefficient: i128,
}

fn pairs_position_absent() -> PairsPosition {
    PairsPosition {
        position_id: 0,
        entry_ratio: 0,
        entry_z_score: 0,
        entry_time: 0,
        long_asset: 0,
        short_asset: 0,
        long_amount: 0,
        short_amount: 0,
        status: PositionStatus::Closed,
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairAnalysis {
    pub correlation: i128,
    pub ratio_mean: i128,
    pub ratio_std_dev: i128,
    pub current_ratio: i128,
    pub z_score: i128,
    pub is_cointegrated: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairsSignal {
    pub long_asset: u32,
    pub short_asset: u32,
    pub entry_ratio: i128,
    pub z_score: i128,
    pub expected_ratio: i128,
    pub confidence: u32,
}

#[contracttype]
enum PairsDataKey {
    Strategy(Address, u64),
    NextStrategyId,
    NextPositionId,
}

pub fn get_next_position_id(env: &Env) -> u64 {
    let id: u64 = env.storage().persistent().get(&PairsDataKey::NextPositionId).unwrap_or(1);
    env.storage().persistent().set(&PairsDataKey::NextPositionId, &(id + 1));
    id
}

pub fn get_next_strategy_id(env: &Env) -> u64 {
    let id: u64 = env.storage().persistent().get(&PairsDataKey::NextStrategyId).unwrap_or(1);
    env.storage().persistent().set(&PairsDataKey::NextStrategyId, &(id + 1));
    id
}

pub fn configure_pairs_strategy(
    env: &Env,
    user: Address,
    asset_a: u32,
    asset_b: u32,
    lookback_period_days: u32,
    entry_z_score: i128,
    exit_z_score: i128,
    position_size_pct: u32,
) -> Result<u64, AutoTradeError> {
    if lookback_period_days == 0 || entry_z_score <= exit_z_score || position_size_pct == 0 || position_size_pct > 10000 {
        return Err(AutoTradeError::InvalidPairsConfig);
    }
    
    let strategy_id = get_next_strategy_id(env);
    
    let strategy = PairsTradingStrategy {
        user: user.clone(),
        asset_a,
        asset_b,
        lookback_period_days,
        entry_z_score,
        exit_z_score,
        position_size_pct,
        active_position: pairs_position_absent(),
        historical_ratio_mean: 0,
        historical_ratio_std_dev: 0,
        correlation_coefficient: 0,
    };
    
    save_strategy(env, &user, strategy_id, &strategy);
    
    Ok(strategy_id)
}

pub fn get_pairs_trading_strategy(env: &Env, user: &Address, strategy_id: u64) -> Result<PairsTradingStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&PairsDataKey::Strategy(user.clone(), strategy_id))
        .ok_or(AutoTradeError::PairsStrategyNotFound)
}

pub fn save_strategy(env: &Env, user: &Address, strategy_id: u64, strategy: &PairsTradingStrategy) {
    env.storage()
        .persistent()
        .set(&PairsDataKey::Strategy(user.clone(), strategy_id), strategy);
}

fn get_historical_prices(env: &Env, asset_id: u32, _lookback_seconds: u64) -> Result<Vec<i128>, AutoTradeError> {
    let history = crate::strategies::stat_arb::get_price_history(env, asset_id);
    if history.len() < 30 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }
    Ok(history)
}

pub fn analyze_asset_pair(
    env: &Env,
    asset_a: u32,
    asset_b: u32,
    lookback_days: u32,
) -> Result<PairAnalysis, AutoTradeError> {
    let lookback_seconds = lookback_days as u64 * 86400;

    let prices_a = get_historical_prices(env, asset_a, lookback_seconds)?;
    let prices_b = get_historical_prices(env, asset_b, lookback_seconds)?;

    if prices_a.len() != prices_b.len() || prices_a.len() < 30 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let correlation = calculate_correlation_from_prices(env, &prices_a, &prices_b)?;

    let mut ratios = Vec::new(env);
    let mut ratio_sum = 0i128;
    for i in 0..prices_a.len() {
        let pa = prices_a.get(i).unwrap();
        let pb = prices_b.get(i).unwrap();
        if pb == 0 {
            return Err(AutoTradeError::InvalidPriceData);
        }
        let ratio = (pa * PRECISION) / pb;
        ratios.push_back(ratio);
        ratio_sum += ratio;
    }

    let n = ratios.len() as i128;
    let ratio_mean = ratio_sum / n;

    let mut variance_sum = 0i128;
    for i in 0..ratios.len() {
        let r = ratios.get(i).unwrap();
        let diff = r - ratio_mean;
        // Avoid overflow for large prices
        variance_sum += (diff * diff) / n;
    }
    let ratio_std_dev = integer_sqrt(variance_sum);

    let current_ratio = ratios.get(ratios.len() - 1).unwrap();

    let z_score = if ratio_std_dev > 0 {
        ((current_ratio - ratio_mean) * PRECISION) / ratio_std_dev
    } else {
        0
    };

    let is_cointegrated = test_cointegration(env, &ratios)?;

    Ok(PairAnalysis {
        correlation,
        ratio_mean,
        ratio_std_dev,
        current_ratio,
        z_score,
        is_cointegrated,
    })
}

pub fn calculate_correlation_from_prices(
    _env: &Env,
    prices_a: &Vec<i128>,
    prices_b: &Vec<i128>,
) -> Result<i128, AutoTradeError> {
    let n = prices_a.len() as i128;
    let mut sum_a = 0i128;
    let mut sum_b = 0i128;

    for i in 0..prices_a.len() {
        sum_a += prices_a.get(i).unwrap();
        sum_b += prices_b.get(i).unwrap();
    }
    
    let mean_a = sum_a / n;
    let mean_b = sum_b / n;

    let mut numerator = 0i128;
    let mut sum_sq_a = 0i128;
    let mut sum_sq_b = 0i128;

    for i in 0..prices_a.len() {
        let diff_a = prices_a.get(i).unwrap() - mean_a;
        let diff_b = prices_b.get(i).unwrap() - mean_b;
        
        numerator += (diff_a * diff_b) / n;
        sum_sq_a += (diff_a * diff_a) / n;
        sum_sq_b += (diff_b * diff_b) / n;
    }

    let denominator = integer_sqrt(sum_sq_a * sum_sq_b);
    
    if denominator == 0 {
        return Ok(0);
    }

    Ok((numerator * PRECISION) / denominator)
}

fn integer_sqrt(value: i128) -> i128 {
    if value <= 0 {
        return 0;
    }
    let mut x0 = value;
    let mut x1 = (x0 + value / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + value / x0) / 2;
    }
    x0
}

pub fn test_cointegration(_env: &Env, ratios: &Vec<i128>) -> Result<bool, AutoTradeError> {
    if ratios.is_empty() {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }
    let mut sum = 0i128;
    for i in 0..ratios.len() {
        sum += ratios.get(i).unwrap();
    }
    let mean = sum / ratios.len() as i128;

    let mut crossings = 0;
    for i in 1..ratios.len() {
        let prev = ratios.get(i - 1).unwrap();
        let curr = ratios.get(i).unwrap();
        if (prev > mean && curr <= mean) || (prev < mean && curr >= mean) {
            crossings += 1;
        }
    }

    let crossing_rate = (crossings * 100) / ratios.len() as u32;
    Ok(crossing_rate > 20)
}

pub fn check_pairs_trading_signal(
    env: &Env,
    user: &Address,
    strategy_id: u64,
) -> Result<Option<PairsSignal>, AutoTradeError> {
    let mut strategy = get_pairs_trading_strategy(env, user, strategy_id)?;

    if strategy.active_position.position_id != 0 {
        return Ok(None);
    }

    let analysis = analyze_asset_pair(env, strategy.asset_a, strategy.asset_b, strategy.lookback_period_days)?;

    strategy.historical_ratio_mean = analysis.ratio_mean;
    strategy.historical_ratio_std_dev = analysis.ratio_std_dev;
    strategy.correlation_coefficient = analysis.correlation;
    save_strategy(env, user, strategy_id, &strategy);

    if abs_i128(analysis.correlation) <= 7000 {
        return Err(AutoTradeError::InsufficientCorrelation);
    }
    if !analysis.is_cointegrated {
        return Err(AutoTradeError::PairNotCointegrated);
    }

    if abs_i128(analysis.z_score) >= strategy.entry_z_score {
        let confidence = calculate_pairs_confidence(&analysis)?;
        let signal = if analysis.z_score > 0 {
            PairsSignal {
                long_asset: strategy.asset_b,
                short_asset: strategy.asset_a,
                entry_ratio: analysis.current_ratio,
                z_score: analysis.z_score,
                expected_ratio: analysis.ratio_mean,
                confidence,
            }
        } else {
            PairsSignal {
                long_asset: strategy.asset_a,
                short_asset: strategy.asset_b,
                entry_ratio: analysis.current_ratio,
                z_score: analysis.z_score,
                expected_ratio: analysis.ratio_mean,
                confidence,
            }
        };
        Ok(Some(signal))
    } else {
        Ok(None)
    }
}

pub fn calculate_pairs_confidence(analysis: &PairAnalysis) -> Result<u32, AutoTradeError> {
    let corr_score = (abs_i128(analysis.correlation) as u32) / 10;
    let z_score_contribution = core::cmp::min(3000, (abs_i128(analysis.z_score) / 10) as u32);
    let cointegration_bonus = if analysis.is_cointegrated { 2000 } else { 0 };

    let total = corr_score + z_score_contribution + cointegration_bonus;
    Ok(core::cmp::min(10000, total))
}

fn abs_i128(val: i128) -> i128 {
    if val < 0 {
        -val
    } else {
        val
    }
}

pub fn calculate_returns(env: &Env, prices: &Vec<i128>) -> Vec<i128> {
    let mut returns = Vec::new(env);
    for i in 1..prices.len() {
        let prev = prices.get(i - 1).unwrap();
        let curr = prices.get(i).unwrap();
        if prev > 0 {
            returns.push_back(((curr - prev) * PRECISION) / prev);
        } else {
            returns.push_back(0);
        }
    }
    returns
}

pub fn calculate_optimal_hedge_ratio(
    env: &Env,
    asset_a: u32,
    asset_b: u32,
    lookback_days: u32,
) -> Result<i128, AutoTradeError> {
    let lookback_seconds = lookback_days as u64 * 86400;
    let prices_a = get_historical_prices(env, asset_a, lookback_seconds)?;
    let prices_b = get_historical_prices(env, asset_b, lookback_seconds)?;

    let returns_a = calculate_returns(env, &prices_a);
    let returns_b = calculate_returns(env, &prices_b);

    if returns_a.is_empty() || returns_b.is_empty() {
        return Ok(PRECISION);
    }

    let mut sum_b = 0i128;
    let mut sum_a = 0i128;
    for i in 0..returns_b.len() {
        sum_b += returns_b.get(i).unwrap();
        sum_a += returns_a.get(i).unwrap();
    }
    
    let n = returns_b.len() as i128;
    let mean_b = sum_b / n;
    let mean_a = sum_a / n;

    let mut covariance_sum = 0i128;
    let mut variance_b_sum = 0i128;

    for i in 0..returns_a.len() {
        let ra = returns_a.get(i).unwrap();
        let rb = returns_b.get(i).unwrap();
        let diff_a = ra - mean_a;
        let diff_b = rb - mean_b;
        covariance_sum += (diff_a * diff_b) / n;
        variance_b_sum += (diff_b * diff_b) / n;
    }

    let covariance = covariance_sum;
    let variance_b = variance_b_sum;

    if variance_b == 0 {
        return Ok(PRECISION);
    }

    let beta = (covariance * PRECISION) / variance_b;
    Ok(beta)
}

fn current_time(env: &Env) -> u64 {
    env.ledger().timestamp()
}

pub fn execute_pairs_trade(
    env: &Env,
    user: &Address,
    strategy_id: u64,
    signal: PairsSignal,
    portfolio_value: i128,
) -> Result<u64, AutoTradeError> {
    let mut strategy = get_pairs_trading_strategy(env, user, strategy_id)?;

    if strategy.active_position.position_id != 0 {
        return Err(AutoTradeError::PairsActivePositionExists);
    }

    let total_position_value = (portfolio_value * strategy.position_size_pct as i128) / 10000;

    let long_amount = total_position_value / 2;
    let short_amount = total_position_value / 2;

    let position_id = get_next_position_id(env);

    let position = PairsPosition {
        position_id,
        entry_ratio: signal.entry_ratio,
        entry_z_score: signal.z_score,
        entry_time: current_time(env),
        long_asset: signal.long_asset,
        short_asset: signal.short_asset,
        long_amount,
        short_amount,
        status: PositionStatus::Open,
    };

    strategy.active_position = position;
    save_strategy(env, user, strategy_id, &strategy);

    env.events().publish(
        (Symbol::new(env, "pairs_trade_exec"), strategy_id),
        (position_id, signal.long_asset, signal.short_asset, signal.z_score),
    );

    Ok(position_id)
}

pub fn check_pairs_exit(
    env: &Env,
    user: &Address,
    strategy_id: u64,
) -> Result<Option<u64>, AutoTradeError> {
    let mut strategy = get_pairs_trading_strategy(env, user, strategy_id)?;

    if strategy.active_position.position_id == 0 {
        return Err(AutoTradeError::PairsNoActivePosition);
    }
    let position = strategy.active_position.clone();

    let analysis = analyze_asset_pair(
        env,
        strategy.asset_a,
        strategy.asset_b,
        strategy.lookback_period_days,
    )?;

    let should_exit = 
        abs_i128(analysis.z_score) <= strategy.exit_z_score ||
        abs_i128(analysis.z_score) >= (abs_i128(position.entry_z_score) * 2);

    if should_exit {
        let total_pnl = 0i128; // Usually we fetch price diff for local pnl check
        
        env.events().publish(
            (Symbol::new(env, "pairs_pos_closed"), strategy_id),
            (position.position_id, analysis.current_ratio, total_pnl, current_time(env) - position.entry_time),
        );
        
        strategy.active_position = pairs_position_absent();
        save_strategy(env, user, strategy_id, &strategy);
        
        return Ok(Some(position.position_id));
    }

    Ok(None)
}
