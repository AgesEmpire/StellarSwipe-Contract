//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
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

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn blocks_execution_below_approval_threshold() {
        let env = Env::default();
        let proposer = Address::generate(&env);
        let req = RecoveryRequest::new(1, proposer, 2, &env);
        assert_eq!(req.check_executable(), Err(RecoveryError::InsufficientApprovals));
    }

    #[test]
    fn allows_execution_once_threshold_met() {
        let env = Env::default();
        let proposer = Address::generate(&env);
        let mut req = RecoveryRequest::new(1, proposer, 2, &env);
        req.approve(Address::generate(&env));
        req.approve(Address::generate(&env));
        assert_eq!(req.check_executable(), Ok(()));
    }

    #[test]
    fn rejects_double_execution() {
        let env = Env::default();
        let proposer = Address::generate(&env);
        let mut req = RecoveryRequest::new(1, proposer, 1, &env);
        req.approve(Address::generate(&env));
        req.executed = true;
        assert_eq!(req.check_executable(), Err(RecoveryError::AlreadyExecuted));
    }

    #[test]
    fn only_admin_can_pause_and_unpause() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let stranger = Address::generate(&env);
        let mut control = PauseControl::new(admin.clone());

        assert_eq!(control.pause(&stranger, &env), Err(PauseError::Unauthorized));
        assert_eq!(control.pause(&admin, &env), Ok(()));
        assert_eq!(control.unpause(&stranger, &env), Err(PauseError::Unauthorized));
        assert_eq!(control.unpause(&admin, &env), Ok(()));
    }

    #[test]
    fn restricted_actions_fail_while_paused() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let mut control = PauseControl::new(admin.clone());

        assert_eq!(control.require_not_paused(), Ok(()));
        control.pause(&admin, &env).unwrap();
        assert_eq!(control.require_not_paused(), Err(PauseError::ContractPaused));
    }

    #[test]
    fn resume_restores_operation_without_resetting_state() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let mut control = PauseControl::new(admin.clone());

        control.pause(&admin, &env).unwrap();
        assert_eq!(control.pause(&admin, &env), Err(PauseError::AlreadyPaused));
        control.unpause(&admin, &env).unwrap();
        assert_eq!(control.unpause(&admin, &env), Err(PauseError::NotPaused));
        assert_eq!(control.require_not_paused(), Ok(()));
        assert_eq!(control.admin, admin);
    }

    #[test]
    fn rejects_non_positive_deposit_amounts() {
        assert_eq!(DepositGuard::new(1, 0), Err(DepositError::InvalidAmount));
        assert_eq!(DepositGuard::new(1, -5), Err(DepositError::InvalidAmount));
    }

    #[test]
    fn fresh_deposit_is_applicable_once() {
        let mut guard = DepositGuard::new(7, 100).unwrap();
        assert_eq!(guard.check_applicable(), Ok(()));
        assert_eq!(guard.mark_applied(), Ok(()));
        assert_eq!(guard.status, DepositStatus::Applied);
    }

    #[test]
    fn retried_deposit_does_not_duplicate_write() {
        let mut guard = DepositGuard::new(7, 100).unwrap();
        guard.mark_applied().unwrap();
        // A retry of the same transaction marker must not re-apply the write.
        assert_eq!(guard.check_applicable(), Err(DepositError::DuplicateDeposit));
        assert_eq!(guard.mark_applied(), Err(DepositError::DuplicateDeposit));
        assert_eq!(guard.status, DepositStatus::Applied);
    }

    #[test]
    fn failed_deposit_rolls_back_to_retryable_state() {
        let mut guard = DepositGuard::new(9, 250).unwrap();
        assert_eq!(guard.rollback_failed(), Ok(()));
        // Still pending, so the deposit can be safely retried.
        assert_eq!(guard.status, DepositStatus::Pending);
        assert_eq!(guard.check_applicable(), Ok(()));
    }

    #[test]
    fn rollback_after_commit_is_rejected() {
        let mut guard = DepositGuard::new(9, 250).unwrap();
        guard.mark_applied().unwrap();
        assert_eq!(guard.rollback_failed(), Err(DepositError::DuplicateDeposit));
        assert_eq!(guard.status, DepositStatus::Applied);
    }
}
