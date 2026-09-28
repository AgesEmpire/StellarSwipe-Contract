//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
//! Also provides a shared append-only emergency action journal (#1166).
//! Also provides delayed guardian recovery for lost administrator access (#1183).
//! Also provides scoped operator capabilities for contract maintenance actions (#1182).
//! Also provides monotonic, auditable bridge attestation threshold updates (#1113).
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
    /// A previously scheduled reduction has not yet matured.
    DelayNotElapsed,
}

/// Monotonic, auditable bridge attestation threshold state (#1113).
///
/// Increases take effect immediately. Reductions are only accepted through an
/// explicit delayed governance flow: the reduction is scheduled, must mature
/// over `reduction_delay` ledgers, and only then can be applied. Every accepted
/// change records the old value, the new value, and the actor so the change is
/// auditable.
#[derive(Clone)]
#[contracttype]
pub struct BridgeThreshold {
    /// Current effective attestation threshold.
    pub threshold: u32,
    /// Delay (in ledgers) a reduction must wait before it can be applied.
    pub reduction_delay: u32,
    /// Pending reduced threshold, if a reduction has been scheduled.
    pub pending_threshold: u32,
    /// Ledger sequence at which the pending reduction was scheduled.
    pub pending_at: u32,
    /// Whether a reduction is currently pending.
    pub has_pending: bool,
    /// Threshold value before the most recent accepted change.
    pub last_old: u32,
    /// Threshold value after the most recent accepted change.
    pub last_new: u32,
    /// Actor that performed the most recent accepted change.
    pub last_actor: Address,
}

impl BridgeThreshold {
    /// Creates a new threshold state. Rejects a zero threshold or zero delay so
    /// a misconfigured bridge cannot bypass attestation or the reduction delay.
    pub fn new(
        threshold: u32,
        reduction_delay: u32,
        actor: Address,
        env: &Env,
    ) -> Result<Self, ThresholdError> {
        if threshold == 0 || reduction_delay == 0 {
            return Err(ThresholdError::InvalidThreshold);
        }
        Ok(Self {
            threshold,
            reduction_delay,
            pending_threshold: 0,
            pending_at: 0,
            has_pending: false,
            last_old: threshold,
            last_new: threshold,
            last_actor: actor,
        })
    }

    /// Applies a threshold change. Increases take effect immediately. Reductions
    /// are rejected unless they go through the explicit delayed governance flow
    /// (`schedule_reduction` then `apply_reduction`). Every accepted change
    /// records the old value, new value, and actor for auditability.
    pub fn set_threshold(
        &mut self,
        caller: &Address,
        new_threshold: u32,
    ) -> Result<(), ThresholdError> {
        caller.require_auth();
        if new_threshold == 0 {
            return Err(ThresholdError::InvalidThreshold);
        }
        if new_threshold < self.threshold {
            return Err(ThresholdError::ReductionRequiresGovernance);
        }
        if new_threshold == self.threshold {
            return Ok(());
        }
        self.record_change(caller, new_threshold);
        Ok(())
    }

    /// Schedules a threshold reduction through the explicit delayed governance
    /// flow. The reduction only takes effect once `reduction_delay` ledgers have
    /// elapsed and `apply_reduction` is called.
    pub fn schedule_reduction(
        &mut self,
        caller: &Address,
        new_threshold: u32,
        env: &Env,
    ) -> Result<(), ThresholdError> {
        caller.require_auth();
        if new_threshold == 0 {
            return Err(ThresholdError::InvalidThreshold);
        }
        if new_threshold >= self.threshold {
            return Err(ThresholdError::InvalidThreshold);
        }
        self.pending_threshold = new_threshold;
        self.pending_at = env.ledger().sequence();
        self.has_pending = true;
        Ok(())
    }

    /// Applies a previously scheduled reduction once the delay has elapsed.
    /// Records the old value, new value, and actor for auditability.
    pub fn apply_reduction(
        &mut self,
        caller: &Address,
        env: &Env,
    ) -> Result<(), ThresholdError> {
        caller.require_auth();
        if !self.has_pending {
            return Err(ThresholdError::InvalidThreshold);
        }
        let elapsed = env.ledger().sequence().saturating_sub(self.pending_at);
        if elapsed < self.reduction_delay {
            return Err(ThresholdError::DelayNotElapsed);
        }
        let new_threshold = self.pending_threshold;
        self.has_pending = false;
        self.pending_threshold = 0;
        self.pending_at = 0;
        self.record_change(caller, new_threshold);
        Ok(())
    }

    /// Records an accepted change and emits an audit event carrying the old
    /// value, the new value, and the actor.
    fn record_change(&mut self, caller: &Address, new_threshold: u32) {
        let old = self.threshold;
        self.threshold = new_threshold;
        self.last_old = old;
        self.last_new = new_threshold;
        self.last_actor = caller.clone();
        Env::default().events().publish(
            (soroban_sdk::symbol_short!("thr_chg"),),
            (old, new_threshold, caller.clone()),
        );
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
