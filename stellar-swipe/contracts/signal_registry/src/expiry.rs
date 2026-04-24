use soroban_sdk::{Address, Env, Map, Vec};
use stellar_swipe_common::{SECONDS_PER_30_DAY_MONTH, SECONDS_PER_DAY};

use crate::events::emit_signal_expired;
use crate::types::{Signal, SignalStatus};

pub const DEFAULT_EXPIRY_SECONDS: u64 = SECONDS_PER_DAY; // 24 hours
pub const MAX_CLEANUP_BATCH_SIZE: u32 = 100; // Process max 100 signals per cleanup call
pub const ARCHIVE_THRESHOLD_SECONDS: u64 = SECONDS_PER_30_DAY_MONTH; // 30 days

#[derive(Clone, Debug, PartialEq)]
pub struct CleanupResult {
    pub signals_processed: u32,
    pub signals_expired: u32,
}

/// Check if a signal has expired based on current time
pub fn is_expired(env: &Env, signal: &Signal) -> bool {
    let current_time = env.ledger().timestamp();
    current_time > signal.expiry
}

/// Check if signal should be archived (expired for more than 30 days)
pub fn should_archive(env: &Env, signal: &Signal) -> bool {
    if signal.status != SignalStatus::Expired {
        return false;
    }

    let current_time = env.ledger().timestamp();
    let time_since_expiry = current_time.saturating_sub(signal.expiry);
    time_since_expiry > ARCHIVE_THRESHOLD_SECONDS
}

/// Update signal to expired status if it has passed expiry time
/// Returns true if status was changed
pub fn check_and_update_expiry(env: &Env, signal: &mut Signal) -> bool {
    // Skip if already expired or executed
    if signal.status == SignalStatus::Expired || signal.status == SignalStatus::Executed {
        return false;
    }

    if is_expired(env, signal) {
        signal.status = SignalStatus::Expired;

        // Emit expiry event
        emit_signal_expired(env, signal.id, signal.provider.clone(), signal.expiry);

        true
    } else {
        false
    }
}

/// Get a signal with automatic expiry checking
pub fn get_signal_with_expiry_check(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    signal_id: u64,
) -> Option<Signal> {
    if let Some(mut signal) = signals_map.get(signal_id) {
        // Check and update expiry status
        if check_and_update_expiry(env, &mut signal) {
            // Status was updated, save it back
            let mut updated_map = signals_map.clone();
            updated_map.set(signal_id, signal.clone());
            env.storage()
                .instance()
                .set(&crate::StorageKey::Signals, &updated_map);
        }
        Some(signal)
    } else {
        None
    }
}

/// Get all active (non-expired) signals for feed
pub fn get_active_signals(env: &Env, signals_map: &Map<u64, Signal>) -> Vec<Signal> {
    let mut active_signals = Vec::new(env);
    let current_time = env.ledger().timestamp();

    // Collect all keys first
    let mut keys = Vec::new(env);
    for i in 0..signals_map.len() {
        if let Some(key) = signals_map.keys().get(i) {
            keys.push_back(key);
        }
    }

    // Then get signals by key
    for i in 0..keys.len() {
        let key = keys.get(i).unwrap();
        if let Some(signal) = signals_map.get(key) {
            // Only include non-expired signals
            if signal.expiry > current_time
                && signal.status != SignalStatus::Expired
                && signal.status != SignalStatus::Executed
            {
                active_signals.push_back(signal);
            }
        }
    }

    active_signals
}

/// Check if address is in list
fn is_in_list(list: &Vec<Address>, addr: &Address) -> bool {
    for i in 0..list.len() {
        if list.get(i).unwrap() == *addr {
            return true;
        }
    }
    false
}

/// Get active signals filtered to only those from followed providers.
/// If followed_providers is empty, returns empty Vec.
pub fn get_active_signals_filtered(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    followed_providers: &Vec<Address>,
) -> Vec<Signal> {
    if followed_providers.is_empty() {
        return Vec::new(env);
    }
    let all_active = get_active_signals(env, signals_map);
    let mut filtered = Vec::new(env);
    for i in 0..all_active.len() {
        let signal = all_active.get(i).unwrap();
        if is_in_list(followed_providers, &signal.provider) {
            filtered.push_back(signal);
        }
    }
    filtered
}

/// Cleanup expired signals in batches
/// Returns number of signals processed and expired
pub fn cleanup_expired_signals(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    limit: u32,
) -> CleanupResult {
    let batch_size = if limit == 0 || limit > MAX_CLEANUP_BATCH_SIZE {
        MAX_CLEANUP_BATCH_SIZE
    } else {
        limit
    };

    let current_time = env.ledger().timestamp();
    let mut signals_processed = 0u32;
    let mut signals_expired = 0u32;
    let mut updated_map = signals_map.clone();

    // Collect all keys first
    let mut keys = Vec::new(env);
    for i in 0..signals_map.len() {
        if let Some(key) = signals_map.keys().get(i) {
            keys.push_back(key);
        }
    }

    // Iterate through keys
    for i in 0..keys.len() {
        if signals_processed >= batch_size {
            break;
        }

        let signal_id = keys.get(i).unwrap();
        if let Some(mut signal) = signals_map.get(signal_id) {
            // Skip already expired or executed signals
            if signal.status == SignalStatus::Expired || signal.status == SignalStatus::Executed {
                continue;
            }

            signals_processed += 1;

            // Check if expired
            if signal.expiry < current_time {
                signal.status = SignalStatus::Expired;
                updated_map.set(signal_id, signal.clone());
                signals_expired += 1;

                // Emit expiry event
                emit_signal_expired(env, signal.id, signal.provider.clone(), signal.expiry);
            }
        }
    }

    // Save updated map if any changes were made
    if signals_expired > 0 {
        env.storage()
            .instance()
            .set(&crate::StorageKey::Signals, &updated_map);
    }

    CleanupResult {
        signals_processed,
        signals_expired,
    }
}

/// Archive old expired signals (optional - removes from active storage)
/// Returns number of signals archived
pub fn archive_old_signals(env: &Env, signals_map: &Map<u64, Signal>, limit: u32) -> u32 {
    let batch_size = if limit == 0 || limit > MAX_CLEANUP_BATCH_SIZE {
        MAX_CLEANUP_BATCH_SIZE
    } else {
        limit
    };

    let current_time = env.ledger().timestamp();
    let mut archived_count = 0u32;
    let mut updated_map = signals_map.clone();

    // Collect signal IDs to archive
    let mut to_archive = Vec::new(env);

    // Collect all keys first
    let mut keys = Vec::new(env);
    for i in 0..signals_map.len() {
        if let Some(key) = signals_map.keys().get(i) {
            keys.push_back(key);
        }
    }

    for i in 0..keys.len() {
        if archived_count >= batch_size {
            break;
        }

        let signal_id = keys.get(i).unwrap();
        if let Some(signal) = signals_map.get(signal_id) {
            // Only archive signals expired for more than 30 days
            if signal.status == SignalStatus::Expired {
                let time_since_expiry = current_time.saturating_sub(signal.expiry);
                if time_since_expiry > ARCHIVE_THRESHOLD_SECONDS {
                    to_archive.push_back(signal_id);
                    archived_count += 1;
                }
            }
        }
    }

    // Remove archived signals from active storage
    for i in 0..to_archive.len() {
        let signal_id = to_archive.get(i).unwrap();
        updated_map.remove(signal_id);
    }

    // Save updated map if any signals were archived
    if archived_count > 0 {
        env.storage()
            .instance()
            .set(&crate::StorageKey::Signals, &updated_map);
    }

    archived_count
}

/// Get count of expired signals
pub fn count_expired_signals(signals_map: &Map<u64, Signal>) -> u32 {
    let mut count = 0u32;

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                if signal.status == SignalStatus::Expired {
                    count += 1;
                }
            }
        }
    }

    count
}

/// Get count of signals pending expiry check
pub fn count_signals_pending_expiry(env: &Env, signals_map: &Map<u64, Signal>) -> u32 {
    let current_time = env.ledger().timestamp();
    let mut count = 0u32;

    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(signal) = signals_map.get(key) {
                // Count signals that are past expiry but not yet marked as expired
                if signal.expiry < current_time
                    && signal.status != SignalStatus::Expired
                    && signal.status != SignalStatus::Executed
                {
                    count += 1;
                }
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SignalAction;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Address, Env, String,
    };

    fn create_test_signal(env: &Env, id: u64, expiry: u64) -> Signal {
        Signal {
            id,
            provider: Address::generate(env),
            asset_pair: String::from_str(env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100_000,
            rationale: String::from_str(env, "Test signal"),
            timestamp: env.ledger().timestamp(),
            expiry,
            status: SignalStatus::Active,
            executions: 0,
            successful_executions: 0,
            total_volume: 0,
            total_roi: 0,
            category: crate::categories::SignalCategory::SWING,
            risk_level: crate::categories::RiskLevel::Medium,
            is_collaborative: false,
            tags: soroban_sdk::Vec::new(env),
            submitted_at: env.ledger().timestamp(),
            rationale_hash: String::from_str(env, "Test signal"),
            confidence: 50,
            adoption_count: 0,
            ai_validation_score: None,
        }
    }

    #[test]
    fn test_is_expired() {
        let env = Env::default();

        // Set a known timestamp
        env.ledger().set_timestamp(1000);
        let current_time = env.ledger().timestamp();

        // Signal expires in future
        let future_signal = create_test_signal(&env, 1, current_time + 100);
        assert!(!is_expired(&env, &future_signal));

        // Signal expired in past
        let past_signal = create_test_signal(&env, 2, current_time.saturating_sub(100));
        assert!(is_expired(&env, &past_signal));

        // Signal expires exactly now (considered expired)
        let now_signal = create_test_signal(&env, 3, current_time);
        assert!(!is_expired(&env, &now_signal)); // current_time is NOT > expiry
    }

    #[test]
    fn test_check_and_update_expiry() {
        let env = Env::default();

        // Set a known timestamp
        env.ledger().set_timestamp(1000);
        let current_time = env.ledger().timestamp();

        // Active signal that should expire
        let mut signal = create_test_signal(&env, 1, current_time.saturating_sub(100));
        assert!(check_and_update_expiry(&env, &mut signal));
        assert_eq!(signal.status, SignalStatus::Expired);

        // Already expired signal (no change)
        let mut expired_signal = create_test_signal(&env, 2, current_time.saturating_sub(100));
        expired_signal.status = SignalStatus::Expired;
        assert!(!check_and_update_expiry(&env, &mut expired_signal));

        // Executed signal (no change)
        let mut executed_signal = create_test_signal(&env, 3, current_time.saturating_sub(100));
        executed_signal.status = SignalStatus::Executed;
        assert!(!check_and_update_expiry(&env, &mut executed_signal));
    }

    #[test]
    fn test_get_active_signals() {
        let env = Env::default();

        // Set a known timestamp
        env.ledger().set_timestamp(10000);
        let current_time = env.ledger().timestamp();
        let mut signals = Map::new(&env);

        // Add 3 active signals
        for i in 0..3 {
            let signal = create_test_signal(&env, i, current_time + 1000);
            signals.set(i, signal);
        }

        // Add 2 expired signals
        for i in 3..5 {
            let mut signal = create_test_signal(&env, i, current_time.saturating_sub(1000));
            signal.status = SignalStatus::Expired;
            signals.set(i, signal);
        }

        // Add 1 executed signal
        let mut executed = create_test_signal(&env, 5, current_time + 1000);
        executed.status = SignalStatus::Executed;
        signals.set(5, executed);

        let active = get_active_signals(&env, &signals);
        assert_eq!(active.len(), 3); // Only the 3 active, non-expired signals
    }

    #[test]
    fn test_should_archive() {
        let env = Env::default();

        // Set a known timestamp far in the future to allow subtraction
        let current_time = 100 * 24 * 60 * 60; // 100 days
        env.ledger().set_timestamp(current_time);

        // Signal expired 31 days ago (should archive)
        let mut old_expired =
            create_test_signal(&env, 1, current_time.saturating_sub(31 * 24 * 60 * 60));
        old_expired.status = SignalStatus::Expired;
        assert!(should_archive(&env, &old_expired));

        // Signal expired 29 days ago (not yet)
        let mut recent_expired =
            create_test_signal(&env, 2, current_time.saturating_sub(29 * 24 * 60 * 60));
        recent_expired.status = SignalStatus::Expired;
        assert!(!should_archive(&env, &recent_expired));

        // Active signal (never archive)
        let active = create_test_signal(&env, 3, current_time + 1000);
        assert!(!should_archive(&env, &active));
    }

    #[test]
    fn test_count_expired_signals() {
        let env = Env::default();

        // Set a known timestamp
        env.ledger().set_timestamp(10000);
        let current_time = env.ledger().timestamp();
        let mut signals = Map::new(&env);

        // Add 4 expired signals
        for i in 0..4 {
            let mut signal = create_test_signal(&env, i, current_time.saturating_sub(1000));
            signal.status = SignalStatus::Expired;
            signals.set(i, signal);
        }

        // Add 3 active signals
        for i in 4..7 {
            let signal = create_test_signal(&env, i, current_time + 1000);
            signals.set(i, signal);
        }

        assert_eq!(count_expired_signals(&signals), 4);
    }

    #[test]
    fn test_count_signals_pending_expiry() {
        let env = Env::default();

        // Set a known timestamp
        env.ledger().set_timestamp(10000);
        let current_time = env.ledger().timestamp();
        let mut signals = Map::new(&env);

        // Add 3 signals past expiry but not marked expired yet
        for i in 0..3 {
            let signal = create_test_signal(&env, i, current_time.saturating_sub(1000));
            signals.set(i, signal);
        }

        // Add 2 already marked as expired
        for i in 3..5 {
            let mut signal = create_test_signal(&env, i, current_time.saturating_sub(1000));
            signal.status = SignalStatus::Expired;
            signals.set(i, signal);
        }

        // Add 2 active signals
        for i in 5..7 {
            let signal = create_test_signal(&env, i, current_time + 1000);
            signals.set(i, signal);
        }

        assert_eq!(count_signals_pending_expiry(&env, &signals), 3);
    }
}
