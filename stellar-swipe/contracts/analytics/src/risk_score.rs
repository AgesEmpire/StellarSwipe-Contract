#![allow(dead_code)]

use crate::DataKey;
use soroban_sdk::{contracttype, Address, Env, Vec};

pub const DEFAULT_TRAILING_WINDOW_SIZE: u32 = 50;
pub const DEFAULT_MIN_SAMPLE_SIZE: u32 = 10;

/// A fixed-point scaling factor. We use 10,000 for basis points (BPS)
/// or 10,000,000 for 7-decimal Stellar-standard fixed point.
/// Let's use 10,000,000 to maintain high precision for variance/mean calculations,
/// which matches Stellar's 7 decimal places standard, or 10,000 for BPS.
/// The prompt mentions "basis points" in other places, so we'll use BPS (10_000)
/// but allow multiplication/division with appropriate scaling.
pub const FIXED_POINT_SCALE: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricReturn {
    /// Return in basis points (e.g., 500 = 5% return)
    pub return_bps: i128,
    /// Ledger timestamp when the signal was resolved/closed
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoreResult {
    InsufficientData,
    Score(i128),
}

/// Computes fixed-point multiplication: (a * b) / SCALE
pub fn fp_mul(a: i128, b: i128) -> i128 {
    // Multiply then divide by SCALE. We use checked operations to avoid panic,
    // though i128 is large enough that overflow is unlikely for typical BPS values.
    a.checked_mul(b)
        .expect("fp_mul overflow")
        .checked_div(FIXED_POINT_SCALE)
        .expect("fp_mul div by zero")
}

/// Computes fixed-point division: (a * SCALE) / b
pub fn fp_div(a: i128, b: i128) -> i128 {
    if b == 0 {
        panic!("fp_div division by zero");
    }
    a.checked_mul(FIXED_POINT_SCALE)
        .expect("fp_div overflow")
        .checked_div(b)
        .expect("fp_div div by zero")
}

/// Integer square root for i128 using Newton's method.
pub fn isqrt(n: i128) -> i128 {
    if n < 0 {
        panic!("isqrt of negative number");
    }
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

pub fn get_trailing_window_size(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::TrailingWindowSize)
        .unwrap_or(DEFAULT_TRAILING_WINDOW_SIZE)
}

pub fn set_trailing_window_size(env: &Env, size: u32) {
    env.storage()
        .instance()
        .set(&DataKey::TrailingWindowSize, &size);
}

pub fn get_min_sample_size(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::MinSampleSize)
        .unwrap_or(DEFAULT_MIN_SAMPLE_SIZE)
}

pub fn set_min_sample_size(env: &Env, size: u32) {
    env.storage().instance().set(&DataKey::MinSampleSize, &size);
}

pub fn get_provider_returns(env: &Env, provider: &Address) -> Vec<HistoricReturn> {
    env.storage()
        .instance()
        .get(&DataKey::ProviderReturns(provider.clone()))
        .unwrap_or(Vec::new(env))
}

pub fn record_provider_return(env: &Env, provider: &Address, ret: HistoricReturn) {
    let mut returns = get_provider_returns(env, provider);
    returns.push_back(ret);

    let window_size = get_trailing_window_size(env);

    // Remove oldest entries if exceeding window size
    while returns.len() > window_size {
        returns.pop_front();
    }

    env.storage()
        .instance()
        .set(&DataKey::ProviderReturns(provider.clone()), &returns);
}

pub fn compute_risk_adjusted_score(env: &Env, provider: &Address) -> ScoreResult {
    let returns = get_provider_returns(env, provider);
    let min_sample = get_min_sample_size(env);

    if returns.len() < min_sample {
        return ScoreResult::InsufficientData;
    }

    let len_i128 = returns.len() as i128;

    // Compute mean
    let mut sum: i128 = 0;
    for ret in returns.iter() {
        sum = sum.checked_add(ret.return_bps).expect("sum overflow");
    }
    let mean = sum / len_i128; // Standard integer division is fine here as both are in same scale

    // Compute sample variance (sum_diff_sq / (n - 1))
    let mut sum_diff_sq: i128 = 0;
    for ret in returns.iter() {
        let diff = ret.return_bps.checked_sub(mean).expect("diff overflow");
        let diff_sq = fp_mul(diff, diff);
        sum_diff_sq = sum_diff_sq
            .checked_add(diff_sq)
            .expect("sum_diff_sq overflow");
    }

    let variance = sum_diff_sq / (len_i128 - 1);

    // Compute standard deviation: isqrt(variance * SCALE)
    let std_dev = isqrt(
        variance
            .checked_mul(FIXED_POINT_SCALE)
            .expect("var scale overflow"),
    );

    // Sharpe-style score = mean / (std_dev + epsilon)
    // We add an epsilon to prevent division by zero or extremely high scores for zero variance.
    // Epsilon of 1 BPS = 1
    let epsilon = 1;
    let denominator = std_dev.checked_add(epsilon).expect("denom overflow");

    let score = fp_div(mean, denominator);

    ScoreResult::Score(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_historic_return_struct() {
        let hr = HistoricReturn {
            return_bps: 1500, // 15%
            timestamp: 1234567890,
        };
        assert_eq!(hr.return_bps, 1500);
        assert_eq!(hr.timestamp, 1234567890);
    }

    #[test]
    fn test_fp_mul() {
        // 5% * 5% = 0.25% -> (500 * 500) / 10000 = 25
        assert_eq!(fp_mul(500, 500), 25);
        // 100% * 15% = 15% -> (10000 * 1500) / 10000 = 1500
        assert_eq!(fp_mul(10000, 1500), 1500);
        // Negative returns
        assert_eq!(fp_mul(-500, 500), -25);
        assert_eq!(fp_mul(-500, -500), 25);
    }

    #[test]
    fn test_fp_div() {
        // 15% / 5% = 3.0 (which is 30000 in basis points)
        // (1500 * 10000) / 500 = 30000
        assert_eq!(fp_div(1500, 500), 30000);
        // 100% / 100% = 1.0 -> 10000
        assert_eq!(fp_div(10000, 10000), 10000);
        // Negative division
        assert_eq!(fp_div(-1500, 500), -30000);
        assert_eq!(fp_div(-1500, -500), 30000);
    }

    #[test]
    #[should_panic(expected = "fp_div division by zero")]
    fn test_fp_div_by_zero() {
        fp_div(1500, 0);
    }

    #[test]
    #[should_panic(expected = "fp_mul overflow")]
    fn test_fp_mul_overflow() {
        // i128::MAX * 2 should overflow
        fp_mul(i128::MAX, 2);
    }

    #[test]
    #[should_panic(expected = "fp_div overflow")]
    fn test_fp_div_overflow() {
        // i128::MAX * FIXED_POINT_SCALE should overflow
        fp_div(i128::MAX, 2);
    }

    #[test]
    fn test_trailing_window_logic() {
        let env = Env::default();
        let id = env.register(crate::AnalyticsContract, ());
        let provider = Address::generate(&env);

        env.as_contract(&id, || {
            // Window size = 3
            set_trailing_window_size(&env, 3);
            assert_eq!(get_trailing_window_size(&env), 3);

            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 100,
                    timestamp: 1,
                },
            );
            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 200,
                    timestamp: 2,
                },
            );
            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 300,
                    timestamp: 3,
                },
            );

            let returns = get_provider_returns(&env, &provider);
            assert_eq!(returns.len(), 3);
            assert_eq!(returns.get(0).unwrap().return_bps, 100);

            // Add 4th, should drop 1st
            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 400,
                    timestamp: 4,
                },
            );

            let returns2 = get_provider_returns(&env, &provider);
            assert_eq!(returns2.len(), 3);
            assert_eq!(returns2.get(0).unwrap().return_bps, 200);
            assert_eq!(returns2.get(2).unwrap().return_bps, 400);
        });
    }

    #[test]
    fn test_isqrt() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(10), 3);
        assert_eq!(isqrt(250000), 500);
    }

    #[test]
    fn test_compute_risk_adjusted_score() {
        let env = Env::default();
        let id = env.register(crate::AnalyticsContract, ());
        let provider = Address::generate(&env);

        env.as_contract(&id, || {
            set_min_sample_size(&env, 3);

            // 1. Insufficient data
            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 500,
                    timestamp: 1,
                },
            );
            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 500,
                    timestamp: 2,
                },
            );
            assert_eq!(
                compute_risk_adjusted_score(&env, &provider),
                ScoreResult::InsufficientData
            );

            // 2. Low variance, positive mean -> high score
            record_provider_return(
                &env,
                &provider,
                HistoricReturn {
                    return_bps: 500,
                    timestamp: 3,
                },
            ); // mean = 500, var = 0, std_dev = 0
            let score_low_var = compute_risk_adjusted_score(&env, &provider);
            if let ScoreResult::Score(val) = score_low_var {
                // mean = 500. denominator = std_dev (0) + 1 = 1.
                // fp_div(500, 1) = (500 * 10000) / 1 = 5,000,000
                assert_eq!(val, 5000000);
            } else {
                panic!("Expected score");
            }

            // 3. High variance, same mean -> lower score
            let provider_high = Address::generate(&env);
            // returns: 1500, -500, 500. mean = (1500 - 500 + 500)/3 = 500
            record_provider_return(
                &env,
                &provider_high,
                HistoricReturn {
                    return_bps: 1500,
                    timestamp: 1,
                },
            );
            record_provider_return(
                &env,
                &provider_high,
                HistoricReturn {
                    return_bps: -500,
                    timestamp: 2,
                },
            );
            record_provider_return(
                &env,
                &provider_high,
                HistoricReturn {
                    return_bps: 500,
                    timestamp: 3,
                },
            );

            // Diff from mean (500): 1000, -1000, 0
            // Diff squared: fp_mul(1000,1000)=100, fp_mul(-1000,-1000)=100, 0
            // sum_diff_sq = 200.
            // variance = 200 / (3-1) = 100.
            // std_dev = isqrt(100 * 10000) = isqrt(1000000) = 1000.
            // denom = 1000 + 1 = 1001.
            // score = fp_div(500, 1001) = (500 * 10000) / 1001 = 4995.
            let score_high_var = compute_risk_adjusted_score(&env, &provider_high);
            if let ScoreResult::Score(val) = score_high_var {
                assert_eq!(val, 4995);
            } else {
                panic!("Expected score");
            }
        });
    }
}
