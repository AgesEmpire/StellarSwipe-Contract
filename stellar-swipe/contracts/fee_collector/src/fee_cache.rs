//! Versioned fee-rate cache.
//!
//! The cached entry stores the config version it was computed under. Any fee
//! config write calls [`invalidate_fee_cache`], which bumps the version and
//! drops the entry, so stale rates can never be served after a rate change.

use soroban_sdk::{contracttype, Env};

use crate::storage::get_fee_rate;

#[contracttype]
#[derive(Clone)]
pub enum FeeCacheKey {
    ConfigVersion,
    Entry,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FeeCacheEntry {
    pub rate_bps: u32,
    pub version: u32,
}

pub fn config_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&FeeCacheKey::ConfigVersion)
        .unwrap_or(0)
}

/// Returns the fee rate, served from cache when the cached version is current.
pub fn cached_fee_rate(env: &Env) -> u32 {
    let version = config_version(env);
    if let Some(entry) = env
        .storage()
        .instance()
        .get::<_, FeeCacheEntry>(&FeeCacheKey::Entry)
    {
        if entry.version == version {
            return entry.rate_bps;
        }
    }
    let rate_bps = get_fee_rate(env);
    env.storage()
        .instance()
        .set(&FeeCacheKey::Entry, &FeeCacheEntry { rate_bps, version });
    rate_bps
}

/// Must be called after every fee config write.
pub fn invalidate_fee_cache(env: &Env) {
    let next = config_version(env).wrapping_add(1);
    env.storage().instance().set(&FeeCacheKey::ConfigVersion, &next);
    env.storage().instance().remove(&FeeCacheKey::Entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FeeCollector, FeeCollectorClient};
    use soroban_sdk::{testutils::Address as _, Address};

    #[test]
    fn test_cache_invalidated_on_fee_rate_change() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &id);
        client.initialize(&Address::generate(&env));

        // (a) populate cache
        assert_eq!(client.fee_rate(), 30);
        env.as_contract(&id, || {
            let entry: FeeCacheEntry = env.storage().instance().get(&FeeCacheKey::Entry).unwrap();
            assert_eq!(entry.rate_bps, 30);
        });

        // (b) change rate
        client.set_fee_rate(&75u32);

        // (c) new rate returned, not the stale cached one
        assert_eq!(client.fee_rate(), 75);
        env.as_contract(&id, || assert_eq!(config_version(&env), 1));
    }

    #[test]
    fn test_cache_hit_between_config_changes() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &id);
        client.initialize(&Address::generate(&env));
        client.fee_rate();

        // Bypass the setter: cache must still serve the cached value.
        env.as_contract(&id, || {
            crate::storage::set_fee_rate(&env, 99);
            assert_eq!(cached_fee_rate(&env), 30);
            invalidate_fee_cache(&env);
            assert_eq!(cached_fee_rate(&env), 99);
        });
    }
}
