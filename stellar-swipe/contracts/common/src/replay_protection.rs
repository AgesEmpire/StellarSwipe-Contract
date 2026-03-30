//! Replay protection: sequential nonces + tx-hash deduplication with 1-hour TTL.
//!
//! Storage layout:
//!   UserNonce(Address)          -> u64   (persistent) — current committed nonce
//!   TxHash([u8;32])             -> u64   (persistent) — ledger timestamp of execution
//!
//! Usage per transaction:
//!   1. `verify_and_commit(env, user, nonce, tx_hash, expiry_ts)` — call once per action.
//!      Returns `Err(ReplayError)` on any violation; on success the nonce is incremented
//!      and the hash is stored.

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env};

// ── Constants ─────────────────────────────────────────────────────────────────

const TX_HASH_TTL_SECS: u64 = 3_600; // 1 hour

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
    if let Some(executed_at) = env
        .storage()
        .persistent()
        .get::<_, u64>(&hash_key)
    {
        if now.saturating_sub(executed_at) < TX_HASH_TTL_SECS {
            emit_replay(env, user, &tx_hash, symbol_short!("dup_tx"));
            return Err(ReplayError::DuplicateTx);
        }
        // Hash expired — fall through and overwrite
    }

    // 4. Commit
    env.storage()
        .persistent()
        .set(&ReplayKey::UserNonce(user.clone()), &nonce);
    env.storage()
        .persistent()
        .set(&hash_key, &now);

    Ok(())
}

// ── Event ─────────────────────────────────────────────────────────────────────

fn emit_replay(env: &Env, user: &Address, tx_hash: &Bytes, reason: soroban_sdk::Symbol) {
    let topics = (symbol_short!("replay"), user.clone(), reason);
    env.events().publish(topics, tx_hash.clone());
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger}, Bytes, Env};

    fn env_user() -> (Env, Address) {
        let env = Env::default();
        let user = Address::generate(&env);
        (env, user)
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
        let (env, user) = env_user();
        for i in 1u64..=5 {
            assert_eq!(
                verify_and_commit(&env, &user, i, hash(&env, i as u8), far_future(&env)),
                Ok(())
            );
        }
        assert_eq!(current_nonce(&env, &user), 5);
    }

    // ── Replay via hash ───────────────────────────────────────────────────────

    #[test]
    fn duplicate_tx_rejected() {
        let (env, user) = env_user();
        let h = hash(&env, 1);
        verify_and_commit(&env, &user, 1, h.clone(), far_future(&env)).unwrap();
        // Resubmit same hash with next nonce — still rejected as duplicate
        let err = verify_and_commit(&env, &user, 2, h, far_future(&env));
        assert_eq!(err, Err(ReplayError::DuplicateTx));
    }

    // ── Nonce gap ─────────────────────────────────────────────────────────────

    #[test]
    fn skipped_nonce_rejected() {
        let (env, user) = env_user();
        verify_and_commit(&env, &user, 1, hash(&env, 1), far_future(&env)).unwrap();
        // Jump to nonce 3 — should fail
        let err = verify_and_commit(&env, &user, 3, hash(&env, 3), far_future(&env));
        assert_eq!(err, Err(ReplayError::InvalidNonce));
    }

    #[test]
    fn repeated_nonce_rejected() {
        let (env, user) = env_user();
        verify_and_commit(&env, &user, 1, hash(&env, 1), far_future(&env)).unwrap();
        let err = verify_and_commit(&env, &user, 1, hash(&env, 2), far_future(&env));
        assert_eq!(err, Err(ReplayError::InvalidNonce));
    }

    // ── Expiry ────────────────────────────────────────────────────────────────

    #[test]
    fn expired_tx_rejected() {
        let (env, user) = env_user();
        let past = env.ledger().timestamp().saturating_sub(1);
        let err = verify_and_commit(&env, &user, 1, hash(&env, 1), past);
        assert_eq!(err, Err(ReplayError::Expired));
    }

    // ── Hash TTL expiry allows reuse ──────────────────────────────────────────

    #[test]
    fn hash_reusable_after_ttl() {
        let (env, user) = env_user();
        let h = hash(&env, 42);
        verify_and_commit(&env, &user, 1, h.clone(), far_future(&env)).unwrap();

        // Advance past 1-hour TTL
        env.ledger().set_timestamp(env.ledger().timestamp() + TX_HASH_TTL_SECS + 1);

        // Same hash is now allowed (TTL expired); nonce must be 2
        assert_eq!(
            verify_and_commit(&env, &user, 2, h, far_future(&env)),
            Ok(())
        );
    }

    // ── Independent users ─────────────────────────────────────────────────────

    #[test]
    fn users_have_independent_nonces() {
        let (env, user1) = env_user();
        let user2 = Address::generate(&env);

        verify_and_commit(&env, &user1, 1, hash(&env, 1), far_future(&env)).unwrap();
        verify_and_commit(&env, &user1, 2, hash(&env, 2), far_future(&env)).unwrap();

        // user2 starts at nonce 1 regardless of user1
        assert_eq!(
            verify_and_commit(&env, &user2, 1, hash(&env, 3), far_future(&env)),
            Ok(())
        );
    }
}
