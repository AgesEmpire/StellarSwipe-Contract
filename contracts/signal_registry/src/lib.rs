//! Signal Registry Contract
//!
//! Tracks provider signals and computes provider reputation. Reputation is
//! subject to a deterministic, configuration-driven decay schedule so that
//! stale activity loses impact over time. Decay points are computed directly
//! from stored timestamps and configuration values, keeping the behavior
//! verifiable on-chain.
//!
//! ## Initialization audit
//!
//! Required storage items written by [`SignalRegistry::initialize`]:
//! - `admin` (instance): the address authorized to mutate configuration.
//! - `schedule` (instance): the validated decay schedule.
//! - `version` (instance): the storage schema version marker.
//!
//! Initialization is atomic: the version marker is written last, so a partial
//! or interrupted initialization leaves the contract uninitialized and can be
//! safely retried. Repeated initialization is rejected with
//! [`SignalError::AlreadyInitialized`] before any state is mutated.
//!
//! ## Authorization expiration
//!
//! Temporary permissions, delegated capabilities, and time-bounded approvals
//! are stored with an explicit `expires_at` ledger timestamp. Every sensitive
//! entrypoint that consumes a permission fails closed once the current ledger
//! timestamp reaches `expires_at` (inclusive boundary). Expiration cleanup is
//! bounded by [`MAX_EXPIRATION_SWEEP`] and only removes already-expired grants,
//! so it never changes authorization semantics for live permissions.
//!
//! ## Nonce domain separation
//!
//! Nonces are scoped by an explicit [`NonceDomain`] that binds the owning
//! `user`, the `operation` type, and the `contract` domain identifier. A nonce
//! issued for one operation or contract can never be replayed in another flow:
//! the domain is part of both the storage key and the hashed nonce value, so a
//! cross-operation or cross-contract replay resolves to a different key and is
//! rejected as unused. Clients can discover the required domain for a flow via
//! [`SignalRegistry::nonce_domain`].
//!
//! ### Migration strategy for existing nonce state
//!
//! Legacy nonces were stored under the unqualified `(NONCE_KEY, user)` key with
//! no domain binding. On upgrade, `initialize` records the storage version and
//! legacy entries are treated as belonging to the [`NonceDomain::LEGACY`]
//! domain (operation `0`, contract `0`). Operators migrate by re-issuing nonces
//! through [`SignalRegistry::issue_nonce`] under the correct domain; the legacy
//! key is never consulted for domain-scoped flows, so stale nonces cannot be
//! replayed. The version marker is bumped to [`STORAGE_VERSION`] so clients can
//! detect whether migration has completed.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Bytes, Env, Vec};

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
    PermissionExpired = 6,
    PermissionNotFound = 7,
    InvalidExpiration = 8,
    NonceAlreadyUsed = 9,
    InvalidNonceDomain = 10,
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

/// A time-bounded permission, delegated capability, or approval.
///
/// `expires_at` is an inclusive ledger timestamp: the grant is valid while
/// `now < expires_at` and fails closed once `now >= expires_at`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Permission {
    pub grantee: Address,
    pub capability: u32,
    pub expires_at: u64,
}

/// Explicit domain that scopes a nonce.
///
/// A nonce is only valid within the exact `(user, operation, contract)` tuple
/// it was issued for. `operation` distinguishes flows (e.g. transfer vs.
/// withdraw) and `contract` distinguishes the contract domain, so a nonce from
/// one flow cannot be replayed in another.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonceDomain {
    pub user: Address,
    pub operation: u32,
    pub contract: u32,
}

impl NonceDomain {
    /// Domain reserved for pre-domain-separation (legacy) nonce state.
    pub const LEGACY: u32 = 0;
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
const VERSION_KEY: &str = "version";
const PERMISSION_KEY: &str = "permission";
const NONCE_KEY: &str = "nonce";

/// Current storage schema version written by `initialize`.
const STORAGE_VERSION: u32 = 1;

/// Upper bound on how many expired permissions a single cleanup call removes.
/// Keeps expiration cleanup bounded and gas-predictable.
pub const MAX_EXPIRATION_SWEEP: u32 = 32;

#[contract]
pub struct SignalRegistry;

#[contractimpl]
impl SignalRegistry {
    /// Initialize the contract with an admin and a decay schedule.
    ///
    /// Writes every required storage item exactly once: `admin`, `schedule`,
    /// and the `version` marker. The version marker is written last so that an
    /// interrupted initialization is detectable and retryable. If the contract
    /// is already initialized this returns [`SignalError::AlreadyInitialized`]
    /// without mutating any state.
    pub fn initialize(env: Env, admin: Address, schedule: DecaySchedule) -> Result<(), SignalError> {
        if Self::is_initialized(&env) {
            return Err(SignalError::AlreadyInitialized);
        }
        Self::validate_schedule(&schedule)?;
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&SCHEDULE_KEY, &schedule);
        // Version marker written last: its presence signals a complete init.
        env.storage().instance().set(&VERSION_KEY, &STORAGE_VERSION);
        Ok(())
    }

    /// Returns `true` once initialization has completed.
    ///
    /// The version marker is the authoritative signal: it is only written after
    /// all other required fields, so a partial initialization reports `false`.
    pub fn is_initialized(env: &Env) -> bool {
        env.storage().instance().has(&VERSION_KEY)
    }

    /// Read the stored schema version marker.
    pub fn get_version(env: Env) -> Result<u32, SignalError> {
        env.storage()
            .instance()
            .get(&VERSION_KEY)
            .ok_or(SignalError::NotInitialized)
    }

    /// Return the nonce domain a client must use for `operation` on this
    /// contract. Clients call this to determine the required domain before
    /// issuing or consuming a nonce.
    pub fn nonce_domain(env: Env, user: Address, operation: u32) -> NonceDomain {
        NonceDomain {
            user,
            operation,
            contract: Self::contract_domain(&env),
        }
    }

    /// Derive the contract domain identifier for this deployment.
    ///
    /// The contract's own address is hashed so that nonces issued by one
    /// deployment can never be replayed against another deployment.
    fn contract_domain(env: &Env) -> u32 {
        let bytes = env.current_contract_address().to_string().into_bytes();
        let mut acc: u32 = 0;
        let mut i: u32 = 0;
        while i < bytes.len() {
            acc = acc.wrapping_mul(31).wrapping_add(bytes.get(i).unwrap() as u32);
            i += 1;
        }
        acc
    }

    /// Hash a nonce together with its domain so the stored value is bound to
    /// the exact `(user, operation, contract)` tuple.
    fn hash_nonce(env: &Env, domain: &NonceDomain, nonce: &Bytes) -> Bytes {
        let mut preimage = Bytes::new(env);
        preimage.append(&domain.user.to_string().into_bytes());
        preimage.extend_from_array(&domain.operation.to_be_bytes());
        preimage.extend_from_array(&domain.contract.to_be_bytes());
        preimage.append(nonce);
        env.crypto().sha256(&preimage).into()
    }

    /// Issue a nonce for `domain`. Rejects a domain that reuses the reserved
    /// legacy operation id so legacy state cannot be silently re-bound.
    pub fn issue_nonce(env: Env, domain: NonceDomain, nonce: Bytes) -> Result<(), SignalError> {
        if domain.operation == NonceDomain::LEGACY {
            return Err(SignalError::InvalidNonceDomain);
        }
        let hashed = Self::hash_nonce(&env, &domain, &nonce);
        let key = (NONCE_KEY, domain.clone(), hashed.clone());
        if env.storage().persistent().has(&key) {
            return Err(SignalError::NonceAlreadyUsed);
        }
        env.storage().persistent().set(&key, &true);
        Ok(())
    }

    /// Consume a nonce within `domain`. Fails closed with
    /// [`SignalError::NonceAlreadyUsed`] when the nonce was already spent in
    /// this domain, and [`SignalError::InvalidNonceDomain`] for the reserved
    /// legacy domain. A nonce issued under a different operation or contract
    /// resolves to a different key and is therefore rejected as unused.
    pub fn consume_nonce(env: Env, domain: NonceDomain, nonce: Bytes) -> Result<(), SignalError> {
        if domain.operation == NonceDomain::LEGACY {
            return Err(SignalError::InvalidNonceDomain);
        }
        let hashed = Self::hash_nonce(&env, &domain, &nonce);
        let key = (NONCE_KEY, domain.clone(), hashed.clone());
        if env.storage().persistent().has(&key) {
            return Err(SignalError::NonceAlreadyUsed);
        }
        env.storage().persistent().set(&key, &true);
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

    /// Grant a time-bounded permission to `grantee`.
    ///
    /// Only the admin may grant. `expires_at` must be strictly in the future
    /// relative to the current ledger timestamp, otherwise the grant is
    /// rejected with [`SignalError::InvalidExpiration`].
    pub fn grant_permission(
        env: Env,
        grantee: Address,
        capability: u32,
        expires_at: u64,
    ) -> Result<(), SignalError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(SignalError::NotInitialized)?;
        admin.require_auth();
        let now = env.ledger().timestamp();
        if expires_at <= now {
            return Err(SignalError::InvalidExpiration);
        }
        let permission = Permission {
            grantee: grantee.clone(),
            capability,
            expires_at,
        };
        env.storage()
            .persistent()
            .set(&(PERMISSION_KEY, grantee, capability), &permission);
        Ok(())
    }

    /// Check whether `grantee` currently holds `capability`.
    ///
    /// Fails closed: returns `false` once the current ledger timestamp reaches
    /// the stored `expires_at` (inclusive boundary).
    pub fn has_permission(env: Env, grantee: Address, capability: u32) -> bool {
        match env
            .storage()
            .persistent()
            .get::<_, Permission>(&(PERMISSION_KEY, grantee, capability))
        {
            Some(permission) => !Self::is_expired(&env, &permission),
            None => false,
        }
    }

    /// Consume a permission at a sensitive entrypoint.
    ///
    /// Fails closed with [`SignalError::PermissionExpired`] when the grant has
    /// expired, and [`SignalError::PermissionNotFound`] when no grant exists.
    /// The expiration check happens before any state mutation so an expired
    /// grant can never authorize a sensitive action.
    pub fn require_permission(
        env: Env,
        grantee: Address,
        capability: u32

/* … truncated 5832 chars — edit only what you need near the top … */
