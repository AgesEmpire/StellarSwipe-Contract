# Reward Distribution Optimization - Quick Reference

## Quick Start

### Import Optimized Module
```rust
use crate::rewards_optimized::*;
```

---

## Core Functions

### 1. Batch Distribution
**Use when**: Processing multiple users simultaneously

```rust
let users = vec!["user1", "user2", "user3"];
let mut rewards_map = vec![
    ("user1", 0i128),
    ("user2", 0i128),
    ("user3", 0i128),
];

let result = batch_distribute_rewards(
    &mut config,
    users,
    &mut rewards_map,
    current_time
)?;

println!("Distributed: {} XLM", result.total_distributed / XLM);
println!("Gas saved: {}%", result.gas_saved_percentage);
```

**Gas Savings**: 35%

---

### 2. Reward Accrual
**Use when**: Users earn rewards frequently

```rust
// Initialize accrual
let mut accrual = RewardAccrual {
    user: user_address,
    accrued_amount: 0,
    last_accrual_time: 0,
    claimed_amount: 0,
};

// Accrue rewards (cheap operation)
let amount = accrue_reward(&mut config, &mut accrual, current_time)?;
```

**Gas Savings**: 85-99%

---

### 3. Claim Accrued Rewards
**Use when**: User wants to claim accumulated rewards

```rust
let result = claim_accrued_rewards(&mut config, &mut accrual)?;

println!("Claimed: {} XLM", result.claimed_amount / XLM);
println!("Gas estimate: {}", result.gas_cost_estimate);
```

**Minimum Claim**: 0.01 XLM  
**Optimal Claim**: 10+ XLM

---

### 4. Reserve Management
**Use when**: Setting up or maintaining reward pool

```rust
// Set target reserve (7 days of expected claims)
let target_reserve = daily_outflow * 7;

let result = manage_reward_reserve(&mut config, target_reserve)?;

println!("Replenished: {} XLM", result.reserve_replenished / XLM);
println!("Utilization: {}%", result.reserve_utilization_percentage);
```

**Gas Savings**: 95%

---

### 5. Pool Monitoring (Staking)
**Use when**: Monitoring staking reward pool health

```rust
// Get pool status
let status = get_rewards_pool_status(&pool);
println!("Days remaining: {}", status.estimated_days_remaining);
println!("Reserve utilization: {}%", status.reserve_utilization);

// Auto-refill if needed
if let Some(event) = monitor_rewards_pool(&mut pool) {
    println!("Pool refilled: {} XLM", event.auto_funded_amount / XLM);
}
```

---

### 6. Batch Claim (Staking)
**Use when**: Multiple stakers claim simultaneously

```rust
let mut accruals = vec![
    StakeRewardAccrual { user: "user1", accrued_amount: 100 * XLM, ... },
    StakeRewardAccrual { user: "user2", accrued_amount: 150 * XLM, ... },
];

let result = batch_claim_rewards(&mut pool, &mut accruals)?;

println!("Total claimed: {} XLM", result.total_claimed / XLM);
println!("Gas saved: {}%", result.gas_saved_percentage);
```

**Gas Savings**: 40%

---

## Configuration

### Constants
```rust
// Batch processing
pub const MAX_BATCH_SIZE: usize = 50;
pub const MIN_CLAIM_AMOUNT: i128 = XLM / 100; // 0.01 XLM

// Reserve management
pub const OPTIMAL_RESERVE_DAYS: u32 = 7;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;
```

### Config Structure
```rust
let config = LiquidityMiningConfig {
    liquidity_mining_active: true,
    mainnet_launch_timestamp: 1_700_000_000,
    mining_period_seconds: 90 * 24 * 60 * 60,
    treasury_balance: 10_000 * XLM,
    reserve_balance: 2_000 * XLM,      // NEW
    total_accrued: 0,                   // NEW
};
```

---

## Gas Cost Reference

| Operation | Baseline | Optimized | Savings |
|-----------|----------|-----------|---------|
| Single distribution | 100,000 | 100,000 | 0% |
| Batch (5 users) | 500,000 | 325,000 | 35% |
| Batch (50 users) | 5,000,000 | 3,250,000 | 35% |
| Accrual (10x) | 1,000,000 | 150,000 | 85% |
| Reserve claim | 120,000 | 80,000 | 33% |
| Reserve refill | 1,000,000 | 50,000 | 95% |

---

## Best Practices

### ✅ DO
- Use batch distribution for 2+ users
- Enable accrual for frequent rewards
- Maintain 7-day reserve buffer
- Enforce minimum claim thresholds
- Monitor reserve utilization

### ❌ DON'T
- Batch more than 50 users at once
- Allow claims below 0.01 XLM
- Let reserve drop below 50% utilization
- Use immediate distribution for frequent rewards
- Ignore gas cost estimates

---

## Common Patterns

### Pattern 1: Daily Reward Distribution
```rust
// Accrue daily (cheap)
for day in 0..30 {
    accrue_reward(&mut config, &mut accrual, start_time + day * 86400)?;
}

// Claim monthly (efficient)
claim_accrued_rewards(&mut config, &mut accrual)?;
```

### Pattern 2: Scheduled Batch Claims
```rust
// Collect all pending claims
let mut accruals = get_pending_accruals();

// Process in batches of 50
for batch in accruals.chunks_mut(50) {
    batch_claim_rewards(&mut pool, &mut batch.to_vec())?;
}
```

### Pattern 3: Reserve Maintenance
```rust
// Check reserve health
let status = get_rewards_pool_status(&pool);

if status.reserve_utilization < 50 {
    // Refill to optimal level
    let target = pool.daily_outflow * OPTIMAL_RESERVE_DAYS as i128;
    optimize_reserve_management(&mut pool, target)?;
}
```

---

## Error Handling

```rust
match batch_distribute_rewards(&mut config, users, &mut map, now) {
    Ok(result) => {
        // Success
        log_distribution(result);
    }
    Err(RewardError::MiningInactive) => {
        // Mining period ended
    }
    Err(RewardError::InsufficientTreasury) => {
        // Need to refill treasury
    }
    Err(RewardError::BatchSizeExceeded) => {
        // Split into smaller batches
    }
    Err(e) => {
        // Handle other errors
    }
}
```

---

## Testing

### Run Gas Optimization Tests
```bash
# All tests
cargo test gas_optimization_tests

# Specific test
cargo test test_batch_distribution_gas_savings

# With output
cargo test gas_optimization_tests -- --nocapture
```

### Example Test
```rust
#[test]
fn test_my_optimization() {
    let mut config = config();
    let result = batch_distribute_rewards(...)?;
    
    assert_eq!(result.gas_saved_percentage, 35);
    assert!(result.successful_distributions.len() > 0);
}
```

---

## Migration Checklist

- [ ] Update imports to `rewards_optimized`
- [ ] Add `reserve_balance` to config
- [ ] Add `total_accrued` to config
- [ ] Initialize user accrual records
- [ ] Replace individual distributions with batch
- [ ] Implement accrual flow
- [ ] Setup reserve management
- [ ] Add monitoring for reserve health
- [ ] Test gas savings
- [ ] Update frontend for claim UI

---

## Monitoring Metrics

### Key Metrics to Track
```rust
// Gas efficiency
let metrics = calculate_distribution_metrics(
    &pool,
    total_claims,
    total_gas_used,
    reserve_hits,
);

println!("Gas saved: {}", metrics.total_gas_saved);
println!("Batch efficiency: {}%", metrics.batch_efficiency);
println!("Reserve hit rate: {}%", metrics.reserve_hit_rate);
println!("Avg claim size: {} XLM", metrics.average_claim_size / XLM);
```

---

## Troubleshooting

### Issue: High gas costs
**Solution**: Check if batching is enabled and reserve is maintained

### Issue: Claims failing
**Solution**: Verify reserve balance and treasury balance are sufficient

### Issue: Low efficiency
**Solution**: Increase batch size and encourage users to accumulate before claiming

### Issue: Reserve depleting quickly
**Solution**: Increase target reserve days or reduce auto-fund threshold

---

## Quick Reference Card

| Need | Function | Gas Savings |
|------|----------|-------------|
| Multiple users | `batch_distribute_rewards()` | 35% |
| Frequent rewards | `accrue_reward()` | 85-99% |
| User claims | `claim_accrued_rewards()` | Varies |
| Reserve setup | `manage_reward_reserve()` | 95% |
| Pool health | `monitor_rewards_pool()` | 80-95% |
| Batch claims | `batch_claim_rewards()` | 40% |

---

## Support

- **Documentation**: `/docs/reward_distribution_optimization.md`
- **Benchmarks**: `/docs/reward_optimization_benchmarks.md`
- **Tests**: `contracts/*/src/gas_optimization_tests.rs`
- **Issue**: #510

---

*Quick Reference Version: 1.0*  
*Last Updated: 2026-06-01*
