//! Historical price storage and TWAP calculation

use crate::errors::OracleError;
use common::AssetPair;
use soroban_sdk::Env;

const BUCKET_SIZE: u64 = 300; // 5 minutes
const MAX_BUCKETS: u64 = 2016; // 7 days at 5-min intervals
const DAY_IN_LEDGERS: u32 = 17280; // ~24 hours

/// Store price snapshot at 5-minute intervals
pub fn store_price(env: &Env, pair: &AssetPair, price: i128) {
    let timestamp = env.ledger().timestamp();
    let bucket = timestamp / BUCKET_SIZE;

    let key = (pair.clone(), bucket);
    env.storage().persistent().set(&key, &price);
    env.storage()
        .persistent()
        .extend_ttl(&key, DAY_IN_LEDGERS * 7, DAY_IN_LEDGERS * 7);

    prune_old_data(env, pair, bucket);
}

/// Get historical price for specific timestamp
pub fn get_historical_price(env: &Env, pair: &AssetPair, timestamp: u64) -> Option<i128> {
    let current_time = env.ledger().timestamp();
    if timestamp > current_time {
        return None; // Reject future timestamps
    }

    let bucket = timestamp / BUCKET_SIZE;
    let key = (pair.clone(), bucket);
    env.storage().persistent().get(&key)
}

/// Calculate TWAP over window (1h, 24h, 7d)
pub fn calculate_twap(
    env: &Env,
    pair: &AssetPair,
    window_seconds: u64,
) -> Result<i128, OracleError> {
    let end_time = env.ledger().timestamp();
    let start_time = end_time.saturating_sub(window_seconds);

    let mut sum: i128 = 0;
    let mut count: u64 = 0;

    let start_bucket = start_time / BUCKET_SIZE;
    let end_bucket = end_time / BUCKET_SIZE;

    for bucket in start_bucket..=end_bucket {
        let key = (pair.clone(), bucket);
        if let Some(price) = env.storage().persistent().get::<_, i128>(&key) {
            sum = sum.saturating_add(price);
            count += 1;
        }
    }

    if count == 0 {
        return Err(OracleError::InsufficientHistoricalData);
    }

    Ok(sum / count as i128)
}

/// Prune data older than 7 days (circular buffer)
fn prune_old_data(env: &Env, pair: &AssetPair, current_bucket: u64) {
    if current_bucket <= MAX_BUCKETS {
        return;
    }

    let oldest_bucket = current_bucket - MAX_BUCKETS;
    let key = (pair.clone(), oldest_bucket);
    env.storage().persistent().remove(&key);
}

/// Query price deviation from TWAP (basis points)
pub fn get_twap_deviation(
    env: &Env,
    pair: &AssetPair,
    current_price: i128,
    window: u64,
) -> Result<i128, OracleError> {
    let twap = calculate_twap(env, pair, window)?;
    if twap == 0 {
        return Err(OracleError::InvalidPrice);
    }
    let deviation = ((current_price - twap).abs() * 10000) / twap;
    Ok(deviation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::Asset;
    use soroban_sdk::{testutils::Ledger, Address, Env, String};

    fn test_pair(env: &Env) -> AssetPair {
        AssetPair {
            base: Asset {
                code: String::from_str(env, "USDC"),
                issuer: Some(Address::generate(env)),
            },
            quote: Asset {
                code: String::from_str(env, "XLM"),
                issuer: None,
            },
        }
    }

    #[test]
    fn test_store_and_retrieve() {
        let env = Env::default();
        let pair = test_pair(&env);

        store_price(&env, &pair, 10_000_000);
        let price = get_historical_price(&env, &pair, env.ledger().timestamp());
        assert_eq!(price, Some(10_000_000));
    }

    #[test]
    fn test_twap_calculation() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1000);
        let pair = test_pair(&env);

        store_price(&env, &pair, 10_000_000);

        env.ledger().with_mut(|li| li.timestamp = 1300);
        store_price(&env, &pair, 11_000_000);

        env.ledger().with_mut(|li| li.timestamp = 1600);
        store_price(&env, &pair, 12_000_000);

        let twap = calculate_twap(&env, &pair, 1000).unwrap();
        assert_eq!(twap, 11_000_000);
    }

    #[test]
    fn test_insufficient_data() {
        let env = Env::default();
        let pair = test_pair(&env);

        let result = calculate_twap(&env, &pair, 3600);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), OracleError::InsufficientHistoricalData);
    }

    #[test]
    fn test_deviation_calculation() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1000);
        let pair = test_pair(&env);

        store_price(&env, &pair, 10_000_000);
        env.ledger().with_mut(|li| li.timestamp = 1300);
        store_price(&env, &pair, 10_000_000);

        let deviation = get_twap_deviation(&env, &pair, 11_000_000, 600).unwrap();
        assert_eq!(deviation, 1000); // 10% deviation
    }

    #[test]
    fn test_twap_1h_window() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 0);
        let pair = test_pair(&env);

        // Store prices every 5 minutes for 1 hour
        for i in 0..13 {
            env.ledger().with_mut(|li| li.timestamp = i * 300);
            store_price(&env, &pair, 10_000_000 + (i as i128 * 100_000));
        }

        env.ledger().with_mut(|li| li.timestamp = 3600);
        let twap = calculate_twap(&env, &pair, 3600).unwrap();
        assert!(twap >= 10_000_000 && twap <= 11_200_000);
    }

    #[test]
    fn test_twap_24h_window() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 0);
        let pair = test_pair(&env);

        // Store prices every hour for 24 hours
        for i in 0..25 {
            env.ledger().with_mut(|li| li.timestamp = i * 3600);
            store_price(&env, &pair, 10_000_000);
        }

        env.ledger().with_mut(|li| li.timestamp = 86400);
        let twap = calculate_twap(&env, &pair, 86400).unwrap();
        assert_eq!(twap, 10_000_000);
    }

    #[test]
    fn test_twap_7d_window() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 0);
        let pair = test_pair(&env);

        // Store prices every 12 hours for 7 days
        for i in 0..15 {
            env.ledger().with_mut(|li| li.timestamp = i * 43200);
            store_price(&env, &pair, 10_000_000 + (i as i128 * 50_000));
        }

        env.ledger().with_mut(|li| li.timestamp = 604800);
        let twap = calculate_twap(&env, &pair, 604800).unwrap();
        assert!(twap >= 10_000_000 && twap <= 10_700_000);
    }

    #[test]
    fn test_future_timestamp_rejected() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1000);
        let pair = test_pair(&env);

        store_price(&env, &pair, 10_000_000);

        // Try to query future timestamp
        let result = get_historical_price(&env, &pair, 2000);
        assert_eq!(result, None);
    }

    #[test]
    fn test_data_pruning_after_7_days() {
        let env = Env::default();
        let pair = test_pair(&env);

        // Store price at day 0
        env.ledger().with_mut(|li| li.timestamp = 0);
        store_price(&env, &pair, 10_000_000);

        // Move to day 8 and store new price (should trigger pruning)
        env.ledger().with_mut(|li| li.timestamp = 8 * 86400);
        store_price(&env, &pair, 11_000_000);

        // Old data should be pruned
        let old_price = get_historical_price(&env, &pair, 0);
        assert_eq!(old_price, None);

        // New data should exist
        let new_price = get_historical_price(&env, &pair, 8 * 86400);
        assert_eq!(new_price, Some(11_000_000));
    }

    #[test]
    fn test_missing_data_points_in_window() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 0);
        let pair = test_pair(&env);

        // Store sparse data points
        store_price(&env, &pair, 10_000_000);
        env.ledger().with_mut(|li| li.timestamp = 1800); // 30 min gap
        store_price(&env, &pair, 12_000_000);

        env.ledger().with_mut(|li| li.timestamp = 3600);
        let twap = calculate_twap(&env, &pair, 3600).unwrap();
        // Should average only available data points
        assert_eq!(twap, 11_000_000);
    }

    #[test]
    fn test_storage_overflow_handling() {
        let env = Env::default();
        let pair = test_pair(&env);

        // Store max buckets + 1
        for i in 0..(MAX_BUCKETS + 10) {
            env.ledger().with_mut(|li| li.timestamp = i * BUCKET_SIZE);
            store_price(&env, &pair, 10_000_000);
        }

        // Oldest data should be pruned
        let oldest = get_historical_price(&env, &pair, 0);
        assert_eq!(oldest, None);
    }

    #[test]
    fn test_manipulation_detection() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 0);
        let pair = test_pair(&env);

        // Store stable prices
        for i in 0..10 {
            env.ledger().with_mut(|li| li.timestamp = i * 300);
            store_price(&env, &pair, 10_000_000);
        }

        // Check deviation with manipulated price
        env.ledger().with_mut(|li| li.timestamp = 3000);
        let deviation = get_twap_deviation(&env, &pair, 11_500_000, 3000).unwrap();
        assert!(deviation > 1000); // >10% deviation indicates manipulation
    }

    #[test]
    fn test_zero_price_twap_error() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1000);
        let pair = test_pair(&env);

        store_price(&env, &pair, 0);

        let result = get_twap_deviation(&env, &pair, 10_000_000, 600);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), OracleError::InvalidPrice);
    }

    #[test]
    fn test_multiple_pairs_isolation() {
        let env = Env::default();
        let pair1 = test_pair(&env);
        let pair2 = AssetPair {
            base: Asset {
                code: String::from_str(&env, "BTC"),
                issuer: Some(Address::generate(&env)),
            },
            quote: Asset {
                code: String::from_str(&env, "XLM"),
                issuer: None,
            },
        };

        env.ledger().with_mut(|li| li.timestamp = 1000);
        store_price(&env, &pair1, 10_000_000);
        store_price(&env, &pair2, 50_000_000);

        let price1 = get_historical_price(&env, &pair1, 1000);
        let price2 = get_historical_price(&env, &pair2, 1000);

        assert_eq!(price1, Some(10_000_000));
        assert_eq!(price2, Some(50_000_000));
    }
}
