//! Issue #1431: revision rules for proposals during active voting.
//!
//! # Field classes
//!
//! | Field                  | Class     | Why |
//! |------------------------|-----------|-----|
//! | `title`                | mutable   | Presentation only — never read by execution. |
//! | `description`          | mutable   | Presentation only — clarifications / typo fixes. |
//! | `proposal_type`        | immutable | It *is* the executed action; validated once at creation. |
//! | `execution_payload`    | immutable | Executed bytes; changing them would change what voters approved. |
//! | `category`             | immutable | Selects quorum / supermajority thresholds. |
//! | `use_quadratic_voting` | immutable | Changes how every already-cast vote is weighted. |
//! | voting window          | immutable | `voting_starts`, `voting_ends`, veto and execution deadlines are fixed at creation. |
//! | `proposer`             | immutable | Ownership and deposit refund target. |
//!
//! Attempting to amend an immutable field always fails with
//! [`GovernanceError::ImmutableProposalField`] and writes nothing.  To change
//! one, withdraw the proposal while it is still `Pending` and resubmit it.
//!
//! # Version and vote-reset policy
//!
//! * Every proposal starts at revision version [`INITIAL_REVISION_VERSION`]
//!   (no storage entry is needed for an unrevised proposal).  Each accepted
//!   amendment bumps the version by exactly one, up to
//!   [`MAX_PROPOSAL_REVISIONS`] amendments per proposal.
//! * Only the original proposer may amend, only while the proposal is
//!   `Pending` or `Active`, and only before `voting_ends`.
//! * An amendment that does not change the stored value is rejected (it would
//!   bump the version without any content change).
//! * **Vote reset:** if any vote has been cast, the amendment clears every
//!   cast vote and zeroes all tallies, because those votes endorsed the
//!   previous text.  Voters must re-vote on the new revision.
//! * **Snapshot is preserved:** the voting-power snapshot taken at creation
//!   (`StorageKey::VoteSnapshots`) is *not* retaken, so every eligible voter
//!   re-votes with exactly the same power and stake movements after creation
//!   still cannot influence the result.
//! * When a reset would occur, at least [`MIN_REVOTE_WINDOW_SECS`] of voting
//!   time must remain so voters can realistically re-vote; otherwise the
//!   amendment is rejected with [`GovernanceError::RevisionWindowClosed`].

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Map, String, Vec};
use stellar_swipe_common::sanitize_string;

use crate::proposals::{
    get_proposal, put_proposal, ProposalCategory, ProposalStatus, ProposalType,
};
use crate::{GovernanceError, StorageKey};

/// Revision version of a proposal that has never been amended.
pub const INITIAL_REVISION_VERSION: u32 = 1;
/// Maximum number of accepted amendments per proposal.
pub const MAX_PROPOSAL_REVISIONS: u32 = 3;
/// Minimum voting time that must remain when an amendment resets votes.
pub const MIN_REVOTE_WINDOW_SECS: u64 = 24 * 60 * 60;

/// A requested change to one proposal field.  Only `Title` and `Description`
/// are mutable; every other variant exists so an attempted change to an
/// immutable field is expressible and rejected explicitly.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalAmendment {
    Title(String),
    Description(String),
    ProposalType(ProposalType),
    ExecutionPayload(Bytes),
    Category(ProposalCategory),
    QuadraticVoting(bool),
    VotingWindow(u64, u64),
}

/// Revision bookkeeping for an amended proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalRevision {
    pub proposal_id: u64,
    /// Current revision version (starts at [`INITIAL_REVISION_VERSION`]).
    pub version: u32,
    /// Ledger timestamp of the latest accepted amendment.
    pub revised_at: u64,
    /// Number of amendments that reset cast votes.
    pub vote_resets: u32,
}

/// Whether `amendment` targets a field that may be changed after creation.
pub fn is_mutable(amendment: &ProposalAmendment) -> bool {
    matches!(
        amendment,
        ProposalAmendment::Title(_) | ProposalAmendment::Description(_)
    )
}

/// Current revision version of `proposal_id`.
pub fn get_revision_version(env: &Env, proposal_id: u64) -> u32 {
    get_revision(env, proposal_id)
        .map(|r| r.version)
        .unwrap_or(INITIAL_REVISION_VERSION)
}

pub fn get_revision(env: &Env, proposal_id: u64) -> Option<ProposalRevision> {
    env.storage()
        .persistent()
        .get(&StorageKey::ProposalRevision(proposal_id))
}

/// Apply `amendment` to `proposal_id` under the rules in the module docs.
/// Returns the new revision version.
pub fn revise_proposal(
    env: &Env,
    proposal_id: u64,
    proposer: Address,
    amendment: ProposalAmendment,
) -> Result<u32, GovernanceError> {
    proposer.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;

    if proposer != proposal.proposer {
        return Err(GovernanceError::Unauthorized);
    }
    if proposal.status != ProposalStatus::Pending && proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }
    let now = env.ledger().timestamp();
    if now >= proposal.voting_ends {
        return Err(GovernanceError::VotingEnded);
    }
    if !is_mutable(&amendment) {
        return Err(GovernanceError::ImmutableProposalField);
    }

    let previous = get_revision(env, proposal_id);
    let applied = previous.as_ref().map(|r| r.version).unwrap_or(INITIAL_REVISION_VERSION)
        - INITIAL_REVISION_VERSION;
    if applied >= MAX_PROPOSAL_REVISIONS {
        return Err(GovernanceError::RevisionLimitReached);
    }

    match &amendment {
        ProposalAmendment::Title(title) => {
            if title.is_empty() || *title == proposal.title {
                return Err(GovernanceError::InvalidProposal);
            }
            sanitize_string(env, title, 256).map_err(|_| GovernanceError::InvalidProposal)?;
            proposal.title = title.clone();
        }
        ProposalAmendment::Description(description) => {
            if description.is_empty() || *description == proposal.description {
                return Err(GovernanceError::InvalidProposal);
            }
            sanitize_string(env, description, 4096)
                .map_err(|_| GovernanceError::InvalidProposal)?;
            proposal.description = description.clone();
        }
        // Unreachable: rejected by `is_mutable` above.
        _ => return Err(GovernanceError::ImmutableProposalField),
    }

    let votes_cast = proposal.voter_list.len();
    let reset = votes_cast > 0;
    if reset {
        if proposal.voting_ends.saturating_sub(now) < MIN_REVOTE_WINDOW_SECS {
            return Err(GovernanceError::RevisionWindowClosed);
        }
        proposal.voters = Map::new(env);
        proposal.voter_list = Vec::new(env);
        proposal.votes_for = 0;
        proposal.votes_against = 0;
        proposal.votes_abstain = 0;
    }

    let version = applied + INITIAL_REVISION_VERSION + 1;
    let vote_resets = previous.map(|r| r.vote_resets).unwrap_or(0) + reset as u32;
    put_proposal(env, &proposal)?;
    env.storage().persistent().set(
        &StorageKey::ProposalRevision(proposal_id),
        &ProposalRevision {
            proposal_id,
            version,
            revised_at: now,
            vote_resets,
        },
    );

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("proprev")),
        (proposal_id, version, reset, votes_cast, now),
    );

    Ok(version)
}
