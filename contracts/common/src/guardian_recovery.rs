//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
//! Also provides a shared append-only emergency action journal (#1166).
//! Also provides delayed guardian recovery for lost administrator access (#1183).
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
/// actions can be gated without corrupting state.
#[derive(Clone)]
#[contracttype]
pub struct PauseControl {
    pub admin: Address,
    pub paused: bool,
}

impl PauseControl {
    pub fn new(admin: Address) -> Self {
        Self {
            admin,
            paused: false,
        }
    }

    /// Pauses the contract. Only the authorized admin may call this.
    pub fn pause(&mut self, caller: &Address, env: &Env) -> Result<(), PauseError> {
        caller.require_auth();
        if caller != &self.admin {
            return Err(PauseError::Unauthorized);
        }
        if self.paused {
            return Err(PauseError::AlreadyPaused);
        }
        self.paused = true;
        Ok(())
    }

    /// Resumes the contract. Only the authorized admin may call this.
    /// Restores normal operation without resetting any accounting state.
    pub fn unpause(&mut self, caller: &Address, env: &Env) -> Result<(), PauseError> {
        caller.require_auth();
        if caller != &self.admin {
            return Err(PauseError::Unauthorized);
        }
        if !self.paused {
            return Err(PauseError::NotPaused);
        }
        self.paused = false;
        Ok(())
    }

    /// Guard for restricted actions (trading, deposits). Returns an explicit
    /// error while the contract is paused.
    pub fn require_not_paused(&self) -> Result<(), PauseError> {
        if self.paused {
            return Err(PauseError::ContractPaused);
        }
        Ok(())
    }
}

/// Errors surfaced by the deposit retry protection path.
#[derive(Debug, PartialEq, Eq)]
pub enum DepositError {
    /// A deposit with this transaction marker was already applied.
    DuplicateDeposit,
    /// A deposit with this transaction marker is already in flight.
    DepositInFlight,
    /// The deposit amount must be strictly positive.
    InvalidAmount,
}

/// Lifecycle marker for a single deposit attempt, keyed by a caller-supplied
/// transaction id. Used to make deposit application idempotent so a retried or
/// partially-failed ledger write cannot double-count balances or rewards.
#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
pub enum DepositStatus {
    /// The deposit has been recorded but its ledger write has not committed.
    Pending,
    /// The deposit's ledger write committed successfully.
    Applied,
}

/// Per-transaction deposit marker enabling safe retries of critical deposit
/// transitions. A deposit is applied at most once per `tx_id`; retries observe
/// the existing marker instead of re-applying balances or rewards.
#[derive(Clone)]
#[contracttype]
pub struct DepositGuard {
    pub tx_id: u64,
    pub amount: i128,
    pub status: DepositStatus,
}

impl DepositGuard {
    /// Creates a new pending marker for a deposit attempt. Rejects non-positive
    /// amounts so a malformed retry cannot corrupt accounting.
    pub fn new(tx_id: u64, amount: i128) -> Result<Self, DepositError> {
        if amount <= 0 {
            return Err(DepositError::InvalidAmount);
        }
        Ok(Self {
            tx_id,
            amount,
            status: DepositStatus::Pending,
        })
    }

    /// Guard that must hold before applying a deposit's ledger write.
    ///
    /// Returns `Ok(())` only for a fresh `Pending` marker. An `Applied` marker
    /// signals a retry of an already-committed deposit and is rejected so
    /// balances and rewards are never duplicated.
    pub fn check_applicable(&self) -> Result<(), DepositError> {
        match self.status {
            DepositStatus::Pending => Ok(()),
            DepositStatus::Applied => Err(DepositError::DuplicateDeposit),
        }
    }

    /// Marks the deposit as applied after its ledger write commits. Idempotent:
    /// re-marking an already-applied deposit is a no-op, so a retried commit
    /// cannot flip state or duplicate the write.
    pub fn mark_applied(&mut self) -> Result<(), DepositError> {
        self.check_applicable()?;
        self.status = DepositStatus::Applied;
        Ok(())
    }

    /// Rolls a failed deposit back to a consistent state. A `Pending` marker is
    /// left untouched so the deposit can be safely retried; an `Applied` marker
    /// is preserved because its ledger write already committed.
    pub fn rollback_failed(&mut self) -> Result<(), DepositError> {
        match self.status {
            DepositStatus::Pending => Ok(()),
            DepositStatus::Applied => Err(DepositError::DuplicateDeposit),
        }
    }
}

/// Kind of emergency action recorded in the shared journal.
#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
pub enum EmergencyAction {
    /// A critical contract was paused.
    Pause,
    /// A paused contract was recovered/resumed.
    Recovery,
    /// A forced settlement was executed.
    ForcedSettlement,
}

/// Errors surfaced by the emergency action journal.
#[derive(Debug, PartialEq, Eq)]
pub enum JournalError {
    /// Caller is not authorized to write to the journal.
    Unauthorized,
    /// The requested query bound is invalid (e.g. zero limit).
    InvalidBound,
}

/// A single appen

/* … truncated 5159 chars — edit only what you need near the top … */
