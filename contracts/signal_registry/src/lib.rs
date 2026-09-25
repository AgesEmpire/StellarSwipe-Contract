//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Bytes, BytesN, Env, Vec};

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
    InvalidCursor = 7,
    InvalidPageSize = 8,
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

/// Canonical inputs that define a Soroban authorization signing domain.
///
/// Every authorization path must derive its signing domain from exactly these
/// fields so that equivalent payloads always hash identically and any altered
/// field produces a different digest.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthPayload {
    pub domain: Bytes,
    pub network: Bytes,
    pub contract: Address,
    pub method: Bytes,
}

/// A single provider relationship entry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRelationship {
    pub provider: Address,
    pub counterparty: Address,
}

/// A single provider membership entry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderMembership {
    pub provider: Address,
    pub group: Address,
}

/// A bounded page of results plus a stable cursor for the next page.
///
/// `next_cursor` is `None` when the caller has reached the final page.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<u32>,
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

/// A versioned contract event carrying a monotonically increasing sequence
/// number.
///
/// `sequence` is assigned from a per-contract counter that is persisted in
/// instance storage, so it survives contract upgrades and never resets. The
/// counter is scoped to this contract instance, giving indexers a single
/// strictly increasing stream they can use to detect gaps, duplicates, and
/// out-of-order delivery. `version` identifies the event schema so consumers
/// can evolve their decoding logic independently of the sequence stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionedEvent {
    pub version: u32,
    pub sequence: u64,
    pub kind: EventKind,
    pub provider: Address,
    pub value: i128,
}

/// The kind of state transition a `VersionedEvent` describes.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    ReputationUpdated = 0,
    ScheduleUpdated = 1,
}

/// Current schema version emitted on every `VersionedEvent`.
pub const EVENT_VERSION: u32 = 1;

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
const RELATIONSHIP_KEY: &str = "relationship";
const MEMBERSHIP_KEY: &str = "membership";

/// Maximum number of items a single page may return. Requests above this
/// bound are rejected with `SignalError::InvalidPageSize`.
const MAX_PAGE_SIZE: u32 = 100;

const SEQUENCE_KEY: &str = "sequence";

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
        // Seed the event sequence counter so the first emitted event is 1.
        env.storage().instance().set(&SEQUENCE_KEY, &0u64);
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
        Self::emit_event(&env, EventKind::ScheduleUpdated, admin, 0);
        Ok(())
    }

    /// Read the currently configured decay schedule.
    pub fn get_decay_schedule(env: Env) -> Result<DecaySchedule, SignalError> {
        env.storage()
            .instance()
            .get(&SCHEDULE_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Read the next sequence number that will be assigned to an emitted
    /// event. Exposed for indexers and tests to verify continuity.
    pub fn next_sequence(env: Env) -> u64 {
        Self::load_sequence(&env).saturating_add(1)
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
        Self::emit_event(&env, EventKind::ReputationUpdated, provider, updated);
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

    /// Canonical helper for hashing Soroban authorization payloads.
    ///
    /// Serializes the domain, network, contract, and method into a single
    /// deterministic byte stream and returns its SHA-256 digest. All
    /// authorization paths must route through this helper so that equivalent
    /// payloads hash identically and any altered field yields a distinct hash.
    pub fn hash_auth_payload(env: Env, payload: AuthPayload) -> BytesN<32> {
        let mut bytes = Bytes::new(&env);
        Self::append_field(&mut bytes, &payload.domain);
        Self::append_field(&mut bytes, &payload.network);
        Self::append_field(&mut bytes, &payload.contract.to_string().into_bytes());
        Self::append_field(&mut bytes, &payload.method);
        env.crypto().sha256(&bytes)
    }

    /// Length-prefix a field so concatenated fields cannot be re-partitioned
    /// into a different payload that collides on the same serialized bytes.
    fn append_field(out: &mut Bytes, field: &Bytes) {
        out.append(&(field.len() as u32).to_be_bytes().into());
        out.append(field);
    }

    /// Record a provider relationship. Read-only queries below expose these
    /// entries in a bounded, cursor-paginated fashion.
    pub fn add_relationship(
        env: Env,
        provider: Address,
        counterparty: Address,
    ) -> Result<(), SignalError> {
        provider.require_auth();
        let key = (RELATIONSHIP_KEY, provider.clone());
        let mut list: Vec<ProviderRelationship> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        list.push_back(ProviderRelationship {
            provider: provider.clone(),
            counterparty,
        });
        env.storage().persistent().set(&key, &list);
        Ok(())
    }

    /// Record a provider membership. Read-only queries below expose these
    /// entries in a bounded, cursor-paginated fashion.
    pub fn add_membership(
        env: Env,
        provider: Address,
        group: Address,
    ) -> Result<(), SignalError> {
        provider.require_auth();
        let key = (MEMBERSHIP_KEY, provider.clone());
        let mut list: Vec<ProviderMembership> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        list.push_back(ProviderMembership {
            provider: provider.clone(),
            group,
        });
        env.storage().persistent().set(&key, &list);
        Ok(())
    }

    /// Read a bounded page of provider relationships.
    ///
    /// `cursor` is the index of the first item to return; pass `None` for the
    /// first page. `limit` must be between 1 and `MAX_PAGE_SIZE` inclusive.
    /// Results are deterministic: the same cursor and limit always return the
    /// same slice of the stored list. `next_cursor` is `None` on the final
    /// page. Invalid cursors (past the end) return `SignalError::InvalidCursor`
    /// and oversized limits return `SignalError::InvalidPageSize`.
    pub fn get_relationships(
        env: Env,
        provider: Address,
        cursor: Option<u32>,
        limit: u32,
    ) -> Result<Page<ProviderRelationship>, SignalError> {
        let key = (RELATIONSHIP_KEY, provider.clone());
        let list: Vec<ProviderRelationship> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        Self::paginate(&env, &list, cursor, limit)
    }

    /// Read a bounded page of provider memberships.
    ///
    /// See `get_relationships` for cursor and limit semantics.
    pub fn get_memberships(
        env: Env,
        provider: Address,
        cursor: Option<u32>,
        limit: u32,
    ) -> Result<Page<ProviderMembership>, SignalError> {
        let key = (MEMBERSHIP_KEY, provider.clone());
        let list: Vec<ProviderMembership> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        Self::paginate(&env, &list, cursor, limit)
    }

    /// Shared pagination helper: validates the cursor and limit, then returns
    /// the requested slice plus a stable next cursor.
    fn paginate<T: Clone>(
        env: &Env,
        list: &Vec<T>,
        cursor: Option<u32>,
        limit: u32,
    ) -> Result<Page<T>, SignalError> {
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(SignalError::InvalidPageSize);
        }
        let len = list.len();
        let start = cursor.unwrap_or(0);
        if start > len {
            return Err(SignalError::InvalidCursor);
        }
        let end = core::cmp::min(start.saturating_add(limit), len);
        let mut items: Vec<T> = Vec::new(env);
        let mut i = start;
        while i < end {
            if let Some(item) = list.get(i) {
                items.push_back(item);
            }
            i += 1;
        }
        let next_cursor = if end < len { Some(end) } else { None };
        Ok(Page { items, next_cursor })
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

    /// Compute the decayed score for a record at a given timestamp.
    ///
    /// `auth_depth` is validated against `MAX_AUTH_TREE_DEPTH` before

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
    use soroban_sdk::{testutils::Address as _, Env};

    fn payload(env: &Env, domain: &str, network: &str, method: &str) -> AuthPayload {
        AuthPayload {
            domain: Bytes::from_slice(env, domain.as_bytes()),
            network: Bytes::from_slice(env, network.as_bytes()),
            contract: Address::generate(env),
            method: Bytes::from_slice(env, method.as_bytes()),
        }
    }

    #[test]
    fn equivalent_payloads_hash_identically() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let base = AuthPayload {
            domain: Bytes::from_slice(&env, b"signal-registry"),
            network: Bytes::from_slice(&env, b"testnet"),
            contract: contract.clone(),
            method: Bytes::from_slice(&env, b"update_reputation"),
        };
        let same = AuthPayload {
            domain: Bytes::from_slice(&env, b"signal-registry"),
            network: Bytes::from_slice(&env, b"testnet"),
            contract,
            method: Bytes::from_slice(&env, b"update_reputation"),
        };
        assert_eq!(
            SignalRegistry::hash_auth_payload(env.clone(), base),
            SignalRegistry::hash_auth_payload(env, same)
        );
    }

    #[test]
    fn altered_fields_change_hash() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let base = AuthPayload {
            domain: Bytes::from_slice(&env, b"signal-registry"),
            network: Bytes::from_slice(&env, b"testnet"),
            contract: contract.clone(),
            method: Bytes::from_slice(&env, b"update_reputation"),
        };
        let base_hash = SignalRegistry::hash_auth_payload(env.clone(), base.clone());

        let mut altered_domain = base.clone();
        altered_domain.domain = Bytes::from_slice(&env, b"other-domain");
        assert_ne!(base_hash, SignalRegistry::hash_auth_payload(env.clone(), altered_domain));

        let mut altered_network = base.clone();
        altered_network.network = Bytes::from_slice(&env, b"mainnet");
        assert_ne!(base_hash, SignalRegistry::hash_auth_payload(env.clone(), altered_network));

        let mut altered_contract = base.clone();
        altered_contract.contract = Address::generate(&env);
        assert_ne!(base_hash, SignalRegistry::hash_auth_payload(env.clone(), altered_contract));

        let mut altered_method = base.clone();
        altered_method.method = Bytes::from_slice(&env, b"set_decay_schedule");
        assert_ne!(base_hash, SignalRegistry::hash_auth_payload(env, altered_method));
    }

    #[test]
    fn field_boundaries_are_unambiguous() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let a = AuthPayload {
            domain: Bytes::from_slice(&env, b"ab"),
            network: Bytes::from_slice(&env, b"c"),
            contract: contract.clone(),
            method: Bytes::from_slice(&env, b"m"),
        };
        let b = AuthPayload {
            domain: Bytes::from_slice(&env, b"a"),
            network: Bytes::from_slice(&env, b"bc"),
            contract,
            method: Bytes::from_slice(&env, b"m"),
        };
        assert_ne!(
            SignalRegistry::hash_auth_payload(env.clone(), a),
            SignalRegistry::hash_auth_payload(env, b)
        );
    }
}

mod auth_cache_tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    fn schedule() -> DecaySchedule {
        DecaySchedule {
            decay_rate_bps: 0,
            decay_interval: 1,
            grace_period: 0,
            min_reputation: 0,
            max_reputation: 1_000,
        }
    }

    fn setup() -> (Env, SignalRegistryClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SignalRegistry);
        let client = SignalRegistryClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin, &schedule());
        (env, client, admin)
    }

    /// A role change app
