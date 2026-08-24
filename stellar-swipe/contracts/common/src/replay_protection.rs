//! Replay protection: sequential nonces + tx-hash deduplication with 1-hour TTL.
//!
//! Storage layout:
//!   UserNonce(Address)          -> u64   (persistent, per-entry TTL) — current committed nonce
//!   TxHash([u8;32])             -> u64   (persistent, per-entry TTL) — ledger timestamp of execution
//!   PendingQueue                -> Vec<PendingNonceRecord> (persistent) — FIFO index of
//!                                  committed tx hashes so `purge_expired_nonces` can find
//!                                  and evict expired entries without key enumeration.
//!
//! Usage per transaction:
//!   1. `verify_and_commit(env, user, nonce, tx_hash, expiry_ts)` — call once per action.
//!      Returns `Err(ReplayError)` on any violation; on success the nonce is incremented,
//!      the hash is stored, both entries' TTLs are extended to cover `expiry_ts`, and a
//!      `PendingNonceRecord` is appended to the purge queue.
//!   2. `purge_expired_nonces(env, max)` — keeper/admin maintenance call. Scans up to
//!      `max` entries from the front of the purge queue and removes the ones whose
//!      `expiry_ts` has passed, reclaiming persistent storage without breaking the
//!      replay guarantee (only entries already past their usable window are removed).

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Symbol, Vec};

use crate::constants::LEDGER_CLOSE_TIME_SECONDS;

// ── Constants ─────────────────────────────────────────────────────────────────

const TX_HASH_TTL_SECS: u64 = 3_600; // 1 hour

/// Floor applied to per-entry TTL extensions so a `expiry_ts` very close to `now`
/// doesn't produce a near-zero TTL that gets evicted before the tx can even settle.
const MIN_TTL_LEDGERS: u32 = 17_280; // ~24 hours at 5s/ledger, matches shared::auth::NONCE_TTL_LEDGERS

/// Convert a duration in seconds to an approximate ledger count.
fn seconds_to_ledgers(seconds: u64) -> u32 {
    let ledgers = seconds / (LEDGER_CLOSE_TIME_SECONDS as u64);
    core::cmp::max(ledgers.min(u32::MAX as u64) as u32, MIN_TTL_LEDGERS)
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// Submitted nonce does not equal current_nonce + 1.
    InvalidNonce,
    /// tx_hash already present in the executed map.
    DuplicateTx,
    /// Transaction's expiry timestamp is in the past.
    Expired,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum ReplayKey {
    UserNonce(Address),
    TxHash(Bytes),
    /// FIFO queue of committed tx hashes, used by [`purge_expired_nonces`] to find
    /// expired entries without relying on persistent-storage key enumeration
    /// (which Soroban does not support).
    PendingQueue,
}

/// One entry in the [`ReplayKey::PendingQueue`] index.
#[contracttype]
#[derive(Clone)]
pub struct PendingNonceRecord {
    pub user: Address,
    pub tx_hash: Bytes,
    pub expiry_ts: u64,
}

// ── Core API ──────────────────────────────────────────────────────────────────

/// Return the current committed nonce for `user` (0 if never used).
pub fn current_nonce(env: &Env, user: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&ReplayKey::UserNonce(user.clone()))
        .unwrap_or(0)
}

/// Validate and commit a transaction.
///
/// - `nonce`     : must equal `current_nonce(user) + 1`
/// - `tx_hash`   : 32-byte hash of the transaction payload (caller-computed)
/// - `expiry_ts` : unix timestamp after which the tx is considered stale
///
/// On success: nonce is incremented, hash stored with current timestamp.
/// On failure: emits a `replay_attempt` event and returns `Err(ReplayError)`.
pub fn verify_and_commit(
    env: &Env,
    user: &Address,
    nonce: u64,
    tx_hash: Bytes,
    expiry_ts: u64,
) -> Result<(), ReplayError> {
    let now = env.ledger().timestamp();

    // 1. Expiry check
    if now > expiry_ts {
        emit_replay(env, user, &tx_hash, symbol_short!("expired"));
        return Err(ReplayError::Expired);
    }

    // 2. Nonce check
    let expected = current_nonce(env, user) + 1;
    if nonce != expected {
        emit_replay(env, user, &tx_hash, symbol_short!("bad_nonce"));
        return Err(ReplayError::InvalidNonce);
    }

    // 3. Duplicate hash check (only within TTL window)
    let hash_key = ReplayKey::TxHash(tx_hash.clone());
    if let Some(executed_at) = env.storage().persistent().get::<_, u64>(&hash_key) {
        if now.saturating_sub(executed_at) < TX_HASH_TTL_SECS {
            emit_replay(env, user, &tx_hash, symbol_short!("dup_tx"));
            return Err(ReplayError::DuplicateTx);
        }
        // Hash expired — fall through and overwrite
    }

    // 4. Commit
    let nonce_key = ReplayKey::UserNonce(user.clone());
    env.storage().persistent().set(&nonce_key, &nonce);
    env.storage().persistent().set(&hash_key, &now);

    // 5. Extend TTL on both persistent entries to cover the trade's expiry window
    // (Issue: replay attack prevention audit — nonces must not be evicted while
    // still within their usable window).
    let ttl_ledgers = seconds_to_ledgers(expiry_ts.saturating_sub(now));
    env.storage()
        .persistent()
        .extend_ttl(&nonce_key, ttl_ledgers, ttl_ledgers);
    env.storage()
        .persistent()
        .extend_ttl(&hash_key, ttl_ledgers, ttl_ledgers);

    // 6. Index this hash in the purge queue so `purge_expired_nonces` can reclaim it
    // once `expiry_ts` has passed.
    let queue_key = ReplayKey::PendingQueue;
    let mut queue: Vec<PendingNonceRecord> = env
        .storage()
        .persistent()
        .get(&queue_key)
        .unwrap_or_else(|| Vec::new(env));
    queue.push_back(PendingNonceRecord {
        user: user.clone(),
        tx_hash,
        expiry_ts,
    });
    env.storage().persistent().set(&queue_key, &queue);
    env.storage()
        .persistent()
        .extend_ttl(&queue_key, ttl_ledgers, ttl_ledgers);

    Ok(())
}

/// Purge tx-hash entries whose `expiry_ts` has passed, scanning at most `max` entries
/// from the front of the purge queue.
///
/// This only removes entries that are objectively past their usable window — it
/// never touches an entry that could still be legitimately referenced, so it cannot
/// weaken the replay guarantee. Entries within the scanned window that are not yet
/// expired are kept in the queue for a future purge pass.
///
/// Returns the number of entries removed.
pub fn purge_expired_nonces(env: &Env, max: u32) -> u32 {
    let now = env.ledger().timestamp();
    let queue_key = ReplayKey::PendingQueue;
    let queue: Vec<PendingNonceRecord> = env
        .storage()
        .persistent()
        .get(&queue_key)
        .unwrap_or_else(|| Vec::new(env));

    let mut remaining: Vec<PendingNonceRecord> = Vec::new(env);
    let mut purged: u32 = 0;

    for (i, record) in queue.iter().enumerate() {
        if (i as u32) < max && record.expiry_ts < now {
            let hash_key = ReplayKey::TxHash(record.tx_hash.clone());
            env.storage().persistent().remove(&hash_key);
            emit_purged(env, &record.user, &record.tx_hash);
            purged = purged.saturating_add(1);
        } else {
            remaining.push_back(record.clone());
        }
    }

    env.storage().persistent().set(&queue_key, &remaining);
    purged
}

// ── Event ─────────────────────────────────────────────────────────────────────

fn emit_replay(env: &Env, user: &Address, tx_hash: &Bytes, reason: soroban_sdk::Symbol) {
    let topics = (Symbol::new(env, "replay_detected"),);
    env.events()
        .publish(topics, (user.clone(), reason, tx_hash.clone()));
}

fn emit_purged(env: &Env, user: &Address, tx_hash: &Bytes) {
    let topics = (Symbol::new(env, "nonce_purged"),);
    env.events()
        .publish(topics, (user.clone(), tx_hash.clone()));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl,
        testutils::{Address as _, Ledger},
        Address, Bytes, Env,
    };

    #[contract]
    struct ReplayHarness;

    #[contractimpl]
    impl ReplayHarness {}

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        let contract_id = env.register(ReplayHarness, ());
        let user = Address::generate(&env);
        (env, contract_id, user)
    }

    fn run<F, R>(env: &Env, contract_id: &Address, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        env.as_contract(contract_id, f)
    }

    fn hash(env: &Env, seed: u8) -> Bytes {
        Bytes::from_array(env, &[seed; 32])
    }

    fn far_future(env: &Env) -> u64 {
        env.ledger().timestamp() + 7_200
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    #[test]
    fn sequential_nonces_succeed() {
        let (env, contract_id, user) = setup();
        run(&env, &contract_id, || {
            for i in 1u64..=5 {
                assert_eq!(
                    verify_and_commit(&env, &user, i, hash(&env, i as u8), far_future(&env)),
                    Ok(())
                );
            }
            assert_eq!(current_nonce(&env, &user), 5);
        });
    }

    // ── Replay via hash ───────────────────────────────────────────────────────

    #[test]
    fn duplicate_tx_rejected() {
        let (env, contract_id, user) = setup();
        run(&env, &contract_id, || {
            let h = hash(&env, 1);
            verify_and_commit(&env, &user, 1, h.clone(), far_future(&env)).unwrap();
            // Resubmit same hash with next nonce — still rejected as duplicate
            let err = verify_and_commit(&env, &user, 2, h, far_future(&env));
            assert_eq!(err, Err(ReplayError::DuplicateTx));
        });
    }

    // ── Nonce gap ─────────────────────────────────────────────────────────────

    #[test]
    fn skipped_nonce_rejected() {
        let (env, contract_id, user) = setup();
        run(&env, &contract_id, || {
            verify_and_commit(&env, &user, 1, hash(&env, 1), far_future(&env)).unwrap();
            // Jump to nonce 3 — should fail
            let err = verify_and_commit(&env, &user, 3, hash(&env, 3), far_future(&env));
            assert_eq!(err, Err(ReplayError::InvalidNonce));
        });
    }

    #[test]
    fn repeated_nonce_rejected() {
        let (env, contract_id, user) = setup();
        run(&env, &contract_id, || {
            verify_and_commit(&env, &user, 1, hash(&env, 1), far_future(&env)).unwrap();
            let err = verify_and_commit(&env, &user, 1, hash(&env, 2), far_future(&env));
            assert_eq!(err, Err(ReplayError::InvalidNonce));
        });
    }

    // ── Expiry ────────────────────────────────────────────────────────────────

    #[test]
    fn expired_tx_rejected() {
        let (env, contract_id, user) = setup();
        run(&env, &contract_id, || {
            env.ledger().set_timestamp(1_000);
            let err = verify_and_commit(&env, &user, 1, hash(&env, 1), 999);
            assert_eq!(err, Err(ReplayError::Expired));
        });
    }

    // ── Hash TTL expiry allows reuse ──────────────────────────────────────────

    #[test]
    fn hash_reusable_after_ttl() {
        let (env, contract_id, user) = setup();
        run(&env, &contract_id, || {
            let h = hash(&env, 42);
            verify_and_commit(&env, &user, 1, h.clone(), far_future(&env)).unwrap();

            // Advance past 1-hour TTL
            env.ledger()
                .set_timestamp(env.ledger().timestamp() + TX_HASH_TTL_SECS + 1);

            // Same hash is now allowed (TTL expired); nonce must be 2
            assert_eq!(
                verify_and_commit(&env, &user, 2, h, far_future(&env)),
                Ok(())
            );
        });
    }

    // ── Independent users ─────────────────────────────────────────────────────

    #[test]
    fn users_have_independent_nonces() {
        let (env, contract_id, user1) = setup();
        run(&env, &contract_id, || {
            let user2 = Address::generate(&env);

            verify_and_commit(&env, &user1, 1, hash(&env, 1), far_future(&env)).unwrap();
            verify_and_commit(&env, &user1, 2, hash(&env, 2), far_future(&env)).unwrap();

            // user2 starts at nonce 1 regardless of user1
            assert_eq!(
                verify_and_commit(&env, &user2, 1, hash(&env, 3), far_future(&env)),
                Ok(())
            );
        });
    }
}
