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
//!
//! Privileged operations that may be retried after an ambiguous transaction
//! submission outcome accept an idempotency key. A repeated key never applies
//! the operation twice: the first successful execution records the key and
//! subsequent calls with the same key are rejected with
//! [`SignalError::IdempotencyKeyReused`].
//!
//! ## Idempotency key semantics
//!
//! * **Scope** — a key is scoped to the privileged operation that consumed it
//!   and to the caller that submitted it. The same key value may therefore be
//!   used independently by different callers or for different operations
//!   without colliding.
//! * **Retention** — a consumed key is retained for
//!   [`IDEMPOTENCY_KEY_RETENTION_SECONDS`] from the ledger timestamp at which
//!   it was consumed. After that window elapses the key may be reused.
//! * **Collision behavior** — presenting a key that is still within its
//!   retention window fails closed with [`SignalError::IdempotencyKeyReused`]
//!   and leaves state untouched. A key is only recorded once the operation it
//!   guards has completed successfully, so a failed execution does not consume
//!   the key and the operation may be retried with the same key.
//!
//! ## Two-step administrator handoff
//!
//! Administrator replacement is a two-step handoff. The current administrator
//! nominates a successor with [`SignalRegistry::propose_admin`], which records
//! the pending administrator without changing the active one. Only the
//! nominated address may then call [`SignalRegistry::accept_admin`] to complete
//! the handoff and become the administrator. Unrelated callers can neither
//! nominate nor accept, and nomination alone never transfers the role.

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
    IdempotencyKeyReused = 12,
    NoPendingAdmin = 13,
    NotPendingAdmin = 14,
    /// Issue #1256: a compare-and-set metadata update was attempted with an
    /// expected revision that does not match the stored revision.
    MetadataRevisionConflict = 15,
    /// Issue #1256: the monotonic metadata revision counter cannot advance
    /// further without overflowing.
    MetadataRevisionOverflow = 16,
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

/// Documented retention period, in seconds, during which a consumed
/// idempotency key remains non-reusable.
pub const IDEMPOTENCY_KEY_RETENTION_SECONDS: u64 = 24 * 60 * 60;

/// Issue #1255: upper bound on the number of idempotency key references
/// examined (and at most removed) by a single reclaim call.
pub const MAX_IDEMPOTENCY_PURGE: u32 = 100;

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

/// Recorded consumption of an idempotency key.
///
/// `operation` scopes the key to the privileged operation that consumed it and
/// `expires_at` bounds how long the key remains non-reusable.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyRecord {
    pub operation: String,
    pub expires_at: u64,
}

/// Reference to a stored idempotency record, used as the input of a bounded
/// reclaim call (issue #1255). The `caller` and `key` identify the record
/// exactly as it was consumed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyKeyRef {
    pub caller: Address,
    pub key: String,
}

/// Versioned provider metadata read (issue #1256). `revision` is monotonic and
/// lets clients detect stale reads; `metadata` is the current payload.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionedMetadata {
    pub revision: u32,
    pub metadata: String,
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
const PENDING_ADMIN_KEY: &str = "pending_admin";
const SCHEDULE_KEY: &str = "schedule";
const REPUTATION_KEY: &str = "reputation";
const METADATA_KEY: &str = "metadata";
const TOMBSTONE_KEY: &str = "tombstone";
const IDEMPOTENCY_KEY: &str = "idempotency";
/// Issue #1256: storage key prefix for the monotonic metadata revision counter.
const METADATA_REVISION_KEY: &str = "metadata_revision";

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

    /// Nominate a successor administrator. Only the current administrator may
    /// call this.
    ///
    /// The nomination is recorded as the pending administrator and does **not**
    /// change the active administrator. The handoff only completes when the
    /// nominated address calls [`Self::accept_admin`].
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        env.storage().instance().set(&PENDING_ADMIN_KEY, &new_admin);
        Ok(())
    }

    /// Accept a pending administrator nomination, completing the handoff.
    ///
    /// Only the address previously nominated via [`Self::propose_admin`] may
    /// call this. On success the caller becomes the administrator and the
    /// pending nomination is cleared.
    pub fn accept_admin(env: Env) -> Result<(), SignalError> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&PENDING_ADMIN_KEY)
            .ok_or(SignalError::NoPendingAdmin)?;
        pending.require_auth();
        env.storage().instance().set(&ADMIN_KEY, &pending);
        env.storage().instance().remove(&PENDING_ADMIN_KEY);
        Ok(())
    }

    /// Read the currently pending administrator nomination, if any.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&PENDING_ADMIN_KEY)
    }

    /// Update the decay schedule. Only the admin may call this.
    ///
    /// `idempotency_key` makes the call replay-safe: a key that is still within
    /// its retention window is rejected with [`SignalError::IdempotencyKeyReused`]
    /// and the schedule is left unchanged.
    pub fn set_decay_schedule(
        env: Env,
        schedule: DecaySchedule,
        idempotency_key: String,
    ) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        Self::consume_idempotency_key(&env, &admin, "set_decay_schedule", &idempotency_key)?;
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

    /// Issue #1255: reclaim expired idempotency records in a bounded batch.
    ///
    /// Only the admin may call this. At most [`MAX_IDEMPOTENCY_PURGE`]
    /// references are examined per call and only records whose retention window
    /// has elapsed are removed. Re-running is safe: already-reclaimed records
    /// are skipped, and live (still non-reusable) records are never removed.
    pub fn purge_expired_idempotency_keys(
        env: Env,
        entries: Vec<IdempotencyKeyRef>,
        max_entries: u32,
    ) -> Result<u32, SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        if max_entries == 0 || max_entries > MAX_IDEMPOTENCY_PURGE {
            return Err(SignalError::InvalidBatchSize);
        }

        let now = env.ledger().timestamp();
        let mut purged: u32 = 0;
        for entry in entries.iter() {
            if purged >= max_entries {
                break;
            }
            let record_key = (IDEMPOTENCY_KEY, entry.caller.clone(), entry.key.clone());
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<(_, Address, String), IdempotencyRecord>(&record_key)
            {
                // Only records past their retention window are reclaimed; a
                // still-reusable key must keep rejecting duplicate operations.
                if now >= record.expires_at {
                    env.storage().persistent().remove(&record_key);
                    purged += 1;
                }
            }
        }
        Ok(purged)
    }

    /// Issue #1256: read provider metadata together with its monotonic
    /// revision. Returns `None` when no record exists, so a missing record is
    /// distinguishable from a stored revision of zero.
    pub fn get_versioned_metadata(env: Env, provider: Address) -> Option<VersionedMetadata> {
        let key = (METADATA_KEY, provider.clone());
        let metadata: String = env.storage().persistent().get(&key)?;
        let rev_key = (METADATA_REVISION_KEY, provider);
        let revision: u32 = env.storage().persistent().get(&rev_key).unwrap_or(0);
        Some(VersionedMetadata { revision, metadata })
    }

    /// Issue #1256: compare-and-set provider metadata update.
    ///
    /// Succeeds only when `expected_revision` matches the currently stored
    /// revision; a stale value returns [`SignalError::MetadataRevisionConflict`]
    /// and leaves metadata unchanged. On success the revision advances by one,
    /// so a client can detect that its read was stale instead of silently
    /// overwriting a concurrent change.
    pub fn set_provider_metadata_checked(
        env: Env,
        provider: Address,
        metadata: String,
        expected_revision: u32,
    ) -> Result<u32, SignalError> {
        provider.require_auth();
        Self::validate_metadata(&metadata)?;
        if Self::record_state(&env, &provider) == RecordState::Tombstoned {
            return Err(SignalError::IdentifierTombstoned);
        }

        let rev_key = (METADATA_REVISION_KEY, provider.clone());
        let current: u32 = env.storage().persistent().get(&rev_key).unwrap_or(0);
        if current != expected_revision {
            return Err(SignalError::MetadataRevisionConflict);
        }
        let next = current
            .checked_add(1)
            .ok_or(SignalError::MetadataRevisionOverflow)?;

        let key = (METADATA_KEY, provider);
        env.storage().persistent().set(&key, &metadata);
        env.storage().persistent().set(&rev_key, &next);
        Ok(next)
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
        let key = (

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
