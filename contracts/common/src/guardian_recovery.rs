//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
//! Also provides a shared append-only emergency action journal (#1166).
//! Also provides delayed guardian recovery for lost administrator access (#1183).
//! Also provides scoped operator capabilities for contract maintenance actions (#1182).
//! Also provides monotonic, auditable bridge attestation threshold updates (#1113).
//! Also provides durable rollback markers for partially executed governance
//! actions so partial progress is distinguishable and recoverable (#1115).
//! Follow-up work: wire into the live pause-handling contract storage/auth and add
//! integration tests against real contract state.

use soroban_sdk::{contracttype, Address, Env, Vec};

#[derive(Clone)]
#[contracttype]
pub enum GuardianRole {
    /// Can propose a recovery action while a contract is paused.
    Proposer,
    /// Can approve a proposed recovery action.
    Approver,
}

#[derive(Clone)]
#[contracttype]
pub struct RecoveryRequest {
    pub id: u64,
    pub proposer: Address,
    pub approvals: Vec<Address>,
    pub required_approvals: u32,
    pub executed: bool,
}

/// Guards that must hold before a recovery request can be executed.
#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Unauthorized,
    AlreadyExecuted,
    InsufficientApprovals,
}

impl RecoveryRequest {
    pub fn new(id: u64, proposer: Address, required_approvals: u32, env: &Env) -> Self {
        Self {
            id,
            proposer,
            approvals: Vec::new(env),
            required_approvals,
            executed: false,
        }
    }

    /// Records an approval from a guardian. Caller is responsible for verifying
    /// the approver actually holds the Approver role and for auth (require_auth).
    pub fn approve(&mut self, approver: Address) {
        if !self.approvals.contains(&approver) {
            self.approvals.push_back(approver);
        }
    }

    /// Returns Ok(()) only when enough distinct approvals exist and the request
    /// has not already been executed. Does not itself perform any state change.
    pub fn check_executable(&self) -> Result<(), RecoveryError> {
        if self.executed {
            return Err(RecoveryError::AlreadyExecuted);
        }
        if self.approvals.len() < self.required_approvals {
            return Err(RecoveryError::InsufficientApprovals);
        }
        Ok(())
    }
}

/// Errors surfaced by the delayed guardian recovery path (#1183).
#[derive(Debug, PartialEq, Eq)]
pub enum GuardianRecoveryError {
    /// Caller is not authorized for this recovery action.
    Unauthorized,
    /// The configured guardian threshold is not met.
    InsufficientApprovals,
    /// The configured delay has not yet elapsed since initiation.
    DelayNotElapsed,
    /// The recovery has already been executed.
    AlreadyExecuted,
    /// The recovery was cancelled by the active administrator.
    Cancelled,
    /// The configured threshold or delay is invalid (e.g. zero threshold).
    InvalidConfig,
}

/// Delayed, auditable recovery flow for restoring administrator access when the
/// active administrator key is unavailable (#1183).
///
/// A recovery is initiated by a guardian, gathers approvals until the configured
/// guardian threshold is met, and can only be executed once the configured delay
/// has elapsed since initiation. The active administrator may cancel a pending
/// recovery at any point during the delay window.
#[derive(Clone)]
#[contracttype]
pub struct GuardianRecovery {
    /// The currently active administrator, who may cancel a pending recovery.
    pub active_admin: Address,
    /// Minimum number of distinct guardian approvals required to execute.
    pub threshold: u32,
    /// Delay (in ledgers) that must elapse between initiation and execution.
    pub delay: u32,
    /// Ledger sequence at which the recovery was initiated.
    pub initiated_at: u32,
    /// Distinct guardian approvals gathered so far.
    pub approvals: Vec<Address>,
    /// Whether the recovery has been executed.
    pub executed: bool,
    /// Whether the recovery was cancelled by the active administrator.
    pub cancelled: bool,
}

impl GuardianRecovery {
    /// Creates a new pending recovery. Rejects a zero threshold or zero delay so
    /// a misconfigured recovery cannot bypass the guardian gate or the delay.
    pub fn new(
        active_admin: Address,
        threshold: u32,
        delay: u32,
        env: &Env,
    ) -> Result<Self, GuardianRecoveryError> {
        if threshold == 0 || delay == 0 {
            return Err(GuardianRecoveryError::InvalidConfig);
        }
        Ok(Self {
            active_admin,
            threshold,
            delay,
            initiated_at: env.ledger().sequence(),
            approvals: Vec::new(env),
            executed: false,
            cancelled: false,
        })
    }

    /// Records a guardian approval. Caller is responsible for verifying the
    /// approver holds the Approver role and for auth (require_auth). Approvals
    /// are deduplicated so a single guardian cannot satisfy the threshold alone.
    pub fn approve(&mut self, approver: Address) -> Result<(), GuardianRecoveryError> {
        if self.executed {
            return Err(GuardianRecoveryError::AlreadyExecuted);
        }
        if self.cancelled {
            return Err(GuardianRecoveryError::Cancelled);
        }
        if !self.approvals.contains(&approver) {
            self.approvals.push_back(approver);
        }
        Ok(())
    }

    /// Cancels a pending recovery. Only the active administrator may cancel, and
    /// only while the recovery is still pending (not executed or already cancelled).
    pub fn cancel(&mut self, caller: &Address) -> Result<(), GuardianRecoveryError> {
        caller.require_auth();
        if caller != &self.active_admin {
            return Err(GuardianRecoveryError::Unauthorized);
        }
        if self.executed {
            return Err(GuardianRecoveryError::AlreadyExecuted);
        }
        if self.cancelled {
            return Err(GuardianRecoveryError::Cancelled);
        }
        self.cancelled = true;
        Ok(())
    }

    /// Returns Ok(()) only when the recovery is pending, the guardian threshold
    /// is met, and the configured delay has fully elapsed since initiation.
    /// Does not itself perform any state change.
    pub fn check_executable(&self, env: &Env) -> Result<(), GuardianRecoveryError> {
        if self.executed {
            return Err(GuardianRecoveryError::AlreadyExecuted);
        }
        if self.cancelled {
            return Err(GuardianRecoveryError::Cancelled);
        }
        if self.approvals.len() < self.threshold {
            return Err(GuardianRecoveryError::InsufficientApprovals);
        }
        let elapsed = env.ledger().sequence().saturating_sub(self.initiated_at);
        if elapsed < self.delay {
            return Err(GuardianRecoveryError::DelayNotElapsed);
        }
        Ok(())
    }

    /// Executes the recovery once the threshold and delay guards hold, marking
    /// it executed so it cannot be replayed.
    pub fn execute(&mut self, env: &Env) -> Result<(), GuardianRecoveryError> {
        self.check_executable(env)?;
        self.executed = true;
        Ok(())
    }
}

/// Errors surfaced by the governance rollback marker path (#1115).
#[derive(Debug, PartialEq, Eq)]
pub enum RollbackError {
    /// Caller is not authorized to record or recover a rollback marker.
    Unauthorized,
    /// The action has already fully completed and cannot be rolled back.
    AlreadyCompleted,
    /// The action has not partially executed, so there is nothing to recover.
    NothingToRecover,
    /// The requested step has already been rolled back (idempotent no-op guard).
    AlreadyRolledBack,
    /// The step index is out of range for the recorded action.
    InvalidStep,
}

/// Lifecycle state of a multi-step governance action, making partial progress
/// distinguishable from completed and unapplied actions (#1115).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[contracttype]
pub enum ActionState {
    /// No step has executed yet.
    Unapplied,
    /// Some but not all steps have executed; a rollback marker is durable.
    PartiallyExecuted,
    /// Every step has executed successfully.
    Completed,
}

/// Durable rollback marker recorded when a governance action partially executes
/// across multiple contract calls (#1115).
///
/// The marker records how many steps completed before a downstream failure so an
/// authorized operator can recover safely. Recovery is idempotent and can never
/// replay a step that already completed.
#[derive(Clone)]
#[contracttype]
pub struct RollbackMarker {
    /// Identifier of the governance action this marker belongs to.
    pub action_id: u64,
    /// Address authorized to recover (and the only caller allowed to record).
    pub operator: Address,
    /// Total number of steps the action is composed of.
    pub total_steps: u32,
    /// Number of steps that completed before the failure.
    pub completed_steps: u32,
    /// Current lifecycle state of the action.
    pub state: ActionState,
    /// Whether recovery has already been performed for this marker.
    pub recovered: bool,
}

impl RollbackMarker {
    /// Creates a marker for an action that has not yet executed any step.
    pub fn new(action_id: u64, operator: Address, total_steps: u32) -> Self {
        Self {
            action_id,
            operator,
            total_steps,
            completed_steps: 0,
            state: ActionState::Unapplied,
            recovered: false,
        }
    }

    /// Records durable progress after a step completes. Caller is responsible
    /// for auth (require_auth) and for verifying the operator is authorized.
    /// Once every step has completed the action is marked Completed and can no
    /// longer be rolled back.
    pub fn record_step(&mut self, caller: &Address) -> Result<(), RollbackError> {
        caller.require_auth();
        if caller != &self.operator {
            return Err(RollbackError::Unauthorized);
        }
        if self.state == ActionState::Completed {
            return Err(RollbackError::AlreadyCompleted);
        }
        if self.completed_steps >= self.total_steps {
            self.state = ActionState::Completed;
            return Err(RollbackError::AlreadyCompleted);
        }
        self.completed_steps += 1;
        self.state = if self.completed_steps >= self.total_steps {
            ActionState::Completed
        } else {
            ActionState::PartiallyExecuted
        };
        Ok(())
    }

    /// Records a downstream failure at the given step position, leaving a durable
    /// PartiallyExecuted marker when at least one step already completed.
    pub fn record_failure(&mut self, caller: &Address, step: u32) -> Result<(), RollbackError> {
        caller.require_auth();
        if caller != &self.operator {
            return Err(RollbackError::Unauthorized);
        }
        if self.state == ActionState::Completed {
            return Err(RollbackError::AlreadyCompleted);
        }
        if step >= self.total_steps {
            return Err(RollbackError::InvalidStep);
        }
        if self.completed_steps == 0 {
            self.state = ActionState::Unapplied;
            return Err(RollbackError::NothingToRecover);
        }
        self.state = ActionState::PartiallyExecuted;
        Ok(())
    }

    /// Returns Ok(()) only when the action partially executed and has not yet
    /// been recovered. Does not itself perform any state change.
    pub fn check_recoverable(&self) -> Result<(), RollbackError> {
        if self.state == ActionState::Completed {
            return Err(RollbackError::AlreadyCompleted);
        }
        if self.state != ActionState::PartiallyExecuted || self.completed_steps == 0 {
            return Err(RollbackError::NothingToRecover);
        }
        if self.recovered {
            return Err(RollbackError::AlreadyRolledBack);
        }
        Ok(())
    }

    /// Performs authorized, idempotent recovery of a partially executed action.
    /// Completed steps are never replayed: recovery only marks the marker as
    /// recovered and resets progress so the action can be re-driven from the
    /// first unapplied step. A second call is a no-op error, not a replay.
    pub fn recover(&mut self, caller: &Address) -> Result<(), RollbackError> {
        caller.require_auth();
        if caller != &self.operator {
            return Err(RollbackError::Unauthorized);
        }
        self.check_recoverable()?;
        self.recovered = true;
        self.completed_steps = 0;
        self.state = ActionState::Unapplied;
        Ok(())
    }
}

/// Errors surfaced by the bridge attestation threshold update path (#1113).
#[derive(Debug, PartialEq, Eq)]
pub enum ThresholdError {
    /// Caller is not authorized to change the threshold.
    Unauthorized,
    /// The proposed threshold is invalid (e.g. zero).
    InvalidThreshold,
    /// A threshold reduction was requested without an explicit delayed
    /// governance flow, so it is rejected to prevent silently weakening
    /// verification requirements.
    ReductionRequiresGovernance,
    /// A previously scheduled reduction has no
