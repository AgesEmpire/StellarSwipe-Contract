// Liquidity Pool Management and Optimization
// Mechanisms for managing and optimizing liquidity pools

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Vec, Symbol};

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

/// Result of impermanent loss calculation for a position
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ImpermanentLossResult {
    pub pool_id: u64,
    pub provider: Address,
    pub share_of_pool_bps: i128,
    pub current_value_a: i128,
    pub current_value_b: i128,
    pub hold_value_a: i128,
    pub hold_value_b: i128,
    pub impermanent_loss_a: i128,
    pub impermanent_loss_bps: i128,
}

/// Event payload emitted on withdrawal recording realized impermanent loss
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct WithdrawalILRecord {
    pub provider: Address,
    pub pool_id: u64,
    pub shares_withdrawn: i128,
    pub withdrawn_a: i128,
    pub withdrawn_b: i128,
    pub hold_basis_a: i128,
    pub hold_basis_b: i128,
    pub impermanent_loss_a: i128,
    pub impermanent_loss_bps: i128,
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
            provider: creator,
            pool_id,
            shares: total_shares,
            deposited_a: initial_a,
            deposited_b: initial_b,
            earned_fees: 0,
            created_at: env.ledger().timestamp(),
        };
        
        Self::store_position(env, &position);
        
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
        Self::update_position(env, provider, pool_id, shares, amount_a, amount_b);
        
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
        
        // Calculate amounts to return
        let amount_a = (shares * pool.reserve_a) / pool.total_shares;
        let amount_b = (shares * pool.reserve_b) / pool.total_shares;
        
        // Fetch position for IL calculation before reserves/shares are mutated
        let position = Self::get_position(env, &provider, pool_id)
            .unwrap_or(LiquidityPosition {
                provider: provider.clone(),
                pool_id,
                shares: 0,
                deposited_a: 0,
                deposited_b: 0,
                earned_fees: 0,
                created_at: 0,
            });
        
        // Compute and emit realized impermanent loss for this withdrawal
        if position.shares > 0 && position.shares >= shares {
            let hold_basis_a = shares
                .checked_mul(position.deposited_a)
                .and_then(|v| v.checked_div(position.shares))
                .unwrap_or(0);
            let hold_basis_b = shares
                .checked_mul(position.deposited_b)
                .and_then(|v| v.checked_div(position.shares))
                .unwrap_or(0);
            let price_a_per_b = if pool.reserve_b > 0 {
                pool.reserve_a.checked_div(pool.reserve_b).unwrap_or(0)
            } else {
                0
            };
            let hold_value_in_a = hold_basis_a
                .checked_add(
                    hold_basis_b.checked_mul(price_a_per_b).unwrap_or(0)
                )
                .unwrap_or(0);
            let withdrawn_value_in_a = amount_a
                .checked_add(
                    amount_b.checked_mul(price_a_per_b).unwrap_or(0)
                )
                .unwrap_or(0);
            let il_a = if hold_value_in_a >= withdrawn_value_in_a {
                hold_value_in_a
                    .checked_sub(withdrawn_value_in_a)
                    .unwrap_or(0)
            } else {
                let gain = withdrawn_value_in_a
                    .checked_sub(hold_value_in_a)
                    .unwrap_or(0);
                gain.checked_neg().unwrap_or(0)
            };
            let il_bps = if hold_value_in_a > 0 {
                let abs_il = if il_a >= 0 { il_a } else { il_a.checked_neg().unwrap_or(0) };
                let raw = abs_il
                    .checked_mul(10000)
                    .and_then(|v| v.checked_div(hold_value_in_a))
                    .unwrap_or(0);
                if il_a >= 0 { raw } else { raw.checked_neg().unwrap_or(0) }
            } else {
                0
            };
            
            let record = WithdrawalILRecord {
                provider: provider.clone(),
                pool_id,
                shares_withdrawn: shares,
                withdrawn_a: amount_a,
                withdrawn_b: amount_b,
                hold_basis_a,
                hold_basis_b,
                impermanent_loss_a: il_a,
                impermanent_loss_bps: il_bps,
            };
            
            env.events().publish(
                (Symbol::new(env, "liquidity_pool"), Symbol::new(env, "withdraw_il")),
                record,
            );
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
    
    /// Reduce position and proportionally reduce deposited amounts
    /// so that deposited_a/b reflect the hold basis for remaining shares.
    fn reduce_position(env: &Env, provider: &Address, pool_id: u64, shares: i128) {
        if let Some(mut position) = env
            .storage()
            .instance()
            .get::<DataKey, LiquidityPosition>(&DataKey::Position(provider.clone(), pool_id))
        {
            if position.shares > 0 && shares > 0 {
                let reduction_a = shares
                    .checked_mul(position.deposited_a)
                    .and_then(|v| v.checked_div(position.shares))
                    .unwrap_or(0);
                let reduction_b = shares
                    .checked_mul(position.deposited_b)
                    .and_then(|v| v.checked_div(position.shares))
                    .unwrap_or(0);
                position.deposited_a = position.deposited_a.saturating_sub(reduction_a);
                position.deposited_b = position.deposited_b.saturating_sub(reduction_b);
            }
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

    /// Compute impermanent loss/gain for a position given current pool state.
    ///
    /// Compares the current withdrawable value of the position's shares against
    /// the value of simply holding the originally deposited assets, both valued
    /// at the current pool price (token A as numeraire).
    ///
    /// **Two-asset only** – this pool design supports exactly two assets per pool.
    ///
    /// Uses checked arithmetic throughout and returns zero for all edge cases
    /// (zero total_shares, zero shares, zero deposited, zero reserve_b) instead of
    /// panicking.
    ///
    /// Gas cost: O(1) – a handful of i128 multiplications and divisions.
    pub fn compute_impermanent_loss(
        pool: &LiquidityPool,
        provider: Address,
        pool_id: u64,
        shares: i128,
        deposited_a: i128,
        deposited_b: i128,
    ) -> ImpermanentLossResult {
        if pool.total_shares == 0 || shares == 0 {
            return ImpermanentLossResult {
                pool_id,
                provider,
                share_of_pool_bps: 0,
                current_value_a: 0,
                current_value_b: 0,
                hold_value_a: deposited_a,
                hold_value_b: deposited_b,
                impermanent_loss_a: 0,
                impermanent_loss_bps: 0,
            };
        }

        let share_of_pool_bps = shares
            .checked_mul(10000)
            .and_then(|v| v.checked_div(pool.total_shares))
            .unwrap_or(0);

        // Current withdrawable amounts
        let current_value_a = shares
            .checked_mul(pool.reserve_a)
            .and_then(|v| v.checked_div(pool.total_shares))
            .unwrap_or(0);
        let current_value_b = shares
            .checked_mul(pool.reserve_b)
            .and_then(|v| v.checked_div(pool.total_shares))
            .unwrap_or(0);

        // Current price of token B in terms of token A: reserve_a / reserve_b
        let price_a_per_b = if pool.reserve_b > 0 {
            pool.reserve_a
                .checked_div(pool.reserve_b)
                .unwrap_or(0)
        } else {
            0
        };

        // Value of LP position in terms of token A
        let pool_value_in_a = current_value_a
            .checked_add(
                current_value_b
                    .checked_mul(price_a_per_b)
                    .unwrap_or(0),
            )
            .unwrap_or(0);

        // Value of simply holding deposited assets in terms of token A
        let hold_value_in_a = deposited_a
            .checked_add(
                deposited_b
                    .checked_mul(price_a_per_b)
                    .unwrap_or(0),
            )
            .unwrap_or(0);

        // Impermanent loss (positive = loss, negative = gain)
        let il_a = if hold_value_in_a >= pool_value_in_a {
            hold_value_in_a
                .checked_sub(pool_value_in_a)
                .unwrap_or(0)
        } else {
            // Impermanent gain: pool value exceeds hold value
            let gain = pool_value_in_a
                .checked_sub(hold_value_in_a)
                .unwrap_or(0);
            gain.checked_neg().unwrap_or(0)
        };

        let il_bps = if hold_value_in_a > 0 {
            let abs_il = if il_a >= 0 { il_a } else { il_a.checked_neg().unwrap_or(0) };
            let raw = abs_il
                .checked_mul(10000)
                .and_then(|v| v.checked_div(hold_value_in_a))
                .unwrap_or(0);
            if il_a >= 0 { raw } else { raw.checked_neg().unwrap_or(0) }
        } else {
            0
        };

        ImpermanentLossResult {
            pool_id,
            provider,
            share_of_pool_bps,
            current_value_a,
            current_value_b,
            hold_value_a: deposited_a,
            hold_value_b: deposited_b,
            impermanent_loss_a: il_a,
            impermanent_loss_bps: il_bps,
        }
    }

    /// View entrypoint: compute current impermanent loss/gain for a provider's position.
    ///
    /// Returns an `ImpermanentLossResult` comparing the current withdrawable
    /// value against the hold-only baseline.
    ///
    /// Gas cost: O(1) – reads two storage entries (pool + position) and performs
    /// a handful of i128 arithmetic operations.
    pub fn get_impermanent_loss(
        env: &Env,
        provider: Address,
        pool_id: u64,
    ) -> Result<ImpermanentLossResult, PoolError> {
        let position = Self::get_position(env, &provider, pool_id)
            .ok_or(PoolError::PositionNotFound)?;
        let pool = Self::get_pool(env, pool_id)
            .ok_or(PoolError::PoolNotFound)?;

        Ok(Self::compute_impermanent_loss(
            &pool,
            provider,
            pool_id,
            position.shares,
            position.deposited_a,
            position.deposited_b,
        ))
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
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&Env::default()),
            token_b: Address::generate(&Env::default()),
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
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&Env::default()),
            token_b: Address::generate(&Env::default()),
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

    // ---------------------------------------------------------------------------
    // Impermanent loss tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_impermanent_loss_no_price_change() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 1_000_000,
            reserve_b: 1_000_000,
            total_shares: 1_000_000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 100_000, 100_000, 100_000,
        );

        // Price 1:1 → withdrawable = deposited = hold → zero IL
        assert_eq!(result.current_value_a, 100_000);
        assert_eq!(result.current_value_b, 100_000);
        assert_eq!(result.hold_value_a, 100_000);
        assert_eq!(result.hold_value_b, 100_000);
        assert_eq!(result.impermanent_loss_a, 0);
        assert_eq!(result.impermanent_loss_bps, 0);
        assert_eq!(result.share_of_pool_bps, 1000); // 10%
    }

    #[test]
    fn test_impermanent_loss_favorable_price() {
        // Token A 4× relative to token B (2 000 000 / 500 000 = 4)
        // Constant product maintained: 2_000_000 * 500_000 = 10^12
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 2_000_000,
            reserve_b: 500_000,
            total_shares: 1_000_000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 100_000, 100_000, 100_000,
        );

        // Withdrawable: 200_000 A, 50_000 B (price of B in A = 4)
        // Pool value: 200_000 + 50_000*4 = 400_000
        // Hold value: 100_000 + 100_000*4 = 500_000
        // IL = 100_000 A (positive = impermanent loss)
        assert_eq!(result.current_value_a, 200_000);
        assert_eq!(result.current_value_b, 50_000);
        assert_eq!(result.impermanent_loss_a, 100_000);
        assert_eq!(result.impermanent_loss_bps, 2000); // 20% = 2000 bps
    }

    #[test]
    fn test_impermanent_loss_unfavorable_price() {
        // Token A 0.25× relative to token B (500 000 / 2 000 000 = 0.25)
        // Constant product maintained: 500_000 * 2_000_000 = 10^12
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 500_000,
            reserve_b: 2_000_000,
            total_shares: 1_000_000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 100_000, 100_000, 100_000,
        );

        // Withdrawable: 50_000 A, 200_000 B (price of B in A = 0.25)
        // Pool value: 50_000 + 200_000*0.25 = 100_000
        // Hold value: 100_000 + 100_000*0.25 = 125_000
        // IL = 25_000 A (positive = impermanent loss)
        assert_eq!(result.current_value_a, 50_000);
        assert_eq!(result.current_value_b, 200_000);
        assert_eq!(result.impermanent_loss_a, 25_000);
        assert_eq!(result.impermanent_loss_bps, 2000); // 20% = 2000 bps
    }

    #[test]
    fn test_impermanent_loss_moderate_price_change() {
        // 2× price change → ~5.73% IL
        // sqrt(2) ≈ 1_414_214 / 1_000_000 scaled
        // pool: (1_414_214, 707_107) with k ≈ 10^12
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 1_414_214,
            reserve_b: 707_107,
            total_shares: 1_000_000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 100_000, 100_000, 100_000,
        );

        // Price B in A ≈ 2.0, pool value ≈ 282_843 A, hold = 300_000 A
        // IL ≈ 17_159 A, IL bps ≈ 571 (~5.71%)
        assert_eq!(result.current_value_a, 141_421);
        assert_eq!(result.current_value_b, 70_710);
        assert_eq!(result.impermanent_loss_a, 17_159);
        assert_eq!(result.impermanent_loss_bps, 571);
    }

    #[test]
    fn test_impermanent_loss_zero_shares() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 1_000_000,
            reserve_b: 1_000_000,
            total_shares: 1_000_000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 0, 100_000, 100_000,
        );

        assert_eq!(result.impermanent_loss_a, 0);
        assert_eq!(result.impermanent_loss_bps, 0);
        assert_eq!(result.current_value_a, 0);
        assert_eq!(result.current_value_b, 0);
    }

    #[test]
    fn test_impermanent_loss_zero_pool() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 0,
            reserve_b: 0,
            total_shares: 0,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 1000, 1000, 1000,
        );

        assert_eq!(result.impermanent_loss_a, 0);
        assert_eq!(result.impermanent_loss_bps, 0);
        assert_eq!(result.current_value_a, 0);
        assert_eq!(result.current_value_b, 0);
    }

    #[test]
    fn test_impermanent_loss_zero_deposited() {
        // Shares from fees/compounding with no personal deposit → impermanent gain
        let env = Env::default();
        let provider = Address::generate(&env);
        let pool = LiquidityPool {
            pool_id: 1,
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            reserve_a: 1_000_000,
            reserve_b: 500_000,
            total_shares: 1_000_000,
            fee_rate: 30,
            created_at: 0,
            last_rebalanced: 0,
        };

        let result = LiquidityPoolManager::compute_impermanent_loss(
            &pool, provider, 1, 100_000, 0, 0,
        );

        // Pool value: 200_000 A, hold value: 0 A → IL = -200_000 (gain)
        assert_eq!(result.current_value_a, 100_000);
        assert_eq!(result.current_value_b, 50_000);
        assert_eq!(result.impermanent_loss_a, -200_000);
        assert_eq!(result.impermanent_loss_bps, 0); // 0 hold → 0 bps (can't compute %)
    }
}
