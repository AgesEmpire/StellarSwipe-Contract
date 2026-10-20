use soroban_sdk::{contracterror, contracttype, Address, Env, Map};

// ── Constants ────────────────────────────────────────────────────────────────

/// Minimum seconds between two executions of the same proposal.
pub const PROPOSAL_COOLDOWN_SECONDS: u64 = 300; // 5 minutes

/// Maximum executions allowed per proposal across its lifetime.
pub const MAX_EXECUTIONS_PER_PROPOSAL: u32 = 3;

/// Minimum seconds that must pass before a newly created proposal can be executed.
pub const PROPOSAL_TIMELOCK_SECONDS: u64 = 60;

// ── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceError {
    ProposalNotFound = 600,
    ProposalAlreadyExecuted = 601,
    ExecutionRateLimited = 602,
    ExecutionLimitReached = 603,
    TimelockNotExpired = 604,
    Unauthorized = 605,
}

// ── Types ────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalStatus {
    Pending,
    Active,
    Executed,
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub created_at: u64,
    pub status: ProposalStatus,
    pub execution_count: u32,
    pub last_executed_at: u64,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum GovKey {
    Proposals,
    ProposalCounter,
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn get_proposals(env: &Env) -> Map<u64, Proposal> {
    env.storage()
        .instance()
        .get(&GovKey::Proposals)
        .unwrap_or(Map::new(env))
}

fn save_proposals(env: &Env, map: &Map<u64, Proposal>) {
    env.storage().instance().set(&GovKey::Proposals, map);
}

fn next_proposal_id(env: &Env) -> u64 {
    let mut counter: u64 = env
        .storage()
        .instance()
        .get(&GovKey::ProposalCounter)
        .unwrap_or(0);
    counter = counter.saturating_add(1);
    env.storage()
        .instance()
        .set(&GovKey::ProposalCounter, &counter);
    counter
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Create a new proposal. Returns the proposal ID.
pub fn create_proposal(env: &Env, proposer: Address) -> u64 {
    let id = next_proposal_id(env);
    let proposal = Proposal {
        id,
        proposer,
        created_at: env.ledger().timestamp(),
        status: ProposalStatus::Pending,
        execution_count: 0,
        last_executed_at: 0,
    };
    let mut proposals = get_proposals(env);
    proposals.set(id, proposal);
    save_proposals(env, &proposals);
    id
}

/// Attempt to execute a proposal, enforcing all timing guardrails.
pub fn execute_proposal(
    env: &Env,
    proposal_id: u64,
    caller: &Address,
) -> Result<(), GovernanceError> {
    caller.require_auth();

    let mut proposals = get_proposals(env);
    let mut proposal = proposals
        .get(proposal_id)
        .ok_or(GovernanceError::ProposalNotFound)?;

    // Guard: already fully executed
    if proposal.status == ProposalStatus::Executed {
        return Err(GovernanceError::ProposalAlreadyExecuted);
    }

    let now = env.ledger().timestamp();

    // Guard: timelock — proposal must be old enough
    if now
        < proposal
            .created_at
            .saturating_add(PROPOSAL_TIMELOCK_SECONDS)
    {
        return Err(GovernanceError::TimelockNotExpired);
    }

    // Guard: cooldown between repeated executions
    if proposal.last_executed_at > 0
        && now
            < proposal
                .last_executed_at
                .saturating_add(PROPOSAL_COOLDOWN_SECONDS)
    {
        return Err(GovernanceError::ExecutionRateLimited);
    }

    // Guard: hard cap on total executions
    if proposal.execution_count >= MAX_EXECUTIONS_PER_PROPOSAL {
        return Err(GovernanceError::ExecutionLimitReached);
    }

    // Apply execution
    proposal.execution_count = proposal.execution_count.saturating_add(1);
    proposal.last_executed_at = now;
    proposal.status = if proposal.execution_count >= MAX_EXECUTIONS_PER_PROPOSAL {
        ProposalStatus::Executed
    } else {
        ProposalStatus::Active
    };

    proposals.set(proposal_id, proposal);
    save_proposals(env, &proposals);

    Ok(())
}

/// Fetch a proposal by ID.
pub fn get_proposal(env: &Env, proposal_id: u64) -> Option<Proposal> {
    get_proposals(env).get(proposal_id)
}
