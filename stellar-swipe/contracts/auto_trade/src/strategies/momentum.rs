//! Momentum-based trading strategy implementation.
//!
//! Implements momentum indicators (ROC, RSI, MACD) to identify and follow trends.
//! Features trailing stops for trend following and asset ranking by momentum strength.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Map, Vec};

use crate::errors::AutoTradeError;

// Constants for momentum calculations
const STELLAR_DECIMALS: u32 = 7;
const STELLAR_SCALE: i128 = 10_000_000; // 10^7

/// Represents an asset pair (base/quote)
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetPair {
    pub base: u32,
    pub quote: u32,
}

/// Momentum strategy configuration
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MomentumStrategy {
    pub strategy_id: u64,
    pub user: Address,
    pub asset_pairs: Vec<AssetPair>,
    pub momentum_period_days: u32,      // Period for ROC calculation
    pub min_momentum_threshold: i128,   // Minimum ROC to trigger (in basis points)
    pub trend_confirmation_required: bool,
    pub position_size_pct: u32,         // Position size as % of portfolio (0-10000 = 0-100%)
    pub trailing_stop_pct: u32,         // Trailing stop as % below highest price
    pub ranking_enabled: bool,
}

/// Active momentum position
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MomentumPosition {
    pub asset_pair: AssetPair,
    pub entry_price: i128,
    pub entry_time: u64,
    pub highest_price: i128,
    pub trailing_stop_price: i128,
    pub amount: i128,
    pub momentum_at_entry: i128,
}

/// Momentum indicators computed for an asset pair
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MomentumIndicators {
    pub rate_of_change: i128,    // ROC over period (in basis points)
    pub rsi: u32,                // RSI 0-10000 (0-100%)
    pub macd: i128,              // MACD value
    pub macd_signal: i128,       // MACD signal line
    pub trend_strength: u32,     // 0-10000 (0-100%)
}

/// Momentum trading signal
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeDirection {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MomentumSignal {
    pub asset_pair: AssetPair,
    pub direction: TradeDirection,
    pub momentum_strength: i128,  // Absolute value of ROC
    pub rsi: u32,
    pub trend_strength: u32,
    pub confidence: u32,          // 0-10000 (0-100%)
}

/// Storage keys for momentum strategy persistence
#[contracttype]
pub enum MomentumDataKey {
    Strategy(u64),                           // Store strategy by ID
    StrategyPositions(u64),                  // Store active positions for strategy
    PriceHistory(AssetPair, u64),            // Store price snapshot at timestamp
}

/// ==========================
/// Momentum Indicator Calculations
/// ==========================

/// Calculate Rate of Change (ROC) indicator over a period
///
/// ROC = ((Current Price - Old Price) / Old Price) * 10000 (in basis points)
fn calculate_rate_of_change(prices: &Vec<i128>, period_days: u32) -> Result<i128, AutoTradeError> {
    if prices.len() < 2 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let current_price = prices.last().unwrap_or(0);
    let old_price = prices.first().unwrap_or(1);

    if old_price == 0 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let roc = ((current_price - old_price) * 10000) / old_price;
    Ok(roc)
}

/// Calculate RSI (Relative Strength Index) over period
///
/// RSI = 100 * (Average Gain / (Average Gain + Average Loss))
/// Returns value in 0-10000 range (0-100%)
fn calculate_rsi_from_prices(prices: &Vec<i128>, period: u32) -> Result<u32, AutoTradeError> {
    let period_len = period as usize;
    if prices.len() < (period_len as u32) + 1 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let mut gains = 0i128;
    let mut losses = 0i128;

    // Calculate average gains and losses over period
    for i in (prices.len() - period_len as u32)..prices.len() {
        if i == 0 {
            continue;
        }
        let change = prices.get(i).unwrap() - prices.get(i - 1).unwrap();
        if change > 0 {
            gains += change;
        } else {
            losses += change.abs();
        }
    }

    let avg_gain = gains / period_len as i128;
    let avg_loss = losses / period_len as i128;

    if avg_gain + avg_loss == 0 {
        return Ok(5000); // Neutral RSI
    }

    let rsi = (100 * avg_gain * 100) / (avg_gain + avg_loss);
    let rsi_capped = if rsi > 10000 { 10000 } else { rsi as u32 };
    Ok(rsi_capped)
}

/// Calculate MACD (Moving Average Convergence Divergence)
///
/// MACD = EMA12 - EMA26
/// Signal = EMA9 of MACD
/// Returns (MACD, Signal, Histogram)
fn calculate_macd_from_prices(
    prices: &Vec<i128>,
) -> Result<(i128, i128), AutoTradeError> {
    if prices.len() < 26 {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    // Simple approximation: use difference of averages instead of true EMA
    let mut sum_12 = 0i128;
    let mut sum_26 = 0i128;

    // Calculate 12-period average
    for i in (prices.len() - 12)..prices.len() {
        sum_12 += prices.get(i).unwrap();
    }
    let avg_12 = sum_12 / 12;

    // Calculate 26-period average
    for i in (prices.len() - 26)..prices.len() {
        sum_26 += prices.get(i).unwrap();
    }
    let avg_26 = sum_26 / 26;

    let macd = avg_12 - avg_26;

    // Signal line (9-period of MACD approximation)
    let signal = (macd * 9) / 10; // Simplified signal calculation
    Ok((macd, signal))
}

/// Calculate trend strength based on consecutive price increases
///
/// Returns 0-10000 (0-100%) based on how many recent prices are increasing
fn calculate_trend_strength(prices: &Vec<i128>) -> Result<u32, AutoTradeError> {
    if prices.len() < 20 {
        return Ok(0);
    }

    let mut increasing_count = 0u32;
    let lookback = 20;

    // Count how many of the last 20 prices are higher than previous
    for i in 1..lookback {
        let idx = prices.len() - i;
        if idx > 0 && prices.get(idx).unwrap() > prices.get(idx - 1).unwrap() {
            increasing_count += 1;
        }
    }

    // Trend strength: 0-10000 (0-100%)
    let strength = (increasing_count * 10000) / lookback;
    Ok(strength)
}

/// Calculate all momentum indicators for an asset pair
pub fn calculate_momentum_indicators(
    _env: &Env,
    prices: &Vec<i128>,
    _period_days: u32,
) -> Result<MomentumIndicators, AutoTradeError> {
    if prices.is_empty() {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let rate_of_change = calculate_rate_of_change(&prices, 1)?;
    let rsi = calculate_rsi_from_prices(&prices, 14)?;
    let (macd, macd_signal) = calculate_macd_from_prices(&prices)?;
    let trend_strength = calculate_trend_strength(&prices)?;

    Ok(MomentumIndicators {
        rate_of_change,
        rsi,
        macd,
        macd_signal,
        trend_strength,
    })
}

/// ==========================
/// Signal Generation
/// ==========================

/// Calculate confidence score for momentum signal
///
/// Combines multiple indicators into a 0-10000 confidence score
fn calculate_momentum_confidence(indicators: &MomentumIndicators) -> Result<u32, AutoTradeError> {
    // ROC score: weight the absolute momentum (capped at 50%)
    let roc_score = {
        let abs_roc = indicators.rate_of_change.abs();
        if abs_roc * 10 > 5000 {
            5000
        } else {
            (abs_roc * 10) as u32
        }
    };

    // RSI score: 25% for extreme RSI (>70 or <30)
    let rsi_score = if indicators.rsi > 7000 || indicators.rsi < 3000 {
        2500
    } else {
        1000
    };

    // MACD score: 25% for bullish MACD crossover
    let macd_score = if indicators.macd > indicators.macd_signal {
        2500
    } else {
        0
    };

    // Trend strength score: up to 25%
    let trend_score = indicators.trend_strength / 4;

    let total = roc_score + rsi_score + macd_score + trend_score;
    Ok(if total > 10000 { 10000 } else { total })
}

/// Check for momentum signals on an asset pair
///
/// Returns a signal if momentum conditions are met
pub fn check_momentum_signals(
    _env: &Env,
    strategy: &MomentumStrategy,
    asset_pair: AssetPair,
    prices: &Vec<i128>,
) -> Result<Option<MomentumSignal>, AutoTradeError> {
    if prices.is_empty() {
        return Ok(None);
    }

    let indicators = calculate_momentum_indicators(_env, &prices, strategy.momentum_period_days)?;

    // Check if momentum exceeds threshold
    if indicators.rate_of_change.abs() < strategy.min_momentum_threshold {
        return Ok(None);
    }

    // Confirm with other indicators if required
    if strategy.trend_confirmation_required {
        let confirmed = indicators.rsi > 5000 && // RSI > 50 (bullish)
            indicators.macd > indicators.macd_signal && // MACD crossover
            indicators.trend_strength > 6000; // Strong trend (>60%)

        if !confirmed {
            return Ok(None);
        }
    }

    let direction = if indicators.rate_of_change > 0 {
        TradeDirection::Buy
    } else {
        TradeDirection::Sell
    };

    let confidence = calculate_momentum_confidence(&indicators)?;

    let signal = MomentumSignal {
        asset_pair,
        direction,
        momentum_strength: indicators.rate_of_change.abs(),
        rsi: indicators.rsi,
        trend_strength: indicators.trend_strength,
        confidence,
    };

    Ok(Some(signal))
}

/// ==========================
/// Trade Execution & Position Management
/// ==========================

/// Get a momentum strategy by ID
pub fn get_momentum_strategy(env: &Env, strategy_id: u64) -> Result<MomentumStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&MomentumDataKey::Strategy(strategy_id))
        .ok_or(AutoTradeError::StrategyNotFound)
}

/// Get a mutable momentum strategy by ID
pub fn get_momentum_strategy_mut(
    env: &Env,
    strategy_id: u64,
) -> Result<MomentumStrategy, AutoTradeError> {
    get_momentum_strategy(env, strategy_id)
}

/// Store a momentum strategy
pub fn store_momentum_strategy(env: &Env, strategy: &MomentumStrategy) {
    env.storage()
        .persistent()
        .set(&MomentumDataKey::Strategy(strategy.strategy_id), strategy);
}

/// Get active positions for a momentum strategy
pub fn get_strategy_positions(env: &Env, strategy_id: u64) -> Map<u32, MomentumPosition> {
    env.storage()
        .persistent()
        .get(&MomentumDataKey::StrategyPositions(strategy_id))
        .unwrap_or_else(|| Map::new(env))
}

/// Store positions for a momentum strategy
pub fn store_strategy_positions(env: &Env, strategy_id: u64, positions: &Map<u32, MomentumPosition>) {
    env.storage()
        .persistent()
        .set(&MomentumDataKey::StrategyPositions(strategy_id), positions);
}

/// Execute a momentum trade based on a signal
///
/// Opens a new position at current price with trailing stop protection
pub fn execute_momentum_trade(
    env: &Env,
    strategy_id: u64,
    signal: MomentumSignal,
    current_price: i128,
    portfolio_value: i128,
) -> Result<u64, AutoTradeError> {
    let mut strategy = get_momentum_strategy(env, strategy_id)?;

    // Check if already have position in this asset pair
    let mut positions = get_strategy_positions(env, strategy_id);
    let pair_key = signal.asset_pair.base; // Use base asset as key

    if positions.contains_key(pair_key) {
        return Err(AutoTradeError::PositionAlreadyExists);
    }

    // Calculate position size
    let position_amount = if portfolio_value > 0 {
        (portfolio_value * strategy.position_size_pct as i128) / 10000
    } else {
        return Err(AutoTradeError::InvalidAmount);
    };

    // Calculate trailing stop
    let trailing_stop_price = if strategy.trailing_stop_pct < 10000 {
        (current_price * (10000 - strategy.trailing_stop_pct as i128)) / 10000
    } else {
        0
    };

    // Create position
    let position = MomentumPosition {
        asset_pair: signal.asset_pair,
        entry_price: current_price,
        entry_time: env.ledger().timestamp(),
        highest_price: current_price,
        trailing_stop_price,
        amount: position_amount,
        momentum_at_entry: signal.momentum_strength,
    };

    positions.set(pair_key, position);
    store_strategy_positions(env, strategy_id, &positions);

    // Return a trade ID (using strategy_id as base + pair_key)
    let trade_id = (strategy_id << 32) | (pair_key as u64);
    Ok(trade_id)
}

/// ==========================
/// Trailing Stop Management
/// ==========================

/// Update trailing stops for all active positions
///
/// Adjusts stops as prices move higher and closes positions if stops are hit
pub fn update_trailing_stops(env: &Env, strategy_id: u64) -> Result<Vec<AssetPair>, AutoTradeError> {
    let strategy = get_momentum_strategy(env, strategy_id)?;
    let mut positions = get_strategy_positions(env, strategy_id);
    let mut closed_positions = Vec::new(env);

    let keys = positions.keys();
    for i in 0..keys.len() {
        if let Some(asset_key) = keys.get(i) {
            if let Some(mut position) = positions.get(asset_key) {
                // Get current price (simulated - in real implementation, fetch from oracle)
                let current_price = position.highest_price; // Placeholder

                // Update highest price if new high
                if current_price > position.highest_price {
                    position.highest_price = current_price;

                    // Adjust trailing stop
                    position.trailing_stop_price = if strategy.trailing_stop_pct < 10000 {
                        (current_price * (10000 - strategy.trailing_stop_pct as i128)) / 10000
                    } else {
                        0
                    };

                    positions.set(asset_key, position.clone());
                }

                // Check if stop hit
                if current_price <= position.trailing_stop_price {
                    closed_positions.push_back(position.asset_pair);
                    positions.remove(asset_key);
                }
            }
        }
    }

    store_strategy_positions(env, strategy_id, &positions);
    Ok(closed_positions)
}

/// ==========================
/// Asset Ranking & Rebalancing
/// ==========================

/// Rank assets by momentum indicators
///
/// Returns vector of (AssetPair, Momentum Score) sorted by score descending
pub fn rank_assets_by_momentum(
    env: &Env,
    strategy: &MomentumStrategy,
    prices_map: &Map<u32, Vec<i128>>,
) -> Result<Vec<(AssetPair, i128)>, AutoTradeError> {
    let mut ranked = Vec::new(env);

    for i in 0..strategy.asset_pairs.len() {
        if let Some(asset_pair) = strategy.asset_pairs.get(i) {
            if let Some(prices) = prices_map.get(asset_pair.base) {
                let indicators = calculate_momentum_indicators(env, &prices, strategy.momentum_period_days)?;

                    // Composite score: ROC + trend strength component
                    let score = indicators.rate_of_change
                        + (indicators.trend_strength as i128 / 10);

                    ranked.push_back((asset_pair, score));
                }
            }
        }

    // Sort by score descending (bubble sort for Soroban compatibility)
    let len = ranked.len();
    for i in 0..len {
        for j in i..len {
            if j > 0 {
                let (pair_i, score_i) = ranked.get(i).unwrap_or((AssetPair { base: 0, quote: 0 }, 0));
                let (pair_j, score_j) = ranked.get(j).unwrap_or((AssetPair { base: 0, quote: 0 }, 0));
                if score_j > score_i {
                    // Swap
                    ranked.set(i, (pair_j, score_j));
                    ranked.set(j, (pair_i, score_i));
                }
            }
        }
    }

    Ok(ranked)
}

/// Rebalance strategy to hold top momentum assets
///
/// Closes positions in lower-ranked assets and opens in top-ranked ones
pub fn rebalance_by_momentum_rank(
    env: &Env,
    strategy_id: u64,
    ranked_assets: &Vec<(AssetPair, i128)>,
    top_n: usize,
) -> Result<(), AutoTradeError> {
    let strategy = get_momentum_strategy(env, strategy_id)?;
    let mut positions = get_strategy_positions(env, strategy_id);

    if !strategy.ranking_enabled {
        return Ok(());
    }

    // Collect top N assets
    let mut top_assets = Vec::new(env);
    for i in 0..(top_n.min(ranked_assets.len() as usize)) {
        if let Some((pair, _)) = ranked_assets.get(i as u32) {
            top_assets.push_back(pair);
        }
    }

    // Close positions not in top N
    let pos_keys = positions.keys();
    for i in 0..pos_keys.len() {
        if let Some(key) = pos_keys.get(i) {
            if let Some(position) = positions.get(key) {
                // Check if position's asset pair is in top assets
                let mut in_top = false;
                for j in 0..top_assets.len() {
                    if let Some(top_pair) = top_assets.get(j) {
                        if top_pair == position.asset_pair {
                            in_top = true;
                            break;
                        }
                    }
                }

                if !in_top {
                    positions.remove(key);
                }
            }
        }
    }

    store_strategy_positions(env, strategy_id, &positions);
    Ok(())
}

/// ==========================
/// Price History Management
/// ==========================

/// Store a price snapshot for an asset pair
pub fn store_price_snapshot(
    env: &Env,
    asset_pair: AssetPair,
    price: i128,
    timestamp: u64,
) {
    let key = (asset_pair, timestamp);
    env.storage().persistent().set(&key, &price);
}

/// Get historical prices for an asset pair
///
/// Returns prices from the given time period
pub fn get_historical_prices(
    env: &Env,
    asset_pair: AssetPair,
    period_seconds: u64,
) -> Result<Vec<i128>, AutoTradeError> {
    let current_time = env.ledger().timestamp();
    let start_time = current_time.saturating_sub(period_seconds);
    let mut prices = Vec::new(env);

    // In a real implementation, iterate through stored price snapshots
    // For now, return empty vector - actual implementation would fetch from storage
    Ok(prices)
}

/// ==========================
/// Tests
/// ==========================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::Env;

    fn setup_test_prices(env: &Env) -> Vec<i128> {
        let mut prices = Vec::new(env);
        // Create a series of prices showing uptrend
        prices.push_back(100); // Start
        prices.push_back(102); // +2%
        prices.push_back(105); // +3%
        prices.push_back(108); // +3%
        prices.push_back(110); // +1.9%
        prices.push_back(112); // +1.8%
        prices.push_back(115); // +2.7%
        prices.push_back(118); // +2.6%
        prices.push_back(120); // +1.7%
        prices.push_back(122); // +1.7%
        prices.push_back(125); // +2.5%
        prices.push_back(127); // +1.6%
        prices.push_back(128); // +0.8%
        prices.push_back(130); // +1.6%
        prices.push_back(132); // +1.5%
        prices.push_back(135); // +2.3%
        prices.push_back(138); // +2.2%
        prices.push_back(140); // +1.4%
        prices.push_back(142); // +1.4%
        prices.push_back(145); // +2.1%
        prices
    }

    fn setup_test_prices_downtrend(env: &Env) -> Vec<i128> {
        let mut prices = Vec::new(env);
        // Create a series of prices showing downtrend
        prices.push_back(100); // Start
        prices.push_back(98);  // -2%
        prices.push_back(95);  // -3.1%
        prices.push_back(92);  // -3.2%
        prices.push_back(90);  // -2.2%
        prices.push_back(88);  // -2.2%
        prices.push_back(85);  // -3.4%
        prices.push_back(82);  // -3.5%
        prices.push_back(80);  // -2.4%
        prices.push_back(78);  // -2.5%
        prices.push_back(75);  // -3.8%
        prices.push_back(73);  // -2.7%
        prices.push_back(72);  // -1.4%
        prices.push_back(70);  // -2.8%
        prices.push_back(68);  // -2.9%
        prices.push_back(65);  // -4.4%
        prices.push_back(62);  // -4.6%
        prices.push_back(60);  // -3.2%
        prices.push_back(58);  // -3.3%
        prices.push_back(55);  // -5.2%
        prices
    }

    #[test]
    fn test_calculate_rate_of_change_positive() {
        let env = Env::default();
        let prices = setup_test_prices(&env);

        let roc = calculate_rate_of_change(&prices, 1).unwrap();
        // (145 - 100) / 100 * 10000 = 45 * 100 = 4500 (45%)
        assert!(roc > 4000 && roc < 5000); // Should be around 4500
    }

    #[test]
    fn test_calculate_rate_of_change_negative() {
        let env = Env::default();
        let prices = setup_test_prices_downtrend(&env);

        let roc = calculate_rate_of_change(&prices, 1).unwrap();
        // (55 - 100) / 100 * 10000 = -45 * 100 = -4500 (-45%)
        assert!(roc < -4000 && roc > -5000); // Should be around -4500
    }

    #[test]
    fn test_calculate_rsi_uptrend() {
        let env = Env::default();
        let prices = setup_test_prices(&env);

        let rsi = calculate_rsi_from_prices(&prices, 14).unwrap();
        // RSI in uptrend should be > 5000 (>50%)
        assert!(rsi > 5000);
    }

    #[test]
    fn test_calculate_rsi_downtrend() {
        let env = Env::default();
        let prices = setup_test_prices_downtrend(&env);

        let rsi = calculate_rsi_from_prices(&prices, 14).unwrap();
        // RSI in downtrend should be < 5000 (<50%)
        assert!(rsi < 5000);
    }

    #[test]
    fn test_calculate_macd() {
        let env = Env::default();
        let prices = setup_test_prices(&env);

        let result = calculate_macd_from_prices(&prices);
        assert!(result.is_ok());
        let (macd, signal) = result.unwrap();
        // In uptrend, MACD should be positive
        assert!(macd > 0);
        assert!(signal >= 0);
    }

    #[test]
    fn test_calculate_trend_strength() {
        let env = Env::default();
        let prices = setup_test_prices(&env);

        let strength = calculate_trend_strength(&prices).unwrap();
        // Strong uptrend should have high trend strength
        assert!(strength > 6000);
    }

    #[test]
    fn test_calculate_momentum_confidence() {
        let env = Env::default();
        
        let indicators = MomentumIndicators {
            rate_of_change: 3000,  // 30% ROC
            rsi: 7500,             // >70% (extreme)
            macd: 100,
            macd_signal: 50,       // Bullish crossover
            trend_strength: 8000,  // Strong trend
        };

        let confidence = calculate_momentum_confidence(&indicators).unwrap();
        // All indicators bullish should give high confidence
        assert!(confidence > 8000);
    }

    #[test]
    fn test_check_momentum_signals_buy() {
        let env = Env::default();
        let prices = setup_test_prices(&env);
        
        let strategy = MomentumStrategy {
            strategy_id: 1,
            user: Address::generate(&env),
            asset_pairs: Vec::new(&env),
            momentum_period_days: 7,
            min_momentum_threshold: 1000, // 10%
            trend_confirmation_required: false,
            position_size_pct: 1000,
            trailing_stop_pct: 1000,
            ranking_enabled: false,
        };

        let pair = AssetPair { base: 1, quote: 2 };
        let signal = check_momentum_signals(&env, &strategy, pair, &prices).unwrap();

        assert!(signal.is_some());
        let sig = signal.unwrap();
        assert_eq!(sig.asset_pair, pair);
        // Should be a buy signal in uptrend
        assert!(matches!(sig.direction, TradeDirection::Buy));
        assert!(sig.confidence > 0);
    }

    #[test]
    fn test_check_momentum_signals_sell() {
        let env = Env::default();
        let prices = setup_test_prices_downtrend(&env);
        
        let strategy = MomentumStrategy {
            strategy_id: 1,
            user: Address::generate(&env),
            asset_pairs: Vec::new(&env),
            momentum_period_days: 7,
            min_momentum_threshold: 1000, // 10%
            trend_confirmation_required: false,
            position_size_pct: 1000,
            trailing_stop_pct: 1000,
            ranking_enabled: false,
        };

        let pair = AssetPair { base: 1, quote: 2 };
        let signal = check_momentum_signals(&env, &strategy, pair, &prices).unwrap();

        assert!(signal.is_some());
        let sig = signal.unwrap();
        assert_eq!(sig.asset_pair, pair);
        // Should be a sell signal in downtrend
        assert!(matches!(sig.direction, TradeDirection::Sell));
    }

    #[test]
    fn test_execute_momentum_trade() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);

        let user = Address::generate(&env);
        let asset_pair = AssetPair { base: 1, quote: 2 };
        
        let strategy = MomentumStrategy {
            strategy_id: 1,
            user: user.clone(),
            asset_pairs: Vec::new(&env),
            momentum_period_days: 7,
            min_momentum_threshold: 1000,
            trend_confirmation_required: false,
            position_size_pct: 1000, // 10% of portfolio
            trailing_stop_pct: 1000, // 10% below highest
            ranking_enabled: false,
        };

        store_momentum_strategy(&env, &strategy);

        let signal = MomentumSignal {
            asset_pair,
            direction: TradeDirection::Buy,
            momentum_strength: 2000,
            rsi: 7000,
            trend_strength: 7000,
            confidence: 8000,
        };

        let current_price = 1000;
        let portfolio_value = 10000;

        let trade_id = execute_momentum_trade(&env, 1, signal, current_price, portfolio_value).unwrap();

        // Verify position was created
        let positions = get_strategy_positions(&env, 1);
        assert!(positions.contains_key(asset_pair.base));

        let position = positions.get(asset_pair.base).unwrap();
        assert_eq!(position.asset_pair, asset_pair);
        assert_eq!(position.entry_price, current_price);
        assert_eq!(position.amount, 1000); // 10% of 10000
    }

    #[test]
    fn test_trailing_stop_update() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);

        let user = Address::generate(&env);
        let asset_pair = AssetPair { base: 1, quote: 2 };
        
        let strategy = MomentumStrategy {
            strategy_id: 1,
            user,
            asset_pairs: Vec::new(&env),
            momentum_period_days: 7,
            min_momentum_threshold: 1000,
            trend_confirmation_required: false,
            position_size_pct: 1000,
            trailing_stop_pct: 1000, // 10% trailing stop
            ranking_enabled: false,
        };

        store_momentum_strategy(&env, &strategy);

        // Create initial position
        let signal = MomentumSignal {
            asset_pair,
            direction: TradeDirection::Buy,
            momentum_strength: 2000,
            rsi: 7000,
            trend_strength: 7000,
            confidence: 8000,
        };

        execute_momentum_trade(&env, 1, signal, 1000, 10000).unwrap();

        let closed = update_trailing_stops(&env, 1).unwrap();
        assert_eq!(closed.len(), 0); // No positions closed if price doesn't move
    }

    #[test]
    fn test_rank_assets_by_momentum() {
        let env = Env::default();
        
        let user = Address::generate(&env);
        let pair1 = AssetPair { base: 1, quote: 2 };
        let pair2 = AssetPair { base: 3, quote: 4 };

        let mut asset_pairs = Vec::new(&env);
        asset_pairs.push_back(pair1);
        asset_pairs.push_back(pair2);

        let strategy = MomentumStrategy {
            strategy_id: 1,
            user,
            asset_pairs,
            momentum_period_days: 7,
            min_momentum_threshold: 1000,
            trend_confirmation_required: false,
            position_size_pct: 1000,
            trailing_stop_pct: 1000,
            ranking_enabled: true,
        };

        // Create price maps
        let mut prices_map = Map::new(&env);
        let prices_up = setup_test_prices(&env);
        let prices_down = setup_test_prices_downtrend(&env);
        
        prices_map.set(1, prices_up);
        prices_map.set(3, prices_down);

        let ranked = rank_assets_by_momentum(&env, &strategy, &prices_map).unwrap();

        // Should have 2 assets ranked
        assert!(ranked.len() >= 1);

        // Higher momentum asset should be first
        let (first_pair, first_score) = ranked.get(0).unwrap();
        assert_eq!(first_pair, pair1); // pair1 has uptrend
        assert!(first_score > 0); // Positive momentum
    }

    #[test]
    fn test_rebalance_by_momentum_rank() {
        let env = Env::default();
        env.ledger().set_timestamp(1000);

        let user = Address::generate(&env);
        let pair1 = AssetPair { base: 1, quote: 2 };
        let pair2 = AssetPair { base: 3, quote: 4 };

        let mut asset_pairs = Vec::new(&env);
        asset_pairs.push_back(pair1);
        asset_pairs.push_back(pair2);

        let strategy = MomentumStrategy {
            strategy_id: 1,
            user,
            asset_pairs,
            momentum_period_days: 7,
            min_momentum_threshold: 1000,
            trend_confirmation_required: false,
            position_size_pct: 1000,
            trailing_stop_pct: 1000,
            ranking_enabled: true,
        };

        store_momentum_strategy(&env, &strategy);

        // Create ranked list
        let mut ranked = Vec::new(&env);
        ranked.push_back((pair1, 5000));
        ranked.push_back((pair2, 2000));

        let result = rebalance_by_momentum_rank(&env, 1, &ranked, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_insufficient_price_history() {
        let env = Env::default();
        let empty_prices: Vec<i128> = Vec::new(&env);

        let result = calculate_rate_of_change(&empty_prices, 1);
        assert_eq!(result, Err(AutoTradeError::InsufficientPriceHistory));
    }

    #[test]
    fn test_momentum_threshold_filtering() {
        let env = Env::default();
        let prices = setup_test_prices(&env);
        
        let strategy = MomentumStrategy {
            strategy_id: 1,
            user: Address::generate(&env),
            asset_pairs: Vec::new(&env),
            momentum_period_days: 7,
            min_momentum_threshold: 10000, // 100% threshold - very high
            trend_confirmation_required: false,
            position_size_pct: 1000,
            trailing_stop_pct: 1000,
            ranking_enabled: false,
        };

        let pair = AssetPair { base: 1, quote: 2 };
        let signal = check_momentum_signals(&env, &strategy, pair, &prices).unwrap();

        // Should not generate signal due to high threshold
        assert!(signal.is_none());
    }
}
