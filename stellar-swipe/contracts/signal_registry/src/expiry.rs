//! Signal expiration and archive behavior (issue #1219).
//!
//! The full specification lives in `docs/signal_expiration.md`. In short:
//!
//! - **Expiry boundary.** A signal is live through its `expiry` second and
//!   expired strictly after it (`now > expiry`, see [`is_expired`]).
//! - **Query visibility is time-derived.** [`is_visible_active`] is the single
//!   predicate every active-signal query uses. It checks the ledger clock, so
//!   an expired signal is hidden whether or not cleanup has run yet.
//! - **Cleanup is bounded and resumable.** [`cleanup_expired_signals`] and
//!   [`archive_old_signals`] each examine at most `MAX_CLEANUP_BATCH_SIZE`
//!   entries per call, resuming from a persisted cursor, and are idempotent.
//!   Cleanup only ever moves a signal *away* from `Active`, so it cannot make
//!   an expired signal visible.

use soroban_sdk::{contracttype, Address, Env, Map, Vec};
use stellar_swipe_common::{SECONDS_PER_30_DAY_MONTH, SECONDS_PER_DAY};

use crate::categories::SignalCategory;
use crate::events::{emit_signal_expired, emit_signals_archived, emit_signals_pruned};
use crate::types::{Signal, SignalStatus};
use crate::StorageKey;

use shared::Expirable;

pub const DEFAULT_EXPIRY_SECONDS: u64 = SECONDS_PER_DAY; // 24 hours
pub const MAX_CLEANUP_BATCH_SIZE: u32 = 100; // Process max 100 signals per cleanup call
pub const ARCHIVE_THRESHOLD_SECONDS: u64 = SECONDS_PER_30_DAY_MONTH; // 30 days

/// Lifecycle of a signal id with respect to expiry, as reported by
/// `get_signal_expiry_state`. Derived on read; never stored.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalExpiryState {
    /// The id was never allocated.
    Unknown,
    /// Returned by active-signal queries.
    Live,
    /// Past expiry but cleanup has not flipped its status yet. Already hidden
    /// from active queries.
    ExpiredPendingCleanup,
    /// Status is `Expired`. Kept in storage until the archive threshold.
    Expired,
    /// Status is `Executed`; hidden from active queries regardless of expiry.
    Closed,
    /// The id was allocated but its record was archived or pruned.
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CleanupResult {
    pub signals_processed: u32,
    pub signals_expired: u32,
    pub expired_signals: Vec<Signal>,
}

impl Expirable for Signal {
    fn expiry_timestamp(&self) -> u64 {
        self.expiry
    }
}

/// Check if a signal has expired based on current ledger time.
///
/// Delegates to the shared [`Expirable`] trait implementation on [`Signal`].
/// Boundary: `current_time > signal.expiry` (exclusive — at the exact
/// expiry second the signal is still considered live).
pub fn is_expired(env: &Env, signal: &Signal) -> bool {
    signal.is_expired(env.ledger().timestamp())
}

/// The one visibility rule for active-signal queries.
///
/// A signal is visible iff its status is not `Expired`/`Executed` **and** it
/// is not past expiry at `now`. Because the time check does not depend on the
/// stored status, a signal whose cleanup has not run yet is still hidden.
pub fn is_visible_active(signal: &Signal, now: u64) -> bool {
    signal.status != SignalStatus::Expired
        && signal.status != SignalStatus::Executed
        && !signal.is_expired(now)
}

/// A signal may be archived (removed from the signals map) once it is expired
/// — either marked `Expired`, or still `Active` but past expiry because
/// cleanup has not run — and more than [`ARCHIVE_THRESHOLD_SECONDS`] have
/// elapsed since its expiry. Other terminal statuses are kept as history.
pub fn is_archivable(signal: &Signal, now: u64) -> bool {
    let expired = match signal.status {
        SignalStatus::Expired => true,
        SignalStatus::Active => signal.is_expired(now),
        _ => false,
    };
    expired && now.saturating_sub(signal.expiry) > ARCHIVE_THRESHOLD_SECONDS
}

/// Check if signal should be archived (expired for more than 30 days)
pub fn should_archive(env: &Env, signal: &Signal) -> bool {
    is_archivable(signal, env.ledger().timestamp())
}

/// Classify `signal_id` without mutating storage.
pub fn expiry_state(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    signal_id: u64,
) -> SignalExpiryState {
    let Some(signal) = signals_map.get(signal_id) else {
        let counter: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::SignalCounter)
            .unwrap_or(0);
        return if signal_id >= 1 && signal_id <= counter {
            SignalExpiryState::Removed
        } else {
            SignalExpiryState::Unknown
        };
    };
    let now = env.ledger().timestamp();
    match signal.status {
        SignalStatus::Expired => SignalExpiryState::Expired,
        SignalStatus::Executed => SignalExpiryState::Closed,
        _ if signal.is_expired(now) => SignalExpiryState::ExpiredPendingCleanup,
        _ => SignalExpiryState::Live,
    }
}

fn clamp_batch(limit: u32) -> u32 {
    if limit == 0 || limit > MAX_CLEANUP_BATCH_SIZE {
        MAX_CLEANUP_BATCH_SIZE
    } else {
        limit
    }
}

/// Last signal id examined by a cursor-driven sweep; 0 before the first run.
pub fn get_cursor(env: &Env, key: &StorageKey) -> u64 {
    env.storage().instance().get(key).unwrap_or(0)
}

/// The ids a sweep examines this call: at most `budget` of `keys` (sorted
/// ascending, as `Map::keys` returns them), starting strictly after `cursor`
/// and wrapping to the lowest id. No id appears twice. The cursor may name an
/// id that has since been removed; the sweep resumes at the next larger id.
fn cursor_window(env: &Env, keys: &Vec<u64>, cursor: u64, budget: u32) -> Vec<u64> {
    let mut window = Vec::new(env);
    let n = keys.len();
    if n == 0 || budget == 0 {
        return window;
    }
    let start = match keys.binary_search(cursor) {
        Ok(i) => i + 1,
        Err(i) => i,
    };
    let take = if budget < n { budget } else { n };
    for k in 0..take {
        window.push_back(keys.get_unchecked((start + k) % n));
    }
    window
}

/// Drop removed signals from the active-signal indexes. Signals still marked
/// `Active` release the provider's active-signal slot and emit
/// `signal_expired`, since cleanup never got to announce them.
fn release_removed_signals(env: &Env, removed: &Vec<Signal>) {
    let mut cat_map: Map<SignalCategory, Vec<u64>> = env
        .storage()
        .instance()
        .get(&StorageKey::ActiveSignalsByCategory)
        .unwrap_or(Map::new(env));
    let mut cat_changed = false;
    for signal in removed.iter() {
        if signal.status == SignalStatus::Active {
            crate::validation::decrement_provider_active_count(env, &signal.provider);
            emit_signal_expired(env, signal.id, signal.provider.clone(), signal.expiry);
        }
        if let Some(list) = cat_map.get(signal.category.clone()) {
            if let Some(pos) = list.first_index_of(signal.id) {
                let mut list = list;
                list.remove(pos);
                cat_map.set(signal.category.clone(), list);
                cat_changed = true;
            }
        }
    }
    if cat_changed {
        env.storage()
            .instance()
            .set(&StorageKey::ActiveSignalsByCategory, &cat_map);
    }
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
            if is_visible_active(&signal, current_time) {
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

/// Mark expired `Active` signals as `Expired`, examining at most
/// `limit` entries (clamped to `1..=MAX_CLEANUP_BATCH_SIZE`; 0 means the max).
///
/// The sweep resumes after the id stored in
/// [`StorageKey::ExpiryCleanupCursor`] and wraps around, so repeated calls
/// visit every signal even when the lowest ids are long-lived. Every examined
/// entry counts toward the budget, whatever its status, so the per-call work
/// is bounded. Re-running over an already-swept range is a no-op, so a
/// failed or repeated call can simply be retried.
///
/// `signals_processed` is the number of entries examined; `signals_expired`
/// the number moved to `Expired` (each emits `signal_expired`).
pub fn cleanup_expired_signals(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    limit: u32,
) -> CleanupResult {
    let current_time = env.ledger().timestamp();
    let mut signals_processed = 0u32;
    let mut signals_expired = 0u32;
    let mut expired_signals = Vec::new(env);
    let mut updated_map = signals_map.clone();

    let window = cursor_window(
        env,
        &signals_map.keys(),
        get_cursor(env, &StorageKey::ExpiryCleanupCursor),
        clamp_batch(limit),
    );

    for signal_id in window.iter() {
        signals_processed += 1;
        if let Some(mut signal) = signals_map.get(signal_id) {
            // Only active signals consume provider capacity and can transition
            // to Expired in this cleanup path.
            if signal.status == SignalStatus::Active && signal.is_expired(current_time) {
                signal.status = SignalStatus::Expired;
                updated_map.set(signal_id, signal.clone());
                signals_expired += 1;
                expired_signals.push_back(signal.clone());

                emit_signal_expired(env, signal.id, signal.provider.clone(), signal.expiry);
            }
        }
    }

    if let Some(last) = window.last() {
        env.storage()
            .instance()
            .set(&StorageKey::ExpiryCleanupCursor, &last);
    }

    // Save updated map if any changes were made
    if signals_expired > 0 {
        env.storage()
            .instance()
            .set(&StorageKey::Signals, &updated_map);
    }

    CleanupResult {
        signals_processed,
        signals_expired,
        expired_signals,
    }
}

/// Permanently remove up to `max_entries` signals whose expiry timestamp has
/// passed (issue #779).
///
/// Unlike [`cleanup_expired_signals`], which only flips signal status, this
/// deletes entries from the signals map to reclaim instance storage. Uses the
/// same expiry boundary as [`is_expired`] (a signal at exactly its expiry
/// second is still live). Emits a single `signals_pruned` event when at least
/// one signal is removed.
///
/// Returns the pruned signals so callers can update secondary indexes.
/// `max_entries == 0` is a no-op and returns an empty Vec.
pub fn prune_expired_signals(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    max_entries: u32,
) -> Vec<Signal> {
    let mut pruned = Vec::new(env);
    if max_entries == 0 {
        return pruned;
    }

    let mut updated_map = signals_map.clone();
    let keys = signals_map.keys();
    for i in 0..keys.len() {
        if pruned.len() >= max_entries {
            break;
        }
        let signal_id = keys.get(i).unwrap();
        if let Some(signal) = signals_map.get(signal_id) {
            if is_expired(env, &signal) {
                updated_map.remove(signal_id);
                pruned.push_back(signal);
            }
        }
    }

    if !pruned.is_empty() {
        env.storage()
            .instance()
            .set(&crate::StorageKey::Signals, &updated_map);
        emit_signals_pruned(env, pruned.len(), env.ledger().sequence());
    }

    pruned
}

/// Count signals whose expiry timestamp has passed, regardless of status —
/// i.e. what [`prune_expired_signals`] would remove given a large enough
/// budget. Surfaced through `health_check` (issue #779).
pub fn count_prunable_signals(env: &Env, signals_map: &Map<u64, Signal>) -> u32 {
    let current_time = env.ledger().timestamp();
    let mut count = 0u32;

    let keys = signals_map.keys();
    for i in 0..keys.len() {
        let key = keys.get(i).unwrap();
        if let Some(signal) = signals_map.get(key) {
            if signal.is_expired(current_time) {
                count += 1;
            }
        }
    }

    count
}

/// Remove signals that satisfy [`is_archivable`] from the signals map,
/// examining at most `limit` entries (same clamping as
/// [`cleanup_expired_signals`]) from the [`StorageKey::ArchiveCursor`]
/// position. Returns the number of signals removed.
///
/// Archived ids are dropped from the category index. Signals archived while
/// still `Active` (cleanup never ran) also release the provider's
/// active-signal slot and emit `signal_expired`. When anything is removed, a
/// single `signals_archived` event is emitted. After archiving,
/// `get_signal_expiry_state` reports the id as `Removed`.
pub fn archive_old_signals(env: &Env, signals_map: &Map<u64, Signal>, limit: u32) -> u32 {
    let current_time = env.ledger().timestamp();
    let mut updated_map = signals_map.clone();
    let mut archived = Vec::new(env);

    let window = cursor_window(
        env,
        &signals_map.keys(),
        get_cursor(env, &StorageKey::ArchiveCursor),
        clamp_batch(limit),
    );

    for signal_id in window.iter() {
        if let Some(signal) = signals_map.get(signal_id) {
            if is_archivable(&signal, current_time) {
                updated_map.remove(signal_id);
                archived.push_back(signal);
            }
        }
    }

    if let Some(last) = window.last() {
        env.storage()
            .instance()
            .set(&StorageKey::ArchiveCursor, &last);
    }

    if !archived.is_empty() {
        env.storage()
            .instance()
            .set(&StorageKey::Signals, &updated_map);
        release_removed_signals(env, &archived);
        emit_signals_archived(env, archived.len(), current_time);
    }

    archived.len()
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
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
            warning_emitted: false,
            benchmark_return_bps: None,
            alpha_bps: None,
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
    fn test_prune_expired_signals() {
        let env = Env::default();
        env.ledger().set_timestamp(10000);
        let current_time = env.ledger().timestamp();

        #[allow(deprecated)]
        let contract_id = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&contract_id, || {
            let mut signals = Map::new(&env);

            // 3 expired, 2 active
            for i in 0..3 {
                signals.set(i, create_test_signal(&env, i, current_time - 1000));
            }
            for i in 3..5 {
                signals.set(i, create_test_signal(&env, i, current_time + 1000));
            }

            // max_entries = 0 is a no-op
            assert_eq!(prune_expired_signals(&env, &signals, 0).len(), 0);

            // Partial prune: budget smaller than expired count
            assert_eq!(prune_expired_signals(&env, &signals, 2).len(), 2);

            // Full prune: budget larger than expired count
            assert_eq!(prune_expired_signals(&env, &signals, 100).len(), 3);

            // A signal at exactly its expiry second is still live — not pruned
            let mut boundary = Map::new(&env);
            boundary.set(1, create_test_signal(&env, 1, current_time));
            assert_eq!(prune_expired_signals(&env, &boundary, 100).len(), 0);
        });
    }

    #[test]
    fn test_count_prunable_signals() {
        let env = Env::default();
        env.ledger().set_timestamp(10000);
        let current_time = env.ledger().timestamp();
        let mut signals = Map::new(&env);

        // 2 past expiry (one already marked Expired, one still Active)
        signals.set(0, create_test_signal(&env, 0, current_time - 1000));
        let mut marked = create_test_signal(&env, 1, current_time - 1000);
        marked.status = SignalStatus::Expired;
        signals.set(1, marked);

        // 1 active in the future
        signals.set(2, create_test_signal(&env, 2, current_time + 1000));

        assert_eq!(count_prunable_signals(&env, &signals), 2);
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
