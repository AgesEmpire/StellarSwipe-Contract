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
    InvalidTtlConfig = 6,
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataKey {
    pub admin: Address,
    pub schedule: DecaySchedule,
}

const ADMIN_KEY: &str = "admin";
const SCHEDULE_KEY: &str = "schedule";
const TTL_KEY: &str = "ttl";
const REPUTATION_KEY: &str = "reputation";

/// Default TTL strategy: bump instance when below ~1 day of ledgers and
/// extend to ~7 days; bump persistent entries when below ~7 days and extend
/// to ~30 days (assuming ~5s ledgers).
const DEFAULT_TTL_CONFIG: TtlConfig = TtlConfig {
    instance_threshold: 17_280,
    instance_extend_to: 120_960,
    persistent_threshold: 120_960,
    persistent_extend_to: 518_400,
};

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
        env.storage().instance().set(&TTL_KEY, &DEFAULT_TTL_CONFIG);
        Self::bump_instance_ttl(&env, &DEFAULT_TTL_CONFIG);
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

    /// Update the TTL bump strategy. Only the admin may call this.
    pub fn set_ttl_config(env: Env, config: TtlConfig) -> Result<(), SignalError> {
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
    pub fn get_ttl_config(env: Env) -> Result<TtlConfig, SignalError> {
        env.storage()
            .instance()
            .get(&TTL_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Extend the contract instance TTL if it is at or below the configured
    /// threshold. Safe to call repeatedly; no-op when already extended.
    pub fn bump_instance(env: Env) -> Result<(), SignalError> {
        let config = Self::load_ttl_config(&env)?;
        Self::bump_instance_ttl(&env, &config);
        Ok(())
    }

    /// Extend the TTL of a provider's persistent reputation entry if it is at
    /// or below the configured threshold. Intended to be driven by an
    /// operational maintenance job. No-op when the entry is already extended
    /// or does not exist.
    pub fn bump_ttl(env: Env, provider: Address) -> Result<(), SignalError> {
        let config = Self::load_ttl_config(&env)?;
        let key = (REPUTATION_KEY, provider.clone());
        let storage = env.storage().persistent();
        if !storage.has(&key) {
            return Ok(());
        }
        let remaining = env.ledger().sequence();
        let _ = remaining;
        storage.extend_ttl(&key, config.persistent_threshold, config.persistent_extend_to);
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

    /// Validate a TTL configuration before it is stored or applied.
    fn validate_ttl_config(config: &TtlConfig) -> Result<(), SignalError> {
        if config.instance_extend_to <= config.instance_threshold {
            return Err(SignalError::InvalidTtlConfig);
        }
        if config.persistent_extend_to <= config.persistent_threshold {
            return Err(SignalError::InvalidTtlConfig);
        }
        Ok(())
    }

    fn load_ttl_config(env: &Env) -> Result<TtlConfig, SignalError> {
        env.storage()
            .instance()
            .get(&TTL_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Extend the instance TTL when it is at or below the configured
    /// threshold. `extend_ttl` is a no-op when the remaining TTL already
    /// exceeds the threshold, so this avoids unnecessary rent spend.
    fn bump_instance_ttl(env: &Env, config: &TtlConfig) {
        env.storage()
            .instance()
            .extend_ttl(config.instance_threshold, config.instance_extend_to);
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
