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
//! ## Authorization tree depth
//!
//! Authorization trees are traversed recursively. To protect execution
//! resources and keep validation behavior consistent, every tree-consuming
//! entrypoint enforces [`MAX_AUTH_TREE_DEPTH`] *before* recursion begins.
//! Trees deeper than the limit are rejected with the stable
//! [`ReplayError::AuthTreeTooDeep`] contract error.
//!
//! ## Event sequencing
//!
//! Versioned contract events carry a monotonically increasing sequence
//! number so indexers can detect gaps, duplicates, and out-of-order
//! delivery. Sequence state is scoped per contract and persisted in contract
//! storage (see [`SequenceTracker`]), so it survives upgrades rather than
//! resetting with transient state.

use std::collections::BTreeSet;

/// Domain-separation tag mixed into every digest.
const DOMAIN_TAG: &[u8] = b"signal-replay-v1";

/// Maximum supported depth of an authorization tree.
///
/// Depth is counted as the number of nested authorization nodes, with a
/// single (non-nested) node having depth `1`. Trees deeper than this bound
/// are rejected before any recursive traversal is attempted.
pub const MAX_AUTH_TREE_DEPTH: u32 = 32;

/// Deterministic replay-protection key for a cross-contract signal submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubmissionDigest(pub [u8; 32]);

impl SubmissionDigest {
    /// Returns the raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
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
/// trace and may be retried with identical inputs.
#[derive(Clone, Debug, Default)]
pub struct ReplayProtection {
    consumed: BTreeSet<SubmissionDigest>,
}

/// Error returned when a submission is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The digest was already accepted by a previous successful submission.
    AlreadyConsumed,
    /// The authorization tree exceeded [`MAX_AUTH_TREE_DEPTH`].
    AuthTreeTooDeep,
}

/// Validates the depth of an authorization tree before recursive traversal.
///
/// Returns `Ok(())` when `depth` is within [`MAX_AUTH_TREE_DEPTH`] (a depth of
/// `0` is treated as an empty tree and is accepted), and
/// [`ReplayError::AuthTreeTooDeep`] otherwise. Every tree-consuming
/// entrypoint must call this before descending into the tree.
pub fn check_auth_tree_depth(depth: u32) -> Result<(), ReplayError> {
    if depth > MAX_AUTH_TREE_DEPTH {
        return Err(ReplayError::AuthTreeTooDeep);
    }
    Ok(())
}

/// Monotonic sequence state for versioned contract events.
///
/// The tracker is scoped to a single contract and is intended to be persisted
/// in that contract's storage, so the sequence survives upgrades instead of
/// resetting with transient state. Each call to [`SequenceTracker::next`]
/// returns the next sequence value, starting at `1` for the first event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SequenceTracker {
    last: u64,
}

impl SequenceTracker {
    /// Creates a tracker with no events emitted yet.
    pub fn new() -> Self {
        Self { last: 0 }
    }

    /// Returns the most recently issued sequence value (`0` before any event).
    pub fn last(&self) -> u64 {
        self.last
    }

    /// Issues the next monotonically increasing sequence value.
    ///
    /// Values start at `1` and increase by exactly one per call, so indexers
    /// can detect gaps, duplicates, and out-of-order delivery.
    pub fn next(&mut self) -> u64 {
        self.last = self.last.saturating_add(1);
        self.last
    }
}

impl ReplayProtection {
    /// Creates an empty replay-protection tracker.
    pub fn new() -> Self {
        Self {
            consumed: BTreeSet::new(),
        }
    }

    /// Returns `true` if the digest has already been accepted.
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

    /// Checks a submission for replay and validates the authorization tree
    /// depth before any recursive traversal.
    ///
    /// This is the tree-consuming entrypoint guard: it rejects over-limit
    /// trees with [`ReplayError::AuthTreeTooDeep`] before the digest is
    /// computed or consumed.
    pub fn check_with_auth_depth(
        &self,
        ctx: &SubmissionContext<'_>,
        auth_tree_depth: u32,
    ) -> Result<SubmissionDigest, ReplayError> {
        check_auth_tree_depth(auth_tree_depth)?;
        self.check(ctx)
    }

    /// Marks a submission as accepted, consuming its digest.
    ///
    /// Must only be called after the submission has succeeded. Returns the
    /// digest that was consumed.
    pub fn commit(&mut self, ctx: &SubmissionContext<'_>) -> SubmissionDigest {
        let digest = ctx.digest();
        self.consumed.insert(digest);
        digest
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
        execute().map_err(SubmitError::Execution)?;
        self.consumed.insert(digest);
        Ok(digest)
    }

    /// Convenience wrapper that also enforces the authorization tree depth
    /// bound before executing the submission.
    ///
    /// Over-limit trees are rejected with [`ReplayError::AuthTreeTooDeep`]
    /// and the digest is never consumed.
    pub fn submit_with_auth_depth<F, E>(
        &mut self,
        ctx: &SubmissionContext<'_>,
        auth_tree_depth: u32,
        execute: F,
    ) -> Result<SubmissionDigest, SubmitError<E>>
    where
        F: FnOnce() -> Result<(), E>,
    {
        let digest = self
            .check_with_auth_depth(ctx, auth_tree_depth)
            .map_err(SubmitError::Replay)?;
        execute().map_err(SubmitError::Execution)?;
        self.consumed.insert(digest);
        Ok(digest)
    }
}

/// Error returned by [`ReplayProtection::submit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmitError<E> {
    /// The submission was rejected as a replay.
    Replay(ReplayError),
    /// The submission failed during execution; the digest was not consumed.
    Execution(E),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(
        provider: &'a [u8],
        payload: &'a [u8],
        caller: &'a [u8],
        registry: &'a [u8],
    ) -> SubmissionContext<'a> {
        SubmissionContext {
            provider,
            payload,
            caller,
            registry,
        }
    }

    #[test]
    fn sequence_starts_at_one_and_increases_monotonically() {
        let mut tracker = SequenceTracker::new();
        assert_eq!(tracker.last(), 0);
        assert_eq!(tracker.next(), 1);
        assert_eq!(tracker.next(), 2);
        assert_eq!(tracker.next(), 3);
        assert_eq!(tracker.last(), 3);
    }

    #[test]
    fn sequence_is_continuous_across_multiple_transactions() {
        // Simulate a persisted tracker being reloaded between transactions.
        let mut tracker = SequenceTracker::new();
        let mut observed = Vec::new();
        for _ in 0..5 {
            observed.push(tracker.next());
        }
        // Reload from persisted state (survives upgrade) and continue.
        let mut reloaded = tracker;
        observed.push(reloaded.next());
        assert_eq!(observed, vec![1, 2, 3, 4, 5, 6]);
        for pair in observed.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "sequence must have no gaps");
        }
    }

    #[test]
    fn sequence_state_is_scoped_per_contract() {
        let mut registry_a = SequenceTracker::new();
        let mut registry_b = SequenceTracker::new();
        assert_eq!(registry_a.next(), 1);
        assert_eq!(registry_a.next(), 2);
        // A separate contract's tracker is unaffected by another's events.
        assert_eq!(registry_b.next(), 1);
        assert_eq!(registry_a.next(), 3);
    }

    #[test]
    fn digest_is_deterministic_and_field_bound() {
        let a = ctx(b"p", b"payload", b"c", b"r").digest();
        let b = ctx(b"p", b"payload", b"c", b"r").digest();
        assert_eq!(a, b);
        let c = ctx(b"p", b"payloa", b"dc", b"r").digest();
        assert_ne!(a, c);
    }

    #[test]
    fn replay_is_rejected_and_failed_submission_is_retryable() {
        let mut rp = ReplayProtection::new();
        let c = ctx(b"p", b"payload", b"c", b"r");
        assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
        assert_eq!(rp.check(&c), Err(ReplayError::AlreadyConsumed));

        let mut rp2 = ReplayProtection::new();
        let c2 = ctx(b"p2", b"payload", b"c", b"r");
        assert!(matches!(
            rp2.submit(&c2, || Err::<(), ()>(())),
            Err(SubmitError::Execution(()))
        ));
        assert!(rp2.check(&c2).is_ok());
    }

    #[test]
    fn auth_tree_depth_is_enforced_before_consumption() {
        let mut rp = ReplayProtection::new();
        let c = ctx(b"p", b"payload", b"c", b"r");
        assert_eq!(
            rp.check_with_auth_depth(&c, MAX_AUTH_TREE_DEPTH + 1),
            Err(ReplayError::AuthTreeTooDeep)
        );
        assert!(rp.check_with_auth_depth(&c, MAX_AUTH_TREE_DEPTH).is_ok());
    }
}
