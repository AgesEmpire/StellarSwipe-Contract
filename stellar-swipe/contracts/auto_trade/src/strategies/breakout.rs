//! Breakout Trading Strategy
//!
//! Identifies support/resistance levels and trades confirmed breakouts with volume verification.
//! Features:
//! - Automatic support/resistance level detection
//! - Volume-confirmed breakout detection
//! - False breakout detection and handling
//! - Dynamic level updates after successful breakouts
//! - Performance analytics and win-rate tracking

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::errors::AutoTradeError;
use crate::iceberg::AssetPair;

// Constants
const PRICE_LEVEL_TOLERANCE_BPS: i128 = 20; // 0.2% tolerance for level clustering
const MIN_CANDLE_HISTORY: usize = 20;
const MAX_KEY_LEVELS: usize = 10;
const DEFAULT_LOOKBACK_SECONDS: u64 = 86400; // 1 day
const CANDLE_DURATION_SECONDS: u64 = 3600;  // 1 hour candles

/// Represents a price candle (OHLCV)
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candle {
    pub timestamp: u64,
    pub open: i128,
    pub high: i128,
    pub low: i128,
    pub close: i128,
    pub volume: i128,
}

/// Types of price levels
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelType {
    Resistance,
    Support,
    Pivot,
}

/// Represents a detected support/resistance level
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PriceLevel {
    pub price: i128,
    pub level_type: LevelType,
    pub strength: u32,        // How many times price tested this level
    pub last_test: u64,       // Timestamp of last test
}

/// Breakout direction
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakDirection {
    Upward,   // Broke above resistance
    Downward, // Broke below support
}

/// Position status
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionStatus {
    Open,
    Closed,
}

/// Active breakout position
#[contracttype]
#[derive(Clone, Debug)]
pub struct BreakoutPosition {
    pub position_id: u64,
    pub direction: BreakDirection,
    pub breakout_level: i128,
    pub entry_price: i128,
    pub entry_volume: i128,        // Volume ratio when entered
    pub stop_loss: i128,
    pub target_price: i128,
    pub amount: i128,
    pub status: PositionStatus,
    pub entry_time: u64,
}

/// Breakout trading signal
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakoutSignal {
    pub direction: BreakDirection,
    pub breakout_level: i128,
    pub current_price: i128,
    pub target_price: i128,
    pub stop_loss: i128,
    pub level_strength: u32,
    pub volume_ratio: u32,        // Current volume as % of average
    pub confidence: u32,          // 0-10000 (0-100%)
}

/// Breakout strategy configuration
#[contracttype]
#[derive(Clone, Debug)]
pub struct BreakoutStrategy {
    pub strategy_id: u64,
    pub user: Address,
    pub asset_pair: AssetPair,
    pub lookback_period_days: u32,
    pub volume_multiplier: u32,   // e.g., 150 = 1.5x average volume required
    pub confirmation_candles: u32, // Number of candles to confirm breakout
    pub position_size_pct: u32,   // Position size as % of portfolio (0-10000 = 0-100%)
    pub active_positions: Vec<BreakoutPosition>,
    pub key_levels: Vec<PriceLevel>,
}

/// Performance metrics for breakout strategy
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakoutPerformance {
    pub total_breakouts_detected: u32,
    pub total_trades_executed: u32,
    pub successful_breakouts: u32, // Reached target
    pub false_breakouts: u32,
    pub stopped_out: u32,
    pub avg_breakout_profit_pct: i32,
    pub avg_volume_on_breakout: u32,
}

/// Storage keys for breakout strategy
#[contracttype]
pub enum BreakoutDataKey {
    Strategy(u64),
    StrategyPositions(u64),
    KeyLevels(u64),
    PriceHistory(AssetPair, u64),
}

/// ==========================
/// Support/Resistance Detection
/// ==========================

/// Identify key support and resistance levels from historical data
pub fn identify_key_levels(
    asset_pair: AssetPair,
    lookback_days: u32,
) -> Result<Vec<PriceLevel>, AutoTradeError> {
    let lookback_seconds = lookback_days as u64 * DEFAULT_LOOKBACK_SECONDS;
    let candles = get_historical_candles(asset_pair, lookback_seconds, CANDLE_DURATION_SECONDS)?;

    if candles.len() < MIN_CANDLE_HISTORY {
        return Err(AutoTradeError::InsufficientPriceHistory);
    }

    let mut levels: Vec<PriceLevel> = Vec::new();

    // Find local highs (resistance)
    for i in 2..candles.len() - 2 {
        let current_high = candles.get(i).unwrap().high;
        let prev1_high = candles.get(i - 1).unwrap().high;
        let prev2_high = candles.get(i - 2).unwrap().high;
        let next1_high = candles.get(i + 1).unwrap().high;
        let next2_high = candles.get(i + 2).unwrap().high;

        if current_high > prev1_high
            && current_high > prev2_high
            && current_high > next1_high
            && current_high > next2_high
        {
            let tolerance = (current_high * PRICE_LEVEL_TOLERANCE_BPS) / 10000;

            // Check if we already have a level near this price
            let mut found = false;
            for level in levels.iter_mut() {
                if (level.price - current_high).abs() < tolerance {
                    level.strength += 1;
                    level.last_test = candles.get(i).unwrap().timestamp;
                    found = true;
                    break;
                }
            }

            if !found {
                levels.push_back(PriceLevel {
                    price: current_high,
                    level_type: LevelType::Resistance,
                    strength: 1,
                    last_test: candles.get(i).unwrap().timestamp,
                });
            }
        }
    }

    // Find local lows (support)
    for i in 2..candles.len() - 2 {
        let current_low = candles.get(i).unwrap().low;
        let prev1_low = candles.get(i - 1).unwrap().low;
        let prev2_low = candles.get(i - 2).unwrap().low;
        let next1_low = candles.get(i + 1).unwrap().low;
        let next2_low = candles.get(i + 2).unwrap().low;

        if current_low < prev1_low
            && current_low < prev2_low
            && current_low < next1_low
            && current_low < next2_low
        {
            let tolerance = (current_low * PRICE_LEVEL_TOLERANCE_BPS) / 10000;

            let mut found = false;
            for level in levels.iter_mut() {
                if (level.price - current_low).abs() < tolerance {
                    level.strength += 1;
                    level.last_test = candles.get(i).unwrap().timestamp;
                    found = true;
                    break;
                }
            }

            if !found {
                levels.push_back(PriceLevel {
                    price: current_low,
                    level_type: LevelType::Support,
                    strength: 1,
                    last_test: candles.get(i).unwrap().timestamp,
                });
            }
        }
    }

    // Sort by strength (strongest first)
    // Manual sort since Vec doesn't have a sort method
    for i in 0..levels.len() {
        for j in (i + 1)..levels.len() {
            let level_i = levels.get(i).unwrap().strength;
            let level_j = levels.get(j).unwrap().strength;
            if level_j > level_i {
                let temp = levels.get(i).unwrap().clone();
                let _ = levels.set(i, levels.get(j).unwrap().clone());
                let _ = levels.set(j, temp);
            }
        }
    }

    // Keep top 10 strongest levels
    while levels.len() > MAX_KEY_LEVELS {
        let _ = levels.pop_back();
    }

    Ok(levels)
}

/// ==========================
/// Breakout Detection
/// ==========================

/// Detect if a breakout is occurring with volume confirmation
pub fn detect_breakout(strategy_id: u64) -> Result<Option<BreakoutSignal>, AutoTradeError> {
    let strategy = get_breakout_strategy(strategy_id)?;

    // Update key levels
    let key_levels = identify_key_levels(strategy.asset_pair, strategy.lookback_period_days)?;

    let current_candle = get_latest_candle(strategy.asset_pair, CANDLE_DURATION_SECONDS)?;
    let current_price = current_candle.close;
    let current_volume = current_candle.volume;

    // Calculate average volume
    let avg_volume = calculate_average_volume(strategy.asset_pair, strategy.lookback_period_days)?;

    // Check for breakouts on each key level
    for i in 0..key_levels.len() {
        let level = key_levels.get(i).unwrap();

        match level.level_type {
            LevelType::Resistance => {
                // Check if price broke above resistance
                if current_price > level.price {
                    let volume_ratio = if avg_volume > 0 {
                        ((current_volume * 100) / avg_volume) as u32
                    } else {
                        100
                    };

                    if volume_ratio >= strategy.volume_multiplier {
                        // Check for confirmation
                        if is_breakout_confirmed(
                            strategy.asset_pair,
                            level.price,
                            BreakDirection::Upward,
                            strategy.confirmation_candles,
                        )? {
                            return Ok(Some(create_breakout_signal(
                                level,
                                BreakDirection::Upward,
                                current_price,
                                current_volume,
                                avg_volume,
                            )?));
                        }
                    }
                }
            }
            LevelType::Support => {
                // Check if price broke below support
                if current_price < level.price {
                    let volume_ratio = if avg_volume > 0 {
                        ((current_volume * 100) / avg_volume) as u32
                    } else {
                        100
                    };

                    if volume_ratio >= strategy.volume_multiplier {
                        if is_breakout_confirmed(
                            strategy.asset_pair,
                            level.price,
                            BreakDirection::Downward,
                            strategy.confirmation_candles,
                        )? {
                            return Ok(Some(create_breakout_signal(
                                level,
                                BreakDirection::Downward,
                                current_price,
                                current_volume,
                                avg_volume,
                            )?));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(None)
}

/// Verify that breakout is confirmed by checking recent candles stayed on breakout side
fn is_breakout_confirmed(
    asset_pair: AssetPair,
    level_price: i128,
    direction: BreakDirection,
    confirmation_candles: u32,
) -> Result<bool, AutoTradeError> {
    let recent_candles =
        get_recent_candles(asset_pair, confirmation_candles as usize, CANDLE_DURATION_SECONDS)?;

    for i in 0..recent_candles.len() {
        let candle = recent_candles.get(i).unwrap();
        let stayed_above = match direction {
            BreakDirection::Upward => candle.low > level_price,
            BreakDirection::Downward => candle.high < level_price,
        };

        if !stayed_above {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Create a breakout signal with targets and stop losses
fn create_breakout_signal(
    level: &PriceLevel,
    direction: BreakDirection,
    current_price: i128,
    current_volume: i128,
    avg_volume: i128,
) -> Result<BreakoutSignal, AutoTradeError> {
    let level_range = (level.price * 50) / 10000; // 0.5% range

    let (target_price, stop_loss) = match direction {
        BreakDirection::Upward => {
            // Target: 2x the breakout range above resistance
            let target = level.price + (level_range * 2);
            // Stop: Just below the broken resistance (now support)
            let stop = level.price - (level_range / 2);
            (target, stop)
        }
        BreakDirection::Downward => {
            // Target: 2x the breakout range below support
            let target = level.price - (level_range * 2);
            // Stop: Just above the broken support (now resistance)
            let stop = level.price + (level_range / 2);
            (target, stop)
        }
    };

    let volume_ratio = if avg_volume > 0 {
        ((current_volume * 100) / avg_volume) as u32
    } else {
        100
    };

    Ok(BreakoutSignal {
        direction,
        breakout_level: level.price,
        current_price,
        target_price,
        stop_loss,
        level_strength: level.strength,
        volume_ratio,
        confidence: calculate_breakout_confidence(level.strength, volume_ratio)?,
    })
}

/// Calculate confidence score for breakout signal
fn calculate_breakout_confidence(
    level_strength: u32,
    volume_ratio: u32,
) -> Result<u32, AutoTradeError> {
    // Stronger level + higher volume = higher confidence
    let strength_score = if level_strength * 1000 > 4000 {
        4000
    } else {
        level_strength * 1000
    };

    let volume_contribution = if volume_ratio > 100 {
        volume_ratio - 100
    } else {
        0
    };
    let volume_score = if volume_contribution * 20 > 4000 {
        4000
    } else {
        volume_contribution * 20
    };

    let base_score = 2000; // 20% base
    Ok(strength_score + volume_score + base_score)
}

/// ==========================
/// Trade Execution
/// ==========================

/// Execute a breakout trade based on signal
pub fn execute_breakout_trade(
    strategy_id: u64,
    signal: &BreakoutSignal,
) -> Result<u64, AutoTradeError> {
    let mut strategy = get_breakout_strategy_mut(strategy_id)?;

    let portfolio_value = get_portfolio_value(strategy.user)?;
    let position_amount = (portfolio_value * strategy.position_size_pct as i128) / 10000;

    // For simplicity, trade execution is assumed to succeed
    let position_id = generate_position_id();

    let position = BreakoutPosition {
        position_id,
        direction: signal.direction,
        breakout_level: signal.breakout_level,
        entry_price: signal.current_price,
        entry_volume: signal.volume_ratio as i128,
        stop_loss: signal.stop_loss,
        target_price: signal.target_price,
        amount: position_amount,
        status: PositionStatus::Open,
        entry_time: 0, // Would be set to env.ledger().timestamp()
    };

    strategy.active_positions.push_back(position);

    Ok(position_id)
}

/// ==========================
/// Exit Management
/// ==========================

/// Check if any open positions should be exited
pub fn check_breakout_exits(strategy_id: u64) -> Result<Vec<u64>, AutoTradeError> {
    let mut strategy = get_breakout_strategy_mut(strategy_id)?;
    let current_price = get_current_price(strategy.asset_pair)?;

    let mut closed_positions: Vec<u64> = Vec::new();

    for i in 0..strategy.active_positions.len() {
        let mut position = strategy.active_positions.get(i).unwrap().clone();

        if position.status != PositionStatus::Open {
            continue;
        }

        let should_exit = match position.direction {
            BreakDirection::Upward => {
                current_price >= position.target_price || current_price <= position.stop_loss
            }
            BreakDirection::Downward => {
                current_price <= position.target_price || current_price >= position.stop_loss
            }
        };

        if should_exit {
            position.status = PositionStatus::Closed;
            let _ = strategy.active_positions.set(i, position);
            let _ = closed_positions.push_back(position.position_id);
        }
    }

    Ok(closed_positions)
}

/// ==========================
/// False Breakout Detection
/// ==========================

/// Detect if a position is a false breakout (price reversed through level)
pub fn detect_false_breakout(position: &BreakoutPosition, current_price: i128) -> bool {
    match position.direction {
        BreakDirection::Upward => {
            // Price broke above resistance but fell back below
            current_price < position.breakout_level
        }
        BreakDirection::Downward => {
            // Price broke below support but rose back above
            current_price > position.breakout_level
        }
    }
}

/// Handle false breakout by closing position early
pub fn handle_false_breakout(strategy_id: u64, position_id: u64) -> Result<(), AutoTradeError> {
    let mut strategy = get_breakout_strategy_mut(strategy_id)?;

    for i in 0..strategy.active_positions.len() {
        let mut position = strategy.active_positions.get(i).unwrap().clone();
        if position.position_id == position_id {
            position.status = PositionStatus::Closed;
            let _ = strategy.active_positions.set(i, position);
            break;
        }
    }

    Ok(())
}

/// ==========================
/// Dynamic Level Updates
/// ==========================

/// Update key levels after a successful breakout
pub fn update_key_levels_on_breakout(
    strategy_id: u64,
    broken_level: i128,
    direction: BreakDirection,
) -> Result<(), AutoTradeError> {
    let mut strategy = get_breakout_strategy_mut(strategy_id)?;

    let tolerance = (broken_level * PRICE_LEVEL_TOLERANCE_BPS) / 10000;

    for i in 0..strategy.key_levels.len() {
        let mut level = strategy.key_levels.get(i).unwrap().clone();

        if (level.price - broken_level).abs() < tolerance {
            level.level_type = match direction {
                BreakDirection::Upward => LevelType::Support,
                BreakDirection::Downward => LevelType::Resistance,
            };
            level.strength += 1;

            let _ = strategy.key_levels.set(i, level);
        }
    }

    Ok(())
}

/// ==========================
/// Performance Analytics
/// ==========================

/// Analyze performance of breakout strategy
pub fn analyze_breakout_performance(strategy_id: u64) -> Result<BreakoutPerformance, AutoTradeError> {
    let strategy = get_breakout_strategy(strategy_id)?;

    let total_positions = strategy.active_positions.len() as u32;
    let mut successful = 0u32;
    let mut false_breakouts = 0u32;
    let mut stopped_out = 0u32;
    let mut total_profit_pct: i32 = 0;
    let mut total_volume: i128 = 0;

    for i in 0..strategy.active_positions.len() {
        let position = strategy.active_positions.get(i).unwrap();

        if position.status == PositionStatus::Closed {
            let current_price = get_current_price(strategy.asset_pair)?;

            let pnl_pct = if position.entry_price > 0 {
                ((current_price - position.entry_price) * 10000) / position.entry_price
            } else {
                0
            };

            total_profit_pct += pnl_pct as i32;
            total_volume += position.entry_volume;

            if detect_false_breakout(position, current_price) {
                false_breakouts += 1;
            } else {
                let hit_target = match position.direction {
                    BreakDirection::Upward => current_price >= position.target_price,
                    BreakDirection::Downward => current_price <= position.target_price,
                };

                if hit_target {
                    successful += 1;
                } else {
                    stopped_out += 1;
                }
            }
        }
    }

    let avg_profit_pct = if total_positions > 0 {
        total_profit_pct / total_positions as i32
    } else {
        0
    };

    let avg_volume = if total_positions > 0 {
        (total_volume / total_positions as i128) as u32
    } else {
        0
    };

    Ok(BreakoutPerformance {
        total_breakouts_detected: total_positions,
        total_trades_executed: total_positions,
        successful_breakouts: successful,
        false_breakouts,
        stopped_out,
        avg_breakout_profit_pct: avg_profit_pct,
        avg_volume_on_breakout: avg_volume,
    })
}

/// ==========================
/// Helper Functions
/// ==========================

/// Get historical candles (mock implementation)
#[allow(unused_variables)]
fn get_historical_candles(
    asset_pair: AssetPair,
    lookback_seconds: u64,
    candle_duration_seconds: u64,
) -> Result<Vec<Candle>, AutoTradeError> {
    // This would fetch from price history storage
    // For now, return empty vector - integration point with price oracle
    Ok(Vec::new())
}

/// Get latest candle
#[allow(unused_variables)]
fn get_latest_candle(
    asset_pair: AssetPair,
    candle_duration_seconds: u64,
) -> Result<Candle, AutoTradeError> {
    // This would fetch from price history storage
    Ok(Candle {
        timestamp: 0,
        open: 0,
        high: 0,
        low: 0,
        close: 0,
        volume: 0,
    })
}

/// Get recent candles for confirmation
#[allow(unused_variables)]
fn get_recent_candles(
    asset_pair: AssetPair,
    num_candles: usize,
    candle_duration_seconds: u64,
) -> Result<Vec<Candle>, AutoTradeError> {
    // This would fetch from price history storage
    Ok(Vec::new())
}

/// Calculate average volume over a period
#[allow(unused_variables)]
fn calculate_average_volume(
    asset_pair: AssetPair,
    lookback_days: u32,
) -> Result<i128, AutoTradeError> {
    // This would calculate from historical candles
    Ok(1_000_000) // Placeholder
}

/// Get current price of asset pair
#[allow(unused_variables)]
fn get_current_price(asset_pair: AssetPair) -> Result<i128, AutoTradeError> {
    // This would fetch from price oracle
    Ok(0)
}

/// Get portfolio value for a user
#[allow(unused_variables)]
fn get_portfolio_value(user: Address) -> Result<i128, AutoTradeError> {
    // This would fetch from portfolio storage
    Ok(1_000_000) // Placeholder
}

/// Get breakout strategy by ID
#[allow(unused_variables)]
fn get_breakout_strategy(strategy_id: u64) -> Result<BreakoutStrategy, AutoTradeError> {
    // This would fetch from storage
    Err(AutoTradeError::StrategyNotFound)
}

/// Get mutable reference to breakout strategy
#[allow(unused_variables)]
fn get_breakout_strategy_mut(strategy_id: u64) -> Result<BreakoutStrategy, AutoTradeError> {
    // This would fetch from storage
    Err(AutoTradeError::StrategyNotFound)
}

/// Generate unique position ID
fn generate_position_id() -> u64 {
    // In real implementation, this would use a counter
    42
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_position(direction: BreakDirection, breakout_level: i128) -> BreakoutPosition {
        BreakoutPosition {
            position_id: 1,
            direction,
            breakout_level,
            entry_price: breakout_level,
            entry_volume: 200,
            stop_loss: breakout_level - 50,
            target_price: breakout_level + 100,
            amount: 1_000,
            status: PositionStatus::Open,
            entry_time: 0,
        }
    }

    #[test]
    fn false_breakout_detected_for_upward_reversal() {
        let position = sample_position(BreakDirection::Upward, 1000);
        assert!(detect_false_breakout(&position, 999));
    }

    #[test]
    fn false_breakout_detected_for_downward_reversal() {
        let position = sample_position(BreakDirection::Downward, 1000);
        assert!(detect_false_breakout(&position, 1001));
    }

    #[test]
    fn no_false_breakout_when_price_stays_on_correct_side() {
        let upward_position = sample_position(BreakDirection::Upward, 1000);
        let downward_position = sample_position(BreakDirection::Downward, 1000);

        assert!(!detect_false_breakout(&upward_position, 1005));
        assert!(!detect_false_breakout(&downward_position, 995));
    }

    #[test]
    fn breakout_confidence_has_expected_floor_and_caps() {
        // Base 20% + strength 10% + volume 0% = 30%
        let low = calculate_breakout_confidence(1, 100).unwrap();
        assert_eq!(low, 3000);

        // Strength and volume both capped at 40% each: 20 + 40 + 40 = 100%
        let high = calculate_breakout_confidence(10, 500).unwrap();
        assert_eq!(high, 10000);
    }

    #[test]
    fn create_breakout_signal_sets_upward_targets_and_stops() {
        let level = PriceLevel {
            price: 10_000,
            level_type: LevelType::Resistance,
            strength: 3,
            last_test: 0,
        };

        let signal = create_breakout_signal(&level, BreakDirection::Upward, 10_050, 2_000, 1_000)
            .unwrap();

        // range = 0.5% of 10_000 = 50
        assert_eq!(signal.target_price, 10_100);
        assert_eq!(signal.stop_loss, 9_975);
        assert_eq!(signal.volume_ratio, 200);
    }

    #[test]
    fn create_breakout_signal_sets_downward_targets_and_stops() {
        let level = PriceLevel {
            price: 10_000,
            level_type: LevelType::Support,
            strength: 2,
            last_test: 0,
        };

        let signal = create_breakout_signal(
            &level,
            BreakDirection::Downward,
            9_950,
            1_500,
            1_000,
        )
        .unwrap();

        // range = 50, target = 10_000 - 100, stop = 10_000 + 25
        assert_eq!(signal.target_price, 9_900);
        assert_eq!(signal.stop_loss, 10_025);
        assert_eq!(signal.volume_ratio, 150);
    }
}
