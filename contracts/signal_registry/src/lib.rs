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

    /// Emit a bounded event payload at the contract boundary.
    ///
    /// The `label` (metadata), `message` (string), and `tags` (vector) are all
    /// validated against the configured size limits before any event is
    /// emitted. If any component is oversized the call fails with
    /// [`SignalError::EventPayloadTooLarge`] and no event is produced.
    pub fn emit_event(
        env: Env,
        label: String,
        message: String,
        tags: Vec<String>,
    ) -> Result<(), SignalError> {
        Self::validate_event_payload(&label, &message, &tags)?;
        env.events().publish((label,), (message, tags));
        Ok(())
    }

    /// Validate an event payload against the configured size limits.
    ///
    /// Returns [`SignalError::EventPayloadTooLarge`] if the metadata label,
    /// string message, or tag vector exceeds its respective maximum. Limits
    /// are inclusive: a value exactly at the limit is accepted, one over is
    /// rejected.
    fn validate_event_payload(
        label: &String,
        message: &String,
        tags: &Vec<String>,
    ) -> Result<(), SignalError> {
        if label.len() > MAX_EVENT_METADATA_LEN {
            return Err(SignalError::EventPayloadTooLarge);
        }
        if message.len() > MAX_EVENT_STRING_LEN {
            return Err(SignalError::EventPayloadTooLarge);
        }
        if tags.len() > MAX_EVENT_VECTOR_LEN {
            return Err(SignalError::EventPayloadTooLarge);
        }
        Ok(())
    }

    /// Validate a user-controlled key component against the documented
    /// constraints, returning [`SignalError::InvalidStorageKey`] on failure.
    fn validate_key_component(component: &str) -> Result<(), SignalError> {
        if is_valid_key_component(component) {
            Ok(())
        } else {
            Err(SignalError::InvalidStorageKey)
        }
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
        record.score.saturating_sub(lost)
    }

    /// Clamp a score to the configured reputation bounds.
    fn clamp(score: i128, schedule: &DecaySchedule) -> i128 {
        if score < schedule.min_reputation {
            schedule.min_reputation
        } else if score > schedule.max_reputation {
            schedule.max_reputation
        } else {
            score
        }
    }

    /// Validate a decay schedule.
    fn validate_schedule(schedule: &DecaySchedule) -> Result<(), SignalError> {
        if schedule.min_reputation > schedule.max_reputation {
            return Err(SignalError::InvalidDecayConfig);
        }
        if schedule.decay_rate_bps > 10_000 {
            return Err(SignalError::InvalidDecayConfig);
        }
        Ok(())
    }

    /// Load a provider's reputation record, defaulting to a zeroed record.
    fn load_reputation(env: &Env, provider: &Address) -> ReputationRecord {
        env.storage()
            .persistent()
            .get(&(REPUTATION_KEY, provider.clone()))
            .unwrap_or(ReputationRecord {
                score: 0,
                last_updated: 0,
            })
    }

    /// Persist a provider's reputation record.
    fn store_reputation(env: &Env, provider: &Address, record: &ReputationRecord) {
        env.storage()
            .persistent()
            .set(&(REPUTATION_KEY, provider.clone()), record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{vec, Env, String};

    fn schedule() -> DecaySchedule {
        DecaySchedule {
            decay_rate_bps: 100,
            decay_interval: 100,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 1_000,
        }
    }

    #[test]
    fn valid_key_components_are_accepted() {
        assert!(is_valid_key_component("provider-1"));
        assert!(is_valid_key_component("a.b_c-9"));
        assert!(is_valid_key_component(&"x".repeat(MAX_KEY_COMPONENT_LEN as usize)));
    }

    #[test]
    fn oversized_key_component_is_rejected() {
        let oversized = "x".repeat(MAX_KEY_COMPONENT_LEN as usize + 1);
        assert!(!is_valid_key_component(&oversized));
        assert_eq!(
            SignalRegistry::validate_key_component(&oversized),
            Err(SignalError::InvalidStorageKey)
        );
    }

    #[test]
    fn empty_and_invalid_charset_components_are_rejected() {
        assert!(!is_valid_key_component(""));
        assert!(!is_valid_key_component("has space"));
        assert!(!is_valid_key_component("colon:injected"));
        assert_eq!(
            SignalRegistry::validate_key_component("bad/key"),
            Err(SignalError::InvalidStorageKey)
        );
    }

    #[test]
    fn namespaces_are_distinct_per_domain() {
        assert_ne!(StorageNamespace::Config.prefix(), StorageNamespace::Reputation.prefix());
        assert!(ADMIN_KEY.starts_with(StorageNamespace::Config.prefix()));
        assert!(SCHEDULE_KEY.starts_with(StorageNamespace::Config.prefix()));
        assert!(REPUTATION_KEY.starts_with(StorageNamespace::Reputation.prefix()));
    }

    #[test]
    fn namespaced_keys_do_not_collide_with_legacy_keys() {
        assert_ne!(ADMIN_KEY, LEGACY_ADMIN_KEY);
        assert_ne!(SCHEDULE_KEY, LEGACY_SCHEDULE_KEY);
        assert_ne!(REPUTATION_KEY, LEGACY_REPUTATION_KEY);
    }

    #[test]
    fn legacy_state_is_migrated_to_namespaced_keys() {
        let env = Env::default();
        let admin = Address::generate(&env);
        env.storage().instance().set(&LEGACY_ADMIN_KEY, &admin);
        env.storage().instance().set(&LEGACY_SCHEDULE_KEY, &schedule());

        SignalRegistry::migrate_legacy_keys(env.clone()).unwrap();

        assert!(env.storage().instance().has(&ADMIN_KEY));
        assert!(env.storage().instance().has(&SCHEDULE_KEY));
        assert!(!env.storage().instance().has(&LEGACY_ADMIN_KEY));
        assert!(!env.storage().instance().has(&LEGACY_SCHEDULE_KEY));
    }

    #[test]
    fn reputation_round_trips_under_namespaced_key() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        SignalRegistry::initialize(env.clone(), admin, schedule()).unwrap();
        SignalRegistry::update_reputation(env.clone(), provider.clone(), 500).unwrap();
        assert_eq!(SignalRegistry::get_reputation(env, provider).unwrap(), 500);
    }

    #[test]
    fn oversized_event_payload_is_rejected() {
        let env = Env::default();
        let label = String::from_str(&env, "label");
        let message = String::from_str(&env, &"m".repeat(MAX_EVENT_STRING_LEN as usize + 1));
        let tags = vec![&env];
        assert_eq!(
            SignalRegistry::emit_event(env, label, message, tags),
            Err(SignalError::EventPayloadTooLarge)
        );
    }
}
