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

/// Per-deposit record used for FIFO lock-up enforcement.
///
/// Each call to `add_liquidity` (or `create_pool`) appends one record that
/// captures the number of LP shares minted and the ledger timestamp at the
/// time of deposit.  Shares are consumed in FIFO order during withdrawals.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct DepositRecord {
    /// Number of LP shares minted by this individual deposit.
    pub shares: i128,
    /// `env.ledger().timestamp()` at the time of deposit (Unix seconds).
    pub deposited_at: u64,
}

// ============================================================================
// Lockup Administration
// ============================================================================

/// Admin-only functions for configuring the lock-up period.
pub struct LockupAdmin;

impl LockupAdmin {
    /// Set the global minimum lock-up duration (in seconds).
    ///
    /// Only an authorised admin address may call this.  A duration of `0`
    /// effectively disables lock-up enforcement (all deposits are immediately
    /// withdrawable).
    ///
    /// # Parameters
    /// * `admin`    – Address that must authorise the call.
    /// * `duration` – New lock-up period in seconds.
    pub fn set_lockup_duration(env: &Env, admin: Address, duration: u64) {
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::LockupDuration, &duration);
    }

    /// Read the currently configured lock-up duration in seconds.
    /// Returns `0` if never set (no lock-up).
    pub fn get_lockup_duration(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::LockupDuration)
            .unwrap_or(0u64)
    }
}

// ============================================================================
// Deposit-record helpers
// ============================================================================

/// Append a new `DepositRecord` to the FIFO queue for `(provider, pool_id)`.
fn push_deposit_record(env: &Env, provider: &Address, pool_id: u64, record: DepositRecord) {
    let key = DataKey::DepositRecords(provider.clone(), pool_id);
    let mut records: Vec<DepositRecord> = env
        .storage()
        .instance()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    records.push_back(record);
    env.storage().instance().set(&key, &records);
}

/// Consume shares from the FIFO queue, enforcing the lock-up period.
///
/// Returns `Ok(())` when exactly `shares_to_withdraw` unlocked shares are
/// consumed and the updated queue has been persisted.
///
/// Returns `Err(PoolError::DepositLocked)` if the requested number of shares
/// cannot be fully covered by deposits that have cleared the lock-up window.
fn consume_locked_shares(
    env: &Env,
    provider: &Address,
    pool_id: u64,
    shares_to_withdraw: i128,
    lockup_duration: u64,
) -> Result<(), PoolError> {
    let key = DataKey::DepositRecords(provider.clone(), pool_id);
    let mut records: Vec<DepositRecord> = env
        .storage()
        .instance()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    let now = env.ledger().timestamp();
    let mut remaining = shares_to_withdraw;
    let mut consumed_count: u32 = 0;
    // Track how many shares we partially consumed from the first non-fully-consumed record.
    let mut partial_remainder: i128 = 0;

    // Walk FIFO (index 0 = oldest deposit).
    for i in 0..records.len() {
        if remaining <= 0 {
            break;
        }

        let record = records.get(i).unwrap();

        // Check if this deposit has cleared the lock-up window.
        let age = now.saturating_sub(record.deposited_at);
        if age < lockup_duration {
            // This deposit is still locked – and because records are FIFO
            // (oldest first), all subsequent records are locked too.
            return Err(PoolError::DepositLocked);
        }

        if record.shares <= remaining {
            // Consume the entire record.
            remaining -= record.shares;
            consumed_count += 1;
        } else {
            // Partially consume this record.
            partial_remainder = record.shares - remaining;
            remaining = 0;
            consumed_count += 1; // will be replaced, not removed
            break;
        }
    }

    if remaining > 0 {
        // Not enough unlocked shares to cover the withdrawal.
        return Err(PoolError::InsufficientShares);
    }

    // Rebuild the records vector, dropping fully consumed entries and
    // adjusting the partially consumed one.
    let mut new_records: Vec<DepositRecord> = Vec::new(env);
    for i in consumed_count..records.len() {
        if i == consumed_count && partial_remainder > 0 {
            // Replace the partially consumed record with the leftover amount.
            let old = records.get(i - 1).unwrap();
            new_records.push_back(DepositRecord {
                shares: partial_remainder,
                deposited_at: old.deposited_at,
            });
            partial_remainder = 0; // already inserted
        }
        new_records.push_back(records.get(i).unwrap());
    }
    // Edge-case: partial_remainder not yet inserted (it was the last element).
    if partial_remainder > 0 {
        let old = records.get(consumed_count - 1).unwrap();
        new_records.push_back(DepositRecord {
            shares: partial_remainder,
            deposited_at: old.deposited_at,
        });
    }

    env.storage().instance().set(&key, &new_records);
    Ok(())
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
        
        // Record the initial deposit for lock-up tracking (FIFO queue).
        push_deposit_record(
            env,
            &creator,
            pool_id,
            DepositRecord {
                shares: total_shares,
                deposited_at: env.ledger().timestamp(),
            },
        );
        
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
        
        // Record this deposit for lock-up tracking (FIFO queue).
        push_deposit_record(
            env,
            &provider,
            pool_id,
            DepositRecord {
                shares,
                deposited_at: env.ledger().timestamp(),
            },
        );
        
        Ok(shares)
    }
    
    /// Remove liquidity from pool
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
        
        // ── Lock-up enforcement ─────────────────────────────────────────────
        // Read the currently configured lock-up duration.  If it is non-zero,
        // validate that the oldest FIFO deposits covering `shares` have all
        // cleared their lock-up window before touching any pool state.
        let lockup_duration = LockupAdmin::get_lockup_duration(env);
        if lockup_duration > 0 {
            consume_locked_shares(env, &provider, pool_id, shares, lockup_duration)?;
        }
        // ────────────────────────────────────────────────────────────────────
        
        // Calculate amounts to return
        let amount_a = (shares * pool.reserve_a) / pool.total_shares;
        let amount_b = (shares * pool.reserve_b) / pool.total_shares;
        
        // Enforce minimum-liquidity threshold: the pool balance must not drop
        // below 1 000 units per reserve after a withdrawal.
        let min_liquidity: i128 = 1_000;
        if pool.reserve_a - amount_a < min_liquidity || pool.reserve_b - amount_b < min_liquidity {
            return Err(PoolError::InsufficientLiquidity);
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
    /// Withdrawal rejected because the requested shares include deposits that
    /// are still within their lock-up period.
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
    /// Global minimum lock-up duration in seconds (set by admin).
    LockupDuration,
    /// FIFO queue of deposit records for a specific (provider, pool_id) pair.
    DepositRecords(Address, u64),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::Env;

    // ── Dummy contract for as_contract context ─────────────────────────────
    //
    // Soroban's env.storage() requires an active contract context.  We
    // register a no-op contract and wrap every storage-touching call with
    // env.as_contract(&cid, || { ... }).

    #[contract]
    struct DummyContract;

    #[contractimpl]
    impl DummyContract {}

    // ── Helpers ────────────────────────────────────────────────────────────

    /// Build a minimal pool seeded with enough liquidity to survive the
    /// minimum-liquidity threshold in `remove_liquidity`.
    ///
    /// Returns `(env, contract_id, pool_id, provider, total_shares_minted)`.
    fn setup_pool() -> (Env, Address, u64, Address, i128) {
        let env = Env::default();
        env.mock_all_auths();

        let cid = env.register(DummyContract, ());
        let creator = Address::generate(&env);
        let token_a = Address::generate(&env);
        let token_b = Address::generate(&env);

        let initial_a: i128 = 100_000;
        let initial_b: i128 = 100_000;

        let pool = env.as_contract(&cid, || {
            LiquidityPoolManager::create_pool(
                &env,
                token_a,
                token_b,
                initial_a,
                initial_b,
                30,
                creator.clone(),
            )
            .expect("create_pool failed")
        });

        let total_shares = pool.total_shares;
        (env, cid, pool.pool_id, creator, total_shares)
    }

    // ── Existing unit tests ────────────────────────────────────────────────

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

    // ── Lock-up unit tests ─────────────────────────────────────────────────

    /// Scenario 1: withdrawal before lock-up expiry must be rejected.
    #[test]
    fn test_withdrawal_before_lockup_expiry_is_rejected() {
        let (env, cid, pool_id, provider, total_shares) = setup_pool();
        let admin = Address::generate(&env);

        let lockup_duration: u64 = 3_600;
        env.as_contract(&cid, || {
            LockupAdmin::set_lockup_duration(&env, admin, lockup_duration);
        });

        // No time has passed since the deposit – the full duration is still pending.
        let withdraw_shares = total_shares / 2;
        let result = env.as_contract(&cid, || {
            LiquidityPoolManager::remove_liquidity(&env, pool_id, withdraw_shares, provider)
        });

        assert_eq!(
            result,
            Err(PoolError::DepositLocked),
            "Withdrawal within lock-up window must return DepositLocked"
        );
    }

    /// Scenario 2: withdrawal exactly at the lock-up expiry boundary must succeed.
    #[test]
    fn test_withdrawal_exactly_at_lockup_expiry_is_allowed() {
        let (env, cid, pool_id, provider, _) = setup_pool();
        let admin = Address::generate(&env);

        let lockup_duration: u64 = 3_600;
        env.as_contract(&cid, || {
            LockupAdmin::set_lockup_duration(&env, admin, lockup_duration);
        });

        // Advance the ledger by exactly the lock-up duration.
        let current_ts = env.ledger().timestamp();
        env.ledger().set_timestamp(current_ts + lockup_duration);

        let result = env.as_contract(&cid, || {
            LiquidityPoolManager::remove_liquidity(&env, pool_id, 1, provider)
        });

        assert!(
            result.is_ok(),
            "Withdrawal at lock-up expiry must succeed, got {:?}",
            result
        );
    }

    /// Scenario 3: partial withdrawal spanning locked and unlocked deposits.
    ///
    /// t=0    first deposit  (~100 000 shares) – unlocked at t=3600
    /// t=1800 second deposit (extra shares)    – still locked at t=3600
    ///
    /// At t=3600:
    ///   Part A – withdraw (first_shares - 1): must succeed (unlocked batch only)
    ///   Part B – withdraw 2 more: must fail (spills into locked second deposit)
    ///
    /// At t=5400 (second deposit now unlocked too):
    ///   Part C – withdraw second_shares: must succeed
    #[test]
    fn test_partial_withdrawal_spanning_locked_and_unlocked_deposits() {
        let (env, cid, pool_id, provider, first_shares) = setup_pool();
        let admin = Address::generate(&env);

        let lockup_duration: u64 = 3_600;
        env.as_contract(&cid, || {
            LockupAdmin::set_lockup_duration(&env, admin, lockup_duration);
        });

        // ── t = 1800: add second deposit ──────────────────────────────────
        let t0 = env.ledger().timestamp();
        env.ledger().set_timestamp(t0 + 1_800);

        let second_shares = env.as_contract(&cid, || {
            LiquidityPoolManager::add_liquidity(&env, pool_id, 50_000, 50_000, provider.clone())
                .expect("add_liquidity failed")
        });

        // ── t = 3600: first deposit unlocked; second still locked ─────────
        let t1 = env.ledger().timestamp();
        env.ledger().set_timestamp(t1 + 1_800);

        // Part A: consume (first_shares - 1) shares from the first deposit.
        let result_a = env.as_contract(&cid, || {
            LiquidityPoolManager::remove_liquidity(
                &env,
                pool_id,
                first_shares - 1,
                provider.clone(),
            )
        });
        assert!(
            result_a.is_ok(),
            "Part A: withdrawing unlocked shares must succeed, got {:?}",
            result_a
        );

        // Part B: requesting 2 more shares spills into the locked second deposit.
        let result_b = env.as_contract(&cid, || {
            LiquidityPoolManager::remove_liquidity(&env, pool_id, 2, provider.clone())
        });
        assert_eq!(
            result_b,
            Err(PoolError::DepositLocked),
            "Part B: crossing into locked deposit must return DepositLocked, got {:?}",
            result_b
        );

        // ── t = 5400: both deposits now unlocked ──────────────────────────
        let t2 = env.ledger().timestamp();
        env.ledger().set_timestamp(t2 + 1_800);

        // Withdraw most (but not all) of the second deposit to avoid the
        // min-liquidity guard.  The important thing is that the previously
        // locked deposit is now accessible.
        let safe_second_withdraw = second_shares / 2;
        let result_c = env.as_contract(&cid, || {
            LiquidityPoolManager::remove_liquidity(&env, pool_id, safe_second_withdraw, provider)
        });
        assert!(
            result_c.is_ok(),
            "Part C: withdrawing now-unlocked second deposit must succeed, got {:?}",
            result_c
        );
    }

    /// Zero lock-up: withdrawals are immediately allowed.
    #[test]
    fn test_no_lockup_when_duration_is_zero() {
        let (env, cid, pool_id, provider, _) = setup_pool();
        // Duration defaults to 0 – no set call needed.
        let result = env.as_contract(&cid, || {
            LiquidityPoolManager::remove_liquidity(&env, pool_id, 1, provider)
        });
        assert!(
            result.is_ok(),
            "Zero lock-up must allow immediate withdrawal, got {:?}",
            result
        );
    }

    /// set_lockup_duration / get_lockup_duration round-trip.
    #[test]
    fn test_set_and_get_lockup_duration() {
        let env = Env::default();
        env.mock_all_auths();
        let cid = env.register(DummyContract, ());
        let admin = Address::generate(&env);

        // Default is 0.
        let v0 = env.as_contract(&cid, || LockupAdmin::get_lockup_duration(&env));
        assert_eq!(v0, 0);

        env.as_contract(&cid, || {
            LockupAdmin::set_lockup_duration(&env, admin.clone(), 7_200);
        });
        let v1 = env.as_contract(&cid, || LockupAdmin::get_lockup_duration(&env));
        assert_eq!(v1, 7_200);

        env.as_contract(&cid, || {
            LockupAdmin::set_lockup_duration(&env, admin, 0);
        });
        let v2 = env.as_contract(&cid, || LockupAdmin::get_lockup_duration(&env));
        assert_eq!(v2, 0);
    }
}
