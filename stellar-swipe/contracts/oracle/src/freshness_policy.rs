//! Shared oracle freshness / timestamp policy (issue #986).
//!
//! Every price read path must funnel through [`validate_observation`] so that
//! stale, future-dated and non-monotonic observations are rejected the same way
//! no matter which caller performs the read. Keeping the limits in one module
//! means there is no alternate read path with weaker validation.

use crate::errors::OracleError;

/// Maximum age (seconds) of an observation before it is considered stale.
pub const MAX_PRICE_AGE_SECS: u64 = 300;

/// Tolerated clock skew (seconds) for observations timestamped ahead of the
/// current ledger. Anything beyond this is treated as future-dated.
pub const MAX_CLOCK_SKEW_SECS: u64 = 30;

/// Freshness limits applied to a price observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FreshnessPolicy {
    /// Maximum accepted age in seconds.
    pub max_age_secs: u64,
    /// Maximum accepted forward clock skew in seconds.
    pub max_skew_secs: u64,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        Self {
            max_age_secs: MAX_PRICE_AGE_SECS,
            max_skew_secs: MAX_CLOCK_SKEW_SECS,
        }
    }
}

impl FreshnessPolicy {
    /// Builds a policy with explicit limits.
    pub fn new(max_age_secs: u64, max_skew_secs: u64) -> Self {
        Self {
            max_age_secs,
            max_skew_secs,
        }
    }
}

/// Validates a single observation against the policy.
///
/// * `now` — current ledger timestamp.
/// * `observed_at` — timestamp carried by the observation.
/// * `last_accepted_at` — timestamp of the last accepted observation for this
///   feed, or `None` for the first ever write.
///
/// Returns [`OracleError::StalePrice`] for observations that are too old,
/// too far in the future, or that regress below the last accepted timestamp.
pub fn validate_observation(
    policy: &FreshnessPolicy,
    now: u64,
    observed_at: u64,
    last_accepted_at: Option<u64>,
) -> Result<(), OracleError> {
    // Future-dated beyond the tolerated clock skew.
    if observed_at > now.saturating_add(policy.max_skew_secs) {
        return Err(OracleError::StalePrice);
    }

    // Older than the freshness window.
    if now.saturating_sub(observed_at) > policy.max_age_secs {
        return Err(OracleError::StalePrice);
    }

    // Monotonic: a new observation may never move the feed clock backwards.
    if let Some(prev) = last_accepted_at {
        if observed_at < prev {
            return Err(OracleError::StalePrice);
        }
    }

    Ok(())
}

/// Convenience wrapper using the default policy.
pub fn validate_with_defaults(
    now: u64,
    observed_at: u64,
    last_accepted_at: Option<u64>,
) -> Result<(), OracleError> {
    validate_observation(&FreshnessPolicy::default(), now, observed_at, last_accepted_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10_000;

    #[test]
    fn fresh_observation_accepted() {
        assert!(validate_with_defaults(NOW, NOW - 10, Some(NOW - 60)).is_ok());
    }

    #[test]
    fn boundary_age_accepted_and_one_second_past_rejected() {
        // Exactly at the window edge is still valid.
        assert!(validate_with_defaults(NOW, NOW - MAX_PRICE_AGE_SECS, None).is_ok());
        // One second older is stale.
        assert_eq!(
            validate_with_defaults(NOW, NOW - MAX_PRICE_AGE_SECS - 1, None),
            Err(OracleError::StalePrice)
        );
    }

    #[test]
    fn boundary_skew_accepted_and_one_second_past_rejected() {
        assert!(validate_with_defaults(NOW, NOW + MAX_CLOCK_SKEW_SECS, None).is_ok());
        assert_eq!(
            validate_with_defaults(NOW, NOW + MAX_CLOCK_SKEW_SECS + 1, None),
            Err(OracleError::StalePrice)
        );
    }

    #[test]
    fn regressing_timestamp_rejected() {
        assert_eq!(
            validate_with_defaults(NOW, NOW - 100, Some(NOW - 50)),
            Err(OracleError::StalePrice)
        );
    }

    #[test]
    fn equal_timestamp_is_monotonic() {
        assert!(validate_with_defaults(NOW, NOW - 50, Some(NOW - 50)).is_ok());
    }

    #[test]
    fn independent_feeds_keep_independent_clocks() {
        // Feed A is fresh, feed B lags but is still inside the window: both pass
        // because monotonicity is tracked per feed, not globally.
        assert!(validate_with_defaults(NOW, NOW - 5, Some(NOW - 20)).is_ok());
        assert!(validate_with_defaults(NOW, NOW - 200, Some(NOW - 250)).is_ok());
    }

    #[test]
    fn custom_policy_limits_are_honoured() {
        let strict = FreshnessPolicy::new(30, 0);
        assert!(validate_observation(&strict, NOW, NOW - 30, None).is_ok());
        assert_eq!(
            validate_observation(&strict, NOW, NOW - 31, None),
            Err(OracleError::StalePrice)
        );
        assert_eq!(
            validate_observation(&strict, NOW, NOW + 1, None),
            Err(OracleError::StalePrice)
        );
    }
}
