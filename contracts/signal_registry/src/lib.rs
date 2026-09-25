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

    fn setup() -> (Env, SignalRegistryClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin, &schedule());
        (env, client)
    }

    #[test]
    fn boundary_depth_is_accepted() {
        let (env, client) = setup();
        let provider = Address::generate(&env);
        // Depth exactly at the documented maximum is allowed.
        assert_eq!(
            client.update_reputation(&provider, &10, &MAX_AUTH_TREE_DEPTH),
            10
        );
        assert_eq!(client.get_reputation(&provider, &MAX_AUTH_TREE_DEPTH), 10);
    }

    #[test]
    fn over_limit_depth_is_rejected() {
        let (env, client) = setup();
        let provider = Address::generate(&env);
        let too_deep = MAX_AUTH_TREE_DEPTH + 1;
        assert_eq!(
            client.try_update_reputation(&provider, &10, &too_deep),
            Err(Ok(SignalError::AuthTreeTooDeep))
        );
        assert_eq!(
            client.try_get_reputation(&provider, &too_deep),
            Err(Ok(SignalError::AuthTreeTooDeep))
        );
    }

    #[test]
    fn guard_applies_to_every_tree_consuming_entrypoint() {
        let (env, client) = setup();
        let provider = Address::generate(&env);
        let too_deep = MAX_AUTH_TREE_DEPTH + 1;
        // Both tree-consuming entrypoints must reject over-limit trees with
        // the same stable error before touching state.
        assert_eq!(
            client.try_update_reputation(&provider, &1, &too_deep),
            Err(Ok(SignalError::AuthTreeTooDeep))
        );
        assert_eq!(
            client.try_get_reputation(&provider, &too_deep),
            Err(Ok(SignalError::AuthTreeTooDeep))
        );
    }
}
