#![allow(dead_code)]

use core::cmp::{max, min};

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

use crate::errors::AutoTradeError;
use crate::risk;

pub const STAT_ARB_SCALE: i128 = 10_000;
const MIN_BASKET_SIZE: u32 = 3;
const MAX_BASKET_SIZE: u32 = 5;
const MIN_HISTORY_POINTS: u32 = 4;
const MAX_HISTORY_POINTS: u32 = 120;
const MAX_RESIDUAL_HISTORY: u32 = 32;
const LN_2_FIXED: i128 = 6_931;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatArbSignalAction {
    Hold,
    EnterLong,
    EnterShort,
    Exit,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatArbExitReason {
    None,
    Converged,
    StopLoss,
    CointegrationBreakdown,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatArbAssetPosition {
    pub asset_id: u32,
    pub quantity: i128,
    pub entry_price: i128,
    pub target_weight: i128,
    pub is_long: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatArbPortfolio {
    pub portfolio_id: u64,
    pub asset_positions: Vec<StatArbAssetPosition>,
    pub entry_residual: i128,
    pub entry_z_score: i128,
    pub entry_time: u64,
    pub last_rebalanced_at: u64,
    pub total_value: i128,
    pub is_long_residual: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatArbStrategy {
    pub user: Address,
    pub asset_basket: Vec<u32>,
    pub lookback_period_days: u32,
    pub cointegration_threshold: i128,
    pub hedge_ratios: Vec<i128>,
    pub entry_z_score: i128,
    pub exit_z_score: i128,
    pub rebalance_frequency_hours: u32,
    pub active_portfolio: bool,
    pub residual_history: Vec<i128>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CointegrationTest {
    pub asset_group: Vec<u32>,
    pub is_cointegrated: bool,
    pub adf_statistic: i128,
    pub p_value: i128,
    pub hedge_ratios: Vec<i128>,
    pub half_life: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatArbSignal {
    pub action: StatArbSignalAction,
    pub residual: i128,
    pub z_score: i128,
    pub half_life: u32,
    pub is_cointegrated: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatArbExitCheck {
    pub should_exit: bool,
    pub reason: StatArbExitReason,
    pub residual: i128,
    pub z_score: i128,
    pub is_cointegrated: bool,
}

#[contracttype]
enum StatArbDataKey {
    Strategy(Address),
    ActivePortfolio(Address),
    PriceHistory(u32),
    NextPortfolioId,
}

pub fn set_price_history(
    env: &Env,
    asset_id: u32,
    prices: Vec<i128>,
) -> Result<(), AutoTradeError> {
    validate_price_history(&prices)?;

    env.storage()
        .persistent()
        .set(&StatArbDataKey::PriceHistory(asset_id), &prices);

    if let Some(price) = prices.get(prices.len() - 1) {
        risk::set_asset_price(env, asset_id, price);
    }

    Ok(())
}

pub fn get_price_history(env: &Env, asset_id: u32) -> Vec<i128> {
    env.storage()
        .persistent()
        .get(&StatArbDataKey::PriceHistory(asset_id))
        .unwrap_or_else(|| Vec::new(env))
}

#[allow(clippy::too_many_arguments)]
pub fn configure_strategy(
    env: &Env,
    user: &Address,
    asset_basket: Vec<u32>,
    lookback_period_days: u32,
    cointegration_threshold: i128,
    entry_z_score: i128,
    exit_z_score: i128,
    rebalance_frequency_hours: u32,
) -> Result<StatArbStrategy, AutoTradeError> {
    validate_asset_basket(&asset_basket)?;

    if lookback_period_days < MIN_HISTORY_POINTS
        || cointegration_threshold <= 0
        || entry_z_score <= 0
        || exit_z_score < 0
        || exit_z_score >= entry_z_score
    {
        return Err(AutoTradeError::InvalidStatArbConfig);
    }

    let strategy = StatArbStrategy {
        user: user.clone(),
        asset_basket,
        lookback_period_days,
        cointegration_threshold,
        hedge_ratios: Vec::new(env),
        entry_z_score,
        exit_z_score,
        rebalance_frequency_hours,
        active_portfolio: false,
        residual_history: Vec::new(env),
    };

    save_strategy(env, &strategy);
    Ok(strategy)
}

pub fn get_strategy(env: &Env, user: &Address) -> Option<StatArbStrategy> {
    env.storage()
        .persistent()
        .get(&StatArbDataKey::Strategy(user.clone()))
}

pub fn get_active_portfolio(env: &Env, user: &Address) -> Option<StatArbPortfolio> {
    env.storage()
        .persistent()
        .get(&StatArbDataKey::ActivePortfolio(user.clone()))
}

pub fn test_cointegration_for_assets(
    env: &Env,
    asset_basket: Vec<u32>,
    lookback_period_days: u32,
    cointegration_threshold: i128,
) -> Result<CointegrationTest, AutoTradeError> {
    validate_asset_basket(&asset_basket)?;
    let aligned = get_aligned_price_histories(env, &asset_basket, lookback_period_days)?;
    let hedge_ratios = calculate_hedge_ratios_ols(env, &aligned)?;
    let residuals = construct_portfolio(env, &aligned, &hedge_ratios)?;
    let (adf_statistic, p_value) = augmented_dickey_fuller_test(env, &residuals)?;
    let half_life = calculate_mean_reversion_halflife(adf_statistic);
    let is_cointegrated = adf_statistic <= -cointegration_threshold && half_life > 0;

    Ok(CointegrationTest {
        asset_group: asset_basket,
        is_cointegrated,
        adf_statistic,
        p_value,
        hedge_ratios,
        half_life,
    })
}

pub fn test_cointegration(env: &Env, user: &Address) -> Result<CointegrationTest, AutoTradeError> {
    let strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    test_cointegration_for_assets(
        env,
        strategy.asset_basket,
        strategy.lookback_period_days,
        strategy.cointegration_threshold,
    )
}

pub fn check_stat_arb_signal(env: &Env, user: &Address) -> Result<StatArbSignal, AutoTradeError> {
    let strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    let test = test_cointegration(env, user)?;
    let aligned =
        get_aligned_price_histories(env, &strategy.asset_basket, strategy.lookback_period_days)?;
    let residuals = construct_portfolio(env, &aligned, &test.hedge_ratios)?;
    let latest_residual = residuals
        .get(residuals.len() - 1)
        .ok_or(AutoTradeError::InsufficientPriceHistory)?;
    let z_score = calculate_z_score(&residuals, latest_residual)?;

    let action = if !test.is_cointegrated {
        StatArbSignalAction::Hold
    } else if z_score >= strategy.entry_z_score {
        StatArbSignalAction::EnterShort
    } else if z_score <= -strategy.entry_z_score {
        StatArbSignalAction::EnterLong
    } else if abs_i128(z_score) <= strategy.exit_z_score {
        StatArbSignalAction::Exit
    } else {
        StatArbSignalAction::Hold
    };

    Ok(StatArbSignal {
        action,
        residual: latest_residual,
        z_score,
        half_life: test.half_life,
        is_cointegrated: test.is_cointegrated,
    })
}

pub fn execute_stat_arb_trade(
    env: &Env,
    user: &Address,
    total_value: i128,
) -> Result<StatArbPortfolio, AutoTradeError> {
    if total_value <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    if get_active_portfolio(env, user).is_some() {
        return Err(AutoTradeError::ActivePortfolioExists);
    }

    let mut strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    let test = test_cointegration(env, user)?;
    if !test.is_cointegrated {
        return Err(AutoTradeError::NonCointegratedBasket);
    }

    let signal = check_stat_arb_signal(env, user)?;
    let is_long_residual = match signal.action {
        StatArbSignalAction::EnterLong => true,
        StatArbSignalAction::EnterShort => false,
        _ => return Err(AutoTradeError::NoTradeSignal),
    };

    let positions = build_positions(
        env,
        &strategy.asset_basket,
        &test.hedge_ratios,
        total_value,
        is_long_residual,
    )?;
    let portfolio = StatArbPortfolio {
        portfolio_id: next_portfolio_id(env),
        asset_positions: positions,
        entry_residual: signal.residual,
        entry_z_score: signal.z_score,
        entry_time: env.ledger().timestamp(),
        last_rebalanced_at: env.ledger().timestamp(),
        total_value,
        is_long_residual,
    };

    strategy.hedge_ratios = test.hedge_ratios.clone();
    strategy.active_portfolio = true;
    strategy.residual_history = truncate_tail(
        env,
        &collect_recent_residuals(env, user, &strategy.hedge_ratios)?,
        MAX_RESIDUAL_HISTORY,
    );
    save_strategy(env, &strategy);
    env.storage()
        .persistent()
        .set(&StatArbDataKey::ActivePortfolio(user.clone()), &portfolio);

    Ok(portfolio)
}

pub fn rebalance_stat_arb_portfolio(
    env: &Env,
    user: &Address,
) -> Result<StatArbPortfolio, AutoTradeError> {
    let mut strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    let active = get_active_portfolio(env, user).ok_or(AutoTradeError::NoActivePortfolio)?;
    let test = test_cointegration(env, user)?;
    if !test.is_cointegrated {
        return Err(AutoTradeError::NonCointegratedBasket);
    }

    let min_rebalance_gap = strategy.rebalance_frequency_hours as u64 * 3600;
    if env
        .ledger()
        .timestamp()
        .saturating_sub(active.last_rebalanced_at)
        < min_rebalance_gap
    {
        return Ok(active);
    }

    let updated_positions = build_positions(
        env,
        &strategy.asset_basket,
        &test.hedge_ratios,
        active.total_value,
        active.is_long_residual,
    )?;

    let updated = StatArbPortfolio {
        portfolio_id: active.portfolio_id,
        asset_positions: updated_positions,
        entry_residual: active.entry_residual,
        entry_z_score: active.entry_z_score,
        entry_time: active.entry_time,
        last_rebalanced_at: env.ledger().timestamp(),
        total_value: active.total_value,
        is_long_residual: active.is_long_residual,
    };

    strategy.hedge_ratios = test.hedge_ratios.clone();
    strategy.residual_history = truncate_tail(
        env,
        &collect_recent_residuals(env, user, &strategy.hedge_ratios)?,
        MAX_RESIDUAL_HISTORY,
    );
    save_strategy(env, &strategy);
    env.storage()
        .persistent()
        .set(&StatArbDataKey::ActivePortfolio(user.clone()), &updated);

    Ok(updated)
}

pub fn check_stat_arb_exit(env: &Env, user: &Address) -> Result<StatArbExitCheck, AutoTradeError> {
    let strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    let portfolio = get_active_portfolio(env, user).ok_or(AutoTradeError::NoActivePortfolio)?;
    let signal = check_stat_arb_signal(env, user)?;

    if !signal.is_cointegrated {
        return Ok(StatArbExitCheck {
            should_exit: true,
            reason: StatArbExitReason::CointegrationBreakdown,
            residual: signal.residual,
            z_score: signal.z_score,
            is_cointegrated: false,
        });
    }

    if abs_i128(signal.z_score) <= strategy.exit_z_score {
        return Ok(StatArbExitCheck {
            should_exit: true,
            reason: StatArbExitReason::Converged,
            residual: signal.residual,
            z_score: signal.z_score,
            is_cointegrated: true,
        });
    }

    let risk_config = risk::get_risk_config(env, user);
    let entry_reference = max(abs_i128(portfolio.entry_residual), 1);
    let adverse_move = if portfolio.is_long_residual {
        max(portfolio.entry_residual - signal.residual, 0)
    } else {
        max(signal.residual - portfolio.entry_residual, 0)
    };

    if adverse_move * 100 >= entry_reference * risk_config.stop_loss_pct as i128 {
        return Ok(StatArbExitCheck {
            should_exit: true,
            reason: StatArbExitReason::StopLoss,
            residual: signal.residual,
            z_score: signal.z_score,
            is_cointegrated: true,
        });
    }

    Ok(StatArbExitCheck {
        should_exit: false,
        reason: StatArbExitReason::None,
        residual: signal.residual,
        z_score: signal.z_score,
        is_cointegrated: true,
    })
}

pub fn close_stat_arb_portfolio(
    env: &Env,
    user: &Address,
) -> Result<StatArbPortfolio, AutoTradeError> {
    let mut strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    let portfolio = get_active_portfolio(env, user).ok_or(AutoTradeError::NoActivePortfolio)?;

    env.storage()
        .persistent()
        .remove(&StatArbDataKey::ActivePortfolio(user.clone()));

    strategy.active_portfolio = false;
    save_strategy(env, &strategy);

    Ok(portfolio)
}

pub fn calculate_hedge_ratios_ols(
    env: &Env,
    aligned_histories: &Vec<Vec<i128>>,
) -> Result<Vec<i128>, AutoTradeError> {
    if aligned_histories.len() < MIN_BASKET_SIZE {
        return Err(AutoTradeError::InvalidBasketSize);
    }

    let base_series = aligned_histories
        .get(0)
        .ok_or(AutoTradeError::InsufficientPriceHistory)?;

    let mut hedge_ratios = Vec::new(env);
    hedge_ratios.push_back(STAT_ARB_SCALE);

    for i in 1..aligned_histories.len() {
        let series = aligned_histories
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        let beta = calculate_regression_coefficient(&base_series, &series)?;
        hedge_ratios.push_back(-beta);
    }

    Ok(hedge_ratios)
}

pub fn calculate_regression_coefficient(
    dependent: &Vec<i128>,
    independent: &Vec<i128>,
) -> Result<i128, AutoTradeError> {
    if dependent.len() != independent.len() || dependent.len() < MIN_HISTORY_POINTS {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let n = dependent.len() as i128;
    let mut sum_y = 0i128;
    let mut sum_x = 0i128;
    let mut sum_xy = 0i128;
    let mut sum_x2 = 0i128;

    for i in 0..dependent.len() {
        let y = dependent
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        let x = independent
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        sum_y += y;
        sum_x += x;
        sum_xy += x * y;
        sum_x2 += x * x;
    }

    let numerator = n * sum_xy - (sum_x * sum_y);
    let denominator = n * sum_x2 - (sum_x * sum_x);
    if denominator == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    Ok(numerator * STAT_ARB_SCALE / denominator)
}

pub fn construct_portfolio(
    env: &Env,
    aligned_histories: &Vec<Vec<i128>>,
    hedge_ratios: &Vec<i128>,
) -> Result<Vec<i128>, AutoTradeError> {
    if aligned_histories.len() != hedge_ratios.len() || aligned_histories.is_empty() {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let sample_len = aligned_histories
        .get(0)
        .ok_or(AutoTradeError::InsufficientPriceHistory)?
        .len();
    let mut residuals = Vec::new(env);

    for sample_idx in 0..sample_len {
        let mut residual = 0i128;
        for asset_idx in 0..aligned_histories.len() {
            let asset_prices = aligned_histories
                .get(asset_idx)
                .ok_or(AutoTradeError::InsufficientPriceHistory)?;
            let price = asset_prices
                .get(sample_idx)
                .ok_or(AutoTradeError::InsufficientPriceHistory)?;
            let ratio = hedge_ratios
                .get(asset_idx)
                .ok_or(AutoTradeError::InvalidPriceData)?;
            residual += price * ratio / STAT_ARB_SCALE;
        }
        residuals.push_back(residual);
    }

    Ok(residuals)
}

pub fn augmented_dickey_fuller_test(
    _env: &Env,
    residuals: &Vec<i128>,
) -> Result<(i128, i128), AutoTradeError> {
    if residuals.len() < MIN_HISTORY_POINTS {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let mut lagged = Vec::new(_env);
    let mut diffs = Vec::new(_env);

    for i in 1..residuals.len() {
        let previous = residuals
            .get(i - 1)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        let current = residuals
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        lagged.push_back(previous);
        diffs.push_back(current - previous);
    }

    let phi = calculate_regression_coefficient(&diffs, &lagged)?;
    let p_value = (STAT_ARB_SCALE - abs_i128(phi)).clamp(0, STAT_ARB_SCALE);
    Ok((phi, p_value))
}

pub fn calculate_mean_reversion_halflife(adf_statistic: i128) -> u32 {
    if adf_statistic >= 0 {
        return 0;
    }

    let speed = abs_i128(adf_statistic);
    if speed == 0 {
        return 0;
    }

    (LN_2_FIXED * STAT_ARB_SCALE / speed) as u32
}

fn validate_asset_basket(asset_basket: &Vec<u32>) -> Result<(), AutoTradeError> {
    let len = asset_basket.len();
    if !(MIN_BASKET_SIZE..=MAX_BASKET_SIZE).contains(&len) {
        return Err(AutoTradeError::InvalidBasketSize);
    }

    for i in 0..len {
        let asset = asset_basket
            .get(i)
            .ok_or(AutoTradeError::InvalidBasketSize)?;
        if asset == 0 {
            return Err(AutoTradeError::InvalidBasketSize);
        }
        for j in (i + 1)..len {
            if asset
                == asset_basket
                    .get(j)
                    .ok_or(AutoTradeError::InvalidBasketSize)?
            {
                return Err(AutoTradeError::InvalidBasketSize);
            }
        }
    }

    Ok(())
}

fn validate_price_history(prices: &Vec<i128>) -> Result<(), AutoTradeError> {
    if prices.len() < MIN_HISTORY_POINTS || prices.len() > MAX_HISTORY_POINTS {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    for i in 0..prices.len() {
        if prices
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?
            <= 0
        {
            return Err(AutoTradeError::InvalidPriceData);
        }
    }

    Ok(())
}

fn get_aligned_price_histories(
    env: &Env,
    asset_basket: &Vec<u32>,
    lookback_period_days: u32,
) -> Result<Vec<Vec<i128>>, AutoTradeError> {
    let mut shortest = u32::MAX;
    let mut all_histories = Vec::new(env);

    for i in 0..asset_basket.len() {
        let asset_id = asset_basket
            .get(i)
            .ok_or(AutoTradeError::InvalidBasketSize)?;
        let history = get_price_history(env, asset_id);
        validate_price_history(&history)?;
        shortest = min(shortest, history.len());
        all_histories.push_back(history);
    }

    let target_len = min(shortest, max(lookback_period_days, MIN_HISTORY_POINTS));
    if target_len < MIN_HISTORY_POINTS {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let mut aligned = Vec::new(env);
    for i in 0..all_histories.len() {
        let history = all_histories
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        aligned.push_back(truncate_tail(env, &history, target_len));
    }

    Ok(aligned)
}

fn collect_recent_residuals(
    env: &Env,
    user: &Address,
    hedge_ratios: &Vec<i128>,
) -> Result<Vec<i128>, AutoTradeError> {
    let strategy = get_strategy(env, user).ok_or(AutoTradeError::InvalidStatArbConfig)?;
    let aligned =
        get_aligned_price_histories(env, &strategy.asset_basket, strategy.lookback_period_days)?;
    construct_portfolio(env, &aligned, hedge_ratios)
}

fn build_positions(
    env: &Env,
    asset_basket: &Vec<u32>,
    hedge_ratios: &Vec<i128>,
    total_value: i128,
    is_long_residual: bool,
) -> Result<Vec<StatArbAssetPosition>, AutoTradeError> {
    let mut total_abs_weight = 0i128;
    for i in 0..hedge_ratios.len() {
        total_abs_weight += abs_i128(
            hedge_ratios
                .get(i)
                .ok_or(AutoTradeError::InvalidPriceData)?,
        );
    }
    if total_abs_weight == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let mut positions = Vec::new(env);
    for i in 0..asset_basket.len() {
        let asset_id = asset_basket
            .get(i)
            .ok_or(AutoTradeError::InvalidBasketSize)?;
        let hedge_ratio = hedge_ratios
            .get(i)
            .ok_or(AutoTradeError::InvalidPriceData)?;
        let latest_price = latest_price_for_asset(env, asset_id)?;
        let leg_value = total_value * abs_i128(hedge_ratio) / total_abs_weight;
        let quantity = max(1, leg_value * STAT_ARB_SCALE / latest_price);
        let base_is_long = hedge_ratio > 0;
        let is_long = if is_long_residual {
            base_is_long
        } else {
            !base_is_long
        };

        positions.push_back(StatArbAssetPosition {
            asset_id,
            quantity,
            entry_price: latest_price,
            target_weight: hedge_ratio,
            is_long,
        });
    }

    Ok(positions)
}

fn calculate_z_score(residuals: &Vec<i128>, latest_residual: i128) -> Result<i128, AutoTradeError> {
    let avg = mean(residuals)?;
    let variance = variance(residuals, avg)?;
    if variance == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    let std_dev = integer_sqrt(variance);
    if std_dev == 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }

    Ok((latest_residual - avg) * STAT_ARB_SCALE / std_dev)
}

fn mean(series: &Vec<i128>) -> Result<i128, AutoTradeError> {
    if series.is_empty() {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }
    let mut total = 0i128;
    for i in 0..series.len() {
        total += series
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
    }
    Ok(total / series.len() as i128)
}

fn variance(series: &Vec<i128>, avg: i128) -> Result<i128, AutoTradeError> {
    if series.len() < 2 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let mut total = 0i128;
    for i in 0..series.len() {
        let point = series
            .get(i)
            .ok_or(AutoTradeError::InsufficientPriceHistory)?;
        let diff = point - avg;
        total += diff * diff;
    }
    Ok(total / series.len() as i128)
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

fn latest_price_for_asset(env: &Env, asset_id: u32) -> Result<i128, AutoTradeError> {
    if let Some(price) = risk::get_asset_price(env, asset_id) {
        if price > 0 {
            return Ok(price);
        }
    }

    let history = get_price_history(env, asset_id);
    let price = history
        .get(history.len().saturating_sub(1))
        .ok_or(AutoTradeError::InsufficientPriceHistory)?;
    if price <= 0 {
        return Err(AutoTradeError::InvalidPriceData);
    }
    Ok(price)
}

fn truncate_tail(env: &Env, series: &Vec<i128>, target_len: u32) -> Vec<i128> {
    let mut truncated = Vec::new(env);
    let start = series.len().saturating_sub(target_len);
    for i in start..series.len() {
        if let Some(value) = series.get(i) {
            truncated.push_back(value);
        }
    }
    truncated
}

fn save_strategy(env: &Env, strategy: &StatArbStrategy) {
    env.storage()
        .persistent()
        .set(&StatArbDataKey::Strategy(strategy.user.clone()), strategy);
}

fn next_portfolio_id(env: &Env) -> u64 {
    let next = env
        .storage()
        .persistent()
        .get(&StatArbDataKey::NextPortfolioId)
        .unwrap_or(1u64);
    env.storage()
        .persistent()
        .set(&StatArbDataKey::NextPortfolioId, &(next + 1));
    next
}

fn abs_i128(value: i128) -> i128 {
    if value < 0 {
        -value
    } else {
        value
    }
}

pub fn emit_strategy_configured(env: &Env, user: &Address, strategy: &StatArbStrategy) {
    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "stat_arb_configured"), user.clone()),
        strategy.clone(),
    );
}

pub fn emit_trade_opened(env: &Env, user: &Address, portfolio: &StatArbPortfolio) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "stat_arb_opened"),
            user.clone(),
            portfolio.portfolio_id,
        ),
        portfolio.clone(),
    );
}

pub fn emit_rebalanced(env: &Env, user: &Address, portfolio: &StatArbPortfolio) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "stat_arb_rebalanced"),
            user.clone(),
            portfolio.portfolio_id,
        ),
        portfolio.clone(),
    );
}

pub fn emit_closed(
    env: &Env,
    user: &Address,
    portfolio: &StatArbPortfolio,
    reason: StatArbExitReason,
) {
    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "stat_arb_closed"),
            user.clone(),
            portfolio.portfolio_id,
        ),
        reason,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as TestAddress, Ledger};
    use soroban_sdk::{contract, Address};

    #[contract]
    struct TestContract;

    fn setup_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env
    }

    fn test_user(env: &Env) -> Address {
        Address::generate(env)
    }

    fn history(env: &Env, values: &[i128]) -> Vec<i128> {
        let mut prices = Vec::new(env);
        for value in values {
            prices.push_back(*value);
        }
        prices
    }

    fn seed_histories(env: &Env) {
        set_price_history(env, 1, history(env, &[100, 101, 102, 103, 104, 180])).unwrap();
        set_price_history(env, 2, history(env, &[80, 81, 82, 83, 84, 85])).unwrap();
        set_price_history(env, 3, history(env, &[60, 61, 62, 63, 64, 65])).unwrap();
    }

    #[test]
    fn hedge_ratio_calculation_returns_expected_relative_weights() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());

        env.as_contract(&contract_id, || {
            let mut aligned = Vec::new(&env);
            aligned.push_back(history(&env, &[100, 102, 104, 106]));
            aligned.push_back(history(&env, &[50, 51, 52, 53]));
            aligned.push_back(history(&env, &[25, 26, 27, 28]));

            let hedge_ratios = calculate_hedge_ratios_ols(&env, &aligned).unwrap();
            assert_eq!(hedge_ratios.get(0).unwrap(), STAT_ARB_SCALE);
            assert_eq!(hedge_ratios.get(1).unwrap(), -20_000);
            assert_eq!(hedge_ratios.get(2).unwrap(), -20_000);
        });
    }

    #[test]
    fn portfolio_construction_uses_hedge_ratios() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());

        env.as_contract(&contract_id, || {
            let mut aligned = Vec::new(&env);
            aligned.push_back(history(&env, &[100, 102, 104, 106]));
            aligned.push_back(history(&env, &[50, 51, 52, 53]));
            aligned.push_back(history(&env, &[25, 26, 27, 28]));

            let hedge_ratios = calculate_hedge_ratios_ols(&env, &aligned).unwrap();
            let residuals = construct_portfolio(&env, &aligned, &hedge_ratios).unwrap();
            assert_eq!(residuals.len(), 4);
            assert_eq!(residuals.get(0).unwrap(), -50);
        });
    }

    #[test]
    fn adf_flags_stationary_vs_non_stationary_series() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());

        env.as_contract(&contract_id, || {
            let stationary = history(&env, &[5, -4, 3, -2, 1, -1]);
            let trending = history(&env, &[1, 2, 4, 8, 16, 32]);

            let (stationary_stat, _) = augmented_dickey_fuller_test(&env, &stationary).unwrap();
            let (trending_stat, _) = augmented_dickey_fuller_test(&env, &trending).unwrap();

            assert!(stationary_stat < 0);
            assert!(trending_stat >= stationary_stat);
        });
    }

    #[test]
    fn three_asset_basket_cointegration_path_works() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());

        env.as_contract(&contract_id, || {
            seed_histories(&env);
            let mut basket = Vec::new(&env);
            basket.push_back(1);
            basket.push_back(2);
            basket.push_back(3);

            let result = test_cointegration_for_assets(&env, basket, 6, 1).unwrap();
            assert_eq!(result.asset_group.len(), 3);
            assert!(result.is_cointegrated);
            assert!(result.half_life > 0);
        });
    }

    #[test]
    fn signal_generation_triggers_on_residual_divergence() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());
        let user = test_user(&env);

        env.as_contract(&contract_id, || {
            set_price_history(&env, 1, history(&env, &[100, 101, 102, 103, 104, 180])).unwrap();
            set_price_history(&env, 2, history(&env, &[80, 81, 82, 83, 84, 85])).unwrap();
            set_price_history(&env, 3, history(&env, &[60, 61, 62, 63, 64, 65])).unwrap();

            let mut basket = Vec::new(&env);
            basket.push_back(1);
            basket.push_back(2);
            basket.push_back(3);

            configure_strategy(&env, &user, basket, 6, 1, 500, 250, 1).unwrap();
            let signal = check_stat_arb_signal(&env, &user).unwrap();
            assert_eq!(signal.action, StatArbSignalAction::EnterShort);
        });
    }

    #[test]
    fn non_cointegrated_basket_is_rejected() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());
        let user = test_user(&env);

        env.as_contract(&contract_id, || {
            set_price_history(&env, 1, history(&env, &[100, 110, 120, 130, 140, 150])).unwrap();
            set_price_history(&env, 2, history(&env, &[90, 100, 130, 160, 190, 220])).unwrap();
            set_price_history(&env, 3, history(&env, &[50, 55, 60, 80, 120, 170])).unwrap();

            let mut basket = Vec::new(&env);
            basket.push_back(1);
            basket.push_back(2);
            basket.push_back(3);

            configure_strategy(&env, &user, basket, 6, 5_000, 5_000, 1_000, 1).unwrap();
            let result = test_cointegration(&env, &user).unwrap();
            assert!(!result.is_cointegrated);
        });
    }

    #[test]
    fn zero_variance_and_insufficient_data_are_rejected() {
        let env = setup_env();
        let contract_id = env.register(TestContract, ());

        env.as_contract(&contract_id, || {
            let flat = history(&env, &[100, 100, 100, 100]);
            assert_eq!(
                calculate_regression_coefficient(&flat, &flat),
                Err(AutoTradeError::InvalidPriceData)
            );

            let short = history(&env, &[1, 2, 3]);
            assert_eq!(
                augmented_dickey_fuller_test(&env, &short),
                Err(AutoTradeError::InsufficientPriceHistory)
            );
        });
    }
}
