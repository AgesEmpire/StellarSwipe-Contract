use soroban_sdk::{Address, Env};

use crate::proposals::{self, VoteType};
use crate::GovernanceError;

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
