//! Transaction-scoped fee configuration cache for `collect_fee` hot path.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};
use stellar_swipe_common::perf::{invalidate_tx_cache, tx_cache_or_compute};

use crate::storage::{
    get_burn_rate, get_fee_optimization_config, get_network_condition_score, get_protocol_token,
    FeeOptimizationConfig, MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS,
};
use stellar_swipe_common::Asset;

const CACHE_KEY: Symbol = symbol_short!("fee_cfg");

#[contracttype]
#[derive(Clone, Debug)]
pub struct TxFeeConfigCache {
    pub optimization_config: FeeOptimizationConfig,
    pub network_score: u32,
    pub burn_rate: u32,
    pub protocol_token: Option<Address>,
}

pub fn load_tx_fee_config(env: &Env) -> TxFeeConfigCache {
    tx_cache_or_compute(env, CACHE_KEY, || TxFeeConfigCache {
        optimization_config: get_fee_optimization_config(env),
        network_score: get_network_condition_score(env),
        burn_rate: get_burn_rate(env),
        protocol_token: get_protocol_token(env),
    })
}

/// Invalidate the cached [`TxFeeConfigCache`] entry.
///
/// `load_tx_fee_config` caches its result in Soroban temporary storage, which
/// survives across transactions until its TTL lapses (see the note on
/// `tx_cache_or_compute`). Every admin entry point that writes one of the
/// values baked into `TxFeeConfigCache` — the fee optimization config, the
/// network condition score, the burn rate, or the protocol token — MUST call
/// this immediately after the write. Otherwise `collect_fee` and
/// `current_dynamic_fee_rate` can keep serving the pre-change values for the
/// remainder of the cache's TTL window (issue #801).
///
/// This does not touch the base fee rate (`storage::get_fee_rate`), which is
/// read fresh on every call and was never part of this cache.
pub fn invalidate_fee_cache(env: &Env) {
    invalidate_tx_cache(env, CACHE_KEY);
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
    use soroban_sdk::testutils::Address as _;

    use crate::storage::set_fee_optimization_config;
    use crate::FeeCollector;

    fn register_contract(env: &Env) -> Address {
        env.register(FeeCollector, ())
    }

    fn sample_config(min_effective_rate_bps: u32) -> FeeOptimizationConfig {
        FeeOptimizationConfig {
            max_dynamic_rate_bps: MAX_FEE_RATE_BPS,
            congestion_sensitivity_bps: 0,
            min_effective_rate_bps,
            max_retry_attempts: 3,
        }
    }

    /// Issue #801: a fee-config write must not leave the tx-scoped cache
    /// serving stale values. Between config changes the cache must still
    /// short-circuit recomputation (no performance regression); only an
    /// explicit `invalidate_fee_cache` call (as every fee-config-mutating
    /// entry point now performs) should force a fresh read.
    #[test]
    fn invalidate_fee_cache_forces_fresh_read_after_config_change() {
        let env = Env::default();
        let contract_id = register_contract(&env);

        env.as_contract(&contract_id, || {
            // Seed the cache with the initial (low) minimum effective rate.
            set_fee_optimization_config(&env, &sample_config(MIN_FEE_RATE_BPS));
            let first = load_tx_fee_config(&env);
            assert_eq!(
                first.optimization_config.min_effective_rate_bps,
                MIN_FEE_RATE_BPS
            );

            // Change the config "out from under" the cache without invalidating yet.
            let new_min = MIN_FEE_RATE_BPS + 42;
            set_fee_optimization_config(&env, &sample_config(new_min));

            // Performance guarantee: with no invalidation, the cache still hits
            // and serves the previously cached value.
            let still_cached = load_tx_fee_config(&env);
            assert_eq!(
                still_cached.optimization_config.min_effective_rate_bps, MIN_FEE_RATE_BPS,
                "cache should still be effective between config changes"
            );

            // Now invalidate, exactly as every fee-config-mutating entry point does.
            invalidate_fee_cache(&env);

            let refreshed = load_tx_fee_config(&env);
            assert_eq!(
                refreshed.optimization_config.min_effective_rate_bps, new_min,
                "after invalidation the cache must recompute and reflect the new config"
            );
        });
    }

    /// The effective fee rate a trader is actually charged must reflect the
    /// new config immediately after invalidation, not the stale cached value.
    #[test]
    fn effective_fee_rate_reflects_new_config_after_invalidation() {
        let env = Env::default();
        let contract_id = register_contract(&env);
        let token = Address::generate(&env);

        env.as_contract(&contract_id, || {
            set_fee_optimization_config(&env, &sample_config(MIN_FEE_RATE_BPS));
            let cache_before = load_tx_fee_config(&env);
            let rate_before = effective_fee_rate_cached(&env, 0, &token, &cache_before);
            assert_eq!(rate_before, MIN_FEE_RATE_BPS);

            let new_min = MIN_FEE_RATE_BPS + 10;
            set_fee_optimization_config(&env, &sample_config(new_min));
            invalidate_fee_cache(&env);

            let cache_after = load_tx_fee_config(&env);
            let rate_after = effective_fee_rate_cached(&env, 0, &token, &cache_after);
            assert_eq!(
                rate_after, new_min,
                "trader-facing effective fee rate must use the new config after invalidation"
            );
        });
    }
}
