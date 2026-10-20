//! Deterministic replay protection for cross-contract signal submissions.
//!
//! A signal submission that crosses a contract boundary is bound to a
//! deterministic digest computed from:
//!
//! * the provider identity (`provider`),
//! * the signal payload (`payload`),
//! * the caller context (`caller`),
//! * the intended registry address (`registry`).
//!
//! The digest is the replay-protection key. A digest is only marked as
//! consumed once the submission has been accepted, so a failed submission
//! never consumes a nonce and can be safely retried with the same inputs.
//!
//! ## Digest format (for SDK and indexer authors)
//!
//! ```text
//! digest = sha256(
//!     "signal-replay-v1" || 0x00 ||
//!     len(provider) as u32_be || provider ||
//!     len(payload)  as u32_be || payload  ||
//!     len(caller)   as u32_be || caller   ||
//!     len(registry) as u32_be || registry
//! )
//! ```
//!
//! Each field is length-prefixed with a big-endian `u32` so that distinct
//! field boundaries can never collide. The domain-separation tag
//! `"signal-replay-v1"` prevents cross-protocol digest reuse.
//!
//! ## Bounded nonce compaction
//!
//! Consumed digests are stored in a bounded window so storage growth is
//! capped. Compaction evicts the *oldest* consumed digests once the window is
//! full. Eviction never reopens a claim that is still inside the window, so
//! duplicate and reordered claims within the retained window remain rejected.
//!
//! Behavior is explicitly defined by age:
//!
//! * **Newer nonces** (within the most recent `capacity` consumed digests)
//!   are retained and any replay of them is rejected with
//!   [`ReplayError::AlreadyConsumed`].
//! * **Older nonces** (evicted by compaction) are no longer tracked; a claim
//!   whose digest has aged out of the window is treated as fresh. Callers that
//!   require unbounded protection must persist evicted digests off-chain.
//!
//! ## Idempotency keys for privileged operations
//!
//! Privileged operations (e.g. registry administration) may be retried after
//! an ambiguous transaction submission outcome, where the caller cannot tell
//! whether the transaction was applied. Such operations are keyed by an
//! explicit, caller-supplied [`IdempotencyKey`] rather than by the derived
//! submission digest.
//!
//! * **Scope** — a key is scoped to the privileged operation it guards. The
//!   same key used for a different operation is a distinct claim and does not
//!   collide. Keys are opaque caller-chosen bytes; callers should use a
//!   collision-resistant value (e.g. a UUID or a hash of the request).
//! * **Retention** — consumed keys are retained in the same bounded window as
//!   submission digests (see [`DEFAULT_REPLAY_CAPACITY`]). Once a key ages out
//!   of the window it is no longer tracked and may be reused; callers that
//!   require unbounded protection must persist evicted keys off-chain.
//! * **Collision behavior** — a repeated key for the same operation is
//!   rejected with [`ReplayError::AlreadyConsumed`] and the operation is not
//!   applied a second time. A key is only consumed after the operation
//!   succeeds, so a failed execution leaves the key unconsumed and the
//!   operation may be retried with the same key.

use std::collections::{BTreeSet, VecDeque};

/// Domain-separation tag mixed into every digest.
const DOMAIN_TAG: &[u8] = b"signal-replay-v1";

/// Domain-separation tag mixed into every idempotency key.
const IDEMPOTENCY_DOMAIN_TAG: &[u8] = b"signal-idempotency-v1";

/// Default number of consumed digests retained before compaction evicts the
/// oldest entries.
pub const DEFAULT_REPLAY_CAPACITY: usize = 4096;

/// Deterministic replay-protection key for a cross-contract signal submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubmissionDigest(pub [u8; 32]);

impl SubmissionDigest {
    /// Returns the raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Caller-supplied idempotency key for a privileged operation.
///
/// The key is scoped to `operation` so that the same key value used for a
/// different privileged operation is treated as a distinct claim. See the
/// module docs for scope, retention and collision behavior.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdempotencyKey {
    /// Identifier of the privileged operation the key is scoped to.
    pub operation: Vec<u8>,
    /// Opaque caller-chosen key value.
    pub key: Vec<u8>,
}

impl IdempotencyKey {
    /// Creates a key scoped to `operation` with the given opaque `key` value.
    pub fn new(operation: impl Into<Vec<u8>>, key: impl Into<Vec<u8>>) -> Self {
        Self {
            operation: operation.into(),
            key: key.into(),
        }
    }

    /// Computes the deterministic digest binding the operation scope and key.
    ///
    /// The digest is domain-separated from submission digests so an
    /// idempotency key can never collide with a submission digest.
    pub fn digest(&self) -> SubmissionDigest {
        let mut hasher = Sha256::new();
        hasher.update(IDEMPOTENCY_DOMAIN_TAG);
        hasher.update([0u8]);
        for field in [self.operation.as_slice(), self.key.as_slice()] {
            hasher.update((field.len() as u32).to_be_bytes());
            hasher.update(field);
        }
        SubmissionDigest(hasher.finalize())
    }
}

/// Inputs that fully determine a submission's replay-protection digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionContext<'a> {
    /// Identity of the provider that produced the signal.
    pub provider: &'a [u8],
    /// Opaque signal payload.
    pub payload: &'a [u8],
    /// Caller context (e.g. originating contract / account).
    pub caller: &'a [u8],
    /// Address of the registry the submission is intended for.
    pub registry: &'a [u8],
}

impl<'a> SubmissionContext<'a> {
    /// Computes the deterministic digest binding provider, payload, caller and
    /// intended registry.
    pub fn digest(&self) -> SubmissionDigest {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_TAG);
        hasher.update([0u8]);
        for field in [self.provider, self.payload, self.caller, self.registry] {
            hasher.update((field.len() as u32).to_be_bytes());
            hasher.update(field);
        }
        SubmissionDigest(hasher.finalize())
    }
}

/// Tracks which submission digests have already been accepted.
///
/// Digests are only recorded on success, so failed submissions leave no
/// trace and may be retried with identical inputs. Storage is bounded: once
/// `capacity` digests are retained, committing a new digest evicts the oldest
/// one (see the module docs for the older/newer nonce contract).
#[derive(Clone, Debug)]
pub struct ReplayProtection {
    consumed: BTreeSet<SubmissionDigest>,
    /// Insertion order of consumed digests, used to evict the oldest entries
    /// during compaction.
    order: VecDeque<SubmissionDigest>,
    capacity: usize,
}

impl Default for ReplayProtection {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when a submission is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The digest was already accepted by a previous successful submission.
    AlreadyConsumed,
}

impl ReplayProtection {
    /// Creates an empty replay-protection tracker with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_REPLAY_CAPACITY)
    }

    /// Creates an empty tracker retaining at most `capacity` consumed digests.
    ///
    /// A capacity of `0` disables retention; every claim is treated as fresh.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            consumed: BTreeSet::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    /// Returns the maximum number of consumed digests retained.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of currently retained consumed digests.
    pub fn len(&self) -> usize {
        self.consumed.len()
    }

    /// Returns `true` if no consumed digests are currently retained.
    pub fn is_empty(&self) -> bool {
        self.consumed.is_empty()
    }

    /// Returns `true` if the digest has already been accepted and is still
    /// within the retained window.
    pub fn is_consumed(&self, digest: &SubmissionDigest) -> bool {
        self.consumed.contains(digest)
    }

    /// Checks a submission for replay without consuming its digest.
    ///
    /// Call this before executing the submission. A failed execution leaves
    /// the digest unconsumed, so the same submission can be retried.
    pub fn check(&self, ctx: &SubmissionContext<'_>) -> Result<SubmissionDigest, ReplayError> {
        let digest = ctx.digest();
        if self.consumed.contains(&digest) {
            return Err(ReplayError::AlreadyConsumed);
        }
        Ok(digest)
    }

    /// Marks a submission as accepted, consuming its digest.
    ///
    /// Must only be called after the submission has succeeded. Returns the
    /// digest that was consumed. If the retained window is full, the oldest
    /// consumed digest is evicted to keep storage bounded.
    pub fn commit(&mut self, ctx: &SubmissionContext<'_>) -> SubmissionDigest {
        let digest = ctx.digest();
        self.record(digest);
        digest
    }

    /// Checks a privileged operation for replay without consuming its key.
    ///
    /// Call this before executing the operation. A failed execution leaves
    /// the key unconsumed, so the operation can be retried with the same key.
    pub fn check_key(&self, key: &IdempotencyKey) -> Result<SubmissionDigest, ReplayError> {
        let digest = key.digest();
        if self.consumed.contains(&digest) {
            return Err(ReplayError::AlreadyConsumed);
        }
        Ok(digest)
    }

    /// Marks a privileged operation as applied, consuming its idempotency key.
    ///
    /// Must only be called after the operation has succeeded. Returns the
    /// digest that was consumed. If the retained window is full, the oldest
    /// consumed entry is evicted to keep storage bounded.
    pub fn commit_key(&mut self, key: &IdempotencyKey) -> SubmissionDigest {
        let digest = key.digest();
        self.record(digest);
        digest
    }

    /// Convenience wrapper for privileged operations: checks the idempotency
    /// key, runs `execute`, and only consumes the key if `execute` succeeds.
    ///
    /// A repeated key for the same operation is rejected with
    /// [`ReplayError::AlreadyConsumed`] and `execute` is not run, so the
    /// operation cannot be applied twice. On failure the key is not consumed
    /// and the error is propagated so the caller can retry the same operation.
    pub fn submit_key<F, E>(
        &mut self,
        key: &IdempotencyKey,
        execute: F,
    ) -> Result<SubmissionDigest, SubmitError<E>>
    where
        F: FnOnce() -> Result<(), E>,
    {
        let digest = self.check_key(key).map_err(SubmitError::Replay)?;
        execute().map_err(SubmitError::Execute)?;
        self.record(digest);
        Ok(digest)
    }

    /// Inserts `digest` into the retained window, compacting if necessary.
    fn record(&mut self, digest: SubmissionDigest) {
        if self.capacity == 0 {
            return;
        }
        if self.consumed.insert(digest) {
            self.order.push_back(digest);
        }
        self.compact();
    }

    /// Evicts the oldest consumed digests until the window fits `capacity`.
    ///
    /// Eviction only removes digests that have aged out of the window; digests
    /// still retained are never reopened, so duplicate and reordered claims
    /// within the window remain rejected.
    fn compact(&mut self) {
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.consumed.remove(&oldest);
            }
        }
    }

    /// Convenience wrapper: checks the submission, runs `execute`, and only
    /// consumes the digest if `execute` succeeds.
    ///
    /// On failure the digest is not consumed and the error is propagated so
    /// the caller can retry the same submission.
    pub fn submit<F, E>(
        &mut self,
        ctx: &SubmissionContext<'_>,
        execute: F,
    ) -> Result<SubmissionDigest, SubmitError<E>>
    where
        F: FnOnce() -> Result<(), E>,
    {
        let digest = self.check(ctx).map_err(SubmitError::Replay)?;
        execute().map_err(SubmitError::Execute)?;
        self.record(digest);
        Ok(digest)
    }
}

/// Error returned by [`ReplayProtection::submit`] and
/// [`ReplayProtection::submit_key`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError<E> {
    /// The submission or operation was rejected as a replay.
    Replay(ReplayError),
    /// The wrapped execution failed; the digest/key was not consumed.
    Execute(E),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(op: &[u8], value: &[u8]) -> IdempotencyKey {
        IdempotencyKey::new(op.to_vec(), value.to_vec())
    }

    #[test]
    fn same_key_retry_is_rejected() {
        let mut rp = ReplayProtection::new();
        let k = key(b"set-admin", b"req-1");

        assert!(rp.submit_key(&k, || Ok::<(), ()>(())).is_ok());
        assert_eq!(rp.check_key(&k), Err(ReplayError::AlreadyConsumed));
        assert_eq!(
            rp.submit_key(&k, || Ok::<(), ()>(())),
            Err(SubmitError::Replay(ReplayError::AlreadyConsumed))
        );
    }

    #[test]
    fn distinct_keys_are_independent() {
        let mut rp = ReplayProtection::new();
        let a = key(b"set-admin", b"req-1");
        let b = key(b"set-admin", b"req-2");

        assert!(rp.submit_key(&a, || Ok::<(), ()>(())).is_ok());
        assert!(rp.submit_key(&b, || Ok::<(), ()>(())).is_ok());
        assert_eq!(rp.len(), 2);
    }

    #[test]
    fn same_key_different_operation_does_not_collide() {
        let mut rp = ReplayProtection::new();
        let a = key(b"set-admin", b"req-1");
        let b = key(b"set-fee", b"req-1");

        assert!(rp.submit_key(&a, || Ok::<(), ()>(())).is_ok());
        assert!(rp.submit_key(&b, || Ok::<(), ()>(())).is_ok());
    }

    #[test]
    fn failed_execution_does_not_consume_key() {
        let mut rp = ReplayProtection::new();
        let k = key(b"set-admin", b"req-1");

        assert_eq!(
            rp.submit_key(&k, || Err::<(), &str>("boom")),
            Err(SubmitError::Execute("boom"))
        );
        assert!(!rp.is_consumed(&k.digest()));
        // Retrying with the same key after a failure succeeds.
        assert!(rp.submit_key(&k, || Ok::<(), ()>(())).is_ok());
    }
}
