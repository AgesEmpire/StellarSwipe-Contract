//! Per-provider and global ledger-window submission quotas (Issue #1138).
//!
//! Windows are `ledger_sequence / window_ledgers`, so counters reset
//! deterministically at window boundaries.  [`check`] is read-only and runs
//! before any state is written; [`consume`] runs only after a submission has
//! fully succeeded, so a rejected submission leaves no partial state.
//! A limit of `0` disables that quota.
//!
//! Issue #1422 extends this module with per-category ceilings.  Governance
//! may store a `CategoryQuotaConfig` for each `SignalCategory`.  A category
//! quota is entirely independent of the global `QuotaConfig`, so both can be
//! active simultaneously.  Category counters are also held in temporary
//! storage and keyed by `(category_discriminant, window)` so they reset at
//! the same `window_ledgers` boundary as the global quota.

use crate::categories::SignalCategory;
use crate::errors::AdminError;
use soroban_sdk::{contracttype, Address, Env};

/// Bounds for administrator-configured limits.
pub const MIN_WINDOW_LEDGERS: u32 = 1;
pub const MAX_WINDOW_LEDGERS: u32 = 535_680; // ~31 days at 5s ledgers
pub const MAX_PROVIDER_LIMIT: u32 = 1_000;
pub const MAX_GLOBAL_LIMIT: u32 = 100_000;
/// Maximum per-category ceiling a governance call may set.
pub const MAX_CATEGORY_LIMIT: u32 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaConfig {
    pub window_ledgers: u32,
    pub per_provider_limit: u32,
    pub global_limit: u32,
}

/// Per-category submission ceiling (Issue #1422).
///
/// `window_ledgers` must match the global `QuotaConfig::window_ledgers` when
/// both are active, so the same ledger-window boundary is used everywhere.
/// A `category_limit` of `0` disables the category quota.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryQuotaConfig {
    /// Must be in `[MIN_WINDOW_LEDGERS, MAX_WINDOW_LEDGERS]`.
    pub window_ledgers: u32,
    /// Maximum total submissions accepted for this category per window.
    /// `0` disables the ceiling.
    pub category_limit: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum QuotaKey {
    Config,
    /// (provider, window) -> count
    Provider(Address, u32),
    /// window -> count
    Global(u32),
    /// Per-category global counter: (category_discriminant, window) -> count.
    /// We encode `SignalCategory` as a `u32` discriminant to keep the key
    /// `contracttype`-compatible without nesting an enum inside another.
    Category(u32, u32),
    /// Per-category quota configuration keyed by category discriminant.
    CategoryConfig(u32),
}

/// Map `SignalCategory` to a stable `u32` discriminant for storage keys.
pub fn category_discriminant(cat: &SignalCategory) -> u32 {
    match cat {
        SignalCategory::SCALP => 0,
        SignalCategory::SWING => 1,
        SignalCategory::LONG_TERM => 2,
        SignalCategory::ARBITRAGE => 3,
        SignalCategory::PREMIUM => 4,
    }
}

pub fn validate(config: &QuotaConfig) -> Result<(), AdminError> {
    if config.window_ledgers < MIN_WINDOW_LEDGERS
        || config.window_ledgers > MAX_WINDOW_LEDGERS
        || config.per_provider_limit > MAX_PROVIDER_LIMIT
        || config.global_limit > MAX_GLOBAL_LIMIT
        || (config.global_limit != 0 && config.per_provider_limit > config.global_limit)
    {
        return Err(AdminError::InvalidParameter);
    }
    Ok(())
}

/// Validate a `CategoryQuotaConfig`.
///
/// - `window_ledgers` must be within `[MIN_WINDOW_LEDGERS, MAX_WINDOW_LEDGERS]`.
/// - `category_limit` must not exceed `MAX_CATEGORY_LIMIT`.
/// - When a global `QuotaConfig` is active its `window_ledgers` must match,
///   so category and global counters always align to the same boundary.
pub fn validate_category(env: &Env, config: &CategoryQuotaConfig) -> Result<(), AdminError> {
    if config.window_ledgers < MIN_WINDOW_LEDGERS || config.window_ledgers > MAX_WINDOW_LEDGERS {
        return Err(AdminError::InvalidParameter);
    }
    if config.category_limit > MAX_CATEGORY_LIMIT {
        return Err(AdminError::InvalidParameter);
    }
    // When a global quota is configured the window lengths must agree.
    if let Some(global) = get_config(env) {
        if global.window_ledgers != config.window_ledgers {
            return Err(AdminError::InvalidParameter);
        }
    }
    Ok(())
}

pub fn set_config(env: &Env, config: &QuotaConfig) -> Result<(), AdminError> {
    validate(config)?;
    env.storage().instance().set(&QuotaKey::Config, config);
    Ok(())
}

/// Store a per-category quota ceiling (Issue #1422).
pub fn set_category_config(
    env: &Env,
    category: &SignalCategory,
    config: &CategoryQuotaConfig,
) -> Result<(), AdminError> {
    validate_category(env, config)?;
    let disc = category_discriminant(category);
    env.storage()
        .instance()
        .set(&QuotaKey::CategoryConfig(disc), config);
    Ok(())
}

/// Retrieve the per-category quota config, if one has been set.
pub fn get_category_config(env: &Env, category: &SignalCategory) -> Option<CategoryQuotaConfig> {
    let disc = category_discriminant(category);
    env.storage()
        .instance()
        .get(&QuotaKey::CategoryConfig(disc))
}

pub fn get_config(env: &Env) -> Option<QuotaConfig> {
    env.storage().instance().get(&QuotaKey::Config)
}

pub fn current_window(env: &Env, window_ledgers: u32) -> u32 {
    env.ledger().sequence() / window_ledgers
}

fn count(env: &Env, key: &QuotaKey) -> u32 {
    env.storage().temporary().get(key).unwrap_or(0)
}

/// Returns `(provider_used, global_used)` for the current window.
pub fn usage(env: &Env, provider: &Address) -> (u32, u32) {
    match get_config(env) {
        None => (0, 0),
        Some(c) => {
            let w = current_window(env, c.window_ledgers);
            (
                count(env, &QuotaKey::Provider(provider.clone(), w)),
                count(env, &QuotaKey::Global(w)),
            )
        }
    }
}

/// Returns the number of submissions recorded for `category` in the current window.
/// Returns `0` when no category config has been set for this category.
pub fn category_usage(env: &Env, category: &SignalCategory) -> u32 {
    let Some(cc) = get_category_config(env, category) else {
        return 0;
    };
    let w = current_window(env, cc.window_ledgers);
    let disc = category_discriminant(category);
    count(env, &QuotaKey::Category(disc, w))
}

/// Read-only quota check; performs no writes.
///
/// Checks both the global per-provider / global limits **and** the
/// per-category ceiling (Issue #1422).  The category ceiling does not
/// bypass the global limit — both must pass for `Ok(())` to be returned.
pub fn check(env: &Env, provider: &Address, category: &SignalCategory) -> Result<(), AdminError> {
    // --- global quota ---
    if let Some(c) = get_config(env) {
        let w = current_window(env, c.window_ledgers);
        if c.per_provider_limit > 0
            && count(env, &QuotaKey::Provider(provider.clone(), w)) >= c.per_provider_limit
        {
            return Err(AdminError::RateLimitExceeded);
        }
        if c.global_limit > 0 && count(env, &QuotaKey::Global(w)) >= c.global_limit {
            return Err(AdminError::RateLimitExceeded);
        }
    }

    // --- per-category quota (Issue #1422) ---
    if let Some(cc) = get_category_config(env, category) {
        if cc.category_limit > 0 {
            let w = current_window(env, cc.window_ledgers);
            let disc = category_discriminant(category);
            if count(env, &QuotaKey::Category(disc, w)) >= cc.category_limit {
                return Err(AdminError::RateLimitExceeded);
            }
        }
    }

    Ok(())
}

/// Records one accepted submission.  Call only after the submission succeeded.
///
/// Increments the global per-provider and global counters (when a global
/// config is present) **and** the per-category counter (when a category
/// config is present).  All increments are atomic with the same call.
pub fn consume(env: &Env, provider: &Address, category: &SignalCategory) {
    // --- global counters ---
    if let Some(c) = get_config(env) {
        let w = current_window(env, c.window_ledgers);
        // Counters live one window past their own so they expire on their own.
        let ttl = c.window_ledgers.saturating_mul(2);
        for key in [QuotaKey::Provider(provider.clone(), w), QuotaKey::Global(w)] {
            let next = count(env, &key).saturating_add(1);
            env.storage().temporary().set(&key, &next);
            env.storage().temporary().extend_ttl(&key, ttl, ttl);
        }
    }

    // --- per-category counter (Issue #1422) ---
    if let Some(cc) = get_category_config(env, category) {
        let w = current_window(env, cc.window_ledgers);
        let disc = category_discriminant(category);
        let cat_key = QuotaKey::Category(disc, w);
        let next = count(env, &cat_key).saturating_add(1);
        let ttl = cc.window_ledgers.saturating_mul(2);
        env.storage().temporary().set(&cat_key, &next);
        env.storage().temporary().extend_ttl(&cat_key, ttl, ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn make_provider(env: &Env) -> Address {
        Address::generate(env)
    }

    // ── #1422 category quota tests ────────────────────────────────────────────

    #[test]
    fn test_category_quota_no_config_always_passes() {
        let env = Env::default();
        let provider = make_provider(&env);
        // No config set for SCALP — check should pass.
        assert!(check(&env, &provider, &SignalCategory::SCALP).is_ok());
    }

    #[test]
    fn test_category_quota_exhaustion_blocks_submission() {
        let env = Env::default();
        let provider = make_provider(&env);

        set_category_config(
            &env,
            &SignalCategory::SWING,
            &CategoryQuotaConfig {
                window_ledgers: 100,
                category_limit: 2,
            },
        )
        .unwrap();

        // First two submissions pass.
        assert!(check(&env, &provider, &SignalCategory::SWING).is_ok());
        consume(&env, &provider, &SignalCategory::SWING);
        assert!(check(&env, &provider, &SignalCategory::SWING).is_ok());
        consume(&env, &provider, &SignalCategory::SWING);

        // Third is blocked.
        assert_eq!(
            check(&env, &provider, &SignalCategory::SWING),
            Err(AdminError::RateLimitExceeded)
        );
    }

    #[test]
    fn test_category_quota_does_not_affect_other_categories() {
        let env = Env::default();
        let provider = make_provider(&env);

        set_category_config(
            &env,
            &SignalCategory::SCALP,
            &CategoryQuotaConfig {
                window_ledgers: 100,
                category_limit: 1,
            },
        )
        .unwrap();

        consume(&env, &provider, &SignalCategory::SCALP);
        // SCALP is now exhausted.
        assert!(check(&env, &provider, &SignalCategory::SCALP).is_err());
        // SWING has no quota — still passes.
        assert!(check(&env, &provider, &SignalCategory::SWING).is_ok());
    }

    #[test]
    fn test_category_quota_window_reset() {
        let env = Env::default();
        let provider = make_provider(&env);

        set_category_config(
            &env,
            &SignalCategory::LONG_TERM,
            &CategoryQuotaConfig {
                window_ledgers: 10,
                category_limit: 1,
            },
        )
        .unwrap();

        consume(&env, &provider, &SignalCategory::LONG_TERM);
        assert!(check(&env, &provider, &SignalCategory::LONG_TERM).is_err());

        // Advance ledger past the window boundary.
        env.ledger().with_mut(|li| li.sequence_number += 10);
        // Counter is now in a new window — quota resets.
        assert!(check(&env, &provider, &SignalCategory::LONG_TERM).is_ok());
    }

    #[test]
    fn test_category_quota_does_not_bypass_global_limit() {
        let env = Env::default();
        let provider = make_provider(&env);

        // Global quota: max 1 submission per provider per window.
        set_config(
            &env,
            &QuotaConfig {
                window_ledgers: 100,
                per_provider_limit: 1,
                global_limit: 100,
            },
        )
        .unwrap();
        // Category quota: generous ceiling of 10.
        set_category_config(
            &env,
            &SignalCategory::ARBITRAGE,
            &CategoryQuotaConfig {
                window_ledgers: 100,
                category_limit: 10,
            },
        )
        .unwrap();

        // First submission OK and consume both counters.
        assert!(check(&env, &provider, &SignalCategory::ARBITRAGE).is_ok());
        consume(&env, &provider, &SignalCategory::ARBITRAGE);

        // Global per-provider limit now blocks even though category ceiling is not reached.
        assert_eq!(
            check(&env, &provider, &SignalCategory::ARBITRAGE),
            Err(AdminError::RateLimitExceeded)
        );
    }

    #[test]
    fn test_category_usage_returns_correct_count() {
        let env = Env::default();
        let p1 = make_provider(&env);
        let p2 = make_provider(&env);

        set_category_config(
            &env,
            &SignalCategory::PREMIUM,
            &CategoryQuotaConfig {
                window_ledgers: 100,
                category_limit: 50,
            },
        )
        .unwrap();

        consume(&env, &p1, &SignalCategory::PREMIUM);
        consume(&env, &p2, &SignalCategory::PREMIUM);

        // Both submissions land in the same global category counter.
        assert_eq!(category_usage(&env, &SignalCategory::PREMIUM), 2);
        // Other categories untouched.
        assert_eq!(category_usage(&env, &SignalCategory::SWING), 0);
    }

    #[test]
    fn test_validate_category_config_window_mismatch_with_global() {
        let env = Env::default();

        set_config(
            &env,
            &QuotaConfig {
                window_ledgers: 100,
                per_provider_limit: 10,
                global_limit: 1000,
            },
        )
        .unwrap();

        // Category window differs from global window → should be rejected.
        let result = set_category_config(
            &env,
            &SignalCategory::SWING,
            &CategoryQuotaConfig {
                window_ledgers: 200,
                category_limit: 5,
            },
        );
        assert_eq!(result, Err(AdminError::InvalidParameter));
    }

    #[test]
    fn test_category_limit_zero_disables_quota() {
        let env = Env::default();
        let provider = make_provider(&env);

        set_category_config(
            &env,
            &SignalCategory::SCALP,
            &CategoryQuotaConfig {
                window_ledgers: 100,
                category_limit: 0, // disabled
            },
        )
        .unwrap();

        // Even after many consumes the check should still pass.
        for _ in 0..200 {
            consume(&env, &provider, &SignalCategory::SCALP);
        }
        assert!(check(&env, &provider, &SignalCategory::SCALP).is_ok());
    }
}
