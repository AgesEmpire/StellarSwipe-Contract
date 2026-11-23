use crate::errors::{SignalEditError, VersioningError};
use crate::events;
use crate::types::{Signal, SignalStatus};
use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};
use stellar_swipe_common::SECONDS_PER_HOUR;

const MAX_UPDATES_PER_SIGNAL: u32 = 5;
const UPDATE_COOLDOWN_SECONDS: u64 = SECONDS_PER_HOUR; // 1 hour

#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalVersion {
    pub version: u32,
    pub signal_id: u64,
    pub price: i128,
    pub rationale: String,
    pub expiry: u64,
    pub updated_at: u64,
    pub updated_by: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CopyRecord {
    pub user: Address,
    pub signal_id: u64,
    pub version: u32,
    pub copied_at: u64,
    pub notified_of_updates: Vec<u32>,
}

#[contracttype]
#[derive(Clone)]
pub enum VersioningStorageKey {
    SignalVersions(u64, u32), // (signal_id, version)
    LatestVersion(u64),
    UpdateCount(u64),
    LastUpdateTime(u64),
    CopyRecords(Address, u64), // (user, signal_id)
}

pub fn update_signal(
    env: &Env,
    signal_id: u64,
    updater: &Address,
    new_price: Option<i128>,
    new_rationale: Option<String>,
    new_expiry: Option<u64>,
    signal: &mut Signal,
) -> Result<u32, VersioningError> {
    // Verify ownership
    if signal.provider != *updater {
        return Err(VersioningError::NotSignalOwner);
    }

    // Check signal is active
    if signal.status != SignalStatus::Active {
        return Err(VersioningError::CannotUpdateInactive);
    }

    // Check update count
    let update_count_key = VersioningStorageKey::UpdateCount(signal_id);
    let update_count: u32 = env
        .storage()
        .persistent()
        .get(&update_count_key)
        .unwrap_or(0);
    if update_count >= MAX_UPDATES_PER_SIGNAL {
        return Err(VersioningError::MaxUpdatesReached);
    }

    // Check cooldown
    let last_update_key = VersioningStorageKey::LastUpdateTime(signal_id);
    let last_update: u64 = env
        .storage()
        .persistent()
        .get(&last_update_key)
        .unwrap_or(0);
    let current_time = env.ledger().timestamp();
    if current_time < last_update + UPDATE_COOLDOWN_SECONDS {
        return Err(VersioningError::UpdateCooldown);
    }

    // Check expiry
    if current_time >= signal.expiry {
        return Err(VersioningError::SignalExpired);
    }

    // Get current version
    let version_key = VersioningStorageKey::LatestVersion(signal_id);
    let current_version: u32 = env.storage().persistent().get(&version_key).unwrap_or(1);
    let new_version = current_version + 1;

    // Store current state as version
    let version_record = SignalVersion {
        version: current_version,
        signal_id,
        price: signal.price,
        rationale: signal.rationale.clone(),
        expiry: signal.expiry,
        updated_at: current_time,
        updated_by: updater.clone(),
    };

    let version_storage_key = VersioningStorageKey::SignalVersions(signal_id, current_version);
    env.storage()
        .persistent()
        .set(&version_storage_key, &version_record);

    // Apply updates
    if let Some(price) = new_price {
        if price <= 0 {
            return Err(VersioningError::InvalidPrice);
        }
        signal.price = price;
    }

    if let Some(rationale) = new_rationale {
        signal.rationale = rationale;
    }

    if let Some(expiry) = new_expiry {
        if expiry <= current_time {
            return Err(VersioningError::InvalidExpiry);
        }
        signal.expiry = expiry;
    }

    // Update metadata
    env.storage().persistent().set(&version_key, &new_version);
    env.storage()
        .persistent()
        .set(&update_count_key, &(update_count + 1));
    env.storage()
        .persistent()
        .set(&last_update_key, &current_time);

    // Emit event
    events::emit_signal_updated(env, signal_id, new_version, updater.clone());

    Ok(new_version)
}

/// Edit a signal with a full audit trail (Issue #686).
///
/// Stores the current signal state as a versioned snapshot before applying
/// edits. The edit is only permitted if no follower has copy-traded the
/// signal yet (`adoption_count == 0`).
///
/// Unlike `update_signal`, this entrypoint uses a simplified audit model
/// without cooldown or max-update limits, and returns the new version number.
pub fn edit_signal_with_audit(
    env: &Env,
    signal_id: u64,
    updater: &Address,
    new_price: Option<i128>,
    new_rationale: Option<String>,
    new_expiry: Option<u64>,
    signal: &mut Signal,
) -> Result<u32, SignalEditError> {
    // Verify ownership
    if signal.provider != *updater {
        return Err(SignalEditError::NotSignalOwner);
    }

    // Reject if any follower has already copy-traded
    if signal.adoption_count > 0 {
        return Err(SignalEditError::SignalAlreadyCopied);
    }

    let current_time = env.ledger().timestamp();

    // Check expiry
    if current_time >= signal.expiry {
        return Err(SignalEditError::EditWindowClosed);
    }

    // Get current version
    let version_key = VersioningStorageKey::LatestVersion(signal_id);
    let current_version: u32 = env.storage().persistent().get(&version_key).unwrap_or(1);
    let new_version = current_version + 1;

    // Store current state as a versioned snapshot (append-only audit trail)
    let version_record = SignalVersion {
        version: current_version,
        signal_id,
        price: signal.price,
        rationale: signal.rationale.clone(),
        expiry: signal.expiry,
        updated_at: current_time,
        updated_by: updater.clone(),
    };

    let version_storage_key = VersioningStorageKey::SignalVersions(signal_id, current_version);
    env.storage()
        .persistent()
        .set(&version_storage_key, &version_record);

    // Apply updates
    if let Some(price) = new_price {
        if price <= 0 {
            return Err(SignalEditError::FieldNotEditable);
        }
        signal.price = price;
    }

    if let Some(rationale) = new_rationale {
        let blen = rationale.len();
        if blen == 0 || blen > 500 {
            return Err(SignalEditError::FieldNotEditable);
        }
        signal.rationale = rationale;
    }

    if let Some(expiry) = new_expiry {
        if expiry <= current_time {
            return Err(SignalEditError::EditWindowClosed);
        }
        signal.expiry = expiry;
    }

    // Update version metadata
    env.storage().persistent().set(&version_key, &new_version);

    // Emit event
    events::emit_signal_updated(env, signal_id, new_version, updater.clone());

    Ok(new_version)
}

pub fn get_signal_history(env: &Env, signal_id: u64) -> Vec<SignalVersion> {
    let version_key = VersioningStorageKey::LatestVersion(signal_id);
    let latest_version: u32 = env.storage().persistent().get(&version_key).unwrap_or(1);

    let mut history = Vec::new(env);
    for v in 1..=latest_version {
        let version_storage_key = VersioningStorageKey::SignalVersions(signal_id, v);
        if let Some(version) = env.storage().persistent().get(&version_storage_key) {
            history.push_back(version);
        }
    }

    history
}

pub fn record_copy(env: &Env, user: &Address, signal_id: u64, version: u32) {
    let copy_key = VersioningStorageKey::CopyRecords(user.clone(), signal_id);
    let copy_record = CopyRecord {
        user: user.clone(),
        signal_id,
        version,
        copied_at: env.ledger().timestamp(),
        notified_of_updates: Vec::new(env),
    };
    env.storage().persistent().set(&copy_key, &copy_record);

    // Emit event
    events::emit_copy_recorded(env, user.clone(), signal_id, version);
}

pub fn get_copy_record(env: &Env, user: &Address, signal_id: u64) -> Option<CopyRecord> {
    let copy_key = VersioningStorageKey::CopyRecords(user.clone(), signal_id);
    env.storage().persistent().get(&copy_key)
}

pub fn get_pending_updates(env: &Env, user: &Address, signal_id: u64) -> Vec<u32> {
    let copy_record = match get_copy_record(env, user, signal_id) {
        Some(record) => record,
        None => return Vec::new(env),
    };

    let version_key = VersioningStorageKey::LatestVersion(signal_id);
    let latest_version: u32 = env.storage().persistent().get(&version_key).unwrap_or(1);

    let mut pending = Vec::new(env);
    for v in (copy_record.version + 1)..=latest_version {
        if !copy_record.notified_of_updates.contains(&v) {
            pending.push_back(v);
        }
    }

    pending
}

pub fn mark_notified(env: &Env, user: &Address, signal_id: u64, version: u32) {
    let copy_key = VersioningStorageKey::CopyRecords(user.clone(), signal_id);
    if let Some(mut record) = env.storage().persistent().get::<_, CopyRecord>(&copy_key) {
        if !record.notified_of_updates.contains(&version) {
            record.notified_of_updates.push_back(version);
            env.storage().persistent().set(&copy_key, &record);
        }
    }
}

pub fn get_latest_version(env: &Env, signal_id: u64) -> u32 {
    let version_key = VersioningStorageKey::LatestVersion(signal_id);
    env.storage().persistent().get(&version_key).unwrap_or(1)
}

pub fn get_update_count(env: &Env, signal_id: u64) -> u32 {
    let update_count_key = VersioningStorageKey::UpdateCount(signal_id);
    env.storage()
        .persistent()
        .get(&update_count_key)
        .unwrap_or(0)
}

// ── Issue #1419: Immutable amendment history ──────────────────────────────────

/// Return the `SignalVersion` record for `revision` of `signal_id`.
///
/// Revision numbers are monotonic: revision 1 is the original state,
/// revision N is the state before the N-th amendment was applied.
/// Returns `None` for a revision that does not exist.
pub fn get_revision(env: &Env, signal_id: u64, revision: u32) -> Option<SignalVersion> {
    if revision == 0 {
        return None;
    }
    let version_storage_key = VersioningStorageKey::SignalVersions(signal_id, revision);
    env.storage().persistent().get(&version_storage_key)
}

/// Return the latest stored `SignalVersion` snapshot for `signal_id`.
///
/// This is the state of the signal before its most recent amendment.
/// Returns `None` if the signal has never been amended.
pub fn get_current_revision(env: &Env, signal_id: u64) -> Option<SignalVersion> {
    let latest = get_latest_version(env, signal_id);
    if latest <= 1 {
        // No amendment has been stored yet (latest == 1 means only the
        // initial version exists and no snapshot has been written).
        return env
            .storage()
            .persistent()
            .get(&VersioningStorageKey::SignalVersions(signal_id, 1));
    }
    // The snapshot stored at `latest - 1` is the most recently captured state.
    get_revision(env, signal_id, latest.saturating_sub(1))
}

/// Return a prior revision within `max_lookback` steps of the current revision.
///
/// `revision` is clamped to `[1, latest_version]` and further bounded so
/// it is at most `max_lookback` steps behind the current latest version.
/// Returns `None` when no snapshot exists for the clamped revision.
pub fn get_prior_revision(
    env: &Env,
    signal_id: u64,
    revision: u32,
    max_lookback: u32,
) -> Option<SignalVersion> {
    let latest = get_latest_version(env, signal_id);
    if latest == 0 || revision == 0 {
        return None;
    }
    // Enforce bound: requested revision must not be more than max_lookback
    // versions behind the current latest.
    let floor = latest.saturating_sub(max_lookback);
    let clamped = revision.clamp(floor.max(1), latest);
    get_revision(env, signal_id, clamped)
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::types::{Signal, SignalAction, SignalStatus};
    use crate::categories::{RiskLevel, SignalCategory};
    use soroban_sdk::{testutils::Address as _, Env, String, Vec};

    fn make_signal(env: &Env, provider: &soroban_sdk::Address) -> Signal {
        Signal {
            id: 1,
            provider: provider.clone(),
            asset_pair: String::from_str(env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100,
            rationale: String::from_str(env, "original"),
            timestamp: 0,
            expiry: 9_999_999,
            status: SignalStatus::Active,
            executions: 0,
            successful_executions: 0,
            total_volume: 0,
            total_roi: 0,
            category: SignalCategory::SWING,
            tags: Vec::new(env),
            risk_level: RiskLevel::Low,
            is_collaborative: false,
            submitted_at: 0,
            rationale_hash: String::from_str(env, "hash"),
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
    fn test_revision_monotonically_increases() {
        let env = Env::default();
        let provider = soroban_sdk::Address::generate(&env);
        let mut signal = make_signal(&env, &provider);

        // First amendment.
        let v1 = update_signal(
            &env,
            1,
            &provider,
            Some(200),
            None,
            None,
            &mut signal,
        )
        .unwrap();

        // Second amendment.
        env.ledger().with_mut(|li| li.timestamp += UPDATE_COOLDOWN_SECONDS + 1);
        let v2 = update_signal(
            &env,
            1,
            &provider,
            Some(300),
            None,
            None,
            &mut signal,
        )
        .unwrap();

        assert!(v2 > v1, "revision must be monotonically increasing");
    }

    #[test]
    fn test_history_unchanged_after_later_edits() {
        let env = Env::default();
        let provider = soroban_sdk::Address::generate(&env);
        let mut signal = make_signal(&env, &provider);

        // First amendment — snapshot of original price (100) is stored.
        update_signal(&env, 1, &provider, Some(200), None, None, &mut signal).unwrap();
        let snap_after_first = get_revision(&env, 1, 1).unwrap();
        let price_in_first_snap = snap_after_first.price;

        // Second amendment.
        env.ledger().with_mut(|li| li.timestamp += UPDATE_COOLDOWN_SECONDS + 1);
        update_signal(&env, 1, &provider, Some(300), None, None, &mut signal).unwrap();

        // The first snapshot must be unchanged.
        let snap_after_second = get_revision(&env, 1, 1).unwrap();
        assert_eq!(
            snap_after_second.price, price_in_first_snap,
            "revision 1 must not change after a later amendment"
        );
    }

    #[test]
    fn test_get_current_revision_returns_latest_snapshot() {
        let env = Env::default();
        let provider = soroban_sdk::Address::generate(&env);
        let mut signal = make_signal(&env, &provider);

        // No amendments yet.
        assert!(get_current_revision(&env, 1).is_none());

        update_signal(&env, 1, &provider, Some(500), None, None, &mut signal).unwrap();
        env.ledger().with_mut(|li| li.timestamp += UPDATE_COOLDOWN_SECONDS + 1);
        update_signal(&env, 1, &provider, Some(600), None, None, &mut signal).unwrap();

        // Latest version is now 3; snapshot at version 2 is the most recent.
        let current = get_current_revision(&env, 1).unwrap();
        // The snapshot stored at version 2 captured the signal at price 500.
        assert_eq!(current.price, 500);
    }

    #[test]
    fn test_get_prior_revision_bounded_lookup() {
        let env = Env::default();
        let provider = soroban_sdk::Address::generate(&env);
        let mut signal = make_signal(&env, &provider);

        // Create 3 amendments.
        for new_price in [200i128, 300, 400] {
            env.ledger().with_mut(|li| li.timestamp += UPDATE_COOLDOWN_SECONDS + 1);
            update_signal(&env, 1, &provider, Some(new_price), None, None, &mut signal).unwrap();
        }
        // Latest version == 4; snapshots at 1, 2, 3.

        // Requesting revision 1 with max_lookback=2 from latest(4):
        // floor = 4 - 2 = 2; revision 1 is clamped to 2.
        let r = get_prior_revision(&env, 1, 1, 2).unwrap();
        assert_eq!(r.version, 2);

        // Requesting revision 3 (within max_lookback=5) from latest(4):
        // floor = max(1, 4-5) = 1; revision 3 passes through unchanged.
        let r3 = get_prior_revision(&env, 1, 3, 5).unwrap();
        assert_eq!(r3.version, 3);
    }
}
