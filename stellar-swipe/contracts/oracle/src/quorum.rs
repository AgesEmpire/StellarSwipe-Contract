//! Oracle quorum validation (issue #987).
//!
//! A price is only exposed to trading decisions once a quorum of independent,
//! authorized observations agrees. Duplicate providers, unauthorized providers
//! and non-positive values are discarded before the quorum is counted, so they
//! can never inflate it. When validation fails the caller keeps the last valid
//! state and receives a documented error.
//!
//! Observations are identified by `(provider, sequence)` (Issue #1212): a
//! replayed or older observation from a provider is ignored and a newer one
//! supersedes it, so each provider contributes exactly one vote. Future-dated
//! observations are rejected before any are counted (Issue #1213).

use crate::errors::OracleError;
use crate::observation::{self, Admission};
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
    /// Ledger sequence the observation was made at; with `provider` it
    /// uniquely identifies the observation (Issue #1212).
    pub sequence: u32,
    /// Observation time; must not be later than the ledger timestamp (Issue #1213).
    pub timestamp: u64,
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
/// * [`OracleError::FutureTimestamp`] — an observation is timestamped after the ledger time.
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
    // Timestamps are validated before any observation is used.
    for obs in observations.iter() {
        observation::ensure_not_future(env, obs.timestamp)?;
    }

    // (provider, sequence, price) — at most one entry per provider.
    let mut votes: Vec<(Address, u64, i128)> = Vec::new(env);

    for obs in observations.iter() {
        // Zero or negative values are never acceptable.
        if obs.price <= 0 {
            return Err(OracleError::InvalidPrice);
        }
        // Unauthorized providers cannot contribute to quorum.
        if !authorized.contains(&obs.provider) {
            continue;
        }
        let sequence = obs.sequence as u64;
        match observation::admit(
            votes.iter().map(|(p, s, _)| (p, s)),
            &obs.provider,
            sequence,
        ) {
            Ok(Admission::Append) => votes.push_back((obs.provider, sequence, obs.price)),
            Ok(Admission::Replace(i)) => votes.set(i, (obs.provider, sequence, obs.price)),
            // Replayed or older observation: never adds weight.
            Err(_) => continue,
        }
    }

    let mut prices: Vec<i128> = Vec::new(env);
    for (_, _, price) in votes.iter() {
        prices.push_back(price);
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
        obs_at(providers, idx, price, 1)
    }

    fn obs_at(providers: &Vec<Address>, idx: u32, price: i128, sequence: u32) -> Observation {
        Observation {
            provider: providers.get_unchecked(idx),
            price,
            sequence,
            timestamp: 0,
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
                sequence: 1,
                timestamp: 0,
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

    // ── Observation identity (Issue #1212) ──────────────────────────────────

    #[test]
    fn replayed_observation_cannot_inflate_quorum() {
        let env = Env::default();
        let auth = providers(&env, 3);
        // Provider 0's observation at sequence 7 is replayed twice.
        let observations = vec![
            &env,
            obs_at(&auth, 0, 100, 7),
            obs_at(&auth, 0, 100, 7),
            obs_at(&auth, 0, 100, 7),
            obs_at(&auth, 1, 100, 7),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Err(OracleError::InsufficientOracles)
        );
    }

    #[test]
    fn distinct_sources_at_same_sequence_are_independent() {
        let env = Env::default();
        let auth = providers(&env, 3);
        let observations = vec![
            &env,
            obs_at(&auth, 0, 100, 7),
            obs_at(&auth, 1, 101, 7),
            obs_at(&auth, 2, 102, 7),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Ok(101)
        );
    }

    #[test]
    fn sequence_advancement_supersedes_without_adding_weight() {
        let env = Env::default();
        let auth = providers(&env, 3);
        // Provider 0 advances from sequence 7 to 8: still a single vote, and the
        // newer observation is the one counted.
        let observations = vec![
            &env,
            obs_at(&auth, 0, 100, 7),
            obs_at(&auth, 0, 104, 8),
            obs_at(&auth, 1, 104, 7),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &QuorumConfig::default()),
            Err(OracleError::InsufficientOracles)
        );

        let config = QuorumConfig {
            min_quorum: 2,
            ..QuorumConfig::default()
        };
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &config),
            Ok(104)
        );
    }

    #[test]
    fn older_sequence_after_newer_is_ignored() {
        let env = Env::default();
        let auth = providers(&env, 3);
        let config = QuorumConfig {
            min_quorum: 2,
            ..QuorumConfig::default()
        };
        // Sequence 8 arrives first; a late sequence-7 replay must not win.
        let observations = vec![
            &env,
            obs_at(&auth, 0, 104, 8),
            obs_at(&auth, 0, 100, 7),
            obs_at(&auth, 1, 104, 8),
        ];
        assert_eq!(
            validate_quorum(&env, &observations, &auth, &config),
            Ok(104)
        );
    }

    // ── Future timestamps (Issue #1213) ─────────────────────────────────────

    fn stamped(providers: &Vec<Address>, ts: u64) -> Vec<Observation> {
        let env = providers.env();
        let mut v = Vec::new(env);
        for i in 0..3 {
            let mut o = obs_at(providers, i, 100, 1);
            o.timestamp = ts;
            v.push_back(o);
        }
        v
    }

    #[test]
    fn current_and_past_timestamps_are_accepted() {
        use soroban_sdk::testutils::Ledger as _;
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let auth = providers(&env, 3);
        for ts in [0u64, 999, 1_000] {
            assert_eq!(
                validate_quorum(&env, &stamped(&auth, ts), &auth, &QuorumConfig::default()),
                Ok(100)
            );
        }
    }

    #[test]
    fn future_timestamp_is_rejected_at_one_second_past_ledger() {
        use soroban_sdk::testutils::Ledger as _;
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let auth = providers(&env, 3);
        for ts in [1_001u64, u64::MAX] {
            assert_eq!(
                validate_quorum(&env, &stamped(&auth, ts), &auth, &QuorumConfig::default()),
                Err(OracleError::FutureTimestamp)
            );
        }
        // One future observation among otherwise valid ones still rejects.
        let mut mixed = stamped(&auth, 1_000);
        let mut bad = mixed.get_unchecked(2);
        bad.timestamp = 1_001;
        mixed.set(2, bad);
        assert_eq!(
            validate_quorum(&env, &mixed, &auth, &QuorumConfig::default()),
            Err(OracleError::FutureTimestamp)
        );
    }
}
