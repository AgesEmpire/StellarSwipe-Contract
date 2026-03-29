use soroban_sdk::{contracttype, Address, Env};

use crate::BridgeError;

const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolType {
    ConstantProduct,
    StableSwap,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidityPool {
    pub id: u64,
    pub asset_a: soroban_sdk::String,
    pub asset_b: soroban_sdk::String,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_lp_tokens: i128,
    pub fee_bps: u32,
    pub reward_bps: u32,
    pub pool_type: PoolType,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidityPosition {
    pub provider: Address,
    pub pool_id: u64,
    pub lp_tokens: i128,
    pub rewards_earned: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapResult {
    pub amount_in: i128,
    pub amount_out: i128,
    pub fee_paid: i128,
    pub new_reserve_in: i128,
    pub new_reserve_out: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolHealth {
    pub pool_id: u64,
    pub total_liquidity: i128,
    pub imbalance_bps: u32,
    pub utilization_bps: u32,
}

#[contracttype]
enum LiquidityKey {
    PoolCounter,
    Pool(u64),
    Position(Address, u64),
}

pub fn create_pool(
    env: &Env,
    asset_a: soroban_sdk::String,
    asset_b: soroban_sdk::String,
    pool_type: PoolType,
    fee_bps: u32,
    reward_bps: u32,
) -> Result<u64, BridgeError> {
    if asset_a == asset_b {
        return Err(BridgeError::InvalidOperation);
    }

    let pool_id = next_pool_id(env);
    let pool = LiquidityPool {
        id: pool_id,
        asset_a,
        asset_b,
        reserve_a: 0,
        reserve_b: 0,
        total_lp_tokens: 0,
        fee_bps,
        reward_bps,
        pool_type,
        created_at: env.ledger().timestamp(),
    };

    env.storage()
        .persistent()
        .set(&LiquidityKey::Pool(pool_id), &pool);
    Ok(pool_id)
}

pub fn add_liquidity(
    env: &Env,
    provider: Address,
    pool_id: u64,
    amount_a: i128,
    amount_b: i128,
) -> Result<i128, BridgeError> {
    if amount_a <= 0 || amount_b <= 0 {
        return Err(BridgeError::InvalidAmount);
    }

    let mut pool = get_pool(env, pool_id)?;
    let minted_lp = if pool.total_lp_tokens == 0 {
        integer_sqrt(amount_a * amount_b)
    } else {
        let lp_a = (amount_a * pool.total_lp_tokens) / pool.reserve_a;
        let lp_b = (amount_b * pool.total_lp_tokens) / pool.reserve_b;
        core::cmp::min(lp_a, lp_b)
    };

    if minted_lp <= 0 {
        return Err(BridgeError::InvalidAmount);
    }

    pool.reserve_a += amount_a;
    pool.reserve_b += amount_b;
    pool.total_lp_tokens += minted_lp;

    let mut position = get_position(env, provider.clone(), pool_id);
    position.lp_tokens += minted_lp;
    position.rewards_earned += reward_for_liquidity(amount_a + amount_b, pool.reward_bps);

    store_pool(env, &pool);
    store_position(env, &position);

    Ok(minted_lp)
}

pub fn remove_liquidity(
    env: &Env,
    provider: Address,
    pool_id: u64,
    lp_amount: i128,
) -> Result<(i128, i128, i128), BridgeError> {
    if lp_amount <= 0 {
        return Err(BridgeError::InvalidAmount);
    }

    let mut pool = get_pool(env, pool_id)?;
    let mut position = get_position(env, provider.clone(), pool_id);
    if position.lp_tokens < lp_amount || pool.total_lp_tokens <= 0 {
        return Err(BridgeError::InvalidAmount);
    }

    let amount_a = (pool.reserve_a * lp_amount) / pool.total_lp_tokens;
    let amount_b = (pool.reserve_b * lp_amount) / pool.total_lp_tokens;
    let reward = (position.rewards_earned * lp_amount) / position.lp_tokens;

    position.lp_tokens -= lp_amount;
    position.rewards_earned -= reward;
    pool.reserve_a -= amount_a;
    pool.reserve_b -= amount_b;
    pool.total_lp_tokens -= lp_amount;

    store_pool(env, &pool);
    store_position(env, &position);

    Ok((amount_a, amount_b, reward))
}

pub fn swap(
    env: &Env,
    _trader: Address,
    pool_id: u64,
    input_asset: soroban_sdk::String,
    amount_in: i128,
    min_amount_out: i128,
) -> Result<SwapResult, BridgeError> {
    if amount_in <= 0 {
        return Err(BridgeError::InvalidAmount);
    }

    let mut pool = get_pool(env, pool_id)?;
    let fee_paid = (amount_in * pool.fee_bps as i128) / BPS_DENOMINATOR;
    let amount_after_fee = amount_in - fee_paid;

    let (amount_out, new_reserve_in, new_reserve_out, input_is_a) = if input_asset == pool.asset_a {
        let out = quote_swap(
            pool.pool_type,
            amount_after_fee,
            pool.reserve_a,
            pool.reserve_b,
        )?;
        (
            out,
            pool.reserve_a + amount_after_fee,
            pool.reserve_b - out,
            true,
        )
    } else if input_asset == pool.asset_b {
        let out = quote_swap(
            pool.pool_type,
            amount_after_fee,
            pool.reserve_b,
            pool.reserve_a,
        )?;
        (
            out,
            pool.reserve_b + amount_after_fee,
            pool.reserve_a - out,
            false,
        )
    } else {
        return Err(BridgeError::InvalidOperation);
    };

    if amount_out < min_amount_out || amount_out <= 0 {
        return Err(BridgeError::InvalidOperation);
    }

    if input_is_a {
        pool.reserve_a = new_reserve_in;
        pool.reserve_b = new_reserve_out;
    } else {
        pool.reserve_b = new_reserve_in;
        pool.reserve_a = new_reserve_out;
    }

    store_pool(env, &pool);

    Ok(SwapResult {
        amount_in,
        amount_out,
        fee_paid,
        new_reserve_in,
        new_reserve_out,
    })
}

pub fn get_pool(env: &Env, pool_id: u64) -> Result<LiquidityPool, BridgeError> {
    env.storage()
        .persistent()
        .get(&LiquidityKey::Pool(pool_id))
        .ok_or(BridgeError::TransferNotFound)
}

pub fn get_position(env: &Env, provider: Address, pool_id: u64) -> LiquidityPosition {
    env.storage()
        .persistent()
        .get(&LiquidityKey::Position(provider.clone(), pool_id))
        .unwrap_or(LiquidityPosition {
            provider,
            pool_id,
            lp_tokens: 0,
            rewards_earned: 0,
        })
}

pub fn get_pool_health(env: &Env, pool_id: u64) -> Result<PoolHealth, BridgeError> {
    let pool = get_pool(env, pool_id)?;
    let total_liquidity = pool.reserve_a + pool.reserve_b;
    let larger = core::cmp::max(pool.reserve_a, pool.reserve_b);
    let smaller = core::cmp::min(pool.reserve_a, pool.reserve_b);
    let imbalance_bps = if larger == 0 {
        0
    } else {
        (((larger - smaller) * BPS_DENOMINATOR) / larger) as u32
    };
    let utilization_bps = if pool.total_lp_tokens == 0 || total_liquidity == 0 {
        0
    } else {
        ((pool.total_lp_tokens * BPS_DENOMINATOR) / total_liquidity) as u32
    };

    Ok(PoolHealth {
        pool_id,
        total_liquidity,
        imbalance_bps,
        utilization_bps,
    })
}

fn store_pool(env: &Env, pool: &LiquidityPool) {
    env.storage()
        .persistent()
        .set(&LiquidityKey::Pool(pool.id), pool);
}

fn store_position(env: &Env, position: &LiquidityPosition) {
    env.storage().persistent().set(
        &LiquidityKey::Position(position.provider.clone(), position.pool_id),
        position,
    );
}

fn next_pool_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&LiquidityKey::PoolCounter)
        .unwrap_or(0);
    let next = current + 1;
    env.storage()
        .persistent()
        .set(&LiquidityKey::PoolCounter, &next);
    next
}

fn reward_for_liquidity(total_added: i128, reward_bps: u32) -> i128 {
    (total_added * reward_bps as i128) / BPS_DENOMINATOR
}

fn quote_swap(
    pool_type: PoolType,
    amount_in: i128,
    reserve_in: i128,
    reserve_out: i128,
) -> Result<i128, BridgeError> {
    if reserve_in <= 0 || reserve_out <= 0 {
        return Err(BridgeError::InvalidOperation);
    }

    match pool_type {
        PoolType::ConstantProduct => {
            let numerator = amount_in * reserve_out;
            let denominator = reserve_in + amount_in;
            Ok(numerator / denominator)
        }
        PoolType::StableSwap => {
            let amount_out = core::cmp::min(amount_in, reserve_out);
            Ok(amount_out)
        }
    }
}

fn integer_sqrt(value: i128) -> i128 {
    if value <= 0 {
        return 0;
    }

    let mut x0 = value;
    let mut x1 = (x0 + value / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + value / x0) / 2;
    }
    x0
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract,
        testutils::{Address as _, Ledger as _},
    };

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(TestContract, ());
        (env, contract_id)
    }

    #[test]
    fn pool_add_remove_and_rewards_work() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let pool_id = create_pool(
                &env,
                soroban_sdk::String::from_str(&env, "wETH"),
                soroban_sdk::String::from_str(&env, "wUSDC"),
                PoolType::ConstantProduct,
                30,
                200,
            )
            .unwrap();

            let minted = add_liquidity(&env, provider.clone(), pool_id, 1_000, 1_000).unwrap();
            assert!(minted > 0);

            let position = get_position(&env, provider.clone(), pool_id);
            assert!(position.rewards_earned > 0);

            let (amount_a, amount_b, reward) =
                remove_liquidity(&env, provider, pool_id, minted / 2).unwrap();
            assert!(amount_a > 0 && amount_b > 0 && reward > 0);
        });
    }

    #[test]
    fn constant_product_swap_updates_reserves() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        let trader = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let pool_id = create_pool(
                &env,
                soroban_sdk::String::from_str(&env, "wETH"),
                soroban_sdk::String::from_str(&env, "wUSDC"),
                PoolType::ConstantProduct,
                30,
                0,
            )
            .unwrap();
            add_liquidity(&env, provider, pool_id, 1_000, 2_000).unwrap();

            let result = swap(
                &env,
                trader,
                pool_id,
                soroban_sdk::String::from_str(&env, "wETH"),
                100,
                1,
            )
            .unwrap();

            assert!(result.amount_out > 0);
            let pool = get_pool(&env, pool_id).unwrap();
            assert!(pool.reserve_a > 1_000);
            assert!(pool.reserve_b < 2_000);
        });
    }

    #[test]
    fn pool_health_detects_imbalance() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let pool_id = create_pool(
                &env,
                soroban_sdk::String::from_str(&env, "wETH"),
                soroban_sdk::String::from_str(&env, "wUSDC"),
                PoolType::StableSwap,
                5,
                50,
            )
            .unwrap();
            add_liquidity(&env, provider, pool_id, 500, 1_500).unwrap();

            let health = get_pool_health(&env, pool_id).unwrap();
            assert!(health.imbalance_bps > 0);
            assert!(health.total_liquidity > 0);
        });
    }
}
