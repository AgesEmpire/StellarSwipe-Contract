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

use std::collections::{BTreeSet, VecDeque};

/// Domain-separation tag mixed into every digest.
const DOMAIN_TAG: &[u8] = b"signal-replay-v1";

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
        execute().map_err(SubmitError::Execution)?;
        self.record(digest);
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
    fn same_payload_replay_is_rejected() {
        let mut rp = ReplayProtection::new();
        let c = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-1");

        assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
        assert_eq!(
            rp.submit(&c, || Ok::<(), ()>(())),
            Err(SubmitError::Replay(ReplayError::AlreadyConsumed))
        );
    }

    #[test]
    fn altered_payload_is_accepted() {
        let mut rp = ReplayProtection::new();
        let first = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-1");
        let altered = ctx(b"provider-a", b"signal-2", b"caller-1", b"registry-1");

        assert!(rp.submit(&first, || Ok::<(), ()>(())).is_ok());
        assert!(rp.submit(&altered, || Ok::<(), ()>(())).is_ok());
    }

    #[test]
    fn wrong_registry_is_accepted() {
        let mut rp = ReplayProtection::new();
        let intended = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-1");
        let other = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-2");

        assert!(rp.submit(&intended, || Ok::<(), ()>(())).is_ok());
        assert!(rp.submit(&other, || Ok::<(), ()>(())).is_ok());
    }

    #[test]
    fn cross_provider_replay_is_rejected() {
        let mut rp = ReplayProtection::new();
        let provider_a = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-1");
        let provider_b = ctx(b"provider-b", b"signal-1", b"caller-1", b"registry-1");

        assert!(rp.submit(&provider_a, || Ok::<(), ()>(())).is_ok());
        // A different provider produces a different digest, so it is not a replay.
        assert!(rp.submit(&provider_b, || Ok::<(), ()>(())).is_ok());
        // Replaying provider-a's exact submission is rejected.
        assert_eq!(
            rp.submit(&provider_a, || Ok::<(), ()>(())),
            Err(SubmitError::Replay(ReplayError::AlreadyConsumed))
        );
    }

    #[test]
    fn failed_submission_does_not_consume_digest() {
        let mut rp = ReplayProtection::new();
        let c = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-1");

        assert_eq!(
            rp.submit(&c, || Err::<(), &str>("boom")),
            Err(SubmitError::Execution("boom"))
        );
        assert!(!rp.is_consumed(&c.digest()));

        // Retry with the same submission succeeds and consumes the digest.
        assert!(rp.submit(&c, || Ok::<(), &str>(())).is_ok());
        assert!(rp.is_consumed(&c.digest()));
    }

    #[test]
    fn compaction_bounds_storage() {
        let mut rp = ReplayProtection::with_capacity(4);
        for i in 0..10u32 {
            let payload = i.to_be_bytes();
            let c = ctx(b"provider-a", &payload, b"caller-1", b"registry-1");
            assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
        }
        assert_eq!(rp.len(), 4);
        assert_eq!(rp.capacity(), 4);
    }

    #[test]
    fn newer_nonces_remain_rejected_after_compaction() {
        let mut rp = ReplayProtection::with_capacity(4);
        // Consume more than capacity so compaction runs.
        for i in 0..10u32 {
            let payload = i.to_be_bytes();
            let c = ctx(b"provider-a", &payload, b"caller-1", b"registry-1");
            assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
        }
        // The most recent nonces are still retained and rejected on replay.
        for i in 6..10u32 {
            let payload = i.to_be_bytes();
            let c = ctx(b"provider-a", &payload, b"caller-1", b"registry-1");
            assert_eq!(
                rp.submit(&c, || Ok::<(), ()>(())),
                Err(SubmitError::Replay(ReplayError::AlreadyConsumed))
            );
        }
    }

    #[test]
    fn older_nonces_are_evicted_after_compaction() {
        let mut rp = ReplayProtection::with_capacity(4);
        for i in 0..10u32 {
            let payload = i.to_be_bytes();
            let c = ctx(b"provider-a", &payload, b"caller-1", b"registry-1");
            assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
        }
        // The oldest nonces have aged out of the window and are treated as fresh.
        for i in 0..6u32 {
            let payload = i.to_be_bytes();
            let c = ctx(b"provider-a", &payload, b"caller-1", b"registry-1");
            assert!(!rp.is_consumed(&c.digest()));
        }
    }

    #[test]
    fn zero_capacity_disables_retention() {
        let mut rp = ReplayProtection::with_capacity(0);
        let c = ctx(b"provider-a", b"signal-1", b"caller-1", b"registry-1");
        assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
        assert!(rp.is_empty());
        assert!(!rp.is_consumed(&c.digest()));
    }

    /// Property test: for any sequence of distinct claims, replaying any claim
    /// that is still within the retained window is always rejected, even after
    /// compaction has evicted older claims.
    #[test]
    fn property_duplicates_rejected_within_window() {
        let capacity = 8usize;
        let mut rp = ReplayProtection::with_capacity(capacity);
        let mut seen: Vec<u32> = Vec::new();

        // Deterministic pseudo-random sequence of distinct claim ids.
        let mut state: u32 = 0x9E37_79B9;
        for _ in 0..200 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let id = state;
            let payload = id.to_be_bytes();
            let c = ctx(b"provider-a", &payload, b"caller-1", b"registry-1");

            // A fresh claim is accepted.
            assert!(rp.submit(&c, || Ok::<(), ()>(())).is_ok());
            seen.push(id);

            // Every claim still within the retained window must be rejected.
            let window_start = seen.len().saturating_sub(capacity);
            for &retained in &seen[window_start..] {
                let rp_payload = retained.to_be_bytes();
                let rc = ctx(b"provider-a", &rp_payload, b"caller-1", b"registry-1");
                assert_eq!(
                    rp.submit(&rc, || Ok::<(), ()>(())),
                    Err(SubmitError::Replay(ReplayError::AlreadyConsumed)),
                    "retained claim {retained} must remain rejected after compaction"
                );
            }

            // Storage never exceeds the configured bound.
            assert!(rp.len() <= capacity);
        }
    }
}
