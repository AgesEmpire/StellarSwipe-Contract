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
//!
//! Finally, this module tracks collected fee balances independently per asset.
//! Each asset (identified by its contract address) has its own balance so that
//! collection, balance queries, and dust handling never mix units across
//! assets. Deployments that previously stored a single aggregate balance can
//! migrate it into a chosen asset bucket via [`migrate_aggregate_balance`].

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
    /// Collected fee balance scoped to a specific asset (by contract address).
    AssetBalance(Address),
    /// Legacy aggregate balance key, retained only for migration.
    AggregateBalance,
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

/// Returns the collected fee balance for `asset`, defaulting to 0.
///
/// Balances are tracked per asset so that fees denominated in different
/// Stellar assets are never summed together.
pub fn get_asset_balance(env: &Env, asset: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&FeeCacheKey::AssetBalance(asset.clone()))
        .unwrap_or(0)
}

/// Credits `amount` of `asset` to the asset-scoped fee balance.
///
/// Only the balance for `asset` is affected; other assets are untouched.
pub fn credit_asset_balance(env: &Env, asset: &Address, amount: i128) {
    let current = get_asset_balance(env, asset);
    let next = current
        .checked_add(amount)
        .expect("asset balance overflow");
    env.storage()
        .instance()
        .set(&FeeCacheKey::AssetBalance(asset.clone()), &next);
}

/// Returns true when `asset`'s balance is at or below `dust_threshold` and
/// therefore eligible for dust handling.
///
/// Eligibility is evaluated strictly against the selected asset's balance so
/// that a large balance in one asset cannot mask dust in another.
pub fn is_dust_eligible(env: &Env, asset: &Address, dust_threshold: i128) -> bool {
    get_asset_balance(env, asset) <= dust_threshold
}

/// Deducts `amount` of `asset` from the asset-scoped fee balance.
///
/// Panics if `amount` exceeds the balance for `asset`, preventing deductions
/// from being satisfied by another asset's funds.
pub fn deduct_asset_balance(env: &Env, asset: &Address, amount: i128) {
    let current = get_asset_balance(env, asset);
    if amount > current {
        panic!("insufficient asset balance");
    }
    let next = current - amount;
    env.storage()
        .instance()
        .set(&FeeCacheKey::AssetBalance(asset.clone()), &next);
}

/// Migrates a legacy aggregate balance into the balance bucket for `asset`.
///
/// Existing deployments that stored a single aggregate balance can call this
/// once to move that value into a chosen asset bucket. The legacy key is
/// removed so the migration is idempotent; subsequent calls are no-ops.
pub fn migrate_aggregate_balance(env: &Env, asset: &Address) {
    let legacy: Option<i128> = env.storage().instance().get(&FeeCacheKey::AggregateBalance);
    if let Some(amount) = legacy {
        credit_asset_balance(env, asset, amount);
        env.storage()
            .instance()
            .remove(&FeeCacheKey::AggregateBalance);
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

    #[test]
    fn balances_are_tracked_independently_per_asset() {
        let env = Env::default();
        let usdc = Address::generate(&env);
        let eurc = Address::generate(&env);

        assert_eq!(get_asset_balance(&env, &usdc), 0);
        assert_eq!(get_asset_balance(&env, &eurc), 0);

        credit_asset_balance(&env, &usdc, 1_000);
        credit_asset_balance(&env, &eurc, 250);

        // Each asset keeps its own balance; no cross-asset mixing.
        assert_eq!(get_asset_balance(&env, &usdc), 1_000);
        assert_eq!(get_asset_balance(&env, &eurc), 250);
    }

    #[test]
    fn dust_eligibility_and_deduction_are_asset_scoped() {
        let env = Env::default();
        let usdc = Address::generate(&env);
        let eurc = Address::generate(&env);

        credit_asset_balance(&env, &usdc, 10_000);
        credit_asset_balance(&env, &eurc, 5);

        // Only the small-balance asset is dust eligible.
        assert!(!is_dust_eligible(&env, &usdc, 100));
        assert!(is_dust_eligible(&env, &eurc, 100));

        // Deducting from one asset leaves the other untouched.
        deduct_asset_balance(&env, &eurc, 5);
        assert_eq!(get_asset_balance(&env, &eurc), 0);
        assert_eq!(get_asset_balance(&env, &usdc), 10_000);
    }

    #[test]
    #[should_panic(expected = "insufficient asset balance")]
    fn deduction_cannot_borrow_from_another_asset() {
        let env = Env::default();
        let usdc = Address::generate(&env);
        let eurc = Address::generate(&env);

        credit_asset_balance(&env, &usdc, 1_000);
        // eurc has no balance; a large usdc balance must not cover it.
        deduct_asset_balance(&env, &eurc, 1);
    }

    #[test]
    fn legacy_aggregate_balance_migrates_into_asset_bucket() {
        let env = Env::default();
        let usdc = Address::generate(&env);

        env.storage()
            .instance()
            .set(&FeeCacheKey::AggregateBalance, &500i128);

        migrate_aggregate_balance(&env, &usdc);
        assert_eq!(get_asset_balance(&env, &usdc), 500);

        // Migration is idempotent: the legacy key is gone.
        migrate_aggregate_balance(&env, &usdc);
        assert_eq!(get_asset_balance(&env, &usdc), 500);
    }
}
