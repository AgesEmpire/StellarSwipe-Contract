#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};
use crate::errors::AutoTradeError;

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
}

// Storage functions
pub fn get_next_twap_id(env: &Env) -> u64 {
    let counter: u64 = env.storage().persistent().get(&TWAPStorageKey::Counter).unwrap_or(0);
    let next_id = counter + 1;
    env.storage().persistent().set(&TWAPStorageKey::Counter, &next_id);
    next_id
}

pub fn store_twap_order(env: &Env, order_id: u64, order: &TWAPOrder) {
    env.storage().persistent().set(&TWAPStorageKey::Order(order_id), order);
    
    // Add to active orders list if active
    let mut active_orders: Vec<u64> = env.storage().persistent().get(&TWAPStorageKey::ActiveOrders).unwrap_or_else(|| Vec::new(env));
    if order.status == TWAPStatus::Active && !active_orders.contains(order_id) {
        active_orders.push_back(order_id);
        env.storage().persistent().set(&TWAPStorageKey::ActiveOrders, &active_orders);
    } else if order.status != TWAPStatus::Active && order.status != TWAPStatus::Paused {
        if let Some(pos) = active_orders.first_index_of(order_id) {
            active_orders.remove(pos);
            env.storage().persistent().set(&TWAPStorageKey::ActiveOrders, &active_orders);
        }
    }
}

pub fn get_twap_order(env: &Env, order_id: u64) -> Result<TWAPOrder, AutoTradeError> {
    env.storage().persistent().get(&TWAPStorageKey::Order(order_id)).ok_or(AutoTradeError::TWAPOrderNotFound)
}

pub fn get_active_twap_orders(env: &Env) -> Vec<TWAPOrder> {
    let active_ids: Vec<u64> = env.storage().persistent().get(&TWAPStorageKey::ActiveOrders).unwrap_or_else(|| Vec::new(env));
    let mut active_orders = Vec::new(env);
    for id in active_ids.iter() {
        if let Some(order) = env.storage().persistent().get(&TWAPStorageKey::Order(id)) {
            active_orders.push_back(order);
        }
    }
    active_orders
}

// Core functions
pub fn create_twap_order(
    env: &Env,
    user: Address,
    pair: AssetPair,
    total_amount: i128,
    duration_minutes: u32,
    num_segments: Option<u32>
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

        let elapsed = if current > twap.start_time { current - twap.start_time } else { 0 };
        let expected_segments = (elapsed / twap.interval_seconds.max(1)) as u32;

        while twap.segments_executed < expected_segments && twap.segments_executed < twap.total_segments {
            // Execute segment
            match execute_twap_segment(env, &mut twap) {
                Ok(trade_id) => {
                    executed_ids.push_back(trade_id);
                },
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

    twap.filled_amount += simulated_fill;
    twap.weighted_price += simulated_price * simulated_fill;
    twap.segments_executed += 1;

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "TWAPSegmentExecuted"), twap.id, twap.segments_executed),
        (simulated_fill, simulated_price),
    );

    Ok(simulated_trade_id)
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
            (String::from_str(env, "High volatility"), twap.interval_seconds),
        );
        store_twap_order(env, order_id, &twap);
    }
    
    Ok(())
}

pub fn cancel_twap_order(env: &Env, order_id: u64, user: Address) -> Result<CancellationSummary, AutoTradeError> {
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

// Dummy functions representing market interactions
fn get_market_price(_env: &Env, _pair: &AssetPair) -> Result<i128, AutoTradeError> {
    // In production, this would query a decentralized oracle or DEX liquidity pool
    Ok(100_000)
}

fn calculate_volatility(_env: &Env, _pair: &AssetPair, _period: u32) -> Result<u32, AutoTradeError> {
    Ok(1500)
}

fn get_baseline_volatility(_env: &Env, _pair: &AssetPair) -> Result<u32, AutoTradeError> {
    Ok(1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    fn setup_env() -> (Env, Address) {
        let env = Env::default();
        let user = Address::generate(&env);
        env.ledger().set_timestamp(1_000);
        (env, user)
    }

    #[test]
    fn test_create_twap_order() {
        let (env, user) = setup_env();
        let pair = AssetPair {
            base: String::from_str(&env, "XLM"),
            quote: String::from_str(&env, "USDC"),
        };

        // 10000 XLM over 60 mins -> default segments: max(60/5, 4) = 12
        let result = create_twap_order(&env, user.clone(), pair.clone(), 10000, 60, None);
        assert!(result.is_ok());

        let order_id = result.unwrap();
        let twap = get_twap_order(&env, order_id).unwrap();

        assert_eq!(twap.user, user);
        assert_eq!(twap.total_amount, 10000);
        assert_eq!(twap.duration_seconds, 3600);
        assert_eq!(twap.total_segments, 12);
        assert_eq!(twap.interval_seconds, 300); // 3600 / 12 = 300 sec (5 minutes)
        assert_eq!(twap.amount_per_segment, 10000 / 12);
        assert_eq!(twap.segments_executed, 0);
        assert_eq!(twap.status, TWAPStatus::Active);
    }

    #[test]
    fn test_twap_segment_execution() {
        let (env, user) = setup_env();
        let pair = AssetPair {
            base: String::from_str(&env, "XLM"),
            quote: String::from_str(&env, "USDC"),
        };

        let order_id = create_twap_order(&env, user.clone(), pair.clone(), 12000, 60, Some(12)).unwrap();
        
        let twap_before = get_twap_order(&env, order_id).unwrap();
        assert_eq!(twap_before.amount_per_segment, 1000);

        // Advance time by 5 minutes (1 segment interval)
        env.ledger().set_timestamp(1_000 + 301);
        let executed_ids = execute_twap_segments(&env);
        assert_eq!(executed_ids.len(), 1);

        let twap_after_1 = get_twap_order(&env, order_id).unwrap();
        assert_eq!(twap_after_1.segments_executed, 1);
        assert_eq!(twap_after_1.filled_amount, 1000);

        // Advance time by 15 more minutes (3 segment intervals)
        env.ledger().set_timestamp(1301 + 900);
        let executed_ids_2 = execute_twap_segments(&env);
        assert_eq!(executed_ids_2.len(), 3);

        let twap_after_4 = get_twap_order(&env, order_id).unwrap();
        assert_eq!(twap_after_4.segments_executed, 4);
        assert_eq!(twap_after_4.filled_amount, 4000);
    }

    #[test]
    fn test_twap_cancellation() {
        let (env, user) = setup_env();
        let pair = AssetPair {
            base: String::from_str(&env, "BTC"),
            quote: String::from_str(&env, "USD"),
        };

        let order_id = create_twap_order(&env, user.clone(), pair.clone(), 6000, 60, Some(6)).unwrap();
        
        // Execute 2 segments (20 minutes pass)
        env.ledger().set_timestamp(1_000 + 1201);
        execute_twap_segments(&env);

        let summary = cancel_twap_order(&env, order_id, user.clone()).unwrap();
        assert_eq!(summary.segments_executed, 2);
        assert_eq!(summary.filled_amount, 2000);
        assert_eq!(summary.remaining_amount, 4000);

        let twap = get_twap_order(&env, order_id).unwrap();
        assert_eq!(twap.status, TWAPStatus::Cancelled);
    }

    #[test]
    fn test_twap_dynamic_adjustment() {
        let (env, user) = setup_env();
        let pair = AssetPair {
            base: String::from_str(&env, "ETH"),
            quote: String::from_str(&env, "USD"),
        };

        // Create with 10 minute interval
        let order_id = create_twap_order(&env, user.clone(), pair.clone(), 1000, 100, Some(10)).unwrap();
        let initial_interval = get_twap_order(&env, order_id).unwrap().interval_seconds;
        assert_eq!(initial_interval, 600);

        // Adjust strategy (volatility is mocked to be > baseline * 1.5)
        let _ = adjust_twap_strategy(&env, order_id);

        let adjusted_twap = get_twap_order(&env, order_id).unwrap();
        // 50% increase in interval
        assert_eq!(adjusted_twap.interval_seconds, 600 * 150 / 100);
    }

    #[test]
    fn test_order_completion() {
        let (env, user) = setup_env();
        let pair = AssetPair {
            base: String::from_str(&env, "SOL"),
            quote: String::from_str(&env, "USDC"),
        };

        let order_id = create_twap_order(&env, user.clone(), pair.clone(), 5000, 50, Some(5)).unwrap();
        
        // Fast forward beyond the entire duration (50 mins = 3000 seconds)
        env.ledger().set_timestamp(1_000 + 3001);
        let executed_ids = execute_twap_segments(&env);
        
        assert_eq!(executed_ids.len(), 5);

        let twap = get_twap_order(&env, order_id).unwrap();
        assert_eq!(twap.segments_executed, 5);
        assert_eq!(twap.filled_amount, 5000);
        assert_eq!(twap.status, TWAPStatus::Complete);
    }
}
