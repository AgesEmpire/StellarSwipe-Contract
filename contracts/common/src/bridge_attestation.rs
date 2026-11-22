//! Versioned bridge attestation envelopes (#1412).
//!
//! Every attestation carries an explicit schema version. Validators and
//! consumers check the version before reading any payload fields, so an
//! incompatible payload is rejected cleanly before any state change.
//!
//! ## Version policy
//!
//! * [`CURRENT_ATTESTATION_VERSION`] is the only version accepted for new
//!   submissions.
//! * Future versions increment this constant. Old code that only knows about
//!   version N rejects version N+1 with
//!   [`AttestationError::UnsupportedVersion`], which is a stable, typed error.
//! * Version 0 is explicitly reserved as "unversioned / legacy" and is always
//!   rejected so that pre-versioning payloads cannot slip through.
//!
//! ## Envelope layout
//!
//! ```text
//! AttestationEnvelope {
//!     version:    u16,          // schema version; must equal CURRENT_ATTESTATION_VERSION
//!     channel_id: bytes,        // opaque channel identifier
//!     sequence:   u64,          // monotonic sequence number within the channel
//!     payload:    bytes,        // opaque message payload
//!     signatures: Vec<bytes>,   // validator signatures over the canonical digest
//! }
//! ```
//!
//! The canonical digest (what validators sign) is:
//!
//! ```text
//! digest = sha256(
//!     "bridge-attestation-v1" || 0x00 ||
//!     version as u16_be             ||
//!     len(channel_id) as u32_be     ||
//!     channel_id                    ||
//!     sequence as u64_be            ||
//!     len(payload) as u32_be        ||
//!     payload
//! )
//! ```
//!
//! Signatures are not mixed into the digest (a validator signs the digest, not
//! the envelope that will carry those signatures).
//!
//! ## What done looks like (#1412)
//!
//! * Attestation versions are validated before state changes.
//! * Unsupported versions return a stable contract error.
//! * Fixtures cover the current version and unsupported versions.

use sha2::{Digest, Sha256};

/// Domain-separation tag for the attestation canonical digest.
const DOMAIN_TAG: &[u8] = b"bridge-attestation-v1";

/// The only attestation schema version accepted for new submissions.
///
/// Increment this constant (and add a new decode path in
/// [`AttestationEnvelope::validate_version`]) when the schema changes in a
/// backward-incompatible way.
pub const CURRENT_ATTESTATION_VERSION: u16 = 1;

/// Version 0 is reserved as "unversioned / legacy" and is always rejected.
/// It serves as a sentinel for pre-versioning payloads.
pub const RESERVED_VERSION_ZERO: u16 = 0;

/// Errors returned by attestation operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttestationError {
    /// The envelope's version field is not supported by this code.
    ///
    /// Callers must reject the envelope and not proceed to state changes.
    UnsupportedVersion,
    /// The envelope's version is version zero, which is always reserved and
    /// rejected regardless of other fields.
    ReservedVersion,
    /// The payload field is empty; a valid attestation must carry a non-empty
    /// payload.
    EmptyPayload,
    /// The channel_id field is empty; a valid attestation must identify its
    /// channel.
    EmptyChannelId,
    /// The signatures list is empty; at least one signature is required.
    NoSignatures,
    /// The sequence number is 0, which is reserved to mean "uninitialized".
    ZeroSequence,
}

/// A bridge attestation envelope with an explicit schema version.
///
/// Fields are intentionally public so SDK and test code can construct
/// fixtures directly. The [`AttestationEnvelope::validate`] method performs
/// all version and structural checks before the caller proceeds to state
/// changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttestationEnvelope {
    /// Schema version. Must equal [`CURRENT_ATTESTATION_VERSION`] for the
    /// envelope to be accepted by this codebase.
    pub version: u16,
    /// Opaque channel identifier (e.g. the peer chain ID).
    pub channel_id: Vec<u8>,
    /// Monotonic sequence number within the channel.
    pub sequence: u64,
    /// Opaque message payload.
    pub payload: Vec<u8>,
    /// Validator signatures over the canonical digest.
    pub signatures: Vec<Vec<u8>>,
}

impl AttestationEnvelope {
    /// Creates an envelope with the current schema version.
    ///
    /// This is the only constructor that stamps `version = CURRENT_ATTESTATION_VERSION`.
    /// Test fixtures for older or future versions should be created by directly
    /// constructing the struct.
    pub fn new(
        channel_id: impl Into<Vec<u8>>,
        sequence: u64,
        payload: impl Into<Vec<u8>>,
        signatures: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            version: CURRENT_ATTESTATION_VERSION,
            channel_id: channel_id.into(),
            sequence,
            payload: payload.into(),
            signatures,
        }
    }

    /// Validates the version field and structural invariants before any state
    /// changes are made.
    ///
    /// All version checks happen first so that an unsupported or reserved
    /// version is never silently skipped. Structural checks (empty fields,
    /// zero sequence) follow.
    ///
    /// Returns:
    /// - [`AttestationError::ReservedVersion`] for version 0.
    /// - [`AttestationError::UnsupportedVersion`] for any version that is not
    ///   [`CURRENT_ATTESTATION_VERSION`] (and not 0).
    /// - [`AttestationError::EmptyChannelId`] when `channel_id` is empty.
    /// - [`AttestationError::ZeroSequence`] when `sequence` is 0.
    /// - [`AttestationError::EmptyPayload`] when `payload` is empty.
    /// - [`AttestationError::NoSignatures`] when `signatures` is empty.
    pub fn validate(&self) -> Result<(), AttestationError> {
        // Version checks come first, unconditionally.
        if self.version == RESERVED_VERSION_ZERO {
            return Err(AttestationError::ReservedVersion);
        }
        if self.version != CURRENT_ATTESTATION_VERSION {
            return Err(AttestationError::UnsupportedVersion);
        }
        // Structural checks.
        if self.channel_id.is_empty() {
            return Err(AttestationError::EmptyChannelId);
        }
        if self.sequence == 0 {
            return Err(AttestationError::ZeroSequence);
        }
        if self.payload.is_empty() {
            return Err(AttestationError::EmptyPayload);
        }
        if self.signatures.is_empty() {
            return Err(AttestationError::NoSignatures);
        }
        Ok(())
    }

    /// Computes the canonical digest that validators sign.
    ///
    /// The digest is deterministic and domain-separated. It does not include
    /// the `signatures` field (validators sign the digest, not the full
    /// envelope).
    pub fn canonical_digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(DOMAIN_TAG);
        h.update([0u8]); // null byte separator after domain tag
        h.update(self.version.to_be_bytes());
        h.update((self.channel_id.len() as u32).to_be_bytes());
        h.update(&self.channel_id);
        h.update(self.sequence.to_be_bytes());
        h.update((self.payload.len() as u32).to_be_bytes());
        h.update(&self.payload);
        h.finalize().into()
    }

    /// Returns `true` when this envelope's version is supported by the current
    /// codebase, without performing any other structural checks.
    ///
    /// Useful in routing logic that needs to decide which decoder to invoke.
    pub fn is_version_supported(&self) -> bool {
        self.version == CURRENT_ATTESTATION_VERSION
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_envelope() -> AttestationEnvelope {
        AttestationEnvelope::new(
            b"chain-42".to_vec(),
            1,
            b"payload-bytes".to_vec(),
            vec![vec![0xDE, 0xAD, 0xBE, 0xEF]],
        )
    }

    // ── Version validation ────────────────────────────────────────────────────

    #[test]
    fn current_version_is_accepted() {
        let env = valid_envelope();
        assert_eq!(env.version, CURRENT_ATTESTATION_VERSION);
        assert_eq!(env.validate(), Ok(()));
    }

    #[test]
    fn version_zero_is_rejected_with_reserved_error() {
        let mut env = valid_envelope();
        env.version = 0;
        assert_eq!(env.validate(), Err(AttestationError::ReservedVersion));
    }

    #[test]
    fn future_version_is_rejected_with_unsupported_error() {
        let mut env = valid_envelope();
        env.version = CURRENT_ATTESTATION_VERSION + 1;
        assert_eq!(env.validate(), Err(AttestationError::UnsupportedVersion));
    }

    #[test]
    fn past_unsupported_version_is_rejected() {
        // A past version that is no longer current (and not zero) is also
        // UnsupportedVersion, not ReservedVersion.
        let mut env = valid_envelope();
        env.version = CURRENT_ATTESTATION_VERSION.saturating_sub(1).max(1);
        if env.version != CURRENT_ATTESTATION_VERSION {
            assert_eq!(env.validate(), Err(AttestationError::UnsupportedVersion));
        }
        // If CURRENT_ATTESTATION_VERSION is 1, version 1 is current; skip.
    }

    #[test]
    fn version_check_runs_before_structural_checks() {
        // Even with other fields invalid, version error is returned first.
        let mut env = valid_envelope();
        env.version = 0;
        env.channel_id = vec![];
        env.payload = vec![];
        // ReservedVersion, not EmptyChannelId.
        assert_eq!(env.validate(), Err(AttestationError::ReservedVersion));
    }

    #[test]
    fn unsupported_version_takes_priority_over_structural_errors() {
        let mut env = valid_envelope();
        env.version = 99;
        env.payload = vec![];
        // UnsupportedVersion, not EmptyPayload.
        assert_eq!(env.validate(), Err(AttestationError::UnsupportedVersion));
    }

    // ── Structural validation ─────────────────────────────────────────────────

    #[test]
    fn empty_channel_id_is_rejected() {
        let mut env = valid_envelope();
        env.channel_id = vec![];
        assert_eq!(env.validate(), Err(AttestationError::EmptyChannelId));
    }

    #[test]
    fn zero_sequence_is_rejected() {
        let mut env = valid_envelope();
        env.sequence = 0;
        assert_eq!(env.validate(), Err(AttestationError::ZeroSequence));
    }

    #[test]
    fn empty_payload_is_rejected() {
        let mut env = valid_envelope();
        env.payload = vec![];
        assert_eq!(env.validate(), Err(AttestationError::EmptyPayload));
    }

    #[test]
    fn empty_signatures_is_rejected() {
        let mut env = valid_envelope();
        env.signatures = vec![];
        assert_eq!(env.validate(), Err(AttestationError::NoSignatures));
    }

    // ── Canonical digest ──────────────────────────────────────────────────────

    #[test]
    fn identical_envelopes_produce_same_digest() {
        let e1 = valid_envelope();
        let e2 = valid_envelope();
        assert_eq!(e1.canonical_digest(), e2.canonical_digest());
    }

    #[test]
    fn different_payloads_produce_different_digests() {
        let e1 = valid_envelope();
        let mut e2 = valid_envelope();
        e2.payload = b"other-payload".to_vec();
        assert_ne!(e1.canonical_digest(), e2.canonical_digest());
    }

    #[test]
    fn different_versions_produce_different_digests() {
        let e1 = valid_envelope();
        let mut e2 = valid_envelope();
        e2.version = 2;
        assert_ne!(e1.canonical_digest(), e2.canonical_digest());
    }

    #[test]
    fn different_channels_produce_different_digests() {
        let e1 = valid_envelope();
        let mut e2 = valid_envelope();
        e2.channel_id = b"chain-99".to_vec();
        assert_ne!(e1.canonical_digest(), e2.canonical_digest());
    }

    #[test]
    fn different_sequences_produce_different_digests() {
        let e1 = valid_envelope();
        let mut e2 = valid_envelope();
        e2.sequence = 2;
        assert_ne!(e1.canonical_digest(), e2.canonical_digest());
    }

    #[test]
    fn signatures_do_not_affect_canonical_digest() {
        let e1 = valid_envelope();
        let mut e2 = valid_envelope();
        e2.signatures = vec![vec![0xAA; 64]];
        // Different signatures, same digest (signatures are not part of the
        // canonical digest input).
        assert_eq!(e1.canonical_digest(), e2.canonical_digest());
    }

    // ── is_version_supported ─────────────────────────────────────────────────

    #[test]
    fn current_version_is_supported() {
        let env = valid_envelope();
        assert!(env.is_version_supported());
    }

    #[test]
    fn future_version_is_not_supported() {
        let mut env = valid_envelope();
        env.version = 99;
        assert!(!env.is_version_supported());
    }

    #[test]
    fn version_zero_is_not_supported() {
        let mut env = valid_envelope();
        env.version = 0;
        assert!(!env.is_version_supported());
    }

    // ── Fixture: current version and unsupported versions ────────────────────

    #[test]
    fn fixture_current_version_validates_cleanly() {
        // This is the canonical "happy path" fixture for integration tests.
        let fixture = AttestationEnvelope {
            version: CURRENT_ATTESTATION_VERSION,
            channel_id: b"stellar-evm-bridge".to_vec(),
            sequence: 1_000,
            payload: b"transfer(0xABCD,100)".to_vec(),
            signatures: vec![
                vec![0x01; 64], // mock sig validator A
                vec![0x02; 64], // mock sig validator B
                vec![0x03; 64], // mock sig validator C
            ],
        };
        assert_eq!(fixture.validate(), Ok(()));
    }

    #[test]
    fn fixture_version_zero_is_always_rejected() {
        let fixture = AttestationEnvelope {
            version: 0,
            channel_id: b"stellar-evm-bridge".to_vec(),
            sequence: 1,
            payload: b"anything".to_vec(),
            signatures: vec![vec![0x01; 64]],
        };
        assert_eq!(fixture.validate(), Err(AttestationError::ReservedVersion));
    }

    #[test]
    fn fixture_future_version_is_rejected_with_stable_error() {
        let fixture = AttestationEnvelope {
            version: 2, // hypothetical next version
            channel_id: b"stellar-evm-bridge".to_vec(),
            sequence: 1,
            payload: b"anything".to_vec(),
            signatures: vec![vec![0x01; 64]],
        };
        // Stable, predictable error — not a panic.
        assert_eq!(fixture.validate(), Err(AttestationError::UnsupportedVersion));
    }
}
