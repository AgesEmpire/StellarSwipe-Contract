#![allow(dead_code)]
use soroban_sdk::{contracttype, Address, Env, Map, Vec};

use crate::errors::AutoTradeError;
use crate::risk;

// ── Constants ────────────────────────────────────────────────────────────────

/// Correlation threshold above which two assets are considered highly correlated (0.7 = 7000 bps).
pub const HIGH_CORR_THRESHOLD: i128 = 7000;
/// Correlation below which an asset is a good diversifier (0.3 = 3000 bps).
pub const LOW_CORR_THRESHOLD: i128 = 3000;
/// Scale factor: correlations are stored as basis points in [-10000, 10000].
pub const CORR_SCALE: i128 = 10_000;
/// Cache TTL: recalculate matrix after 24 h.
pub const MATRIX_TTL_SECS: u64 = 86_400;

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
pub enum CorrKey {
    Limits(Address),
    Matrix(Address),
}

// ── Public types ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrelationRisk {
    /// Percentage of portfolio value that is highly correlated with the new asset.
    pub correlated_exposure_pct: i128,
    /// Number of existing holdings with |corr| > HIGH_CORR_THRESHOLD vs the new asset.
    pub highly_correlated_assets: u32,
    pub risk_level: RiskLevel,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrelationLimits {
    /// Max % of portfolio allowed in highly-correlated assets (e.g. 70).
    pub max_correlated_exposure_pct: u32,
    /// Max |correlation| coefficient in bps (e.g. 7000 = 0.7).
    pub max_single_correlation: u32,
    /// Max number of highly-correlated positions.
    pub max_correlated_positions: u32,
}

impl Default for CorrelationLimits {
    fn default() -> Self {
        CorrelationLimits {
            max_correlated_exposure_pct: 70,
            max_single_correlation: 7000,
            max_correlated_positions: 3,
        }
    }
}

/// Cached correlation matrix for a user's portfolio.
#[contracttype]
#[derive(Clone, Debug)]
pub struct CorrelationMatrix {
    /// Flat map: key = (asset_a * 1_000_000 + asset_b), value = correlation bps.
    pub correlations: Map<u64, i128>,
    pub last_updated: u64,
}

// ── Integer square-root (Newton's method) ────────────────────────────────────

fn isqrt(n: i128) -> i128 {
    if n <= 0 {
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

// ── Correlation key encoding ──────────────────────────────────────────────────

#[inline]
fn pair_key(a: u32, b: u32) -> u64 {
    (a as u64) * 1_000_000 + (b as u64)
}

// ── Core calculation ──────────────────────────────────────────────────────────

/// Pearson correlation between two assets over `window` price samples.
/// Returns value in basis points [-10000, 10000].
/// Falls back to 0 (unknown / low correlation) when history is insufficient.
pub fn calculate_correlation(env: &Env, asset_a: u32, asset_b: u32, window: u32) -> i128 {
    let prices_a = get_price_history(env, asset_a, window);
    let prices_b = get_price_history(env, asset_b, window);

    // Need at least 2 prices to compute 1 return, and at least 2 returns for correlation.
    let min_len = prices_a.len().min(prices_b.len());
    if min_len < 3 {
        return 0; // insufficient history → treat as uncorrelated
    }

    let returns_a = compute_returns(env, &prices_a);
    let returns_b = compute_returns(env, &prices_b);

    let n = returns_a.len().min(returns_b.len()) as i128;
    if n < 2 {
        return 0;
    }

    let mut sum_a = 0i128;
    let mut sum_b = 0i128;
    for i in 0..(n as u32) {
        sum_a += returns_a.get(i).unwrap_or(0);
        sum_b += returns_b.get(i).unwrap_or(0);
    }
    let mean_a = sum_a / n;
    let mean_b = sum_b / n;

    let mut numerator = 0i128;
    let mut sum_sq_a = 0i128;
    let mut sum_sq_b = 0i128;

    for i in 0..(n as u32) {
        let da = returns_a.get(i).unwrap_or(0) - mean_a;
        let db = returns_b.get(i).unwrap_or(0) - mean_b;
        numerator += da * db;
        sum_sq_a += da * da;
        sum_sq_b += db * db;
    }

    let denominator = isqrt(sum_sq_a * sum_sq_b);
    if denominator == 0 {
        return 0;
    }

    (numerator * CORR_SCALE) / denominator
}

// ── Matrix ────────────────────────────────────────────────────────────────────

/// Build (or refresh) the correlation matrix for a set of asset IDs.
pub fn build_correlation_matrix(env: &Env, assets: &Vec<u32>) -> CorrelationMatrix {
    let mut correlations: Map<u64, i128> = Map::new(env);
    let len = assets.len();

    for i in 0..len {
        let a = assets.get(i).unwrap();
        for j in (i + 1)..len {
            let b = assets.get(j).unwrap();
            let corr = calculate_correlation(env, a, b, 30);
            correlations.set(pair_key(a, b), corr);
            correlations.set(pair_key(b, a), corr);
        }
    }

    CorrelationMatrix {
        correlations,
        last_updated: env.ledger().timestamp(),
    }
}

/// Retrieve cached matrix or rebuild if stale / missing.
pub fn get_or_build_matrix(env: &Env, user: &Address, assets: &Vec<u32>) -> CorrelationMatrix {
    if let Some(cached) = env
        .storage()
        .persistent()
        .get::<CorrKey, CorrelationMatrix>(&CorrKey::Matrix(user.clone()))
    {
        if env.ledger().timestamp().saturating_sub(cached.last_updated) < MATRIX_TTL_SECS {
            return cached;
        }
    }
    let matrix = build_correlation_matrix(env, assets);
    env.storage()
        .persistent()
        .set(&CorrKey::Matrix(user.clone()), &matrix);
    matrix
}

// ── Portfolio correlation check ───────────────────────────────────────────────

/// Assess the correlation risk of adding `new_asset` with `new_amount` to the user's portfolio.
pub fn check_portfolio_correlation(
    env: &Env,
    user: &Address,
    new_asset: u32,
    new_amount: i128,
) -> Result<CorrelationRisk, AutoTradeError> {
    let positions = risk::get_user_positions(env, user);
    let total_portfolio_value = risk::calculate_portfolio_value(env, user);

    // Collect asset IDs from current positions + new asset for matrix.
    let mut asset_ids: Vec<u32> = Vec::new(env);
    let keys = positions.keys();
    for i in 0..keys.len() {
        if let Some(id) = keys.get(i) {
            asset_ids.push_back(id);
        }
    }
    // Ensure new asset is in the list for matrix building.
    let mut new_asset_in_list = false;
    for i in 0..asset_ids.len() {
        if asset_ids.get(i) == Some(new_asset) {
            new_asset_in_list = true;
            break;
        }
    }
    if !new_asset_in_list {
        asset_ids.push_back(new_asset);
    }

    let matrix = get_or_build_matrix(env, user, &asset_ids);

    let mut high_corr_exposure = 0i128;
    let mut high_corr_count = 0u32;

    for i in 0..keys.len() {
        if let Some(holding_id) = keys.get(i) {
            if holding_id == new_asset {
                continue;
            }
            let corr = matrix
                .correlations
                .get(pair_key(holding_id, new_asset))
                .unwrap_or(0);

            if corr.abs() > HIGH_CORR_THRESHOLD {
                high_corr_count += 1;
                if let Some(pos) = positions.get(holding_id) {
                    let price = risk::get_asset_price(env, holding_id).unwrap_or(pos.entry_price);
                    high_corr_exposure += pos.amount * price / 100;
                }
            }
        }
    }

    let new_total_correlated = high_corr_exposure + new_amount;
    let base = if total_portfolio_value > 0 {
        total_portfolio_value
    } else {
        new_amount.max(1)
    };
    let corr_pct = (new_total_correlated * 100) / base;

    let risk_level = if corr_pct > 70 {
        RiskLevel::High
    } else if corr_pct > 50 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    Ok(CorrelationRisk {
        correlated_exposure_pct: corr_pct,
        highly_correlated_assets: high_corr_count,
        risk_level,
    })
}

// ── Limit enforcement ─────────────────────────────────────────────────────────

pub fn get_correlation_limits(env: &Env, user: &Address) -> CorrelationLimits {
    env.storage()
        .persistent()
        .get(&CorrKey::Limits(user.clone()))
        .unwrap_or_default()
}

pub fn set_correlation_limits(env: &Env, user: &Address, limits: &CorrelationLimits) {
    env.storage()
        .persistent()
        .set(&CorrKey::Limits(user.clone()), limits);
}

/// Returns `Err(CorrelationLimitExceeded)` if the new trade would breach the user's limits.
pub fn enforce_correlation_limits(
    env: &Env,
    user: &Address,
    new_asset: u32,
    new_amount: i128,
) -> Result<(), AutoTradeError> {
    let limits = get_correlation_limits(env, user);
    let risk = check_portfolio_correlation(env, user, new_asset, new_amount)?;

    if risk.correlated_exposure_pct > limits.max_correlated_exposure_pct as i128 {
        return Err(AutoTradeError::CorrelationLimitExceeded);
    }
    if risk.highly_correlated_assets > limits.max_correlated_positions {
        return Err(AutoTradeError::TooManyCorrelatedPositions);
    }

    Ok(())
}

// ── Diversification suggestions ───────────────────────────────────────────────

/// Return up to 5 asset IDs from `available` that have low average correlation
/// with the user's current holdings.
pub fn suggest_diversification(
    env: &Env,
    user: &Address,
    available: &Vec<u32>,
) -> Vec<u32> {
    let positions = risk::get_user_positions(env, user);
    let holding_keys = positions.keys();

    // Build a combined asset list for the matrix.
    let mut all_assets: Vec<u32> = Vec::new(env);
    for i in 0..holding_keys.len() {
        if let Some(id) = holding_keys.get(i) {
            all_assets.push_back(id);
        }
    }
    for i in 0..available.len() {
        if let Some(id) = available.get(i) {
            all_assets.push_back(id);
        }
    }

    let matrix = get_or_build_matrix(env, user, &all_assets);

    // Collect (avg_corr, asset_id) pairs for candidates.
    let mut suggestions: Vec<u32> = Vec::new(env);

    'outer: for i in 0..available.len() {
        let candidate = available.get(i).unwrap();

        // Skip if already held.
        for j in 0..holding_keys.len() {
            if holding_keys.get(j) == Some(candidate) {
                continue 'outer;
            }
        }

        let mut avg_corr = 0i128;
        let mut count = 0i128;

        for j in 0..holding_keys.len() {
            if let Some(holding_id) = holding_keys.get(j) {
                let corr = matrix
                    .correlations
                    .get(pair_key(holding_id, candidate))
                    .unwrap_or(0);
                avg_corr += corr.abs();
                count += 1;
            }
        }

        if count == 0 || avg_corr / count < LOW_CORR_THRESHOLD {
            suggestions.push_back(candidate);
            if suggestions.len() >= 5 {
                break;
            }
        }
    }

    suggestions
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn get_price_history(env: &Env, asset_id: u32, window: u32) -> Vec<i128> {
    use crate::risk::RiskDataKey;
    let mut prices = Vec::new(env);
    let count: u32 = env
        .storage()
        .persistent()
        .get(&RiskDataKey::AssetPriceHistoryCount(asset_id))
        .unwrap_or(0);
    if count == 0 {
        return prices;
    }
    let window = window.min(count).min(30);
    for i in 0..window {
        let idx = (count + 30 - 1 - i) % 30;
        if let Some(price) = env
            .storage()
            .persistent()
            .get(&RiskDataKey::AssetPriceHistory(asset_id, idx))
        {
            prices.push_front(price);
        }
    }
    prices
}

fn compute_returns(env: &Env, prices: &Vec<i128>) -> Vec<i128> {
    let mut returns = Vec::new(env);
    for i in 1..prices.len() {
        let prev = prices.get(i - 1).unwrap_or(0);
        let curr = prices.get(i).unwrap_or(0);
        if prev > 0 {
            returns.push_back((curr - prev) * CORR_SCALE / prev);
        }
    }
    returns
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract,
        testutils::{Address as _, Ledger as _},
        Env,
    };

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let contract_addr = env.register(TestContract, ());
        (env, contract_addr)
    }

    fn seed_prices(env: &Env, asset_id: u32, prices: &[i128]) {
        use crate::risk::RiskDataKey;
        for (i, &p) in prices.iter().enumerate() {
            env.storage().persistent().set(
                &RiskDataKey::AssetPriceHistory(asset_id, i as u32),
                &p,
            );
        }
        env.storage().persistent().set(
            &RiskDataKey::AssetPriceHistoryCount(asset_id),
            &(prices.len() as u32),
        );
    }

    // ── calculate_correlation ─────────────────────────────────────────────────

    #[test]
    fn test_high_positive_correlation() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            // Identical price series → perfect correlation (10000 bps).
            let prices = [100i128, 102, 105, 103, 108, 110, 107, 112];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);
            let corr = calculate_correlation(&env, 1, 2, 30);
            assert_eq!(corr, CORR_SCALE, "identical series must give +10000 bps");
        });
    }

    #[test]
    fn test_negative_correlation() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            // Perfectly inverse series.
            let a = [100i128, 102, 104, 106, 108];
            let b = [108i128, 106, 104, 102, 100];
            seed_prices(&env, 1, &a);
            seed_prices(&env, 2, &b);
            let corr = calculate_correlation(&env, 1, 2, 30);
            assert!(corr < 0, "inverse series must give negative correlation");
        });
    }

    #[test]
    fn test_insufficient_history_returns_zero() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            seed_prices(&env, 1, &[100i128, 101]);
            seed_prices(&env, 2, &[100i128, 101]);
            let corr = calculate_correlation(&env, 1, 2, 30);
            assert_eq!(corr, 0);
        });
    }

    // ── build_correlation_matrix ──────────────────────────────────────────────

    #[test]
    fn test_matrix_symmetry() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let prices = [100i128, 102, 101, 105, 103, 107, 106, 110];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);
            seed_prices(&env, 3, &[50i128, 51, 49, 52, 50, 53, 51, 54]);

            let mut assets = Vec::new(&env);
            assets.push_back(1u32);
            assets.push_back(2u32);
            assets.push_back(3u32);

            let matrix = build_correlation_matrix(&env, &assets);

            // Symmetry: corr(1,2) == corr(2,1)
            let c12 = matrix.correlations.get(pair_key(1, 2)).unwrap_or(0);
            let c21 = matrix.correlations.get(pair_key(2, 1)).unwrap_or(0);
            assert_eq!(c12, c21);
        });
    }

    // ── check_portfolio_correlation ───────────────────────────────────────────

    #[test]
    fn test_low_risk_when_no_holdings() {
        let (env, addr) = setup();
        let user = Address::generate(&env);
        env.as_contract(&addr, || {
            let risk = check_portfolio_correlation(&env, &user, 1, 1_000).unwrap();
            assert_eq!(risk.risk_level, RiskLevel::Low);
            assert_eq!(risk.highly_correlated_assets, 0);
        });
    }

    #[test]
    fn test_high_risk_with_correlated_holdings() {
        let (env, addr) = setup();
        let user = Address::generate(&env);
        env.as_contract(&addr, || {
            // Seed identical price history for assets 1 and 2 → corr = 10000 bps.
            let prices = [100i128, 102, 105, 103, 108, 110, 107, 112];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);

            // User holds asset 1 with large value.
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);

            // Adding asset 2 (perfectly correlated) should flag high risk.
            let result = check_portfolio_correlation(&env, &user, 2, 5_000).unwrap();
            assert_eq!(result.highly_correlated_assets, 1);
            assert!(result.correlated_exposure_pct > 0);
        });
    }

    // ── enforce_correlation_limits ────────────────────────────────────────────

    #[test]
    fn test_enforce_blocks_over_limit() {
        let (env, addr) = setup();
        let user = Address::generate(&env);
        env.as_contract(&addr, || {
            let prices = [100i128, 102, 105, 103, 108, 110, 107, 112];
            seed_prices(&env, 1, &prices);
            seed_prices(&env, 2, &prices);

            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 10_000, 100);

            // Tight limits: 0 correlated positions allowed.
            set_correlation_limits(
                &env,
                &user,
                &CorrelationLimits {
                    max_correlated_exposure_pct: 70,
                    max_single_correlation: 7000,
                    max_correlated_positions: 0,
                },
            );

            let result = enforce_correlation_limits(&env, &user, 2, 5_000);
            assert_eq!(result, Err(AutoTradeError::TooManyCorrelatedPositions));
        });
    }

    #[test]
    fn test_enforce_allows_uncorrelated_trade() {
        let (env, addr) = setup();
        let user = Address::generate(&env);
        env.as_contract(&addr, || {
            // No price history for asset 3 → correlation defaults to 0.
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 1_000, 100);

            let result = enforce_correlation_limits(&env, &user, 3, 500);
            assert!(result.is_ok());
        });
    }

    // ── suggest_diversification ───────────────────────────────────────────────

    #[test]
    fn test_suggest_returns_low_corr_assets() {
        let (env, addr) = setup();
        let user = Address::generate(&env);
        env.as_contract(&addr, || {
            // Asset 1 held; asset 2 has no history (corr=0 → good diversifier).
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 1_000, 100);

            let mut available = Vec::new(&env);
            available.push_back(2u32);
            available.push_back(3u32);

            let suggestions = suggest_diversification(&env, &user, &available);
            assert!(suggestions.len() > 0);
        });
    }

    #[test]
    fn test_suggest_excludes_already_held() {
        let (env, addr) = setup();
        let user = Address::generate(&env);
        env.as_contract(&addr, || {
            risk::set_asset_price(&env, 1, 100);
            risk::update_position(&env, &user, 1, 1_000, 100);

            // Available list only contains the already-held asset.
            let mut available = Vec::new(&env);
            available.push_back(1u32);

            let suggestions = suggest_diversification(&env, &user, &available);
            assert_eq!(suggestions.len(), 0);
        });
    }
}
