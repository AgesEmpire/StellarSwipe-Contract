//! Oracle quorum validation (issue #987).
//!
//! A price is only exposed to trading decisions once a quorum of independent,
//! authorized observations agrees. Duplicate providers, unauthorized providers
//! and non-positive values are discarded before the quorum is counted, so they
//! can never inflate it. When validation fails the caller keeps the last valid
//! state and receives a documented error.

use crate::errors::OracleError;
use soroban_sdk::{contracttype, Address, Env, Vec};

/// Minimum number of distinct authorized observations required.
pub const DEFAULT_MIN_QUORUM: u32 = 3;

/// Maximum tolerated deviation of a single observation from the median,
/// in basis points (1000 bps = 10%).
pub const DEFAULT_MAX_DEVIATION_BPS: i128 = 1000;

/// A single price observation submitted by a provider.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    /// Provider that submitted the price.
    pub provider: Address,
    /// Submitted price; must be strictly positive.
    pub price: i128,
}

/// Quorum and deviation rules enforced before a price is accepted.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumConfig {
    /// Minimum count of distinct valid observations.
    pub min_quorum: u32,
    /// Maximum allowed deviation from the median, in basis points.
    pub max_deviation_bps: i128,
}

impl Default for QuorumConfig {
    fn default() -> Self {
        Self {
            min_quorum: DEFAULT_MIN_QUORUM,
            max_deviation_bps: DEFAULT_MAX_DEVIATION_BPS,
        }
    }
}

/// Validates observations and returns the agreed median price.
///
/// # Errors
/// * [`OracleError::InvalidPrice`] — an observation carries a zero or negative price.
/// * [`OracleError::InsufficientOracles`] — fewer distinct authorized providers than the quorum.
/// * [`OracleError::UnreliablePrice`] — an accepted observation deviates from the median
///   by more than `max_deviation_bps`.
pub fn validate_quorum(
    env: &Env,
    observations: &Vec<Observation>,
    authorized: &Vec<Address>,
    config: &QuorumConfig,
) -> Result<i128, OracleError> {
    let mut seen: Vec<Address> = Vec::new(env);
    let mut prices: Vec<i128> = Vec::new(env);

    for obs in observations.iter() {
        // Zero or negative values are never acceptable.
        if obs.price <= 0 {
            return Err(OracleError::InvalidPrice);
        }
        // Unauthorized providers cannot contribute to quorum.
        if !authorized.contains(&obs.provider) {
            continue;
        }
        // Duplicate providers count once.
        if seen.contains(&obs.provider) {
            continue;
        }
        seen.push_back(obs.provider.clone());
        prices.push_back(obs.price);
    }

    if (prices.len() as u32) < config.min_quorum {
        return Err(OracleError::InsufficientOracles);
    }

    let median = median_of(&prices);

    for price in prices.iter() {
        let diff = (price - median).abs();
        // diff / median > max_bps / 10_000, rearranged to avoid division.
        if diff.saturating_mul(10_000) > median.saturating_mul(config.max_deviation_bps) {
            return Err(OracleError::UnreliablePrice);
        }
    }

    Ok(median)
}

/// Median of a non-empty price set (insertion sort — sets are quorum-sized).
fn median_of(prices: &Vec<i128>) -> i128 {
    let mut sorted = prices.clone();
    let len = sorted.len();
    for i in 1..len {
        let mut j = i;
        while j > 0 && sorted.get_unchecked(j - 1) > sorted.get_unchecked(j) {
            let a = sorted.get_unchecked(j - 1);
            let b = sorted.get_unchecked(j);
            sorted.set(j - 1, b);
            sorted.set(j, a);
            j -= 1;
        }
    }
    let mid = len / 2;
    if len % 2 == 1 {
        sorted.get_unchecked(mid)
    } else {
        (sorted.get_unchecked(mid - 1) + sorted.get_unchecked(mid)) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env};

    fn providers(env: &Env, n: u32) -> Vec<Address> {
        let mut v = Vec::new(env);
        for _ in 0..n {
            v.push_back(Address::generate(env));
        }
        v
    }

    fn obs(providers: &Vec<Address>, idx: u32, price: i128) -> Observation {
        Observation {
            provider: providers.get_unchecked(idx),
            price,
        }
    }

    #[test]
    fn quorum_met_returns_median() {
        let env = Env::default();
        let auth = providers(&env, 3);
        let observations = vec![
            &env,
            obs(&auth, 0, 100),
            obs(&auth, 1, 102),
            obs(&auth, 2, 101),
        ];
        let price = validate_quorum(&env, &observations, &auth, &QuorumConfig::default()).unwrap();
        assert_eq!(price, 101);
    }

    #[test]
    fn duplicate_provider_cannot_inflate_quorum() {
        let env = Env::default();
        let auth = providers(&env, 3);
        let observations = vec![
            &env,
            obs(&auth, 0, 100),
            obs(&auth, 0, 100),
            obs(&auth, 1, 100),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Err(OracleError::InsufficientOracles)
        );
    }

    #[test]
    fn unauthorized_provider_cannot_inflate_quorum() {
        let env = Env::default();
        let auth = providers(&env, 2);
        let rogue = Address::generate(&env);
        let observations = vec![
            &env,
            obs(&auth, 0, 100),
            obs(&auth, 1, 100),
            Observation {
                provider: rogue,
                price: 100,
            },
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Err(OracleError::InsufficientOracles)
        );
    }

    #[test]
    fn non_positive_price_rejected() {
        let env = Env::default();
        let auth = providers(&env, 3);
        for bad in [0i128, -1i128] {
            let observations = vec![
                &env,
                obs(&auth, 0, 100),
                obs(&auth, 1, bad),
                obs(&auth, 2, 100),
            ];
            assert_eq!(
                validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
                Err(OracleError::InvalidPrice)
            );
        }
    }

    #[test]
    fn feed_disagreement_rejected() {
        let env = Env::default();
        let auth = providers(&env, 3);
        let observations = vec![
            &env,
            obs(&auth, 0, 100),
            obs(&auth, 1, 101),
            obs(&auth, 2, 500),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Err(OracleError::UnreliablePrice)
        );
    }

    #[test]
    fn deviation_boundary_is_inclusive() {
        let env = Env::default();
        let auth = providers(&env, 3);
        // Median 100, outlier exactly 10% away.
        let observations = vec![
            &env,
            obs(&auth, 0, 100),
            obs(&auth, 1, 100),
            obs(&auth, 2, 110),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()).unwrap(),
            100
        );
    }

    #[test]
    fn missing_feeds_return_insufficient_oracles() {
        let env = Env::default();
        let auth = providers(&env, 3);
        let observations = vec![&env, obs(&auth, 0, 100)];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Err(OracleError::InsufficientOracles)
        );
    }
}
