//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Vec};

/// Errors returned by the signal registry contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SignalError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidDecayConfig = 4,
    InvalidReputationUpdate = 5,
    AuthTreeTooDeep = 6,
}

/// Maximum depth of an authorization tree accepted by tree-consuming
/// entrypoints. Trees deeper than this are rejected before any recursive
/// traversal begins, bounding execution resources and keeping validation
/// behavior consistent across entrypoints.
///
/// The limit is inclusive: a tree whose depth equals `MAX_AUTH_TREE_DEPTH` is
/// accepted, while a tree one level deeper returns `AuthTreeTooDeep`.
pub const MAX_AUTH_TREE_DEPTH: u32 = 8;

/// Configuration-driven reputation decay schedule.
///
/// `decay_rate_bps` is the number of basis points (1/100th of a percent) of
/// reputation lost per elapsed `decay_interval` after the `grace_period`.
/// `min_reputation` and `max_reputation` bound the resulting score.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecaySchedule {
    pub decay_rate_bps: u32,
    pub decay_interval: u64,
    pub grace_period: u64,
    pub min_reputation: i128,
    pub max_reputation: i128,
}

/// Stored reputation record for a provider.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationRecord {
    pub score: i128,
    pub last_updated: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataKey {
    pub admin: Address,
    pub schedule: DecaySchedule,
}

/// The kind of contract operation whose storage footprint is being estimated.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OperationKind {
    /// A brand new reputation record is written for a provider.
    NewRecord = 0,
    /// An existing reputation record is overwritten in place.
    UpdateRecord = 1,
}

/// Projected storage footprint and rent implications for a contract
/// operation, produced before submission.
///
/// `projected_entries` is the number of persistent storage entries the
/// operation will touch, `projected_bytes` is the serialized byte footprint
/// of those entries, and `ttl_impact` is the number of ledgers the operation
/// extends the relevant entry's time-to-live by. All fields are derived
/// purely from the inputs, so identical inputs always yield identical
/// estimates.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StorageRentEstimate {
    pub projected_entries: u32,
    pub projected_bytes: u32,
    pub ttl_impact: u32,
}

/// A versioned contract event carrying a monotonically increasing sequence
/// number.
///
/// `sequence` is assigned from a per-contract counter that is persisted in
/// instance storage, so it survives contract upgrades and never resets. The
/// counter is scoped to this contract instance, giving indexers a single
/// strictly increasing stream they can use to detect gaps, duplicates, and
/// out-of-order delivery. `version` identifies the event schema so consumers
/// can evolve their decoding logic independently of the sequence stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionedEvent {
    pub version: u32,
    pub sequence: u64,
    pub kind: EventKind,
    pub provider: Address,
    pub value: i128,
}

/// The kind of state transition a `VersionedEvent` describes.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    ReputationUpdated = 0,
    ScheduleUpdated = 1,
}

/// Current schema version emitted on every `VersionedEvent`.
pub const EVENT_VERSION: u32 = 1;

/// Serialized byte footprint of a single reputation record: a 16-byte `i128`
/// score plus an 8-byte `u64` timestamp.
const REPUTATION_RECORD_BYTES: u32 = 24;

/// Ledgers of TTL extension applied per storage write. Matches the default
/// persistent entry lifetime used by the registry.
const TTL_LEDGERS_PER_WRITE: u32 = 100;

/// Upper bound on the number of entries a single operation may project.
/// Requests that would exceed this are rejected as near-limit requests.
pub const MAX_PROJECTED_ENTRIES: u32 = 64;

const ADMIN_KEY: &str = "admin";
const SCHEDULE_KEY: &str = "schedule";
const REPUTATION_KEY: &str = "reputation";
const SEQUENCE_KEY: &str = "sequence";

#[contract]
pub struct SignalRegistry;

#[contractimpl]
impl SignalRegistry {
    /// Initialize the contract with an admin and a decay schedule.
    pub fn initialize(env: Env, admin: Address, schedule: DecaySchedule) -> Result<(), SignalError> {
        if env.storage().instance().has(&ADMIN_KEY) {
            return Err(SignalError::AlreadyInitialized);
        }
        Self::validate_schedule(&schedule)?;
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&SCHEDULE_KEY, &schedule);
        // Seed the event sequence counter so the first emitted event is 1.
        env.storage().instance().set(&SEQUENCE_KEY, &0u64);
        Ok(())
    }

    /// Update the decay schedule. Only the admin may call this.
    pub fn set_decay_schedule(env: Env, schedule: DecaySchedule) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        Self::validate_schedule(&schedule)?;
        env.storage().instance().set(&SCHEDULE_KEY, &schedule);
        Self::emit_event(&env, EventKind::ScheduleUpdated, admin, 0);
        Ok(())
    }

    /// Read the currently configured decay schedule.
    pub fn get_decay_schedule(env: Env) -> Result<DecaySchedule, SignalError> {
        env.storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Read the next sequence number that will be assigned to an emitted
    /// event. Exposed for indexers and tests to verify continuity.
    pub fn next_sequence(env: Env) -> u64 {
        Self::load_sequence(&env).saturating_add(1)
    }

    /// Estimate the storage footprint and rent implications of a contract
    /// operation before it is submitted.
    ///
    /// `kind` selects the operation being projected and `record_count` is the
    /// number of reputation records the operation will touch. The estimate is
    /// deterministic: it depends only on `kind` and `record_count`, so
    /// identical inputs always produce identical results. Requests that would
    /// project more than `MAX_PROJECTED_ENTRIES` entries are rejected.
    pub fn estimate_storage_rent(
        kind: OperationKind,
        record_count: u32,
    ) -> Result<StorageRentEstimate, SignalError> {
        if record_count == 0 || record_count > MAX_PROJECTED_ENTRIES {
            return Err(SignalError::InvalidReputationUpdate);
        }

        // New records allocate a fresh entry; updates overwrite an existing
        // one, so both touch exactly one entry per record.
        let projected_entries = record_count;
        let projected_bytes = record_count.saturating_mul(REPUTATION_RECORD_BYTES);
        let ttl_impact = match kind {
            OperationKind::NewRecord => record_count.saturating_mul(TTL_LEDGERS_PER_WRITE),
            OperationKind::UpdateRecord => record_count.saturating_mul(TTL_LEDGERS_PER_WRITE),
        };

        Ok(StorageRentEstimate {
            projected_entries,
            projected_bytes,
            ttl_impact,
        })
    }

    /// Apply a reputation update for a provider, first decaying the stored
    /// score according to the configured schedule.
    ///
    /// `auth_depth` is the depth of the caller's authorization tree. It is
    /// validated against `MAX_AUTH_TREE_DEPTH` before any state is read or
    /// mutated, so over-limit trees fail fast with a stable error.
    pub fn update_reputation(
        env: Env,
        provider: Address,
        delta: i128,
        auth_depth: u32,
    ) -> Result<i128, SignalError> {
        Self::check_auth_depth(auth_depth)?;

        let schedule: DecaySchedule = env
            .storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)?;

        let now = env.ledger().timestamp();
        let record = Self::load_reputation(&env, &provider);

        // Decay the existing score based on elapsed time since last update.
        let decayed = Self::apply_decay(&record, now, &schedule);

        // Apply the new delta and clamp to configured thresholds.
        let updated = Self::clamp(decayed.saturating_add(delta), &schedule);

        let new_record = ReputationRecord {
            score: updated,
            last_updated: now,
        };
        Self::store_reputation(&env, &provider, &new_record);
        Self::emit_event(&env, EventKind::ReputationUpdated, provider, updated);
        Ok(updated)
    }

    /// Read the current (decayed) reputation for a provider without mutating
    /// state. Useful for off-chain verification and contract tests.
    ///
    /// `auth_depth` is validated against `MAX_AUTH_TREE_DEPTH` before

/* … truncated 6176 chars — edit only what you need near the top … */
