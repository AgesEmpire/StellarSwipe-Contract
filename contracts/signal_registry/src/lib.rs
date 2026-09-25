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
    AuthTreeTooDeep = 6,
}

/// Maximum depth of an authorization tree accepted by tree-consuming
/// entrypoints. Trees deeper than this are rejected before any recursive
/// traversal begins, bounding execution resources and keeping validation
/// behavior consistent across entrypoints.
///
/// The limit is inclusive: a tree whose depth equals `MAX_AUTH_TREE_DEPTH` is
/// accepted, while a tree one level deeper returns `AuthTreeTooDeep`.
pub const MAX_AUTH_TREE_DEPTH: u32 = 8;

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

/// The kind of contract operation whose storage footprint is being estimated.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OperationKind {
    /// A brand new reputation record is written for a provider.
    NewRecord = 0,
    /// An existing reputation record is overwritten in place.
    UpdateRecord = 1,
}

/// Projected storage footprint and rent implications for a contract
/// operation, produced before submission.
///
/// `projected_entries` is the number of persistent storage entries the
/// operation will touch, `projected_bytes` is the serialized byte footprint
/// of those entries, and `ttl_impact` is the number of ledgers the operation
/// extends the relevant entry's time-to-live by. All fields are derived
/// purely from the inputs, so identical inputs always yield identical
/// estimates.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StorageRentEstimate {
    pub projected_entries: u32,
    pub projected_bytes: u32,
    pub ttl_impact: u32,
}

/// Serialized byte footprint of a single reputation record: a 16-byte `i128`
/// score plus an 8-byte `u64` timestamp.
const REPUTATION_RECORD_BYTES: u32 = 24;

/// Ledgers of TTL extension applied per storage write. Matches the default
/// persistent entry lifetime used by the registry.
const TTL_LEDGERS_PER_WRITE: u32 = 100;

/// Upper bound on the number of entries a single operation may project.
/// Requests that would exceed this are rejected as near-limit requests.
pub const MAX_PROJECTED_ENTRIES: u32 = 64;

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

    /// Estimate the storage footprint and rent implications of a contract
    /// operation before it is submitted.
    ///
    /// `kind` selects the operation being projected and `record_count` is the
    /// number of reputation records the operation will touch. The estimate is
    /// deterministic: it depends only on `kind` and `record_count`, so
    /// identical inputs always produce identical results. Requests that would
    /// project more than `MAX_PROJECTED_ENTRIES` entries are rejected.
    pub fn estimate_storage_rent(
        kind: OperationKind,
        record_count: u32,
    ) -> Result<StorageRentEstimate, SignalError> {
        if record_count == 0 || record_count > MAX_PROJECTED_ENTRIES {
            return Err(SignalError::InvalidReputationUpdate);
        }

        // New records allocate a fresh entry; updates overwrite an existing
        // one, so both touch exactly one entry per record.
        let projected_entries = record_count;
        let projected_bytes = record_count.saturating_mul(REPUTATION_RECORD_BYTES);
        let ttl_impact = match kind {
            OperationKind::NewRecord => record_count.saturating_mul(TTL_LEDGERS_PER_WRITE),
            OperationKind::UpdateRecord => record_count.saturating_mul(TTL_LEDGERS_PER_WRITE),
        };

        Ok(StorageRentEstimate {
            projected_entries,
            projected_bytes,
            ttl_impact,
        })
    }

    /// Apply a reputation update for a provider, first decaying the stored
    /// score according to the configured schedule.
    ///
    /// `auth_depth` is the depth of the caller's authorization tree. It is
    /// validated against `MAX_AUTH_TREE_DEPTH` before any state is read or
    /// mutated, so over-limit trees fail fast with a stable error.
    pub fn update_reputation(
        env: Env,
        provider: Address,
        delta: i128,
        auth_depth: u32,
    ) -> Result<i128, SignalError> {
        Self::check_auth_depth(auth_depth)?;

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
    ///
    /// `auth_depth` is validated against `MAX_AUTH_TREE_DEPTH` before the
    /// stored record is read.
    pub fn get_reputation(
        env: Env,
        provider: Address,
        auth_depth: u32,
    ) -> Result<i128, SignalError> {
        Self::check_auth_depth(auth_depth)?;

        let schedule: DecaySchedule = env
            .storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)?;
        let record = Self::load_reputation(&env, &provider);
        let now = env.ledger().timestamp();
        Ok(Self::apply_decay(&record, now, &schedule))
    }

    /// Enforce the documented maximum authorization tree depth before any
    /// recursive traversal. Returns `AuthTreeTooDeep` for over-limit trees.
    fn check_auth_depth(depth: u32) -> Result<(), SignalError> {
        if depth > MAX_AUTH_TREE_DEPTH {
            return Err(SignalError::AuthTreeTooDeep);
        }
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
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    fn schedule() -> DecaySchedule {
        DecaySchedule {
            decay_rate_bps: 100,
            decay_interval: 100,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 10_000,
        }
    }

    #[test]
    fn estimate_new_record_is_deterministic() {
        let a = SignalRegistry::estimate_storage_rent(OperationKind::NewRecord, 3).unwrap();
        let b = SignalRegistry::estimate_storage_rent(OperationKind::NewRecord, 3).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.projected_entries, 3);
        assert_eq!(a.projected_bytes, 3 * REPUTATION_RECORD_BYTES);
        assert_eq!(a.ttl_impact, 3 * TTL_LEDGERS_PER_WRITE);
    }

    #[test]
    fn estimate_update_record_reports_ttl_impact() {
        let est = SignalRegistry::estimate_storage_rent(OperationKind::UpdateRecord, 1).unwrap();
        assert_eq!(est.projected_entries, 1);
        assert_eq!(est.projected_bytes, REPUTATION_RECORD_BYTES);
        assert_eq!(est.ttl_impact, TTL_LEDGERS_PER_WRITE);
    }

    #[test]
    fn estimate_near_limit_request_is_accepted() {
        let est =
            SignalRegistry::estimate_storage_rent(OperationKind::NewRecord, MAX_PROJECTED_ENTRIES)
                .unwrap();
        assert_eq!(est.projected_entries, MAX_PROJECTED_ENTRIES);
        assert_eq!(est.projected_bytes, MAX_PROJECTED_ENTRIES * REPUTATION_RECORD_BYTES);
    }

    #[test]
    fn estimate_over_limit_request_is_rejected() {
        let err = SignalRegistry::estimate_storage_rent(
            OperationKind::NewRecord,
            MAX_PROJECTED_ENTRIES + 1,
        );
        assert_eq!(err, Err(SignalError::InvalidReputationUpdate));
    }

    #[test]
    fn estimate_zero_records_is_rejected() {
        let err = SignalRegistry::estimate_storage_rent(OperationKind::UpdateRecord, 0);
        assert_eq!(err, Err(SignalError::InvalidReputationUpdate));
    }

    #[test]
    fn update_reputation_still_works() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        env.mock_all_auths();
        SignalRegistry::initialize(env.clone(), admin, schedule()).unwrap();
        let score = SignalRegistry::update_reputation(env.clone(), provider.clone(), 500, 1).unwrap();
        assert_eq!(score, 500);
        let read = SignalRegistry::get_reputation(env, provider, 1).unwrap();
        assert_eq!(read, 500);
    }
}
