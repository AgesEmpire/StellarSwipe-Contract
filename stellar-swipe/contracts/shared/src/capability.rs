//! Scoped capability delegation (Issue: scoped capability delegation).
//!
//! Provides fine-grained, time-bounded delegation of privileged capabilities
//! (upgrades, pause changes, treasury ops, etc.) without granting full admin
//! control. Any contract that wants scoped delegation imports these helpers
//! and stores the `CapabilityState` in its own instance storage.

use soroban_sdk::{contracterror, contracttype, Address, Env, Map, Symbol};

// ── Errors ─────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CapabilityError {
    InsufficientCapability = 1,
    DelegationNotFound = 2,
    DelegationExpired = 3,
    MaxDelegationsReached = 4,
    SelfDelegation = 5,
}

// ── Types ──────────────────────────────────────────────────────────────────────

/// Privileged capability that can be delegated independently.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CapabilityScope {
    Upgrade = 1,
    Pause = 2,
    Treasury = 3,
    ParameterChange = 4,
    AdminTransfer = 5,
    EmergencyAction = 6,
    ContractConfig = 7,
}

/// A single capability delegation record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDelegation {
    pub delegator: Address,
    pub delegate: Address,
    pub scope: CapabilityScope,
    pub expires_at: u64,
}

/// Top-level delegation state for a contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityState {
    /// Keyed by (delegate, scope_code) so a delegate can hold the same scope
    /// from at most one delegator.  New delegations overwrite prior ones for
    /// the same (delegate, scope) pair.
    pub delegations: Map<(Address, u32), CapabilityDelegation>,
}

#[contracttype]
pub enum CapabilityStorageKey {
    State,
    DelegationCount(Address),
}

pub const MAX_DELEGATIONS_PER_DELEGATOR: u32 = 32;

// ── Helpers ────────────────────────────────────────────────────────────────────

pub fn empty_capability_state(env: &Env) -> CapabilityState {
    CapabilityState {
        delegations: Map::new(env),
    }
}

pub fn get_capability_state(env: &Env) -> CapabilityState {
    env.storage()
        .instance()
        .get(&CapabilityStorageKey::State)
        .unwrap_or_else(|| empty_capability_state(env))
}

pub fn put_capability_state(env: &Env, state: &CapabilityState) {
    env.storage()
        .instance()
        .set(&CapabilityStorageKey::State, state);
}

/// Grant `delegate` the right to perform actions protected by `scope` until
/// `expires_at` (ledger timestamp).  `expires_at == 0` means no expiry.
pub fn delegate_capability(
    env: &Env,
    delegator: &Address,
    delegate: &Address,
    scope: CapabilityScope,
    expires_at: u64,
) -> Result<(), CapabilityError> {
    delegator.require_auth();
    if delegate == delegator {
        return Err(CapabilityError::SelfDelegation);
    }

    let count_key = CapabilityStorageKey::DelegationCount(delegator.clone());
    let count: u32 = env.storage().instance().get(&count_key).unwrap_or(0);

    if count >= MAX_DELEGATIONS_PER_DELEGATOR {
        return Err(CapabilityError::MaxDelegationsReached);
    }

    let mut state = get_capability_state(env);
    let key = (delegate.clone(), scope as u32);
    let is_new = !state.delegations.contains_key(key.clone());

    state.delegations.set(
        key,
        CapabilityDelegation {
            delegator: delegator.clone(),
            delegate: delegate.clone(),
            scope,
            expires_at,
        },
    );

    put_capability_state(env, &state);

    if is_new {
        env.storage().instance().set(&count_key, &(count + 1));
    }

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "capability"),
            Symbol::new(env, "delegated"),
        ),
        (
            delegator.clone(),
            delegate.clone(),
            scope as u32,
            expires_at,
        ),
    );

    Ok(())
}

/// Remove a previously granted capability delegation.
pub fn revoke_capability(
    env: &Env,
    delegator: &Address,
    delegate: &Address,
    scope: CapabilityScope,
) -> Result<(), CapabilityError> {
    delegator.require_auth();

    let mut state = get_capability_state(env);
    let key = (delegate.clone(), scope as u32);

    if !state.delegations.contains_key(key.clone()) {
        return Err(CapabilityError::DelegationNotFound);
    }

    state.delegations.remove(key.clone());
    put_capability_state(env, &state);

    let count_key = CapabilityStorageKey::DelegationCount(delegator.clone());
    let count: u32 = env.storage().instance().get(&count_key).unwrap_or(0);
    if count > 0 {
        env.storage().instance().set(&count_key, &(count - 1));
    }

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "capability"), Symbol::new(env, "revoked")),
        (delegator.clone(), delegate.clone(), scope as u32),
    );

    Ok(())
}

/// Return `Ok(())` if `caller` holds a valid (non-expired) delegation for
/// `scope`, otherwise `Err(CapabilityError::InsufficientCapability)`.
pub fn require_capability(
    env: &Env,
    caller: &Address,
    scope: CapabilityScope,
) -> Result<(), CapabilityError> {
    let now = env.ledger().timestamp();
    let state = get_capability_state(env);

    for (_, delegation) in state.delegations.iter() {
        if delegation.delegate == *caller
            && delegation.scope == scope
            && (delegation.expires_at == 0 || delegation.expires_at > now)
        {
            return Ok(());
        }
    }

    Err(CapabilityError::InsufficientCapability)
}

/// Count delegations issued by `delegator`.
pub fn delegation_count(env: &Env, delegator: &Address) -> u32 {
    env.storage()
        .instance()
        .get(&CapabilityStorageKey::DelegationCount(delegator.clone()))
        .unwrap_or(0)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract,
        testutils::{Address as _, Ledger as _},
        Env,
    };

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(TestContract, ());
        (env, id)
    }

    #[test]
    fn delegate_and_require_succeeds() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        env.as_contract(&_id, || {
            delegate_capability(&env, &admin, &operator, CapabilityScope::Upgrade, 0).unwrap();
            assert!(require_capability(&env, &operator, CapabilityScope::Upgrade).is_ok());
        });
    }

    #[test]
    fn require_fails_for_undelegated_scope() {
        let (env, _id) = setup();
        let someone = Address::generate(&env);

        env.as_contract(&_id, || {
            assert_eq!(
                require_capability(&env, &someone, CapabilityScope::Upgrade),
                Err(CapabilityError::InsufficientCapability)
            );
        });
    }

    #[test]
    fn revoke_removes_capability() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        env.as_contract(&_id, || {
            delegate_capability(&env, &admin, &operator, CapabilityScope::Upgrade, 0).unwrap();
            assert!(require_capability(&env, &operator, CapabilityScope::Upgrade).is_ok());

            revoke_capability(&env, &admin, &operator, CapabilityScope::Upgrade).unwrap();
            assert_eq!(
                require_capability(&env, &operator, CapabilityScope::Upgrade),
                Err(CapabilityError::InsufficientCapability)
            );
        });
    }

    #[test]
    fn delegation_expires_after_ttl() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        env.as_contract(&_id, || {
            let now = env.ledger().timestamp();
            delegate_capability(&env, &admin, &operator, CapabilityScope::Upgrade, now + 50)
                .unwrap();
            assert!(require_capability(&env, &operator, CapabilityScope::Upgrade).is_ok());

            env.ledger().with_mut(|l| l.sequence_number += 100);
            assert_eq!(
                require_capability(&env, &operator, CapabilityScope::Upgrade),
                Err(CapabilityError::InsufficientCapability)
            );
        });
    }

    #[test]
    fn self_delegation_rejected() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);

        env.as_contract(&_id, || {
            assert_eq!(
                delegate_capability(&env, &admin, &admin, CapabilityScope::Upgrade, 0),
                Err(CapabilityError::SelfDelegation)
            );
        });
    }

    #[test]
    fn max_delegations_enforced() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);

        env.as_contract(&_id, || {
            for _ in 0..MAX_DELEGATIONS_PER_DELEGATOR {
                let delegate = Address::generate(&env);
                delegate_capability(&env, &admin, &delegate, CapabilityScope::Upgrade, 0).unwrap();
            }
            let extra = Address::generate(&env);
            assert_eq!(
                delegate_capability(&env, &admin, &extra, CapabilityScope::Upgrade, 0),
                Err(CapabilityError::MaxDelegationsReached)
            );
        });
    }

    #[test]
    fn delegation_count_tracks_active_delegations() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);
        let op1 = Address::generate(&env);
        let op2 = Address::generate(&env);

        env.as_contract(&_id, || {
            assert_eq!(delegation_count(&env, &admin), 0);
            delegate_capability(&env, &admin, &op1, CapabilityScope::Upgrade, 0).unwrap();
            assert_eq!(delegation_count(&env, &admin), 1);
            delegate_capability(&env, &admin, &op2, CapabilityScope::Pause, 0).unwrap();
            assert_eq!(delegation_count(&env, &admin), 2);
            revoke_capability(&env, &admin, &op1, CapabilityScope::Upgrade).unwrap();
            assert_eq!(delegation_count(&env, &admin), 1);
        });
    }

    #[test]
    fn revoke_not_found_returns_error() {
        let (env, _id) = setup();
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        env.as_contract(&_id, || {
            assert_eq!(
                revoke_capability(&env, &admin, &operator, CapabilityScope::Upgrade),
                Err(CapabilityError::DelegationNotFound)
            );
        });
    }

    #[test]
    fn delegation_emits_event() {
        use soroban_sdk::testutils::Events;
        let (env, _id) = setup();
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        env.as_contract(&_id, || {
            delegate_capability(&env, &admin, &operator, CapabilityScope::Upgrade, 0).unwrap();
            assert!(!env.events().all().is_empty());
        });
    }
}
