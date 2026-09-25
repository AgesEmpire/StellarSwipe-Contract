//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.
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
    PermissionExpired = 6,
    PermissionNotFound = 7,
    InvalidExpiration = 8,
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataKey {
    pub admin: Address,
    pub schedule: DecaySchedule,
}

const ADMIN_KEY: &str = "admin";
const SCHEDULE_KEY: &str = "schedule";
const REPUTATION_KEY: &str = "reputation";
const VERSION_KEY: &str = "version";
const PERMISSION_KEY: &str = "permission";

/// Current storage schema version written by `initialize`.
const STORAGE_VERSION: u32 = 1;

/// Upper bound on how many expired permissions a single cleanup call removes.
/// Keeps expiration cleanup bounded and gas-predictable.
pub const MAX_EXPIRATION_SWEEP: u32 = 32;

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
        if Self::is_initialized(&env) {
            return Err(SignalError::AlreadyInitialized);
        }
        Self::validate_schedule(&schedule)?;
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&SCHEDULE_KEY, &schedule);
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

    /// Grant a time-bounded permission to `grantee`.
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
        let permission: Permission = env
            .storage()
            .persistent()
            .get(&(PERMISSION_KEY, grantee, capability))
            .ok_or(SignalError::PermissionNotFound)?;
        if Self::is_expired(&env, &permission) {
            return Err(SignalError::PermissionExpired);
        }
        Ok(())
    }

    /// Remove up to [`MAX_EXPIRATION_SWEEP`] expired permissions for `grantee`.
    ///
    /// Bounded cleanup: only already-expired grants are removed, so live
    /// permissions and authorization semantics are unchanged. Returns the
    /// number of entries removed.
    pub fn sweep_expired_permissions(
        env: Env,
        grantee: Address,
        capabilities: Vec<u32>,
    ) -> Result<u32, SignalError> {
        let mut removed: u32 = 0;
        for capability in capabilities.iter() {
            if removed >= MAX_EXPIRATION_SWEEP {
                break;
            }
            let key = (PERMISSION_KEY, grantee.clone(), capability);
            if let Some(permission) = env.storage().persistent().get::<_, Permission>(&key) {
                if Self::is_expired(&env, &permission) {
                    env.storage().persistent().remove(&key);
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Returns `true` when the permission has expired at the current ledger
    /// timestamp. The boundary is inclusive: `now >= expires_at` is expired.
    fn is_expired(env: &Env, permission: &Permission) -> bool {
        env.ledger().timestamp() >= permission.expires_at
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

    /// Validate a decay schedule before it is stored.
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
