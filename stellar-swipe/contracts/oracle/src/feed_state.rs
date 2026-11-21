//! Explicit oracle feed lifecycle (Issue #1307).
//!
//! A feed (price source) moves through `Pending -> Active <-> Disabled -> Retired`.
//! Only `Active` feeds may submit new observations. Disabling or retiring a
//! feed never deletes stored observations, so historical reads, audits and
//! in-flight challenges remain deterministic. Sources with no recorded state
//! predate this lifecycle and are treated as `Active` for compatibility.

use crate::errors::OracleError;
use soroban_sdk::{contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedState {
    /// Registered but not yet allowed to publish.
    Pending,
    /// Allowed to submit new observations.
    Active,
    /// Temporarily stopped; may be reactivated or retired.
    Disabled,
    /// Permanently stopped (terminal).
    Retired,
}

#[contracttype]
#[derive(Clone)]
enum FeedKey {
    State(Address),
}

/// Whether `from -> to` is an allowed lifecycle transition.
pub fn is_valid_transition(from: FeedState, to: FeedState) -> bool {
    use FeedState::*;
    matches!(
        (from, to),
        (Pending, Active)
            | (Pending, Retired)
            | (Active, Disabled)
            | (Active, Retired)
            | (Disabled, Active)
            | (Disabled, Retired)
    )
}

pub fn get_state(env: &Env, source: &Address) -> FeedState {
    env.storage()
        .persistent()
        .get(&FeedKey::State(source.clone()))
        .unwrap_or(FeedState::Active)
}

/// Apply a lifecycle transition, rejecting disallowed moves.
pub fn transition(env: &Env, source: &Address, to: FeedState) -> Result<(), OracleError> {
    let from = get_state(env, source);
    if !is_valid_transition(from, to) {
        return Err(OracleError::InvalidFeedTransition);
    }
    set_state(env, source, to);
    env.events().publish(
        (Symbol::new(env, "feed_state"), source.clone()),
        (from, to),
    );
    Ok(())
}

/// Whether the feed already has an explicit lifecycle record.
pub fn has_state(env: &Env, source: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&FeedKey::State(source.clone()))
}

fn set_state(env: &Env, source: &Address, state: FeedState) {
    env.storage()
        .persistent()
        .set(&FeedKey::State(source.clone()), &state);
}

/// Record a newly registered feed as `Pending`; fails if it already has a
/// lifecycle record so registration cannot bypass transition rules.
pub fn register_pending(env: &Env, source: &Address) -> Result<(), OracleError> {
    if has_state(env, source) {
        return Err(OracleError::InvalidFeedTransition);
    }
    set_state(env, source, FeedState::Pending);
    Ok(())
}

/// Reject submissions from feeds that are not `Active`.
pub fn require_active(env: &Env, source: &Address) -> Result<(), OracleError> {
    if get_state(env, source) == FeedState::Active {
        Ok(())
    } else {
        Err(OracleError::FeedNotActive)
    }
}
