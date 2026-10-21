//! Explicit provider lifecycle status (Issue #1137).
//!
//! Status is stored canonically instead of being inferred from unrelated
//! fields.  Providers with no stored status resolve to `Banned` when a ban is
//! recorded, otherwise `Active` (backwards compatible with existing data).
//!
//! Allowed transitions:
//!
//! | from      | to                          | authority        |
//! |-----------|-----------------------------|------------------|
//! | Pending   | Active, Retired             | admin            |
//! | Active    | Suspended, Banned           | admin            |
//! | Active    | Retired                     | admin / provider |
//! | Suspended | Active, Banned, Retired     | admin            |
//! | Banned    | Active (appeal reversal)    | admin            |
//! | Retired   | — (terminal)                |                  |

use crate::admin;
use crate::errors::ProviderStatusError;
use crate::providers;
use soroban_sdk::{contracttype, symbol_short, Address, Env};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ProviderStatus {
    Pending = 0,
    Active = 1,
    Suspended = 2,
    Banned = 3,
    Retired = 4,
}

#[contracttype]
#[derive(Clone)]
pub enum ProviderStatusKey {
    Status(Address),
}

/// Canonical status of `provider`.
pub fn get_status(env: &Env, provider: &Address) -> ProviderStatus {
    env.storage()
        .persistent()
        .get(&ProviderStatusKey::Status(provider.clone()))
        .unwrap_or_else(|| {
            if providers::is_provider_banned(env, provider) {
                ProviderStatus::Banned
            } else {
                ProviderStatus::Active
            }
        })
}

/// Whether `provider` may submit signals under its canonical status.
pub fn can_submit(env: &Env, provider: &Address) -> bool {
    get_status(env, provider) == ProviderStatus::Active
}

/// Whether `from -> to` is in the transition table.
pub fn is_allowed(from: ProviderStatus, to: ProviderStatus) -> bool {
    use ProviderStatus::*;
    matches!(
        (from, to),
        (Pending, Active)
            | (Pending, Retired)
            | (Active, Suspended)
            | (Active, Banned)
            | (Active, Retired)
            | (Suspended, Active)
            | (Suspended, Banned)
            | (Suspended, Retired)
            | (Banned, Active)
    )
}

/// Applies `provider: current -> next` after authorization and table checks.
/// Emits `(prov_st, provider) -> (previous, next)`.
pub fn transition(
    env: &Env,
    caller: &Address,
    provider: &Address,
    next: ProviderStatus,
) -> Result<ProviderStatus, ProviderStatusError> {
    let is_admin = admin::require_admin(env, caller).is_ok();
    let self_retire = caller == provider && next == ProviderStatus::Retired;
    if !is_admin && !self_retire {
        return Err(ProviderStatusError::Unauthorized);
    }
    caller.require_auth();

    let previous = get_status(env, provider);
    if previous == next {
        return Err(ProviderStatusError::NoChange);
    }
    if !is_allowed(previous, next) {
        return Err(ProviderStatusError::InvalidTransition);
    }
    env.storage()
        .persistent()
        .set(&ProviderStatusKey::Status(provider.clone()), &next);
    env.events().publish(
        (symbol_short!("prov_st"), provider.clone()),
        (previous as u32, next as u32),
    );
    Ok(previous)
}
