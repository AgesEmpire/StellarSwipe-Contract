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
}

/// Machine-readable ABI entry describing a single public contract error.
///
/// Consumers (clients, SDKs, indexers) can map numeric error codes to stable
/// identifiers and the entrypoints that may return them without parsing
/// human-readable display strings.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorAbiEntry {
    /// Stable identifier matching the Rust variant name.
    pub name: Symbol,
    /// Numeric value returned on-chain for this error.
    pub code: u32,
    /// Entrypoints that may surface this error.
    pub entrypoints: Vec<Symbol>,
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

const ADMIN_KEY: &str = "admin";
const SCHEDULE_KEY: &str = "schedule";
const REPUTATION_KEY: &str = "reputation";

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

    /// Export the machine-readable error ABI for this contract.
    ///
    /// The artifact is derived from the Rust source of truth: every public
    /// error variant is listed with its numeric value and the entrypoints that
    /// may return it. Consumers can map errors without parsing display strings.
    pub fn error_abi(env: Env) -> Vec<ErrorAbiEntry> {
        let mut entries = Vec::new(&env);

        let mut already_initialized = Vec::new(&env);
        already_initialized.push_back(Symbol::new(&env, "initialize"));
        entries.push_back(ErrorAbiEntry {
            name: Symbol::new(&env, "AlreadyInitialized"),
            code: SignalError::AlreadyInitialized as u32,
            entrypoints: already_initialized,
        });

        let mut not_initialized = Vec::new(&env);
        not_initialized.push_back(Symbol::new(&env, "set_decay_schedule"));
        not_initialized.push_back(Symbol::new(&env, "get_decay_schedule"));
        not_initialized.push_back(Symbol::new(&env, "update_reputation"));
        not_initialized.push_back(Symbol::new(&env, "get_reputation"));
        entries.push_back(ErrorAbiEntry {
            name: Symbol::new(&env, "NotInitialized"),
            code: SignalError::NotInitialized as u32,
            entrypoints: not_initialized,
        });

        let mut unauthorized = Vec::new(&env);
        unauthorized.push_back(Symbol::new(&env, "set_decay_schedule"));
        entries.push_back(ErrorAbiEntry {
            name: Symbol::new(&env, "Unauthorized"),
            code: SignalError::Unauthorized as u32,
            entrypoints: unauthorized,
        });

        let mut invalid_decay_config = Vec::new(&env);
        invalid_decay_config.push_back(Symbol::new(&env, "initialize"));
        invalid_decay_config.push_back(Symbol::new(&env, "set_decay_schedule"));
        entries.push_back(ErrorAbiEntry {
            name: Symbol::new(&env, "InvalidDecayConfig"),
            code: SignalError::InvalidDecayConfig as u32,
            entrypoints: invalid_decay_config,
        });

        let mut invalid_reputation_update = Vec::new(&env);
        invalid_reputation_update.push_back(Symbol::new(&env, "update_reputation"));
        entries.push_back(ErrorAbiEntry {
            name: Symbol::new(&env, "InvalidReputationUpdate"),
            code: SignalError::InvalidReputationUpdate as u32,
            entrypoints: invalid_reputation_update,
        });

        entries
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

#[cfg(test)]
mod auth_negative_tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Env, IntoVal, Symbol, TryFromVal, Val};

    fn schedule() -> DecaySchedule {
        DecaySchedule {
            decay_rate_bps: 100,
            decay_interval: 100,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 10_000,
        }
    }

    fn setup() -> (Env, SignalRegistryClient<'static>, Address) {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &contract_id);
        env.mock_all_auths();
        client.initialize(&admin, &sche

/* … truncated 5720 chars — edit only what you need near the top … */
