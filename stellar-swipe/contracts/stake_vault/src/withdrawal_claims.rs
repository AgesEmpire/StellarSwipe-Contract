//! Queued withdrawal claims (Issue #1133).
//!
//! When an outbound token transfer cannot complete in the original
//! transaction (resource limits, a failing downstream token), the payout is
//! recorded as a queue entry instead of being lost or reverting the whole
//! accounting change. Each entry binds `claimant`, `token`, `amount` and a
//! per-claimant monotonically increasing `nonce`.
//!
//! * **Replay safety**: an entry is marked `Claimed` before the transfer
//!   (CEI); a second claim of the same nonce fails with
//!   `CampaignAllocationNotClaimable`. If the retry transfer itself fails the
//!   call errors and the host reverts the status change, so funds stay
//!   queued.
//! * **Expiry**: after `expires_at` an entry can no longer be claimed, but it
//!   can always be cancelled, which re-credits the claimant's accrued reward
//!   balance — funds are never stranded.

use soroban_sdk::{contracttype, symbol_short, token, Address, Env};

use crate::{reward_index, StakeVaultError};

/// Default lifetime of a queued claim: 30 days.
pub const DEFAULT_CLAIM_TTL_SECS: u64 = 30 * 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum QueuedClaimStatus {
    Pending = 0,
    Claimed = 1,
    Cancelled = 2,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedClaim {
    pub claimant: Address,
    pub token: Address,
    pub amount: i128,
    pub nonce: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub status: QueuedClaimStatus,
}

#[contracttype]
#[derive(Clone)]
pub enum QueuedClaimKey {
    NextNonce(Address),
    Claim(Address, u64),
}

pub fn get(env: &Env, claimant: &Address, nonce: u64) -> Option<QueuedClaim> {
    env.storage()
        .persistent()
        .get(&QueuedClaimKey::Claim(claimant.clone(), nonce))
}

fn put(env: &Env, c: &QueuedClaim) {
    env.storage()
        .persistent()
        .set(&QueuedClaimKey::Claim(c.claimant.clone(), c.nonce), c);
}

fn try_pay(env: &Env, tok: &Address, to: &Address, amount: i128) -> bool {
    matches!(
        token::Client::new(env, tok).try_transfer(&env.current_contract_address(), to, &amount),
        Ok(Ok(()))
    )
}

/// Pay `amount` now, or queue it if the transfer fails.
/// Returns `None` when paid, `Some(nonce)` when queued.
pub fn pay_or_queue(
    env: &Env,
    claimant: &Address,
    tok: &Address,
    amount: i128,
) -> Result<Option<u64>, StakeVaultError> {
    if amount <= 0 {
        return Err(StakeVaultError::InvalidAmount);
    }
    if try_pay(env, tok, claimant, amount) {
        return Ok(None);
    }
    let nk = QueuedClaimKey::NextNonce(claimant.clone());
    let nonce: u64 = env.storage().persistent().get(&nk).unwrap_or(0);
    env.storage()
        .persistent()
        .set(&nk, &nonce.checked_add(1).ok_or(StakeVaultError::StakeOverflow)?);
    let now = env.ledger().timestamp();
    let c = QueuedClaim {
        claimant: claimant.clone(),
        token: tok.clone(),
        amount,
        nonce,
        created_at: now,
        expires_at: now.saturating_add(DEFAULT_CLAIM_TTL_SECS),
        status: QueuedClaimStatus::Pending,
    };
    put(env, &c);
    env.events().publish(
        (symbol_short!("wqueue"), symbol_short!("queued")),
        (claimant.clone(), tok.clone(), amount, nonce, c.expires_at),
    );
    Ok(Some(nonce))
}

fn load_pending(env: &Env, claimant: &Address, nonce: u64) -> Result<QueuedClaim, StakeVaultError> {
    let c = get(env, claimant, nonce).ok_or(StakeVaultError::NoUnstakeQueued)?;
    if c.status != QueuedClaimStatus::Pending {
        return Err(StakeVaultError::CampaignAllocationNotClaimable);
    }
    Ok(c)
}

/// Retry a queued claim. Errors (and reverts) if the transfer still fails.
pub fn claim(env: &Env, claimant: &Address, nonce: u64) -> Result<i128, StakeVaultError> {
    claimant.require_auth();
    let mut c = load_pending(env, claimant, nonce)?;
    if env.ledger().timestamp() >= c.expires_at {
        return Err(StakeVaultError::CampaignAllocationExpired);
    }
    c.status = QueuedClaimStatus::Claimed;
    put(env, &c);
    shared::token_error::map_result(token::Client::new(env, &c.token).try_transfer(
        &env.current_contract_address(),
        claimant,
        &c.amount,
    ))
    .map_err(StakeVaultError::from)?;
    env.events().publish(
        (symbol_short!("wqueue"), symbol_short!("claimed")),
        (claimant.clone(), c.token, c.amount, nonce),
    );
    Ok(c.amount)
}

/// Cancel a pending (possibly expired) claim, re-crediting the amount to the
/// claimant's accrued reward balance so it can be claimed again later.
pub fn cancel(env: &Env, claimant: &Address, nonce: u64) -> Result<i128, StakeVaultError> {
    claimant.require_auth();
    let mut c = load_pending(env, claimant, nonce)?;
    c.status = QueuedClaimStatus::Cancelled;
    put(env, &c);
    reward_index::credit_accrued(env, claimant, &c.token, c.amount)?;
    env.events().publish(
        (symbol_short!("wqueue"), symbol_short!("cancel")),
        (claimant.clone(), c.token, c.amount, nonce),
    );
    Ok(c.amount)
}
