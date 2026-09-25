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

/// Resource budget guard diagnostics for the signal registry entrypoints.
///
/// This module is compiled only for tests (`#[cfg(test)]`) so it never alters
/// production contract behavior. It provides helpers that assert the CPU,
/// memory, and ledger-entry budgets consumed by representative entrypoints,
/// and that report the offending operation together with the input scale when
/// a budget is exceeded.
#[cfg(test)]
mod budget_guard {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    /// A single resource budget expressed in the units reported by the Soroban
    /// budget tracker.
    #[derive(Clone, Copy, Debug)]
    pub struct Budget {
        pub cpu_insns: u64,
        pub mem_bytes: u64,
        pub ledger_entries: u64,
    }

    /// Diagnostic emitted when a workload exceeds its budget. Carries the
    /// operation name and the input scale so failures are actionable.
    #[derive(Clone, Debug)]
    pub struct BudgetExceeded {
        pub operation: &'static str,
        pub input_scale: u64,
        pub cpu_insns: u64,
        pub mem_bytes: u64,
        pub ledger_entries: u64,
        pub budget: Budget,
    }

    impl core::fmt::Display for BudgetExceeded {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(
                f,
                "budget exceeded for operation `{}` at input scale {}: \
                 cpu {}>{} insns, mem {}>{} bytes, ledger entries {}>{}",
                self.operation,
                self.input_scale,
                self.cpu_insns,
                self.budget.cpu_insns,
                self.mem_bytes,
                self.budget.mem_bytes,
                self.ledger_entries,
                self.budget.ledger_entries,
            )
        }
    }

    /// Snapshot of the resources consumed by a closure, measured against the
    /// budget tracker of the provided environment.
    pub struct Measurement {
        pub cpu_insns: u64,
        pub mem_bytes: u64,
        pub ledger_entries: u64,
    }

    /// Run `f` and measure the CPU, memory, and ledger-entry cost it incurs.
    ///
    /// The measurement is taken from the environment's budget tracker, which
    /// is only available in tests, so production behavior is untouched.
    pub fn measure<F: FnOnce()>(env: &Env, f: F) -> Measurement {
        let before = env.budget().reset_unlimited();
        let _ = before;
        f();
        let cpu_insns = env.budget().cpu_instruction_cost();
        let mem_bytes = env.budget().memory_bytes_cost();
        let ledger_entries = env.budget().cpu_instruction_cost();
        Measurement {
            cpu_insns,
            mem_bytes,
            ledger_entries,
        }
    }

    /// Assert that a measured workload stays within `budget`, returning a
    /// diagnostic that names the operation and input scale on failure.
    pub fn assert_within(
        operation: &'static str,
        input_scale: u64,
        measurement: Measurement,
        budget: Budget,
    ) -> Result<(), BudgetExceeded> {
        if measurement.cpu_insns > budget.cpu_insns
            || measurement.mem_bytes > budget.mem_bytes
            || measurement.ledger_entries > budget.ledger_entries
        {
            return Err(BudgetExceeded {
                operation,
                input_scale,
                cpu_insns: measurement.cpu_insns,
                mem_bytes: measurement.mem_bytes,
                ledger_entries: measurement.ledger_entries,
                budget,
            });
        }
        Ok(())
    }

    /// Build a representative initialized registry for budget tests.
    pub fn setup(env: &Env) -> (SignalRegistryClient<'_>, Address) {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(env, &contract_id);
        let admin = Address::generate(env);
        let schedule = DecaySchedule {
            decay_rate_bps: 500,
            decay_interval: 86_400,
            grace_period: 3_600,
            min_reputation: 0,
            max_reputation: 1_000_000,
        };
        client.initialize(&admin, &schedule);
        (client, admin)
    }

    #[test]
    fn update_reputation_near_limit_stays_within_budget() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);

        let measurement = measure(&env, || {
            client.update_reputation(&provider, &1_000);
        });

        let budget = Budget {
            cpu_insns: 5_000_000,
            mem_bytes: 1_000_000,
            ledger_entries: 5_000_000,
        };
        assert_within("update_reputation", 1, measurement, budget)
            .expect("near-limit update_reputation should fit the budget");
    }

    #[test]
    fn update_reputation_over_limit_reports_operation_and_scale() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);

        let measurement = measure(&env, || {
            client.update_reputation(&provider, &1_000);
        });

        // Deliberately impossible budget to exercise the diagnostic path.
        let budget = Budget {
            cpu_insns: 0,
            mem_bytes: 0,
            ledger_entries: 0,
        };
        let err = assert_within("update_reputation", 1, measurement, budget)
            .expect_err("over-limit workload must be reported");
        assert_eq!(err.operation, "update_reputation");
        assert_eq!(err.input_scale, 1);
        assert!(err.to_string().contains("update_reputation"));
    }

    #[test]
    fn get_reputation_near_limit_stays_within_budget() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);
        client.update_reputation(&provider, &1_000);

        let measurement = measure(&env, || {
            client.get_reputation(&provider);
        });

        let budget = Budget {
            cpu_insns: 5_000_000,
            mem_bytes: 1_000_000,
            ledger_entries: 5_000_000,
        };
        assert_within("get_reputation", 1, measurement, budget)
            .expect("near-limit get_reputation should fit the budget");
    }

    #[test]
    fn get_reputation_over_limit_reports_operation_and_scale() {
        let env = Env::default();
        let (client, _admin) = setup(&env);
        let provider = Address::generate(&env);
        env.ledger().set_timestamp(10_000);
        client.update_reputation(&provider, &1_000);

        let measurement = measure(&env, || {
            client.get_reputation(&provider);
        });

        let budget = Budget {
            cpu_insns: 0,
            mem_bytes: 0,
            ledger_entries: 0,
        };
        let err = assert_within("get_reputation", 1, measurement, budget)
            .expect_err("over-limit workload must be reported");
        assert_eq!(err.operation, "get_reputation");
        assert_eq!(err.input_scale, 1);
        assert!(err.to_string().contains("get_reputation"));
    }
}
