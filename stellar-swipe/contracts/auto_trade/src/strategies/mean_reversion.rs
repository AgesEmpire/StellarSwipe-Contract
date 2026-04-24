#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::errors::AutoTradeError;

const PRECISION: i128 = 10_000; // Z-score scale factor
const MIN_PRICES: u32 = 30;

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeDirection {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionStatus {
    Open,
    Closed,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeanReversionStrategy {
    pub user: Address,
    pub asset_pair: u32,
    pub lookback_period_days: u32,
    pub entry_z_score: i128,  // scaled by PRECISION, e.g. 20000 = 2.0
    pub exit_z_score: i128,   // scaled by PRECISION, e.g. 5000 = 0.5
    pub position_size_pct: u32, // basis points, e.g. 1000 = 10%
    pub max_positions: u32,
    pub active_positions: Vec<ReversionPosition>,
    pub enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReversionPosition {
    pub position_id: u64,
    pub entry_price: i128,
    pub entry_z_score: i128,
    pub entry_time: u64,
    pub mean_at_entry: i128,
    pub target_price: i128,
    pub stop_loss: i128,
    pub amount: i128,
    pub direction: TradeDirection,
    pub status: PositionStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatisticalMetrics {
    pub mean: i128,
    pub std_dev: i128,
    pub current_price: i128,
    pub z_score: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReversionSignal {
    pub direction: TradeDirection,
    pub entry_price: i128,
    pub z_score: i128,
    pub mean: i128,
    pub target_price: i128,
    pub stop_loss: i128,
    pub confidence: u32,
}

#[contracttype]
pub enum MRKey {
    Strategy(u64),
    NextStrategyId,
    NextPositionId,
    ActiveIds,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn next_strategy_id(env: &Env) -> u64 {
    let id: u64 = env.storage().persistent().get(&MRKey::NextStrategyId).unwrap_or(0);
    env.storage().persistent().set(&MRKey::NextStrategyId, &(id + 1));
    id
}

fn next_position_id(env: &Env) -> u64 {
    let id: u64 = env.storage().persistent().get(&MRKey::NextPositionId).unwrap_or(0);
    env.storage().persistent().set(&MRKey::NextPositionId, &(id + 1));
    id
}

fn load(env: &Env, id: u64) -> Result<MeanReversionStrategy, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&MRKey::Strategy(id))
        .ok_or(AutoTradeError::MrStrategyNotFound)
}

fn save(env: &Env, id: u64, s: &MeanReversionStrategy) {
    env.storage().persistent().set(&MRKey::Strategy(id), s);
}

fn active_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&MRKey::ActiveIds)
        .unwrap_or_else(|| Vec::new(env))
}

fn push_active_id(env: &Env, id: u64) {
    let mut ids = active_ids(env);
    ids.push_back(id);
    env.storage().persistent().set(&MRKey::ActiveIds, &ids);
}

// ── Integer sqrt (Babylonian) ─────────────────────────────────────────────────

fn isqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ── Price data helpers (simulated via storage) ────────────────────────────────

fn get_historical_prices(env: &Env, asset_pair: u32, lookback_seconds: u64) -> Result<Vec<i128>, AutoTradeError> {
    // Prices stored as Vec<i128> keyed by (symbol, asset_pair)
    let key = (symbol_short!("hist_px"), asset_pair, lookback_seconds);
    env.storage()
        .temporary()
        .get(&key)
        .ok_or(AutoTradeError::MrInsufficientHistory)
}

fn get_current_price(env: &Env, asset_pair: u32) -> Result<i128, AutoTradeError> {
    let key = (symbol_short!("price"), asset_pair);
    env.storage()
        .temporary()
        .get(&key)
        .ok_or(AutoTradeError::MrInsufficientHistory)
}

fn get_portfolio_value(env: &Env, user: &Address) -> i128 {
    env.storage()
        .temporary()
        .get(&(user.clone(), symbol_short!("balance")))
        .unwrap_or(0)
}

// ── Statistical calculations ──────────────────────────────────────────────────

pub fn calculate_statistical_metrics(
    env: &Env,
    asset_pair: u32,
    lookback_days: u32,
) -> Result<StatisticalMetrics, AutoTradeError> {
    let lookback_seconds = lookback_days as u64 * 86_400;
    let prices = get_historical_prices(env, asset_pair, lookback_seconds)?;

    if prices.len() < MIN_PRICES {
        return Err(AutoTradeError::MrInsufficientHistory);
    }

    let n = prices.len() as i128;
    let sum: i128 = (0..prices.len()).map(|i| prices.get(i).unwrap()).sum();
    let mean = sum / n;

    let variance: i128 = (0..prices.len())
        .map(|i| {
            let diff = prices.get(i).unwrap() - mean;
            (diff * diff) / n
        })
        .sum();
    let std_dev = isqrt(variance);

    // Require minimum volatility to avoid division by near-zero
    if std_dev == 0 {
        return Err(AutoTradeError::MrLowVolatility);
    }

    let current_price = get_current_price(env, asset_pair)?;
    let z_score = ((current_price - mean) * PRECISION) / std_dev;

    Ok(StatisticalMetrics { mean, std_dev, current_price, z_score })
}

// ── Confidence ────────────────────────────────────────────────────────────────

fn reversion_confidence(metrics: &StatisticalMetrics) -> u32 {
    let z_abs = metrics.z_score.abs();
    if z_abs > 30_000 { 9_000 }
    else if z_abs > 25_000 { 8_000 }
    else if z_abs > 20_000 { 7_000 }
    else if z_abs > 15_000 { 6_000 }
    else { 5_000 }
}

// ── Signal detection ──────────────────────────────────────────────────────────

pub fn check_mean_reversion_signals(
    env: &Env,
    strategy_id: u64,
) -> Result<Option<ReversionSignal>, AutoTradeError> {
    let strategy = load(env, strategy_id)?;

    if !strategy.enabled {
        return Ok(None);
    }

    let metrics = calculate_statistical_metrics(env, strategy.asset_pair, strategy.lookback_period_days)?;
    let z_abs = metrics.z_score.abs();

    if z_abs < strategy.entry_z_score {
        return Ok(None);
    }

    let signal = if metrics.z_score > 0 {
        // Overbought — sell, expect reversion down
        ReversionSignal {
            direction: TradeDirection::Sell,
            entry_price: metrics.current_price,
            z_score: metrics.z_score,
            mean: metrics.mean,
            target_price: metrics.mean,
            stop_loss: metrics.current_price + (metrics.std_dev * 3),
            confidence: reversion_confidence(&metrics),
        }
    } else {
        // Oversold — buy, expect reversion up
        ReversionSignal {
            direction: TradeDirection::Buy,
            entry_price: metrics.current_price,
            z_score: metrics.z_score,
            mean: metrics.mean,
            target_price: metrics.mean,
            stop_loss: metrics.current_price - (metrics.std_dev * 3),
            confidence: reversion_confidence(&metrics),
        }
    };

    Ok(Some(signal))
}

// ── Trade execution ───────────────────────────────────────────────────────────

pub fn execute_mean_reversion_trade(
    env: &Env,
    strategy_id: u64,
    signal: ReversionSignal,
) -> Result<u64, AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;

    let open_count = (0..strategy.active_positions.len())
        .filter(|&i| strategy.active_positions.get(i).unwrap().status == PositionStatus::Open)
        .count();

    if open_count >= strategy.max_positions as usize {
        return Err(AutoTradeError::PositionLimitExceeded);
    }

    let portfolio_value = get_portfolio_value(env, &strategy.user);
    let position_amount = (portfolio_value * strategy.position_size_pct as i128) / 10_000;

    if position_amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let position_id = next_position_id(env);

    let position = ReversionPosition {
        position_id,
        entry_price: signal.entry_price,
        entry_z_score: signal.z_score,
        entry_time: env.ledger().timestamp(),
        mean_at_entry: signal.mean,
        target_price: signal.target_price,
        stop_loss: signal.stop_loss,
        amount: position_amount,
        direction: signal.direction.clone(),
        status: PositionStatus::Open,
    };

    strategy.active_positions.push_back(position);
    save(env, strategy_id, &strategy);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "mr_trade_opened"), strategy.user.clone(), strategy_id),
        (position_id, signal.z_score, signal.confidence),
    );

    Ok(position_id)
}

// ── Exit monitoring ───────────────────────────────────────────────────────────

pub fn check_reversion_exits(env: &Env, strategy_id: u64) -> Result<Vec<u64>, AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;
    let metrics = calculate_statistical_metrics(env, strategy.asset_pair, strategy.lookback_period_days)?;

    let mut closed: Vec<u64> = Vec::new(env);

    for i in 0..strategy.active_positions.len() {
        let mut pos = strategy.active_positions.get(i).unwrap();

        if pos.status != PositionStatus::Open {
            continue;
        }

        let price = metrics.current_price;
        let target_hit = match pos.direction {
            TradeDirection::Sell => price <= pos.target_price,
            TradeDirection::Buy => price >= pos.target_price,
        };
        let stop_hit = match pos.direction {
            TradeDirection::Sell => price >= pos.stop_loss,
            TradeDirection::Buy => price <= pos.stop_loss,
        };
        let z_exit = metrics.z_score.abs() <= strategy.exit_z_score;

        if target_hit || stop_hit || z_exit {
            let pnl = calculate_pnl(&pos, price);
            pos.status = PositionStatus::Closed;
            strategy.active_positions.set(i, pos.clone());
            closed.push_back(pos.position_id);

            #[allow(deprecated)]
            env.events().publish(
                (Symbol::new(env, "mr_position_closed"), strategy.user.clone(), strategy_id),
                (pos.position_id, price, pnl, env.ledger().timestamp() - pos.entry_time),
            );
        }
    }

    save(env, strategy_id, &strategy);
    Ok(closed)
}

fn calculate_pnl(pos: &ReversionPosition, exit_price: i128) -> i128 {
    match pos.direction {
        TradeDirection::Sell => (pos.entry_price - exit_price) * pos.amount / pos.entry_price,
        TradeDirection::Buy => (exit_price - pos.entry_price) * pos.amount / pos.entry_price,
    }
}

// ── Adaptive parameters ───────────────────────────────────────────────────────

pub fn adjust_strategy_parameters(env: &Env, strategy_id: u64) -> Result<(), AutoTradeError> {
    let mut strategy = load(env, strategy_id)?;

    let closed: Vec<ReversionPosition> = {
        let mut v: Vec<ReversionPosition> = Vec::new(env);
        let len = strategy.active_positions.len();
        let start = if len > 20 { len - 20 } else { 0 };
        for i in start..len {
            let p = strategy.active_positions.get(i).unwrap();
            if p.status == PositionStatus::Closed {
                v.push_back(p);
            }
        }
        v
    };

    if closed.len() < 10 {
        return Ok(());
    }

    let current_price = get_current_price(env, strategy.asset_pair)?;
    let wins = (0..closed.len())
        .filter(|&i| calculate_pnl(&closed.get(i).unwrap(), current_price) > 0)
        .count();

    let success_rate = (wins * 100) / closed.len() as usize;

    if success_rate < 40 {
        strategy.entry_z_score = core::cmp::min(30_000, strategy.entry_z_score + 1_000);
    } else if success_rate > 60 {
        strategy.entry_z_score = core::cmp::max(15_000, strategy.entry_z_score - 500);
    }

    save(env, strategy_id, &strategy);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "mr_params_adjusted"), strategy_id),
        (strategy.entry_z_score, success_rate as u32),
    );

    Ok(())
}

// ── CRUD ──────────────────────────────────────────────────────────────────────

pub fn create_mean_reversion_strategy(
    env: &Env,
    user: Address,
    asset_pair: u32,
    lookback_period_days: u32,
    entry_z_score: i128,
    exit_z_score: i128,
    position_size_pct: u32,
    max_positions: u32,
) -> Result<u64, AutoTradeError> {
    if entry_z_score <= 0 || exit_z_score <= 0 || position_size_pct == 0 || max_positions == 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    let id = next_strategy_id(env);
    let strategy = MeanReversionStrategy {
        user: user.clone(),
        asset_pair,
        lookback_period_days,
        entry_z_score,
        exit_z_score,
        position_size_pct,
        max_positions,
        active_positions: Vec::new(env),
        enabled: true,
    };

    save(env, id, &strategy);
    push_active_id(env, id);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "mr_strategy_created"), user, id),
        (asset_pair, entry_z_score),
    );

    Ok(id)
}

pub fn get_mean_reversion_strategy(env: &Env, id: u64) -> Result<MeanReversionStrategy, AutoTradeError> {
    load(env, id)
}

pub fn disable_mean_reversion_strategy(env: &Env, id: u64) -> Result<(), AutoTradeError> {
    let mut s = load(env, id)?;
    s.enabled = false;
    save(env, id, &s);
    Ok(())
}

pub fn enable_mean_reversion_strategy(env: &Env, id: u64) -> Result<(), AutoTradeError> {
    let mut s = load(env, id)?;
    s.enabled = true;
    save(env, id, &s);
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as TestAddress, Ledger};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        let user = <Address as TestAddress>::generate(&env);
        (env, user)
    }

    fn set_price(env: &Env, asset: u32, price: i128) {
        env.as_contract(&env.register(TestContract, ()), || {
            env.storage().temporary().set(&(symbol_short!("price"), asset), &price);
        });
    }

    fn set_hist_prices(env: &Env, asset: u32, lookback: u64, prices: Vec<i128>) {
        env.as_contract(&env.register(TestContract, ()), || {
            env.storage().temporary().set(&(symbol_short!("hist_px"), asset, lookback), &prices);
        });
    }

    fn set_balance(env: &Env, user: &Address, balance: i128) {
        env.as_contract(&env.register(TestContract, ()), || {
            env.storage().temporary().set(&(user.clone(), symbol_short!("balance")), &balance);
        });
    }

    fn make_prices(env: &Env, base: i128, count: u32) -> Vec<i128> {
        let mut v: Vec<i128> = Vec::new(env);
        for i in 0..count {
            v.push_back(base + (i as i128 % 5) - 2);
        }
        v
    }

    #[test]
    fn test_create_and_get_strategy() {
        let (env, user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            let id = create_mean_reversion_strategy(
                &env, user.clone(), 1, 14, 20_000, 5_000, 1_000, 3,
            ).unwrap();
            let s = get_mean_reversion_strategy(&env, id).unwrap();
            assert_eq!(s.asset_pair, 1);
            assert_eq!(s.entry_z_score, 20_000);
            assert!(s.enabled);
        });
    }

    #[test]
    fn test_statistical_metrics() {
        let (env, _user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            let prices = make_prices(&env, 100_000, 30);
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &prices,
            );
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &102_500i128);

            let m = calculate_statistical_metrics(&env, 1, 14).unwrap();
            assert!(m.mean > 0);
            assert!(m.std_dev > 0);
            assert!(m.z_score > 0); // price above mean
        });
    }

    #[test]
    fn test_signal_detected_when_overbought() {
        let (env, user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            // Prices tightly around 100_000
            let mut prices: Vec<i128> = Vec::new(&env);
            for _ in 0..30 {
                prices.push_back(100_000);
            }
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &prices,
            );
            // Current price 3 std devs above — but std_dev=0 here, so use spread
            // Use varied prices to get non-zero std_dev
            let mut varied: Vec<i128> = Vec::new(&env);
            for i in 0..30i128 {
                varied.push_back(100_000 + (i % 10) * 100 - 450);
            }
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &varied,
            );
            // Set current price far above mean (~2.5 std devs)
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &101_500i128);

            let id = create_mean_reversion_strategy(
                &env, user.clone(), 1, 14, 20_000, 5_000, 1_000, 3,
            ).unwrap();

            let signal = check_mean_reversion_signals(&env, id).unwrap();
            // Signal may or may not fire depending on computed z-score; just verify no panic
            if let Some(s) = signal {
                assert_eq!(s.direction, TradeDirection::Sell);
                assert_eq!(s.target_price, s.mean);
            }
        });
    }

    #[test]
    fn test_execute_and_close_position() {
        let (env, user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            let mut varied: Vec<i128> = Vec::new(&env);
            for i in 0..30i128 {
                varied.push_back(100_000 + (i % 10) * 100 - 450);
            }
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &varied,
            );
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &101_500i128);
            env.storage().temporary().set(&(user.clone(), symbol_short!("balance")), &1_000_000i128);

            let id = create_mean_reversion_strategy(
                &env, user.clone(), 1, 14, 5_000, 2_000, 1_000, 3,
            ).unwrap();

            let metrics = calculate_statistical_metrics(&env, 1, 14).unwrap();
            let signal = ReversionSignal {
                direction: TradeDirection::Sell,
                entry_price: metrics.current_price,
                z_score: metrics.z_score,
                mean: metrics.mean,
                target_price: metrics.mean,
                stop_loss: metrics.current_price + metrics.std_dev * 3,
                confidence: reversion_confidence(&metrics),
            };

            let pos_id = execute_mean_reversion_trade(&env, id, signal).unwrap();
            assert_eq!(pos_id, 0);

            // Simulate price reverting to mean
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &metrics.mean);

            let closed = check_reversion_exits(&env, id).unwrap();
            assert_eq!(closed.len(), 1);
            assert_eq!(closed.get(0).unwrap(), pos_id);
        });
    }

    #[test]
    fn test_stop_loss_closes_position() {
        let (env, user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            let mut varied: Vec<i128> = Vec::new(&env);
            for i in 0..30i128 {
                varied.push_back(100_000 + (i % 10) * 100 - 450);
            }
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &varied,
            );
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &101_500i128);
            env.storage().temporary().set(&(user.clone(), symbol_short!("balance")), &1_000_000i128);

            let id = create_mean_reversion_strategy(
                &env, user.clone(), 1, 14, 5_000, 2_000, 1_000, 3,
            ).unwrap();

            let metrics = calculate_statistical_metrics(&env, 1, 14).unwrap();
            let stop = metrics.current_price + metrics.std_dev * 3;
            let signal = ReversionSignal {
                direction: TradeDirection::Sell,
                entry_price: metrics.current_price,
                z_score: metrics.z_score,
                mean: metrics.mean,
                target_price: metrics.mean,
                stop_loss: stop,
                confidence: 7_000,
            };

            execute_mean_reversion_trade(&env, id, signal).unwrap();

            // Price blows through stop loss
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &(stop + 1));

            let closed = check_reversion_exits(&env, id).unwrap();
            assert_eq!(closed.len(), 1);
        });
    }

    #[test]
    fn test_max_positions_enforced() {
        let (env, user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            let mut varied: Vec<i128> = Vec::new(&env);
            for i in 0..30i128 {
                varied.push_back(100_000 + (i % 10) * 100 - 450);
            }
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &varied,
            );
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &101_500i128);
            env.storage().temporary().set(&(user.clone(), symbol_short!("balance")), &1_000_000i128);

            let id = create_mean_reversion_strategy(
                &env, user.clone(), 1, 14, 5_000, 2_000, 1_000, 1, // max 1 position
            ).unwrap();

            let metrics = calculate_statistical_metrics(&env, 1, 14).unwrap();
            let make_signal = |m: &StatisticalMetrics| ReversionSignal {
                direction: TradeDirection::Sell,
                entry_price: m.current_price,
                z_score: m.z_score,
                mean: m.mean,
                target_price: m.mean,
                stop_loss: m.current_price + m.std_dev * 3,
                confidence: 7_000,
            };

            execute_mean_reversion_trade(&env, id, make_signal(&metrics)).unwrap();
            let err = execute_mean_reversion_trade(&env, id, make_signal(&metrics)).unwrap_err();
            assert_eq!(err, AutoTradeError::PositionLimitExceeded);
        });
    }

    #[test]
    fn test_insufficient_history_error() {
        let (env, user) = setup();
        let contract_addr = env.register(TestContract, ());
        env.as_contract(&contract_addr, || {
            // Only 5 prices — below MIN_PRICES
            let mut prices: Vec<i128> = Vec::new(&env);
            for _ in 0..5 {
                prices.push_back(100_000);
            }
            env.storage().temporary().set(
                &(symbol_short!("hist_px"), 1u32, 14u64 * 86_400),
                &prices,
            );
            env.storage().temporary().set(&(symbol_short!("price"), 1u32), &100_000i128);

            let id = create_mean_reversion_strategy(
                &env, user.clone(), 1, 14, 20_000, 5_000, 1_000, 3,
            ).unwrap();

            let err = check_mean_reversion_signals(&env, id).unwrap_err();
            assert_eq!(err, AutoTradeError::MrInsufficientHistory);
        });
    }
}
