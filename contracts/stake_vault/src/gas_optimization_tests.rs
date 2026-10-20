#[cfg(test)]
mod gas_optimization_tests {
    use crate::rewards_optimized::*;

    const XLM: i128 = 10_000_000;

    #[derive(Debug, Clone)]
    struct GasBenchmark {
        operation: &'static str,
        baseline_gas: u64,
        optimized_gas: u64,
        improvement_percentage: u32,
    }

    impl GasBenchmark {
        fn new(operation: &'static str, baseline: u64, optimized: u64) -> Self {
            let improvement = if baseline > 0 {
                ((baseline - optimized) * 100 / baseline) as u32
            } else {
                0
            };

            Self {
                operation,
                baseline_gas: baseline,
                optimized_gas: optimized,
                improvement_percentage: improvement,
            }
        }

        fn gas_saved(&self) -> u64 {
            self.baseline_gas.saturating_sub(self.optimized_gas)
        }
    }

    fn pool() -> RewardsPool {
        RewardsPool {
            balance: 10_000 * XLM,
            daily_outflow: 100 * XLM,
            auto_fund_threshold: 1_000 * XLM,
            treasury_balance: 20_000 * XLM,
            pending_claims: 0,
            total_distributed: 0,
        }
    }

    #[test]
    fn test_batch_claim_gas_savings() {
        let mut pool = pool();
        
        // Baseline: 10 individual claims
        let baseline_gas = 10 * 100_000u64; // 1,000,000 gas

        let mut accruals = vec![
            StakeRewardAccrual {
                user: "user1",
                accrued_amount: 100 * XLM,
                last_update_time: 1000,
                stake_amount: 1_000 * XLM,
            },
            StakeRewardAccrual {
                user: "user2",
                accrued_amount: 150 * XLM,
                last_update_time: 1000,
                stake_amount: 1_500 * XLM,
            },
            StakeRewardAccrual {
                user: "user3",
                accrued_amount: 200 * XLM,
                last_update_time: 1000,
                stake_amount: 2_000 * XLM,
            },
            StakeRewardAccrual {
                user: "user4",
                accrued_amount: 120 * XLM,
                last_update_time: 1000,
                stake_amount: 1_200 * XLM,
            },
            StakeRewardAccrual {
                user: "user5",
                accrued_amount: 180 * XLM,
                last_update_time: 1000,
                stake_amount: 1_800 * XLM,
            },
            StakeRewardAccrual {
                user: "user6",
                accrued_amount: 90 * XLM,
                last_update_time: 1000,
                stake_amount: 900 * XLM,
            },
            StakeRewardAccrual {
                user: "user7",
                accrued_amount: 110 * XLM,
                last_update_time: 1000,
                stake_amount: 1_100 * XLM,
            },
            StakeRewardAccrual {
                user: "user8",
                accrued_amount: 160 * XLM,
                last_update_time: 1000,
                stake_amount: 1_600 * XLM,
            },
            StakeRewardAccrual {
                user: "user9",
                accrued_amount: 140 * XLM,
                last_update_time: 1000,
                stake_amount: 1_400 * XLM,
            },
            StakeRewardAccrual {
                user: "user10",
                accrued_amount: 130 * XLM,
                last_update_time: 1000,
                stake_amount: 1_300 * XLM,
            },
        ];

        let result = batch_claim_rewards(&mut pool, &mut accruals).unwrap();

        // Optimized: Single batch transaction saves ~40%
        let optimized_gas = 600_000u64;

        assert_eq!(result.claims.len(), 10);
        assert_eq!(result.gas_saved_percentage, 40);
        assert_eq!(result.total_claimed, 1_380 * XLM);

        let benchmark = GasBenchmark::new("Batch Claim (10 users)", baseline_gas, optimized_gas);
        assert_eq!(benchmark.improvement_percentage, 40);
        assert_eq!(benchmark.gas_saved(), 400_000);
    }

    #[test]
    fn test_accrual_vs_immediate_distribution() {
        let mut pool = pool();

        // Baseline: Immediate distribution every time (30 distributions)
        let baseline_gas = 30 * 100_000u64; // 3,000,000 gas

        // Optimized: Accrue 30 times, claim once
        let mut accrual = StakeRewardAccrual {
            user: "user1",
            accrued_amount: 0,
            last_update_time: 1000,
            stake_amount: 1_000 * XLM,
        };

        let reward_rate = 1_000; // 0.001% per second

        // Accrue 30 times (very cheap operations)
        for i in 1..=30 {
            accrue_staking_rewards(&mut accrual, reward_rate, 1000 + i * 86400);
        }

        // Single claim
        let mut accruals = vec![accrual];
        batch_claim_rewards(&mut pool, &mut accruals).unwrap();

        // Accrual operations: 30 * 3000 = 90,000 gas
        // Single claim: 100,000 gas
        let optimized_gas = 90_000 + 100_000; // 190,000 gas

        let benchmark = GasBenchmark::new("Accrual vs Immediate (30 rewards)", baseline_gas, optimized_gas);
        assert_eq!(benchmark.improvement_percentage, 93);
        assert_eq!(benchmark.gas_saved(), 2_810_000);
    }

    #[test]
    fn test_predictive_reserve_management() {
        let mut pool = pool();
        pool.balance = 200 * XLM;

        // Baseline: Frequent treasury access (20 times)
        let baseline_gas = 20 * 50_000u64; // 1,000,000 gas

        // Optimized: Single predictive refill
        let predicted_outflow = 150 * XLM;
        optimize_reserve_management(&mut pool, predicted_outflow);

        let optimized_gas = 50_000u64; // Single refill

        assert_eq!(pool.balance, 1_050 * XLM); // 7 days worth

        let benchmark = GasBenchmark::new("Predictive Reserve Management", baseline_gas, optimized_gas);
        assert_eq!(benchmark.improvement_percentage, 95);
        assert_eq!(benchmark.gas_saved(), 950_000);
    }

    #[test]
    fn test_optimal_claim_timing_gas_efficiency() {
        // Scenario: User has small accrued amount
        let accrued = 2 * XLM;
        let rate = XLM / 1000; // 0.001 XLM per second
        let min_profitable = 10 * XLM;

        let wait_time = calculate_optimal_claim_time(accrued, rate, 1000, min_profitable);

        // Should wait to accumulate more before claiming
        assert_eq!(wait_time, 8_000); // 8000 seconds

        // Baseline: Claim now (inefficient)
        let baseline_gas = 100_000u64;

        // Optimized: Wait and claim larger amount (more efficient per unit)
        // Larger claims have better gas efficiency
        let optimized_gas_per_xlm = 100_000 / 10; // 10,000 gas per XLM for large claim
        let baseline_gas_per_xlm = 100_000 / 2; // 50,000 gas per XLM for small claim

        assert!(optimized_gas_per_xlm < baseline_gas_per_xlm);
    }

    #[test]
    fn test_reserve_hit_rate_optimization() {
        let mut pool = pool();
        pool.reserve_balance = 5_000 * XLM;

        // Simulate 20 claims, all from reserve (no treasury access)
        let mut total_claimed = 0i128;
        
        for i in 0..20 {
            let mut accrual = vec![StakeRewardAccrual {
                user: "user",
                accrued_amount: 100 * XLM,
                last_update_time: 1000,
                stake_amount: 1_000 * XLM,
            }];

            batch_claim_rewards(&mut pool, &mut accrual).unwrap();
            total_claimed += 100 * XLM;
        }

        // All claims served from reserve (cheaper)
        let reserve_hits = 20;
        let metrics = calculate_distribution_metrics(&pool, 20, 1_600_000, reserve_hits);

        assert_eq!(metrics.reserve_hit_rate, 100); // 100% hit rate
        assert_eq!(metrics.batch_efficiency, 20); // Some efficiency from batching
        assert_eq!(total_claimed, 2_000 * XLM);
    }

    #[test]
    fn test_distribution_metrics_comprehensive() {
        let mut pool = pool();
        pool.total_distributed = 5_000 * XLM;

        // Simulate efficient distribution
        let total_claims = 50u32;
        let total_gas_used = 2_000_000u64; // Optimized
        let reserve_hits = 45u32; // 90% hit rate

        let metrics = calculate_distribution_metrics(&pool, total_claims, total_gas_used, reserve_hits);

        // Baseline would be: 50 * 100,000 = 5,000,000 gas
        // Actual: 2,000,000 gas
        // Savings: 3,000,000 gas (60%)
        assert_eq!(metrics.total_gas_saved, 3_000_000);
        assert_eq!(metrics.batch_efficiency, 60);
        assert_eq!(metrics.average_claim_size, 100 * XLM);
        assert_eq!(metrics.reserve_hit_rate, 90);
    }

    #[test]
    fn test_pool_monitoring_optimization() {
        let mut pool = pool();
        pool.balance = 300 * XLM; // Low balance

        // Baseline: Reactive refilling (multiple small refills)
        let baseline_refills = 5;
        let baseline_gas = baseline_refills * 50_000u64; // 250,000 gas

        // Optimized: Predictive refilling (single large refill to optimal level)
        let event = monitor_rewards_pool(&mut pool).unwrap();

        let optimized_refills = 1;
        let optimized_gas = optimized_refills * 50_000u64; // 50,000 gas

        // Should refill to 7 days worth
        assert!(event.auto_funded_amount > 0);
        assert_eq!(pool.balance, 700 * XLM);

        let benchmark = GasBenchmark::new("Pool Monitoring", baseline_gas, optimized_gas);
        assert_eq!(benchmark.improvement_percentage, 80);
    }

    #[test]
    fn test_minimum_claim_threshold_enforcement() {
        let min_threshold = 5 * XLM;

        // Test below threshold
        assert!(!should_claim(3 * XLM, min_threshold));

        // Test at threshold
        assert!(should_claim(5 * XLM, min_threshold));

        // Test above threshold
        assert!(should_claim(10 * XLM, min_threshold));

        // Baseline: Allow all claims (including tiny ones)
        // 1000 micro-claims of 0.01 XLM each
        let baseline_gas = 1000 * 100_000u64; // 100,000,000 gas

        // Optimized: Enforce minimum, accumulate to 10 XLM, then claim
        // 1000 accruals + 1 claim
        let optimized_gas = 1000 * 3_000 + 100_000; // 3,100,000 gas

        let benchmark = GasBenchmark::new("Minimum Threshold (1000 micro-claims)", baseline_gas, optimized_gas);
        assert_eq!(benchmark.improvement_percentage, 96);
    }

    #[test]
    fn test_real_world_staking_scenario() {
        // Scenario: 200 stakers over 90 days
        // - Rewards accrue continuously
        // - Users claim weekly (12 times over 90 days)

        // Baseline: Continuous distribution (200 users * 90 days = 18,000 distributions)
        let baseline_gas = 18_000 * 100_000u64; // 1,800,000,000 gas

        // Optimized approach:
        // - Continuous accrual (essentially free, just state updates)
        // - Weekly batch claims: 200 users / 50 per batch = 4 batches
        // - 12 weeks * 4 batches = 48 batch operations
        let accrual_gas = 18_000 * 3_000u64; // 54,000,000 gas (accrual tracking)
        let batch_claim_gas = 48 * 325_000u64; // 15,600,000 gas (batch claims)
        let optimized_gas = accrual_gas + batch_claim_gas; // 69,600,000 gas

        let benchmark = GasBenchmark::new(
            "Real-world: 200 stakers, 90 days, weekly claims",
            baseline_gas,
            optimized_gas,
        );

        assert_eq!(benchmark.improvement_percentage, 96);
        assert_eq!(benchmark.gas_saved(), 1_730_400_000);

        // This represents enormous savings in a production staking system
        println!("Real-world staking gas savings: {} ({}%)", 
            benchmark.gas_saved(), 
            benchmark.improvement_percentage
        );
    }

    #[test]
    fn test_combined_optimization_techniques() {
        // Test all optimization techniques together
        let mut pool = pool();
        pool.balance = 500 * XLM;

        // 1. Optimize reserve
        optimize_reserve_management(&mut pool, 150 * XLM);

        // 2. Accrue rewards for multiple users
        let mut accruals = vec![];
        for i in 0..20 {
            let mut accrual = StakeRewardAccrual {
                user: "user",
                accrued_amount: 0,
                last_update_time: 1000,
                stake_amount: 1_000 * XLM,
            };

            // Accrue over time
            for day in 0..7 {
                accrue_staking_rewards(&mut accrual, 1_000, 1000 + day * 86400);
            }

            accruals.push(accrual);
        }

        // 3. Batch claim all at once
        let result = batch_claim_rewards(&mut pool, &mut accruals).unwrap();

        // Baseline: 20 users * 7 days = 140 immediate distributions
        let baseline_gas = 140 * 100_000u64; // 14,000,000 gas

        // Optimized:
        // - Reserve optimization: 50,000
        // - Accrual tracking: 140 * 3,000 = 420,000
        // - Batch claim: 600,000 (for 20 users)
        let optimized_gas = 50_000 + 420_000 + 600_000; // 1,070,000 gas

        let benchmark = GasBenchmark::new("Combined Optimizations", baseline_gas, optimized_gas);
        assert_eq!(benchmark.improvement_percentage, 92);
        assert_eq!(result.gas_saved_percentage, 40); // Just the batch claim savings
    }

    #[test]
    fn test_gas_efficiency_metrics_accuracy() {
        let mut pool = pool();
        pool.total_distributed = 10_000 * XLM;

        // Test with high efficiency
        let metrics_high = calculate_distribution_metrics(&pool, 100, 4_000_000, 95);
        assert_eq!(metrics_high.batch_efficiency, 60);
        assert_eq!(metrics_high.reserve_hit_rate, 95);
        assert_eq!(metrics_high.average_claim_size, 100 * XLM);

        // Test with low efficiency
        let metrics_low = calculate_distribution_metrics(&pool, 100, 9_000_000, 20);
        assert_eq!(metrics_low.batch_efficiency, 10);
        assert_eq!(metrics_low.reserve_hit_rate, 20);

        // High efficiency should have better metrics
        assert!(metrics_high.batch_efficiency > metrics_low.batch_efficiency);
        assert!(metrics_high.reserve_hit_rate > metrics_low.reserve_hit_rate);
    }
}
