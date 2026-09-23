//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
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
}
