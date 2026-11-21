//! Multisig approval workflow for signal_registry critical admin operations.

use soroban_sdk::{Address, Env, String, Vec};
use stellar_swipe_common::multisig::{
    self, ApprovalProposal, CriticalActionType, MultisigError, MultisigTimelockConfig,
    ProposalStatus,
};

use crate::admin::{
    self, get_multisig_signers, get_multisig_threshold, is_multisig_enabled, require_admin,
};
use crate::errors::AdminError;

/// Local storage for proposal payloads (keyed by proposal id).
#[soroban_sdk::contracttype]
#[derive(Clone)]
enum MultisigPayloadKey {
    Payload(u64),
}

/// Serialized action parameters executed after M-of-N approval and timelock.
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CriticalActionPayload {
    SetTradeFee(u32),
    SetMinStake(i128),
    SetRiskDefaults(u32, u32),
    PauseCategory(String, Option<u64>, String),
    UnpauseCategory(String),
    SetTierSignalLimits(u32, u32, u32),
    PauseFeeCollection,
    ResumeFeeCollection,
    SetGuardian(Address),
    ProposeAdminTransfer(Address),
}

impl From<MultisigError> for AdminError {
    fn from(err: MultisigError) -> Self {
        match err {
            MultisigError::Unauthorized => AdminError::Unauthorized,
            MultisigError::NotInitialized => AdminError::NotInitialized,
            MultisigError::InvalidParameter => AdminError::InvalidParameter,
            MultisigError::DuplicateSigner => AdminError::DuplicateSigner,
            MultisigError::InsufficientSignatures => AdminError::InsufficientSignatures,
            MultisigError::ProposalNotFound => AdminError::ProposalNotFound,
            MultisigError::AlreadyApproved => AdminError::AlreadyApproved,
            MultisigError::ProposalNotApproved => AdminError::ProposalNotApproved,
            MultisigError::TimelockNotElapsed => AdminError::TimelockNotElapsed,
            MultisigError::ProposalAlreadyExecuted => AdminError::ProposalAlreadyExecuted,
            MultisigError::ProposalCancelled => AdminError::ProposalCancelled,
            MultisigError::TooManyProposals => AdminError::TooManyProposals,
            MultisigError::RequiresMultisigApproval => AdminError::RequiresMultisigApproval,
            MultisigError::InvalidThreshold => AdminError::InvalidParameter,
        }
    }
}

/// Returns true when multisig is enabled and the caller must use the approval workflow.
pub fn critical_ops_require_approval(env: &Env) -> bool {
    is_multisig_enabled(env)
}

/// Block direct critical admin calls when multisig approval workflow is active.
pub fn require_direct_admin_or_not_multisig(env: &Env, caller: &Address) -> Result<(), AdminError> {
    if critical_ops_require_approval(env) {
        let _ = require_admin(env, caller)?;
        return Err(AdminError::RequiresMultisigApproval);
    }
    require_admin(env, caller)
}

fn signers_and_threshold(env: &Env) -> Result<(Vec<Address>, u32), AdminError> {
    if !is_multisig_enabled(env) {
        return Err(AdminError::NotInitialized);
    }
    Ok((get_multisig_signers(env), get_multisig_threshold(env)))
}

fn store_payload(env: &Env, proposal_id: u64, payload: &CriticalActionPayload) {
    env.storage()
        .instance()
        .set(&MultisigPayloadKey::Payload(proposal_id), payload);
}

fn load_payload(env: &Env, proposal_id: u64) -> Result<CriticalActionPayload, AdminError> {
    env.storage()
        .instance()
        .get(&MultisigPayloadKey::Payload(proposal_id))
        .ok_or(AdminError::InvalidParameter)
}

fn action_type_for_payload(payload: &CriticalActionPayload) -> CriticalActionType {
    match payload {
        CriticalActionPayload::SetTradeFee(_) => CriticalActionType::FeeChange,
        CriticalActionPayload::SetMinStake(_)
        | CriticalActionPayload::SetRiskDefaults(_, _)
        | CriticalActionPayload::SetTierSignalLimits(_, _, _) => {
            CriticalActionType::ParameterUpdate
        }
        CriticalActionPayload::PauseCategory(_, _, _) => CriticalActionType::Pause,
        CriticalActionPayload::UnpauseCategory(_) => CriticalActionType::Unpause,
        CriticalActionPayload::SetGuardian(_) => CriticalActionType::SetGuardian,
        CriticalActionPayload::ProposeAdminTransfer(_) => CriticalActionType::AdminTransfer,
        CriticalActionPayload::PauseFeeCollection | CriticalActionPayload::ResumeFeeCollection => {
            CriticalActionType::ConfigUpdate
        }
    }
}

/// Propose a critical action for M-of-N approval.
pub fn propose_critical_action(
    env: &Env,
    caller: &Address,
    payload: CriticalActionPayload,
) -> Result<u64, AdminError> {
    let (signers, threshold) = signers_and_threshold(env)?;
    let action_type = action_type_for_payload(&payload);
    let empty_payload = soroban_sdk::Bytes::new(env);
    let proposal_id =
        multisig::propose(env, caller, &signers, threshold, action_type, empty_payload)
            .map_err(AdminError::from)?;
    store_payload(env, proposal_id, &payload);
    Ok(proposal_id)
}

/// Approve a pending critical action proposal.
pub fn approve_proposal(
    env: &Env,
    caller: &Address,
    proposal_id: u64,
) -> Result<ProposalStatus, AdminError> {
    let (signers, threshold) = signers_and_threshold(env)?;
    multisig::approve(env, caller, &signers, threshold, proposal_id).map_err(Into::into)
}

/// Cancel a pending or timelocked proposal.
pub fn cancel_proposal(env: &Env, caller: &Address, proposal_id: u64) -> Result<(), AdminError> {
    let (signers, _) = signers_and_threshold(env)?;
    multisig::cancel(env, caller, &signers, proposal_id).map_err(Into::into)
}

/// Execute an approved proposal after timelock elapses.
pub fn execute_proposal(env: &Env, caller: &Address, proposal_id: u64) -> Result<(), AdminError> {
    let (signers, _) = signers_and_threshold(env)?;
    let proposal = multisig::prepare_execution(env, caller, &signers, proposal_id)?;
    dispatch_payload(env, caller, &proposal)
}

pub fn get_approval_proposal(env: &Env, proposal_id: u64) -> Result<ApprovalProposal, AdminError> {
    multisig::get_proposal(env, proposal_id).map_err(Into::into)
}

pub fn get_timelock_config(env: &Env) -> MultisigTimelockConfig {
    multisig::get_timelock_config(env)
}

pub fn set_timelock_config(
    env: &Env,
    caller: &Address,
    config: MultisigTimelockConfig,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();
    multisig::set_timelock_config(env, config, caller.clone()).map_err(Into::into)
}

fn dispatch_payload(
    env: &Env,
    caller: &Address,
    proposal: &ApprovalProposal,
) -> Result<(), AdminError> {
    let payload = load_payload(env, proposal.id)?;
    match payload {
        CriticalActionPayload::SetTradeFee(fee) => admin::set_trade_fee_direct(env, caller, fee),
        CriticalActionPayload::SetMinStake(amount) => {
            admin::set_min_stake_direct(env, caller, amount)
        }
        CriticalActionPayload::SetRiskDefaults(stop_loss, position_limit) => {
            admin::set_risk_defaults_direct(env, caller, stop_loss, position_limit)
        }
        CriticalActionPayload::PauseCategory(category, duration, reason) => {
            admin::pause_category_direct(env, caller, category, duration, reason)
        }
        CriticalActionPayload::UnpauseCategory(category) => {
            admin::unpause_category_direct(env, caller, category)
        }
        CriticalActionPayload::SetTierSignalLimits(bronze, silver, gold) => {
            admin::set_tier_signal_limits_direct(env, caller, bronze, silver, gold)
        }
        CriticalActionPayload::PauseFeeCollection => {
            admin::pause_fee_collection_direct(env, caller)
        }
        CriticalActionPayload::ResumeFeeCollection => {
            admin::resume_fee_collection_direct(env, caller)
        }
        CriticalActionPayload::SetGuardian(guardian) => {
            admin::set_guardian_direct(env, caller, guardian)
        }
        CriticalActionPayload::ProposeAdminTransfer(new_admin) => {
            admin::propose_admin_transfer_direct(env, caller, new_admin)
        }
    }
}
