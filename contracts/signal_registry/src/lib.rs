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
}

/// Maximum length, in bytes, of a string carried in an event payload.
pub const MAX_EVENT_STRING_LEN: u32 = 256;

/// Maximum number of elements in a vector carried in an event payload.
pub const MAX_EVENT_VECTOR_LEN: u32 = 64;

/// Maximum length, in bytes, of event metadata (e.g. a topic or label).
pub const MAX_EVENT_METADATA_LEN: u32 = 128;

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

    /// Emit a bounded event payload at the contract boundary.
    ///
    /// The `label` (metadata), `message` (string), and `tags` (vector) are all
    /// validated against the configured size limits before any event is
    /// emitted. If any component is oversized the call fails with
    /// [`SignalError::EventPayloadTooLarge`] and no event is produced.
    pub fn emit_event(
        env: Env,
        label: String,
        message: String,
        tags: Vec<String>,
    ) -> Result<(), SignalError> {
        Self::validate_event_payload(&label, &message, &tags)?;
        env.events().publish((label,), (message, tags));
        Ok(())
    }

    /// Validate an event payload against the configured size limits.
    ///
    /// Returns [`SignalError::EventPayloadTooLarge`] if the metadata label,
    /// string message, or tag vector exceeds its respective maximum. Limits
    /// are inclusive: a value exactly at the limit is accepted, one over is
    /// rejected.
    fn validate_event_payload(
        label: &String,
        message: &String,
        tags: &Vec<String>,
    ) -> Result<(), SignalError> {
        if label.len() > MAX_EVENT_METADATA_LEN {
            return Err(SignalError::EventPayloadTooLarge);
        }
        if message.len() > MAX_EVENT_STRING_LEN {
            return Err(SignalError::EventPayloadTooLarge);
        }
        if tags.len() > MAX_EVENT_VECTOR_LEN {
            return Err(SignalError::EventPayloadTooLarge);
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
    use soroban_sdk::{testutils::Events, Env, String, Vec};

    fn setup() -> (Env, SignalRegistryClient<'static>) {
        let env = Env::default();
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &contract_id);
        (env, client)
    }

    fn string_of_len(env: &Env, len: u32) -> String {
        let mut s = String::from_str(env, "");
        let mut i = 0;
        while i < len {
            s.push_str(&String::from_str(env, "a"));
            i += 1;
        }
        s
    }

    fn vec_of_len(env: &Env, len: u32) -> Vec<String> {
        let mut v = Vec::new(env);
        let mut i = 0;
        while i < len {
            v.push_back(String::from_str(env, "t"));
            i += 1;
        }
        v
    }

    #[test]
    fn accepts_payload_at_exact_limits() {
        let (env, client) = setup();
        let label = string_of_len(&env, MAX_EVENT_METADATA_LEN);
        let message = string_of_len(&env, MAX_EVENT_STRING_LEN);
        let tags = vec_of_len(&env, MAX_EVENT_VECTOR_LEN);
        assert_eq!(client.emit_event(&label, &message, &tags), Ok(()));
    }

    #[test]
    fn rejects_metadata_one_over_limit() {
        let (env, client) = setup();
        let label = string_of_len(&env, MAX_EVENT_METADATA_LEN + 1);
        let message = String::from_str(&env, "ok");
        let tags = Vec::new(&env);
        assert_eq!(
            client.emit_event(&label, &message, &tags),
            Err(SignalError::EventPayloadTooLarge)
        );
    }

    #[test]
    fn rejects_string_one_over_limit() {
        let (env, client) = setup();
        let label = String::from_str(&env, "ok");
        let message = string_of_len(&env, MAX_EVENT_STRING_LEN + 1);
        let tags = Vec::new(&env);
        assert_eq!(
            client.emit_event(&label, &message, &tags),
            Err(SignalError::EventPayloadTooLarge)
        );
    }

    #[test]
    fn rejects_vector_one_over_limit() {
        let (env, client) = setup();
        let label = String::from_str(&env, "ok");
        let message = String::from_str(&env, "ok");
        let tags = vec_of_len(&env, MAX_EVENT_VECTOR_LEN + 1);
        assert_eq!(
            client.emit_event(&label, &message, &tags),
            Err(SignalError::EventPayloadTooLarge)
        );
    }

    #[test]
    fn oversized_payload_emits_no_event() {
        let (env, client) = setup();
        let label = String::from_str(&env, "ok");
        let message = string_of_len(&env, MAX_EVENT_STRING_LEN + 1);
        let tags = Vec::new(&env);
        let _ = client.emit_event(&label, &message, &tags);
        // No event should have been published for a rejected payload.
        assert_eq!(env.events().all().len(), 0);
    }

    #[test]
    fn resource_budget_is_predictable_for_bounded_payloads() {
        let (env, client) = setup();
        let label = String::from_str(&env, "ok");
        let message = string_of_len(&env, MAX_EVENT_STRING_LEN);
        let tags = vec_of_len(&env, MAX_EVENT_VECTOR_LEN);
        // Repeated bounded emissions succeed deterministically.
        let mut i = 0;
        while i < 8 {
            assert_eq!(client.emit_event(&label, &message, &tags), Ok(()));
            i += 1;
        }
        assert_eq!(env.events().all().len(), 8);
    }
}
