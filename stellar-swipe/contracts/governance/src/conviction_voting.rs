use core::cmp::max;

use soroban_sdk::{contracttype, symbol_short, Address, Env, Map, String, Vec};

use crate::{checked_mul, GovernanceError, StorageKey};

const PRECISION: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConvictionStatus {
    Active,
    Funded,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvictionVote {
    pub voter: Address,
    pub tokens_committed: i128,
    pub vote_started: u64,
    pub last_conviction_update: u64,
    pub current_conviction: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvictionProposal {
    pub id: u64,
    pub proposer: Address,
    pub title: String,
    pub requested_amount: i128,
    pub beneficiary: Address,
    pub conviction_accumulated: i128,
    pub conviction_threshold: i128,
    pub status: ConvictionStatus,
    pub votes: Map<Address, ConvictionVote>,
    pub voters: Vec<Address>,
    pub funding_granted: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvictionVotingPool {
    pub pool_id: u64,
    pub funding_amount: i128,
    pub refill_rate: i128,
    pub refill_period: u64,
    pub proposals: Map<u64, ConvictionProposal>,
    pub proposal_ids: Vec<u64>,
    pub total_conviction: i128,
    pub last_refill: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvictionState {
    pub pools: Map<u64, ConvictionVotingPool>,
    pub pool_ids: Vec<u64>,
    pub next_pool_id: u64,
    pub next_proposal_id: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvictionAnalytics {
    pub proposal_id: u64,
    pub total_voters: u32,
    pub total_tokens_committed: i128,
    pub current_conviction: i128,
    pub conviction_threshold: i128,
    pub progress_pct: u32,
    pub avg_vote_age_days: u32,
    pub estimated_funding_time: Option<u64>,
}

pub fn empty_conviction_state(env: &Env) -> ConvictionState {
    ConvictionState {
        pools: Map::new(env),
        pool_ids: Vec::new(env),
        next_pool_id: 1,
        next_proposal_id: 1,
    }
}

pub fn get_conviction_state(env: &Env) -> ConvictionState {
    env.storage()
        .instance()
        .get(&StorageKey::ConvictionState)
        .unwrap_or_else(|| empty_conviction_state(env))
}

pub fn put_conviction_state(env: &Env, state: &ConvictionState) {
    env.storage()
        .instance()
        .set(&StorageKey::ConvictionState, state);
}

pub fn create_conviction_pool(
    env: &Env,
    funding_amount: i128,
    refill_rate: i128,
    refill_period: u64,
) -> Result<u64, GovernanceError> {
    if funding_amount <= 0 || refill_rate < 0 || refill_period == 0 {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }

    let mut state = get_conviction_state(env);
    let pool_id = state.next_pool_id;
    let pool = ConvictionVotingPool {
        pool_id,
        funding_amount,
        refill_rate,
        refill_period,
        proposals: Map::new(env),
        proposal_ids: Vec::new(env),
        total_conviction: 0,
        last_refill: env.ledger().timestamp(),
    };

    state.pools.set(pool_id, pool);
    state.pool_ids.push_back(pool_id);
    state.next_pool_id = pool_id.saturating_add(1);
    put_conviction_state(env, &state);

    Ok(pool_id)
}

pub fn create_conviction_proposal(
    env: &Env,
    pool_id: u64,
    proposer: Address,
    title: String,
    requested_amount: i128,
    beneficiary: Address,
) -> Result<u64, GovernanceError> {
    proposer.require_auth();
    if requested_amount <= 0 || title.is_empty() {
        return Err(GovernanceError::InvalidProposal);
    }

    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;

    if requested_amount > pool.funding_amount {
        return Err(GovernanceError::InsufficientBalance);
    }

    let proposal_id = state.next_proposal_id;
    let threshold = calculate_conviction_threshold(
        requested_amount,
        pool.funding_amount,
        if pool.total_conviction > 0 {
            pool.total_conviction
        } else {
            1_000
        },
    )?;

    let proposal = ConvictionProposal {
        id: proposal_id,
        proposer,
        title,
        requested_amount,
        beneficiary,
        conviction_accumulated: 0,
        conviction_threshold: threshold,
        status: ConvictionStatus::Active,
        votes: Map::new(env),
        voters: Vec::new(env),
        funding_granted: 0,
    };

    pool.proposals.set(proposal_id, proposal);
    pool.proposal_ids.push_back(proposal_id);
    state.pools.set(pool_id, pool);
    state.next_proposal_id = proposal_id.saturating_add(1);
    put_conviction_state(env, &state);

    Ok(proposal_id)
}

pub fn vote_conviction(
    env: &Env,
    pool_id: u64,
    proposal_id: u64,
    voter: Address,
    tokens_to_commit: i128,
) -> Result<(), GovernanceError> {
    voter.require_auth();
    if tokens_to_commit <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }

    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;
    let mut proposal = pool
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    if proposal.status != ConvictionStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }

    if let Some(mut existing) = proposal.votes.get(voter.clone()) {
        update_conviction_vote(env, &mut existing, tokens_to_commit)?;
        proposal.votes.set(voter.clone(), existing);
    } else {
        let vote = ConvictionVote {
            voter: voter.clone(),
            tokens_committed: tokens_to_commit,
            vote_started: env.ledger().timestamp(),
            last_conviction_update: env.ledger().timestamp(),
            current_conviction: 0,
        };
        proposal.votes.set(voter.clone(), vote);
        proposal.voters.push_back(voter.clone());
    }

    pool.proposals.set(proposal_id, proposal);
    state.pools.set(pool_id, pool);
    put_conviction_state(env, &state);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("cvote")),
        (pool_id, proposal_id, voter, tokens_to_commit),
    );

    Ok(())
}

pub fn update_proposal_conviction(
    env: &Env,
    pool_id: u64,
    proposal_id: u64,
) -> Result<i128, GovernanceError> {
    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;
    let mut proposal = pool
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    let mut total_conviction = 0i128;
    let mut idx = 0;
    while idx < proposal.voters.len() {
        let voter = proposal.voters.get(idx).unwrap();
        if let Some(mut vote) = proposal.votes.get(voter.clone()) {
            update_vote_conviction(env, &mut vote)?;
            total_conviction = total_conviction.saturating_add(vote.current_conviction);
            proposal.votes.set(voter, vote);
        }
        idx += 1;
    }

    proposal.conviction_accumulated = total_conviction;
    if total_conviction >= proposal.conviction_threshold
        && proposal.status == ConvictionStatus::Active
    {
        execute_conviction_funding_internal(&mut pool, &mut proposal)?;
    }

    let proposal_conviction = proposal.conviction_accumulated;
    pool.proposals.set(proposal_id, proposal);
    state.pools.set(pool_id, pool);
    put_conviction_state(env, &state);

    Ok(proposal_conviction)
}

pub fn execute_conviction_funding(
    env: &Env,
    pool_id: u64,
    proposal_id: u64,
) -> Result<(), GovernanceError> {
    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;
    let mut proposal = pool
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    execute_conviction_funding_internal(&mut pool, &mut proposal)?;

    pool.proposals.set(proposal_id, proposal);
    state.pools.set(pool_id, pool);
    put_conviction_state(env, &state);
    Ok(())
}

pub fn change_conviction_vote(
    env: &Env,
    pool_id: u64,
    from_proposal: u64,
    to_proposal: u64,
    voter: Address,
) -> Result<(), GovernanceError> {
    voter.require_auth();

    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;

    let mut source = pool
        .proposals
        .get(from_proposal)
        .ok_or(GovernanceError::ProposalNotFound)?;
    let vote = source
        .votes
        .get(voter.clone())
        .ok_or(GovernanceError::CrossCommitteeRequestNotFound)?;
    source.votes.remove(voter.clone());

    let decayed_tokens = apply_vote_switch_decay(vote.tokens_committed);

    source.conviction_accumulated = source
        .conviction_accumulated
        .saturating_sub(vote.current_conviction);

    let mut target = pool
        .proposals
        .get(to_proposal)
        .ok_or(GovernanceError::ProposalNotFound)?;

    let new_vote = ConvictionVote {
        voter: voter.clone(),
        tokens_committed: decayed_tokens,
        vote_started: env.ledger().timestamp(),
        last_conviction_update: env.ledger().timestamp(),
        current_conviction: 0,
    };

    target.votes.set(voter.clone(), new_vote);
    if !contains_voter(&target.voters, &voter) {
        target.voters.push_back(voter.clone());
    }

    pool.proposals.set(from_proposal, source);
    pool.proposals.set(to_proposal, target);
    state.pools.set(pool_id, pool);
    put_conviction_state(env, &state);

    Ok(())
}

pub fn refill_conviction_pool(env: &Env, pool_id: u64) -> Result<i128, GovernanceError> {
    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;

    let now = env.ledger().timestamp();
    let elapsed = now.saturating_sub(pool.last_refill);
    if elapsed < pool.refill_period {
        return Ok(0);
    }

    let refills = elapsed / pool.refill_period;
    let refill_amount = pool.refill_rate.saturating_mul(refills as i128);
    pool.funding_amount = pool.funding_amount.saturating_add(refill_amount);
    pool.last_refill = now;

    state.pools.set(pool_id, pool);
    put_conviction_state(env, &state);

    Ok(refill_amount)
}

pub fn withdraw_conviction_vote(
    env: &Env,
    pool_id: u64,
    proposal_id: u64,
    voter: Address,
) -> Result<i128, GovernanceError> {
    voter.require_auth();

    let mut state = get_conviction_state(env);
    let mut pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;
    let mut proposal = pool
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    let vote = proposal
        .votes
        .get(voter.clone())
        .ok_or(GovernanceError::CrossCommitteeRequestNotFound)?;
    proposal.votes.remove(voter);

    proposal.conviction_accumulated = proposal
        .conviction_accumulated
        .saturating_sub(vote.current_conviction);

    let lost = vote.current_conviction;
    pool.proposals.set(proposal_id, proposal);
    state.pools.set(pool_id, pool);
    put_conviction_state(env, &state);

    Ok(lost)
}

pub fn analyze_conviction_proposal(
    env: &Env,
    pool_id: u64,
    proposal_id: u64,
) -> Result<ConvictionAnalytics, GovernanceError> {
    let state = get_conviction_state(env);
    let pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;
    let proposal = pool
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    let total_voters = proposal.voters.len();
    let mut total_tokens = 0i128;
    let mut total_age = 0u64;

    let mut idx = 0;
    while idx < proposal.voters.len() {
        let voter = proposal.voters.get(idx).unwrap();
        if let Some(vote) = proposal.votes.get(voter) {
            total_tokens = total_tokens.saturating_add(vote.tokens_committed);
            total_age = total_age
                .saturating_add(env.ledger().timestamp().saturating_sub(vote.vote_started));
        }
        idx += 1;
    }

    let progress_pct = if proposal.conviction_threshold > 0 {
        ((proposal.conviction_accumulated.saturating_mul(10_000)) / proposal.conviction_threshold)
            as u32
    } else {
        0
    };

    let avg_vote_age_days = if total_voters > 0 {
        (total_age / total_voters as u64 / 86_400) as u32
    } else {
        0
    };

    let estimated_funding_time = if progress_pct < 10_000 {
        estimate_conviction_completion(env, &proposal)
    } else {
        None
    };

    Ok(ConvictionAnalytics {
        proposal_id,
        total_voters,
        total_tokens_committed: total_tokens,
        current_conviction: proposal.conviction_accumulated,
        conviction_threshold: proposal.conviction_threshold,
        progress_pct,
        avg_vote_age_days,
        estimated_funding_time,
    })
}

pub fn get_conviction_growth_curve(
    env: &Env,
    pool_id: u64,
    proposal_id: u64,
    days: u32,
) -> Result<Vec<(u64, i128)>, GovernanceError> {
    let state = get_conviction_state(env);
    let pool = state
        .pools
        .get(pool_id)
        .ok_or(GovernanceError::ConvictionPoolNotFound)?;
    let proposal = pool
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    let mut curve: Vec<(u64, i128)> = Vec::new(env);
    let mut day = 0;
    while day <= days {
        let ts = env.ledger().timestamp().saturating_add(day as u64 * 86_400);
        let mut projected = 0i128;

        let mut idx = 0;
        while idx < proposal.voters.len() {
            let voter = proposal.voters.get(idx).unwrap();
            if let Some(vote) = proposal.votes.get(voter) {
                let elapsed = ts.saturating_sub(vote.vote_started);
                let conviction = calculate_conviction(vote.tokens_committed, elapsed);
                projected = projected.saturating_add(conviction);
            }
            idx += 1;
        }

        curve.push_back((ts, projected));
        day += 1;
    }

    Ok(curve)
}

fn calculate_conviction(tokens: i128, time_elapsed: u64) -> i128 {
    let days_elapsed = time_elapsed / 86_400;
    if days_elapsed == 0 {
        return 0;
    }

    let sqrt_days = integer_sqrt(days_elapsed as i128);
    tokens.saturating_mul(sqrt_days) / 1000
}

fn calculate_conviction_threshold(
    requested_amount: i128,
    total_pool: i128,
    total_conviction: i128,
) -> Result<i128, GovernanceError> {
    if requested_amount <= 0 || total_pool <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let amount_ratio = checked_mul(requested_amount, PRECISION)? / total_pool;
    let ratio_squared = checked_mul(amount_ratio, amount_ratio)? / PRECISION;
    let alpha = 200i128;
    let threshold =
        checked_mul(ratio_squared, total_conviction)?.saturating_mul(alpha) / (PRECISION * 100);
    Ok(max(1000, threshold))
}

fn integer_sqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

fn update_vote_conviction(env: &Env, vote: &mut ConvictionVote) -> Result<(), GovernanceError> {
    let elapsed = env.ledger().timestamp().saturating_sub(vote.vote_started);
    vote.current_conviction = calculate_conviction(vote.tokens_committed, elapsed);
    vote.last_conviction_update = env.ledger().timestamp();
    Ok(())
}

fn update_conviction_vote(
    env: &Env,
    vote: &mut ConvictionVote,
    new_tokens: i128,
) -> Result<(), GovernanceError> {
    if new_tokens > vote.tokens_committed {
        vote.tokens_committed = new_tokens;
        vote.vote_started = env.ledger().timestamp();
        vote.last_conviction_update = env.ledger().timestamp();
        vote.current_conviction = 0;
    } else if new_tokens < vote.tokens_committed {
        update_vote_conviction(env, vote)?;
        let reduction_ratio = checked_mul(new_tokens, PRECISION)? / vote.tokens_committed;
        vote.current_conviction =
            checked_mul(vote.current_conviction, reduction_ratio)? / PRECISION;
        vote.tokens_committed = new_tokens;
    }
    Ok(())
}

fn execute_conviction_funding_internal(
    pool: &mut ConvictionVotingPool,
    proposal: &mut ConvictionProposal,
) -> Result<(), GovernanceError> {
    if proposal.conviction_accumulated < proposal.conviction_threshold {
        return Err(GovernanceError::InvalidCommitteeAction);
    }
    if pool.funding_amount < proposal.requested_amount {
        return Err(GovernanceError::InsufficientBalance);
    }

    pool.funding_amount = pool
        .funding_amount
        .saturating_sub(proposal.requested_amount);
    proposal.status = ConvictionStatus::Funded;
    proposal.funding_granted = proposal.requested_amount;

    Ok(())
}

fn apply_vote_switch_decay(tokens: i128) -> i128 {
    tokens.saturating_sub(tokens / 10)
}

fn estimate_conviction_completion(env: &Env, proposal: &ConvictionProposal) -> Option<u64> {
    if proposal.voters.is_empty() {
        return None;
    }

    let mut total_tokens = 0i128;
    let mut idx = 0;
    while idx < proposal.voters.len() {
        let voter = proposal.voters.get(idx).unwrap();
        if let Some(vote) = proposal.votes.get(voter) {
            total_tokens = total_tokens.saturating_add(vote.tokens_committed);
        }
        idx += 1;
    }

    if total_tokens <= 0 {
        return None;
    }

    let remaining = proposal
        .conviction_threshold
        .saturating_sub(proposal.conviction_accumulated);
    if remaining <= 0 {
        return Some(env.ledger().timestamp());
    }

    let growth_per_day = (total_tokens.saturating_mul(10)).max(1);
    let days_needed = remaining.saturating_mul(1000) / growth_per_day;
    Some(
        env.ledger()
            .timestamp()
            .saturating_add(days_needed as u64 * 86_400),
    )
}

fn contains_voter(voters: &Vec<Address>, target: &Address) -> bool {
    let mut i = 0;
    while i < voters.len() {
        if voters.get(i).unwrap() == *target {
            return true;
        }
        i += 1;
    }
    false
}
