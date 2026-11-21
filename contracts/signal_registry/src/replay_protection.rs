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
//! ## Nonce domains
//!
//! Auto-trade order intents are partitioned into independent nonce domains so
//! that replaying one intent cannot consume another intent's nonce:
//!
//! * [`NonceDomain::UserOrder`] — user-submitted order intents.
//! * [`NonceDomain::KeeperRetry`] — keeper retries of a previously submitted
//!   intent.
//! * [`NonceDomain::Administrative`] — administrative actions.
//!
//! The domain is encoded into the authorization payload (see
//! [`NonceDomain::tag`]) and mixed into the derived digest, so the same inputs
//! submitted under different domains produce distinct digests. Each domain
//! maintains its own nonce sequence and only increments on successful use;
//! replay within a domain and cross-domain collisions are rejected.
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

/// Independent nonce domain for auto-trade order intents.
///
/// Each domain maintains its own nonce sequence so that replaying an intent in
/// one domain cannot consume another domain's nonce. The domain is encoded in
/// the authorization payload via [`NonceDomain::tag`] and mixed into the
/// derived digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NonceDomain {
    /// User-submitted order intents.
    UserOrder,
    /// Keeper retries of a previously submitted intent.
    KeeperRetry,
    /// Administrative actions.
    Administrative,
}

impl NonceDomain {
    /// Returns the stable byte tag encoded into authorization payloads.
    pub fn tag(&self) -> u8 {
        match self {
            NonceDomain::UserOrder => 0x01,
            NonceDomain::KeeperRetry => 0x02,
            NonceDomain::Administrative => 0x03,
        }
    }

    /// Returns the domain for a stable byte tag, if any.
    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0x01 => Some(NonceDomain::UserOrder),
            0x02 => Some(NonceDomain::KeeperRetry),
            0x03 => Some(NonceDomain::Administrative),
            _ => None,
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
    /// Nonce domain the submission belongs to.
    pub domain: NonceDomain,
}

impl<'a> SubmissionContext<'a> {
    /// Computes the deterministic digest binding provider, payload, caller,
    /// intended registry and nonce domain.
    ///
    /// The domain tag is mixed in so the same inputs submitted under different
    /// domains produce distinct digests and cannot collide across domains.
    pub fn digest(&self) -> SubmissionDigest {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_TAG);
        hasher.update([0u8]);
        hasher.update([self.domain.tag()]);
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
}
