//! Slashing decisions with evidence references (Issue #1433).
//!
//! `slash_stake_with_evidence` attaches a bounded evidence reference (an
//! off-chain digest, CID or URI, as raw bytes) and a decision reason to a
//! slash, for later audit.
//!
//! * **Bounded**: the evidence reference must be
//!   `1..=MAX_EVIDENCE_LEN` bytes and the decision reason
//!   `1..=MAX_DECISION_REASON_LEN` bytes.  Both are checked before any state
//!   is touched.
//! * **Atomic**: validation runs first, then the slash accounting
//!   (`do_slash`, which writes the `SlashRecord`), then this audit record —
//!   all in one invocation.  Any error in any step returns `Err`, and the host
//!   reverts every write made by the call, so there is never a slash without
//!   its audit record or an audit record without its slash.
//! * **Immutable**: the record is written exactly once, keyed by `slash_id`,
//!   at the moment the slash is finalized.  There is no update path; an
//!   attempt to write a second record for the same `slash_id` is rejected.
//! * **No duplicate decisions**: the sha256 of the evidence reference is
//!   bound to the first slash that used it.  Re-submitting the same evidence
//!   (for the same or any other provider) is rejected with
//!   [`StakeVaultError::DuplicateSlashDecision`] before any slash runs.

use soroban_sdk::{contracttype, Address, Bytes, BytesN, Env, String};

use crate::StakeVaultError;

/// Maximum byte length of an evidence reference.
pub const MAX_EVIDENCE_LEN: u32 = 256;
/// Maximum byte length of a decision reason.
pub const MAX_DECISION_REASON_LEN: u32 = 512;

#[contracttype]
#[derive(Clone)]
pub enum SlashEvidenceKey {
    /// slash_id → SlashEvidence.
    Evidence(u64),
    /// sha256(evidence) → slash_id that first used it.
    EvidenceUsed(BytesN<32>),
}

/// Immutable audit record attached to a slash.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashEvidence {
    pub slash_id: u64,
    pub provider: Address,
    /// Raw evidence reference (digest, CID, URI…), at most `MAX_EVIDENCE_LEN` bytes.
    pub evidence: Bytes,
    /// sha256 of `evidence`; the de-duplication key.
    pub evidence_hash: BytesN<32>,
    /// Human-readable decision reason.
    pub decision_reason: String,
    /// Slashed amount, copied from the slash accounting for audit.
    pub slash_amount: i128,
    pub recorded_at: u64,
}

/// Validate bounds and uniqueness. Pure reads — returns the evidence hash.
pub fn validate(
    env: &Env,
    evidence: &Bytes,
    decision_reason: &String,
) -> Result<BytesN<32>, StakeVaultError> {
    if evidence.is_empty() || evidence.len() > MAX_EVIDENCE_LEN {
        return Err(StakeVaultError::InvalidEvidence);
    }
    if decision_reason.is_empty() || decision_reason.len() > MAX_DECISION_REASON_LEN {
        return Err(StakeVaultError::InvalidEvidence);
    }
    let hash: BytesN<32> = env.crypto().sha256(evidence).into();
    if env
        .storage()
        .persistent()
        .has(&SlashEvidenceKey::EvidenceUsed(hash.clone()))
    {
        return Err(StakeVaultError::DuplicateSlashDecision);
    }
    Ok(hash)
}

/// Write the audit record for a slash that has just been applied.
pub fn record(env: &Env, record: &SlashEvidence) -> Result<(), StakeVaultError> {
    let key = SlashEvidenceKey::Evidence(record.slash_id);
    if env.storage().persistent().has(&key) {
        return Err(StakeVaultError::DuplicateSlashDecision);
    }
    env.storage().persistent().set(&key, record);
    env.storage().persistent().set(
        &SlashEvidenceKey::EvidenceUsed(record.evidence_hash.clone()),
        &record.slash_id,
    );
    Ok(())
}

pub fn get(env: &Env, slash_id: u64) -> Option<SlashEvidence> {
    env.storage()
        .persistent()
        .get(&SlashEvidenceKey::Evidence(slash_id))
}
