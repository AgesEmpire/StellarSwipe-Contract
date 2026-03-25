use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Map, String, Vec};
use stellar_swipe_common::Asset;

use crate::{
    add_balance, checked_add, checked_mul, checked_sub, get_staked_balance, get_total_supply,
    get_treasury, put_treasury, require_admin, GovernanceError, StorageKey,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalType {
    ParameterChange(String, i128, i128),
    TreasurySpend(Address, i128, Asset, String),
    FeatureToggle(String, bool),
    ContractUpgrade(String, Bytes),
    SignalProposal(String),
    Custom(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Pending,
    Active,
    Succeeded,
    Failed,
    Executed,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoteType {
    For,
    Against,
    Abstain,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Vote {
    pub voter: Address,
    pub vote_type: VoteType,
    pub voting_power: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub proposal_type: ProposalType,
    pub title: String,
    pub description: String,
    pub execution_payload: Bytes,
    pub voting_starts: u64,
    pub voting_ends: u64,
    pub votes_for: i128,
    pub votes_against: i128,
    pub votes_abstain: i128,
    pub status: ProposalStatus,
    pub voters: Map<Address, Vote>,
    pub voter_list: Vec<Address>,
    pub executed_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernanceConfig {
    pub min_proposal_threshold: i128,
    pub voting_period: u64,
    pub voting_delay: u64,
    pub quorum_threshold: u32,
    pub approval_threshold: u32,
    pub execution_delay: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalStatistics {
    pub total_proposals: u32,
    pub active_proposals: u32,
    pub succeeded_proposals: u32,
    pub failed_proposals: u32,
    pub executed_proposals: u32,
    pub avg_participation_rate: u32,
    pub avg_approval_rate: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteDelegation {
    pub delegator: Address,
    pub delegate: Address,
    pub delegated_power: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegationState {
    pub delegations: Map<Address, VoteDelegation>,
    pub delegators: Vec<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalsState {
    pub proposals: Map<u64, Proposal>,
    pub proposal_ids: Vec<u64>,
    pub next_proposal_id: u64,
}

const BPS_DENOMINATOR: i128 = 10_000;

pub fn default_governance_config() -> GovernanceConfig {
    GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: 7 * 24 * 60 * 60,
        voting_delay: 60,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 0,
    }
}

pub fn empty_proposals_state(env: &Env) -> ProposalsState {
    ProposalsState {
        proposals: Map::new(env),
        proposal_ids: Vec::new(env),
        next_proposal_id: 1,
    }
}

pub fn empty_delegation_state(env: &Env) -> DelegationState {
    DelegationState {
        delegations: Map::new(env),
        delegators: Vec::new(env),
    }
}

pub fn get_governance_config(env: &Env) -> GovernanceConfig {
    env.storage()
        .instance()
        .get(&StorageKey::GovernanceConfig)
        .unwrap_or_else(default_governance_config)
}

pub fn configure_governance(
    env: &Env,
    admin: &Address,
    config: GovernanceConfig,
) -> Result<GovernanceConfig, GovernanceError> {
    require_admin(env, admin)?;
    if config.min_proposal_threshold <= 0
        || config.voting_period == 0
        || config.quorum_threshold > 10_000
        || config.approval_threshold > 10_000
    {
        return Err(GovernanceError::InvalidGovernanceConfig);
    }
    env.storage()
        .instance()
        .set(&StorageKey::GovernanceConfig, &config);
    Ok(config)
}

pub fn get_proposals_state(env: &Env) -> ProposalsState {
    env.storage()
        .instance()
        .get(&StorageKey::ProposalsState)
        .unwrap_or_else(|| empty_proposals_state(env))
}

pub fn put_proposals_state(env: &Env, state: &ProposalsState) {
    env.storage().instance().set(&StorageKey::ProposalsState, state);
}

pub fn get_delegation_state(env: &Env) -> DelegationState {
    env.storage()
        .instance()
        .get(&StorageKey::Delegations)
        .unwrap_or_else(|| empty_delegation_state(env))
}

pub fn put_delegation_state(env: &Env, state: &DelegationState) {
    env.storage().instance().set(&StorageKey::Delegations, state);
}

pub fn create_proposal(
    env: &Env,
    proposer: Address,
    proposal_type: ProposalType,
    title: String,
    description: String,
    execution_payload: Bytes,
) -> Result<u64, GovernanceError> {
    proposer.require_auth();
    if title.is_empty() || description.is_empty() {
        return Err(GovernanceError::InvalidProposal);
    }

    let config = get_governance_config(env);
    let power = get_effective_voting_power(env, &proposer);
    if power < config.min_proposal_threshold {
        return Err(GovernanceError::NoVotingPower);
    }

    validate_proposal(env, &proposal_type)?;

    let mut state = get_proposals_state(env);
    let id = state.next_proposal_id;
    let now = env.ledger().timestamp();

    let proposal = Proposal {
        id,
        proposer: proposer.clone(),
        proposal_type,
        title,
        description,
        execution_payload,
        voting_starts: now.saturating_add(config.voting_delay),
        voting_ends: now
            .saturating_add(config.voting_delay)
            .saturating_add(config.voting_period),
        votes_for: 0,
        votes_against: 0,
        votes_abstain: 0,
        status: ProposalStatus::Pending,
        voters: Map::new(env),
        voter_list: Vec::new(env),
        executed_at: None,
    };

    state.proposals.set(id, proposal.clone());
    state.proposal_ids.push_back(id);
    state.next_proposal_id = id.saturating_add(1);
    put_proposals_state(env, &state);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("propnew")),
        (id, proposer, proposal.voting_starts, proposal.voting_ends),
    );

    Ok(id)
}

pub fn get_proposal(env: &Env, proposal_id: u64) -> Result<Proposal, GovernanceError> {
    get_proposals_state(env)
        .proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)
}

pub fn put_proposal(env: &Env, proposal: &Proposal) -> Result<(), GovernanceError> {
    let mut state = get_proposals_state(env);
    if !state.proposals.contains_key(proposal.id) {
        return Err(GovernanceError::ProposalNotFound);
    }
    state.proposals.set(proposal.id, proposal.clone());
    put_proposals_state(env, &state);
    Ok(())
}

pub fn cast_vote(
    env: &Env,
    proposal_id: u64,
    voter: Address,
    vote_type: VoteType,
) -> Result<(), GovernanceError> {
    voter.require_auth();
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
    if proposal.voters.contains_key(voter.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    let power = get_effective_voting_power(env, &voter);
    if power <= 0 {
        return Err(GovernanceError::NoVotingPower);
    }

    let vote = Vote {
        voter: voter.clone(),
        vote_type: vote_type.clone(),
        voting_power: power,
        timestamp: now,
    };
    proposal.voters.set(voter.clone(), vote);
    proposal.voter_list.push_back(voter.clone());

    match vote_type {
        VoteType::For => proposal.votes_for = checked_add(proposal.votes_for, power)?,
        VoteType::Against => proposal.votes_against = checked_add(proposal.votes_against, power)?,
        VoteType::Abstain => proposal.votes_abstain = checked_add(proposal.votes_abstain, power)?,
    }

    if proposal.status == ProposalStatus::Pending {
        proposal.status = ProposalStatus::Active;
    }
    put_proposal(env, &proposal)
}

pub fn finalize_proposal(env: &Env, proposal_id: u64) -> Result<ProposalStatus, GovernanceError> {
    let mut proposal = get_proposal(env, proposal_id)?;
    if env.ledger().timestamp() < proposal.voting_ends {
        return Err(GovernanceError::InvalidDuration);
    }
    if proposal.status != ProposalStatus::Pending && proposal.status != ProposalStatus::Active {
        return Err(GovernanceError::ProposalNotActive);
    }

    let cfg = get_governance_config(env);
    let total_votes = proposal
        .votes_for
        .saturating_add(proposal.votes_against)
        .saturating_add(proposal.votes_abstain);
    let total_supply = get_total_supply(env)?;

    if total_supply <= 0 {
        return Err(GovernanceError::InvalidSupply);
    }

    let quorum_met = total_votes.saturating_mul(BPS_DENOMINATOR)
        >= total_supply.saturating_mul(cfg.quorum_threshold as i128);

    if !quorum_met {
        proposal.status = ProposalStatus::Failed;
        put_proposal(env, &proposal)?;
        return Ok(ProposalStatus::Failed);
    }

    let cast_votes = proposal.votes_for.saturating_add(proposal.votes_against);
    let approved = cast_votes > 0
        && proposal.votes_for.saturating_mul(BPS_DENOMINATOR)
            >= cast_votes.saturating_mul(cfg.approval_threshold as i128);

    proposal.status = if approved {
        ProposalStatus::Succeeded
    } else {
        ProposalStatus::Failed
    };
    let status = proposal.status.clone();
    put_proposal(env, &proposal)?;

    if status == ProposalStatus::Succeeded && cfg.execution_delay == 0 {
        let _ = execute_proposal(env, proposal_id, proposal.proposer.clone());
    }

    Ok(status)
}

pub fn execute_proposal(
    env: &Env,
    proposal_id: u64,
    executor: Address,
) -> Result<ProposalStatus, GovernanceError> {
    executor.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;
    if proposal.status != ProposalStatus::Succeeded {
        return Err(GovernanceError::ProposalNotApproved);
    }

    let ready = proposal
        .voting_ends
        .saturating_add(get_governance_config(env).execution_delay);
    if env.ledger().timestamp() < ready {
        return Err(GovernanceError::InvalidDuration);
    }

    execute_proposal_action(env, &proposal)?;
    proposal.status = ProposalStatus::Executed;
    proposal.executed_at = Some(env.ledger().timestamp());
    put_proposal(env, &proposal)?;
    Ok(ProposalStatus::Executed)
}

pub fn execute_proposal_action(env: &Env, proposal: &Proposal) -> Result<(), GovernanceError> {
    match &proposal.proposal_type {
        ProposalType::ParameterChange(parameter, _current, proposed) => {
            let mut params: Map<String, i128> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceParameters)
                .unwrap_or(Map::new(env));
            params.set(parameter.clone(), *proposed);
            env.storage()
                .instance()
                .set(&StorageKey::GovernanceParameters, &params);
        }
        ProposalType::TreasurySpend(recipient, amount, asset, _purpose) => {
            let mut treasury = get_treasury(env);
            let bal = treasury.assets.get(asset.clone()).unwrap_or(0);
            if bal < *amount {
                return Err(GovernanceError::InsufficientBalance);
            }
            treasury.assets.set(asset.clone(), checked_sub(bal, *amount)?);
            put_treasury(env, &treasury);
            add_balance(env, recipient, *amount)?;
        }
        ProposalType::FeatureToggle(feature, enabled) => {
            let mut flags: Map<String, bool> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceFeatures)
                .unwrap_or(Map::new(env));
            flags.set(feature.clone(), *enabled);
            env.storage()
                .instance()
                .set(&StorageKey::GovernanceFeatures, &flags);
        }
        ProposalType::ContractUpgrade(contract_name, new_hash) => {
            let mut upgrades: Map<String, Bytes> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceUpgrades)
                .unwrap_or(Map::new(env));
            upgrades.set(contract_name.clone(), new_hash.clone());
            env.storage()
                .instance()
                .set(&StorageKey::GovernanceUpgrades, &upgrades);
        }
        ProposalType::SignalProposal(_) => {}
        ProposalType::Custom(_) => {}
    }
    Ok(())
}

pub fn execute_proposal_action_by_id(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let proposal = get_proposal(env, proposal_id)?;
    execute_proposal_action(env, &proposal)
}

pub fn mark_proposal_executed(env: &Env, proposal_id: u64) -> Result<(), GovernanceError> {
    let mut proposal = get_proposal(env, proposal_id)?;
    proposal.status = ProposalStatus::Executed;
    proposal.executed_at = Some(env.ledger().timestamp());
    put_proposal(env, &proposal)
}

pub fn cancel_proposal(
    env: &Env,
    proposal_id: u64,
    canceller: Address,
) -> Result<ProposalStatus, GovernanceError> {
    canceller.require_auth();
    let mut proposal = get_proposal(env, proposal_id)?;
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(GovernanceError::NotInitialized)?;

    let guardian_ok = env
        .storage()
        .instance()
        .get::<_, Address>(&StorageKey::Guardian)
        .map(|g| g == canceller)
        .unwrap_or(false);

    if canceller != proposal.proposer && canceller != admin && !guardian_ok {
        return Err(GovernanceError::Unauthorized);
    }
    if proposal.status == ProposalStatus::Executed {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    proposal.status = ProposalStatus::Cancelled;
    put_proposal(env, &proposal)?;
    Ok(ProposalStatus::Cancelled)
}

pub fn calculate_proposal_statistics(env: &Env) -> Result<ProposalStatistics, GovernanceError> {
    let state = get_proposals_state(env);
    let total_supply = get_total_supply(env)?;

    let mut total = 0u32;
    let mut active = 0u32;
    let mut succeeded = 0u32;
    let mut failed = 0u32;
    let mut executed = 0u32;
    let mut part_total = 0u64;
    let mut part_count = 0u32;
    let mut appr_total = 0u64;
    let mut appr_count = 0u32;

    let mut i = 0;
    while i < state.proposal_ids.len() {
        let id = state.proposal_ids.get(i).unwrap();
        if let Some(p) = state.proposals.get(id) {
            total = total.saturating_add(1);
            match p.status {
                ProposalStatus::Pending | ProposalStatus::Active => active = active.saturating_add(1),
                ProposalStatus::Succeeded => succeeded = succeeded.saturating_add(1),
                ProposalStatus::Failed => failed = failed.saturating_add(1),
                ProposalStatus::Executed => executed = executed.saturating_add(1),
                _ => {}
            }

            let all_votes = p
                .votes_for
                .saturating_add(p.votes_against)
                .saturating_add(p.votes_abstain);
            if total_supply > 0 {
                part_total = part_total
                    .saturating_add((all_votes.saturating_mul(BPS_DENOMINATOR) / total_supply) as u64);
                part_count = part_count.saturating_add(1);
            }

            let cast_votes = p.votes_for.saturating_add(p.votes_against);
            if cast_votes > 0 {
                appr_total = appr_total
                    .saturating_add((p.votes_for.saturating_mul(BPS_DENOMINATOR) / cast_votes) as u64);
                appr_count = appr_count.saturating_add(1);
            }
        }
        i += 1;
    }

    Ok(ProposalStatistics {
        total_proposals: total,
        active_proposals: active,
        succeeded_proposals: succeeded,
        failed_proposals: failed,
        executed_proposals: executed,
        avg_participation_rate: if part_count > 0 {
            (part_total / part_count as u64) as u32
        } else {
            0
        },
        avg_approval_rate: if appr_count > 0 {
            (appr_total / appr_count as u64) as u32
        } else {
            0
        },
    })
}

pub fn get_all_proposals(env: &Env) -> Vec<Proposal> {
    let state = get_proposals_state(env);
    let mut out = Vec::new(env);
    let mut i = 0;
    while i < state.proposal_ids.len() {
        let id = state.proposal_ids.get(i).unwrap();
        if let Some(p) = state.proposals.get(id) {
            out.push_back(p);
        }
        i += 1;
    }
    out
}

pub fn delegate_voting_power(
    env: &Env,
    delegator: Address,
    delegate: Address,
) -> Result<(), GovernanceError> {
    delegator.require_auth();
    if delegator == delegate {
        return Err(GovernanceError::InvalidProposal);
    }

    let mut state = get_delegation_state(env);
    if state
        .delegations
        .get(delegator.clone())
        .map(|d| d.active)
        .unwrap_or(false)
    {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    let power = get_staked_balance(env, &delegator);
    if power <= 0 {
        return Err(GovernanceError::NoVotingPower);
    }

    state.delegations.set(
        delegator.clone(),
        VoteDelegation {
            delegator: delegator.clone(),
            delegate,
            delegated_power: power,
            active: true,
        },
    );
    if !contains_address(&state.delegators, &delegator) {
        state.delegators.push_back(delegator);
    }
    put_delegation_state(env, &state);
    Ok(())
}

pub fn undelegate_voting_power(env: &Env, delegator: Address) -> Result<(), GovernanceError> {
    delegator.require_auth();
    let mut state = get_delegation_state(env);
    let mut d = state
        .delegations
        .get(delegator.clone())
        .ok_or(GovernanceError::CrossCommitteeRequestNotFound)?;
    d.active = false;
    state.delegations.set(delegator, d);
    put_delegation_state(env, &state);
    Ok(())
}

pub fn get_effective_voting_power(env: &Env, user: &Address) -> i128 {
    let state = get_delegation_state(env);
    let own = if state
        .delegations
        .get(user.clone())
        .map(|d| d.active)
        .unwrap_or(false)
    {
        0
    } else {
        get_staked_balance(env, user)
    };

    let mut delegated = 0i128;
    let mut i = 0;
    while i < state.delegators.len() {
        let delegator = state.delegators.get(i).unwrap();
        if let Some(d) = state.delegations.get(delegator) {
            if d.active && d.delegate == *user {
                delegated = delegated.saturating_add(d.delegated_power);
            }
        }
        i += 1;
    }

    own.saturating_add(delegated)
}

fn validate_proposal(env: &Env, p: &ProposalType) -> Result<(), GovernanceError> {
    match p {
        ProposalType::ParameterChange(parameter, current, proposed) => {
            if parameter.is_empty() {
                return Err(GovernanceError::InvalidProposal);
            }
            if *current > 0 {
                let delta = (*proposed - *current).abs();
                if checked_mul(delta, 2)? >= *current {
                    return Err(GovernanceError::InvalidProposal);
                }
            }
        }
        ProposalType::TreasurySpend(_recipient, amount, asset, _purpose) => {
            let treasury = get_treasury(env);
            let bal = treasury.assets.get(asset.clone()).unwrap_or(0);
            if *amount <= 0 || *amount > bal || amount.saturating_mul(10) > bal {
                return Err(GovernanceError::BudgetExceeded);
            }
        }
        ProposalType::ContractUpgrade(_name, hash) => {
            if hash.len() != 32 {
                return Err(GovernanceError::InvalidProposal);
            }
        }
        _ => {}
    }
    Ok(())
}

fn contains_address(list: &Vec<Address>, target: &Address) -> bool {
    let mut i = 0;
    while i < list.len() {
        if list.get(i).unwrap() == *target {
            return true;
        }
        i += 1;
    }
    false
}
