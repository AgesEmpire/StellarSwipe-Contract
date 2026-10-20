//! Fee rate caching for the fee_collector contract.
//!
//! Fee rate computations are cached in instance storage to avoid recomputing
//! them on every trade. The cache is invalidated whenever the fee rate config
//! is updated so that a rate change takes effect immediately rather than after
//! the cache TTL expires.
//!
//! This module also maintains the allowlist of authorized revenue sources.
//! Fee deposits are restricted to registered sources so that arbitrary
//! contracts cannot contaminate accounting or attribution reports. Allowlist
//! mutations are authorized and evented, and unknown sources are rejected
//! before any token transfer is attempted.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

/// Storage key for the cached fee rate.
#[contracttype]
#[derive(Clone)]
pub enum FeeCacheKey {
    /// Cached fee rate value.
    FeeRate,
    /// Monotonic version incremented on every fee rate config change.
    ConfigVersion,
    /// Allowlist entry for an authorized revenue source.
    RevenueSource(Address),
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

/// Returns true when `source` is a registered, authorized revenue source.
pub fn is_revenue_source(env: &Env, source: &Address) -> bool {
    env.storage()
        .instance()
        .get(&FeeCacheKey::RevenueSource(source.clone()))
        .unwrap_or(false)
}

/// Registers `source` as an authorized revenue source.
///
/// The caller is responsible for authorization (admin/owner gating); this
/// helper only performs the storage write and emits the allowlist event.
pub fn add_revenue_source(env: &Env, source: &Address) {
    env.storage()
        .instance()
        .set(&FeeCacheKey::RevenueSource(source.clone()), &true);
    env.events()
        .publish((symbol_short!("rev_src_add"), source.clone()), ());
}

/// Removes `source` from the authorized revenue source allowlist.
///
/// Removal takes effect immediately: any subsequent deposit from `source`
/// fails the allowlist check before a token transfer is attempted, even if an
/// accounting period is still in flight.
pub fn remove_revenue_source(env: &Env, source: &Address) {
    env.storage()
        .instance()
        .remove(&FeeCacheKey::RevenueSource(source.clone()));
    env.events()
        .publish((symbol_short!("rev_src_rm"), source.clone()), ());
}

/// Guards a fee deposit, failing before any token transfer when `source` is
/// not an authorized revenue source.
///
/// Call this at the very start of the deposit entrypoint, before invoking the
/// token contract, so unknown sources cannot move funds or contaminate
/// accounting.
pub fn require_revenue_source(env: &Env, source: &Address) {
    if !is_revenue_source(env, source) {
        panic!("revenue source not allowlisted");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

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

    #[test]
    fn unknown_source_is_rejected() {
        let env = Env::default();
        let source = Address::generate(&env);

        assert!(!is_revenue_source(&env, &source));
    }

    #[test]
    fn registered_source_remains_compatible() {
        let env = Env::default();
        let source = Address::generate(&env);

        add_revenue_source(&env, &source);
        assert!(is_revenue_source(&env, &source));

        // Existing registered sources keep passing the deposit guard.
        require_revenue_source(&env, &source);
    }

    #[test]
    #[should_panic(expected = "revenue source not allowlisted")]
    fn deposit_guard_fails_for_unknown_source() {
        let env = Env::default();
        let source = Address::generate(&env);

        require_revenue_source(&env, &source);
    }

    #[test]
    #[should_panic(expected = "revenue source not allowlisted")]
    fn removal_during_in_flight_period_blocks_deposit() {
        let env = Env::default();
        let source = Address::generate(&env);

        // Source is registered and an accounting period is in flight.
        add_revenue_source(&env, &source);
        require_revenue_source(&env, &source);

        // Removal takes effect immediately, before any token transfer.
        remove_revenue_source(&env, &source);
        assert!(!is_revenue_source(&env, &source));

        require_revenue_source(&env, &source);
    }
}
