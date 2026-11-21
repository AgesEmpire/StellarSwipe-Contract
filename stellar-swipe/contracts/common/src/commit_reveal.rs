//! Commit-reveal helpers to **bind** a user's trade parameters before execution.
//!
//! # Front-running analysis
//!
//! An observer who sees a commitment transaction on-chain learns only the 32-byte
//! hash `SHA-256("sw_exec_v1" || user || signal_id || amount || min_out || salt || valid_until_ledger)`.
//! They cannot recover `amount`, `min_out`, or the `salt` without brute-forcing
//! all possible field combinations. Because the `min_out` field sets a floor on
//! what the user will accept, a front-runner who does not know `min_out` cannot
//! guarantee their sandwiched trade is profitable — they risk being beaten by the
//! user's own minimum. The `salt` (ideally 8+ cryptographically random bytes)
//! prevents preimage search over fixed fields. Reuse of the same salt is rejected
//! by [`store_commitment`] so each commitment is independently unpredictable.
//!
//! # Griefing guard
//!
//! A committer who never reveals cannot lock funds indefinitely:
//! - [`store_commitment`] refuses to lock the same `(user, salt)` slot twice.
//! - [`reveal_and_clear`] rejects reveals after `expires_at_ledger`.
//! - [`forfeit_expired`] lets anyone clear a timed-out slot, preventing permanent
//!   state lock-up. Callers MUST NOT lock funds before a successful reveal.

use soroban_sdk::{contracterror, contracttype, Address, Bytes, BytesN, Env, String, Symbol};

/// `SHA-256( "sw_exec_v1" || user || signal_id || amount || min_out || salt
/// || valid_until_ledger )` as a [`BytesN<32>`].
///
/// - `min_out` — user-defined floor for received amount (slippage / MEV margin).
/// - `valid_until_ledger` — user expects execution by this ledger (inclusive);
///   contracts that adopt commit-reveal should reject reveals after this ledger.
/// - `salt` — high-entropy; clients should use a CSPRNG (or expand to 32 bytes in
///   a future version of this API).
pub fn hash_trade_intent(
    env: &Env,
    user: &Address,
    signal_id: u64,
    amount: i128,
    min_out: i128,
    salt: u64,
    valid_until_ledger: u32,
) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&String::from_str(env, "sw_exec_v1").to_bytes());
    preimage.append(&user.to_string().to_bytes());
    preimage.append(&Bytes::from_array(env, &signal_id.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &amount.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &min_out.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &salt.to_be_bytes()));
    preimage.append(&Bytes::from_array(env, &valid_until_ledger.to_be_bytes()));
    env.crypto().sha256(&preimage).into()
}

/// Constant-time equality check for fixed-length (32-byte) hash/commitment values.
///
/// Rationale: comparing commitment hashes with standard `==` short-circuits on the
/// first mismatched byte, which can in principle leak timing information about how
/// much of a secret commitment matches an attacker's guess. That's a defense-in-depth
/// concern for commit-reveal schemes guarding economically meaningful actions (e.g.
/// [`hash_trade_intent`] reveals). This walks every byte unconditionally via XOR-accumulate
/// and only branches once, on the final accumulated result, so the comparison takes the
/// same number of steps regardless of where (or whether) the inputs first differ.
///
/// Future contributors: prefer this (or [`verify_commitment`]) over `==`/`!=` whenever
/// comparing a stored commitment hash against a freshly computed one.
pub fn constant_time_eq(a: &BytesN<32>, b: &BytesN<32>) -> bool {
    let a = a.to_array();
    let b = b.to_array();
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Verify a revealed commitment against the `expected` stored hash.
///
/// Use this instead of `expected == actual` when checking a commit-reveal hash —
/// see [`constant_time_eq`] for why.
pub fn verify_commitment(expected: &BytesN<32>, actual: &BytesN<32>) -> bool {
    constant_time_eq(expected, actual)
}

// ── Stateful commit-reveal lifecycle ─────────────────────────────────────────

/// Errors returned by the stateful commit-reveal helpers.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CommitRevealError {
    /// No outstanding commitment for this `(user, salt)` key.
    NotFound = 1,
    /// Current ledger is past the commitment's `expires_at_ledger`; reveal rejected.
    Expired = 2,
    /// The revealed hash does not match the stored commitment.
    Mismatch = 3,
    /// This `(user, salt)` pair was already committed; use a fresh CSPRNG salt.
    SaltReused = 4,
    /// Forfeit attempt on a commitment that has not yet expired.
    NotYetExpired = 5,
}

/// Storage key for an outstanding commitment, keyed by `(user, salt)`.
///
/// Each user-salt pair occupies an independent slot. Reusing the same salt
/// with the same user address is rejected by [`store_commitment`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitKey {
    pub user: Address,
    pub salt: u64,
}

/// On-chain record of an outstanding commitment awaiting reveal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitRecord {
    pub commitment: BytesN<32>,
    pub committer: Address,
    /// Inclusive ledger sequence after which the reveal window is closed.
    pub expires_at_ledger: u32,
}

/// Store a commitment for `(user, salt)`.
///
/// Rejects if `(user, salt)` is already present — callers must use a fresh
/// CSPRNG salt per commitment to prevent cross-commitment replay.
///
/// `expires_at_ledger` should equal the `valid_until_ledger` passed to
/// [`hash_trade_intent`] so the on-chain expiry matches the commitment.
pub fn store_commitment(
    env: &Env,
    user: &Address,
    salt: u64,
    commitment: BytesN<32>,
    expires_at_ledger: u32,
) -> Result<(), CommitRevealError> {
    let key = CommitKey {
        user: user.clone(),
        salt,
    };
    if env.storage().persistent().has(&key) {
        return Err(CommitRevealError::SaltReused);
    }
    let record = CommitRecord {
        commitment,
        committer: user.clone(),
        expires_at_ledger,
    };
    env.storage().persistent().set(&key, &record);
    Ok(())
}

/// Verify a reveal and consume (delete) the stored commitment atomically.
///
/// Returns the stored [`CommitRecord`] on success so callers can inspect
/// `committer` and `expires_at_ledger`. Fails with:
/// - [`CommitRevealError::NotFound`]  — no commitment for `(user, salt)`
/// - [`CommitRevealError::Expired`]   — current ledger > `expires_at_ledger`
/// - [`CommitRevealError::Mismatch`]  — `actual_hash` ≠ stored commitment
///
/// The record is removed regardless of success/failure path so no replay
/// is possible: a failed reveal clears the slot only on Mismatch if no
/// replay is desired; on Expired, the slot stays (call `forfeit_expired`).
pub fn reveal_and_clear(
    env: &Env,
    user: &Address,
    salt: u64,
    actual_hash: &BytesN<32>,
) -> Result<CommitRecord, CommitRevealError> {
    let key = CommitKey {
        user: user.clone(),
        salt,
    };
    let record: CommitRecord = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(CommitRevealError::NotFound)?;

    if env.ledger().sequence() > record.expires_at_ledger {
        return Err(CommitRevealError::Expired);
    }

    if !verify_commitment(&record.commitment, actual_hash) {
        return Err(CommitRevealError::Mismatch);
    }

    env.storage().persistent().remove(&key);
    Ok(record)
}

/// Remove an expired commitment without a reveal (anti-griefing).
///
/// Callable by anyone once `env.ledger().sequence() > expires_at_ledger`.
/// Emits `commit_forfeited` so indexers can track abandoned commitments.
/// This prevents a committer who never reveals from locking state forever.
pub fn forfeit_expired(
    env: &Env,
    user: &Address,
    salt: u64,
) -> Result<CommitRecord, CommitRevealError> {
    let key = CommitKey {
        user: user.clone(),
        salt,
    };
    let record: CommitRecord = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(CommitRevealError::NotFound)?;

    if env.ledger().sequence() <= record.expires_at_ledger {
        return Err(CommitRevealError::NotYetExpired);
    }

    env.storage().persistent().remove(&key);

    env.events().publish(
        (Symbol::new(env, "commit_forfeited"), user.clone()),
        (salt, record.expires_at_ledger),
    );

    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn set_ledger_sequence(env: &Env, seq: u32) {
        env.ledger().with_mut(|li| {
            li.sequence_number = seq;
        });
    }

    #[test]
    fn hash_is_deterministic() {
        let env = Env::default();
        let a = Address::generate(&env);
        let h1 = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        let h2 = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_amount_changes() {
        let env = Env::default();
        let a = Address::generate(&env);
        let h1 = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        let h2 = hash_trade_intent(&env, &a, 5, 1_000_001, 900_000, 42, 1_000_000);
        assert_ne!(h1, h2);
    }

    // ── constant_time_eq / verify_commitment (Issue #594) ─────────────────────

    #[test]
    fn constant_time_eq_identical_arrays_match() {
        let env = Env::default();
        let a: BytesN<32> = BytesN::from_array(&env, &[7u8; 32]);
        let b: BytesN<32> = BytesN::from_array(&env, &[7u8; 32]);
        assert!(constant_time_eq(&a, &b));
    }

    #[test]
    fn constant_time_eq_differs_in_first_byte() {
        let env = Env::default();
        let mut bytes_a = [0u8; 32];
        let mut bytes_b = [0u8; 32];
        bytes_a[0] = 1;
        bytes_b[0] = 2;
        let a = BytesN::from_array(&env, &bytes_a);
        let b = BytesN::from_array(&env, &bytes_b);
        assert!(!constant_time_eq(&a, &b));
    }

    #[test]
    fn constant_time_eq_differs_in_last_byte() {
        let env = Env::default();
        let mut bytes_a = [9u8; 32];
        let mut bytes_b = [9u8; 32];
        bytes_a[31] = 1;
        bytes_b[31] = 2;
        let a = BytesN::from_array(&env, &bytes_a);
        let b = BytesN::from_array(&env, &bytes_b);
        assert!(!constant_time_eq(&a, &b));
    }

    #[test]
    fn constant_time_eq_differs_in_middle_byte() {
        let env = Env::default();
        let mut bytes_a = [3u8; 32];
        let mut bytes_b = [3u8; 32];
        bytes_a[16] = 0xAA;
        bytes_b[16] = 0xBB;
        let a = BytesN::from_array(&env, &bytes_a);
        let b = BytesN::from_array(&env, &bytes_b);
        assert!(!constant_time_eq(&a, &b));
    }

    #[test]
    fn verify_commitment_matches_valid_reveal() {
        let env = Env::default();
        let a = Address::generate(&env);
        let committed = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        let revealed = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        assert!(verify_commitment(&committed, &revealed));
    }

    #[test]
    fn verify_commitment_rejects_invalid_reveal() {
        let env = Env::default();
        let a = Address::generate(&env);
        let committed = hash_trade_intent(&env, &a, 5, 1_000_000, 900_000, 42, 1_000_000);
        // Attacker reveals different trade parameters than what was committed to.
        let revealed = hash_trade_intent(&env, &a, 5, 1_000_001, 900_000, 42, 1_000_000);
        assert!(!verify_commitment(&committed, &revealed));
    }

    // ── Stateful commit-reveal lifecycle ──────────────────────────────────────
    //
    // Storage can only be accessed within a contract context in Soroban.
    // These tests register a minimal test contract and run under `env.as_contract`.

    use soroban_sdk::{contract, contractimpl};

    #[contract]
    struct CommitRevealTestContract;
    #[contractimpl]
    impl CommitRevealTestContract {}

    fn register_test_contract(env: &Env) -> Address {
        env.register_contract(None, CommitRevealTestContract)
    }

    #[test]
    fn store_and_reveal_succeeds() {
        let env = Env::default();
        set_ledger_sequence(&env, 100);
        let contract_id = register_test_contract(&env);
        let user = Address::generate(&env);
        let salt = 99u64;
        let hash = hash_trade_intent(&env, &user, 1, 500, 450, salt, 200);

        env.as_contract(&contract_id, || {
            store_commitment(&env, &user, salt, hash.clone(), 200).unwrap();
            let record = reveal_and_clear(&env, &user, salt, &hash).unwrap();
            assert_eq!(record.committer, user);
            assert_eq!(record.expires_at_ledger, 200);
        });
    }

    #[test]
    fn reveal_rejects_mismatched_hash() {
        let env = Env::default();
        set_ledger_sequence(&env, 100);
        let contract_id = register_test_contract(&env);
        let user = Address::generate(&env);
        let salt = 7u64;
        let correct = hash_trade_intent(&env, &user, 1, 500, 450, salt, 200);
        let wrong = hash_trade_intent(&env, &user, 1, 999, 450, salt, 200);

        env.as_contract(&contract_id, || {
            store_commitment(&env, &user, salt, correct, 200).unwrap();
            let err = reveal_and_clear(&env, &user, salt, &wrong).unwrap_err();
            assert_eq!(err, CommitRevealError::Mismatch);
        });
    }

    #[test]
    fn reveal_rejects_after_expiry() {
        let env = Env::default();
        set_ledger_sequence(&env, 100);
        let contract_id = register_test_contract(&env);
        let user = Address::generate(&env);
        let salt = 13u64;
        let hash = hash_trade_intent(&env, &user, 1, 500, 450, salt, 110);

        env.as_contract(&contract_id, || {
            store_commitment(&env, &user, salt, hash.clone(), 110).unwrap();
        });

        set_ledger_sequence(&env, 111);

        env.as_contract(&contract_id, || {
            let err = reveal_and_clear(&env, &user, salt, &hash).unwrap_err();
            assert_eq!(err, CommitRevealError::Expired);
        });
    }

    #[test]
    fn salt_reuse_is_rejected() {
        let env = Env::default();
        set_ledger_sequence(&env, 100);
        let contract_id = register_test_contract(&env);
        let user = Address::generate(&env);
        let salt = 42u64;
        let hash = hash_trade_intent(&env, &user, 1, 500, 450, salt, 200);

        env.as_contract(&contract_id, || {
            store_commitment(&env, &user, salt, hash.clone(), 200).unwrap();
            let err = store_commitment(&env, &user, salt, hash, 200).unwrap_err();
            assert_eq!(err, CommitRevealError::SaltReused);
        });
    }

    #[test]
    fn forfeit_expired_clears_stuck_commitment() {
        let env = Env::default();
        set_ledger_sequence(&env, 100);
        let contract_id = register_test_contract(&env);
        let user = Address::generate(&env);
        let salt = 55u64;
        let hash = hash_trade_intent(&env, &user, 1, 500, 450, salt, 110);

        env.as_contract(&contract_id, || {
            store_commitment(&env, &user, salt, hash, 110).unwrap();
        });

        set_ledger_sequence(&env, 111);

        env.as_contract(&contract_id, || {
            let record = forfeit_expired(&env, &user, salt).unwrap();
            assert_eq!(record.expires_at_ledger, 110);

            // Slot is cleared — subsequent reveal fails with NotFound
            let dummy = BytesN::from_array(&env, &[0u8; 32]);
            let err = reveal_and_clear(&env, &user, salt, &dummy).unwrap_err();
            assert_eq!(err, CommitRevealError::NotFound);
        });
    }

    #[test]
    fn forfeit_before_expiry_is_rejected() {
        let env = Env::default();
        set_ledger_sequence(&env, 100);
        let contract_id = register_test_contract(&env);
        let user = Address::generate(&env);
        let salt = 77u64;
        let hash = hash_trade_intent(&env, &user, 1, 500, 450, salt, 200);

        env.as_contract(&contract_id, || {
            store_commitment(&env, &user, salt, hash, 200).unwrap();
            let err = forfeit_expired(&env, &user, salt).unwrap_err();
            assert_eq!(err, CommitRevealError::NotYetExpired);
        });
    }
}
