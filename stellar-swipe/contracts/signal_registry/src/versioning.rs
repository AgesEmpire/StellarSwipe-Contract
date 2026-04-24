use crate::errors::VersioningError;
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
