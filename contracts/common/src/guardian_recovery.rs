//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
//! Also provides a shared append-only emergency action journal (#1166).
//! Also provides delayed guardian recovery for lost administrator access (#1183).
//! Also provides scoped operator capabilities for contract maintenance actions (#1182).
//! Also provides safe overlap during bridge validator-set rotation (#1218).
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

/// Errors surfaced by the bridge validator-set rotation path (#1218).
#[derive(Debug, PartialEq, Eq)]
pub enum ValidatorSetError {
    /// The configured overlap window is invalid (e.g. zero).
    InvalidConfig,
    /// The rotation has not yet been activated.
    NotActivated,
    /// The rotation has already been retired.
    AlreadyRetired,
    /// The rotation is still within its activation boundary.
    NotYetActive,
    /// The rotation is still within its retirement boundary.
    NotYetRetired,
}

/// A rotated bridge validator set with explicit activation and retirement
/// boundaries and a bounded overlap window (#1218).
///
/// During rotation the previous set remains valid for verification until the
/// retirement boundary, but only within the bounded overlap window. Messages
/// signed before activation are verified against the previous set; messages
/// signed during the overlap window may be verified against either set; and
/// messages signed after retirement must be verified against the new set only.
#[derive(Clone)]
#[contracttype]
pub struct ValidatorSetRotation {
    /// Ledger sequence at which the new validator set becomes active.
    pub activated_at: u32,
    /// Ledger sequence at which the previous validator set is retired.
    pub retired_at: u32,
    /// Maximum number of ledgers the previous set may remain valid after
    /// activation. Bounds the overlap so signatures from unintended sets are
    /// not accepted indefinitely.
    pub overlap_window: u32,
}

impl ValidatorSetRotation {
    /// Creates a rotation with explicit activation and retirement boundaries.
    /// Rejects a zero overlap window and a retirement boundary that does not
    /// fall strictly after activation, so the overlap is always bounded and
    /// well-ordered.
    pub fn new(
        activated_at: u32,
        retired_at: u32,
        overlap_window: u32,
    ) -> Result<Self, ValidatorSetError> {
        if overlap_window == 0 || retired_at <= activated_at {
            return Err(ValidatorSetError::InvalidConfig);
        }
        Ok(Self {
            activated_at,
            retired_at,
            overlap_window,
        })
    }

    /// Returns the effective retirement boundary, clamped so the previous set
    /// never remains valid beyond the bounded overlap window after activation.
    pub fn effective_retired_at(&self) -> u32 {
        let bounded = self.activated_at.saturating_add(self.overlap_window);
        if self.retired_at < bounded {
            self.retired_at
        } else {
            bounded
        }
    }

    /// Returns true while the previous validator set is still valid for
    /// verification, i.e. within the bounded overlap window after activation.
    pub fn previous_set_valid(&self, env: &Env) -> bool {
        let now = env.ledger().sequence();
        now >= self.activated_at && now < self.effective_retired_at()
    }

    /// Returns true once the new validator set is the only valid set, i.e. the
    /// bounded overlap window has fully elapsed.
    pub fn new_set_only(&self, env: &Env) -> bool {
        env.ledger().sequence() >= self.effective_retired_at()
    }

    /// Returns Ok(()) when the rotation is active and the previous set is still
    /// within its bounded overlap window. Does not itself perform any state change.
    pub fn check_overlap(&self, env: &Env) -> Result<(), ValidatorSetError> {
        let now = env.ledger().sequence();
        if now < self.activated_at {
            return Err(ValidatorSetError::NotYetActive);
        }
        if now >= self.effective_retired_at() {
            return Err(ValidatorSetError::AlreadyRetired);
        }
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
