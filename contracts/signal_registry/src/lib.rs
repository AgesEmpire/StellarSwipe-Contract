//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.
//!
//! Provider metadata is validated before any persistent write. Metadata must
//! be non-empty, valid UTF-8, and at most [`MAX_METADATA_LEN`] bytes long, and
//! must already be in the canonical form described below.
//!
//! ## Canonical provider metadata policy (issue #1249)
//!
//! The contract never rewrites metadata: it stores exactly the bytes it was
//! given, so it only accepts payloads that are already canonical. Checks run
//! in this order and the first failure is returned; a rejected payload never
//! reaches storage. [`check_metadata_policy`] is the reference implementation.
//!
//! 1. **Length** — `0` bytes is [`SignalError::EmptyMetadata`]; more than
//!    [`MAX_METADATA_LEN`] bytes is [`SignalError::MetadataTooLong`]. Length
//!    is counted in UTF-8 bytes, not characters.
//! 2. **Encoding** — the bytes must be well-formed UTF-8 (no truncated or
//!    overlong sequences, no encoded surrogates), otherwise
//!    [`SignalError::InvalidMetadataEncoding`].
//! 3. **Empty-like values** — a payload made only of Unicode `White_Space`
//!    characters (spaces, tabs, newlines, no-break or ideographic spaces) is
//!    [`SignalError::EmptyMetadata`].
//! 4. **Per character**, scanning left to right:
//!    * Unicode control characters (`Cc`: U+0000–U+001F, U+007F–U+009F,
//!      including tab and newline) are [`SignalError::MetadataControlCharacter`].
//!    * Invisible or misleading code points — format characters (zero-width
//!      characters, bidi embeddings/overrides/isolates, BOM, soft hyphen),
//!      other default-ignorable code points (variation selectors, Hangul
//!      fillers, tags), private-use code points, noncharacters, and U+FFFC /
//!      U+FFFD — are [`SignalError::MetadataDisallowedCharacter`].
//!    * Code points that make canonically equivalent text ambiguous are
//!      [`SignalError::MetadataNotNormalized`]: generic combining marks
//!      (U+0300–U+036F, U+1AB0–U+1AFF, U+1DC0–U+1DFF, U+20D0–U+20FF,
//!      U+FE20–U+FE2F), kana voicing marks, Arabic combining madda/hamza,
//!      Hangul conjoining jamo, NFC singletons (e.g. U+212B ANGSTROM SIGN,
//!      U+037E GREEK QUESTION MARK, CJK compatibility ideographs), NFC
//!      composition exclusions, and decomposed Brahmic vowel pairs whose
//!      precomposed form is the NFC form (e.g. Tamil U+0B92 U+0BD7).
//!      Clients should submit NFC text with precomposed characters.
//!    * The only accepted whitespace is a single U+0020 SPACE between two
//!      non-space characters. Leading, trailing, or repeated spaces and any
//!      other whitespace character are
//!      [`SignalError::MetadataNonCanonicalWhitespace`].
//!
//! The policy is deterministic: it depends only on the payload bytes. It does
//! not ship full Unicode normalization tables, so canonical reordering of
//! multiple combining marks and compositions in supplementary-plane scripts
//! are outside its scope; see `docs/provider_metadata_policy.md`.
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
//!
//! ## Address validation (issue #1171)
//!
//! Every state-changing entrypoint rejects sentinel addresses — the all-zero
//! account ([`ZERO_ACCOUNT_STRKEY`]), the all-zero contract
//! ([`ZERO_CONTRACT_STRKEY`]), and the registry's own address — with
//! [`SignalError::InvalidAddress`] before any authorization check or storage
//! write. Read-only queries never fail on a sentinel; they report the absent
//! record because no sentinel can ever have stored state. The full inventory
//! is in `docs/address_validation_audit.md`.

#![no_std]

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
    /// Issue #1252: a bounded multi-provider lookup was requested with more
    /// addresses than the documented maximum. No work is performed.
    TooManyProviders = 15,
    /// Issue #1256: a compare-and-set metadata update was attempted with an
    /// expected revision that does not match the stored revision.
    MetadataRevisionConflict = 16,
    /// Issue #1256: the monotonic metadata revision counter cannot advance
    /// further without overflowing.
    MetadataRevisionOverflow = 17,
    /// Issue #1249: metadata contains a Unicode control character.
    MetadataControlCharacter = 18,
    /// Issue #1249: metadata contains an invisible, private-use,
    /// noncharacter, or replacement code point.
    MetadataDisallowedCharacter = 19,
    /// Issue #1249: metadata contains a code point that is not in canonical
    /// (NFC, precomposed) form.
    MetadataNotNormalized = 20,
    /// Issue #1249: metadata has leading, trailing, repeated, or non-ASCII
    /// whitespace.
    MetadataNonCanonicalWhitespace = 21,
    /// Issue #1171: an address argument is a sentinel (all-zero account,
    /// all-zero contract, or this contract). Shared code across contracts.
    InvalidAddress = 100,
}

/// Maximum accepted length, in bytes, of provider metadata.
///
/// Metadata is stored and indexed on-chain, so an explicit upper bound keeps
/// storage and indexing costs predictable. Values longer than this are
/// rejected before any persistent write occurs.
pub const MAX_METADATA_LEN: u32 = 256;

/// Issue #1171: strkey of the account whose ed25519 public key is all zero
/// bytes. It has no usable signer and is the conventional "zero address".
pub const ZERO_ACCOUNT_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/// Issue #1171: strkey of the contract whose 32-byte id is all zero bytes.
pub const ZERO_CONTRACT_STRKEY: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4";

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

/// Issue #1252: documented maximum number of provider addresses accepted by a
/// single bounded multi-provider reputation lookup. Requests above this bound
/// are rejected before any work is performed so caller cost stays predictable.
pub const MAX_REPUTATION_LOOKUP: u32 = 20;

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

/// Single entry of a bounded multi-provider reputation lookup (issue #1252).
///
/// `known` is an explicit representation for unknown providers: an entry is
/// always produced for every requested address, in the requested order, and
/// `known == false` marks an address with no stored reputation record so
/// callers never have to infer absence from a zeroed score.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationLookupEntry {
    pub provider: Address,
    pub known: bool,
    pub record: ReputationRecord,
}

/// Structured payload for signal registry record lifecycle events (issue #1251).
///
/// A consumer can replay these events into a model of the active registry
/// view: `active == true` registers or revises a record, `active == false`
/// removes it. `operation` distinguishes the exact transition and `timestamp`
/// is the ledger time at which it happened.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryRecordEvent {
    pub provider: Address,
    pub operation: Symbol,
    pub active: bool,
    pub timestamp: u64,
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
        Self::require_valid_address(&env, &admin)?;
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
        Self::require_valid_address(&env, &new_admin)?;
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
        Self::require_valid_address(&env, &provider)?;
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

    /// Issue #1252: bounded multi-provider reputation lookup.
    ///
    /// Returns one [`ReputationLookupEntry`] for every requested provider, in
    /// the exact order supplied, so callers get a deterministic mapping back to
    /// their request. Requests larger than [`MAX_REPUTATION_LOOKUP`] are
    /// rejected with [`SignalError::TooManyProviders`] before any storage read.
    /// Unknown providers yield an explicit `known == false` entry with a zeroed
    /// record rather than being silently omitted.
    pub fn get_reputations(
        env: Env,
        providers: Vec<Address>,
    ) -> Result<Vec<ReputationLookupEntry>, SignalError> {
        if providers.len() > MAX_REPUTATION_LOOKUP {
            return Err(SignalError::TooManyProviders);
        }

        let mut entries = Vec::new(&env);
        for provider in providers.iter() {
            let key = (REPUTATION_KEY, provider.clone());
            let stored: Option<ReputationRecord> = env.storage().persistent().get(&key);
            let known = stored.is_some();
            let record = stored.unwrap_or(ReputationRecord {
                score: 0,
                last_updated: 0,
            });
            entries.push_back(ReputationLookupEntry {
                provider,
                known,
                record,
            });
        }
        Ok(entries)
    }

    /// Store provider metadata after validating it against the canonical
    /// metadata policy (issue #1249).
    ///
    /// Validation runs before any persistent write, so empty, over-limit,
    /// malformed, or non-canonical values never reach storage. Identifiers that
    /// are currently tombstoned cannot be reused until their retention period
    /// elapses.
    pub fn set_provider_metadata(
        env: Env,
        provider: Address,
        metadata: String,
    ) -> Result<(), SignalError> {
        Self::require_valid_address(&env, &provider)?;
        provider.require_auth();
        Self::validate_metadata(&metadata)?;
        if Self::record_state(&env, &provider) == RecordState::Tombstoned {
            return Err(SignalError::IdentifierTombstoned);
        }
        let key = (METADATA_KEY, provider.clone());
        // Issue #1251: distinguish a first registration from a revision so the
        // event stream is replay-complete.
        let existed = env.storage().persistent().has(&key);
        env.storage().persistent().set(&key, &metadata);

        Self::emit_record_event(
            &env,
            provider,
            if existed {
                Symbol::new(&env, "record_revised")
            } else {
                Symbol::new(&env, "record_registered")
            },
            true,
        );
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
        Self::require_valid_address(&env, &provider)?;
        provider.require_auth();
        let key = (METADATA_KEY, provider.clone());
        env.storage().persistent().remove(&key);
        let tomb_key = (TOMBSTONE_KEY, provider.clone());
        if !env.storage().persistent().has(&tomb_key) {
            let now = env.ledger().timestamp();
            let tombstone = Tombstone {
                removed_at: now,
                expires_at: now.saturating_add(TOMBSTONE_RETENTION_SECONDS),
            };
            env.storage().persistent().set(&tomb_key, &tombstone);
        }
        // Issue #1251: emit the removal transition. Re-removing an already
        // tombstoned identifier is idempotent and emits the same removal event
        // so replay stays consistent with the returned state.
        Self::emit_record_event(&env, provider, Symbol::new(&env, "record_removed"), false);
        Ok(())
    }

    /// Emit a structured registry record lifecycle event (issue #1251).
    fn emit_record_event(env: &Env, provider: Address, operation: Symbol, active: bool) {
        env.events().publish(
            (operation.clone(), provider.clone()),
            RegistryRecordEvent {
                provider,
                operation,
                active,
                timestamp: env.ledger().timestamp(),
            },
        );
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
        for provider in providers.iter() {
            if purged >= max_entries {
                break;
            }
            let tomb_key = (TOMBSTONE_KEY, provider.clone());
            if let Some(tombstone) = env
                .storage()
                .persistent()
                .get::<(_, Address), Tombstone>(&tomb_key)
            {
                if now >= tombstone.expires_at {
                    env.storage().persistent().remove(&tomb_key);
                    purged += 1;
                }
            }
        }
        Ok(purged)
    }

    /// Apply a reputation update for a provider, first decaying the stored
    /// score according to the configured schedule.
    pub fn update_reputation(
        env: Env,
        provider: Address,
        delta: i128,
    ) -> Result<i128, SignalError> {
        Self::require_valid_address(&env, &provider)?;
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
        Ok(updated)
    }

    /// Read the current (decayed) reputation for a provider without mutating
    /// state. Useful for off-chain verification and contract tests.
    pub fn get_reputation(env: Env, provider: Address) -> Result<i128, SignalError> {
        let schedule: DecaySchedule = env
            .storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)?;
        let record = Self::load_reputation(&env, &provider);
        let now = env.ledger().timestamp();
        Ok(Self::apply_decay(&record, now, &schedule))
    }

    /// Classify an identifier as active, tombstoned, or unknown.
    ///
    /// A tombstone whose retention window has elapsed is treated as unknown,
    /// so the identifier becomes reusable once the documented period passes.
    fn record_state(env: &Env, provider: &Address) -> RecordState {
        let meta_key = (METADATA_KEY, provider.clone());
        if env.storage().persistent().has(&meta_key) {
            return RecordState::Active;
        }
        let tomb_key = (TOMBSTONE_KEY, provider.clone());
        if let Some(tombstone) = env
            .storage()
            .persistent()
            .get::<(_, Address), Tombstone>(&tomb_key)
        {
            if env.ledger().timestamp() < tombstone.expires_at {
                return RecordState::Tombstoned;
            }
        }
        RecordState::Unknown
    }

    /// Compute the decayed score for a record at a given timestamp.
    ///
    /// Deterministic: depends only on the stored record, the timestamp, and
    /// the configured schedule.
    fn apply_decay(record: &ReputationRecord, now: u64, schedule: &DecaySchedule) -> i128 {
        if now <= record.last_updated {
            return record.score;
        }
        let elapsed = now - record.last_updated;
        if elapsed <= schedule.grace_period {
            return record.score;
        }
        let decayable = elapsed - schedule.grace_period;
        if schedule.decay_interval == 0 {
            return record.score;
        }
        let intervals = decayable / schedule.decay_interval;
        if intervals == 0 {
            return record.score;
        }
        // Points lost = score * rate_bps * intervals / 10_000, computed with
        // saturating arithmetic to remain deterministic on-chain.
        let rate = schedule.decay_rate_bps as i128;
        let lost = record
            .score
            .saturating_mul(rate)
            .saturating_mul(intervals as i128)
            / 10_000;
        let decayed = record.score.saturating_sub(lost);
        Self::clamp(decayed, schedule)
    }

    /// Clamp a reputation value to the configured thresholds.
    fn clamp(value: i128, schedule: &DecaySchedule) -> i128 {
        if value < schedule.min_reputation {
            schedule.min_reputation
        } else if value > schedule.max_reputation {
            schedule.max_reputation
        } else {
            value
        }
    }

    /// Validate a decay schedule before it is stored or applied.
    fn validate_schedule(schedule: &DecaySchedule) -> Result<(), SignalError> {
        if schedule.decay_rate_bps > 10_000 {
            return Err(SignalError::InvalidDecayConfig);
        }
        if schedule.decay_interval == 0 {
            return Err(SignalError::InvalidDecayConfig);
        }
        if schedule.min_reputation > schedule.max_reputation {
            return Err(SignalError::InvalidDecayConfig);
        }
        Ok(())
    }

    /// Validate provider metadata against the canonical metadata policy
    /// (issue #1249) before it is stored.
    fn validate_metadata(metadata: &String) -> Result<(), SignalError> {
        let len = metadata.len();
        if len == 0 {
            return Err(SignalError::EmptyMetadata);
        }
        if len > MAX_METADATA_LEN {
            return Err(SignalError::MetadataTooLong);
        }
        let mut buf = [0u8; MAX_METADATA_LEN as usize];
        let bytes = &mut buf[..len as usize];
        metadata.copy_into_slice(bytes);
        check_metadata_policy(bytes)
    }

    /// Consume an idempotency key for a privileged operation.
    ///
    /// Records are stored per `(caller, key)` and remember the operation that
    /// consumed them. A key still inside its retention window for the same
    /// operation is rejected with [`SignalError::IdempotencyKeyReused`]. The
    /// record is written in the same invocation as the guarded operation, so a
    /// failed operation rolls the record back and the key stays usable.
    fn consume_idempotency_key(
        env: &Env,
        caller: &Address,
        operation: &str,
        key: &String,
    ) -> Result<(), SignalError> {
        let operation = String::from_str(env, operation);
        let record_key = (IDEMPOTENCY_KEY, caller.clone(), key.clone());
        let now = env.ledger().timestamp();
        if let Some(record) = env
            .storage()
            .persistent()
            .get::<(_, Address, String), IdempotencyRecord>(&record_key)
        {
            if record.operation == operation && now < record.expires_at {
                return Err(SignalError::IdempotencyKeyReused);
            }
        }
        env.storage().persistent().set(
            &record_key,
            &IdempotencyRecord {
                operation,
                expires_at: now.saturating_add(IDEMPOTENCY_KEY_RETENTION_SECONDS),
            },
        );
        Ok(())
    }

    /// Issue #1171: reject sentinel addresses with a stable error.
    fn require_valid_address(env: &Env, address: &Address) -> Result<(), SignalError> {
        if *address == env.current_contract_address()
            || *address == Address::from_str(env, ZERO_ACCOUNT_STRKEY)
            || *address == Address::from_str(env, ZERO_CONTRACT_STRKEY)
        {
            return Err(SignalError::InvalidAddress);
        }
        Ok(())
    }

    /// Load the stored reputation record for a provider, defaulting to zero.
    fn load_reputation(env: &Env, provider: &Address) -> ReputationRecord {
        let key = (REPUTATION_KEY, provider.clone());
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(ReputationRecord {
                score: 0,
                last_updated: env.ledger().timestamp(),
            })
    }

    /// Persist a reputation record for a provider.
    fn store_reputation(env: &Env, provider: &Address, record: &ReputationRecord) {
        let key = (REPUTATION_KEY, provider.clone());
        env.storage().persistent().set(&key, record);
    }
}

/// Check raw metadata bytes against the canonical provider metadata policy
/// (issue #1249). See the module documentation for the ordered rule list.
///
/// This is a pure function of `bytes`, so off-chain clients can run the exact
/// same check before submitting a transaction.
pub fn check_metadata_policy(bytes: &[u8]) -> Result<(), SignalError> {
    if bytes.is_empty() {
        return Err(SignalError::EmptyMetadata);
    }
    if bytes.len() > MAX_METADATA_LEN as usize {
        return Err(SignalError::MetadataTooLong);
    }
    let text = core::str::from_utf8(bytes).map_err(|_| SignalError::InvalidMetadataEncoding)?;
    if text.chars().all(char::is_whitespace) {
        return Err(SignalError::EmptyMetadata);
    }

    let mut prev: Option<char> = None;
    for c in text.chars() {
        if c.is_control() {
            return Err(SignalError::MetadataControlCharacter);
        }
        if is_disallowed_metadata_char(c) {
            return Err(SignalError::MetadataDisallowedCharacter);
        }
        if is_non_canonical_metadata_char(c)
            || prev.is_some_and(|p| composes_to_precomposed(p, c))
        {
            return Err(SignalError::MetadataNotNormalized);
        }
        if c.is_whitespace() {
            // Only a single interior ASCII space is canonical.
            if c != ' ' || matches!(prev, None | Some(' ')) {
                return Err(SignalError::MetadataNonCanonicalWhitespace);
            }
        }
        prev = Some(c);
    }
    if prev == Some(' ') {
        return Err(SignalError::MetadataNonCanonicalWhitespace);
    }
    Ok(())
}

/// Invisible or misleading code points that never belong in metadata:
/// format and other default-ignorable characters, private use, noncharacters,
/// and the object/replacement characters.
fn is_disallowed_metadata_char(c: char) -> bool {
    let cp = c as u32;
    // Noncharacters U+xxFFFE / U+xxFFFF in every plane.
    if cp & 0xFFFE == 0xFFFE {
        return true;
    }
    matches!(
        cp,
        // Format characters (Cf) and default-ignorable code points.
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xFFF0..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
            // Object replacement and replacement character (lossy decoding).
            | 0xFFFC..=0xFFFD
            // Private use areas.
            | 0xE000..=0xF8FF
            | 0xF0000..=0x10FFFF
            // Arabic presentation-form noncharacters.
            | 0xFDD0..=0xFDEF
    )
}

/// Code points whose presence means the text is not in canonical (NFC,
/// precomposed) form, or whose meaning depends on a canonical decomposition.
fn is_non_canonical_metadata_char(c: char) -> bool {
    matches!(
        c as u32,
        // Generic combining mark blocks.
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
            // Arabic combining madda / hamza above / hamza below.
            | 0x0653..=0x0655
            // Combining kana voiced / semi-voiced sound marks.
            | 0x3099..=0x309A
            // Hangul conjoining jamo (decomposed syllables).
            | 0x1100..=0x11FF
            | 0xA960..=0xA97F
            | 0xD7B0..=0xD7FF
            // NFC singletons.
            | 0x0374
            | 0x037E
            | 0x0387
            | 0x1F71
            | 0x1F73
            | 0x1F75
            | 0x1F77
            | 0x1F79
            | 0x1F7B
            | 0x1F7D
            | 0x1FBB
            | 0x1FBE
            | 0x1FC9
            | 0x1FCB
            | 0x1FD3
            | 0x1FDB
            | 0x1FE3
            | 0x1FEB
            | 0x1FEE
            | 0x1FEF
            | 0x1FF9
            | 0x1FFB
            | 0x1FFD
            | 0x2126
            | 0x212A
            | 0x212B
            | 0x2329
            | 0x232A
            // CJK compatibility ideographs that are NFC singletons (the
            // unified ideographs in this block, e.g. U+FA0E, stay allowed).
            | 0xF900..=0xFA0D
            | 0xFA10
            | 0xFA12
            | 0xFA15..=0xFA1E
            | 0xFA20
            | 0xFA22
            | 0xFA25..=0xFA26
            | 0xFA2A..=0xFA6D
            | 0xFA70..=0xFAD9
            | 0x2F800..=0x2FA1D
            // NFC composition exclusions (script-specific precomposed forms).
            | 0x0958..=0x095F
            | 0x09DC..=0x09DD
            | 0x09DF
            | 0x0A33
            | 0x0A36
            | 0x0A59..=0x0A5B
            | 0x0A5E
            | 0x0B5C..=0x0B5D
            | 0x0F43
            | 0x0F4D
            | 0x0F52
            | 0x0F57
            | 0x0F5C
            | 0x0F69
            | 0x0F73
            | 0x0F75..=0x0F79
            | 0x0F81
            | 0x0F93
            | 0x0F9D
            | 0x0FA2
            | 0x0FA7
            | 0x0FAC
            | 0x0FB9
            | 0x2ADC
            | 0xFB1D
            | 0xFB1F
            | 0xFB2A..=0xFB4E
            | 0x1D15E..=0x1D164
            | 0x1D1BB..=0x1D1C0
    )
}

/// Decomposed BMP sequences whose NFC form is a single precomposed code point
/// outside the generic combining-mark blocks (Indic and Brahmic vowel signs,
/// nukta forms, Myanmar, and Balinese). Accepting both spellings would store
/// canonically equivalent metadata as different values.
fn composes_to_precomposed(prev: char, c: char) -> bool {
    matches!(
        (prev as u32, c as u32),
        // Devanagari nukta forms.
        (0x0928, 0x093C) | (0x0930, 0x093C) | (0x0933, 0x093C)
            // Bengali.
            | (0x09C7, 0x09BE) | (0x09C7, 0x09D7)
            // Oriya.
            | (0x0B47, 0x0B56) | (0x0B47, 0x0B3E) | (0x0B47, 0x0B57)
            // Tamil.
            | (0x0B92, 0x0BD7) | (0x0BC6, 0x0BBE) | (0x0BC7, 0x0BBE) | (0x0BC6, 0x0BD7)
            // Telugu.
            | (0x0C46, 0x0C56)
            // Kannada.
            | (0x0CBF, 0x0CD5) | (0x0CC6, 0x0CD5) | (0x0CC6, 0x0CD6) | (0x0CC6, 0x0CC2)
            | (0x0CCA, 0x0CD5)
            // Malayalam.
            | (0x0D46, 0x0D3E) | (0x0D47, 0x0D3E) | (0x0D46, 0x0D57)
            // Sinhala.
            | (0x0DD9, 0x0DCA) | (0x0DD9, 0x0DCF) | (0x0DDC, 0x0DCA) | (0x0DD9, 0x0DDF)
            // Myanmar.
            | (0x1025, 0x102E)
            // Balinese.
            | (0x1B05, 0x1B35) | (0x1B07, 0x1B35) | (0x1B09, 0x1B35) | (0x1B0B, 0x1B35)
            | (0x1B0D, 0x1B35) | (0x1B11, 0x1B35) | (0x1B3A, 0x1B35) | (0x1B3C, 0x1B35)
            | (0x1B3E, 0x1B35) | (0x1B3F, 0x1B35) | (0x1B42, 0x1B35)
    )
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
    use soroban_sdk::TryFromVal;

    fn schedule() -> DecaySchedule {
        DecaySchedule {
            decay_rate_bps: 1_000,
            decay_interval: 100,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 1_000,
        }
    }

    fn setup(env: &Env) -> (SignalRegistryClient<'_>, Address) {
        env.mock_all_auths();
        let contract_id = env.register(SignalRegistry, ());
        let client = SignalRegistryClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.initialize(&admin, &schedule());
        (client, admin)
    }

    fn sentinels(env: &Env, client: &SignalRegistryClient) -> [Address; 3] {
        [
            Address::from_str(env, ZERO_ACCOUNT_STRKEY),
            Address::from_str(env, ZERO_CONTRACT_STRKEY),
            client.address.clone(),
        ]
    }

    fn policy(text: &str) -> Result<(), SignalError> {
        check_metadata_policy(text.as_bytes())
    }

    fn repeat(unit: &str, times: usize) -> std::string::String {
        unit.repeat(times)
    }

    fn no_record_event(env: &Env) -> bool {
        !env.events().all().iter().any(|(_, _, data)| {
            RegistryRecordEvent::try_from_val(env, &data).is_ok()
        })
    }

    // ── Issue #1249: canonical metadata policy ─────────────────────────────

    #[test]
    fn canonical_metadata_is_accepted() {
        for text in [
            "a",
            "Alpha Signals",
            "Provider #42: BTC/USDC momentum",
            "Café Zürich",
            "Δελτίο σημάτων",
            "東京 シグナル",
            "가나다",
            "ஒளவை",
            "\u{0B94}",
            "\u{FA0E}",
            "rocket 🚀 desk",
        ] {
            assert_eq!(policy(text), Ok(()), "expected {text:?} to be accepted");
        }
    }

    #[test]
    fn empty_like_metadata_is_rejected() {
        for text in ["", " ", "   ", "\t", "\n", "\r\n", " \t ", "\u{00A0}", "\u{3000}", "\u{2028}"] {
            assert_eq!(
                policy(text),
                Err(SignalError::EmptyMetadata),
                "expected {text:?} to be empty-like"
            );
        }
    }

    #[test]
    fn control_characters_are_rejected() {
        for text in ["a\u{0}b", "a\tb", "line\nbreak", "a\rb", "a\u{1B}[0m", "a\u{7F}", "a\u{85}b", "\u{9F}a"] {
            assert_eq!(
                policy(text),
                Err(SignalError::MetadataControlCharacter),
                "expected {text:?} to be rejected"
            );
        }
    }

    #[test]
    fn invisible_and_misleading_characters_are_rejected() {
        for text in [
            "pay\u{200B}pal",
            "\u{FEFF}bom",
            "user\u{202E}gnp.exe",
            "a\u{2066}b\u{2069}",
            "soft\u{00AD}hyphen",
            "heart\u{FE0F}",
            "tag\u{E0041}",
            "private\u{E000}",
            "lossy\u{FFFD}",
            "nonchar\u{FFFF}",
            "nonchar\u{FDD0}",
            "nonchar\u{1FFFE}",
            "filler\u{3164}",
        ] {
            assert_eq!(
                policy(text),
                Err(SignalError::MetadataDisallowedCharacter),
                "expected {text:?} to be rejected"
            );
        }
    }

    #[test]
    fn non_normalized_characters_are_rejected() {
        for text in [
            "Cafe\u{0301}",
            "a\u{20DD}",
            "\u{212B}ngstrom",
            "\u{2126}",
            "\u{212A}elvin",
            "why\u{037E}",
            "\u{F900}",
            "\u{2F800}",
            "\u{1100}\u{1161}",
            "\u{304B}\u{3099}",
            "\u{0627}\u{0653}",
            "\u{0958}",
            "\u{0B92}\u{0BD7}",
            "\u{0928}\u{093C}",
            "\u{0D46}\u{0D3E}",
        ] {
            assert_eq!(
                policy(text),
                Err(SignalError::MetadataNotNormalized),
                "expected {text:?} to be rejected"
            );
        }
        // The precomposed spellings of the same text are accepted.
        for text in ["Caf\u{00E9}", "\u{00C5}ngstrom", "\u{AC00}", "\u{304C}", "\u{0622}", "\u{0B94}", "\u{0929}", "\u{0D4A}"] {
            assert_eq!(policy(text), Ok(()), "expected {text:?} to be accepted");
        }
    }

    #[test]
    fn non_canonical_whitespace_is_rejected() {
        for text in [
            " leading",
            "trailing ",
            "double  space",
            "nbsp\u{00A0}space",
            "ideographic\u{3000}space",
            "thin\u{2009}space",
            "line\u{2028}separator",
        ] {
            assert_eq!(
                policy(text),
                Err(SignalError::MetadataNonCanonicalWhitespace),
                "expected {text:?} to be rejected"
            );
        }
        assert_eq!(policy("single spaces only"), Ok(()));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let cases: [&[u8]; 7] = [
            &[0xFF],
            &[0x80],
            &[b'a', 0xE2, 0x82],
            &[0xC0, 0xAF],
            &[0xED, 0xA0, 0x80],
            &[0xF4, 0x90, 0x80, 0x80],
            &[0xF0, 0x9F, 0x9A],
        ];
        for bytes in cases {
            assert_eq!(
                check_metadata_policy(bytes),
                Err(SignalError::InvalidMetadataEncoding),
                "expected {bytes:?} to be rejected"
            );
        }
    }

    #[test]
    fn length_is_counted_in_bytes_at_multibyte_boundaries() {
        let max = MAX_METADATA_LEN as usize;
        // Exactly MAX_METADATA_LEN bytes with 2-, 3-, and 4-byte characters.
        let two = repeat("é", max / 2);
        let three = std::format!("{}a", repeat("€", max / 3));
        let four = repeat("🚀", max / 4);
        for text in [&two, &three, &four] {
            assert_eq!(text.len(), max);
            assert_eq!(policy(text), Ok(()));
        }
        // One byte over the limit is rejected even though it is short in chars.
        let over = std::format!("{two}a");
        assert_eq!(over.len(), max + 1);
        assert_eq!(policy(&over), Err(SignalError::MetadataTooLong));
        // A payload that fits only by cutting a multibyte character in half is
        // an encoding error, not a silently truncated value.
        let mut cut = repeat("a", max - 3).into_bytes();
        cut.extend_from_slice(&"€".as_bytes()[..2]);
        assert_eq!(cut.len(), max - 1);
        assert_eq!(
            check_metadata_policy(&cut),
            Err(SignalError::InvalidMetadataEncoding)
        );
    }

    #[test]
    fn rejected_metadata_never_reaches_storage() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        let original = String::from_str(&env, "Alpha Signals");
        client.set_provider_metadata(&provider, &original);

        let too_long = repeat("a", MAX_METADATA_LEN as usize + 1);
        let invalid: [(String, SignalError); 6] = [
            (String::from_str(&env, ""), SignalError::EmptyMetadata),
            (String::from_str(&env, "   "), SignalError::EmptyMetadata),
            (String::from_str(&env, "Alpha\nSignals"), SignalError::MetadataControlCharacter),
            (String::from_str(&env, "Alpha Signals "), SignalError::MetadataNonCanonicalWhitespace),
            (String::from_bytes(&env, &[b'a', 0xE2, 0x82]), SignalError::InvalidMetadataEncoding),
            (String::from_str(&env, &too_long), SignalError::MetadataTooLong),
        ];
        for (metadata, error) in invalid.iter() {
            assert_eq!(
                client.try_set_provider_metadata(&provider, metadata),
                Err(Ok(*error))
            );
            assert!(no_record_event(&env));
            assert_eq!(
                client.try_set_provider_metadata_checked(&provider, metadata, &0),
                Err(Ok(*error))
            );
            assert_eq!(client.get_provider_metadata(&provider), Some(original.clone()));
        }
        assert_eq!(client.get_versioned_metadata(&provider).unwrap().revision, 0);
        assert_eq!(client.set_provider_metadata_checked(&provider, &String::from_str(&env, "Beta"), &0), 1);
    }

    // ── Issue #1171: address validation ────────────────────────────────────

    #[test]
    fn initialize_rejects_sentinel_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(SignalRegistry, ());
        let client = SignalRegistryClient::new(&env, &contract_id);

        for sentinel in sentinels(&env, &client) {
            assert_eq!(
                client.try_initialize(&sentinel, &schedule()),
                Err(Ok(SignalError::InvalidAddress))
            );
        }
        assert_eq!(
            client.try_get_decay_schedule(),
            Err(Ok(SignalError::NotInitialized))
        );
        client.initialize(&Address::generate(&env), &schedule());
    }

    #[test]
    fn state_changing_entrypoints_reject_sentinel_addresses() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let metadata = String::from_str(&env, "Alpha Signals");

        for sentinel in sentinels(&env, &client) {
            assert_eq!(
                client.try_propose_admin(&sentinel),
                Err(Ok(SignalError::InvalidAddress))
            );
            assert_eq!(
                client.try_set_provider_metadata(&sentinel, &metadata),
                Err(Ok(SignalError::InvalidAddress))
            );
            assert!(no_record_event(&env));
            assert_eq!(
                client.try_set_provider_metadata_checked(&sentinel, &metadata, &0),
                Err(Ok(SignalError::InvalidAddress))
            );
            assert_eq!(
                client.try_remove_provider_metadata(&sentinel),
                Err(Ok(SignalError::InvalidAddress))
            );
            assert!(no_record_event(&env));
            assert_eq!(
                client.try_update_reputation(&sentinel, &10),
                Err(Ok(SignalError::InvalidAddress))
            );

            // Read-only queries report the absent record instead of failing.
            assert_eq!(client.get_provider_metadata(&sentinel), None);
            assert_eq!(client.get_versioned_metadata(&sentinel), None);
            assert_eq!(client.get_record_state(&sentinel), RecordState::Unknown);
            assert_eq!(client.get_reputation(&sentinel), 0);
            let lookup = client.get_reputations(&Vec::from_array(&env, [sentinel.clone()]));
            assert!(!lookup.get(0).unwrap().known);
        }
        assert_eq!(client.get_pending_admin(), None);
    }

    // ── Behavior restored alongside the audit ──────────────────────────────

    #[test]
    fn metadata_lifecycle_and_tombstones() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        let metadata = String::from_str(&env, "Alpha Signals");

        client.set_provider_metadata(&provider, &metadata);
        assert_eq!(client.get_record_state(&provider), RecordState::Active);
        client.remove_provider_metadata(&provider);
        assert_eq!(client.get_record_state(&provider), RecordState::Tombstoned);
        assert_eq!(
            client.try_set_provider_metadata(&provider, &metadata),
            Err(Ok(SignalError::IdentifierTombstoned))
        );

        let providers = Vec::from_array(&env, [provider.clone()]);
        assert_eq!(client.purge_expired_tombstones(&providers, &10), 0);
        env.ledger().with_mut(|l| l.timestamp += TOMBSTONE_RETENTION_SECONDS);
        assert_eq!(client.purge_expired_tombstones(&providers, &10), 1);
        assert_eq!(client.get_record_state(&provider), RecordState::Unknown);
        client.set_provider_metadata(&provider, &metadata);
    }

    #[test]
    fn idempotency_key_cannot_be_reused_within_retention() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let key = String::from_str(&env, "op-1");
        let mut updated = schedule();
        updated.decay_rate_bps = 500;

        client.set_decay_schedule(&updated, &key);
        assert_eq!(
            client.try_set_decay_schedule(&schedule(), &key),
            Err(Ok(SignalError::IdempotencyKeyReused))
        );
        assert_eq!(client.get_decay_schedule(), updated);

        // A failed operation does not consume its key.
        let invalid_key = String::from_str(&env, "op-2");
        let mut invalid = schedule();
        invalid.decay_interval = 0;
        assert_eq!(
            client.try_set_decay_schedule(&invalid, &invalid_key),
            Err(Ok(SignalError::InvalidDecayConfig))
        );
        client.set_decay_schedule(&schedule(), &invalid_key);

        env.ledger().with_mut(|l| l.timestamp += IDEMPOTENCY_KEY_RETENTION_SECONDS);
        client.set_decay_schedule(&updated, &key);
    }

    #[test]
    fn reputation_decays_per_schedule() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);

        assert_eq!(client.update_reputation(&provider, &500), 500);
        env.ledger().with_mut(|l| l.timestamp += 200);
        // Two intervals at 10% each: 500 - 500 * 1_000 * 2 / 10_000 = 400.
        assert_eq!(client.get_reputation(&provider), 400);
    }
}
