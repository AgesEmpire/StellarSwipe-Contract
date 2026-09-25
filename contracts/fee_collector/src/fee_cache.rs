//! Fee rate caching for the fee_collector contract.
//!
//! Fee rate computations are cached in instance storage to avoid recomputing
//! them on every trade. The cache is invalidated whenever the fee rate config
//! is updated so that a rate change takes effect immediately rather than after
//! the cache TTL expires.

use soroban_sdk::{contracttype, Env};

/// Storage key for the cached fee rate.
#[contracttype]
#[derive(Clone)]
pub enum FeeCacheKey {
    /// Cached fee rate value.
    FeeRate,
    /// Monotonic version incremented on every fee rate config change.
    ConfigVersion,
}

/// Returns the current config version, defaulting to 0 when unset.
fn config_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&FeeCacheKey::ConfigVersion)
        .unwrap_or(0)
}

/// Reads the cached fee rate, if present and still valid.
///
/// The cache entry is keyed by the current config version so that any config
/// change automatically invalidates previously cached values.
pub fn get_cached_fee_rate(env: &Env) -> Option<u32> {
    let version = config_version(env);
    let cached: Option<(u32, u32)> = env.storage().instance().get(&FeeCacheKey::FeeRate);
    match cached {
        Some((cached_version, rate)) if cached_version == version => Some(rate),
        _ => None,
    }
}

/// Stores a computed fee rate in the cache, tagged with the current config
/// version.
pub fn set_cached_fee_rate(env: &Env, rate: u32) {
    let version = config_version(env);
    env.storage()
        .instance()
        .set(&FeeCacheKey::FeeRate, &(version, rate));
}

/// Invalidates the cached fee rate and bumps the config version.
///
/// Must be called after any fee rate config write (e.g. `set_fee_rate`,
/// `set_protocol_fee_bps`, `set_provider_fee_bps`) so that subsequent fee
/// queries recompute and return the new rate instead of a stale cached value.
pub fn invalidate_fee_cache(env: &Env) {
    let next = config_version(env).wrapping_add(1);
    env.storage()
        .instance()
        .set(&FeeCacheKey::ConfigVersion, &next);
    env.storage().instance().remove(&FeeCacheKey::FeeRate);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn cache_returns_stored_rate_between_config_changes() {
        let env = Env::default();
        assert_eq!(get_cached_fee_rate(&env), None);

        set_cached_fee_rate(&env, 30);
        assert_eq!(get_cached_fee_rate(&env), Some(30));

        // Cache remains effective while config is unchanged.
        assert_eq!(get_cached_fee_rate(&env), Some(30));
    }

    #[test]
    fn invalidate_fee_cache_clears_stale_rate() {
        let env = Env::default();

        // (a) query the cache before a rate change.
        set_cached_fee_rate(&env, 30);
        assert_eq!(get_cached_fee_rate(&env), Some(30));

        // (b) change the fee rate config.
        invalidate_fee_cache(&env);

        // (c) query again: the stale cached value must not be returned.
        assert_eq!(get_cached_fee_rate(&env), None);

        // New rate is cached and returned after recomputation.
        set_cached_fee_rate(&env, 50);
        assert_eq!(get_cached_fee_rate(&env), Some(50));
    }

    #[test]
    fn config_version_bump_invalidates_previous_cache_entry() {
        let env = Env::default();

        set_cached_fee_rate(&env, 30);
        assert_eq!(get_cached_fee_rate(&env), Some(30));

        // Simulate a config write that bumps the version without removing the
        // entry: the versioned key must auto-miss.
        let next = config_version(&env).wrapping_add(1);
        env.storage()
            .instance()
            .set(&FeeCacheKey::ConfigVersion, &next);

        assert_eq!(get_cached_fee_rate(&env), None);
    }
}
