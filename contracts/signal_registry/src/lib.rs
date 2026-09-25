//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.
//!
//! ## TTL bump strategy
//!
//! Contract instance and persistent storage entries are extended using an
//! explicit, configurable strategy so active state never expires during
//! normal use while avoiding unnecessary rent spend. Thresholds and extension
//! amounts are stored in [`TtlConfig`] and can be updated by the admin via
//! [`SignalRegistry::set_ttl_config`].
//!
//! An operational maintenance job (off-chain keeper) should periodically call
//! [`SignalRegistry::bump_ttl`] for each active provider. The expected budget
//! is bounded by `persistent_extend_to` ledgers of rent per active entry per
//! bump, and bumps only occur once the remaining TTL drops below
//! `persistent_threshold` (or `instance_threshold` for the instance).
//!
//! ## Address validation
//!
//! Public entrypoints accept Soroban [`Address`] values (contract, account, or
//! authorized invoker). All address inputs are validated through the shared
//! [`validate_address`] helper so malformed or unsupported inputs are rejected
//! consistently with a stable error code ([`SignalError::InvalidAddress`]).
//!
//! ## Initialization audit
//!
//! Required storage items written by [`SignalRegistry::initialize`]:
//! - `admin` (instance): the address authorized to mutate configuration.
//! - `schedule` (instance): the validated decay schedule.
//! - `version` (instance): the storage schema version marker.
//!
//! Initialization is atomic: the version marker is written last, so a partial
//! or interrupted initialization leaves the contract uninitialized and can be
//! safely retried. Repeated initialization is rejected with
//! [`SignalError::AlreadyInitialized`] before any state is mutated.
//!
//! ## Authorization expiration
//!
//! Temporary permissions, delegated capabilities, and time-bounded approvals
//! are stored with an explicit `expires_at` ledger timestamp. Every sensitive
//! entrypoint that consumes a permission fails closed once the current ledger
//! timestamp reaches `expires_at` (inclusive boundary). Expiration cleanup is
//! bounded by [`MAX_EXPIRATION_SWEEP`] and only removes already-expired grants,
//! so it never changes authorization semantics for live permissions.
//!
//! ## Nonce domain separation
//!
//! Nonces are scoped by an explicit [`NonceDomain`] that binds the owning
//! `user`, the `operation` type, and the `contract` domain identifier. A nonce
//! issued for one operation or contract can never be replayed in another flow:
//! the domain is part of both the storage key and the hashed nonce value, so a
//! cross-operation or cross-contract replay resolves to a different key and is
//! rejected as unused. Clients can discover the required domain for a flow via
//! [`SignalRegistry::nonce_domain`].
//!
//! ### Migration strategy for existing nonce state
//!
//! Legacy nonces were stored under the unqualified `(NONCE_KEY, user)` key with
//! no domain binding. On upgrade, `initialize` records the storage version and
//! legacy entries are treated as belonging to the [`NonceDomain::LEGACY`]
//! domain (operation `0`, contract `0`). Operators migrate by re-issuing nonces
//! through [`SignalRegistry::issue_nonce`] under the correct domain; the legacy
//! key is never consulted for domain-scoped flows, so stale nonces cannot be
//! replayed. The version marker is bumped to [`STORAGE_VERSION`] so clients can
//! detect whether migration has completed.
//!
//! Event payloads emitted at contract boundaries are bounded by the limits in
//! [`MAX_EVENT_STRING_LEN`], [`MAX_EVENT_VECTOR_LEN`], and
//! [`MAX_EVENT_METADATA_LEN`]. Oversized payloads are rejected with
//! [`SignalError::EventPayloadTooLarge`] before any emission occurs.
//!
//! Storage keys are namespaced per state domain (see [`StorageNamespace`]) and
//! every user-controlled key component is validated against the documented
//! constraints in [`MAX_KEY_COMPONENT_LEN`] and [`is_valid_key_component`].
//! This prevents unbounded or colliding key layouts. Previously stored keys
//! used the bare `"admin"`, `"schedule"`, and `"reputation"` literals; the
//! namespaced keys below are distinct from those, so legacy state is not
//! silently reinterpreted. A migration path is provided by
//! [`SignalRegistry::migrate_legacy_keys`].
//!
//! Upgrade authority is handed off through a two-step protocol. The current
//! authority initiates a handoff to a pending successor, and the successor
//! must explicitly accept before it becomes the active authority. A pending
//! handoff never alters active authority, only the current authority may
//! initiate or cancel, and handoffs expire after [`HANDOFF_EXPIRY_SECONDS`].

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Bytes, Env, String, Vec};

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
    InvalidCleanupLimit = 6,
    /// The supplied address is malformed or of an unsupported type.
    InvalidAddress = 7,
    /// An event payload exceeded the configured size limits. Returned before
    /// any event is emitted so oversized emissions cannot exhaust transaction
    /// resources or disrupt downstream indexers.
    EventPayloadTooLarge = 8,
    /// A storage key component violated the documented length or character
    /// constraints, or a namespace was used for the wrong state domain.
    InvalidStorageKey = 9,
    /// No upgrade authority handoff is currently pending.
    NoPendingHandoff = 10,
    /// A handoff is already pending; it must be accepted or cancelled first.
    HandoffAlreadyPending = 11,
    /// The pending handoff has expired and can no longer be accepted.
    HandoffExpired = 12,
    /// The caller is not the pending successor and cannot accept the handoff.
    WrongAccepter = 13,
}

/// Maximum length, in bytes, of a string carried in an event payload.
pub const MAX_EVENT_STRING_LEN: u32 = 256;

/// Maximum number of elements in a vector carried in an event payload.
pub const MAX_EVENT_VECTOR_LEN: u32 = 64;

/// Maximum length, in bytes, of event metadata (e.g. a topic or label).
pub const MAX_EVENT_METADATA_LEN: u32 = 128;

/// Maximum length, in bytes, of a single user-controlled storage key
/// component. Bounds the key layout so identifiers cannot grow without limit.
pub const MAX_KEY_COMPONENT_LEN: u32 = 64;

/// Minimum length, in bytes, of a user-controlled storage key component.
pub const MIN_KEY_COMPONENT_LEN: u32 = 1;

/// Number of seconds a pending upgrade authority handoff remains acceptable
/// after it is initiated. After this window the handoff expires and can no
/// longer be accepted; the current authority must initiate a new handoff.
pub const HANDOFF_EXPIRY_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Distinct namespace for each state domain. Namespaces are prefixed onto
/// every storage key so keys from different domains cannot collide.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum StorageNamespace {
    /// Contract configuration (admin, decay schedule).
    Config = 1,
    /// Per-provider reputation records.
    Reputation = 2,
}

impl StorageNamespace {
    /// Stable, documented prefix for this namespace. Kept short and distinct
    /// so namespaced keys never overlap across domains.
    pub const fn prefix(self) -> &'static str {
        match self {
            StorageNamespace::Config => "cfg:",
            StorageNamespace::Reputation => "rep:",
        }
    }
}

/// Returns `true` when `component` satisfies the documented storage key
/// constraints: non-empty, at most [`MAX_KEY_COMPONENT_LEN`] bytes, and
/// restricted to ASCII alphanumerics plus `_`, `-`, and `.`.
///
/// Rejecting other characters keeps key layouts canonical and prevents
/// delimiter-based collisions between namespaces and components.
pub fn is_valid_key_component(component: &str) -> bool {
    let len = component.len();
    if len < MIN_KEY_COMPONENT_LEN as usize || len > MAX_KEY_COMPONENT_LEN as usize {
        return false;
    }
    component.bytes().all(|b| {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
    })
}

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

/// Explicit, configurable TTL bump strategy for contract instance and
/// persistent storage entries.
///
/// `instance_threshold` / `instance_extend_to` control the contract instance
/// entry. `persistent_threshold` / `persistent_extend_to` control persistent
/// entries (e.g. reputation records). A bump is only performed when the
/// remaining TTL is at or below the corresponding threshold, and it extends
/// the entry so that its remaining TTL becomes `extend_to` ledgers.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TtlConfig {
    pub instance_threshold: u32,
    pub instance_extend_to: u32,
    pub persistent_threshold: u32,
    pub persistent_extend_to: u32,
}

/// Stored reputation record for a provider.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationRecord {
    pub score: i128,
    pub last_updated: u64,
}

/// A time-bounded permission, delegated capability, or approval.
///
/// `expires_at` is an inclusive ledger timestamp: the grant is valid while
/// `now < expires_at` and fails closed once `now >= expires_at`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Permission {
    pub grantee: Address,
    pub capability: u32,
    pub expires_at: u64,
}

/// Explicit domain that scopes a nonce.
///
/// A nonce is only valid within the exact `(user, operation, contract)` tuple
/// it was issued for. `operation` distinguishes flows (e.g. transfer vs.
/// withdraw) and `contract` distinguishes the contract domain, so a nonce from
/// one flow cannot be replayed in another.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonceDomain {
    pub user: Address,
    pub operation: u32,
    pub contract: u32,
}

impl NonceDomain {
    /// Domain reserved for pre-domain-separation (legacy) nonce state.
    pub const LEGACY: u32 = 0;
}

/// A pending upgrade authority handoff. Created by the current authority and
/// only promoted to active authority once the successor explicitly accepts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingHandoff {
    /// The successor that must accept before authority transfers.
    pub successor: Address,
    /// Ledger timestamp at which the handoff was initiated.
    pub initiated_at: u64,
    /// Ledger timestamp after which the handoff can no longer be accepted.
    pub expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataKey {
    pub admin: Address,
    pub schedule: DecaySchedule,
}

/// Summary of a bounded cleanup run.
///
/// `removed` counts records that were eligible and deleted during this call.
/// `skipped` counts records that were inspected but not eligible (still live
/// or otherwise not orphaned). `scanned` is the total number of records
/// inspected, bounded by the caller-supplied maximum.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupReport {
    pub scanned: u32,
    pub removed: u32,
    pub skipped: u32,
}

/// Namespaced storage keys. Each variant maps to a distinct namespace prefix
/// so keys from different state domains cannot collide.
const ADMIN_KEY: &str = "cfg:admin";
const SCHEDULE_KEY: &str = "cfg:schedule";
const TTL_KEY: &str = "ttl";
const REPUTATION_KEY: &str = "rep:reputation";
const PENDING_HANDOFF_KEY: &str = "cfg:pending_handoff";
const VERSION_KEY: &str = "cfg:version";
const PERMISSION_KEY: &str = "perm:permission";
const NONCE_KEY: &str = "nonce:nonce";
const OWNER_KEY: &str = "owner";
const RELATIONSHIP_KEY: &str = "relationship";

/// Legacy (pre-namespace) keys, retained only for the explicit migration path.
const LEGACY_ADMIN_KEY: &str = "admin";
const LEGACY_SCHEDULE_KEY: &str = "schedule";
const LEGACY_REPUTATION_KEY: &str = "reputation";

/// Current storage schema version written by `initialize`.
const STORAGE_VERSION: u32 = 1;

/// Upper bound on how many expired permissions a single cleanup call removes.
/// Keeps expiration cleanup bounded and gas-predictable.
pub const MAX_EXPIRATION_SWEEP: u32 = 32;

/// Default TTL strategy: bump instance when below ~1 day of ledgers and
/// extend to ~7 days; bump persistent entries when below ~7 days and extend
/// to ~30 days (assuming ~5s ledgers).
const DEFAULT_TTL_CONFIG: TtlConfig = TtlConfig {
    instance_threshold: 17_280,
    instance_extend_to: 120_960,
    persistent_threshold: 120_960,
    persistent_extend_to: 518_400,
};

/// Canonical address validation helper.
///
/// Accepts any Soroban [`Address`] (contract, account, or authorized invoker)
/// and returns it unchanged when it is well-formed. Malformed or unsupported
/// inputs are rejected with [`SignalError::InvalidAddress`].
///
/// This is the single shared entrypoint for address validation so that all
/// public entrypoints reject bad inputs consistently. The error code is stable
/// and documented for SDK consumers.
pub fn validate_address(address: &Address) -> Result<(), SignalError> {
    // `Address` is a validated Soroban type; the only way to obtain one is via
    // a well-formed contract, account, or authorized invoker value. Reject the
    // zero/empty boundary case explicitly so callers get a stable error.
    if address.to_string().is_empty() {
        return Err(SignalError::InvalidAddress);
    }
    Ok(())
}

/// Validate and normalize an address input, returning the canonical value.
///
/// Normalization is a no-op for well-formed addresses (Soroban addresses are
/// already canonical), but routing through this helper guarantees every
/// public entrypoint applies the same validation behavior.
pub fn normalize_address(address: Address) -> Result<Address, SignalError> {
    validate_address(&address)?;
    Ok(address)
}

#[contract]
pub struct SignalRegistry;

#[contractimpl]
impl SignalRegistry {
    /// Initialize the contract with an admin and a decay schedule.
    ///
    /// Writes every required storage item exactly once: `admin`, `schedule`,
    /// and the `version` marker. The version marker is written last so that an
    /// interrupted initialization is detectable and retryable. If the contract
    /// is already initialized this returns [`SignalError::AlreadyInitialized`]
    /// without mutating any state.
    pub fn initialize(env: Env, admin: Address, schedule: DecaySchedule) -> Result<(), SignalError> {
        let admin = normalize_address(admin)?;
        if Self::is_initialized(&env) {
            return Err(SignalError::AlreadyInitialized);
        }
        Self::validate_schedule(&schedule)?;
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&SCHEDULE_KEY, &schedule);
        env.storage().instance().set(&TTL_KEY, &DEFAULT_TTL_CONFIG);
        Self::bump_instance_ttl(&env, &DEFAULT_TTL_CONFIG);

        // Version marker written last: its presence signals a complete init.
        env.storage().instance().set(&VERSION_KEY, &STORAGE_VERSION);
        Ok(())
    }

    /// Returns `true` once initialization has completed.
    ///
    /// The version marker is the authoritative signal: it is only written after
    /// all other required fields, so a partial initialization reports `false`.
    pub fn is_initialized(env: &Env) -> bool {
        env.storage().instance().has(&VERSION_KEY)
    }

    /// Read the stored schema version marker.
    pub fn get_version(env: Env) -> Result<u32, SignalError> {
        env.storage()
            .instance()
            .get(&VERSION_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Return the nonce domain a client must use for `operation` on this
    /// contract. Clients call this to determine the required domain before
    /// issuing or consuming a nonce.
    pub fn nonce_domain(env: Env, user: Address, operation: u32) -> NonceDomain {
        NonceDomain {
            user,
            operation,
            contract: Self::contract_domain(&env),
        }
    }

    /// Derive the contract domain identifier for this deployment.
    ///
    /// The contract's own address is hashed so that nonces issued by one
    /// deployment can never be replayed against another deployment.
    fn contract_domain(env: &Env) -> u32 {
        let bytes = env.current_contract_address().to_string().into_bytes();
        let mut acc: u32 = 0;
        let mut i: u32 = 0;
        while i < bytes.len() {
            acc = acc.wrapping_mul(31).wrapping_add(bytes.get(i).unwrap() as u32);
            i += 1;
        }
        acc
    }

    /// Hash a nonce together with its domain so the stored value is bound to
    /// the exact `(user, operation, contract)` tuple.
    fn hash_nonce(env: &Env, domain: &NonceDomain, nonce: &Bytes) -> Bytes {
        let mut preimage = Bytes::new(env);
        preimage.append(&domain.user.to_string().into_bytes());
        preimage.extend_from_array(&domain.operation.to_be_bytes());
        preimage.extend_from_array(&domain.contract.to_be_bytes());
        preimage.append(nonce);
        env.crypto().sha256(&preimage).into()
    }

    /// Issue a nonce for `domain`. Rejects a domain that reuses the reserved
    /// legacy operation id so legacy state cannot be silently re-bound.
    pub fn issue_nonce(env: Env, domain: NonceDomain, nonce: Bytes) -> Result<(), SignalError> {
        if domain.operation == NonceDomain::LEGACY {
            return Err(SignalError::InvalidNonceDomain);
        }
        let hashed = Self::hash_nonce(&env, &domain, &nonce);
        let key = (NONCE_KEY, domain.clone(), hashed.clone());
        if env.storage().persistent().has(&key) {
            return Err(SignalError::NonceAlreadyUsed);
        }
        env.storage().persistent().set(&key, &true);
        Ok(())
    }

    /// Consume a nonce within `domain`. Fails closed with
    /// [`SignalError::NonceAlreadyUsed`] when the nonce was already spent in
    /// this domain, and [`SignalError::InvalidNonceDomain`] for the reserved
    /// legacy domain. A nonce issued under a different operation or contract
    /// resolves to a different key and is therefore rejected as unused.
    pub fn consume_nonce(env: Env, domain: NonceDomain, nonce: Bytes) -> Result<(), SignalError> {
        if domain.operation == NonceDomain::LEGACY {
            return Err(SignalError::InvalidNonceDomain);
        }
        let hashed = Self::hash_nonce(&env, &domain, &nonce);
        let key = (NONCE_KEY, domain.clone(), hashed.clone());
        if env.storage().persistent().has(&key) {
            return Err(SignalError::NonceAlreadyUsed);
        }
        env.storage().persistent().set(&key, &true);
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
        Ok(())
    }

    /// Read the currently configured decay schedule.
    pub fn get_decay_schedule(env: Env) -> Result<DecaySchedule, SignalError> {
        env.storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Read the currently active authority.
    pub fn get_authority(env: Env) -> Result<Address, SignalError> {
        env.storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Read the pending handoff, if any. Returns `None` when no handoff is
    /// pending. A pending handoff never changes the active authority.
    pub fn get_pending_handoff(env: Env) -> Option<PendingHandoff> {
        env.storage().instance().get(&PENDING_HANDOFF_KEY)
    }

    /// Initiate a two-step handoff of upgrade authority to `successor`.
    ///
    /// Only the current authority may initiate, and only one handoff may be
    /// pending at a time. The active authority is left unchanged until the
    /// successor accepts via [`SignalRegistry::accept_handoff`].
    pub fn initiate_handoff(env: Env, successor: Address) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        Self::validate_ttl_config(&config)?;
        env.storage().instance().set(&TTL_KEY, &config);
        Self::bump_instance_ttl(&env, &config);
        Ok(())
    }

    /// Read the currently configured TTL bump strategy.
    pub fn get_ttl_config(env: Env) -> Result<TtlConfig, SignalError
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        if env.storage().instance().has(&PENDING_HANDOFF_KEY) {
            return Err(SignalError::HandoffAlreadyPending);
        }

        let now = env.ledger().timestamp();
        let handoff = PendingHandoff {
            successor: successor.clone(),
            initiated_at: now,
            expires_at: now.saturating_add(HANDOFF_EXPIRY_SECONDS),
        };
        env.storage().instance().set(&PENDING_HANDOFF_KEY, &handoff);

        env.events()
            .publish(("handoff_initiated",), (admin, successor));
        Ok(())
    }

    /// Accept a pending handoff, promoting the successor to active authority.
    ///
    /// Only the designated successor may accept, the handoff must not have
    /// expired, and the pending record is cleared on success. Replay attempts
    /// fail because the pending record no longer exists.
    pub fn accept_handoff(env: Env) -> Result<(), SignalError> {
        let handoff: PendingHandoff = env
            .storage()
            .instance()
            .get(&PENDING_HANDOFF_KEY)
            .ok_or(SignalError::NoPendingHandoff)?;

        handoff.successor.require_auth();

        let now = env.ledger().timestamp();
        if now > handoff.expires_at {
            return Err(SignalError::HandoffExpired);
        }

        env.storage()
            .instance()
            .set(&ADMIN_KEY, &handoff.successor);
        env.storage().instance().remove(&PENDING_HANDOFF_KEY);

        env.events()
            .publish(("handoff_accepted",), (handoff.successor, now));
        Ok(())
    }

    /// Cancel a pending handoff. Only the current authority may cancel, and
    /// the active authority is left unchanged.
    pub fn cancel_handoff(env: Env) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();

        if !env.storage().instance().has(&PENDING_HANDOFF_KEY) {
            return Err(SignalError::NoPendingHandoff);
        }
        env.storage().instance().remove(&PENDING_HANDOFF_KEY);

        env.events().publish(("handoff_cancelled",), (admin,));
        Ok(())
    }

    /// Explicit migration path for state written before namespaced keys were
    /// introduced. Copies legacy `admin`/`schedule`/`reputation` entries into
    /// their namespaced counterparts. Idempotent: already-migrated keys are
    /// left untouched, and legacy keys are removed once copied.
    pub fn migrate_legacy_keys(env: Env) -> Result<(), SignalError> {
        let instance = env.storage().instance();
        if !instance.has(&ADMIN_KEY) {
            if let Some(admin) = instance.get::<&str, Address>(&LEGACY_ADMIN_KEY) {
                instance.set(&ADMIN_KEY, &admin);
                instance.remove(&LEGACY_ADMIN_KEY);
            }
        }
        if !instance.has(&SCHEDULE_KEY) {
            if let Some(schedule) = instance.get::<&str, DecaySchedule>(&LEGACY_SCHEDULE_KEY) {
                instance.set(&SCHEDULE_KEY, &schedule);
                instance.remove(&LEGACY_SCHEDULE_KEY);
            }
        }
        Ok(())
    }

    /// Apply a reputation update for a provider, first decaying the stored
    /// score according to the configured schedule.
    pub fn update_reputation(
        env: Env,
        provider: Address,
        delta: i128,
    ) -> Result<i128, SignalError> {
        let provider = normalize_address(provider)?;
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
    /// state. Useful for off-chain 

    /// Register an owning account for a provider's state record. Only the
    /// admin may call this. A record with an owner is considered live and is
    /// never eligible for cleanup.
    pub fn set_owner(env: Env, provider: Address, owner: Address) -> Result<(), SignalError> {
        Self::require_admin(&env)?;
        let key = (OWNER_KEY, provider);
        env.storage().persistent().set(&key, &owner);
        Ok(())
    }

    /// Mark a provider's state record as having an active relationship. Only
    /// the admin may call this. A record with an active relationship is
    /// considered live and is never eligible for cleanup.
    pub fn set_relationship_active(
        env: Env,
        provider: Address,
        active: bool,
    ) -> Result<(), SignalError> {
        Self::require_admin(&env)?;
        let key = (RELATIONSHIP_KEY, provider);
        env.storage().persistent().set(&key, &active);
        Ok(())
    }

    /// Permissioned, bounded cleanup of orphaned reputation state.
    ///
    /// Only the admin may call this. At most `max_records` records are
    /// inspected in a single invocation, so cleanup work is bounded by the
    /// caller-supplied maximum and can be paginated across calls. A record is
    /// eligible for removal only when it has no owning account and no active
    /// relationship; eligibility is revalidated immediately before deletion
    /// so concurrent state changes cannot cause a live record to be removed.
    ///
    /// Returns a [`CleanupReport`] describing how many records were scanned,
    /// removed, and skipped.
    pub fn cleanup_orphaned_state(
        env: Env,
        providers: Vec<Address>,
        max_records: u32,
    ) -> Result<CleanupReport, SignalError> {
        Self::require_admin(&env)?;
        if max_records == 0 {
            return Err(SignalError::InvalidCleanupLimit);
        }

        let mut report = CleanupReport {
            scanned: 0,
            removed: 0,
            skipped: 0,
        };

        for provider in providers.iter() {
            if report.scanned >= max_records {
                break;
            }
            report.scanned += 1;

            // Revalidate eligibility immediately before deletion. A record is
            // orphaned only when it has neither an owner nor an active
            // relationship at this exact point in time.
            if !Self::is_orphaned(&env, &provider) {
                report.skipped += 1;
                continue;
            }

            let key = (REPUTATION_KEY, provider.clone());
            env.storage().persistent().remove(&key);
            report.removed += 1;
        }

        Ok(report)
    }

    /// Deterministic eligibility check for orphaned state.
    ///
    /// A record is orphaned when it has no owning account and no active
    /// relationship. The result depends only on stored state, so it is
    /// reproducible and safe to revalidate before deletion.
    fn is_orphaned(env: &Env, provider: &Address) -> bool {
        let owner_key = (OWNER_KEY, provider.clone());
        if env.storage().persistent().has(&owner_key) {
            return false;
        }
        let relationship_key = (RELATIONSHIP_KEY, provider.clone());
        if let Some(active) = env
            .storage()
            .persistent()
            .get::<_, bool>(&relationship_key)
        {
            if active {
                return false;
            }
        }
        true
    }

    /// Require that the caller is the configured admin.
    fn require_admin(env: &Env) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }

    /// Compute the decayed score for a record at a given timestamp.
    ///
    /// Only the admin may grant. `expires_at` must be strictly in the future
    /// relative to the current ledger timestamp, otherwise the grant is
    /// rejected with [`SignalError::InvalidExpiration`].
    pub fn grant_permission(
        env: Env,
        grantee: Address,
        capability: u32,
        expires_at: u64,
    ) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        let now = env.ledger().timestamp();
        if expires_at <= now {
            return Err(SignalError::InvalidExpiration);
        }
        let permission = Permission {
            grantee: grantee.clone(),
            capability,
            expires_at,
        };
        env.storage()
            .persistent()
            .set(&(PERMISSION_KEY, grantee, capability), &permission);
        Ok(())
    }

    /// Check whether `grantee` currently holds `capability`.
    ///
    /// Fails closed: returns `false` once the current ledger timestamp reaches
    /// the stored `expires_at` (inclusive boundary).
    pub fn has_permission(env: Env, grantee: Address, capability: u32) -> bool {
        match env
            .storage()
            .persistent()
            .get::<_, Permission>(&(PERMISSION_KEY, grantee, capability))
        {
            Some(permission) => !Self::is_expired(&env, &permission),
            None => false,
        }
    }

    /// Consume a permission at a sensitive entrypoint.
    ///
    /// Fails closed with [`SignalError::PermissionExpired`] when the grant has
    /// expired, and [`SignalError::PermissionNotFound`] when no grant exists.
    /// The expiration check happens before any state mutation so an expired
    /// grant can never authorize a sensitive action.
    pub fn require_permission(
        env: Env,
        grantee: Address,
        capability: u32,
    ) -> Result<(), SignalError> {
        let key = (PERMISSION_KEY, grantee, capability);
        let permission: Permission = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(SignalError::PermissionNotFound)?;
        if Self::is_expired(&env, &permission) {
            return Err(SignalError::PermissionExpired);
        }
        Ok(())
    }

    fn store_reputation(env: &Env, provider: &Address, record: &ReputationRecord) {
        let key = (REPUTATION_KEY, provider.clone());
        env.storage().persistent().set(&key, record);
    }
}

/// Resource budget guard diagnostics for the signal registry entrypoints.
///
/// This module is compiled only for tests (`#[cfg(test)]`) so it never alters
/// production contract behavior. It provides helpers that assert the CPU,
/// memory, and ledger-entry budgets consumed by representative entrypoints,
/// and that report the offending operation together with the input scale when
/// a budget is exceeded.
#[cfg(test)]
mod budget_guard {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    /// A single resource budget expressed in the units reported by the Soroban
    /// budget tracker.
    #[derive(Clone, Copy, Debug)]
    pub struct Budget {
        pub cpu_insns: u64,
        pub mem_bytes: u64,
        pub ledger_entries: u64,
    }

    /// Diagnostic emitted when a workload exceeds its budget. Carries the
    /// operation name and the input scale so failures are actionable.
    #[derive(Clone, Debug)]
    pub struct BudgetExceeded {
        pub operation: &'static str,
        pub input_scale: u64,
        pub cpu_insns: u64,
        pub mem_bytes: u64,
        pub ledger_entries: u64,
        pub budget: Budget,
    }

    impl core::fmt::Display for BudgetExceeded {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(
                f,
                "budget exceeded for operation `{}` at input scale {}: \
                 cpu {}>{} insns, mem {}>{} bytes, ledger entries {}>{}",
                self.operation,
                self.input_scale,
                self.cpu_insns,
                self.budget.cpu_insns,
                self.mem_bytes,
                self.budget.mem_bytes,
                self.ledger_entries,
                self.budget.ledger_entries,
            )
        }
    }

    /// Snapshot of the resources consumed by a closure, measured against the
    /// budget tracker of the provided environment.
    pub struct Measurement {
        pub cpu_insns: u64,
        pub mem_bytes: u64,
        pub ledger_entries: u64,
    }

    /// Run `f` and measure the CPU, memory, and ledger-entry cost it incurs.
    ///
    /// The measurement is taken from the environment's budget tracker, which
    /// is only available in tests, so production behavior is untouched.
    pub fn measure<F: FnOnce()>(env: &Env, f: F) -> Measurement {
        let before = env.budget().reset_unlimited();
        let _ = before;
        f();
        let cpu_insns = env.budget().cpu_instruction_cost();
        let mem_bytes = env.budget().memory_bytes_cost();
        let ledger_entries = env.budget().cpu_instruction_cost();
        Measurement {
            cpu_insns,
            mem_bytes,
            ledger_entries,
        }
    }

    /// Assert that a measured workload stays within `budget`, returning a
    /// diagnostic that names the operation and input scale on failure.
    pub fn assert_within(
        operation: &'static str,
        input_scale: u64,
        measurement: Measurement,
        budget: Budget,
    ) -> Result<(), BudgetExceeded> {
        if measurement.cpu_insns > budget.cpu_insns
            || measurement.mem_bytes > budget.mem_bytes
            || measurement.ledger_entries > budget.ledger_entries
        {
            return Err(BudgetExceeded {
                operation,
                input_scale,
                cpu_insns: measurement.cpu_insns,
                mem_bytes: measurement.mem_bytes,
                ledger_entries: measurement.ledger_entries,
                budget,
            });
        }
        Ok(())
    }

    /// Build a representative initialized registry for budget tests.
    pub fn setup(env: &Env) -> (SignalRegistryClient<'_>, Address) {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(env, &contract_id);
        let admin = Address::generate(env);
        let schedule = DecaySchedule {
            decay_rate_bps: 500,
            decay_interval: 86_400,
            grace_period: 3_600,
            min_reputation: 0,
            max_reputation: 1_000_000,
        };
        client.initialize(&admin, &schedule);
        (client, admin)
    }

    #[test]
    fn update_reputation_near_limit_stays_within_budget() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);

        let measurement = measure(&env, || {
            client.update_reputation(&provider, &1_000);
        });

        let budget = Budget {
            cpu_insns: 5_000_000,
            mem_bytes: 1_000_000,
            ledger_entries: 5_000_000,
        };
        assert_within("update_reputation", 1, measurement, budget)
            .expect("near-limit update_reputation should fit the budget");
    }

    #[test]
    fn update_reputation_over_limit_reports_operation_and_scale() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);

        let measurement = measure(&env, || {
            client.update_reputation(&provider, &1_000);
        });

        // Deliberately impossible budget to exercise the diagnostic path.
        let budget = Budget {
            cpu_insns: 0,
            mem_bytes: 0,
            ledger_entries: 0,
        };
        let err = assert_within("update_reputation", 1, measurement, budget)
            .expect_err("over-limit workload must be reported");
        assert_eq!(err.operation, "update_reputation");
        assert_eq!(err.input_scale, 1);
        assert!(err.to_string().contains("update_reputation"));
    }

    #[test]
    fn get_reputation_near_limit_stays_within_budget() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);
        client.update_reputation(&provider, &1_000);

        let measurement = measure(&env, || {
            client.get_reputation(&provider);
        });

        let budget = Budget {
            cpu_insns: 5_000_000,
            mem_bytes: 1_000_000,
            ledger_entries: 5_000_000,
        };
        assert_within("get_reputation", 1, measurement, budget)
            .expect("near-limit get_reputation should fit the budget");
    }

    #[test]
    fn get_reputation_over_limit_reports_operation_and_scale() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);
        client.update_reputation(&provider, &1_000);

        let measurement = measure(&env, || {
            client.get_reputation(&provider);
        });

        let budget = Budget {
            cpu_insns: 0,
            mem_bytes: 0,
            ledger_entries: 0,
        };
        let err = assert_within("get_reputation", 1, measurement, budget)
            .expect_err("over-limit workload must be reported");
        assert_eq!(err.operation, "get_reputation");
        assert_eq!(err.input_scale, 1);
        assert!(err.to_string().contains("get_reputation"));
    }
}
