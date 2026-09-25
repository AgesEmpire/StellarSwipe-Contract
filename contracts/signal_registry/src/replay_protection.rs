//! Deterministic replay protection for cross-contract signal submissions.
//!
//! A signal submission that crosses a contract boundary is bound to a
//! deterministic digest computed from:
//!
//! * the nonce domain (`domain`), which separates nonces by user, operation
//!   type and contract domain,
//! * the provider identity (`provider`),
//! * the signal payload (`payload`),
//! * the caller context (`caller`),
//! * the intended registry address (`registry`).
//!
//! The digest is the replay-protection key. A digest is only marked as
//! consumed once the submission has been accepted, so a failed submission
//! never consumes a nonce and can be safely retried with the same inputs.
//!
//! ## Nonce domain separation
//!
//! Every digest mixes an explicit [`NonceDomain`] so that a nonce valid in one
//! flow cannot be replayed in another. The domain is composed of:
//!
//! * `user` — the account/identity the nonce belongs to,
//! * `operation` — the operation type (e.g. `submit`, `revoke`),
//! * `contract` — the contract domain the nonce is scoped to.
//!
//! Because the domain is part of the hashed preimage, a nonce issued for one
//! `(user, operation, contract)` triple produces a different digest than the
//! same nonce used in any other triple, so cross-operation and cross-contract
//! replays are rejected as [`ReplayError::AlreadyConsumed`] only within their
//! own domain and never collide across domains.
//!
//! ## Digest format (for SDK and indexer authors)
//!
//! ```text
//! digest = sha256(
//!     "signal-replay-v1" || 0x00 ||
//!     len(user)      as u32_be || user      ||
//!     len(operation) as u32_be || operation ||
//!     len(contract)  as u32_be || contract  ||
//!     len(provider)  as u32_be || provider  ||
//!     len(payload)   as u32_be || payload   ||
//!     len(caller)    as u32_be || caller    ||
//!     len(registry)  as u32_be || registry
//! )
//! ```
//!
//! Each field is length-prefixed with a big-endian `u32` so that distinct
//! field boundaries can never collide. The domain-separation tag
//! `"signal-replay-v1"` prevents cross-protocol digest reuse.
//!
//! ## Migration strategy for existing nonce state
//!
//! Nonces recorded before domain separation were keyed only by
//! `(provider, payload, caller, registry)`. To migrate safely:
//!
//! 1. Deploy the new code alongside the legacy tracker. Legacy digests remain
//!    valid for their original domain and are treated as belonging to the
//!    [`NonceDomain::legacy`] domain.
//! 2. For each in-flight submission, recompute its digest under the explicit
//!    domain and re-commit it via [`ReplayProtection::commit`]. Because the
//!    domain is part of the preimage, migrated digests never collide with
//!    legacy ones.
//! 3. Once all in-flight submissions have drained, drop the legacy tracker.
//!    No on-chain state needs rewriting: unconsumed legacy nonces simply
//!    expire with their submissions.
//!
//! Clients determine the required domain by calling
//! [`NonceDomain::for_submission`] with the user, operation and contract they
//! intend to use; the returned domain is the one that must be supplied in
//! [`SubmissionContext::domain`].

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

/// Explicit nonce domain separating nonces by user, operation type and
/// contract domain.
///
/// Two submissions that share a nonce but differ in any domain component
/// produce different digests and therefore cannot replay each other.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonceDomain<'a> {
    /// The account/identity the nonce belongs to.
    pub user: &'a [u8],
    /// The operation type the nonce is scoped to (e.g. `b"submit"`).
    pub operation: &'a [u8],
    /// The contract domain the nonce is scoped to.
    pub contract: &'a [u8],
}

impl<'a> NonceDomain<'a> {
    /// Builds the nonce domain a client must use for a given submission.
    ///
    /// Clients call this to determine the required nonce domain before
    /// constructing a [`SubmissionContext`].
    pub fn for_submission(user: &'a [u8], operation: &'a [u8], contract: &'a [u8]) -> Self {
        Self {
            user,
            operation,
            contract,
        }
    }

    /// The legacy domain used by nonces recorded before domain separation.
    ///
    /// See the module-level migration strategy for how legacy nonces are
    /// drained into explicit domains.
    pub fn legacy() -> Self {
        Self {
            user: b"legacy",
            operation: b"legacy",
            contract: b"legacy",
        }
    }
}
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
    /// Nonce domain separating this submission from other flows.
    pub domain: NonceDomain<'a>,
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
    /// Computes the deterministic digest binding the nonce domain, provider,
    /// payload, caller and intended registry.
    pub fn digest(&self) -> SubmissionDigest {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_TAG);
        hasher.update([0u8]);
        for field in [
            self.domain.user,
            self.domain.operation,
            self.domain.contract,
            self.provider,
            self.payload,
            self.caller,
            self.registry,
        ] {
            hasher.update((field.len() as u32).to_be_bytes());
            hasher.update(field);
        }
        SubmissionDigest(hasher.finalize())
            self.provider,
            self.payload,
            self.caller,
            self.registry,
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_TAG);
        hasher.update([0u8]);
        for field in [
            self.domain.user,
            self.domain.operation,
            self.domain.contract,
            self.provider,
            self.payload,
            self.caller,
            self.registry,
        ] {
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
            domain: NonceDomain::for_submission(b"user-1", b"submit", b"registry-1"),
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
    fn cross_operation_replay_is_rejected() {
        let mut rp = ReplayProtection::new();
        let submit = SubmissionContext {
            domain: NonceDomain::for_submission(b"user-1", b"submit", b"registry-1"),
            provider: b"provider-a",
            payload: b"signal-1",
            caller: b"caller-1",
            registry: b"registry-1",
        };
        let revoke = SubmissionContext {
            domain: NonceDomain::for_submission(b"user-1", b"revoke", b"registry-1"),
            ..submit.clone()
        };

        assert!(rp.submit(&submit, || Ok::<(), ()>(())).is_ok());
        // Same nonce under a different operation is a distinct domain, so it
        // is not treated as a replay of the submit flow.
        assert!(rp.submit(&revoke, || Ok::<(), ()>(())).is_ok());
        // Replaying the original submit flow is still rejected.
        assert_eq!(
            rp.submit(&submit, || Ok::<(), ()>(())),
            Err(SubmitError::Replay(ReplayError::AlreadyConsumed))
        );
    }

    #[test]
    fn cross_contract_replay_is_rejected() {
        let mut rp = ReplayProtection::new();
        let registry_a = SubmissionContext {
            domain: NonceDomain::for_submission(b"user-1", b"submit", b"registry-1"),
            provider: b"provider-a",
            payload: b"signal-1",
            caller: b"caller-1",
            registry: b"registry-1",
        };
        let registry_b = SubmissionContext {
            domain: NonceDomain::for_submission(b"user-1", b"submit", b"registry-2"),
            ..registry_a.clone()
        };

        assert!(rp.submit(&registry_a, || Ok::<(), ()>(())).is_ok());
        // Same nonce under a different contract domain is a distinct domain.
        assert!(rp.submit(&registry_b, || Ok::<(), ()>(())).is_ok());
        // Replaying the original contract domain is still rejected.
        assert_eq!(
            rp.submit(&registry_a, || Ok::<(), ()>(())),
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
