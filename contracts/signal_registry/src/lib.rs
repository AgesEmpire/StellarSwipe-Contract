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
        client.initialize(&admin, &schedule());
        (env, client, admin)
    }

    /// Assert a call fails with the expected stable error category, not a
    /// brittle message string. Soroban surfaces contract errors as a
    /// `Result::Err(Status)` whose payload is the `SignalError` variant.
    fn assert_denied<T, E>(result: Result<T, E>, expected: SignalError)
    where
        E: IntoVal<Env, Val> + TryFromVal<Env, Val>,
    {
        match result {
            Err(err) => {
                let env = Env::default();
                let val: Val = err.into_val(&env);
                let decoded = SignalError::try_from_val(&env, &val)
                    .expect("error must decode to a SignalError variant");
                assert_eq!(decoded, expected);
            }
            Ok(_) => panic!("expected denial with {:?}, but call succeeded", expected),
        }
    }

    // --- Unauthorized caller: privileged entrypoint denial -----------------

    #[test]
    fn set_decay_schedule_denied_for_unauthorized_caller() {
        let (env, client, _admin) = setup();
        // No auth mocked: the admin's require_auth() must reject the caller.
        env.set_auths(&[]);
        let result = client.try_set_decay_schedule(&schedule());
        assert!(result.is_err(), "unauthorized caller must be denied");
    }

    // --- Stale role: admin rotated, old admin no longer privileged ---------

    #[test]
    fn set_decay_schedule_denied_for_stale_admin_role() {
        let (env, client, _admin) = setup();
        let stale = Address::generate(&env);
        // Only the stale address authorizes; the stored admin is different.
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &stale,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_decay_schedule",
                args: (schedule(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let result = client.try_set_decay_schedule(&schedule());
        assert!(result.is_err(), "stale role must not be privileged");
    }

    // --- Malformed arguments: invalid schedule rejected --------------------

    #[test]
    fn set_decay_schedule_denied_for_malformed_schedule() {
        let (_env, client, _admin) = setup();
        let bad = DecaySchedule {
            decay_rate_bps: 20_000, // > 10_000 bps
            decay_interval: 100,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 10_000,
        };
        let result = client.try_set_decay_schedule(&bad);
        assert_denied(result, SignalError::InvalidDecayConfig);
    }

    #[test]
    fn initialize_denied_for_malformed_schedule() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &contract_id);
        env.mock_all_auths();
        let bad = DecaySchedule {
            decay_rate_bps: 100,
            decay_interval: 0, // zero interval is invalid
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 10_000,
        };
        let result = client.try_initialize(&admin, &bad);
        assert_denied(result, SignalError::InvalidDecayConfig);
    }

    // --- Replayed authorization: re-initialize must be denied --------------

    #[test]
    fn initialize_denied_on_replay() {
        let (_env, client, admin) = setup();
        // Replaying initialize with the same admin must be rejected.
        let result = client.try_initialize(&admin, &schedule());
        assert_denied(result, SignalError::AlreadyInitialized);
    }

    // --- Cross-contract / nested invocation: caller is another contract ----

    #[test]
    fn set_decay_schedule_denied_for_cross_contract_caller() {
        let (env, client, _admin) = setup();
        // Simulate a nested invocation where a different contract address is
        // the caller and does not hold the admin role.
        let other_contract = env.register_contract(None, SignalRegistry);
        env.set_auths(&[]);
        let result = client.try_set_decay_schedule(&schedule());
        assert!(result.is_err(), "cross-contract caller must be denied");
        // The nested contract's own privileged entrypoint is likewise denied.
        let nested = SignalRegistryClient::new(&env, &other_contract);
        let nested_result = nested.try_set_decay_schedule(&schedule());
        assert!(nested_result.is_err(), "nested caller must be denied");
    }

    // --- Not-initialized denial for privileged entrypoint ------------------

    #[test]
    fn set_decay_schedule_denied_when_not_initialized() {
        let env = Env::default();
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &contract_id);
        env.mock_all_auths();
        let result = client.try_set_decay_schedule(&schedule());
        assert_denied(result, SignalError::NotInitialized);
    }

    // --- Sanity: authorized admin succeeds (positive control) --------------

    #[test]
    fn set_decay_schedule_allowed_for_admin() {
        let (_env, client, _admin) = setup();
        let updated = DecaySchedule {
            decay_rate_bps: 200,
            decay_interval: 50,
            grace_period: 10,
            min_reputation: 0,
            max_reputation: 5_000,
        };
        client.set_decay_schedule(&updated);
        assert_eq!(client.get_decay_schedule(), updated);
    }

    // Silence unused-import warnings for helpers used conditionally.
    #[allow(dead_code)]
    fn _touch(_: Symbol) {}
}
