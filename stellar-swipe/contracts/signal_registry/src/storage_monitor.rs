//! Storage capacity monitoring for the signal registry.
//!
//! Instance storage is a single ledger entry capped at 64 KB. We track the
//! number of entries in the three largest instance maps (Signals, ProviderStats,
//! ProviderStakes) as a proxy for usage and emit a warning event when the total
//! exceeds 80% of the configured limit.

use soroban_sdk::{Address, Env, Map};

use crate::events::emit_storage_capacity_warning;
use crate::expiry::archive_old_signals;
use crate::types::{ProviderPerformance, Signal};
use crate::stake::StakeInfo;
use crate::StorageKey;

/// Default entry-count limit for instance storage (conservative for 64 KB cap).
pub const INSTANCE_ENTRY_LIMIT: u32 = 1000;
/// Warning threshold: 80%.
const WARNING_THRESHOLD_BPS: u32 = 8000;

#[derive(Clone, Debug, PartialEq)]
pub struct StorageUsage {
    pub signal_count: u32,
    pub provider_stats_count: u32,
    pub provider_stakes_count: u32,
    pub total: u32,
    pub limit: u32,
    /// Usage in basis points (0-10000).
    pub usage_bps: u32,
}

/// Count entries across the three main instance maps and return usage stats.
pub fn get_storage_usage(env: &Env) -> StorageUsage {
    let signal_count = env
        .storage()
        .instance()
        .get::<_, Map<u64, Signal>>(&StorageKey::Signals)
        .map(|m| m.len())
        .unwrap_or(0);

    let provider_stats_count = env
        .storage()
        .instance()
        .get::<_, Map<Address, ProviderPerformance>>(&StorageKey::ProviderStats)
        .map(|m| m.len())
        .unwrap_or(0);

    let provider_stakes_count = env
        .storage()
        .instance()
        .get::<_, Map<Address, StakeInfo>>(&StorageKey::ProviderStakes)
        .map(|m| m.len())
        .unwrap_or(0);

    let total = signal_count + provider_stats_count + provider_stakes_count;
    let limit = INSTANCE_ENTRY_LIMIT;
    let usage_bps = ((total as u64 * 10000) / limit as u64) as u32;

    StorageUsage {
        signal_count,
        provider_stats_count,
        provider_stakes_count,
        total,
        limit,
        usage_bps,
    }
}

/// Check storage usage and emit a `StorageCapacityWarning` event if >= 80%.
/// Returns the current usage.
pub fn check_storage_capacity(env: &Env) -> StorageUsage {
    let usage = get_storage_usage(env);
    if usage.usage_bps >= WARNING_THRESHOLD_BPS {
        // storage_type 0 = instance
        emit_storage_capacity_warning(env, 0, usage.total, usage.limit);
    }
    usage
}

/// Admin-triggered cleanup: archive old expired signals to reduce instance storage.
/// Returns the number of signals removed.
pub fn admin_cleanup_storage(env: &Env, batch_size: u32) -> u32 {
    let signals: Map<u64, Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    archive_old_signals(env, &signals, batch_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{RiskLevel, SignalCategory};
    use crate::types::{SignalAction, SignalStatus};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, Map, String};

    fn make_signal(env: &Env, id: u64, status: SignalStatus, expiry: u64) -> Signal {
        Signal {
            id,
            provider: Address::generate(env),
            asset_pair: String::from_str(env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100,
            rationale: String::from_str(env, "test"),
            timestamp: env.ledger().timestamp(),
            expiry,
            status,
            executions: 0,
            successful_executions: 0,
            total_volume: 0,
            total_roi: 0,
            category: SignalCategory::SWING,
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
            tags: soroban_sdk::Vec::new(env),
            submitted_at: env.ledger().timestamp(),
            rationale_hash: String::from_str(env, "hash"),
            confidence: 50,
            adoption_count: 0,
            ai_validation_score: None,
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
        }
    }

    #[test]
    fn test_zero_usage_when_empty() {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            let usage = get_storage_usage(&env);
            assert_eq!(usage.total, 0);
            assert_eq!(usage.usage_bps, 0);
        });
    }

    #[test]
    fn test_warning_emitted_at_80_percent() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            // Insert 800 signals (80% of 1000 limit)
            let mut signals: Map<u64, Signal> = Map::new(&env);
            for i in 0..800u64 {
                signals.set(i, make_signal(&env, i, SignalStatus::Active, 2_000_000));
            }
            env.storage().instance().set(&StorageKey::Signals, &signals);

            let usage = check_storage_capacity(&env);
            assert!(usage.usage_bps >= 8000);
            assert_eq!(usage.signal_count, 800);
        });
    }

    #[test]
    fn test_no_warning_below_80_percent() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);
            for i in 0..799u64 {
                signals.set(i, make_signal(&env, i, SignalStatus::Active, 2_000_000));
            }
            env.storage().instance().set(&StorageKey::Signals, &signals);

            let usage = check_storage_capacity(&env);
            assert!(usage.usage_bps < 8000);
        });
    }

    #[test]
    fn test_cleanup_removes_old_expired_signals() {
        let env = Env::default();
        // Set time to 100 days in seconds so signals can be 31+ days expired
        let now: u64 = 100 * 24 * 60 * 60;
        env.ledger().set_timestamp(now);
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || {
            let mut signals: Map<u64, Signal> = Map::new(&env);
            // 5 signals expired 31+ days ago
            let old_expiry = now - (31 * 24 * 60 * 60);
            for i in 0..5u64 {
                let mut s = make_signal(&env, i, SignalStatus::Expired, old_expiry);
                s.status = SignalStatus::Expired;
                signals.set(i, s);
            }
            // 3 active signals
            for i in 5..8u64 {
                signals.set(i, make_signal(&env, i, SignalStatus::Active, now + 86400));
            }
            env.storage().instance().set(&StorageKey::Signals, &signals);

            let before = get_storage_usage(&env);
            assert_eq!(before.signal_count, 8);

            let removed = admin_cleanup_storage(&env, 10);
            assert_eq!(removed, 5);

            let after = get_storage_usage(&env);
            assert_eq!(after.signal_count, 3);
        });
    }
}
