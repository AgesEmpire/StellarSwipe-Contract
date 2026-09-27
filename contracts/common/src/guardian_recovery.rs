//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
//! Also provides a shared append-only emergency action journal (#1166).
//! Also provides delayed guardian recovery for lost administrator access (#1183).
//! Also provides scoped operator capabilities for contract maintenance actions (#1182).
//! Also defines cancellation rules for queued governance timelock actions (#1210).
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

/// Lifecycle states a queued governance timelock action can occupy (#1210).
///
/// Cancellation is only permitted from `Queued`; once an action is `Executed`
/// or `Cancelled` it is terminal and can never be executed later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[contracttype]
pub enum TimelockState {
    /// Waiting out the timelock delay; the only cancellable state.
    Queued,
    /// The timelock delay has elapsed and the action may be executed.
    Ready,
    /// The action has been executed; terminal.
    Executed,
    /// The action was cancelled; terminal and never executable.
    Cancelled,
}

/// Errors surfaced by the governance timelock cancellation path (#1210).
#[derive(Debug, PartialEq, Eq)]
pub enum TimelockError {
    /// Caller is not authorized to cancel this action.
    Unauthorized,
    /// The action is not in a state from which cancellation is allowed.
    NotCancellable,
    /// The action has already been executed and can never be cancelled.
    AlreadyExecuted,
    /// The action has already been cancelled.
    AlreadyCancelled,
    /// The action was cancelled and can never be executed.
    Cancelled,
    /// The configured timelock delay has not yet elapsed.
    DelayNotElapsed,
}

/// A queued governance action subject to a timelock delay (#1210).
///
/// Cancellation authority: only the `governance_admin` (the governance authority
/// that queued the action) may cancel. Cancellation is allowed only while the
/// action is `Queued`; `Executed` and `Cancelled` are terminal states. A
/// cancelled action can never be executed later because `check_executable`
/// rejects the `Cancelled` state before any execution can occur.
#[derive(Clone)]
#[contracttype]
pub struct TimelockAction {
    /// The governance authority that queued the action and may cancel it.
    pub governance_admin: Address,
    /// Delay (in ledgers) that must elapse between queuing and execution.
    pub delay: u32,
    /// Ledger sequence at which the action was queued.
    pub queued_at: u32,
    /// Current lifecycle state of the action.
    pub state: TimelockState,
}

impl TimelockAction {
    /// Queues a new governance action. Rejects a zero delay so a misconfigured
    /// action cannot bypass the timelock.
    pub fn new(
        governance_admin: Address,
        delay: u32,
        env: &Env,
    ) -> Result<Self, TimelockError> {
        if delay == 0 {
            return Err(TimelockError::NotCancellable);
        }
        Ok(Self {
            governance_admin,
            delay,
            queued_at: env.ledger().sequence(),
            state: TimelockState::Queued,
        })
    }

    /// Cancels a queued governance action. Only the governance admin may cancel,
    /// and only while the action is still `Queued`. Executed or already-cancelled
    /// actions are rejected so cancellation cannot corrupt terminal state.
    pub fn cancel(&mut self, caller: &Address) -> Result<(), TimelockError> {
        caller.require_auth();
        if caller != &self.governance_admin {
            return Err(TimelockError::Unauthorized);
        }
        match self.state {
            TimelockState::Queued => {
                self.state = TimelockState::Cancelled;
                Ok(())
            }
            TimelockState::Executed => Err(TimelockError::AlreadyExecuted),
            TimelockState::Cancelled => Err(TimelockError::AlreadyCancelled),
            TimelockState::Ready => Err(TimelockError::NotCancellable),
        }
    }

    /// Returns Ok(()) only when the action is still queued/ready and the timelock
    /// delay has fully elapsed. A cancelled action is always rejected, so it can
    /// never be executed later.
    pub fn check_executable(&self, env: &Env) -> Result<(), TimelockError> {
        match self.state {
            TimelockState::Cancelled => return Err(TimelockError::Cancelled),
            TimelockState::Executed => return Err(TimelockError::AlreadyExecuted),
            TimelockState::Queued | TimelockState::Ready => {}
        }
        let elapsed = env.ledger().sequence().saturating_sub(self.queued_at);
        if elapsed < self.delay {
            return Err(TimelockError::DelayNotElapsed);
        }
        Ok(())
    }

    /// Executes the action once the timelock delay has elapsed, marking it
    /// executed so it cannot be replayed. Cancelled actions are rejected by
    /// `check_executable` and can never reach this state change.
    pub fn execute(&mut self, env: &Env) -> Result<(), TimelockError> {
        self.check_executable(env)?;
        self.state = TimelockState::Executed;
        Ok(())
    }
}

/// Errors surfaced by the pause/unpause control path.
#[derive(Debug, PartialEq, Eq)]
pub enum PauseError {
    /// Caller is not the authorized admin.
    Unauthorized,
    /// Contract is already paused.
    AlreadyPaused,
    /// Contract is not currently paused.
    NotPaused,
    /// A restricted action was attempted while the contract is paused.
    ContractPaused,
}

/// Admin-gated pause state for a critical contract.
///
/// Only the stored admin may pause or resume. Pausing/resuming never touches
/// reserve or share accounting; it only flips the `paused` flag so restricted
/// actions can be gated without corrupti

/* … truncated 5155 chars — edit only what you need near the top … */
