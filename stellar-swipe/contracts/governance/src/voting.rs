use soroban_sdk::{Address, Env};

use crate::proposals::{self, Proposal, VoteType};
use crate::GovernanceError;

/// #1289: the single deadline rule shared by vote submission *and* vote
/// replacement. Votes are accepted only in `[voting_starts, voting_ends)`;
/// the exact ledger `voting_ends` is already too late.
pub fn require_voting_open(env: &Env, proposal: &Proposal) -> Result<(), GovernanceError> {
    let now = env.ledger().timestamp();
    if now < proposal.voting_starts {
        return Err(GovernanceError::VotingNotStarted);
    }
    if now >= proposal.voting_ends {
        return Err(GovernanceError::VotingEnded);
    }
    Ok(())
}

pub fn cast_vote(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    proposals::cast_vote(env, proposal_id, voter, vote_type)
}

pub fn delegate_voting_power(
    env: &Env,
    delegator: Address,
    delegate: Address,
) -> Result<(), GovernanceError> {
    proposals::delegate_voting_power(env, delegator, delegate)
}

pub fn undelegate_voting_power(env: &Env, delegator: Address) -> Result<(), GovernanceError> {
    proposals::undelegate_voting_power(env, delegator)
}

pub fn get_effective_voting_power(env: &Env, user: Address) -> i128 {
    proposals::get_effective_voting_power(env, &user)
}
