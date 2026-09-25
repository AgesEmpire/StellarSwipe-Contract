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
    /// The supplied address is malformed or of an unsupported type.
    InvalidAddress = 7,
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
    pub fn initialize(env: Env, admin: Address, schedule: DecaySchedule) -> Result<(), SignalError> {
        let admin = normalize_address(admin)?;
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
        let provider = normalize_address(provider)?;
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

/* … truncated 4131 chars — edit only what you need near the top … */
