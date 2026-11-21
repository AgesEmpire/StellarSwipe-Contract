//! M-of-N approval workflow for critical contract operations.
//!
//! Signers propose actions, collect approvals until the threshold is met,
//! wait for a configurable timelock, then execute.

use soroban_sdk::{contracttype, Address, Bytes, Env, Map, Symbol, Vec};

/// Default timelock delays (seconds).
pub const DEFAULT_FEE_CHANGE_DELAY: u64 = 3 * 86_400;
pub const DEFAULT_PARAMETER_DELAY: u64 = 3 * 86_400;
pub const DEFAULT_PAUSE_DELAY: u64 = 0;
pub const DEFAULT_UNPAUSE_DELAY: u64 = 86_400;
pub const DEFAULT_GUARDIAN_DELAY: u64 = 2 * 86_400;
pub const DEFAULT_ADMIN_TRANSFER_DELAY: u64 = 2 * 86_400;
pub const DEFAULT_CONFIG_DELAY: u64 = 2 * 86_400;

/// Maximum signers and active proposals to bound storage growth.
pub const MAX_SIGNERS: u32 = 20;
pub const MAX_ACTIVE_PROPOSALS: u32 = 50;

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum MultisigError {
    Unauthorized = 1,
    NotInitialized = 2,
    InvalidParameter = 3,
    DuplicateSigner = 4,
    InsufficientSignatures = 5,
    ProposalNotFound = 6,
    AlreadyApproved = 7,
    ProposalNotApproved = 8,
    TimelockNotElapsed = 9,
    ProposalAlreadyExecuted = 10,
    ProposalCancelled = 11,
    TooManyProposals = 12,
    RequiresMultisigApproval = 13,
    InvalidThreshold = 14,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CriticalActionType {
    FeeChange,
    ParameterUpdate,
    Pause,
    Unpause,
    SetGuardian,
    AdminTransfer,
    ConfigUpdate,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Pending,
    Approved,
    Executed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ApprovalProposal {
    pub id: u64,
    pub proposer: Address,
    pub action_type: CriticalActionType,
    pub payload: Bytes,
    pub approvals: Vec<Address>,
    pub status: ProposalStatus,
    pub created_at: u64,
    pub approved_at: u64,
    pub executable_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MultisigTimelockConfig {
    pub fee_change_delay: u64,
    pub parameter_delay: u64,
    pub pause_delay: u64,
    pub unpause_delay: u64,
    pub guardian_delay: u64,
    pub admin_transfer_delay: u64,
    pub config_delay: u64,
}

impl MultisigTimelockConfig {
    pub fn default_config() -> Self {
        Self {
            fee_change_delay: DEFAULT_FEE_CHANGE_DELAY,
            parameter_delay: DEFAULT_PARAMETER_DELAY,
            pause_delay: DEFAULT_PAUSE_DELAY,
            unpause_delay: DEFAULT_UNPAUSE_DELAY,
            guardian_delay: DEFAULT_GUARDIAN_DELAY,
            admin_transfer_delay: DEFAULT_ADMIN_TRANSFER_DELAY,
            config_delay: DEFAULT_CONFIG_DELAY,
        }
    }

    pub fn delay_for(&self, action: &CriticalActionType) -> u64 {
        match action {
            CriticalActionType::FeeChange => self.fee_change_delay,
            CriticalActionType::ParameterUpdate => self.parameter_delay,
            CriticalActionType::Pause => self.pause_delay,
            CriticalActionType::Unpause => self.unpause_delay,
            CriticalActionType::SetGuardian => self.guardian_delay,
            CriticalActionType::AdminTransfer => self.admin_transfer_delay,
            CriticalActionType::ConfigUpdate => self.config_delay,
        }
    }
}

#[contracttype]
#[derive(Clone)]
pub enum MultisigStorageKey {
    NextProposalId,
    ActiveProposalCount,
    Proposal(u64),
    TimelockConfig,
}

// ── Event helpers ────────────────────────────────────────────────────────────

pub fn emit_proposal_created(
    env: &Env,
    proposal_id: u64,
    proposer: Address,
    action_type: CriticalActionType,
) {
    let topics = (Symbol::new(env, "multisig_proposal_created"),);
    env.events()
        .publish(topics, (proposal_id, proposer, action_type));
}

pub fn emit_approval_recorded(
    env: &Env,
    proposal_id: u64,
    approver: Address,
    approval_count: u32,
    threshold: u32,
) {
    let topics = (Symbol::new(env, "multisig_approval_recorded"),);
    env.events()
        .publish(topics, (proposal_id, approver, approval_count, threshold));
}

pub fn emit_proposal_approved(env: &Env, proposal_id: u64, executable_at: u64) {
    let topics = (Symbol::new(env, "multisig_proposal_approved"),);
    env.events().publish(topics, (proposal_id, executable_at));
}

pub fn emit_proposal_executed(env: &Env, proposal_id: u64, executor: Address) {
    let topics = (Symbol::new(env, "multisig_proposal_executed"),);
    env.events().publish(topics, (proposal_id, executor));
}

pub fn emit_proposal_cancelled(env: &Env, proposal_id: u64, cancelled_by: Address) {
    let topics = (Symbol::new(env, "multisig_proposal_cancelled"),);
    env.events().publish(topics, (proposal_id, cancelled_by));
}

pub fn emit_timelock_config_updated(env: &Env, updated_by: Address) {
    let topics = (Symbol::new(env, "multisig_timelock_updated"),);
    env.events().publish(topics, updated_by);
}

// ── Validation ─────────────────────────────────────────────────────────────────

pub fn validate_signer_config(signers: &Vec<Address>, threshold: u32) -> Result<(), MultisigError> {
    if threshold == 0 || threshold > signers.len() {
        return Err(MultisigError::InvalidThreshold);
    }
    if signers.len() > MAX_SIGNERS {
        return Err(MultisigError::InvalidParameter);
    }
    for i in 0..signers.len() {
        for j in (i + 1)..signers.len() {
            if signers.get(i).unwrap() == signers.get(j).unwrap() {
                return Err(MultisigError::DuplicateSigner);
            }
        }
    }
    Ok(())
}

pub fn is_signer(signers: &Vec<Address>, address: &Address) -> bool {
    for i in 0..signers.len() {
        if &signers.get(i).unwrap() == address {
            return true;
        }
    }
    false
}

fn require_signer(signers: &Vec<Address>, caller: &Address) -> Result<(), MultisigError> {
    if !is_signer(signers, caller) {
        return Err(MultisigError::Unauthorized);
    }
    Ok(())
}

fn has_approved(proposal: &ApprovalProposal, signer: &Address) -> bool {
    for i in 0..proposal.approvals.len() {
        if &proposal.approvals.get(i).unwrap() == signer {
            return true;
        }
    }
    false
}

// ── Storage helpers ───────────────────────────────────────────────────────────

pub fn get_timelock_config(env: &Env) -> MultisigTimelockConfig {
    env.storage()
        .instance()
        .get(&MultisigStorageKey::TimelockConfig)
        .unwrap_or_else(MultisigTimelockConfig::default_config)
}

pub fn set_timelock_config(
    env: &Env,
    config: MultisigTimelockConfig,
    updated_by: Address,
) -> Result<(), MultisigError> {
    env.storage()
        .instance()
        .set(&MultisigStorageKey::TimelockConfig, &config);
    emit_timelock_config_updated(env, updated_by);
    Ok(())
}

fn next_proposal_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&MultisigStorageKey::NextProposalId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&MultisigStorageKey::NextProposalId, &(id + 1));
    id
}

fn active_proposal_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&MultisigStorageKey::ActiveProposalCount)
        .unwrap_or(0)
}

fn increment_active_proposals(env: &Env) {
    let count = active_proposal_count(env);
    env.storage()
        .instance()
        .set(&MultisigStorageKey::ActiveProposalCount, &(count + 1));
}

fn decrement_active_proposals(env: &Env) {
    let count = active_proposal_count(env);
    if count > 0 {
        env.storage()
            .instance()
            .set(&MultisigStorageKey::ActiveProposalCount, &(count - 1));
    }
}

pub fn store_proposal(env: &Env, proposal: &ApprovalProposal) {
    env.storage()
        .instance()
        .set(&MultisigStorageKey::Proposal(proposal.id), proposal);
}

pub fn get_proposal(env: &Env, proposal_id: u64) -> Result<ApprovalProposal, MultisigError> {
    env.storage()
        .instance()
        .get(&MultisigStorageKey::Proposal(proposal_id))
        .ok_or(MultisigError::ProposalNotFound)
}

// ── Core workflow ─────────────────────────────────────────────────────────────

/// Create a new approval proposal. Proposer's approval is counted automatically.
pub fn propose(
    env: &Env,
    proposer: &Address,
    signers: &Vec<Address>,
    threshold: u32,
    action_type: CriticalActionType,
    payload: Bytes,
) -> Result<u64, MultisigError> {
    proposer.require_auth();
    require_signer(signers, proposer)?;
    validate_signer_config(signers, threshold)?;

    if active_proposal_count(env) >= MAX_ACTIVE_PROPOSALS {
        return Err(MultisigError::TooManyProposals);
    }

    let config = get_timelock_config(env);
    let delay = config.delay_for(&action_type);
    let now = env.ledger().timestamp();
    let id = next_proposal_id(env);

    let mut approvals = Vec::new(env);
    approvals.push_back(proposer.clone());

    let mut status = ProposalStatus::Pending;
    let mut approved_at = 0u64;
    let mut executable_at = 0u64;

    if approvals.len() >= threshold {
        status = ProposalStatus::Approved;
        approved_at = now;
        executable_at = now.saturating_add(delay);
        emit_proposal_approved(env, id, executable_at);
    }

    let proposal = ApprovalProposal {
        id,
        proposer: proposer.clone(),
        action_type: action_type.clone(),
        payload,
        approvals,
        status,
        created_at: now,
        approved_at,
        executable_at,
    };

    store_proposal(env, &proposal);
    increment_active_proposals(env);
    emit_proposal_created(env, id, proposer.clone(), action_type);

    let approval_count = proposal.approvals.len();
    emit_approval_recorded(env, id, proposer.clone(), approval_count, threshold);

    Ok(id)
}

/// Record an approval from a signer. Transitions to Approved when threshold is met.
pub fn approve(
    env: &Env,
    caller: &Address,
    signers: &Vec<Address>,
    threshold: u32,
    proposal_id: u64,
) -> Result<ProposalStatus, MultisigError> {
    caller.require_auth();
    require_signer(signers, caller)?;

    let mut proposal = get_proposal(env, proposal_id)?;

    match proposal.status {
        ProposalStatus::Executed => return Err(MultisigError::ProposalAlreadyExecuted),
        ProposalStatus::Cancelled => return Err(MultisigError::ProposalCancelled),
        ProposalStatus::Approved => return Ok(ProposalStatus::Approved),
        ProposalStatus::Pending => {}
    }

    if has_approved(&proposal, caller) {
        return Err(MultisigError::AlreadyApproved);
    }

    proposal.approvals.push_back(caller.clone());
    let approval_count = proposal.approvals.len();
    emit_approval_recorded(env, proposal_id, caller.clone(), approval_count, threshold);

    if approval_count >= threshold {
        let config = get_timelock_config(env);
        let delay = config.delay_for(&proposal.action_type);
        let now = env.ledger().timestamp();
        proposal.status = ProposalStatus::Approved;
        proposal.approved_at = now;
        proposal.executable_at = now.saturating_add(delay);
        emit_proposal_approved(env, proposal_id, proposal.executable_at);
    }

    store_proposal(env, &proposal);
    Ok(proposal.status.clone())
}

/// Cancel a pending or approved (not yet executed) proposal.
pub fn cancel(
    env: &Env,
    caller: &Address,
    signers: &Vec<Address>,
    proposal_id: u64,
) -> Result<(), MultisigError> {
    caller.require_auth();
    require_signer(signers, caller)?;

    let mut proposal = get_proposal(env, proposal_id)?;

    match proposal.status {
        ProposalStatus::Executed => return Err(MultisigError::ProposalAlreadyExecuted),
        ProposalStatus::Cancelled => return Err(MultisigError::ProposalCancelled),
        ProposalStatus::Pending | ProposalStatus::Approved => {}
    }

    proposal.status = ProposalStatus::Cancelled;
    store_proposal(env, &proposal);
    decrement_active_proposals(env);
    emit_proposal_cancelled(env, proposal_id, caller.clone());
    Ok(())
}

/// Mark a proposal ready for execution after timelock. Returns the proposal payload.
pub fn prepare_execution(
    env: &Env,
    executor: &Address,
    signers: &Vec<Address>,
    proposal_id: u64,
) -> Result<ApprovalProposal, MultisigError> {
    executor.require_auth();
    require_signer(signers, executor)?;

    let mut proposal = get_proposal(env, proposal_id)?;

    if proposal.status == ProposalStatus::Executed {
        return Err(MultisigError::ProposalAlreadyExecuted);
    }
    if proposal.status == ProposalStatus::Cancelled {
        return Err(MultisigError::ProposalCancelled);
    }
    if proposal.status != ProposalStatus::Approved {
        return Err(MultisigError::ProposalNotApproved);
    }

    let now = env.ledger().timestamp();
    if now < proposal.executable_at {
        return Err(MultisigError::TimelockNotElapsed);
    }

    proposal.status = ProposalStatus::Executed;
    store_proposal(env, &proposal);
    decrement_active_proposals(env);
    emit_proposal_executed(env, proposal_id, executor.clone());

    Ok(proposal)
}

/// Returns summary counts for monitoring.
pub fn get_multisig_stats(env: &Env) -> Map<Symbol, u64> {
    let mut stats = Map::new(env);
    stats.set(
        Symbol::new(env, "active_proposals"),
        active_proposal_count(env) as u64,
    );
    stats.set(
        Symbol::new(env, "next_proposal_id"),
        env.storage()
            .instance()
            .get(&MultisigStorageKey::NextProposalId)
            .unwrap_or(1u64),
    );
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn signers(env: &Env) -> Vec<Address> {
        let a = Address::generate(env);
        let b = Address::generate(env);
        let c = Address::generate(env);
        soroban_sdk::vec![env, a, b, c]
    }

    #[test]
    fn test_validate_signer_config_rejects_invalid_threshold() {
        let env = Env::default();
        let signers = signers(&env);
        assert_eq!(
            validate_signer_config(&signers, 4),
            Err(MultisigError::InvalidThreshold)
        );
    }

    #[test]
    fn test_validate_signer_config_rejects_duplicates() {
        let env = Env::default();
        let a = Address::generate(&env);
        let signers = soroban_sdk::vec![&env, a.clone(), a];
        assert_eq!(
            validate_signer_config(&signers, 1),
            Err(MultisigError::DuplicateSigner)
        );
    }

    #[test]
    fn test_timelock_config_delay_for_action_types() {
        let config = MultisigTimelockConfig::default_config();
        assert_eq!(
            config.delay_for(&CriticalActionType::FeeChange),
            DEFAULT_FEE_CHANGE_DELAY
        );
        assert_eq!(
            config.delay_for(&CriticalActionType::Pause),
            DEFAULT_PAUSE_DELAY
        );
    }
}
