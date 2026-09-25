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

use std::collections::BTreeSet;

/// Domain-separation tag mixed into every digest.
const DOMAIN_TAG: &[u8] = b"signal-replay-v1";

/// Canonical hashing helper for Soroban authorization payloads.
///
/// Every authorization path derives its signing domain and serialized bytes
/// through this single helper so that equivalent payloads always hash
/// identically and altered fields never collide. The serialization is
/// length-prefixed with big-endian `u32` lengths and separated by a `0x00`
/// byte after the domain tag, matching the replay-protection digest format.
pub struct AuthPayloadHasher;

impl AuthPayloadHasher {
    /// Hashes a canonical authorization payload.
    ///
    /// `domain` is the domain-separation tag (e.g. `"signal-replay-v1"`),
    /// `network` is the network passphrase / id, `contract` is the contract
    /// address, `method` is the entrypoint name, and `args` are the serialized
    /// invocation arguments. Each field is length-prefixed so distinct field
    /// boundaries can never collide.
    pub fn hash(
        domain: &[u8],
        network: &[u8],
        contract: &[u8],
        method: &[u8],
        args: &[u8],
    ) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(domain);
        hasher.update([0u8]);
        for field in [network, contract, method, args] {
            hasher.update((field.len() as u32).to_be_bytes());
            hasher.update(field);
        }
        hasher.finalize()
    }
}

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
        SubmissionDigest(AuthPayloadHasher::hash(
            DOMAIN_TAG,
            self.provider,
            self.payload,
            self.caller,
            self.registry,
        ))
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
    fn equivalent_payloads_hash_identically() {
        let a = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        let b = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn altered_domain_changes_hash() {
        let base = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        let altered = AuthPayloadHasher::hash(
            b"signal-replay-v2",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        assert_ne!(base, altered);
    }

    #[test]
    fn altered_network_changes_hash() {
        let base = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        let altered = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"mainnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        assert_ne!(base, altered);
    }

    #[test]
    fn altered_contract_changes_hash() {
        let base = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        let altered = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-2",
            b"submit_signal",
            b"args-1",
        );
        assert_ne!(base, altered);
    }

    #[test]
    fn altered_method_changes_hash() {
        let base = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        let altered = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal_v2",
            b"args-1",
        );
        assert_ne!(base, altered);
    }

    #[test]
    fn altered_args_changes_hash() {
        let base = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-1",
        );
        let altered = AuthPayloadHasher::hash(
            b"signal-replay-v1",
            b"testnet",
            b"registry-1",
            b"submit_signal",
            b"args-2",
        );
        assert_ne!(base, altered);
    }

    #[test]
    fn field_boundaries_do_not_collide() {
        // Without length prefixes, ("ab", "c") and ("a", "bc") would collide.
        let a = AuthPayloadHasher::hash(b"d", b"ab", b"c", b"m", b"x");
        let b = AuthPayloadHasher::hash(b"d", b"a", b"bc", b"m", b"x");
        assert_ne!(a, b);
    }
}
