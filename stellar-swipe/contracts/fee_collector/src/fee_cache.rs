//! Versioned fee-rate cache.
//!
//! The cached entry stores the config version it was computed under. Any fee
//! config write calls [`invalidate_fee_cache`], which bumps the version and
//! drops the entry, so stale rates can never be served after a rate change.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

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

// ── Transaction-scoped fee configuration cache for the `collect_fee` hot path ──

use stellar_swipe_common::perf::tx_cache_or_compute;

use crate::storage::{
    bump_config_version as _, get_burn_rate, get_config_version, get_fee_optimization_config,
    get_network_condition_score, get_protocol_token, FeeOptimizationConfig, MAX_FEE_RATE_BPS,
    MIN_FEE_RATE_BPS,
};
use stellar_swipe_common::Asset;

/// Cache key includes the config version so any fee-config write automatically
/// produces a cache miss on the next call — no explicit flush needed.
fn versioned_cache_key(env: &Env) -> Symbol {
    let v = get_config_version(env);
    // Encode version into a short symbol: "fc_" + last digit of version.
    // For a tx-scoped cache this is sufficient: within a single transaction
    // the version is stable; across transactions the Env is fresh anyway.
    // We use a fixed-width decimal suffix (0-9, wrapping) which fits the
    // 9-char symbol_short limit.
    match v % 10 {
        0 => symbol_short!("fee_cfg_0"),
        1 => symbol_short!("fee_cfg_1"),
        2 => symbol_short!("fee_cfg_2"),
        3 => symbol_short!("fee_cfg_3"),
        4 => symbol_short!("fee_cfg_4"),
        5 => symbol_short!("fee_cfg_5"),
        6 => symbol_short!("fee_cfg_6"),
        7 => symbol_short!("fee_cfg_7"),
        8 => symbol_short!("fee_cfg_8"),
        _ => symbol_short!("fee_cfg_9"),
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TxFeeConfigCache {
    pub optimization_config: FeeOptimizationConfig,
    pub network_score: u32,
    pub burn_rate: u32,
    pub protocol_token: Option<Address>,
}

pub fn load_tx_fee_config(env: &Env) -> TxFeeConfigCache {
    tx_cache_or_compute(env, versioned_cache_key(env), || TxFeeConfigCache {
        optimization_config: get_fee_optimization_config(env),
        network_score: get_network_condition_score(env),
        burn_rate: get_burn_rate(env),
        protocol_token: get_protocol_token(env),
    })
}

pub fn effective_fee_rate_cached(
    _env: &Env,
    base_rate: u32,
    token: &Address,
    cache: &TxFeeConfigCache,
) -> u32 {
    let config = &cache.optimization_config;
    let network_adjustment = (cache.network_score as u64)
        .saturating_mul(config.congestion_sensitivity_bps as u64)
        .checked_div(10_000)
        .unwrap_or(0) as u32;

    let mut fee_rate = base_rate.saturating_add(network_adjustment);
    fee_rate = fee_rate.max(config.min_effective_rate_bps);
    fee_rate = fee_rate.min(config.max_dynamic_rate_bps.min(MAX_FEE_RATE_BPS));

    if let Some(ref protocol_token) = cache.protocol_token {
        if token == protocol_token {
            fee_rate = (fee_rate / 2).max(MIN_FEE_RATE_BPS);
        }
    }

    fee_rate
}

/// No-op placeholder for batch settlement: volume oracle flush deferred to keeper.
pub fn defer_volume_record(_env: &Env, _trader: &Address, _asset: &Asset, _amount: i128) {
    // Intentionally empty — batch fee settlement can aggregate off-chain.
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
