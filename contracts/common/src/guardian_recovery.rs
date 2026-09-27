//! Initial scaffold for guardian-assisted recovery of paused critical contracts (#921).
//! Defines the guardian role model, a recovery request lifecycle, and approval gating.
//! Also provides admin-gated pause/unpause controls for critical contracts (#1018).
//! Also provides deposit retry protection for failed ledger writes (#1029).
//! Also provides a shared append-only emergency action journal (#1166).
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

/// A single append-only journal record for an emergency action.
///
/// Records are immutable once written and carry the acting address plus a
/// human-readable reason so the audit trail is self-describing.
#[derive(Clone)]
#[contracttype]
pub struct JournalRecord {
    /// Monotonic sequence number; also the ordering key for queries.
    pub seq: u64,
    /// The emergency action that was taken.
    pub action: EmergencyAction,
    /// The address that performed the action.
    pub actor: Address,
    /// Free-form reason metadata describing why the action was taken.
    pub reason: soroban_sdk::String,
    /// Ledger timestamp at which the record was appended.
    pub timestamp: u64,
}

/// Shared append-only journal of emergency actions.
///
/// Every emergency entrypoint (pause, recovery, forced settlement) appends
/// exactly one record. Records are never mutated or removed, and queries are
/// bounded and returned in ascending sequence order.
#[derive(Clone)]
#[contracttype]
pub struct EmergencyJournal {
    pub records: Vec<JournalRecord>,
    pub next_seq: u64,
}

impl EmergencyJournal {
    pub fn new(env: &Env) -> Self {
        Self {
            records: Vec::new(env),
            next_seq: 0,
        }
    }

    /// Appends exactly one record for an emergency action. The caller must have
    /// already authenticated the actor; this method re-checks auth so an
    /// unauthorized write fails before any state is mutated.
    pub fn record(
        &mut self,
        env: &Env,
        actor: &Address,
        action: EmergencyAction,
        reason: soroban_sdk::String,
    ) -> Result<u64, JournalError> {
        actor.require_auth();
        let seq = self.next_seq;
        self.records.push_back(JournalRecord {
            seq,
            action,
            actor: actor.clone(),
            reason,
            timestamp: env.ledger().timestamp(),
        });
        self.next_seq = seq + 1;
        Ok(seq)
    }

    /// Returns a bounded, ordered page of journal records.
    ///
    /// `offset` skips the first `offset` records and `limit` caps the page size.
    /// A zero limit is rejected so queries cannot be unbounded. Records are
    /// returned in ascending sequence order.
    pub fn query(&self, offset: u32, limit: u32, env: &Env) -> Result<Vec<JournalRecord>, JournalError> {
        if limit == 0 {
            return Err(JournalError::InvalidBound);
        }
        let mut page = Vec::new(env);
        let total = self.records.len();
        let mut i = offset;
        let end = offset.saturating_add(limit).min(total);
        while i < end {
            if let Some(rec) = self.records.get(i) {
                page.push_back(rec);
            }
            i += 1;
        }
        Ok(page)
    }

    /// Total number of records currently in the journal.
    pub fn len(&self) -> u32 {
        self.records.len()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::String;

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
    fn journal_records_emergency_actions_in_order() {
        let env = Env::default();
        env.mock_all_auths();
        let actor = Address::generate(&env);
        let mut journal = EmergencyJournal::new(&env);
        let s0 = journal
            .record(&env, &actor, EmergencyAction::Pause, String::from_str(&env, "incident"))
            .unwrap();
        let s1 = journal
            .record(&env, &actor, EmergencyAction::Recovery, String::from_str(&env, "resolved"))
            .unwrap();
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(journal.len(), 2);
        let page = journal.query(0, 10, &env).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().seq, 0);
        assert_eq!(page.get(1).unwrap().seq, 1);
    }

    #[test]
    fn journal_query_is_bounded_and_ordered() {
        let env = Env::default();
        env.mock_all_auths();
        let actor = Address::generate(&env);
        let mut journal = EmergencyJournal::new(&env);
        for _ in 0..5 {
            journal
                .record(&env, &actor, EmergencyAction::ForcedSettlement, String::from_str(&env, "forced"))
                .unwrap();
        }
        assert_eq!(journal.query(0, 0, &env), Err(JournalError::InvalidBound));
        let page = journal.query(1, 2, &env).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().seq, 1);
        assert_eq!(page.get(1).unwrap().seq, 2);
    }
}
