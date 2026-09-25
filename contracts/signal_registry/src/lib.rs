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
//!
//! Upgrade authority is handed off through a two-step protocol. The current
//! authority initiates a handoff to a pending successor, and the successor
//! must explicitly accept before it becomes the active authority. A pending
//! handoff never alters active authority, only the current authority may
//! initiate or cancel, and handoffs expire after [`HANDOFF_EXPIRY_SECONDS`].

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
    /// No upgrade authority handoff is currently pending.
    NoPendingHandoff = 8,
    /// A handoff is already pending; it must be accepted or cancelled first.
    HandoffAlreadyPending = 9,
    /// The pending handoff has expired and can no longer be accepted.
    HandoffExpired = 10,
    /// The caller is not the pending successor and cannot accept the handoff.
    WrongAccepter = 11,
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

/// Number of seconds a pending upgrade authority handoff remains acceptable
/// after it is initiated. After this window the handoff expires and can no
/// longer be accepted; the current authority must initiate a new handoff.
pub const HANDOFF_EXPIRY_SECONDS: u64 = 7 * 24 * 60 * 60;

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

/// A pending upgrade authority handoff. Created by the current authority and
/// only promoted to active authority once the successor explicitly accepts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingHandoff {
    /// The successor that must accept before authority transfers.
    pub successor: Address,
    /// Ledger timestamp at which the handoff was initiated.
    pub initiated_at: u64,
    /// Ledger timestamp after which the handoff can no longer be accepted.
    pub expires_at: u64,
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
const PENDING_HANDOFF_KEY: &str = "cfg:pending_handoff";

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

    /// Read the currently active authority.
    pub fn get_authority(env: Env) -> Result<Address, SignalError> {
        env.storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Read the pending handoff, if any. Returns `None` when no handoff is
    /// pending. A pending handoff never changes the active authority.
    pub fn get_pending_handoff(env: Env) -> Option<PendingHandoff> {
        env.storage().instance().get(&PENDING_HANDOFF_KEY)
    }

    /// Initiate a two-step handoff of upgrade authority to `successor`.
    ///
    /// Only the current authority may initiate, and only one handoff may be
    /// pending at a time. The active authority is left unchanged until the
    /// successor accepts via [`SignalRegistry::accept_handoff`].
    pub fn initiate_handoff(env: Env, successor: Address) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();

        if env.storage().instance().has(&PENDING_HANDOFF_KEY) {
            return Err(SignalError::HandoffAlreadyPending);
        }

        let now = env.ledger().timestamp();
        let handoff = PendingHandoff {
            successor: successor.clone(),
            initiated_at: now,
            expires_at: now.saturating_add(HANDOFF_EXPIRY_SECONDS),
        };
        env.storage().instance().set(&PENDING_HANDOFF_KEY, &handoff);

        env.events()
            .publish(("handoff_initiated",), (admin, successor));
        Ok(())
    }

    /// Accept a pending handoff, promoting the successor to active authority.
    ///
    /// Only the designated successor may accept, the handoff must not have
    /// expired, and the pending record is cleared on success. Replay attempts
    /// fail because the pending record no longer exists.
    pub fn accept_handoff(env: Env) -> Result<(), SignalError> {
        let handoff: PendingHandoff = env
            .storage()
            .instance()
            .get(&PENDING_HANDOFF_KEY)
            .ok_or(SignalError::NoPendingHandoff)?;

        handoff.successor.require_auth();

        let now = env.ledger().timestamp();
        if now > handoff.expires_at {
            return Err(SignalError::HandoffExpired);
        }

        env.storage()
            .instance()
            .set(&ADMIN_KEY, &handoff.successor);
        env.storage().instance().remove(&PENDING_HANDOFF_KEY);

        env.events()
            .publish(("handoff_accepted",), (handoff.successor, now));
        Ok(())
    }

    /// Cancel a pending handoff. Only the current authority may cancel, and
    /// the active authority is left unchanged.
    pub fn cancel_handoff(env: Env) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();

        if !env.storage().instance().has(&PENDING_HANDOFF_KEY) {
            return Err(SignalError::NoPendingHandoff);
        }
        env.storage().instance().remove(&PENDING_HANDOFF_KEY);

        env.events().publish(("handoff_cancelled",), (admin,));
        Ok(())
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
