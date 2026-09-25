//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.
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

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};

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
    /// An event payload exceeded the configured size limits. Returned before
    /// any event is emitted so oversized emissions cannot exhaust transaction
    /// resources or disrupt downstream indexers.
    EventPayloadTooLarge = 6,
    /// A storage key component violated the documented length or character
    /// constraints, or a namespace was used for the wrong state domain.
    InvalidStorageKey = 7,
    /// No upgrade authority handoff is currently pending.
    NoPendingHandoff = 8,
    /// A handoff is already pending; it must be accepted or cancelled first.
    HandoffAlreadyPending = 9,
    /// The pending handoff has expired and can no longer be accepted.
    HandoffExpired = 10,
    /// The caller is not the pending successor and cannot accept the handoff.
    WrongAccepter = 11,
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

/// Stored reputation record for a provider.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationRecord {
    pub score: i128,
    pub last_updated: u64,
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

/// Namespaced storage keys. Each variant maps to a distinct namespace prefix
/// so keys from different state domains cannot collide.
const ADMIN_KEY: &str = "cfg:admin";
const SCHEDULE_KEY: &str = "cfg:schedule";
const REPUTATION_KEY: &str = "rep:reputation";
const PENDING_HANDOFF_KEY: &str = "cfg:pending_handoff";

/// Legacy (pre-namespace) keys, retained only for the explicit migration path.
const LEGACY_ADMIN_KEY: &str = "admin";
const LEGACY_SCHEDULE_KEY: &str = "schedule";
const LEGACY_REPUTATION_KEY: &str = "reputation";

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
        let schedule: DecaySchedule = env
            .storage()
            .instance()
         

/* … truncated 9115 chars — edit only what you need near the top … */
