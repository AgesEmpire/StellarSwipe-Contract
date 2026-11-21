//! Independent partial unbonding requests (Issue #1432).
//!
//! A staker can move part of their position into an unbonding request
//! without touching the rest of it:
//!
//! * **Per-request accounting**: each request records its own `amount`,
//!   `requested_at` and `unlock_at`, and is withdrawn independently by id.
//!   Up to [`MAX_PENDING_UNBONDING_REQUESTS`] requests may be pending at once.
//! * **Unlock time**: `unlock_at = max(now + withdrawal_cooldown, locked_until)`,
//!   so an unbonding request can never release funds before the position's
//!   own lock expires.
//! * **Remainder keeps its accounting basis**: requesting only reduces the
//!   active balance.  The position's `locked_until`, `last_updated` and
//!   deposit timestamp (voting-power lock) are left as they were.  The reward
//!   index checkpoints the staker *before* the participating stake drops, so
//!   rewards earned on the full balance up to the request stay credited; only
//!   future accrual is on the reduced balance.
//! * **Unbonding funds** stay in vault custody but no longer count as active
//!   stake (no voting power, no reward accrual).  Requests are only accepted
//!   while no slash/reward lockup is active, so they cannot be used to dodge
//!   a pending slash.
//! * **Withdrawal**: once `unlock_at` has passed, `withdraw_unbonded` pays one
//!   request; `withdraw_all_unbonded` pays every unlocked request (bounded by
//!   the pending cap).  A request is marked withdrawn before the transfer; a
//!   failed transfer reverts the whole call, so a request is paid at most once.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::StakeVaultError;

/// Maximum number of pending (not yet withdrawn) unbonding requests per staker.
pub const MAX_PENDING_UNBONDING_REQUESTS: u32 = 10;

#[contracttype]
#[derive(Clone)]
pub enum UnbondingKey {
    /// staker → next request id.
    NextId(Address),
    /// (staker, id) → UnbondingRequest.
    Request(Address, u64),
    /// staker → ids of pending requests (bounded).
    Pending(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnbondingRequest {
    pub id: u64,
    pub staker: Address,
    pub amount: i128,
    pub requested_at: u64,
    pub unlock_at: u64,
    pub withdrawn: bool,
}

pub fn pending_ids(env: &Env, staker: &Address) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&UnbondingKey::Pending(staker.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn put_pending_ids(env: &Env, staker: &Address, ids: &Vec<u64>) {
    let key = UnbondingKey::Pending(staker.clone());
    if ids.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, ids);
    }
}

pub fn get(env: &Env, staker: &Address, id: u64) -> Option<UnbondingRequest> {
    env.storage()
        .persistent()
        .get(&UnbondingKey::Request(staker.clone(), id))
}

/// Pending requests for `staker`, in creation order.
pub fn pending_requests(env: &Env, staker: &Address) -> Vec<UnbondingRequest> {
    let ids = pending_ids(env, staker);
    let mut out = Vec::new(env);
    for i in 0..ids.len() {
        if let Some(r) = get(env, staker, ids.get_unchecked(i)) {
            out.push_back(r);
        }
    }
    out
}

/// Sum of pending (not yet withdrawn) unbonding amounts for `staker`.
pub fn pending_total(env: &Env, staker: &Address) -> i128 {
    let reqs = pending_requests(env, staker);
    let mut total: i128 = 0;
    for i in 0..reqs.len() {
        total = total.saturating_add(reqs.get_unchecked(i).amount);
    }
    total
}

/// Create a new request. The caller has already moved `amount` out of the
/// active balance.
pub fn create(
    env: &Env,
    staker: &Address,
    amount: i128,
    unlock_at: u64,
) -> Result<UnbondingRequest, StakeVaultError> {
    let mut ids = pending_ids(env, staker);
    if ids.len() >= MAX_PENDING_UNBONDING_REQUESTS {
        return Err(StakeVaultError::QueueFull);
    }
    let nk = UnbondingKey::NextId(staker.clone());
    let id: u64 = env.storage().persistent().get(&nk).unwrap_or(0);
    env.storage()
        .persistent()
        .set(&nk, &id.checked_add(1).ok_or(StakeVaultError::StakeOverflow)?);
    let req = UnbondingRequest {
        id,
        staker: staker.clone(),
        amount,
        requested_at: env.ledger().timestamp(),
        unlock_at,
        withdrawn: false,
    };
    env.storage()
        .persistent()
        .set(&UnbondingKey::Request(staker.clone(), id), &req);
    ids.push_back(id);
    put_pending_ids(env, staker, &ids);
    Ok(req)
}

/// Mark request `id` withdrawn and drop it from the pending list.
/// Fails if it does not exist, was already withdrawn, or is still locked.
pub fn consume(env: &Env, staker: &Address, id: u64) -> Result<i128, StakeVaultError> {
    let mut req = get(env, staker, id).ok_or(StakeVaultError::NoUnstakeQueued)?;
    if req.withdrawn {
        return Err(StakeVaultError::NoUnstakeQueued);
    }
    if env.ledger().timestamp() < req.unlock_at {
        return Err(StakeVaultError::TimelockNotElapsed);
    }
    req.withdrawn = true;
    env.storage()
        .persistent()
        .set(&UnbondingKey::Request(staker.clone(), id), &req);
    let ids = pending_ids(env, staker);
    let mut kept = Vec::new(env);
    for i in 0..ids.len() {
        let v = ids.get_unchecked(i);
        if v != id {
            kept.push_back(v);
        }
    }
    put_pending_ids(env, staker, &kept);
    Ok(req.amount)
}

/// Ids of pending requests whose unlock time has passed.
pub fn unlocked_ids(env: &Env, staker: &Address) -> Vec<u64> {
    let now = env.ledger().timestamp();
    let reqs = pending_requests(env, staker);
    let mut out = Vec::new(env);
    for i in 0..reqs.len() {
        let r = reqs.get_unchecked(i);
        if now >= r.unlock_at {
            out.push_back(r.id);
        }
    }
    out
}
