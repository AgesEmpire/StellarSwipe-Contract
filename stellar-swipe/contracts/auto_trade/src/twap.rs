#![allow(dead_code)]

use crate::errors::AutoTradeError;
use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TWAPStatus {
    Active,
    Complete,
    Cancelled,
    Paused,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPair {
    pub base: String,
    pub quote: String,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TWAPOrder {
    pub id: u64,
    pub user: Address,
    pub pair: AssetPair,
    pub total_amount: i128,
    pub duration_seconds: u64,
    pub interval_seconds: u64,
    pub start_time: u64,
    pub segments_executed: u32,
    pub total_segments: u32,
    pub amount_per_segment: i128,
    pub filled_amount: i128,
    pub weighted_price: i128,
    pub price_window_minutes: u32,
    pub status: TWAPStatus,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CancellationSummary {
    pub filled_amount: i128,
    pub remaining_amount: i128,
    pub avg_price: i128,
    pub segments_executed: u32,
}

#[contracttype]
pub enum TWAPStorageKey {
    Counter,
    Order(u64),
    ActiveOrders,
    PriceHistory(AssetPair),
}

// Storage functions
pub fn get_next_twap_id(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .persistent()
        .get(&TWAPStorageKey::Counter)
        .unwrap_or(0);
    let next_id = counter + 1;
    env.storage()
        .persistent()
        .set(&TWAPStorageKey::Counter, &next_id);
    next_id
}

pub fn store_twap_order(env: &Env, order_id: u64, order: &TWAPOrder) {
    env.storage()
        .persistent()
        .set(&TWAPStorageKey::Order(order_id), order);

    // Add to active orders list if active
    let mut active_orders: Vec<u64> = env
        .storage()
        .persistent()
        .get(&TWAPStorageKey::ActiveOrders)
        .unwrap_or_else(|| Vec::new(env));
    if order.status == TWAPStatus::Active && !active_orders.contains(order_id) {
        active_orders.push_back(order_id);
        env.storage()
            .persistent()
            .set(&TWAPStorageKey::ActiveOrders, &active_orders);
    } else if order.status != TWAPStatus::Active && order.status != TWAPStatus::Paused {
        if let Some(pos) = active_orders.first_index_of(order_id) {
            active_orders.remove(pos);
            env.storage()
                .persistent()
                .set(&TWAPStorageKey::ActiveOrders, &active_orders);
        }
    }
}

pub fn get_twap_order(env: &Env, order_id: u64) -> Result<TWAPOrder, AutoTradeError> {
    env.storage()
        .persistent()
        .get(&TWAPStorageKey::Order(order_id))
        .ok_or(AutoTradeError::TWAPOrderNotFound)
}

pub fn get_active_twap_orders(env: &Env) -> Vec<TWAPOrder> {
    let active_ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&TWAPStorageKey::ActiveOrders)
        .unwrap_or_else(|| Vec::new(env));
    let mut active_orders = Vec::new(env);
    for id in active_ids.iter() {
        if let Some(order) = env.storage().persistent().get(&TWAPStorageKey::Order(id)) {
            active_orders.push_back(order);
        }
    }
    active_orders
}

fn record_price_point(env: &Env, pair: &AssetPair, price: i128) {
    let mut history: Vec<i128> = env
        .storage()
        .persistent()
        .get(&TWAPStorageKey::PriceHistory(pair.clone()))
        .unwrap_or_else(|| Vec::new(env));

    if history.len() >= 60 {
        history.remove(0);
    }
    history.push_back(price);
    env.storage()
        .persistent()
        .set(&TWAPStorageKey::PriceHistory(pair.clone()), &history);
}

fn get_price_history(env: &Env, pair: &AssetPair, window: u32) -> Vec<i128> {
    let all: Vec<i128> = env
        .storage()
        .persistent()
        .get(&TWAPStorageKey::PriceHistory(pair.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut recent = Vec::new(env);
    let window = window.max(1);
    let start = if all.len() > window {
        all.len() - window
    } else {
        0
    };
    for i in start..all.len() {
        if let Some(price) = all.get(i) {
            recent.push_back(price);
        }
    }
    recent
}

fn average_price(history: &Vec<i128>) -> i128 {
    if history.len() == 0 {
        return 0;
    }
    let mut sum = 0i128;
    for price in history.iter() {
        sum += price;
    }
    sum / (history.len() as i128)
}

fn volatility_from_history(env: &Env, history: &Vec<i128>) -> u32 {
    if history.len() < 2 {
        return 0;
    }
    let mut returns = Vec::new(env);
    for i in 1..history.len() {
        let prev = history.get(i - 1).unwrap();
        let curr = history.get(i).unwrap();
        if prev > 0 {
            returns.push_back(((curr - prev).abs() * 10_000) / prev);
        }
    }
    if returns.len() == 0 {
        return 0;
    }
    let mut sum = 0i128;
    for r in returns.iter() {
        sum += r;
    }
    let mean = sum / (returns.len() as i128);
    let mut variance = 0i128;
    for r in returns.iter() {
        let diff = r - mean;
        variance += diff * diff;
    }
    let variance = variance / (returns.len() as i128);
    let mut vol = 0i128;
    if variance > 0 {
        let mut x = variance;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + variance / x) / 2;
        }
        vol = x;
    }
    if vol == 0 {
        0
    } else {
        vol as u32
    }
}

fn get_twap_price_window(env: &Env, pair: &AssetPair, window_minutes: u32) -> Vec<i128> {
    get_price_history(env, pair, window_minutes)
}

// Core functions
pub fn create_twap_order(
    env: &Env,
    user: Address,
    pair: AssetPair,
    total_amount: i128,
    duration_minutes: u32,
    num_segments: Option<u32>,
    window_minutes: Option<u32>,
) -> Result<u64, AutoTradeError> {
    user.require_auth();

    if duration_minutes == 0 {
        return Err(AutoTradeError::InvalidTWAPDuration);
    }

    let duration_seconds = duration_minutes as u64 * 60;

    // Default: 1 segment per 5% of duration, min 4
    let default_segments = (duration_minutes / 5).max(4);
    let segments = num_segments.unwrap_or(default_segments);

    if segments == 0 || segments > duration_seconds as u32 {
        return Err(AutoTradeError::InvalidTWAPDuration);
    }

    let interval_seconds = duration_seconds / segments as u64;
    let amount_per_segment = total_amount / segments as i128;
    let price_window_minutes = window_minutes.unwrap_or(15).max(5);

    let order_id = get_next_twap_id(env);

    let twap = TWAPOrder {
        id: order_id,
        user: user.clone(),
        pair,
        total_amount,
        duration_seconds,
        interval_seconds,
        start_time: env.ledger().timestamp(),
        segments_executed: 0,
        total_segments: segments,
        amount_per_segment,
        filled_amount: 0,
        weighted_price: 0,
        price_window_minutes,
        status: TWAPStatus::Active,
    };

    store_twap_order(env, order_id, &twap);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "TWAPOrderCreated"), user, order_id),
        (total_amount, duration_minutes, segments),
    );

    Ok(order_id)
}

pub fn execute_twap_segments(env: &Env) -> Vec<u64> {
    let current = env.ledger().timestamp();
    let active_orders = get_active_twap_orders(env);

    let mut executed_ids = Vec::new(env);

    for mut twap in active_orders.iter() {
        if twap.status != TWAPStatus::Active {
            continue;
        }

        let elapsed = if current > twap.start_time {
            current - twap.start_time
        } else {
            0
        };
        let expected_segments = (elapsed / twap.interval_seconds.max(1)) as u32;

        while twap.segments_executed < expected_segments
            && twap.segments_executed < twap.total_segments
        {
            // Execute segment
            match execute_twap_segment(env, &mut twap) {
                Ok(trade_id) => {
                    executed_ids.push_back(trade_id);
                }
                Err(_e) => {
                    #[allow(deprecated)]
                    env.events().publish(
                        (Symbol::new(env, "TWAPSegmentFailed"), twap.id),
                        twap.segments_executed,
                    );
                    break; // Stop trying to execute further segments on failure
                }
            }
        }

        if twap.segments_executed >= twap.total_segments {
            twap.status = TWAPStatus::Complete;
            let avg_price = if twap.filled_amount > 0 {
                twap.weighted_price / twap.filled_amount
            } else {
                0
            };
            #[allow(deprecated)]
            env.events().publish(
                (Symbol::new(env, "TWAPOrderComplete"), twap.id),
                (twap.filled_amount, avg_price),
            );
        }

        // Save updated state
        store_twap_order(env, twap.id, &twap);
    }

    executed_ids
}

fn execute_twap_segment(env: &Env, twap: &mut TWAPOrder) -> Result<u64, AutoTradeError> {
    let simulated_trade_id = env.ledger().timestamp() + twap.segments_executed as u64;
    let simulated_price = get_market_price(env, &twap.pair)?;
    let simulated_fill = twap.amount_per_segment;

    record_price_point(env, &twap.pair, simulated_price);

    twap.filled_amount += simulated_fill;
    twap.weighted_price += simulated_price * simulated_fill;
    twap.segments_executed += 1;

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "TWAPSegmentExecuted"),
            twap.id,
            twap.segments_executed,
        ),
        (simulated_fill, simulated_price),
    );

    Ok(simulated_trade_id)
}

fn get_market_price(env: &Env, pair: &AssetPair) -> Result<i128, AutoTradeError> {
    let history = get_price_history(env, pair, 4);
    if let Some(last_price) = history.get(history.len().saturating_sub(1)) {
        return Ok(last_price);
    }

    let average = average_price(&history);
    if average > 0 {
        return Ok(average);
    }

    Ok(100_000)
}

fn calculate_volatility(env: &Env, pair: &AssetPair, period: u32) -> Result<u32, AutoTradeError> {
    let history = get_price_history(env, pair, (period + 1).max(3));
    let vol = volatility_from_history(env, &history);
    if vol == 0 {
        Ok(1000)
    } else {
        Ok(vol)
    }
}

fn get_baseline_volatility(env: &Env, pair: &AssetPair) -> Result<u32, AutoTradeError> {
    let history = get_price_history(env, pair, 15);
    let vol = volatility_from_history(env, &history);
    Ok(vol.max(1000))
}

pub fn adjust_twap_strategy(env: &Env, order_id: u64) -> Result<(), AutoTradeError> {
    let mut twap = get_twap_order(env, order_id)?;

    let current_volatility = calculate_volatility(env, &twap.pair, 1)?;
    let baseline_volatility = get_baseline_volatility(env, &twap.pair)?;

    if current_volatility > baseline_volatility * 150 / 100 {
        twap.interval_seconds = twap.interval_seconds * 150 / 100;

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(env, "TWAPAdjusted"), order_id),
            (
                String::from_str(env, "High volatility"),
                twap.interval_seconds,
            ),
        );
        store_twap_order(env, order_id, &twap);
    }

    Ok(())
}

pub fn cancel_twap_order(
    env: &Env,
    order_id: u64,
    user: Address,
) -> Result<CancellationSummary, AutoTradeError> {
    user.require_auth();
    let mut twap = get_twap_order(env, order_id)?;

    if twap.user != user {
        return Err(AutoTradeError::NotTWAPOwner);
    }
    if twap.status != TWAPStatus::Active {
        return Err(AutoTradeError::TWAPNotActive);
    }

    twap.status = TWAPStatus::Cancelled;
    store_twap_order(env, order_id, &twap);

    let remaining_amount = twap.total_amount - twap.filled_amount;
    let avg_price = if twap.filled_amount > 0 {
        twap.weighted_price / twap.filled_amount
    } else {
        0
    };

    let summary = CancellationSummary {
        filled_amount: twap.filled_amount,
        remaining_amount,
        avg_price,
        segments_executed: twap.segments_executed,
    };

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "TWAPOrderCancelled"), order_id),
        (summary.filled_amount, summary.remaining_amount),
    );

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AutoTradeContract, AutoTradeContractClient};
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    fn setup() -> (Env, Address, AutoTradeContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let contract = env.register(AutoTradeContract, ());
        let client = AutoTradeContractClient::new(&env, &contract);
        (env, contract, client)
    }

    fn user(env: &Env) -> Address {
        Address::generate(env)
    }

    #[test]
    fn test_create_twap_order() {
        let (env, _contract, client) = setup();
        let trader = user(&env);
        let pair = AssetPair {
            base: String::from_str(&env, "XLM"),
            quote: String::from_str(&env, "USDC"),
        };

        let order_id = client.create_twap_order(&trader, &pair, &10000, &60, &None, &None);
        let twap = client.get_twap_order(&order_id);

        assert_eq!(twap.user, trader);
        assert_eq!(twap.total_amount, 10000);
        assert_eq!(twap.duration_seconds, 3600);
        assert_eq!(twap.total_segments, 12);
        assert_eq!(twap.interval_seconds, 300);
        assert_eq!(twap.amount_per_segment, 10000 / 12);
        assert_eq!(twap.segments_executed, 0);
        assert_eq!(twap.status, TWAPStatus::Active);
    }

    #[test]
    fn test_twap_segment_execution() {
        let (env, _contract, client) = setup();
        let trader = user(&env);
        let pair = AssetPair {
            base: String::from_str(&env, "XLM"),
            quote: String::from_str(&env, "USDC"),
        };

        let order_id = client.create_twap_order(&trader, &pair, &12000, &60, &Some(12), &None);
        let twap_before = client.get_twap_order(&order_id);
        assert_eq!(twap_before.amount_per_segment, 1000);

        env.ledger().set_timestamp(1_000 + 301);
        let executed_ids = client.execute_twap_segments();
        assert_eq!(executed_ids.len(), 1);

        let twap_after_1 = client.get_twap_order(&order_id);
        assert_eq!(twap_after_1.segments_executed, 1);
        assert_eq!(twap_after_1.filled_amount, 1000);

        env.ledger().set_timestamp(1301 + 900);
        let executed_ids_2 = client.execute_twap_segments();
        assert_eq!(executed_ids_2.len(), 3);

        let twap_after_4 = client.get_twap_order(&order_id);
        assert_eq!(twap_after_4.segments_executed, 4);
        assert_eq!(twap_after_4.filled_amount, 4000);
    }

    #[test]
    fn test_twap_cancellation() {
        let (env, _contract, client) = setup();
        let trader = user(&env);
        let pair = AssetPair {
            base: String::from_str(&env, "BTC"),
            quote: String::from_str(&env, "USD"),
        };

        let order_id = client.create_twap_order(&trader, &pair, &6000, &60, &Some(6), &None);

        env.ledger().set_timestamp(1_000 + 1201);
        client.execute_twap_segments();

        let summary = client.cancel_twap_order(&order_id, &trader);
        assert_eq!(summary.segments_executed, 2);
        assert_eq!(summary.filled_amount, 2000);
        assert_eq!(summary.remaining_amount, 4000);

        let twap = client.get_twap_order(&order_id);
        assert_eq!(twap.status, TWAPStatus::Cancelled);
    }

    #[test]
    fn test_twap_dynamic_adjustment() {
        let (env, contract, client) = setup();
        let trader = user(&env);
        let pair = AssetPair {
            base: String::from_str(&env, "ETH"),
            quote: String::from_str(&env, "USD"),
        };

        let order_id = client.create_twap_order(&trader, &pair, &1000, &100, &Some(10), &None);
        let initial_interval = client.get_twap_order(&order_id).interval_seconds;
        assert_eq!(initial_interval, 600);

        env.as_contract(&contract, || {
            for i in 0..15 {
                record_price_point(&env, &pair, 100_000 + (i as i128));
            }
            record_price_point(&env, &pair, 100_000);
            record_price_point(&env, &pair, 250_000);
        });

        client.adjust_twap_strategy(&order_id);

        let adjusted_twap = client.get_twap_order(&order_id);
        assert_eq!(adjusted_twap.interval_seconds, 600 * 150 / 100);
    }

    #[test]
    fn test_order_completion() {
        let (env, _contract, client) = setup();
        let trader = user(&env);
        let pair = AssetPair {
            base: String::from_str(&env, "SOL"),
            quote: String::from_str(&env, "USDC"),
        };

        let order_id = client.create_twap_order(&trader, &pair, &5000, &50, &Some(5), &None);

        env.ledger().set_timestamp(1_000 + 3001);
        let executed_ids = client.execute_twap_segments();

        assert_eq!(executed_ids.len(), 5);

        let twap = client.get_twap_order(&order_id);
        assert_eq!(twap.segments_executed, 5);
        assert_eq!(twap.filled_amount, 5000);
        assert_eq!(twap.status, TWAPStatus::Complete);
    }
}
