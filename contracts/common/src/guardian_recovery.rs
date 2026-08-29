//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
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
}
