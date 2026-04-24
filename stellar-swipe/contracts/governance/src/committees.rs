#![allow(clippy::too_many_arguments)]

use core::convert::TryFrom;

use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};
use stellar_swipe_common::Asset;

use crate::distribution::update_reward_config;
use crate::errors::GovernanceError;
use crate::treasury;
use crate::{
    checked_add, checked_div, checked_mul, checked_sub, get_distribution_state, get_staked_balance,
    get_treasury, put_treasury,
};

const APPROVAL_RATING_BPS: u32 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitteesState {
    pub committees: Map<u64, Committee>,
    pub committee_ids: Vec<u64>,
    pub elections: Map<u64, CommitteeElection>,
    pub cross_committee_requests: Map<u64, CrossCommitteeRequest>,
    pub next_committee_id: u64,
    pub next_decision_id: u64,
    pub next_cross_req_id: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Committee {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub members: Vec<Address>,
    pub chair: Address,
    pub max_members: u32,
    pub delegated_authorities: Vec<Authority>,
    pub formation_date: u64,
    pub term_end: Option<u64>,
    pub decisions: Vec<CommitteeDecision>,
    pub performance_metrics: PerformanceMetrics,
    pub active: bool,
    pub dissolved_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Authority {
    TreasurySpend(TreasurySpendAuthority),
    ParameterAdjustment(ParameterAdjustmentAuthority),
    GrantApproval(GrantApprovalAuthority),
    EmergencyAction(EmergencyActionAuthority),
    Veto(VetoAuthority),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasurySpendAuthority {
    pub max_amount: i128,
    pub category: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterAdjustmentAuthority {
    pub parameters: Vec<String>,
    pub max_change_pct: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantApprovalAuthority {
    pub max_grant_size: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyActionAuthority {
    pub action_types: Vec<String>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VetoAuthority {
    pub proposal_types: Vec<String>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitteeAction {
    TreasurySpend(TreasurySpendAction),
    RewardConfigUpdate(RewardConfigUpdateAction),
    GrantApproval(GrantApprovalAction),
    EmergencyAction(EmergencyActionPayload),
    Veto(VetoPayload),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasurySpendAction {
    pub recipient: Address,
    pub amount: i128,
    pub asset: Asset,
    pub category: String,
    pub purpose: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardConfigUpdateAction {
    pub reward_bps: u32,
    pub min_claim_threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantApprovalAction {
    pub recipient: Address,
    pub amount: i128,
    pub asset: Asset,
    pub category: String,
    pub purpose: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyActionPayload {
    pub action_type: String,
    pub details: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VetoPayload {
    pub proposal_type: String,
    pub reason: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitteeDecision {
    pub decision_id: u64,
    pub proposal: String,
    pub votes_for: u32,
    pub votes_against: u32,
    pub votes_abstain: u32,
    pub status: DecisionStatus,
    pub executed_at: Option<u64>,
    pub proposed_at: u64,
    pub action: CommitteeAction,
    pub votes: Map<Address, VoteType>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionStatus {
    Voting,
    Approved,
    Rejected,
    Executed,
    Overridden,
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
pub struct PerformanceMetrics {
    pub total_decisions: u32,
    pub decisions_executed: u32,
    pub avg_decision_time: u64,
    pub community_approval_rating: u32,
    pub overridden_decisions: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitteeElection {
    pub committee_id: u64,
    pub candidates: Vec<Address>,
    pub votes: Map<Address, Address>,
    pub election_start: u64,
    pub election_end: u64,
    pub positions_available: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitteeReport {
    pub committee_id: u64,
    pub name: String,
    pub members_count: u32,
    pub days_active: u64,
    pub total_decisions: u32,
    pub decisions_per_month: u32,
    pub execution_rate: u32,
    pub avg_decision_time: u64,
    pub community_approval: u32,
    pub overridden_count: u32,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossCommitteeRequest {
    pub id: u64,
    pub requesting_committee: u64,
    pub approving_committees: Vec<u64>,
    pub proposal: String,
    pub approvals: Map<u64, u64>,
    pub status: CrossCommitteeStatus,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrossCommitteeStatus {
    Pending,
    Approved,
}

pub fn empty_committees_state(env: &Env) -> CommitteesState {
    CommitteesState {
        committees: Map::new(env),
        committee_ids: Vec::new(env),
        elections: Map::new(env),
        cross_committee_requests: Map::new(env),
        next_committee_id: 1,
        next_decision_id: 1,
        next_cross_req_id: 1,
    }
}

pub fn list_committees(env: &Env, state: &CommitteesState) -> Vec<Committee> {
    let mut committees = Vec::new(env);
    let mut index = 0;
    while index < state.committee_ids.len() {
        let committee_id = state.committee_ids.get(index).unwrap();
        if let Some(committee) = state.committees.get(committee_id) {
            committees.push_back(committee);
        }
        index += 1;
    }
    committees
}

pub fn get_committee(
    state: &CommitteesState,
    committee_id: u64,
) -> Result<Committee, GovernanceError> {
    state
        .committees
        .get(committee_id)
        .ok_or(GovernanceError::CommitteeNotFound)
}

pub fn get_election(
    state: &CommitteesState,
    committee_id: u64,
) -> Result<CommitteeElection, GovernanceError> {
    state
        .elections
        .get(committee_id)
        .ok_or(GovernanceError::CommitteeElectionNotFound)
}

pub fn get_cross_committee_request(
    state: &CommitteesState,
    request_id: u64,
) -> Result<CrossCommitteeRequest, GovernanceError> {
    state
        .cross_committee_requests
        .get(request_id)
        .ok_or(GovernanceError::CrossCommitteeRequestNotFound)
}

pub fn create_committee(
    env: &Env,
    state: &mut CommitteesState,
    name: String,
    description: String,
    initial_members: Vec<Address>,
    chair: Address,
    max_members: u32,
    authorities: Vec<Authority>,
    term_duration_days: Option<u32>,
) -> Result<Committee, GovernanceError> {
    if name.is_empty() || description.is_empty() {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }
    if initial_members.is_empty()
        || initial_members.len() > max_members
        || !(3..=15).contains(&max_members)
    {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }
    if !contains_address(&initial_members, &chair) || !has_unique_members(&initial_members) {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }
    validate_authorities(&authorities)?;

    let now = env.ledger().timestamp();
    let term_end = match term_duration_days {
        Some(0) => return Err(GovernanceError::InvalidDuration),
        Some(days) => Some(now.saturating_add(days as u64 * 86_400)),
        None => None,
    };

    let committee = Committee {
        id: state.next_committee_id,
        name,
        description,
        members: initial_members,
        chair,
        max_members,
        delegated_authorities: authorities,
        formation_date: now,
        term_end,
        decisions: Vec::new(env),
        performance_metrics: PerformanceMetrics {
            total_decisions: 0,
            decisions_executed: 0,
            avg_decision_time: 0,
            community_approval_rating: 0,
            overridden_decisions: 0,
        },
        active: true,
        dissolved_at: None,
    };

    state.committees.set(committee.id, committee.clone());
    state.committee_ids.push_back(committee.id);
    state.next_committee_id = state.next_committee_id.saturating_add(1);
    Ok(committee)
}

pub fn propose_decision(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
    proposer: Address,
    proposal: String,
    action: CommitteeAction,
) -> Result<CommitteeDecision, GovernanceError> {
    if proposal.is_empty() {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    let mut committee = get_committee(state, committee_id)?;
    ensure_committee_active(&committee, env.ledger().timestamp())?;
    if !contains_address(&committee.members, &proposer) {
        return Err(GovernanceError::Unauthorized);
    }

    let decision = CommitteeDecision {
        decision_id: state.next_decision_id,
        proposal,
        votes_for: 0,
        votes_against: 0,
        votes_abstain: 0,
        status: DecisionStatus::Voting,
        executed_at: None,
        proposed_at: env.ledger().timestamp(),
        action,
        votes: Map::new(env),
    };

    committee.decisions.push_back(decision.clone());
    committee.performance_metrics.total_decisions = committee
        .performance_metrics
        .total_decisions
        .saturating_add(1);
    state.committees.set(committee_id, committee);
    state.next_decision_id = state.next_decision_id.saturating_add(1);

    Ok(decision)
}

pub fn vote_on_decision(
    state: &mut CommitteesState,
    committee_id: u64,
    decision_id: u64,
    voter: Address,
    vote: VoteType,
) -> Result<CommitteeDecision, GovernanceError> {
    let mut committee = get_committee(state, committee_id)?;
    if !contains_address(&committee.members, &voter) {
        return Err(GovernanceError::Unauthorized);
    }

    let decision_index = find_decision_index(&committee.decisions, decision_id)?;
    let mut decision = committee.decisions.get(decision_index).unwrap();
    if decision.status != DecisionStatus::Voting {
        return Err(GovernanceError::CommitteeDecisionNotOpen);
    }
    if decision.votes.contains_key(voter.clone()) {
        return Err(GovernanceError::AlreadyVoted);
    }

    decision.votes.set(voter, vote.clone());
    match vote {
        VoteType::For => decision.votes_for = decision.votes_for.saturating_add(1),
        VoteType::Against => decision.votes_against = decision.votes_against.saturating_add(1),
        VoteType::Abstain => decision.votes_abstain = decision.votes_abstain.saturating_add(1),
    }

    let majority = committee.members.len() / 2 + 1;
    let total_votes = decision
        .votes_for
        .saturating_add(decision.votes_against)
        .saturating_add(decision.votes_abstain);

    if decision.votes_for >= majority {
        decision.status = DecisionStatus::Approved;
    } else if decision.votes_against >= majority {
        decision.status = DecisionStatus::Rejected;
    } else if total_votes == committee.members.len() {
        decision.status = if decision.votes_for > decision.votes_against {
            DecisionStatus::Approved
        } else {
            DecisionStatus::Rejected
        };
    }

    committee.decisions.set(decision_index, decision.clone());
    state.committees.set(committee_id, committee);
    Ok(decision)
}

pub fn execute_decision(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
    decision_id: u64,
    executor: Address,
) -> Result<CommitteeDecision, GovernanceError> {
    let mut committee = get_committee(state, committee_id)?;
    ensure_committee_active(&committee, env.ledger().timestamp())?;
    if executor != committee.chair && !contains_address(&committee.members, &executor) {
        return Err(GovernanceError::Unauthorized);
    }

    let decision_index = find_decision_index(&committee.decisions, decision_id)?;
    let mut decision = committee.decisions.get(decision_index).unwrap();
    if decision.status != DecisionStatus::Approved {
        return Err(GovernanceError::CommitteeDecisionNotOpen);
    }

    verify_committee_authority(env, &committee, &decision.action)?;
    execute_decision_action(env, &decision.action, env.ledger().timestamp())?;

    decision.status = DecisionStatus::Executed;
    decision.executed_at = Some(env.ledger().timestamp());
    committee.decisions.set(decision_index, decision.clone());

    committee.performance_metrics.decisions_executed = committee
        .performance_metrics
        .decisions_executed
        .saturating_add(1);
    update_avg_decision_time(
        &mut committee.performance_metrics,
        &decision,
        env.ledger().timestamp(),
    )?;

    state.committees.set(committee_id, committee);
    Ok(decision)
}

pub fn start_election(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
    positions_available: u32,
    duration_days: u32,
) -> Result<CommitteeElection, GovernanceError> {
    let committee = get_committee(state, committee_id)?;
    ensure_committee_active(&committee, env.ledger().timestamp())?;
    if positions_available < 3 || positions_available > committee.max_members || duration_days == 0
    {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }
    if let Some(existing) = state.elections.get(committee_id) {
        if env.ledger().timestamp() < existing.election_end {
            return Err(GovernanceError::InvalidCommitteeConfig);
        }
    }

    let now = env.ledger().timestamp();
    let election = CommitteeElection {
        committee_id,
        candidates: Vec::new(env),
        votes: Map::new(env),
        election_start: now,
        election_end: now.saturating_add(duration_days as u64 * 86_400),
        positions_available,
    };

    state.elections.set(committee_id, election.clone());
    Ok(election)
}

pub fn nominate_for_committee(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
    nominee: Address,
    nominator: Address,
) -> Result<CommitteeElection, GovernanceError> {
    let mut election = get_election(state, committee_id)?;
    if env.ledger().timestamp() >= election.election_end {
        return Err(GovernanceError::CommitteeElectionNotActive);
    }
    if get_staked_balance(env, &nominator) <= 0 {
        return Err(GovernanceError::Unauthorized);
    }

    if !contains_address(&election.candidates, &nominee) {
        election.candidates.push_back(nominee);
    }
    state.elections.set(committee_id, election.clone());
    Ok(election)
}

pub fn vote_in_election(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
    voter: Address,
    candidate: Address,
) -> Result<CommitteeElection, GovernanceError> {
    let mut election = get_election(state, committee_id)?;
    let now = env.ledger().timestamp();
    if now < election.election_start || now >= election.election_end {
        return Err(GovernanceError::CommitteeElectionNotActive);
    }
    if !contains_address(&election.candidates, &candidate) {
        return Err(GovernanceError::NotCommitteeCandidate);
    }
    if election.votes.contains_key(voter.clone()) || get_staked_balance(env, &voter) <= 0 {
        return Err(GovernanceError::Unauthorized);
    }

    election.votes.set(voter, candidate);
    state.elections.set(committee_id, election.clone());
    Ok(election)
}

pub fn finalize_election(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
) -> Result<Vec<Address>, GovernanceError> {
    let election = get_election(state, committee_id)?;
    if env.ledger().timestamp() < election.election_end {
        return Err(GovernanceError::CommitteeElectionNotActive);
    }

    let winners = select_election_winners(env, &election)?;
    if winners.len() < 3 {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }

    let mut committee = get_committee(state, committee_id)?;
    committee.members = winners.clone();
    committee.chair = winners.get(0).unwrap();
    state.committees.set(committee_id, committee);
    state.elections.remove(committee_id);

    Ok(winners)
}

pub fn set_community_approval_rating(
    state: &mut CommitteesState,
    committee_id: u64,
    community_approval_rating: u32,
) -> Result<Committee, GovernanceError> {
    if community_approval_rating > APPROVAL_RATING_BPS {
        return Err(GovernanceError::InvalidApprovalRating);
    }
    let mut committee = get_committee(state, committee_id)?;
    committee.performance_metrics.community_approval_rating = community_approval_rating;
    state.committees.set(committee_id, committee.clone());
    Ok(committee)
}

pub fn report_activity(
    env: &Env,
    state: &CommitteesState,
    committee_id: u64,
) -> Result<CommitteeReport, GovernanceError> {
    let committee = get_committee(state, committee_id)?;
    let now = match committee.dissolved_at {
        Some(dissolved_at) => dissolved_at,
        None => env.ledger().timestamp(),
    };

    let days_active = now.saturating_sub(committee.formation_date) / 86_400;
    let decisions_per_month = if days_active > 0 {
        ((committee.performance_metrics.total_decisions as u64 * 30) / days_active) as u32
    } else {
        0
    };
    let execution_rate = if committee.performance_metrics.total_decisions > 0 {
        ((committee.performance_metrics.decisions_executed as i128 * APPROVAL_RATING_BPS as i128)
            / committee.performance_metrics.total_decisions as i128) as u32
    } else {
        0
    };

    Ok(CommitteeReport {
        committee_id,
        name: committee.name,
        members_count: committee.members.len(),
        days_active,
        total_decisions: committee.performance_metrics.total_decisions,
        decisions_per_month,
        execution_rate,
        avg_decision_time: committee.performance_metrics.avg_decision_time,
        community_approval: committee.performance_metrics.community_approval_rating,
        overridden_count: committee.performance_metrics.overridden_decisions,
        active: committee.active,
    })
}

pub fn override_decision(
    state: &mut CommitteesState,
    committee_id: u64,
    decision_id: u64,
) -> Result<CommitteeDecision, GovernanceError> {
    let mut committee = get_committee(state, committee_id)?;
    let decision_index = find_decision_index(&committee.decisions, decision_id)?;
    let mut decision = committee.decisions.get(decision_index).unwrap();
    if decision.status == DecisionStatus::Rejected || decision.status == DecisionStatus::Overridden
    {
        return Err(GovernanceError::CommitteeDecisionNotOpen);
    }

    decision.status = DecisionStatus::Overridden;
    committee.decisions.set(decision_index, decision.clone());
    committee.performance_metrics.overridden_decisions = committee
        .performance_metrics
        .overridden_decisions
        .saturating_add(1);
    state.committees.set(committee_id, committee);
    Ok(decision)
}

pub fn dissolve_committee(
    env: &Env,
    state: &mut CommitteesState,
    committee_id: u64,
) -> Result<Committee, GovernanceError> {
    let mut committee = get_committee(state, committee_id)?;
    if !committee.active {
        return Err(GovernanceError::CommitteeInactive);
    }

    committee.active = false;
    committee.dissolved_at = Some(env.ledger().timestamp());
    state.committees.set(committee_id, committee.clone());
    state.elections.remove(committee_id);
    Ok(committee)
}

pub fn request_cross_committee_approval(
    env: &Env,
    state: &mut CommitteesState,
    requesting_committee: u64,
    requester: Address,
    approving_committees: Vec<u64>,
    proposal: String,
) -> Result<CrossCommitteeRequest, GovernanceError> {
    if proposal.is_empty() || approving_committees.is_empty() {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }

    let committee = get_committee(state, requesting_committee)?;
    ensure_committee_active(&committee, env.ledger().timestamp())?;
    if !contains_address(&committee.members, &requester) {
        return Err(GovernanceError::Unauthorized);
    }

    let mut index = 0;
    while index < approving_committees.len() {
        let committee_id = approving_committees.get(index).unwrap();
        if committee_id == requesting_committee || !state.committees.contains_key(committee_id) {
            return Err(GovernanceError::InvalidCommitteeConfig);
        }
        index += 1;
    }

    let request = CrossCommitteeRequest {
        id: state.next_cross_req_id,
        requesting_committee,
        approving_committees,
        proposal,
        approvals: Map::new(env),
        status: CrossCommitteeStatus::Pending,
    };

    state
        .cross_committee_requests
        .set(request.id, request.clone());
    state.next_cross_req_id = state.next_cross_req_id.saturating_add(1);
    Ok(request)
}

pub fn approve_cross_committee_request(
    state: &mut CommitteesState,
    request_id: u64,
    approving_committee: u64,
    approver: Address,
    decision_id: u64,
) -> Result<CrossCommitteeRequest, GovernanceError> {
    let mut request = get_cross_committee_request(state, request_id)?;
    if request.status != CrossCommitteeStatus::Pending
        || !contains_u64(&request.approving_committees, approving_committee)
    {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }
    if request.approvals.contains_key(approving_committee) {
        return Err(GovernanceError::AlreadyVoted);
    }

    let committee = get_committee(state, approving_committee)?;
    if !contains_address(&committee.members, &approver) {
        return Err(GovernanceError::Unauthorized);
    }

    let decision_index = find_decision_index(&committee.decisions, decision_id)?;
    let decision = committee.decisions.get(decision_index).unwrap();
    if decision.status != DecisionStatus::Approved && decision.status != DecisionStatus::Executed {
        return Err(GovernanceError::CommitteeDecisionNotOpen);
    }

    request.approvals.set(approving_committee, decision_id);
    if request.approvals.len() == request.approving_committees.len() {
        request.status = CrossCommitteeStatus::Approved;
    }

    state
        .cross_committee_requests
        .set(request_id, request.clone());
    Ok(request)
}

fn validate_authorities(authorities: &Vec<Authority>) -> Result<(), GovernanceError> {
    if authorities.is_empty() {
        return Err(GovernanceError::InvalidCommitteeConfig);
    }

    let mut index = 0;
    while index < authorities.len() {
        match authorities.get(index).unwrap() {
            Authority::TreasurySpend(config) => {
                if config.max_amount <= 0 || config.category.is_empty() {
                    return Err(GovernanceError::InvalidCommitteeConfig);
                }
            }
            Authority::ParameterAdjustment(config) => {
                if config.parameters.is_empty() || config.max_change_pct == 0 {
                    return Err(GovernanceError::InvalidCommitteeConfig);
                }
            }
            Authority::GrantApproval(config) => {
                if config.max_grant_size <= 0 {
                    return Err(GovernanceError::InvalidCommitteeConfig);
                }
            }
            Authority::EmergencyAction(config) => {
                if config.action_types.is_empty() {
                    return Err(GovernanceError::InvalidCommitteeConfig);
                }
            }
            Authority::Veto(config) => {
                if config.proposal_types.is_empty() {
                    return Err(GovernanceError::InvalidCommitteeConfig);
                }
            }
        }
        index += 1;
    }

    Ok(())
}

fn ensure_committee_active(committee: &Committee, now: u64) -> Result<(), GovernanceError> {
    if !committee.active {
        return Err(GovernanceError::CommitteeInactive);
    }
    if let Some(term_end) = committee.term_end {
        if now >= term_end {
            return Err(GovernanceError::CommitteeTermEnded);
        }
    }
    Ok(())
}

fn find_decision_index(
    decisions: &Vec<CommitteeDecision>,
    decision_id: u64,
) -> Result<u32, GovernanceError> {
    let mut index = 0;
    while index < decisions.len() {
        if decisions.get(index).unwrap().decision_id == decision_id {
            return Ok(index);
        }
        index += 1;
    }
    Err(GovernanceError::CommitteeDecisionNotFound)
}

fn verify_committee_authority(
    env: &Env,
    committee: &Committee,
    action: &CommitteeAction,
) -> Result<(), GovernanceError> {
    let mut index = 0;
    while index < committee.delegated_authorities.len() {
        match (committee.delegated_authorities.get(index).unwrap(), action) {
            (Authority::TreasurySpend(config), CommitteeAction::TreasurySpend(action)) => {
                if action.amount <= config.max_amount && action.category == config.category {
                    return Ok(());
                }
            }
            (Authority::GrantApproval(config), CommitteeAction::GrantApproval(action)) => {
                if action.amount <= config.max_grant_size {
                    return Ok(());
                }
            }
            (
                Authority::ParameterAdjustment(config),
                CommitteeAction::RewardConfigUpdate(action),
            ) => {
                validate_reward_config_authority(
                    env,
                    &config.parameters,
                    config.max_change_pct,
                    action.reward_bps,
                    action.min_claim_threshold,
                )?;
                return Ok(());
            }
            (Authority::EmergencyAction(config), CommitteeAction::EmergencyAction(action)) => {
                if contains_string(&config.action_types, &action.action_type) {
                    return Ok(());
                }
            }
            (Authority::Veto(config), CommitteeAction::Veto(action)) => {
                if contains_string(&config.proposal_types, &action.proposal_type) {
                    return Ok(());
                }
            }
            _ => {}
        }
        index += 1;
    }

    Err(GovernanceError::NoCommitteeAuthority)
}

fn validate_reward_config_authority(
    env: &Env,
    parameters: &Vec<String>,
    max_change_pct: u32,
    reward_bps: u32,
    min_claim_threshold: i128,
) -> Result<(), GovernanceError> {
    let state = get_distribution_state(env)?;

    let reward_key = String::from_str(env, "liquidity_reward_bps");
    let threshold_key = String::from_str(env, "min_claim_threshold");
    let mut changed_any = false;

    if reward_bps != state.liquidity_reward_bps {
        changed_any = true;
        if !contains_string(parameters, &reward_key)
            || pct_change(state.liquidity_reward_bps as i128, reward_bps as i128)? > max_change_pct
        {
            return Err(GovernanceError::NoCommitteeAuthority);
        }
    }

    if min_claim_threshold != state.min_claim_threshold {
        changed_any = true;
        if !contains_string(parameters, &threshold_key)
            || pct_change(state.min_claim_threshold, min_claim_threshold)? > max_change_pct
        {
            return Err(GovernanceError::NoCommitteeAuthority);
        }
    }

    if !changed_any {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    Ok(())
}

fn execute_decision_action(
    env: &Env,
    action: &CommitteeAction,
    executed_at: u64,
) -> Result<(), GovernanceError> {
    match action {
        CommitteeAction::TreasurySpend(action) => {
            let mut treasury_state = get_treasury(env);
            treasury::execute_spend(
                &mut treasury_state,
                action.recipient.clone(),
                action.amount,
                action.asset.clone(),
                action.category.clone(),
                action.purpose.clone(),
                None,
                executed_at,
            )?;
            put_treasury(env, &treasury_state);
            Ok(())
        }
        CommitteeAction::GrantApproval(action) => {
            let mut treasury_state = get_treasury(env);
            treasury::execute_spend(
                &mut treasury_state,
                action.recipient.clone(),
                action.amount,
                action.asset.clone(),
                action.category.clone(),
                action.purpose.clone(),
                None,
                executed_at,
            )?;
            put_treasury(env, &treasury_state);
            Ok(())
        }
        CommitteeAction::RewardConfigUpdate(action) => {
            update_reward_config(env, action.reward_bps, action.min_claim_threshold)?;
            Ok(())
        }
        CommitteeAction::EmergencyAction(action) => {
            if action.details.is_empty() {
                Err(GovernanceError::InvalidCommitteeAction)
            } else {
                Ok(())
            }
        }
        CommitteeAction::Veto(action) => {
            if action.reason.is_empty() {
                Err(GovernanceError::InvalidCommitteeAction)
            } else {
                Ok(())
            }
        }
    }
}

fn update_avg_decision_time(
    metrics: &mut PerformanceMetrics,
    decision: &CommitteeDecision,
    executed_at: u64,
) -> Result<(), GovernanceError> {
    let execution_time = executed_at.saturating_sub(decision.proposed_at);
    if metrics.decisions_executed == 0 {
        metrics.avg_decision_time = execution_time;
        return Ok(());
    }

    let prior_count = metrics.decisions_executed.saturating_sub(1) as i128;
    let accumulated = checked_mul(metrics.avg_decision_time as i128, prior_count)?;
    let updated_total = checked_add(accumulated, execution_time as i128)?;
    metrics.avg_decision_time = u64::try_from(checked_div(
        updated_total,
        metrics.decisions_executed as i128,
    )?)
    .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    Ok(())
}

fn select_election_winners(
    env: &Env,
    election: &CommitteeElection,
) -> Result<Vec<Address>, GovernanceError> {
    let candidate_votes = tally_election_votes(env, election)?;
    let mut winners = Vec::new(env);
    let mut selected = Map::<Address, bool>::new(env);

    while winners.len() < election.positions_available {
        let mut best_candidate: Option<Address> = None;
        let mut best_votes = i128::MIN;

        let mut index = 0;
        while index < election.candidates.len() {
            let candidate = election.candidates.get(index).unwrap();
            if selected.contains_key(candidate.clone()) {
                index += 1;
                continue;
            }

            let votes = candidate_votes.get(candidate.clone()).unwrap_or(0);
            if votes > best_votes {
                best_votes = votes;
                best_candidate = Some(candidate);
            }
            index += 1;
        }

        match best_candidate {
            Some(candidate) => {
                selected.set(candidate.clone(), true);
                winners.push_back(candidate);
            }
            None => break,
        }
    }

    Ok(winners)
}

fn tally_election_votes(
    env: &Env,
    election: &CommitteeElection,
) -> Result<Map<Address, i128>, GovernanceError> {
    let mut candidate_votes = Map::<Address, i128>::new(env);
    let voters = election.votes.keys();

    let mut index = 0;
    while index < voters.len() {
        let voter = voters.get(index).unwrap();
        let candidate = election.votes.get(voter.clone()).unwrap();
        let voting_power = get_staked_balance(env, &voter);
        let current = candidate_votes.get(candidate.clone()).unwrap_or(0);
        candidate_votes.set(candidate, checked_add(current, voting_power)?);
        index += 1;
    }

    Ok(candidate_votes)
}

fn pct_change(current: i128, proposed: i128) -> Result<u32, GovernanceError> {
    if current <= 0 {
        return Err(GovernanceError::InvalidCommitteeAction);
    }

    let diff = if proposed >= current {
        checked_sub(proposed, current)?
    } else {
        checked_sub(current, proposed)?
    };
    let pct = checked_div(checked_mul(diff, 100)?, current)?;
    u32::try_from(pct).map_err(|_| GovernanceError::ArithmeticOverflow)
}

fn contains_address(addresses: &Vec<Address>, target: &Address) -> bool {
    let mut index = 0;
    while index < addresses.len() {
        if addresses.get(index).unwrap() == *target {
            return true;
        }
        index += 1;
    }
    false
}

fn contains_string(strings: &Vec<String>, target: &String) -> bool {
    let mut index = 0;
    while index < strings.len() {
        if strings.get(index).unwrap() == *target {
            return true;
        }
        index += 1;
    }
    false
}

fn contains_u64(values: &Vec<u64>, target: u64) -> bool {
    let mut index = 0;
    while index < values.len() {
        if values.get(index).unwrap() == target {
            return true;
        }
        index += 1;
    }
    false
}

fn has_unique_members(members: &Vec<Address>) -> bool {
    let mut index = 0;
    while index < members.len() {
        let current = members.get(index).unwrap();
        let mut inner = index + 1;
        while inner < members.len() {
            if members.get(inner).unwrap() == current {
                return false;
            }
            inner += 1;
        }
        index += 1;
    }
    true
}
