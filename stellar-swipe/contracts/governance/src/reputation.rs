use core::cmp::min;

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, String, Vec};

use crate::proposals::{self, ProposalStatus, VoteType};
use crate::{checked_mul, GovernanceError, StorageKey};

const PRECISION: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipationHistory {
    pub proposals_created: u32,
    pub proposals_succeeded: u32,
    pub votes_cast: u32,
    pub votes_aligned_with_outcome: u32,
    pub committee_memberships: u32,
    pub committee_decisions_approved: u32,
    pub delegations_received: u32,
    pub total_tokens_delegated: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Badge {
    ActiveVoter(u32),
    ProposalAuthor(u32),
    CommitteeMember(String),
    EarlyAdopter,
    TopDelegator,
    ConsistentParticipant(u32),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernanceReputation {
    pub user: Address,
    pub reputation_score: u32,
    pub participation_history: ParticipationHistory,
    pub badges: Vec<Badge>,
    pub last_activity: u64,
    pub decay_rate: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationState {
    pub reputations: Map<Address, GovernanceReputation>,
    pub users: Vec<Address>,
}

pub fn empty_reputation_state(env: &Env) -> ReputationState {
    ReputationState {
        reputations: Map::new(env),
        users: Vec::new(env),
    }
}

pub fn get_reputation_state(env: &Env) -> ReputationState {
    env.storage()
        .instance()
        .get(&StorageKey::ReputationState)
        .unwrap_or_else(|| empty_reputation_state(env))
}

pub fn put_reputation_state(env: &Env, state: &ReputationState) {
    env.storage()
        .instance()
        .set(&StorageKey::ReputationState, state);
}

pub fn get_governance_reputation(env: &Env, user: Address) -> GovernanceReputation {
    let state = get_reputation_state(env);
    state
        .reputations
        .get(user.clone())
        .unwrap_or_else(|| GovernanceReputation {
            user,
            reputation_score: 0,
            participation_history: ParticipationHistory {
                proposals_created: 0,
                proposals_succeeded: 0,
                votes_cast: 0,
                votes_aligned_with_outcome: 0,
                committee_memberships: 0,
                committee_decisions_approved: 0,
                delegations_received: 0,
                total_tokens_delegated: 0,
            },
            badges: Vec::new(env),
            last_activity: env.ledger().timestamp(),
            decay_rate: 10,
        })
}

pub fn calculate_reputation_score(env: &Env, user: Address) -> Result<u32, GovernanceError> {
    let rep = get_governance_reputation(env, user);
    let mut score = 0u32;

    let proposal_score = min(
        2000,
        rep.participation_history
            .proposals_created
            .saturating_mul(100),
    );
    score = score.saturating_add(proposal_score);

    if rep.participation_history.proposals_created > 0 {
        let success_rate = rep
            .participation_history
            .proposals_succeeded
            .saturating_mul(10_000)
            / rep.participation_history.proposals_created;
        score = score.saturating_add(min(2000, success_rate.saturating_mul(2) / 10));
    }

    let voting_score = min(
        2000,
        rep.participation_history.votes_cast.saturating_mul(20),
    );
    score = score.saturating_add(voting_score);

    if rep.participation_history.votes_cast > 0 {
        let accuracy = rep
            .participation_history
            .votes_aligned_with_outcome
            .saturating_mul(10_000)
            / rep.participation_history.votes_cast;
        score = score.saturating_add(min(2000, accuracy.saturating_mul(2) / 10));
    }

    let committee_score = min(
        1000,
        rep.participation_history
            .committee_memberships
            .saturating_mul(200),
    );
    score = score.saturating_add(committee_score);

    let delegation_score = min(
        1000,
        rep.participation_history
            .delegations_received
            .saturating_mul(50),
    );
    score = score.saturating_add(delegation_score);

    score = score.saturating_add(calculate_badge_bonus(&rep.badges));
    score = apply_reputation_decay(env, score, rep.last_activity, rep.decay_rate);

    Ok(min(10_000, score))
}

pub fn record_proposal_creation(env: &Env, user: Address) -> Result<(), GovernanceError> {
    let mut state = get_reputation_state(env);
    let mut rep = get_governance_reputation(env, user.clone());

    rep.participation_history.proposals_created = rep
        .participation_history
        .proposals_created
        .saturating_add(1);
    rep.last_activity = env.ledger().timestamp();
    rep.reputation_score = calculate_reputation_score(env, user.clone())?;
    check_and_award_badges(env, &mut rep);

    upsert_rep(&mut state, &user, rep);
    put_reputation_state(env, &state);
    Ok(())
}

pub fn record_vote(
    env: &Env,
    user: Address,
    proposal_id: u64,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    let mut state = get_reputation_state(env);
    let mut rep = get_governance_reputation(env, user.clone());

    rep.participation_history.votes_cast = rep.participation_history.votes_cast.saturating_add(1);
    rep.last_activity = env.ledger().timestamp();
    rep.reputation_score = calculate_reputation_score(env, user.clone())?;
    check_and_award_badges(env, &mut rep);

    upsert_rep(&mut state, &user, rep);
    put_reputation_state(env, &state);

    let mut vote_records: Map<(Address, u64), VoteType> = env
        .storage()
        .instance()
        .get(&StorageKey::VoteRecords)
        .unwrap_or(Map::new(env));
    vote_records.set((user, proposal_id), vote_type);
    env.storage()
        .instance()
        .set(&StorageKey::VoteRecords, &vote_records);

    Ok(())
}

pub fn record_proposal_outcome(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let proposal = proposals::get_proposal(env, proposal_id)?;
    let outcome =
        proposal.status == ProposalStatus::Succeeded || proposal.status == ProposalStatus::Executed;

    let mut state = get_reputation_state(env);

    if outcome {
        let mut proposer_rep = get_governance_reputation(env, proposal.proposer.clone());
        proposer_rep.participation_history.proposals_succeeded = proposer_rep
            .participation_history
            .proposals_succeeded
            .saturating_add(1);
        proposer_rep.reputation_score = calculate_reputation_score(env, proposal.proposer.clone())?;
        upsert_rep(&mut state, &proposal.proposer, proposer_rep);
    }

    let mut idx = 0;
    while idx < proposal.voter_list.len() {
        let voter = proposal.voter_list.get(idx).unwrap();
        if let Some(vote) = proposal.voters.get(voter.clone()) {
            let aligned = matches!(
                (vote.vote_type, outcome),
                (VoteType::For, true) | (VoteType::Against, false)
            );
            if aligned {
                let mut rep = get_governance_reputation(env, voter.clone());
                rep.participation_history.votes_aligned_with_outcome = rep
                    .participation_history
                    .votes_aligned_with_outcome
                    .saturating_add(1);
                rep.reputation_score = calculate_reputation_score(env, voter.clone())?;
                upsert_rep(&mut state, &voter, rep);
            }
        }
        idx += 1;
    }

    put_reputation_state(env, &state);
    Ok(())
}

pub fn cast_reputation_weighted_vote(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();

    let mut proposal = proposals::get_proposal(env, proposal_id)?;
    let token_power = proposals::get_effective_voting_power(env, &voter);
    if token_power <= 0 {
        return Err(GovernanceError::NoVotingPower);
    }
    if proposal.voters.contains_key(voter.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    let reputation = get_governance_reputation(env, voter.clone());
    let multiplier = 10_000u32.saturating_add(reputation.reputation_score / 2);
    let weighted = checked_mul(token_power, multiplier as i128)? / 10_000;

    let vote = crate::proposals::Vote {
        voter: voter.clone(),
        vote_type: vote_type.clone(),
        voting_power: weighted,
        timestamp: env.ledger().timestamp(),
    };

    proposal.voters.set(voter.clone(), vote);
    proposal.voter_list.push_back(voter.clone());
    match vote_type {
        VoteType::For => proposal.votes_for = proposal.votes_for.saturating_add(weighted),
        VoteType::Against => {
            proposal.votes_against = proposal.votes_against.saturating_add(weighted)
        }
        VoteType::Abstain => {
            proposal.votes_abstain = proposal.votes_abstain.saturating_add(weighted)
        }
    }

    proposals::put_proposal(env, &proposal)?;
    record_vote(env, voter.clone(), proposal_id, vote_type)?;

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("repvote")),
        (
            proposal_id,
            voter,
            token_power,
            weighted,
            multiplier as i128,
        ),
    );

    Ok(())
}

pub fn get_reputation_leaderboard(env: &Env, limit: u32) -> Vec<(Address, u32)> {
    let state = get_reputation_state(env);
    let mut sorted: Vec<(Address, u32)> = Vec::new(env);

    let mut i = 0;
    while i < state.users.len() {
        let user = state.users.get(i).unwrap();
        let rep = state.reputations.get(user.clone()).unwrap();

        let mut inserted = false;
        let mut pos = 0;
        while pos < sorted.len() {
            let existing = sorted.get(pos).unwrap();
            if rep.reputation_score > existing.1 {
                sorted.insert(pos, (user.clone(), rep.reputation_score));
                inserted = true;
                break;
            }
            pos += 1;
        }
        if !inserted {
            sorted.push_back((user.clone(), rep.reputation_score));
        }
        i += 1;
    }

    let mut out: Vec<(Address, u32)> = Vec::new(env);
    let mut idx = 0;
    while idx < sorted.len() && idx < limit {
        out.push_back(sorted.get(idx).unwrap());
        idx += 1;
    }
    out
}

pub fn distribute_reputation_rewards(
    env: &Env,
    reward_pool: i128,
) -> Result<Vec<(Address, i128)>, GovernanceError> {
    if reward_pool <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }

    let leaders = get_reputation_leaderboard(env, 100);
    let mut total_reputation = 0u32;
    let mut i = 0;
    while i < leaders.len() {
        total_reputation = total_reputation.saturating_add(leaders.get(i).unwrap().1);
        i += 1;
    }

    let mut payouts: Vec<(Address, i128)> = Vec::new(env);
    if total_reputation == 0 {
        return Ok(payouts);
    }

    let mut idx = 0;
    while idx < leaders.len() {
        let (user, score) = leaders.get(idx).unwrap();
        let reward = checked_mul(reward_pool, score as i128)? / total_reputation as i128;
        if reward > 0 {
            payouts.push_back((user.clone(), reward));
        }
        idx += 1;
    }

    Ok(payouts)
}

fn apply_reputation_decay(env: &Env, score: u32, last_activity: u64, decay_rate: u32) -> u32 {
    if env.ledger().timestamp() <= last_activity || decay_rate == 0 {
        return score;
    }
    let inactive_days = (env.ledger().timestamp() - last_activity) / 86_400;
    if inactive_days == 0 {
        return score;
    }

    let decay_factor = 10_000u32.saturating_sub(decay_rate);
    let mut decayed = score as u64;
    let mut i = 0;
    let capped_days = if inactive_days > 365 {
        365
    } else {
        inactive_days
    };
    while i < capped_days {
        decayed = (decayed * decay_factor as u64) / 10_000;
        i += 1;
    }
    decayed as u32
}

fn calculate_badge_bonus(badges: &Vec<Badge>) -> u32 {
    let mut bonus = 0u32;
    let mut idx = 0;
    while idx < badges.len() {
        let badge = badges.get(idx).unwrap();
        bonus = bonus.saturating_add(match badge {
            Badge::ActiveVoter(_) => 200,
            Badge::ProposalAuthor(successful_proposals) => {
                min(500, successful_proposals.saturating_mul(100))
            }
            Badge::CommitteeMember(_) => 300,
            Badge::EarlyAdopter => 500,
            Badge::TopDelegator => 400,
            Badge::ConsistentParticipant(months) => min(600, months.saturating_mul(50)),
        });
        idx += 1;
    }
    min(2000, bonus)
}

fn check_and_award_badges(env: &Env, rep: &mut GovernanceReputation) {
    let mut awarded: Vec<String> = Vec::new(env);

    if rep.participation_history.votes_cast >= 50 {
        let badge = Badge::ActiveVoter(50);
        if !contains_badge(&rep.badges, &badge) {
            rep.badges.push_back(badge);
            awarded.push_back(String::from_str(env, "ActiveVoter"));
        }
    }

    if rep.participation_history.proposals_succeeded >= 10 {
        let badge = Badge::ProposalAuthor(10);
        if !contains_badge(&rep.badges, &badge) {
            rep.badges.push_back(badge);
            awarded.push_back(String::from_str(env, "ProposalAuthor"));
        }
    }

    if rep.participation_history.total_tokens_delegated >= 100_000 * PRECISION {
        let badge = Badge::TopDelegator;
        if !contains_badge(&rep.badges, &badge) {
            rep.badges.push_back(badge);
            awarded.push_back(String::from_str(env, "TopDelegator"));
        }
    }

    if !awarded.is_empty() {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("gov"), symbol_short!("badges")),
            (rep.user.clone(), awarded),
        );
    }
}

fn contains_badge(badges: &Vec<Badge>, target: &Badge) -> bool {
    let mut idx = 0;
    while idx < badges.len() {
        if badges.get(idx).unwrap() == *target {
            return true;
        }
        idx += 1;
    }
    false
}

fn upsert_rep(state: &mut ReputationState, user: &Address, rep: GovernanceReputation) {
    if !state.reputations.contains_key(user.clone()) {
        state.users.push_back(user.clone());
    }
    state.reputations.set(user.clone(), rep);
}
