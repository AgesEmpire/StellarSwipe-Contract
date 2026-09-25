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
    InvalidCleanupLimit = 6,
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

const ADMIN_KEY: &str = "admin";
const SCHEDULE_KEY: &str = "schedule";
const REPUTATION_KEY: &str = "reputation";
const OWNER_KEY: &str = "owner";
const RELATIONSHIP_KEY: &str = "relationship";

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

    fn store_reputation(env: &Env, provider: &Address, record: &ReputationRecord) {
        let key = (REPUTATION_KEY, provider.clone());
        env.storage().persistent().set(&key, record);
    }
}
