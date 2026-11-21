//! TTL renewal for critical state touched by mutating entrypoints (Issue #1397).
//!
//! Soroban archives persistent entries (and the contract instance) once their
//! TTL reaches zero. Entries such as the V2 stake map, a staker's queued
//! unstake request, or the queue head/tail pointers must never expire while
//! the vault is in supported operation, otherwise a later call would observe
//! an archived entry mid state-transition.
//!
//! # Ordering guarantee
//!
//! Every mutating entrypoint calls [`renew_stake_state`] (or the narrower
//! helpers) **before** its first storage write. Renewal therefore either
//! succeeds and the entrypoint proceeds against entries that are guaranteed
//! to outlive the call by [`CRITICAL_TTL_THRESHOLD_LEDGERS`], or the host
//! traps — aborting the whole invocation before any write has been made, so
//! no partial state is ever persisted. Entries created by the call itself are
//! renewed again right after they are written (see [`renew_persistent`]) so
//! they also receive the configured extension instead of the network minimum.
//!
//! # Configured extension
//!
//! The extension reuses the workspace-wide hot-key policy from
//! `stellar_swipe_common::ttl_manager`, clamped to the network's
//! `max_ttl()` so a lower network cap never turns renewal into a trap:
//!
//! | Constant | Value | Meaning |
//! |---|---|---|
//! | [`CRITICAL_TTL_THRESHOLD_LEDGERS`] | ~6 days | renew when remaining TTL drops below this |
//! | [`CRITICAL_TTL_TARGET_LEDGERS`] | ~30 days | TTL after renewal |

use soroban_sdk::{Address, Env, IntoVal, Val};
use stellar_swipe_common::ttl_manager::{
    HOT_KEY_TTL_TARGET_LEDGERS, HOT_KEY_TTL_THRESHOLD_LEDGERS,
};

use crate::{migration::MigrationKey, StorageKey};

/// Renew once the remaining TTL drops below this many ledgers (~6 days).
pub const CRITICAL_TTL_THRESHOLD_LEDGERS: u32 = HOT_KEY_TTL_THRESHOLD_LEDGERS;

/// TTL (in ledgers) a critical entry is extended to on renewal (~30 days).
pub const CRITICAL_TTL_TARGET_LEDGERS: u32 = HOT_KEY_TTL_TARGET_LEDGERS;

/// Effective `(threshold, extend_to)` pair, clamped to the network maximum so
/// `extend_ttl` never traps on a network whose `max_ttl` is below the target.
fn effective_bounds(env: &Env) -> (u32, u32) {
    let max = env.storage().max_ttl();
    let target = CRITICAL_TTL_TARGET_LEDGERS.min(max);
    let threshold = CRITICAL_TTL_THRESHOLD_LEDGERS.min(target);
    (threshold, target)
}

/// Renew the contract instance (admin, token, queue pointers, auth config).
pub fn renew_instance(env: &Env) {
    let (threshold, target) = effective_bounds(env);
    env.storage().instance().extend_ttl(threshold, target);
}

/// Renew a single persistent entry if it exists. Missing entries are skipped:
/// there is nothing to keep alive and `extend_ttl` would trap on them.
pub fn renew_persistent<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    let storage = env.storage().persistent();
    if !storage.has(key) {
        return;
    }
    let (threshold, target) = effective_bounds(env);
    storage.extend_ttl(key, threshold, target);
}

/// Renew every critical entry a stake-mutating entrypoint for `staker` may
/// read or write: the instance, the V2 stake map, the staker's queued unstake
/// ticket/entry/reservation, and any pending large-withdrawal request.
///
/// Call this before the first storage write of the entrypoint.
pub fn renew_stake_state(env: &Env, staker: &Address) {
    renew_instance(env);
    renew_persistent(env, &MigrationKey::StakesV2);
    renew_persistent(env, &StorageKey::LargeWithdrawalRequestedAt(staker.clone()));
    renew_unstake_request(env, staker);
}

/// Renew a staker's queue pointer, its queue entry, and its reservation.
pub fn renew_unstake_request(env: &Env, staker: &Address) {
    let ticket_key = StorageKey::UserUnstakeTicket(staker.clone());
    renew_persistent(env, &ticket_key);
    renew_persistent(env, &StorageKey::UnstakeReservation(staker.clone()));
    if let Some(ticket) = env.storage().persistent().get::<_, u64>(&ticket_key) {
        renew_persistent(env, &StorageKey::UnstakeQueueEntry(ticket));
    }
}
