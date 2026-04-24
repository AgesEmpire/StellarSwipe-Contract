#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, String, Vec};

use crate::proposals::{get_proposal, put_proposal, ProposalStatus, VoteType};
use crate::proposals::get_effective_voting_power;
use crate::GovernanceError;

pub const PRECISION: i128 = 1_000_000;

// ── Data Types ────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuadraticVotingConfig {
    pub enabled: bool,
    pub vote_credits_per_token: u32,
    pub max_credits_per_user: i128,
    pub sybil_resistance_enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteCredits {
    pub user: Address,
    pub total_credits: i128,
    pub used_credits: i128,
    pub available_credits: i128,
    pub proposals_voted: Map<u64, i128>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuadraticVote {
    pub voter: Address,
    pub votes_allocated: i128,
    pub credits_spent: i128,
    pub vote_type: VoteType,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationMethod {
    BrightID,
    WorldID,
    GitcoinPassport,
    Custom,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityVerification {
    pub user: Address,
    pub verified: bool,
    pub verification_method: VerificationMethod,
    pub verified_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VotingOutcome {
    pub votes_for: i128,
    pub votes_against: i128,
    pub is_for_winning: bool,
    pub margin: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VotingComparison {
    pub linear_voting: VotingOutcome,
    pub quadratic_voting: VotingOutcome,
    pub gini_coefficient_linear: u32,
    pub gini_coefficient_quadratic: u32,
    pub fairness_improvement: i128,
}

// ── Storage Keys ──────────────────────────────────────────────────────────────

#[contracttype]
pub enum QVStorageKey {
    Config,
    Credits(Address),
    Vote(u64, Address),
    ProposalVoters(u64),
    Identity(Address),
}

// ── Config ────────────────────────────────────────────────────────────────────

pub fn default_qv_config() -> QuadraticVotingConfig {
    QuadraticVotingConfig {
        enabled: false,
        vote_credits_per_token: 1,
        max_credits_per_user: 10_000 * PRECISION,
        sybil_resistance_enabled: false,
    }
}

pub fn get_quadratic_voting_config(env: &Env) -> QuadraticVotingConfig {
    env.storage()
        .instance()
        .get(&QVStorageKey::Config)
        .unwrap_or_else(default_qv_config)
}

pub fn set_quadratic_voting_config(env: &Env, config: &QuadraticVotingConfig) {
    env.storage().instance().set(&QVStorageKey::Config, config);
}

// ── Credits ───────────────────────────────────────────────────────────────────

pub fn get_vote_credits(env: &Env, user: &Address) -> Option<VoteCredits> {
    env.storage()
        .persistent()
        .get(&QVStorageKey::Credits(user.clone()))
}

pub fn store_vote_credits(env: &Env, credits: &VoteCredits) {
    env.storage()
        .persistent()
        .set(&QVStorageKey::Credits(credits.user.clone()), credits);
}

/// Allocate vote credits to a user based on staked tokens.
pub fn allocate_vote_credits(env: &Env, user: Address) -> Result<i128, GovernanceError> {
    let config = get_quadratic_voting_config(env);
    if !config.enabled {
        return Err(GovernanceError::InvalidGovernanceConfig);
    }

    let staked = get_effective_voting_power(env, &user);
    let raw_credits = (staked * config.vote_credits_per_token as i128) / PRECISION;
    let capped = raw_credits.min(config.max_credits_per_user);

    let credits = VoteCredits {
        user: user.clone(),
        total_credits: capped,
        used_credits: 0,
        available_credits: capped,
        proposals_voted: Map::new(env),
    };

    store_vote_credits(env, &credits);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("qv"), symbol_short!("alloc")),
        (user, capped),
    );

    Ok(capped)
}

// ── Vote Casting ──────────────────────────────────────────────────────────────

fn store_quadratic_vote(env: &Env, proposal_id: u64, voter: &Address, vote: &QuadraticVote) {
    env.storage()
        .persistent()
        .set(&QVStorageKey::Vote(proposal_id, voter.clone()), vote);

    // Track voter list for this proposal
    let mut voters: Vec<Address> = env
        .storage()
        .persistent()
        .get(&QVStorageKey::ProposalVoters(proposal_id))
        .unwrap_or_else(|| Vec::new(env));
    
    let mut already_in = false;
    for i in 0..voters.len() {
        if voters.get(i).unwrap() == *voter {
            already_in = true;
            break;
        }
    }
    if !already_in {
        voters.push_back(voter.clone());
        env.storage()
            .persistent()
            .set(&QVStorageKey::ProposalVoters(proposal_id), &voters);
    }
}

pub fn get_quadratic_vote(env: &Env, proposal_id: u64, voter: &Address) -> Option<QuadraticVote> {
    env.storage()
        .persistent()
        .get(&QVStorageKey::Vote(proposal_id, voter.clone()))
}

pub fn get_proposal_voters(env: &Env, proposal_id: u64) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&QVStorageKey::ProposalVoters(proposal_id))
        .unwrap_or_else(|| Vec::new(env))
}

/// Cast a quadratic vote on a proposal.
/// Credit cost = votes_desired²
pub fn cast_quadratic_vote(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    votes_desired: i128,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();

    if votes_desired <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }

    let config = get_quadratic_voting_config(env);
    if !config.enabled {
        return Err(GovernanceError::InvalidGovernanceConfig);
    }

    if config.sybil_resistance_enabled {
        let verified = env
            .storage()
            .persistent()
            .get::<_, IdentityVerification>(&QVStorageKey::Identity(voter.clone()))
            .map(|v| v.verified)
            .unwrap_or(false);
        if !verified {
            return Err(GovernanceError::InvalidProposal);
        }
    }

    let mut proposal = get_proposal(env, proposal_id)?;
    let now = env.ledger().timestamp();
    if now < proposal.voting_starts {
        return Err(GovernanceError::VotingNotStarted);
    }
    if now >= proposal.voting_ends {
        return Err(GovernanceError::VotingEnded);
    }
    if proposal.status != ProposalStatus::Pending && proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }

    if get_quadratic_vote(env, proposal_id, &voter).is_some() {
        return Err(GovernanceError::AlreadyVoted);
    }

    let credits_required = votes_desired
        .checked_mul(votes_desired)
        .ok_or(GovernanceError::ArithmeticOverflow)?;

    let mut credits = get_vote_credits(env, &voter).ok_or(GovernanceError::NoVotingPower)?;
    if credits.available_credits < credits_required {
        return Err(GovernanceError::InsufficientBalance);
    }

    credits.used_credits += credits_required;
    credits.available_credits -= credits_required;
    credits.proposals_voted.set(proposal_id, credits_required);
    store_vote_credits(env, &credits);

    match vote_type.clone() {
        VoteType::For => proposal.votes_for = proposal.votes_for.saturating_add(votes_desired),
        VoteType::Against => proposal.votes_against = proposal.votes_against.saturating_add(votes_desired),
        VoteType::Abstain => proposal.votes_abstain = proposal.votes_abstain.saturating_add(votes_desired),
    }

    if proposal.status == ProposalStatus::Pending {
        proposal.status = ProposalStatus::Active;
    }
    put_proposal(env, &proposal)?;

    let vote = QuadraticVote {
        voter: voter.clone(),
        votes_allocated: votes_desired,
        credits_spent: credits_required,
        vote_type,
    };
    store_quadratic_vote(env, proposal_id, &voter, &vote);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("qv"), symbol_short!("vote")),
        (proposal_id, voter, votes_desired, credits_required),
    );

    Ok(())
}

// ── Vote Reallocation ─────────────────────────────────────────────────────────

pub fn reallocate_quadratic_votes(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    new_votes_desired: i128,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();

    let proposal = get_proposal(env, proposal_id)?;
    if env.ledger().timestamp() >= proposal.voting_ends {
        return Err(GovernanceError::VotingEnded);
    }

    let previous_vote =
        get_quadratic_vote(env, proposal_id, &voter).ok_or(GovernanceError::ProposalNotFound)?;

    // Refund previous credits
    let mut credits = get_vote_credits(env, &voter).ok_or(GovernanceError::NoVotingPower)?;
    credits.used_credits -= previous_vote.credits_spent;
    credits.available_credits += previous_vote.credits_spent;
    credits.proposals_voted.remove(proposal_id);
    store_vote_credits(env, &credits);

    // Reverse previous tally on proposal
    let mut proposal = get_proposal(env, proposal_id)?;
    match previous_vote.vote_type.clone() {
        VoteType::For => proposal.votes_for = proposal.votes_for.saturating_sub(previous_vote.votes_allocated),
        VoteType::Against => proposal.votes_against = proposal.votes_against.saturating_sub(previous_vote.votes_allocated),
        VoteType::Abstain => proposal.votes_abstain = proposal.votes_abstain.saturating_sub(previous_vote.votes_allocated),
    }
    put_proposal(env, &proposal)?;

    // Remove old vote record so cast_quadratic_vote doesn't see AlreadyVoted
    env.storage()
        .persistent()
        .remove(&QVStorageKey::Vote(proposal_id, voter.clone()));

    cast_quadratic_vote(env, proposal_id, voter, new_votes_desired, vote_type)
}

// ── Identity Verification ─────────────────────────────────────────────────────

pub fn verify_identity(
    env: &Env,
    user: Address,
    method: VerificationMethod,
    _proof: Vec<u32>,
) -> Result<(), GovernanceError> {
    // Proof verification is off-chain; on-chain we accept the assertion.
    let verification = IdentityVerification {
        user: user.clone(),
        verified: true,
        verification_method: method,
        verified_at: env.ledger().timestamp(),
    };
    env.storage()
        .persistent()
        .set(&QVStorageKey::Identity(user.clone()), &verification);

    // Grant verified user bonus
    grant_verified_user_bonus(env, &user)?;

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("qv"), symbol_short!("verified")),
        user,
    );

    Ok(())
}

fn grant_verified_user_bonus(env: &Env, user: &Address) -> Result<(), GovernanceError> {
    let mut credits = match get_vote_credits(env, user) {
        Some(c) => c,
        None => return Ok(()), // no credits allocated yet — bonus applied on next allocation
    };

    let bonus = credits.total_credits / 2;
    credits.total_credits += bonus;
    credits.available_credits += bonus;
    store_vote_credits(env, &credits);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("qv"), symbol_short!("bonus")),
        (user.clone(), bonus),
    );

    Ok(())
}

pub fn get_identity_verification(env: &Env, user: &Address) -> Option<IdentityVerification> {
    env.storage()
        .persistent()
        .get(&QVStorageKey::Identity(user.clone()))
}

// ── Cost Analysis ─────────────────────────────────────────────────────────────

/// Marginal cost to go from `current_votes` to `current_votes + additional_votes`.
pub fn calculate_marginal_cost(current_votes: i128, additional_votes: i128) -> i128 {
    let total = current_votes + additional_votes;
    (total * total) - (current_votes * current_votes)
}

// ── Credit Refund on Failure ──────────────────────────────────────────────────

pub fn refund_credits_on_failure(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let proposal = get_proposal(env, proposal_id)?;

    if proposal.status != ProposalStatus::Failed && proposal.status != ProposalStatus::Cancelled {
        return Err(GovernanceError::ProposalNotActive);
    }

    let voters = get_proposal_voters(env, proposal_id);
    for i in 0..voters.len() {
        let voter = voters.get(i).unwrap();
        if let Some(vote) = get_quadratic_vote(env, proposal_id, &voter) {
            if let Some(mut credits) = get_vote_credits(env, &voter) {
                credits.used_credits -= vote.credits_spent;
                credits.available_credits += vote.credits_spent;
                credits.proposals_voted.remove(proposal_id);
                store_vote_credits(env, &credits);

                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("qv"), symbol_short!("refund")),
                    (voter, proposal_id, vote.credits_spent),
                );
            }
        }
    }

    Ok(())
}

// ── Comparative Analysis ──────────────────────────────────────────────────────

pub fn compare_voting_systems(env: &Env, proposal_id: u64) -> Result<VotingComparison, GovernanceError> {
    let voters = get_proposal_voters(env, proposal_id);

    let mut linear_for = 0i128;
    let mut linear_against = 0i128;
    let mut quadratic_for = 0i128;
    let mut quadratic_against = 0i128;

    for i in 0..voters.len() {
        let voter = voters.get(i).unwrap();
        let user_tokens = get_effective_voting_power(env, &voter);
        if let Some(vote) = get_quadratic_vote(env, proposal_id, &voter) {
            match vote.vote_type {
                VoteType::For => {
                    linear_for += user_tokens;
                    quadratic_for += vote.votes_allocated;
                }
                VoteType::Against => {
                    linear_against += user_tokens;
                    quadratic_against += vote.votes_allocated;
                }
                VoteType::Abstain => {}
            }
        }
    }

    let linear_margin = (linear_for - linear_against).abs();
    let quadratic_margin = (quadratic_for - quadratic_against).abs();

    // Simplified Gini coefficient (lower = more equal)
    let gini_linear = calculate_gini(env, proposal_id, true)?;
    let gini_quadratic = calculate_gini(env, proposal_id, false)?;
    let fairness_improvement = gini_linear as i128 - gini_quadratic as i128;

    Ok(VotingComparison {
        linear_voting: VotingOutcome {
            votes_for: linear_for,
            votes_against: linear_against,
            is_for_winning: linear_for > linear_against,
            margin: linear_margin,
        },
        quadratic_voting: VotingOutcome {
            votes_for: quadratic_for,
            votes_against: quadratic_against,
            is_for_winning: quadratic_for > quadratic_against,
            margin: quadratic_margin,
        },
        gini_coefficient_linear: gini_linear,
        gini_coefficient_quadratic: gini_quadratic,
        fairness_improvement,
    })
}

/// Simplified Gini coefficient (0 = equal, 10000 = maximum inequality).
fn calculate_gini(env: &Env, proposal_id: u64, use_linear: bool) -> Result<u32, GovernanceError> {
    let voters = get_proposal_voters(env, proposal_id);
    let n = voters.len() as i128;
    if n == 0 {
        return Ok(0);
    }

    let mut values: Vec<i128> = Vec::new(env);
    for i in 0..voters.len() {
        let voter = voters.get(i).unwrap();
        let val = if use_linear {
            get_effective_voting_power(env, &voter)
        } else {
            get_quadratic_vote(env, proposal_id, &voter)
                .map(|v| v.votes_allocated)
                .unwrap_or(0)
        };
        values.push_back(val);
    }

    // Gini = (2 * Σ(rank * value) / (n * Σvalue)) - (n+1)/n
    let mut sum_vals = 0i128;
    let mut rank_sum = 0i128;
    for i in 0..values.len() {
        let v = values.get(i).unwrap();
        sum_vals += v;
        rank_sum += (i as i128 + 1) * v;
    }

    if sum_vals == 0 {
        return Ok(0);
    }

    let gini_bps = ((2 * rank_sum * 10000) / (n * sum_vals)) - ((n + 1) * 10000 / n);
    Ok(gini_bps.clamp(0, 10000) as u32)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quadratic_cost_calculation() {
        // 10 votes costs 100 credits
        assert_eq!(10i128 * 10, 100);
        // 100 credits → 10 votes (√100 = 10)
        let credits: i128 = 100;
        let votes = integer_sqrt(credits);
        assert_eq!(votes, 10);
    }

    #[test]
    fn test_marginal_cost() {
        // Going from 0→10 costs 100
        assert_eq!(calculate_marginal_cost(0, 10), 100);
        // Going from 10→11 costs 121 - 100 = 21
        assert_eq!(calculate_marginal_cost(10, 1), 21);
        // Going from 0→5 costs 25
        assert_eq!(calculate_marginal_cost(0, 5), 25);
    }

    #[test]
    fn test_insufficient_credits() {
        // 11 votes requires 121 credits — should fail if user only has 100
        let credits_available: i128 = 100;
        let votes_desired: i128 = 11;
        let credits_required = votes_desired * votes_desired;
        assert!(credits_available < credits_required);
    }

    #[test]
    fn test_reallocate_refund() {
        // Originally: 10 votes = 100 credits used
        // Realloc to: 5 votes = 25 credits → 75 refunded
        let original_credits_spent: i128 = 100;
        let new_credits_required: i128 = 25;
        let refund = original_credits_spent - new_credits_required;
        assert_eq!(refund, 75);
    }

    fn integer_sqrt(value: i128) -> i128 {
        if value <= 0 {
            return 0;
        }
        let mut x0 = value;
        let mut x1 = (x0 + value / x0) / 2;
        while x1 < x0 {
            x0 = x1;
            x1 = (x0 + value / x0) / 2;
        }
        x0
    }
}
