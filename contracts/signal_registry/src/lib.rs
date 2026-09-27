//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.
//!
//! Provider metadata is validated before any persistent write. Metadata must
//! be non-empty, valid UTF-8, and at most [`MAX_METADATA_LEN`] bytes long.
//!
//! Removed signal_registry records are tombstoned rather than erased so that
//! their identifiers cannot be reused during the documented retention period.
//! Expired tombstones may be purged by the admin in bounded, idempotent
//! batches once the retention window has elapsed.
//!
//! Material contract configuration changes emit structured, auditable events
//! so that indexers and operators can discover them. Each event records the
//! changed setting, its prior and new values, and the actor that authorized
//! the change.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Symbol, Vec};

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
    EmptyMetadata = 6,
    MetadataTooLong = 7,
    InvalidMetadataEncoding = 8,
    IdentifierTombstoned = 9,
    TombstoneNotExpired = 10,
    InvalidBatchSize = 11,
}

/// Maximum accepted length, in bytes, of provider metadata.
///
/// Metadata is stored and indexed on-chain, so an explicit upper bound keeps
/// storage and indexing costs predictable. Values longer than this are
/// rejected before any persistent write occurs.
pub const MAX_METADATA_LEN: u32 = 256;

/// Documented retention period, in seconds, during which a removed
/// signal_registry identifier remains non-reusable.
pub const TOMBSTONE_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Upper bound on the number of expired tombstones purged per cleanup call.
pub const MAX_TOMBSTONE_PURGE: u32 = 100;

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

/// Lifecycle state of a signal_registry identifier.
///
/// `Active` records are live, `Tombstoned` records have been removed but
/// remain non-reusable until `expires_at`, and `Unknown` identifiers have
/// never been registered (or their tombstone has been purged).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordState {
    Active,
    Tombstoned,
    Unknown,
}

/// Tombstone marker stored for a removed identifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tombstone {
    pub removed_at: u64,
    pub expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataKey {
    pub admin: Address,
    pub schedule: DecaySchedule,
}

/// Auditable event emitted when a material contract configuration setting
/// changes. Records the changed setting, its prior and new values, and the
/// actor that authorized the change so indexers and operators can discover
/// configuration drift.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigChanged {
    pub setting: Symbol,
    pub old_value: DecaySchedule,
    pub new_value: DecaySchedule,
    pub actor: Address,
}

const ADMIN_KEY: &str = "admin";
const SCHEDULE_KEY: &str = "schedule";
const REPUTATION_KEY: &str = "reputation";
const METADATA_KEY: &str = "metadata";
const TOMBSTONE_KEY: &str = "tombstone";

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
        Ok(())
    }

    /// Update the decay schedule. Only the admin may call this.
    ///
    /// On success an auditable [`ConfigChanged`] event is emitted carrying the
    /// prior and new schedules along with the authorizing admin. Rejected
    /// updates (unauthorized or invalid) emit no event.
    pub fn set_decay_schedule(env: Env, schedule: DecaySchedule) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        Self::validate_schedule(&schedule)?;
        let old_value: DecaySchedule = env
            .storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)?;
        env.storage().instance().set(&SCHEDULE_KEY, &schedule);
        env.events().publish(
            (Symbol::new(&env, "config_changed"), Symbol::new(&env, "schedule")),
            ConfigChanged {
                setting: Symbol::new(&env, "schedule"),
                old_value,
                new_value: schedule,
                actor: admin,
            },
        );
        Ok(())
    }

    /// Read the currently configured decay schedule.
    pub fn get_decay_schedule(env: Env) -> Result<DecaySchedule, SignalError> {
        env.storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Store provider metadata after validating its length and encoding.
    ///
    /// Validation runs before any persistent write, so empty, over-limit, or
    /// malformed values never reach storage. Identifiers that are currently
    /// tombstoned cannot be reused until their retention period elapses.
    pub fn set_provider_metadata(
        env: Env,
        provider: Address,
        metadata: String,
    ) -> Result<(), SignalError> {
        provider.require_auth();
        Self::validate_metadata(&metadata)?;
        if Self::record_state(&env, &provider) == RecordState::Tombstoned {
            return Err(SignalError::IdentifierTombstoned);
        }
        let key = (METADATA_KEY, provider);
        env.storage().persistent().set(&key, &metadata);
        Ok(())
    }

    /// Read the stored metadata for a provider, if any.
    pub fn get_provider_metadata(env: Env, provider: Address) -> Option<String> {
        let key = (METADATA_KEY, provider);
        env.storage().persistent().get(&key)
    }

    /// Remove a provider's metadata, leaving a tombstone that keeps the
    /// identifier non-reusable for [`TOMBSTONE_RETENTION_SECONDS`].
    ///
    /// Only the provider may remove its own record. Re-removing an already
    /// tombstoned identifier is idempotent and preserves the original
    /// retention window.
    pub fn remove_provider_metadata(env: Env, provider: Address) -> Result<(), SignalError> {
        provider.require_auth();
        let key = (METADATA_KEY, provider.clone());
        env.storage().persistent().remove(&key);
        let tomb_key = (TOMBSTONE_KEY, provider);
        if !env.storage().persistent().has(&tomb_key) {
            let now = env.ledger().timestamp();
            let tombstone = Tombstone {
                removed_at: now,
                expires_at: now.saturating_add(TOMBSTONE_RETENTION_SECONDS),
            };
            env.storage().persistent().set(&tomb_key, &tombstone);
        }
        Ok(())
    }

    /// Report the lifecycle state of an identifier: `Active`, `Tombstoned`,
    /// or `Unknown`.
    pub fn get_record_state(env: Env, provider: Address) -> RecordState {
        Self::record_state(&env, &provider)
    }

    /// Purge expired tombstones in a bounded, idempotent batch.
    ///
    /// Only the admin may call this. At most `max_entries` tombstones are
    /// examined per call, and only those whose retention window has elapsed
    /// are removed. Re-running is safe: already-purged tombstones are simply
    /// skipped.
    pub fn purge_expired_tombstones(
        env: Env,
        providers: Vec<Address>,
        max_entries: u32,
    ) -> Result<u32, SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        if max_entries == 0 || max_entries > MAX_TOMBSTONE_PURGE {
            return Err(SignalError::InvalidBatchSize);
        }
        let now = env.ledger().timestamp();
        let mut purged: u32 = 0;
        for provider in providers

/* … truncated 6101 chars — edit only what you need near the top … */
