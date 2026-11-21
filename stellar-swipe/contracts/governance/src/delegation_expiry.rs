//! Ledger-bounded voting delegation expiry (Issue #1131).
//!
//! A delegation may carry an expiry ledger. Once `ledger.sequence() >=
//! expiry`, the delegation is ignored by vote-power calculation without any
//! action from the delegator. `cleanup_expired` is a bounded, idempotent
//! sweep that marks expired delegations inactive and drops their expiry.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

use crate::errors::GovernanceError;
use crate::proposals::{self, VoteDelegation};

#[contracttype]
#[derive(Clone)]
pub enum DelegationExpiryKey {
    Expiry(Address),
}

/// Upper bound on delegators scanned by one `cleanup_expired` call.
pub const MAX_CLEANUP_BATCH: u32 = 50;

pub fn get_expiry(env: &Env, delegator: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DelegationExpiryKey::Expiry(delegator.clone()))
}

/// True when `delegator` has an expiry at or before the current ledger.
pub fn is_expired(env: &Env, delegator: &Address) -> bool {
    get_expiry(env, delegator)
        .map(|e| env.ledger().sequence() >= e)
        .unwrap_or(false)
}

/// A delegation counts only while it is active and not expired.
pub fn is_live(env: &Env, d: &VoteDelegation) -> bool {
    d.active && !is_expired(env, &d.delegator)
}

fn set_expiry(env: &Env, delegator: &Address, expiry_ledger: u32) -> Result<(), GovernanceError> {
    if expiry_ledger <= env.ledger().sequence() {
        return Err(GovernanceError::InvalidDuration);
    }
    env.storage().persistent().set(
        &DelegationExpiryKey::Expiry(delegator.clone()),
        &expiry_ledger,
    );
    Ok(())
}

pub fn clear_expiry(env: &Env, delegator: &Address) {
    env.storage()
        .persistent()
        .remove(&DelegationExpiryKey::Expiry(delegator.clone()));
}

/// Delegate with an expiry ledger. An expired prior delegation is replaced.
pub fn delegate_until(
    env: &Env,
    delegator: Address,
    delegate: Address,
    expiry_ledger: u32,
) -> Result<(), GovernanceError> {
    if expiry_ledger <= env.ledger().sequence() {
        return Err(GovernanceError::InvalidDuration);
    }
    proposals::delegate_voting_power(env, delegator.clone(), delegate.clone())?;
    set_expiry(env, &delegator, expiry_ledger)?;
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("deleg"), symbol_short!("created")),
        (delegator, delegate, expiry_ledger),
    );
    Ok(())
}

/// Extend a live delegation. Renewal at the boundary ledger (or later) is
/// rejected: the delegation has already expired and must be re-created.
pub fn renew(env: &Env, delegator: Address, new_expiry: u32) -> Result<(), GovernanceError> {
    delegator.require_auth();
    let state = proposals::get_delegation_state(env);
    let d = state
        .delegations
        .get(delegator.clone())
        .ok_or(GovernanceError::CrossCommitteeRequestNotFound)?;
    if !is_live(env, &d) {
        return Err(GovernanceError::InvalidCommitteeAction);
    }
    set_expiry(env, &delegator, new_expiry)?;
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("deleg"), symbol_short!("renewed")),
        (delegator, d.delegate, new_expiry),
    );
    Ok(())
}

/// Mark up to `limit` (capped at [`MAX_CLEANUP_BATCH`]) expired delegations
/// inactive, starting at index `start`. Idempotent: already-cleaned entries
/// have no expiry and are skipped. Returns the number cleaned.
pub fn cleanup_expired(env: &Env, start: u32, limit: u32) -> u32 {
    let mut state = proposals::get_delegation_state(env);
    let end = start
        .saturating_add(limit.min(MAX_CLEANUP_BATCH))
        .min(state.delegators.len());
    let mut cleaned = 0u32;
    let mut i = start;
    while i < end {
        let delegator = state.delegators.get(i).unwrap();
        if is_expired(env, &delegator) {
            if let Some(mut d) = state.delegations.get(delegator.clone()) {
                d.active = false;
                state.delegations.set(delegator.clone(), d.clone());
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("deleg"), symbol_short!("expired")),
                    (delegator.clone(), d.delegate),
                );
            }
            clear_expiry(env, &delegator);
            cleaned += 1;
        }
        i += 1;
    }
    if cleaned > 0 {
        proposals::put_delegation_state(env, &state);
    }
    cleaned
}
