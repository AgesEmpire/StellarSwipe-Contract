use core::cmp::Ordering;

use shared::safe_math::checked_mul_div_floor;
use soroban_sdk::{contracttype, Address, Env};
use stellar_swipe_common::BASIS_POINTS_DENOMINATOR_I128;

use crate::BridgeError;

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
    /// Canonical `(asset_a, asset_b)` pair -> pool id (Issue #1313).
    PoolByPair(soroban_sdk::String, soroban_sdk::String),
}

/// Returns `(asset_a, asset_b)` in canonical order (Issue #1313).
///
/// **Rule:** the asset identifier that sorts first in ascending
/// lexicographic order of its raw bytes becomes `asset_a`. This is the
/// ordering Soroban defines for `String`, so it is deterministic across hosts
/// and SDK versions. Comparison is exact and case-sensitive: identifiers are
/// not trimmed or case-folded, so `"wETH"` and `"weth"` are different assets.
///
/// Either input order yields the same result, so one unordered pair maps to
/// exactly one canonical key. Identical identifiers are rejected with
/// [`BridgeError::IdenticalPoolAssets`].
pub fn canonical_pair(
    asset_a: soroban_sdk::String,
    asset_b: soroban_sdk::String,
) -> Result<(soroban_sdk::String, soroban_sdk::String), BridgeError> {
    match asset_a.cmp(&asset_b) {
        Ordering::Less => Ok((asset_a, asset_b)),
        Ordering::Greater => Ok((asset_b, asset_a)),
        Ordering::Equal => Err(BridgeError::IdenticalPoolAssets),
    }
}

/// Pool id registered for the unordered pair `{asset_a, asset_b}`, if any.
///
/// Input order does not matter. Returns `None` for identical identifiers,
/// which can never form a pool.
pub fn find_pool_for_pair(
    env: &Env,
    asset_a: soroban_sdk::String,
    asset_b: soroban_sdk::String,
) -> Option<u64> {
    let (asset_a, asset_b) = canonical_pair(asset_a, asset_b).ok()?;
    env.storage()
        .persistent()
        .get(&LiquidityKey::PoolByPair(asset_a, asset_b))
}

/// Creates a pool for the unordered pair `{asset_a, asset_b}`.
///
/// The assets are stored in canonical order (see [`canonical_pair`]), so
/// the stored `asset_a`/`asset_b` and the `amount_a`/`amount_b` arguments of
/// [`add_liquidity`] follow that order rather than the caller's argument
/// order.
///
/// # Errors
///
/// - [`BridgeError::IdenticalPoolAssets`] — both identifiers are equal.
/// - [`BridgeError::PoolAlreadyExists`] — a pool for the same pair exists,
///   in either order.
pub fn create_pool(
    env: &Env,
    asset_a: soroban_sdk::String,
    asset_b: soroban_sdk::String,
    pool_type: PoolType,
    fee_bps: u32,
    reward_bps: u32,
) -> Result<u64, BridgeError> {
    let (asset_a, asset_b) = canonical_pair(asset_a, asset_b)?;
    let pair_key = LiquidityKey::PoolByPair(asset_a.clone(), asset_b.clone());
    if env.storage().persistent().has(&pair_key) {
        return Err(BridgeError::PoolAlreadyExists);
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
    env.storage().persistent().set(&pair_key, &pool_id);
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
    let minted_lp = compute_mint_shares(
        pool.reserve_a,
        pool.reserve_b,
        pool.total_lp_tokens,
        amount_a,
        amount_b,
    )?;

    // Every new balance is computed (checked) before anything is written, so
    // an overflow leaves reserves, supply and the position untouched.
    let mut position = get_position(env, provider.clone(), pool_id);
    let reward = amount_a
        .checked_add(amount_b)
        .and_then(|total| {
            checked_mul_div_floor(
                total,
                pool.reward_bps as i128,
                BASIS_POINTS_DENOMINATOR_I128,
            )
        })
        .ok_or(ShareMintError::Overflow)?;
    let reserve_a = checked(pool.reserve_a.checked_add(amount_a))?;
    let reserve_b = checked(pool.reserve_b.checked_add(amount_b))?;
    let total_lp_tokens = checked(pool.total_lp_tokens.checked_add(minted_lp))?;
    let lp_tokens = checked(position.lp_tokens.checked_add(minted_lp))?;
    let rewards_earned = checked(position.rewards_earned.checked_add(reward))?;

    pool.reserve_a = reserve_a;
    pool.reserve_b = reserve_b;
    pool.total_lp_tokens = total_lp_tokens;
    position.lp_tokens = lp_tokens;
    position.rewards_earned = rewards_earned;

    store_pool(env, &pool);
    store_position(env, &position);

    Ok(minted_lp)
}

/// Why a share-mint calculation was rejected (Issue #1315).
///
/// Converted to a [`BridgeError`] at the module boundary, so the public error
/// ABI is unchanged; tests use it to pin the exact rejection reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShareMintError {
    /// A deposit amount is zero or negative.
    InvalidDeposit,
    /// Supply and reserves disagree: LP supply exists with an empty reserve,
    /// or reserves exist with no LP supply backing them.
    InconsistentPool,
    /// An intermediate product or a resulting balance does not fit in `i128`.
    Overflow,
    /// The deposit is too small to mint a single whole LP token.
    ZeroShares,
}

impl From<ShareMintError> for BridgeError {
    fn from(err: ShareMintError) -> Self {
        match err {
            ShareMintError::InconsistentPool => BridgeError::InvalidOperation,
            ShareMintError::InvalidDeposit
            | ShareMintError::Overflow
            | ShareMintError::ZeroShares => BridgeError::InvalidAmount,
        }
    }
}

fn checked(value: Option<i128>) -> Result<i128, ShareMintError> {
    value.ok_or(ShareMintError::Overflow)
}

/// LP tokens minted for depositing `amount_a`/`amount_b` into a pool with the
/// given reserves and supply (Issue #1315). Pure: reads and writes no storage.
///
/// * **First deposit** (`total_lp == 0`): `floor(sqrt(amount_a * amount_b))`.
/// * **Later deposits:** `min(floor(amount_a * total_lp / reserve_a),
///   floor(amount_b * total_lp / reserve_b))`.
///
/// # Rounding direction
///
/// Every division rounds **down**, and the smaller of the two ratios wins, so
/// a depositor never receives more LP tokens than their contribution backs.
/// The rounding remainder (and any excess of the over-supplied asset) stays in
/// the pool and accrues to existing holders. The value of one LP token can
/// therefore never fall because of a deposit, and splitting a deposit into
/// smaller ones never mints more than depositing it at once.
///
/// # Supported range
///
/// All intermediate products are checked: any input whose product
/// (`amount_a * amount_b`, or `amount * total_lp`) does not fit in `i128` is
/// rejected with [`ShareMintError::Overflow`] instead of wrapping or
/// panicking. A deposit that would round to zero LP tokens is rejected with
/// [`ShareMintError::ZeroShares`].
pub fn compute_mint_shares(
    reserve_a: i128,
    reserve_b: i128,
    total_lp: i128,
    amount_a: i128,
    amount_b: i128,
) -> Result<i128, ShareMintError> {
    if amount_a <= 0 || amount_b <= 0 {
        return Err(ShareMintError::InvalidDeposit);
    }

    let minted = if total_lp == 0 {
        if reserve_a != 0 || reserve_b != 0 {
            return Err(ShareMintError::InconsistentPool);
        }
        integer_sqrt(checked(amount_a.checked_mul(amount_b))?)
    } else {
        if total_lp < 0 || reserve_a <= 0 || reserve_b <= 0 {
            return Err(ShareMintError::InconsistentPool);
        }
        let lp_a = checked(checked_mul_div_floor(amount_a, total_lp, reserve_a))?;
        let lp_b = checked(checked_mul_div_floor(amount_b, total_lp, reserve_b))?;
        core::cmp::min(lp_a, lp_b)
    };

    if minted <= 0 {
        return Err(ShareMintError::ZeroShares);
    }
    Ok(minted)
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
    let fee_paid = (amount_in * pool.fee_bps as i128) / BASIS_POINTS_DENOMINATOR_I128;
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
        (((larger - smaller) * BASIS_POINTS_DENOMINATOR_I128) / larger) as u32
    };
    let utilization_bps = if pool.total_lp_tokens == 0 || total_liquidity == 0 {
        0
    } else {
        ((pool.total_lp_tokens * BASIS_POINTS_DENOMINATOR_I128) / total_liquidity) as u32
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

/// `floor(sqrt(value))`, exact for every non-negative `i128`.
///
/// Newton's iteration runs in `u128` from a starting guess `>= sqrt(value)`,
/// so `x + value / x` cannot overflow even for `value == i128::MAX`.
fn integer_sqrt(value: i128) -> i128 {
    if value <= 0 {
        return 0;
    }

    let n = value as u128;
    let bits = 128 - n.leading_zeros();
    let mut x0: u128 = 1 << bits.div_ceil(2);
    let mut x1 = (x0 + n / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + n / x0) / 2;
    }
    x0 as i128
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

    // ── Issue #1315: overflow-safe, deterministic share minting ────────────

    fn new_pool(env: &Env, reward_bps: u32) -> u64 {
        create_pool(
            env,
            soroban_sdk::String::from_str(env, "wETH"),
            soroban_sdk::String::from_str(env, "wUSDC"),
            PoolType::ConstantProduct,
            30,
            reward_bps,
        )
        .unwrap()
    }

    #[test]
    fn integer_sqrt_is_exact_floor_up_to_i128_max() {
        for v in [0i128, 1, 2, 3, 4, 8, 9, 10, 99, 100, 101, 1 << 64] {
            let r = integer_sqrt(v);
            assert!(r * r <= v && (r + 1) * (r + 1) > v, "sqrt({v}) = {r}");
        }
        let r = integer_sqrt(i128::MAX);
        assert_eq!(r, 13_043_817_825_332_782_212);
        assert!(r.checked_mul(r).unwrap() <= i128::MAX);
        assert!((r + 1).checked_mul(r + 1).is_none());
    }

    #[test]
    fn first_deposit_rounds_geometric_mean_down() {
        // sqrt(2 * 3) = 2.449.. -> 2; the remainder stays in the pool.
        assert_eq!(compute_mint_shares(0, 0, 0, 2, 3), Ok(2));
        assert_eq!(compute_mint_shares(0, 0, 0, 1_000, 1_000), Ok(1_000));
        assert_eq!(compute_mint_shares(0, 0, 0, 1, 1), Ok(1));
    }

    #[test]
    fn proportional_deposit_rounds_down_and_uses_smaller_ratio() {
        // 2 * 2 / 3 = 1.33.. -> 1.
        assert_eq!(compute_mint_shares(3, 3, 2, 2, 2), Ok(1));
        // a-ratio gives 10 * 100 / 7 = 142, b-ratio 10 * 100 / 9 = 111.
        assert_eq!(compute_mint_shares(7, 9, 100, 10, 10), Ok(111));
        // Exact division has no remainder.
        assert_eq!(compute_mint_shares(1_000, 2_000, 1_000, 100, 200), Ok(100));
    }

    #[test]
    fn minted_shares_never_exceed_backing() {
        let cases: [(i128, i128, i128, i128, i128); 5] = [
            (3, 3, 2, 2, 2),
            (7, 9, 100, 10, 10),
            (1_000_003, 999_983, 999_991, 12_345, 67_891),
            (17, 1_000_000_007, 131, 5, 999_999_937),
            (1 << 100, 1 << 100, 1 << 60, (1 << 64) + 7, 1 << 64),
        ];
        for (ra, rb, t, a, b) in cases {
            let minted = compute_mint_shares(ra, rb, t, a, b).unwrap();
            // minted / t <= a / ra and minted / t <= b / rb.
            assert!(minted <= checked_mul_div_floor(a, t, ra).unwrap());
            assert!(minted <= checked_mul_div_floor(b, t, rb).unwrap());
        }
    }

    #[test]
    fn minting_is_deterministic() {
        let first = compute_mint_shares(1_000_003, 999_983, 999_991, 12_345, 67_891);
        for _ in 0..10 {
            assert_eq!(
                compute_mint_shares(1_000_003, 999_983, 999_991, 12_345, 67_891),
                first
            );
        }
    }

    #[test]
    fn dust_deposit_minting_zero_shares_is_rejected() {
        assert_eq!(
            compute_mint_shares(1_000_000, 1_000_000, 1, 1, 1),
            Err(ShareMintError::ZeroShares)
        );
        // One side minting zero is enough to reject.
        assert_eq!(
            compute_mint_shares(1_000, 1_000_000, 1_000, 1_000, 1),
            Err(ShareMintError::ZeroShares)
        );
    }

    #[test]
    fn non_positive_amounts_and_inconsistent_pools_are_rejected() {
        assert_eq!(
            compute_mint_shares(0, 0, 0, 0, 5),
            Err(ShareMintError::InvalidDeposit)
        );
        assert_eq!(
            compute_mint_shares(0, 0, 0, 5, -1),
            Err(ShareMintError::InvalidDeposit)
        );
        // Supply without reserves would divide by zero.
        assert_eq!(
            compute_mint_shares(0, 10, 10, 5, 5),
            Err(ShareMintError::InconsistentPool)
        );
        // Orphaned reserves with no supply would be captured by the first minter.
        assert_eq!(
            compute_mint_shares(10, 10, 0, 5, 5),
            Err(ShareMintError::InconsistentPool)
        );
    }

    #[test]
    fn values_near_i128_limits() {
        // Largest first deposits whose product still fits succeed exactly.
        assert_eq!(compute_mint_shares(0, 0, 0, 1 << 63, 1 << 63), Ok(1 << 63));
        assert_eq!(
            compute_mint_shares(0, 0, 0, i128::MAX, 1),
            Ok(13_043_817_825_332_782_212)
        );
        // One past the limit overflows instead of wrapping or panicking.
        assert_eq!(
            compute_mint_shares(0, 0, 0, i128::MAX, 2),
            Err(ShareMintError::Overflow)
        );
        assert_eq!(
            compute_mint_shares(0, 0, 0, 1 << 64, 1 << 64),
            Err(ShareMintError::Overflow)
        );
        // Proportional path: amount * total_lp overflows.
        assert_eq!(
            compute_mint_shares(i128::MAX, i128::MAX, 4, i128::MAX / 2, i128::MAX / 2),
            Err(ShareMintError::Overflow)
        );
        // A product of exactly i128::MAX - 1 fits and divides exactly.
        assert_eq!(
            compute_mint_shares(
                i128::MAX - 1,
                i128::MAX - 1,
                2,
                i128::MAX / 2,
                i128::MAX / 2
            ),
            Ok(1)
        );
        // Representable but below one share: rejected as zero, not overflow.
        assert_eq!(
            compute_mint_shares(i128::MAX, i128::MAX, 1, i128::MAX / 2, i128::MAX / 2),
            Err(ShareMintError::ZeroShares)
        );
        assert_eq!(
            compute_mint_shares(1 << 100, 1 << 100, 1 << 26, 1 << 100, 1 << 100),
            Ok(1 << 26)
        );
    }

    #[test]
    fn split_deposits_never_mint_more_than_one_deposit() {
        let (env, contract_id) = setup();
        let seed = Address::generate(&env);
        let whale = Address::generate(&env);
        let splitter = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let single = new_pool(&env, 0);
            // A second pool for the same pair is rejected (Issue #1313).
            let split = create_pool(
                &env,
                soroban_sdk::String::from_str(&env, "wBTC"),
                soroban_sdk::String::from_str(&env, "wDAI"),
                PoolType::ConstantProduct,
                30,
                0,
            )
            .unwrap();
            add_liquidity(&env, seed.clone(), single, 1_000_003, 999_983).unwrap();
            add_liquidity(&env, seed.clone(), split, 1_000_003, 999_983).unwrap();

            let one_shot = add_liquidity(&env, whale.clone(), single, 70_007, 70_001).unwrap();
            let mut pieces = 0i128;
            for _ in 0..7 {
                pieces += add_liquidity(&env, splitter.clone(), split, 10_001, 10_000).unwrap();
                // Supply never outgrows the reserves backing it.
                let pool = get_pool(&env, split).unwrap();
                let lp = pool.total_lp_tokens as u128;
                assert!(lp * lp <= (pool.reserve_a as u128) * (pool.reserve_b as u128));
            }
            assert!(pieces <= one_shot, "split {pieces} > single {one_shot}");
            assert_eq!(
                get_position(&env, splitter.clone(), split).lp_tokens,
                pieces
            );
        });
    }

    #[test]
    fn rejected_mint_leaves_pool_and_position_unchanged() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let pool_id = new_pool(&env, 200);
            // sqrt(1_000_000 * 1) = 1_000 LP tokens: each one backs 1_000 units of A.
            assert_eq!(
                add_liquidity(&env, provider.clone(), pool_id, 1_000_000, 1),
                Ok(1_000)
            );
            let pool_before = get_pool(&env, pool_id).unwrap();
            let position_before = get_position(&env, provider.clone(), pool_id);

            // Zero-share dust deposit: 999 * 1_000 / 1_000_000 rounds to 0.
            assert_eq!(
                add_liquidity(&env, provider.clone(), pool_id, 999, 1),
                Err(BridgeError::InvalidAmount)
            );
            // Overflowing intermediate product.
            assert_eq!(
                add_liquidity(&env, provider.clone(), pool_id, i128::MAX, i128::MAX),
                Err(BridgeError::InvalidAmount)
            );

            assert_eq!(get_pool(&env, pool_id).unwrap(), pool_before);
            assert_eq!(
                get_position(&env, provider.clone(), pool_id),
                position_before
            );
        });
    }

    #[test]
    fn near_limit_first_deposit_succeeds() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let pool_id = new_pool(&env, 0);
            let minted = add_liquidity(&env, provider.clone(), pool_id, i128::MAX - 10, 1).unwrap();
            assert_eq!(minted, 13_043_817_825_332_782_212);
            let pool = get_pool(&env, pool_id).unwrap();
            assert_eq!(pool.reserve_a, i128::MAX - 10);
            assert_eq!(pool.total_lp_tokens, minted);
        });
    }

    #[test]
    fn reserve_overflow_is_rejected_before_any_write() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        env.as_contract(&contract_id, || {
            let pool_id = new_pool(&env, 0);
            // reserve_a sits just below i128::MAX with a single LP token.
            let mut pool = get_pool(&env, pool_id).unwrap();
            pool.reserve_a = i128::MAX - 10;
            pool.reserve_b = 1;
            pool.total_lp_tokens = 1;
            store_pool(&env, &pool);
            let position_before = get_position(&env, provider.clone(), pool_id);

            // The mint itself is valid (one share, no product overflow)...
            assert_eq!(
                compute_mint_shares(i128::MAX - 10, 1, 1, i128::MAX - 10, 1),
                Ok(1)
            );
            // ...but reserve_a + amount_a cannot be represented.
            assert_eq!(
                add_liquidity(&env, provider.clone(), pool_id, i128::MAX - 10, 1),
                Err(BridgeError::InvalidAmount)
            );
            assert_eq!(get_pool(&env, pool_id).unwrap(), pool);
            assert_eq!(
                get_position(&env, provider.clone(), pool_id),
                position_before
            );
        });
    }

    // ── Issue #1313: canonical asset ordering at pool creation ─────────────

    fn s(env: &Env, value: &str) -> soroban_sdk::String {
        soroban_sdk::String::from_str(env, value)
    }

    fn pool_for(env: &Env, asset_a: &str, asset_b: &str) -> Result<u64, BridgeError> {
        create_pool(
            env,
            s(env, asset_a),
            s(env, asset_b),
            PoolType::ConstantProduct,
            30,
            0,
        )
    }

    fn stored_pool_count(env: &Env) -> u64 {
        env.storage()
            .persistent()
            .get(&LiquidityKey::PoolCounter)
            .unwrap_or(0)
    }

    #[test]
    fn canonical_pair_is_independent_of_input_order() {
        let env = Env::default();
        let cases = [
            // (smaller, larger) by raw byte order.
            ("wETH", "wUSDC"),
            // A strict prefix sorts first.
            ("wBTC", "wBTCx"),
            // Case-sensitive: 'W' (0x57) < 'w' (0x77); no case folding.
            ("WETH", "wETH"),
            ("A", "B"),
        ];
        for (lo, hi) in cases {
            let expected = (s(&env, lo), s(&env, hi));
            assert_eq!(
                canonical_pair(s(&env, lo), s(&env, hi)),
                Ok(expected.clone())
            );
            assert_eq!(canonical_pair(s(&env, hi), s(&env, lo)), Ok(expected));
        }
    }

    #[test]
    fn canonical_pair_rejects_identical_assets() {
        let env = Env::default();
        assert_eq!(
            canonical_pair(s(&env, "wETH"), s(&env, "wETH")),
            Err(BridgeError::IdenticalPoolAssets)
        );
        assert_eq!(
            canonical_pair(s(&env, ""), s(&env, "")),
            Err(BridgeError::IdenticalPoolAssets)
        );
    }

    #[test]
    fn reversed_inputs_are_stored_in_canonical_order() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let pool_id = pool_for(&env, "wUSDC", "wETH").unwrap();
            let pool = get_pool(&env, pool_id).unwrap();
            assert_eq!(pool.asset_a, s(&env, "wETH"));
            assert_eq!(pool.asset_b, s(&env, "wUSDC"));
        });
    }

    #[test]
    fn reversed_pair_cannot_create_second_pool() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let pool_id = pool_for(&env, "wETH", "wUSDC").unwrap();
            let pool_before = get_pool(&env, pool_id).unwrap();

            assert_eq!(
                pool_for(&env, "wUSDC", "wETH"),
                Err(BridgeError::PoolAlreadyExists)
            );
            // The same order is a duplicate too.
            assert_eq!(
                pool_for(&env, "wETH", "wUSDC"),
                Err(BridgeError::PoolAlreadyExists)
            );
            // Pool type and fees do not make a pair distinct.
            assert_eq!(
                create_pool(
                    &env,
                    s(&env, "wUSDC"),
                    s(&env, "wETH"),
                    PoolType::StableSwap,
                    5,
                    100,
                ),
                Err(BridgeError::PoolAlreadyExists)
            );

            // Rejections write nothing: no id consumed, original pool intact.
            assert_eq!(stored_pool_count(&env), 1);
            assert_eq!(get_pool(&env, pool_id).unwrap(), pool_before);
            assert_eq!(
                get_pool(&env, pool_id + 1),
                Err(BridgeError::TransferNotFound)
            );
        });
    }

    #[test]
    fn identical_assets_cannot_create_pool() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            assert_eq!(
                pool_for(&env, "wETH", "wETH"),
                Err(BridgeError::IdenticalPoolAssets)
            );
            assert_eq!(stored_pool_count(&env), 0);
            assert_eq!(
                find_pool_for_pair(&env, s(&env, "wETH"), s(&env, "wETH")),
                None
            );
            // The rejected attempt did not consume pool id 1.
            assert_eq!(pool_for(&env, "wETH", "wUSDC"), Ok(1));
        });
    }

    #[test]
    fn distinct_pairs_get_distinct_pools() {
        let (env, contract_id) = setup();
        env.as_contract(&contract_id, || {
            let eth_usdc = pool_for(&env, "wETH", "wUSDC").unwrap();
            let btc_eth = pool_for(&env, "wETH", "wBTC").unwrap();
            let btc_usdc = pool_for(&env, "wUSDC", "wBTC").unwrap();
            // Pairs sharing one asset, or differing only by case, are distinct.
            let weth_upper = pool_for(&env, "WETH", "wUSDC").unwrap();
            assert_eq!((eth_usdc, btc_eth, btc_usdc, weth_upper), (1, 2, 3, 4));

            for (a, b, id) in [
                ("wETH", "wUSDC", eth_usdc),
                ("wBTC", "wETH", btc_eth),
                ("wBTC", "wUSDC", btc_usdc),
                ("WETH", "wUSDC", weth_upper),
            ] {
                assert_eq!(find_pool_for_pair(&env, s(&env, a), s(&env, b)), Some(id));
                assert_eq!(find_pool_for_pair(&env, s(&env, b), s(&env, a)), Some(id));
                let pool = get_pool(&env, id).unwrap();
                assert_eq!((pool.asset_a, pool.asset_b), (s(&env, a), s(&env, b)));
            }
            assert_eq!(
                find_pool_for_pair(&env, s(&env, "wDAI"), s(&env, "wETH")),
                None
            );
        });
    }

    #[test]
    fn swap_and_deposit_follow_canonical_order() {
        let (env, contract_id) = setup();
        let provider = Address::generate(&env);
        let trader = Address::generate(&env);
        env.as_contract(&contract_id, || {
            // Created reversed, so asset_a is wETH and amount_a funds wETH.
            let pool_id = pool_for(&env, "wUSDC", "wETH").unwrap();
            add_liquidity(&env, provider, pool_id, 1_000, 2_000).unwrap();
            let pool = get_pool(&env, pool_id).unwrap();
            assert_eq!((pool.reserve_a, pool.reserve_b), (1_000, 2_000));

            // Selling wETH grows reserve_a regardless of creation order.
            swap(&env, trader, pool_id, s(&env, "wETH"), 100, 1).unwrap();
            let pool = get_pool(&env, pool_id).unwrap();
            assert!(pool.reserve_a > 1_000);
            assert!(pool.reserve_b < 2_000);
        });
    }
}
