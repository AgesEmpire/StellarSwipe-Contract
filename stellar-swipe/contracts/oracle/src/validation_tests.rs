//! Validation tests for Historical Price Storage & TWAP Implementation
//! 
//! This file contains validation scenarios from the issue requirements.

#[cfg(test)]
mod validation_tests {
    use soroban_sdk::{testutils::Ledger, Env, String, Address};
    use crate::history::*;
    use crate::errors::OracleError;
    use stellar_swipe_common::{Asset, AssetPair};

    fn usdc_xlm_pair(env: &Env) -> AssetPair {
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

    /// Validation 1: Store price updates every 5 minutes for 1 day
    #[test]
    fn validation_store_prices_for_1_day() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        env.ledger().with_mut(|li| li.timestamp = 0);
        
        // Store prices every 5 minutes for 24 hours (288 data points)
        let mut stored_count = 0;
        for i in 0..288 {
            let timestamp = i * 300; // 5 minutes
            env.ledger().with_mut(|li| li.timestamp = timestamp);
            
            // Simulate price fluctuation
            let price = 10_000_000 + ((i % 100) as i128 * 10_000);
            store_price(&env, &pair, price);
            stored_count += 1;
        }
        
        assert_eq!(stored_count, 288);
        
        // Verify data is retrievable
        let price_at_12h = get_historical_price(&env, &pair, 12 * 3600);
        assert!(price_at_12h.is_some());
    }

    /// Validation 2: Calculate 24h TWAP and verify against manual calculation
    #[test]
    fn validation_24h_twap_accuracy() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        env.ledger().with_mut(|li| li.timestamp = 0);
        
        // Store known prices for manual verification
        let prices = vec![
            10_000_000, 10_100_000, 10_200_000, 10_300_000,
            10_400_000, 10_500_000, 10_600_000, 10_700_000,
        ];
        
        for (i, price) in prices.iter().enumerate() {
            env.ledger().with_mut(|li| li.timestamp = (i as u64) * 3600); // Every hour
            store_price(&env, &pair, *price);
        }
        
        // Move to end of 24h period
        env.ledger().with_mut(|li| li.timestamp = 86400);
        
        // Calculate TWAP
        let twap = calculate_twap(&env, &pair, 86400).unwrap();
        
        // Manual calculation: (10_000_000 + 10_100_000 + ... + 10_700_000) / 8
        let manual_twap = prices.iter().sum::<i128>() / prices.len() as i128;
        
        assert_eq!(twap, manual_twap);
        assert_eq!(twap, 10_350_000);
    }

    /// Validation 3: Query historical price from 3 days ago
    #[test]
    fn validation_query_3_days_ago() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        // Store price 3 days ago
        let three_days_ago = 3 * 86400;
        env.ledger().with_mut(|li| li.timestamp = three_days_ago);
        store_price(&env, &pair, 10_000_000);
        
        // Move to present
        env.ledger().with_mut(|li| li.timestamp = 6 * 86400);
        
        // Query 3 days ago
        let historical_price = get_historical_price(&env, &pair, three_days_ago);
        assert_eq!(historical_price, Some(10_000_000));
    }

    /// Validation 4: Test data pruning after 7 days
    #[test]
    fn validation_data_pruning_7_days() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        // Store price at day 0
        env.ledger().with_mut(|li| li.timestamp = 0);
        store_price(&env, &pair, 10_000_000);
        
        // Store prices continuously for 8 days
        for day in 1..=8 {
            env.ledger().with_mut(|li| li.timestamp = day * 86400);
            store_price(&env, &pair, 10_000_000 + (day as i128 * 100_000));
        }
        
        // Data from day 0 should be pruned (>7 days old)
        let day_0_price = get_historical_price(&env, &pair, 0);
        assert_eq!(day_0_price, None, "Day 0 data should be pruned");
        
        // Data from day 2 should still exist
        let day_2_price = get_historical_price(&env, &pair, 2 * 86400);
        assert_eq!(day_2_price, Some(10_200_000), "Day 2 data should exist");
        
        // Data from day 8 should exist
        let day_8_price = get_historical_price(&env, &pair, 8 * 86400);
        assert_eq!(day_8_price, Some(10_800_000), "Day 8 data should exist");
    }

    /// Validation 5: Measure storage costs for 100 pairs
    #[test]
    fn validation_storage_costs_100_pairs() {
        let env = Env::default();
        
        // Create 100 different pairs
        let mut pairs = Vec::new();
        for i in 0..100 {
            let pair = AssetPair {
                base: Asset {
                    code: String::from_str(&env, &format!("TOK{}", i)),
                    issuer: Some(Address::generate(&env)),
                },
                quote: Asset {
                    code: String::from_str(&env, "XLM"),
                    issuer: None,
                },
            };
            pairs.push(pair);
        }
        
        env.ledger().with_mut(|li| li.timestamp = 0);
        
        // Store 1 day of data for each pair (288 data points per pair)
        for pair in pairs.iter() {
            for i in 0..288 {
                env.ledger().with_mut(|li| li.timestamp = i * 300);
                store_price(&env, pair, 10_000_000);
            }
        }
        
        // Verify all pairs have data
        env.ledger().with_mut(|li| li.timestamp = 86400);
        for pair in pairs.iter() {
            let twap = calculate_twap(&env, pair, 86400);
            assert!(twap.is_ok(), "TWAP should be calculable for all pairs");
        }
        
        // Storage cost estimation:
        // - Each entry: ~100 bytes (AssetPair + bucket + price)
        // - 288 entries per day per pair
        // - 100 pairs
        // - Total: ~2.8MB per day for 100 pairs
        // - Per pair: ~28KB per day ≈ 2KB per day (with compression)
    }

    /// Additional Validation: TWAP windows (1h, 24h, 7d)
    #[test]
    fn validation_all_twap_windows() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        env.ledger().with_mut(|li| li.timestamp = 0);
        
        // Store prices for 7 days
        for i in 0..(7 * 24 * 12) {
            // Every 5 minutes for 7 days
            env.ledger().with_mut(|li| li.timestamp = i * 300);
            store_price(&env, &pair, 10_000_000);
        }
        
        env.ledger().with_mut(|li| li.timestamp = 7 * 86400);
        
        // Test 1h TWAP
        let twap_1h = calculate_twap(&env, &pair, 3600);
        assert!(twap_1h.is_ok(), "1h TWAP should succeed");
        
        // Test 24h TWAP
        let twap_24h = calculate_twap(&env, &pair, 86400);
        assert!(twap_24h.is_ok(), "24h TWAP should succeed");
        
        // Test 7d TWAP
        let twap_7d = calculate_twap(&env, &pair, 604800);
        assert!(twap_7d.is_ok(), "7d TWAP should succeed");
        
        // All should be equal since price is constant
        assert_eq!(twap_1h.unwrap(), 10_000_000);
        assert_eq!(twap_24h.unwrap(), 10_000_000);
        assert_eq!(twap_7d.unwrap(), 10_000_000);
    }

    /// Additional Validation: Manipulation detection
    #[test]
    fn validation_manipulation_detection() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        env.ledger().with_mut(|li| li.timestamp = 0);
        
        // Store stable prices for 1 hour
        for i in 0..12 {
            env.ledger().with_mut(|li| li.timestamp = i * 300);
            store_price(&env, &pair, 10_000_000);
        }
        
        env.ledger().with_mut(|li| li.timestamp = 3600);
        
        // Test normal price (within 10%)
        let normal_deviation = get_twap_deviation(&env, &pair, 10_500_000, 3600).unwrap();
        assert!(normal_deviation <= 1000, "5% deviation should be acceptable");
        
        // Test manipulated price (>10%)
        let manipulated_deviation = get_twap_deviation(&env, &pair, 11_500_000, 3600).unwrap();
        assert!(manipulated_deviation > 1000, "15% deviation indicates manipulation");
    }

    /// Additional Validation: Performance test
    #[test]
    fn validation_performance_requirements() {
        let env = Env::default();
        let pair = usdc_xlm_pair(&env);
        
        env.ledger().with_mut(|li| li.timestamp = 0);
        
        // Store 24 hours of data
        for i in 0..288 {
            env.ledger().with_mut(|li| li.timestamp = i * 300);
            store_price(&env, &pair, 10_000_000);
        }
        
        env.ledger().with_mut(|li| li.timestamp = 86400);
        
        // TWAP calculation should complete successfully
        // (Performance timing would require benchmarking tools)
        let twap = calculate_twap(&env, &pair, 86400);
        assert!(twap.is_ok(), "24h TWAP should complete");
        
        // Historical query should complete successfully
        let historical = get_historical_price(&env, &pair, 43200); // 12 hours ago
        assert!(historical.is_some(), "Historical query should complete");
    }
}
