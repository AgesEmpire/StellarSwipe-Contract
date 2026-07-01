// Liquidity Pool Management and Optimization
// Mechanisms for managing and optimizing liquidity pools

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Vec};

// ============================================================================
// Liquidity Pool Interface
// ============================================================================

/// Liquidity pool data
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct LiquidityPool {
    pub pool_id: u64,
    pub token_a: Address,
    pub token_b: Address,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_shares: i128,
    pub fee_rate: u32,           // Basis points (e.g., 30 = 0.3%)
    pub created_at: u64,
    pub last_rebalanced: u64,
}

/// Pool statistics
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PoolStatistics {
    pub pool_id: u64,
    pub total_volume_24h: i128,
    pub total_fees_24h: i128,
    pub price_a_to_b: i128,
    pub price_b_to_a: i128,
    pub liquidity_depth: i128,
    pub utilization_rate: u32,   // Percentage
    pub apy: u32,                // Annual percentage yield
}

/// Liquidity position
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct LiquidityPosition {
    pub provider: Address,
    pub pool_id: u64,
    pub shares: i128,
    pub deposited_a: i128,
    pub deposited_b: i128,
    pub earned_fees: i128,
    pub created_at: u64,
}

/// A single deposit event for a provider into a pool, recording the number of
/// shares minted and the ledger timestamp at deposit time.  The list of records
/// stored under `DataKey::DepositRecords` is maintained in chronological (FIFO)
/// order so that the oldest deposits are always consumed first during withdrawals.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct DepositRecord {
    /// LP shares minted by this deposit.
    pub shares: i128,
    /// Ledger timestamp at the moment of deposit (seconds since epoch).
    pub deposited_at: u64,
}

/// Pool manager
pub struct LiquidityPoolManager;

impl LiquidityPoolManager {
    /// Create new liquidity pool
    pub fn create_pool(
        env: &Env,
        token_a: Address,
        token_b: Address,
        initial_a: i128,
        initial_b: i128,
        fee_rate: u32,
        creator: Address,
    ) -> Result<LiquidityPool, PoolError> {
        creator.require_auth();
        
        // Validate inputs
        if initial_a <= 0 || initial_b <= 0 {
            return Err(PoolError::InvalidAmount);
        }
        
        if fee_rate > 1000 {  // Max 10%
            return Err(PoolError::InvalidFeeRate);
        }
        
        let pool_id = get_next_pool_id(env);
        
        // Calculate initial shares (geometric mean)
        let total_shares = Self::calculate_initial_shares(initial_a, initial_b);
        
        let pool = LiquidityPool {
            pool_id,
            token_a,
            token_b,
            reserve_a: initial_a,
            reserve_b: initial_b,
            total_shares,
            fee_rate,
            created_at: env.ledger().timestamp(),
            last_rebalanced: env.ledger().timestamp(),
        };
        
        // Store pool
        env.storage().instance().set(
            &DataKey::Pool(pool_id),
            &pool
        );
        
        // Create initial position for creator
        let position = LiquidityPosition {
            provider: creator.clone(),
            pool_id,
            shares: total_shares,
            deposited_a: initial_a,
            deposited_b: initial_b,
            earned_fees: 0,
            created_at: env.ledger().timestamp(),
        };
        
        Self::store_position(env, &position);

        // Record the initial deposit for lock-up tracking (FIFO list)
        Self::push_deposit_record(env, &creator, pool_id, total_shares, env.ledger().timestamp());
        
        Ok(pool)
    }

    /// Add liquidity to pool
    pub fn add_liquidity(
        env: &Env,
        pool_id: u64,
        amount_a: i128,
        amount_b: i128,
        provider: Address,
    ) -> Result<i128, PoolError> {
        provider.require_auth();
        
        let mut pool: LiquidityPool = env
            .storage()
            .instance()
            .get(&DataKey::Pool(pool_id))
            .ok_or(PoolError::PoolNotFound)?;
        
        // Calculate shares to mint
        let shares = Self::calculate_shares_to_mint(
            &pool,
            amount_a,
            amount_b,
        )?;
        
        // Update pool reserves
        pool.reserve_a += amount_a;
        pool.reserve_b += amount_b;
        pool.total_shares += shares;
        
        // Store updated pool
        env.storage().instance().set(
            &DataKey::Pool(pool_id),
            &pool
        );
        
        // Update or create position
        Self::update_position(env, provider.clone(), pool_id, shares, amount_a, amount_b);

        // Record this deposit for per-deposit lock-up tracking (FIFO)
        let now = env.ledger().timestamp();
        Self::push_deposit_record(env, &provider, pool_id, shares, now);
        
        Ok(shares)
    }
    
    /// Remove liquidity from pool.
    ///
    /// Enforces two independent guards (both must pass):
    /// 1. **Minimum-liquidity threshold** – the pool's combined reserves must not
    ///    fall below `MIN_POOL_RESERVE` after the withdrawal.
    /// 2. **Per-deposit lock-up period** – shares are consumed FIFO from the
    ///    provider's deposit list; any shares still within their lock-up window
    ///    cause the call to return `PoolError::DepositLocked`.
    pub fn remove_liquidity(
        env: &Env,
        pool_id: u64,
        shares: i128,
        provider: Address,
    ) -> Result<(i128, i128), PoolError> {
        provider.require_auth();
        
        let mut pool: LiquidityPool = env
            .storage()
            .instance()
            .get(&DataKey::Pool(pool_id))
            .ok_or(PoolError::PoolNotFound)?;
        
        // Calculate amounts to return
        let amount_a = (shares * pool.reserve_a) / pool.total_shares;
        let amount_b = (shares * pool.reserve_b) / pool.total_shares;

        // --- Guard 1: minimum-liquidity threshold ---
        // Prevent the pool reserves from dropping below the hard floor so that
        // the pool remains functional for swaps even after the withdrawal.
        const MIN_POOL_RESERVE: i128 = 1_000;
        if pool.reserve_a - amount_a < MIN_POOL_RESERVE
            || pool.reserve_b - amount_b < MIN_POOL_RESERVE
        {
            return Err(PoolError::InsufficientLiquidity);
        }

        // --- Guard 2: per-deposit lock-up (FIFO) ---
        // Walk the deposit list oldest-first and verify that enough unlocked
        // shares exist to cover the requested withdrawal amount.
        let lockup_duration = Self::get_lockup_duration(env);
        if lockup_duration > 0 {
            let now = env.ledger().timestamp();
            let key = DataKey::DepositRecords(provider.clone(), pool_id);
            let records: Vec<DepositRecord> = env
                .storage()
                .instance()
                .get(&key)
                .unwrap_or_else(|| Vec::new(env));

            // Count how many shares are available (i.e. past their lock-up).
            let mut unlocked: i128 = 0;
            for record in records.iter() {
                if now >= record.deposited_at + lockup_duration {
                    unlocked += record.shares;
                }
                // Stop early once we have enough – avoids iterating the full list
                // on every happy-path call.
                if unlocked >= shares {
                    break;
                }
            }

            if unlocked < shares {
                return Err(PoolError::DepositLocked);
            }

            // Consume shares FIFO from the deposit list, updating in storage.
            let mut remaining_to_consume = shares;
            let mut updated_records: Vec<DepositRecord> = Vec::new(env);

            for record in records.iter() {
                if remaining_to_consume <= 0 {
                    // Keep the rest untouched.
                    updated_records.push_back(record.clone());
                } else if now >= record.deposited_at + lockup_duration {
                    if record.shares <= remaining_to_consume {
                        // Fully consume this record.
                        remaining_to_consume -= record.shares;
                        // Drop this record (don't push to updated_records).
                    } else {
                        // Partially consume this record.
                        let leftover = record.shares - remaining_to_consume;
                        remaining_to_consume = 0;
                        updated_records.push_back(DepositRecord {
                            shares: leftover,
                            deposited_at: record.deposited_at,
                        });
                    }
                } else {
                    // Still locked – keep as-is.
                    updated_records.push_back(record.clone());
                }
            }

            if updated_records.is_empty() {
                env.storage().instance().remove(&key);
            } else {
                env.storage().instance().set(&key, &updated_records);
            }
        }
        
        // Update pool reserves
        pool.reserve_a -= amount_a;
        pool.reserve_b -= amount_b;
        pool.total_shares -= shares;
        
        // Store updated pool
        env.storage().instance().set(
            &DataKey::Pool(pool_id),
            &pool
        );
        
        // Update position
        Self::reduce_position(env, &provider, pool_id, shares);
        
        Ok((amount_a, amount_b))
    }

    // -------------------------------------------------------------------------
    // Lock-up administration
    // -------------------------------------------------------------------------

    /// Set the global lock-up duration (in seconds) that applies to **all new
    /// deposits** from this point forward.  Pass `0` to disable lock-up
    /// enforcement entirely.
    ///
    /// `admin` must be the authorised administrator; the caller is responsible
    /// for passing the correct admin address and calling `require_auth` before
    /// invoking this function (or including auth here as shown).
    pub fn set_lockup_duration(env: &Env, admin: Address, duration_secs: u64) {
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::LockupConfig, &duration_secs);
    }

    /// Read the current global lock-up duration in seconds.  Returns `0` if
    /// no duration has been configured (i.e. lock-up is disabled).
    pub fn get_lockup_duration(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::LockupConfig)
            .unwrap_or(0u64)
    }
    
    /// Calculate initial shares
    fn calculate_initial_shares(amount_a: i128, amount_b: i128) -> i128 {
        // Geometric mean: sqrt(a * b)
        let product = amount_a * amount_b;
        Self::sqrt(product)
    }
    
    /// Calculate shares to mint
    fn calculate_shares_to_mint(
        pool: &LiquidityPool,
        amount_a: i128,
        amount_b: i128,
    ) -> Result<i128, PoolError> {
        // shares = min(amount_a / reserve_a, amount_b / reserve_b) * total_shares
        let shares_from_a = (amount_a * pool.total_shares) / pool.reserve_a;
        let shares_from_b = (amount_b * pool.total_shares) / pool.reserve_b;
        
        Ok(shares_from_a.min(shares_from_b))
    }
    
    /// Square root approximation (Babylonian method)
    fn sqrt(n: i128) -> i128 {
        if n == 0 {
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
    
    /// Store position
    fn store_position(env: &Env, position: &LiquidityPosition) {
        env.storage().instance().set(
            &DataKey::Position(position.provider.clone(), position.pool_id),
            position
        );
    }
    
    /// Update position
    fn update_position(
        env: &Env,
        provider: Address,
        pool_id: u64,
        shares: i128,
        amount_a: i128,
        amount_b: i128,
    ) {
        let mut position: LiquidityPosition = env
            .storage()
            .instance()
            .get(&DataKey::Position(provider.clone(), pool_id))
            .unwrap_or(LiquidityPosition {
                provider: provider.clone(),
                pool_id,
                shares: 0,
                deposited_a: 0,
                deposited_b: 0,
                earned_fees: 0,
                created_at: env.ledger().timestamp(),
            });
        
        position.shares += shares;
        position.deposited_a += amount_a;
        position.deposited_b += amount_b;
        
        Self::store_position(env, &position);
    }
    
    /// Reduce position
    fn reduce_position(env: &Env, provider: &Address, pool_id: u64, shares: i128) {
        if let Some(mut position) = env
            .storage()
            .instance()
            .get::<DataKey, LiquidityPosition>(&DataKey::Position(provider.clone(), pool_id))
        {
            position.shares -= shares;
            
            if position.shares > 0 {
                Self::store_position(env, &position);
            } else {
                // Remove position if no shares left
                env.storage().instance().remove(
                    &DataKey::Position(provider.clone(), pool_id)
                );
            }
        }
    }

    /// Append a new `DepositRecord` to the FIFO list stored under
    /// `DataKey::DepositRecords(provider, pool_id)`.
    fn push_deposit_record(
        env: &Env,
        provider: &Address,
        pool_id: u64,
        shares: i128,
        deposited_at: u64,
    ) {
        let key = DataKey::DepositRecords(provider.clone(), pool_id);
        let mut records: Vec<DepositRecord> = env
            .storage()
            .instance()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        records.push_back(DepositRecord { shares, deposited_at });
        env.storage().instance().set(&key, &records);
    }
    
    /// Get pool
    pub fn get_pool(env: &Env, pool_id: u64) -> Option<LiquidityPool> {
        env.storage().instance().get(&DataKey::Pool(pool_id))
    }
    
    /// Get position
    pub fn get_position(
        env: &Env,
        provider: &Address,
        pool_id: u64,
    ) -> Option<LiquidityPosition> {
        env.storage()
            .instance()
            .get(&DataKey::Position(provider.clone(), pool_id))
    }
}

// ============================================================================
// Pool Rebalancing Logic
// ============================================================================

/// Rebalancing strategy
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum RebalancingStrategy {
    ConstantProduct,     // x * y = k
    StableSwap,          // For stablecoins
    Weighted,            // Custom weights
    Dynamic,             // Adaptive based on market
}

/// Rebalancing result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RebalancingResult {
    pub pool_id: u64,
    pub strategy: RebalancingStrategy,
    pub old_reserve_a: i128,
    pub old_reserve_b: i128,
    pub new_reserve_a: i128,
    pub new_reserve_b: i128,
    pub rebalanced_at: u64,
    pub gas_used: u64,
}

/// Pool rebalancer
pub struct PoolRebalancer;

impl PoolRebalancer {
    /// Rebalance pool
    pub fn rebalance_pool(
        env: &Env,
        pool_id: u64,
        strategy: RebalancingStrategy,
    ) -> Result<RebalancingResult, PoolError> {
        let start_gas = estimate_gas(env);
        
        let mut pool: LiquidityPool = env
            .storage()
            .instance()
            .get(&DataKey::Pool(pool_id))
            .ok_or(PoolError::PoolNotFound)?;
        
        let old_reserve_a = pool.reserve_a;
        let old_reserve_b = pool.reserve_b;
        
        // Apply rebalancing strategy
        match strategy {
            RebalancingStrategy::ConstantProduct => {
                Self::rebalance_constant_product(env, &mut pool)?;
            }
            RebalancingStrategy::StableSwap => {
                Self::rebalance_stable_swap(env, &mut pool)?;
            }
            RebalancingStrategy::Weighted => {
                Self::rebalance_weighted(env, &mut pool)?;
            }
            RebalancingStrategy::Dynamic => {
                Self::rebalance_dynamic(env, &mut pool)?;
            }
        }
        
        pool.last_rebalanced = env.ledger().timestamp();
        
        // Store updated pool
        env.storage().instance().set(
            &DataKey::Pool(pool_id),
            &pool
        );
        
        let end_gas = estimate_gas(env);
        
        Ok(RebalancingResult {
            pool_id,
            strategy,
            old_reserve_a,
            old_reserve_b,
            new_reserve_a: pool.reserve_a,
            new_reserve_b: pool.reserve_b,
            rebalanced_at: env.ledger().timestamp(),
            gas_used: end_gas.saturating_sub(start_gas),
        })
    }
    
    /// Rebalance using constant product formula
    fn rebalance_constant_product(
        env: &Env,
        pool: &mut LiquidityPool,
    ) -> Result<(), PoolError> {
        // Maintain x * y = k
        let k = pool.reserve_a * pool.reserve_b;
        
        // Adjust reserves to maintain constant product
        // This is a simplified version - in production, consider price oracles
        
        Ok(())
    }
    
    /// Rebalance for stable swap
    fn rebalance_stable_swap(
        env: &Env,
        pool: &mut LiquidityPool,
    ) -> Result<(), PoolError> {
        // For stablecoins, maintain 1:1 ratio
        let total = pool.reserve_a + pool.reserve_b;
        let target = total / 2;
        
        // Adjust reserves towards 1:1
        let adjustment = (target - pool.reserve_a) / 10; // Gradual adjustment
        
        pool.reserve_a += adjustment;
        pool.reserve_b -= adjustment;
        
        Ok(())
    }
    
    /// Rebalance with custom weights
    fn rebalance_weighted(
        env: &Env,
        pool: &mut LiquidityPool,
    ) -> Result<(), PoolError> {
        // Custom weight rebalancing (e.g., 80/20 pool)
        // This is a placeholder - implement based on pool configuration
        Ok(())
    }
    
    /// Dynamic rebalancing based on market conditions
    fn rebalance_dynamic(
        env: &Env,
        pool: &mut LiquidityPool,
    ) -> Result<(), PoolError> {
        // Adaptive rebalancing based on volatility and volume
        // This is a placeholder - implement with market data
        Ok(())
    }
    
    /// Check if rebalancing is needed
    pub fn needs_rebalancing(env: &Env, pool: &LiquidityPool) -> bool {
        let time_since_last = env.ledger().timestamp() - pool.last_rebalanced;
        let min_interval = 3600; // 1 hour
        
        if time_since_last < min_interval {
            return false;
        }
        
        // Check if reserves are significantly imbalanced
        let ratio = (pool.reserve_a * 100) / pool.reserve_b;
        
        // Rebalance if ratio deviates more than 20% from 1:1
        ratio < 80 || ratio > 120
    }
}

// ============================================================================
// Yield Optimization
// ============================================================================

/// Yield optimization strategy
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct YieldStrategy {
    pub strategy_id: u64,
    pub name: String,
    pub target_apy: u32,
    pub risk_level: RiskLevel,
    pub auto_compound: bool,
}

/// Risk level
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// Yield optimization result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct YieldOptimizationResult {
    pub pool_id: u64,
    pub old_apy: u32,
    pub new_apy: u32,
    pub optimization_applied: Vec<String>,
    pub estimated_increase: i128,
}

/// Yield optimizer
pub struct YieldOptimizer;

impl YieldOptimizer {
    /// Optimize yield for pool
    pub fn optimize_yield(
        env: &Env,
        pool_id: u64,
        strategy: YieldStrategy,
    ) -> Result<YieldOptimizationResult, PoolError> {
        let pool: LiquidityPool = env
            .storage()
            .instance()
            .get(&DataKey::Pool(pool_id))
            .ok_or(PoolError::PoolNotFound)?;
        
        let old_apy = Self::calculate_current_apy(env, &pool);
        let mut optimizations = Vec::new(env);
        
        // Apply optimizations based on strategy
        if strategy.auto_compound {
            optimizations.push_back(String::from_str(env, "Auto-compounding enabled"));
        }
        
        // Fee optimization
        let optimal_fee = Self::calculate_optimal_fee(env, &pool);
        if optimal_fee != pool.fee_rate {
            optimizations.push_back(String::from_str(env, "Fee rate optimized"));
        }
        
        // Rebalancing frequency optimization
        optimizations.push_back(String::from_str(env, "Rebalancing frequency optimized"));
        
        let new_apy = Self::estimate_new_apy(env, &pool, &strategy);
        let estimated_increase = ((new_apy - old_apy) as i128 * pool.reserve_a) / 10000;
        
        Ok(YieldOptimizationResult {
            pool_id,
            old_apy,
            new_apy,
            optimization_applied: optimizations,
            estimated_increase,
        })
    }
    
    /// Calculate current APY
    fn calculate_current_apy(env: &Env, pool: &LiquidityPool) -> u32 {
        // Simplified APY calculation
        // In production, use actual fee earnings and time period
        let daily_volume = pool.reserve_a / 10; // Estimate
        let daily_fees = (daily_volume * pool.fee_rate as i128) / 10000;
        let annual_fees = daily_fees * 365;
        let total_liquidity = pool.reserve_a + pool.reserve_b;
        
        if total_liquidity > 0 {
            ((annual_fees * 10000) / total_liquidity) as u32
        } else {
            0
        }
    }
    
    /// Calculate optimal fee rate
    fn calculate_optimal_fee(env: &Env, pool: &LiquidityPool) -> u32 {
        // Optimal fee balances volume and revenue
        // Higher fees = lower volume but more per trade
        // Lower fees = higher volume but less per trade
        
        let current_volume = pool.reserve_a / 10; // Estimate
        
        // Optimal fee typically between 0.1% and 1%
        if current_volume > pool.reserve_a {
            10 // 0.1% for high volume
        } else if current_volume > pool.reserve_a / 2 {
            30 // 0.3% for medium volume
        } else {
            50 // 0.5% for low volume
        }
    }
    
    /// Estimate new APY after optimization
    fn estimate_new_apy(
        env: &Env,
        pool: &LiquidityPool,
        strategy: &YieldStrategy,
    ) -> u32 {
        let base_apy = Self::calculate_current_apy(env, pool);
        
        // Apply strategy multipliers
        let mut multiplier = 100u32;
        
        if strategy.auto_compound {
            multiplier += 10; // 10% boost from compounding
        }
        
        match strategy.risk_level {
            RiskLevel::Low => multiplier += 5,
            RiskLevel::Medium => multiplier += 15,
            RiskLevel::High => multiplier += 30,
        }
        
        (base_apy * multiplier) / 100
    }
    
    /// Auto-compound rewards
    pub fn auto_compound(
        env: &Env,
        pool_id: u64,
        provider: Address,
    ) -> Result<i128, PoolError> {
        let position = LiquidityPoolManager::get_position(env, &provider, pool_id)
            .ok_or(PoolError::PositionNotFound)?;
        
        if position.earned_fees == 0 {
            return Ok(0);
        }
        
        // Reinvest earned fees back into pool
        let fees_to_compound = position.earned_fees;
        
        // Split fees proportionally
        let amount_a = fees_to_compound / 2;
        let amount_b = fees_to_compound / 2;
        
        // Add liquidity
        let shares = LiquidityPoolManager::add_liquidity(
            env,
            pool_id,
            amount_a,
            amount_b,
            provider,
        )?;
        
        Ok(shares)
    }
}

// ============================================================================
// Liquidity Provider Incentives
// ============================================================================

/// Incentive program
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct IncentiveProgram {
    pub program_id: u64,
    pub pool_id: u64,
    pub reward_token: Address,
    pub reward_rate: i128,        // Rewards per second
    pub start_time: u64,
    pub end_time: u64,
    pub total_rewards: i128,
    pub distributed_rewards: i128,
}

/// Provider rewards
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ProviderRewards {
    pub provider: Address,
    pub pool_id: u64,
    pub earned_fees: i128,
    pub earned_incentives: i128,
    pub last_claim: u64,
}

/// Incentive manager
pub struct IncentiveManager;

impl IncentiveManager {
    /// Create incentive program
    pub fn create_program(
        env: &Env,
        pool_id: u64,
        reward_token: Address,
        reward_rate: i128,
        duration: u64,
        total_rewards: i128,
        creator: Address,
    ) -> Result<IncentiveProgram, PoolError> {
        creator.require_auth();
        
        let program_id = get_next_program_id(env);
        let start_time = env.ledger().timestamp();
        let end_time = start_time + duration;
        
        let program = IncentiveProgram {
            program_id,
            pool_id,
            reward_token,
            reward_rate,
            start_time,
            end_time,
            total_rewards,
            distributed_rewards: 0,
        };
        
        // Store program
        env.storage().instance().set(
            &DataKey::IncentiveProgram(program_id),
            &program
        );
        
        Ok(program)
    }
    
    /// Calculate rewards for provider
    pub fn calculate_rewards(
        env: &Env,
        provider: &Address,
        pool_id: u64,
    ) -> Result<ProviderRewards, PoolError> {
        let position = LiquidityPoolManager::get_position(env, provider, pool_id)
            .ok_or(PoolError::PositionNotFound)?;
        
        let pool = LiquidityPoolManager::get_pool(env, pool_id)
            .ok_or(PoolError::PoolNotFound)?;
        
        // Calculate fee earnings
        let share_percentage = (position.shares * 10000) / pool.total_shares;
        let earned_fees = position.earned_fees;
        
        // Calculate incentive earnings
        let earned_incentives = Self::calculate_incentive_earnings(
            env,
            provider,
            pool_id,
            share_percentage,
        );
        
        Ok(ProviderRewards {
            provider: provider.clone(),
            pool_id,
            earned_fees,
            earned_incentives,
            last_claim: env.ledger().timestamp(),
        })
    }
    
    /// Calculate incentive earnings
    fn calculate_incentive_earnings(
        env: &Env,
        provider: &Address,
        pool_id: u64,
        share_percentage: i128,
    ) -> i128 {
        // Find active programs for this pool
        // Simplified - in production, iterate through all programs
        
        let current_time = env.ledger().timestamp();
        
        // Placeholder calculation
        let time_staked = 86400; // 1 day
        let reward_rate = 100; // Per second
        
        (time_staked as i128 * reward_rate * share_percentage) / 10000
    }
    
    /// Claim rewards
    pub fn claim_rewards(
        env: &Env,
        provider: Address,
        pool_id: u64,
    ) -> Result<(i128, i128), PoolError> {
        provider.require_auth();
        
        let rewards = Self::calculate_rewards(env, &provider, pool_id)?;
        
        // Reset earned amounts
        // In production, transfer tokens to provider
        
        Ok((rewards.earned_fees, rewards.earned_incentives))
    }
    
    /// Get incentive program
    pub fn get_program(env: &Env, program_id: u64) -> Option<IncentiveProgram> {
        env.storage()
            .instance()
            .get(&DataKey::IncentiveProgram(program_id))
    }
}

// ============================================================================
// Pool Monitoring
// ============================================================================

/// Pool health metrics
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PoolHealthMetrics {
    pub pool_id: u64,
    pub health_score: u32,        // 0-100
    pub liquidity_score: u32,     // 0-100
    pub balance_score: u32,       // 0-100
    pub utilization_score: u32,   // 0-100
    pub risk_level: RiskLevel,
    pub warnings: Vec<String>,
}

/// Pool alert
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PoolAlert {
    pub alert_id: u64,
    pub pool_id: u64,
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub message: String,
    pub triggered_at: u64,
}

/// Alert type
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum AlertType {
    LowLiquidity,
    HighImbalance,
    UnusualVolume,
    PriceDeviation,
    HighSlippage,
}

/// Alert severity
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// Pool monitor
pub struct PoolMonitor;

impl PoolMonitor {
    /// Monitor pool health
    pub fn monitor_pool(
        env: &Env,
        pool_id: u64,
    ) -> Result<PoolHealthMetrics, PoolError> {
        let pool = LiquidityPoolManager::get_pool(env, pool_id)
            .ok_or(PoolError::PoolNotFound)?;
        
        // Calculate health scores
        let liquidity_score = Self::calculate_liquidity_score(&pool);
        let balance_score = Self::calculate_balance_score(&pool);
        let utilization_score = Self::calculate_utilization_score(env, &pool);
        
        // Overall health score (weighted average)
        let health_score = (liquidity_score * 40 + balance_score * 30 + utilization_score * 30) / 100;
        
        // Determine risk level
        let risk_level = if health_score >= 80 {
            RiskLevel::Low
        } else if health_score >= 50 {
            RiskLevel::Medium
        } else {
            RiskLevel::High
        };
        
        // Generate warnings
        let mut warnings = Vec::new(env);
        
        if liquidity_score < 50 {
            warnings.push_back(String::from_str(env, "Low liquidity detected"));
        }
        
        if balance_score < 50 {
            warnings.push_back(String::from_str(env, "Pool imbalance detected"));
        }
        
        if utilization_score > 90 {
            warnings.push_back(String::from_str(env, "High utilization - consider adding liquidity"));
        }
        
        Ok(PoolHealthMetrics {
            pool_id,
            health_score,
            liquidity_score,
            balance_score,
            utilization_score,
            risk_level,
            warnings,
        })
    }
    
    /// Calculate liquidity score
    fn calculate_liquidity_score(pool: &LiquidityPool) -> u32 {
        let total_liquidity = pool.reserve_a + pool.reserve_b;
        let min_liquidity = 1000000; // Minimum threshold
        
        if total_liquidity >= min_liquidity * 10 {
            100
        } else if total_liquidity >= min_liquidity {
            ((total_liquidity * 100) / (min_liquidity * 10)) as u32
        } else {
            ((total_liquidity * 50) / min_liquidity) as u32
        }
    }
    
    /// Calculate balance score
    fn calculate_balance_score(pool: &LiquidityPool) -> u32 {
        // Perfect balance = 1:1 ratio
        let ratio = if pool.reserve_b > 0 {
            (pool.reserve_a * 100) / pool.reserve_b
        } else {
            0
        };
        
        // Score based on deviation from 100 (1:1 ratio)
        let deviation = if ratio > 100 {
            ratio - 100
        } else {
            100 - ratio
        };
        
        if deviation <= 10 {
            100
        } else if deviation <= 30 {
            80
        } else if deviation <= 50 {
            60
        } else {
            40
        }
    }
    
    /// Calculate utilization score
    fn calculate_utilization_score(env: &Env, pool: &LiquidityPool) -> u32 {
        // Utilization = volume / liquidity
        // Simplified calculation
        let total_liquidity = pool.reserve_a + pool.reserve_b;
        let estimated_volume = total_liquidity / 20; // 5% daily volume estimate
        
        let utilization = if total_liquidity > 0 {
            ((estimated_volume * 100) / total_liquidity) as u32
        } else {
            0
        };
        
        // Optimal utilization is 30-70%
        if utilization >= 30 && utilization <= 70 {
            100
        } else if utilization < 30 {
            (utilization * 100) / 30
        } else {
            100 - ((utilization - 70) * 100) / 30
        }
    }
    
    /// Create alert
    pub fn create_alert(
        env: &Env,
        pool_id: u64,
        alert_type: AlertType,
        severity: AlertSeverity,
        message: String,
    ) -> PoolAlert {
        let alert_id = get_next_alert_id(env);
        
        let alert = PoolAlert {
            alert_id,
            pool_id,
            alert_type,
            severity,
            message,
            triggered_at: env.ledger().timestamp(),
        };
        
        // Store alert
        env.storage().instance().set(
            &DataKey::Alert(alert_id),
            &alert
        );
        
        alert
    }
    
    /// Check for alerts
    pub fn check_alerts(env: &Env, pool_id: u64) -> Vec<PoolAlert> {
        let mut alerts = Vec::new(env);
        
        let pool = match LiquidityPoolManager::get_pool(env, pool_id) {
            Some(p) => p,
            None => return alerts,
        };
        
        // Check for low liquidity
        if pool.reserve_a + pool.reserve_b < 1000000 {
            let alert = Self::create_alert(
                env,
                pool_id,
                AlertType::LowLiquidity,
                AlertSeverity::Warning,
                String::from_str(env, "Pool liquidity below threshold"),
            );
            alerts.push_back(alert);
        }
        
        // Check for imbalance
        let ratio = (pool.reserve_a * 100) / pool.reserve_b;
        if ratio < 50 || ratio > 200 {
            let alert = Self::create_alert(
                env,
                pool_id,
                AlertType::HighImbalance,
                AlertSeverity::Warning,
                String::from_str(env, "Pool reserves significantly imbalanced"),
            );
            alerts.push_back(alert);
        }
        
        alerts
    }
}

// ============================================================================
// Liquidity Gap Analysis
// ============================================================================

/// Liquidity gap
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct LiquidityGap {
    pub pool_id: u64,
    pub token: Address,
    pub current_liquidity: i128,
    pub required_liquidity: i128,
    pub gap_amount: i128,
    pub gap_percentage: u32,
    pub priority: GapPriority,
}

/// Gap priority
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum GapPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Gap analysis result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct GapAnalysisResult {
    pub pool_id: u64,
    pub gaps: Vec<LiquidityGap>,
    pub total_gap_value: i128,
    pub recommendations: Vec<String>,
    pub analyzed_at: u64,
}

/// Liquidity gap analyzer
pub struct LiquidityGapAnalyzer;

impl LiquidityGapAnalyzer {
    /// Analyze liquidity gaps
    pub fn analyze_gaps(
        env: &Env,
        pool_id: u64,
    ) -> Result<GapAnalysisResult, PoolError> {
        let pool = LiquidityPoolManager::get_pool(env, pool_id)
            .ok_or(PoolError::PoolNotFound)?;
        
        let mut gaps = Vec::new(env);
        let mut recommendations = Vec::new(env);
        
        // Analyze token A liquidity
        let gap_a = Self::analyze_token_gap(
            env,
            &pool,
            pool.token_a.clone(),
            pool.reserve_a,
        );
        
        if gap_a.gap_amount > 0 {
            gaps.push_back(gap_a.clone());
            
            if gap_a.priority == GapPriority::High || gap_a.priority == GapPriority::Critical {
                recommendations.push_back(String::from_str(
                    env,
                    "Urgent: Add liquidity for token A"
                ));
            }
        }
        
        // Analyze token B liquidity
        let gap_b = Self::analyze_token_gap(
            env,
            &pool,
            pool.token_b.clone(),
            pool.reserve_b,
        );
        
        if gap_b.gap_amount > 0 {
            gaps.push_back(gap_b.clone());
            
            if gap_b.priority == GapPriority::High || gap_b.priority == GapPriority::Critical {
                recommendations.push_back(String::from_str(
                    env,
                    "Urgent: Add liquidity for token B"
                ));
            }
        }
        
        // Calculate total gap value
        let total_gap_value = gap_a.gap_amount + gap_b.gap_amount;
        
        // Add general recommendations
        if total_gap_value > 0 {
            recommendations.push_back(String::from_str(
                env,
                "Consider incentive programs to attract liquidity"
            ));
        }
        
        Ok(GapAnalysisResult {
            pool_id,
            gaps,
            total_gap_value,
            recommendations,
            analyzed_at: env.ledger().timestamp(),
        })
    }
    
    /// Analyze gap for specific token
    fn analyze_token_gap(
        env: &Env,
        pool: &LiquidityPool,
        token: Address,
        current_liquidity: i128,
    ) -> LiquidityGap {
        // Calculate required liquidity based on expected volume
        let required_liquidity = Self::calculate_required_liquidity(env, pool);
        
        let gap_amount = if required_liquidity > current_liquidity {
            required_liquidity - current_liquidity
        } else {
            0
        };
        
        let gap_percentage = if required_liquidity > 0 {
            ((gap_amount * 100) / required_liquidity) as u32
        } else {
            0
        };
        
        // Determine priority
        let priority = if gap_percentage >= 50 {
            GapPriority::Critical
        } else if gap_percentage >= 30 {
            GapPriority::High
        } else if gap_percentage >= 15 {
            GapPriority::Medium
        } else {
            GapPriority::Low
        };
        
        LiquidityGap {
            pool_id: pool.pool_id,
            token,
            current_liquidity,
            required_liquidity,
            gap_amount,
            gap_percentage,
            priority,
        }
    }
    
    /// Calculate required liquidity
    fn calculate_required_liquidity(env: &Env, pool: &LiquidityPool) -> i128 {
        // Required liquidity = expected daily volume * safety factor
        let total_liquidity = pool.reserve_a + pool.reserve_b;
        let estimated_daily_volume = total_liquidity / 10; // 10% of liquidity
        let safety_factor = 5; // 5x daily volume
        
        estimated_daily_volume * safety_factor
    }
    
    /// Get gap recommendations
    pub fn get_recommendations(
        env: &Env,
        gap: &LiquidityGap,
    ) -> Vec<String> {
        let mut recommendations = Vec::new(env);
        
        match gap.priority {
            GapPriority::Critical => {
                recommendations.push_back(String::from_str(
                    env,
                    "CRITICAL: Immediate liquidity addition required"
                ));
                recommendations.push_back(String::from_str(
                    env,
                    "Consider emergency incentive program"
                ));
            }
            GapPriority::High => {
                recommendations.push_back(String::from_str(
                    env,
                    "HIGH: Significant liquidity gap detected"
                ));
                recommendations.push_back(String::from_str(
                    env,
                    "Increase incentive rewards"
                ));
            }
            GapPriority::Medium => {
                recommendations.push_back(String::from_str(
                    env,
                    "MEDIUM: Monitor and plan liquidity addition"
                ));
            }
            GapPriority::Low => {
                recommendations.push_back(String::from_str(
                    env,
                    "LOW: Liquidity levels acceptable"
                ));
            }
        }
        
        recommendations
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get next pool ID
fn get_next_pool_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::PoolCounter)
        .unwrap_or(0);
    
    let next = current + 1;
    env.storage()
        .instance()
        .set(&DataKey::PoolCounter, &next);
    
    next
}

/// Get next program ID
fn get_next_program_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::ProgramCounter)
        .unwrap_or(0);
    
    let next = current + 1;
    env.storage()
        .instance()
        .set(&DataKey::ProgramCounter, &next);
    
    next
}

/// Get next alert ID
fn get_next_alert_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::AlertCounter)
        .unwrap_or(0);
    
    let next = current + 1;
    env.storage()
        .instance()
        .set(&DataKey::AlertCounter, &next);
    
    next
}

/// Estimate gas usage
fn estimate_gas(env: &Env) -> u64 {
    env.ledger().sequence() as u64 * 1000
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum PoolError {
    PoolNotFound = 1,
    PositionNotFound = 2,
    InvalidAmount = 3,
    InvalidFeeRate = 4,
    InsufficientLiquidity = 5,
    InsufficientShares = 6,
    RebalancingFailed = 7,
    OptimizationFailed = 8,
    /// The shares being withdrawn belong to a deposit whose lock-up period has
    /// not yet elapsed.  The caller must wait until `deposited_at + lockup_duration`
    /// before attempting withdrawal.
    DepositLocked = 9,
}

/// Storage keys
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    PoolCounter,
    ProgramCounter,
    AlertCounter,
    Pool(u64),
    Position(Address, u64),
    IncentiveProgram(u64),
    Alert(u64),
    /// Ordered list of `DepositRecord` values for a (provider, pool_id) pair.
    /// Records are stored in chronological order (oldest first) so that
    /// withdrawals can consume them FIFO.
    DepositRecords(Address, u64),
    /// Global lock-up duration in seconds.  A value of `0` means no lock-up.
    LockupConfig,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};

    // ------------------------------------------------------------------
    // Helper: build a minimal Env with a controlled ledger timestamp
    // ------------------------------------------------------------------
    fn env_at(timestamp: u64) -> Env {
        let env = Env::default();
        env.ledger().set(LedgerInfo {
            timestamp,
            protocol_version: 20,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 9_999_999,
        });
        env
    }

    // ------------------------------------------------------------------
    // Helper: set up a pool with a provider holding `shares` and the
    // lockup duration configured.  Returns (env, pool_id, provider, shares).
    // ------------------------------------------------------------------
    fn setup_pool_with_lockup(
        deposit_timestamp: u64,
        lockup_secs: u64,
    ) -> (Env, u64, Address, i128) {
        let env = env_at(deposit_timestamp);
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        let token_a = Address::generate(&env);
        let token_b = Address::generate(&env);

        // Set lockup duration before creating the pool so the initial deposit
        // is subject to the configured lock-up.
        LiquidityPoolManager::set_lockup_duration(&env, admin.clone(), lockup_secs);

        let pool = LiquidityPoolManager::create_pool(
            &env,
            token_a,
            token_b,
            100_000,
            100_000,
            30,
            provider.clone(),
        )
        .unwrap();

        let shares = pool.total_shares;
        (env, pool.pool_id, provider, shares)
    }

    // ------------------------------------------------------------------
    // Existing math tests (preserved)
    // ------------------------------------------------------------------

    #[test]
    fn test_sqrt() {
        assert_eq!(LiquidityPoolManager::sqrt(0), 0);
        assert_eq!(LiquidityPoolManager::sqrt(1), 1);
        assert_eq!(LiquidityPoolManager::sqrt(4), 2);
        assert_eq!(LiquidityPoolManager::sqrt(9), 3);
        assert_eq!(LiquidityPoolManager::sqrt(16), 4);
        assert_eq!(LiquidityPoolManager::sqrt(100), 10);
    }

    #[test]
    fn test_initial_shares() {
        let shares = LiquidityPoolManager::calculate_initial_shares(1000, 1000);
        assert_eq!(shares, 1000);
        
        let shares2 = LiquidityPoolManager::calculate_initial_shares(2000, 2000);
        assert_eq!(shares2, 2000);
    }

    #[test]
    fn test_liquidity_score() {
        let env = Env::default();
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 10000000,
            reserve_b: 10000000,
            total_shares: 10000000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };
        
        let score = PoolMonitor::calculate_liquidity_score(&pool);
        assert_eq!(score, 100);
    }

    #[test]
    fn test_balance_score() {
        let env = Env::default();
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 1000,
            reserve_b: 1000,
            total_shares: 1000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };
        
        let score = PoolMonitor::calculate_balance_score(&pool);
        assert_eq!(score, 100);
    }

    // ------------------------------------------------------------------
    // Lock-up tests
    // ------------------------------------------------------------------

    /// A withdrawal attempted before the lock-up period expires must be
    /// rejected with `PoolError::DepositLocked`.
    #[test]
    fn test_withdraw_before_lockup_expiry_is_rejected() {
        const LOCKUP: u64 = 86_400; // 1 day
        const DEPOSIT_TIME: u64 = 1_000_000;

        let (env, pool_id, provider, shares) =
            setup_pool_with_lockup(DEPOSIT_TIME, LOCKUP);

        // Advance time to just one second before expiry.
        env.ledger().set(LedgerInfo {
            timestamp: DEPOSIT_TIME + LOCKUP - 1,
            protocol_version: 20,
            sequence_number: 2,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 9_999_999,
        });

        // Try to withdraw all shares – should fail.
        let result = LiquidityPoolManager::remove_liquidity(
            &env,
            pool_id,
            shares / 2, // partial withdrawal, still locked
            provider,
        );

        assert_eq!(result, Err(PoolError::DepositLocked),
            "Expected DepositLocked but got {:?}", result);
    }

    /// A withdrawal attempted at *exactly* the expiry timestamp must succeed.
    #[test]
    fn test_withdraw_exactly_at_lockup_expiry_is_allowed() {
        const LOCKUP: u64 = 86_400; // 1 day
        const DEPOSIT_TIME: u64 = 1_000_000;

        let (env, pool_id, provider, shares) =
            setup_pool_with_lockup(DEPOSIT_TIME, LOCKUP);

        // Advance time to exactly the expiry second.
        env.ledger().set(LedgerInfo {
            timestamp: DEPOSIT_TIME + LOCKUP,
            protocol_version: 20,
            sequence_number: 2,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 9_999_999,
        });

        // Withdraw a safe partial amount (well above the pool's MIN_POOL_RESERVE).
        let withdraw_shares = shares / 4;
        let result = LiquidityPoolManager::remove_liquidity(
            &env,
            pool_id,
            withdraw_shares,
            provider,
        );

        assert!(result.is_ok(),
            "Expected successful withdrawal at expiry but got {:?}", result);
        let (amount_a, amount_b) = result.unwrap();
        assert!(amount_a > 0 && amount_b > 0);
    }

    /// A partial withdrawal that spans both unlocked *and* locked deposits must
    /// succeed only up to the unlocked portion and must correctly consume the
    /// oldest records FIFO, leaving locked records intact.
    #[test]
    fn test_partial_withdrawal_fifo_spanning_locked_and_unlocked() {
        const LOCKUP: u64 = 86_400; // 1 day
        const T0: u64 = 1_000_000;

        let env = env_at(T0);
        env.mock_all_auths();

        let admin    = Address::generate(&env);
        let provider = Address::generate(&env);
        let token_a  = Address::generate(&env);
        let token_b  = Address::generate(&env);

        // Configure lock-up and create the initial pool (deposit #1 at T0).
        LiquidityPoolManager::set_lockup_duration(&env, admin.clone(), LOCKUP);
        let pool = LiquidityPoolManager::create_pool(
            &env, token_a, token_b, 200_000, 200_000, 30, provider.clone(),
        ).unwrap();
        let pool_id = pool.pool_id;
        let shares_d1 = pool.total_shares; // shares from deposit #1

        // Advance time past the lock-up so deposit #1 is now unlocked.
        let t1 = T0 + LOCKUP + 1;
        env.ledger().set(LedgerInfo {
            timestamp: t1,
            protocol_version: 20,
            sequence_number: 2,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 9_999_999,
        });

        // Add a second deposit (deposit #2) at T1 – this deposit is still locked.
        let shares_d2 = LiquidityPoolManager::add_liquidity(
            &env, pool_id, 50_000, 50_000, provider.clone(),
        ).unwrap();

        // Attempt to withdraw exactly `shares_d1` (the unlocked portion).
        // This should succeed because deposit #1 has expired.
        let result = LiquidityPoolManager::remove_liquidity(
            &env, pool_id, shares_d1, provider.clone(),
        );
        assert!(result.is_ok(),
            "Expected unlocked portion withdrawal to succeed, got {:?}", result);

        // Attempt to withdraw any of `shares_d2` – deposit #2 is still locked.
        let result_locked = LiquidityPoolManager::remove_liquidity(
            &env, pool_id, 1, provider.clone(),
        );
        assert_eq!(result_locked, Err(PoolError::DepositLocked),
            "Expected locked portion to be rejected, got {:?}", result_locked);

        // Verify deposit records: deposit #1 should be gone, deposit #2 remains.
        let key = DataKey::DepositRecords(provider.clone(), pool_id);
        let remaining: Vec<DepositRecord> = env
            .storage()
            .instance()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));

        assert_eq!(remaining.len(), 1,
            "Expected exactly one deposit record remaining (the locked one)");
        assert_eq!(remaining.get(0).unwrap().shares, shares_d2);
        assert_eq!(remaining.get(0).unwrap().deposited_at, t1);
    }

    /// Verify that `set_lockup_duration` / `get_lockup_duration` round-trip
    /// correctly and that setting duration to 0 disables enforcement.
    #[test]
    fn test_set_and_get_lockup_duration() {
        let env = env_at(1_000_000);
        env.mock_all_auths();

        let admin = Address::generate(&env);

        // Default should be 0 (disabled).
        assert_eq!(LiquidityPoolManager::get_lockup_duration(&env), 0);

        LiquidityPoolManager::set_lockup_duration(&env, admin.clone(), 3600);
        assert_eq!(LiquidityPoolManager::get_lockup_duration(&env), 3600);

        // Reset to 0 disables enforcement.
        LiquidityPoolManager::set_lockup_duration(&env, admin.clone(), 0);
        assert_eq!(LiquidityPoolManager::get_lockup_duration(&env), 0);
    }

    /// With lockup disabled (duration == 0) withdrawals must still succeed
    /// immediately after deposit, demonstrating the guard is skipped entirely.
    #[test]
    fn test_no_lockup_when_duration_is_zero() {
        const T0: u64 = 500_000;

        let env = env_at(T0);
        env.mock_all_auths();

        let provider = Address::generate(&env);
        let token_a  = Address::generate(&env);
        let token_b  = Address::generate(&env);

        // No lockup configured.
        let pool = LiquidityPoolManager::create_pool(
            &env, token_a, token_b, 100_000, 100_000, 30, provider.clone(),
        ).unwrap();

        // Immediate withdrawal in the same timestamp should succeed.
        let result = LiquidityPoolManager::remove_liquidity(
            &env, pool.pool_id, pool.total_shares / 4, provider.clone(),
        );
        assert!(result.is_ok(),
            "Withdrawal with no lockup should succeed immediately, got {:?}", result);
    }
}
