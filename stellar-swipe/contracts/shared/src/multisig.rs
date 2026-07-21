//! Generic N-of-M approval trait for privileged contract actions.
//!
//! Any contract that adopts this module stores a [`MultisigConfig`] (list of
//! signers, threshold, and proposal timeout). An admin proposes an opaque
//! `Bytes` payload, then other signers approve it. Once the approval count
//! reaches the configured threshold **and** the proposal has not expired, a
//! caller may execute the action.
//!
//! # Usage
//!
//! 1. Call [`set_config`] during initialisation (admin-only).
//! 2. For each privileged action add a `propose_action` entrypoint that
//!    encodes the action into a `Bytes` payload and calls [`propose`].
//! 3. Add `approve_action` and `execute_action` entrypoints that delegate to
//!    [`approve`] and [`require_can_execute`] / [`mark_executed`].
//! 4. In `execute_action` decode the payload, perform the privileged work,
//!    then call [`mark_executed`].

use soroban_sdk::{contracterror, contracttype, Address, Bytes, Env, Vec};

// ── Constants ───────────────────────────────────────────────────────────────────

/// Maximum number of signers allowed in a config.
pub const MAX_SIGNERS: u32 = 20;

/// Default timeout for proposals (7 days in seconds).
pub const DEFAULT_PROPOSAL_TIMEOUT_SECS: u64 = 604_800;

// ── Errors ──────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MultisigError {
    /// Caller is not a registered signer.
    Unauthorized = 1,
    /// The requested proposal does not exist.
    ProposalNotFound = 2,
    /// The signer has already approved this proposal.
    AlreadyApproved = 3,
    /// The proposal has expired and can no longer be executed.
    ProposalExpired = 4,
    /// The proposal has already been executed.
    AlreadyExecuted = 5,
    /// The approval threshold has not yet been reached.
    ThresholdNotMet = 6,
    /// The config contains invalid values (threshold = 0, threshold > signers, etc.).
    InvalidConfig = 7,
    /// Duplicate signer addresses in the config.
    DuplicateSigner = 8,
}

// ── Storage keys ────────────────────────────────────────────────────────────────

/// Storage keys used by this module.
///
/// The enum discriminant is part of the serialised key, so these will **not**
/// collide with a contract-local `StorageKey` enum that happens to have
/// variants with the same name.
#[contracttype]
#[derive(Clone)]
pub enum MultisigStorageKey {
    Config,
    NextId,
    Proposal(u64),
}

// ── Types ───────────────────────────────────────────────────────────────────────

/// On-chain multi-sig configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultisigConfig {
    /// Registered signer addresses (M).
    pub signers: Vec<Address>,
    /// Approvals required before execution (N).
    pub threshold: u32,
    /// Seconds after which a proposal that hasn't reached threshold expires.
    pub proposal_timeout_secs: u64,
}

/// Status of a proposal.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ProposalStatus {
    /// Waiting for more approvals.
    Pending = 0,
    /// Executed — terminal state.
    Executed = 1,
}

/// A generic multi-sig proposal wrapping an opaque action payload.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    /// Sequential proposal identifier.
    pub id: u64,
    /// Address that created the proposal.
    pub proposer: Address,
    /// Opaque action payload (contract-specific encoding).
    pub payload: Bytes,
    /// Current approval count.
    pub approvals: Vec<Address>,
    /// Current status.
    pub status: ProposalStatus,
    /// Ledger timestamp when the proposal was created.
    pub created_at: u64,
    /// Timestamp after which the proposal expires (exclusive upper bound).
    pub expires_at: u64,
}

// ── Internal helpers ────────────────────────────────────────────────────────────

fn get_next_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&MultisigStorageKey::NextId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&MultisigStorageKey::NextId, &(id + 1));
    id
}

// ── Configuration ───────────────────────────────────────────────────────────────

/// Return the stored multi-sig configuration, or a sensible default.
pub fn get_config(env: &Env) -> Option<MultisigConfig> {
    env.storage()
        .instance()
        .get(&MultisigStorageKey::Config)
}

/// Persist a new multi-sig configuration.
///
/// # Errors
/// - [`MultisigError::InvalidConfig`] — threshold is 0 or exceeds signer count.
/// - [`MultisigError::DuplicateSigner`] — signers list contains a duplicate.
pub fn set_config(env: &Env, config: &MultisigConfig) -> Result<(), MultisigError> {
    validate_config(config)?;
    env.storage()
        .instance()
        .set(&MultisigStorageKey::Config, config);
    Ok(())
}

/// Validate that a configuration is well-formed.
pub fn validate_config(config: &MultisigConfig) -> Result<(), MultisigError> {
    if config.threshold == 0 || config.threshold > config.signers.len() {
        return Err(MultisigError::InvalidConfig);
    }
    if config.signers.len() > MAX_SIGNERS {
        return Err(MultisigError::InvalidConfig);
    }
    let mut i: u32 = 0;
    while i < config.signers.len() {
        let mut j: u32 = i + 1;
        while j < config.signers.len() {
            if config.signers.get(i).unwrap() == config.signers.get(j).unwrap() {
                return Err(MultisigError::DuplicateSigner);
            }
            j += 1;
        }
        i += 1;
    }
    Ok(())
}

/// Return `true` when `address` is in the signer list.
pub fn is_signer(config: &MultisigConfig, address: &Address) -> bool {
    let mut i: u32 = 0;
    while i < config.signers.len() {
        if &config.signers.get(i).unwrap() == address {
            return true;
        }
        i += 1;
    }
    false
}

// ── Proposals ───────────────────────────────────────────────────────────────────

/// Create a new proposal on behalf of `proposer`.
///
/// `proposer` **must** be a registered signer and **must** have already
/// called `require_auth` (or `require_auth_for_args`) before calling this
/// function.
///
/// Returns the newly assigned proposal ID.
///
/// # Errors
/// - [`MultisigError::Unauthorized`] — proposer is not a registered signer.
/// - [`MultisigError::InvalidConfig`] — no multi-sig config has been set.
pub fn propose(
    env: &Env,
    proposer: &Address,
    payload: Bytes,
) -> Result<u64, MultisigError> {
    let config = get_config(env).ok_or(MultisigError::InvalidConfig)?;
    if !is_signer(&config, proposer) {
        return Err(MultisigError::Unauthorized);
    }

    let now = env.ledger().timestamp();
    let id = get_next_id(env);
    let timeout = if config.proposal_timeout_secs > 0 {
        config.proposal_timeout_secs
    } else {
        DEFAULT_PROPOSAL_TIMEOUT_SECS
    };

    let mut approvals: Vec<Address> = Vec::new(env);
    approvals.push_back(proposer.clone());

    let proposal = Proposal {
        id,
        proposer: proposer.clone(),
        payload,
        approvals,
        status: ProposalStatus::Pending,
        created_at: now,
        expires_at: now.saturating_add(timeout),
    };

    env.storage()
        .persistent()
        .set(&MultisigStorageKey::Proposal(id), &proposal);

    Ok(id)
}

/// Record an approval from `caller` for `proposal_id`.
///
/// `caller` **must** have already called `require_auth` before calling this
/// function.
///
/// Returns the **current** number of approvals (after recording this one).
///
/// # Errors
/// - [`MultisigError::ProposalNotFound`] — no proposal with that id.
/// - [`MultisigError::AlreadyExecuted`] — proposal has already been executed.
/// - [`MultisigError::ProposalExpired`] — proposal has expired.
/// - [`MultisigError::AlreadyApproved`] — caller has already approved.
/// - [`MultisigError::Unauthorized`] — caller is not a registered signer.
pub fn approve(env: &Env, caller: &Address, proposal_id: u64) -> Result<u32, MultisigError> {
    let config = get_config(env).ok_or(MultisigError::InvalidConfig)?;
    if !is_signer(&config, caller) {
        return Err(MultisigError::Unauthorized);
    }

    let mut proposal = get_proposal(env, proposal_id)?;

    if proposal.status == ProposalStatus::Executed {
        return Err(MultisigError::AlreadyExecuted);
    }
    if env.ledger().timestamp() > proposal.expires_at {
        return Err(MultisigError::ProposalExpired);
    }
    if has_approved(&proposal, caller) {
        return Err(MultisigError::AlreadyApproved);
    }

    proposal.approvals.push_back(caller.clone());
    let count = proposal.approvals.len();

    env.storage()
        .persistent()
        .set(&MultisigStorageKey::Proposal(proposal_id), &proposal);

    Ok(count)
}

/// Return `true` if `signer` has already approved `proposal`.
pub fn has_approved(proposal: &Proposal, signer: &Address) -> bool {
    let mut i: u32 = 0;
    while i < proposal.approvals.len() {
        if &proposal.approvals.get(i).unwrap() == signer {
            return true;
        }
        i += 1;
    }
    false
}

/// Load a proposal by id.
pub fn get_proposal(env: &Env, proposal_id: u64) -> Result<Proposal, MultisigError> {
    env.storage()
        .persistent()
        .get(&MultisigStorageKey::Proposal(proposal_id))
        .ok_or(MultisigError::ProposalNotFound)
}

// ── Execution guard ─────────────────────────────────────────────────────────────

/// Check whether `proposal` is ready to execute.
///
/// Returns `Ok(())` when:
/// 1. The proposal is still `Pending` (not already executed).
/// 2. The approval count meets or exceeds the config threshold.
/// 3. The current ledger timestamp has not passed `expires_at`.
///
/// # Errors
/// - [`MultisigError::InvalidConfig`] — no multi-sig config set.
/// - [`MultisigError::AlreadyExecuted`] — already executed.
/// - [`MultisigError::ThresholdNotMet`] — not enough approvals yet.
/// - [`MultisigError::ProposalExpired`] — proposal has expired.
pub fn require_can_execute(env: &Env, proposal: &Proposal) -> Result<(), MultisigError> {
    let config = get_config(env).ok_or(MultisigError::InvalidConfig)?;

    if proposal.status == ProposalStatus::Executed {
        return Err(MultisigError::AlreadyExecuted);
    }
    if proposal.approvals.len() < config.threshold {
        return Err(MultisigError::ThresholdNotMet);
    }
    if env.ledger().timestamp() > proposal.expires_at {
        return Err(MultisigError::ProposalExpired);
    }
    Ok(())
}

/// Transition the proposal to `Executed`.
///
/// This should be called **after** the privileged action has successfully
/// completed.  Once marked executed the proposal cannot be run again.
pub fn mark_executed(env: &Env, proposal_id: u64) -> Result<(), MultisigError> {
    let mut proposal = get_proposal(env, proposal_id)?;
    if proposal.status == ProposalStatus::Executed {
        return Err(MultisigError::AlreadyExecuted);
    }
    proposal.status = ProposalStatus::Executed;
    env.storage()
        .persistent()
        .set(&MultisigStorageKey::Proposal(proposal_id), &proposal);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Bytes, Env};

    #[contract]
    struct TestHost;

    #[contractimpl]
    impl TestHost {
        pub fn dummy() {}
    }

    fn setup() -> (Env, Address, Vec<Address>, MultisigConfig) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(TestHost, ());
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let signer_c = Address::generate(&env);
        let signers = soroban_sdk::vec![&env, signer_a.clone(), signer_b.clone(), signer_c.clone()];
        let config = MultisigConfig {
            signers: signers.clone(),
            threshold: 2,
            proposal_timeout_secs: 86_400, // 1 day
        };
        env.as_contract(&contract_id, || {
            set_config(&env, &config).unwrap();
        });
        (env, contract_id, signers, config)
    }

    // ── propose ──────────────────────────────────────────────────────────────────

    #[test]
    fn propose_creates_proposal_and_auto_approves_proposer() {
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let payload = Bytes::new(&env);

        let id = env.as_contract(&cid, || {
            propose(&env, &proposer, payload).unwrap()
        });

        let proposal = env.as_contract(&cid, || get_proposal(&env, id).unwrap());
        assert_eq!(proposal.proposer, proposer);
        assert_eq!(proposal.status, ProposalStatus::Pending);
        assert_eq!(proposal.approvals.len(), 1);
        assert!(has_approved(&proposal, &proposer));
    }

    #[test]
    fn propose_by_non_signer_rejected() {
        let (env, cid, _signers, _config) = setup();
        let impostor = Address::generate(&env);
        let payload = Bytes::new(&env);

        let result = env.as_contract(&cid, || propose(&env, &impostor, payload));
        assert_eq!(result, Err(MultisigError::Unauthorized));
    }

    #[test]
    fn propose_without_config_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(TestHost, ());
        let signer = Address::generate(&env);
        let payload = Bytes::new(&env);

        let result = env.as_contract(&contract_id, || propose(&env, &signer, payload));
        assert_eq!(result, Err(MultisigError::InvalidConfig));
    }

    // ── approve ──────────────────────────────────────────────────────────────────

    #[test]
    fn approval_reaches_threshold() {
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let approver = signers.get(1).unwrap();
        let payload = Bytes::new(&env);

        let id = env.as_contract(&cid, || {
            let id = propose(&env, &proposer, payload).unwrap();
            let count = approve(&env, &approver, id).unwrap();
            assert_eq!(count, 2);
            id
        });

        let proposal = env.as_contract(&cid, || get_proposal(&env, id).unwrap());
        assert_eq!(proposal.approvals.len(), 2);
        assert!(require_can_execute(&env, &proposal).is_ok());
    }

    #[test]
    fn duplicate_approval_rejected() {
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let payload = Bytes::new(&env);

        let id = env.as_contract(&cid, || {
            let id = propose(&env, &proposer, payload).unwrap();
            // First approval by a second signer should succeed
            let _ = approve(&env, &signers.get(1).unwrap(), id).unwrap();
            // Proposer trying again should fail
            let result = approve(&env, &proposer, id);
            assert_eq!(result, Err(MultisigError::AlreadyApproved));
            id
        });
    }

    #[test]
    fn unauthorized_signer_approval_rejected() {
        let (env, cid, _signers, _config) = setup();
        let impostor = Address::generate(&env);
        let payload = Bytes::new(&env);
        let proposer = Address::generate(&env);

        let result = env.as_contract(&cid, || {
            // We need to first create a valid proposal. Since impostor can't propose,
            // let's use the existing signers from the config through a workaround.
            // Actually we need to set up a valid proposal first.
            let _ = set_config(
                &env,
                &MultisigConfig {
                    signers: soroban_sdk::vec![&env, proposer.clone()],
                    threshold: 1,
                    proposal_timeout_secs: 86_400,
                },
            );
            let id = propose(&env, &proposer, payload).unwrap();
            approve(&env, &impostor, id)
        });
        assert_eq!(result, Err(MultisigError::Unauthorized));
    }

    // ── expiry ───────────────────────────────────────────────────────────────────

    #[test]
    fn proposal_expires_and_cannot_execute() {
        use soroban_sdk::testutils::Ledger;
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let approver = signers.get(1).unwrap();
        let payload = Bytes::new(&env);

        let id = env.as_contract(&cid, || {
            let id = propose(&env, &proposer, payload).unwrap();
            let _ = approve(&env, &approver, id).unwrap();
            // Jump past expiry
            env.ledger().with_mut(|l| l.timestamp = 86_401);
            let proposal = get_proposal(&env, id).unwrap();
            let result = require_can_execute(&env, &proposal);
            assert_eq!(result, Err(MultisigError::ProposalExpired));
            // approve should also fail
            let result = approve(&env, &signers.get(2).unwrap(), id);
            assert_eq!(result, Err(MultisigError::ProposalExpired));
            id
        });
    }

    #[test]
    fn proposal_not_expired_at_exact_expiry_second() {
        use soroban_sdk::testutils::Ledger;
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let approver = signers.get(1).unwrap();
        let payload = Bytes::new(&env);

        env.as_contract(&cid, || {
            let id = propose(&env, &proposer, payload).unwrap();
            let _ = approve(&env, &approver, id).unwrap();
            // Jump to exactly the expiry second (exclusive upper bound)
            env.ledger().with_mut(|l| l.timestamp = 86_400);
            let proposal = get_proposal(&env, id).unwrap();
            assert!(require_can_execute(&env, &proposal).is_ok());
        });
    }

    // ── execute ──────────────────────────────────────────────────────────────────

    #[test]
    fn mark_executed_and_idempotent_rejection() {
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let approver = signers.get(1).unwrap();
        let payload = Bytes::new(&env);

        env.as_contract(&cid, || {
            let id = propose(&env, &proposer, payload).unwrap();
            let _ = approve(&env, &approver, id).unwrap();
            let proposal = get_proposal(&env, id).unwrap();
            assert!(require_can_execute(&env, &proposal).is_ok());

            // First execution succeeds
            assert!(mark_executed(&env, id).is_ok());
            // Second attempt fails
            assert_eq!(mark_executed(&env, id), Err(MultisigError::AlreadyExecuted));
            // The proposal should now be Executed
            let proposal = get_proposal(&env, id).unwrap();
            assert_eq!(proposal.status, ProposalStatus::Executed);
        });
    }

    #[test]
    fn execute_before_threshold_rejected() {
        let (env, cid, signers, _config) = setup();
        let proposer = signers.get(0).unwrap();
        let payload = Bytes::new(&env);

        env.as_contract(&cid, || {
            let id = propose(&env, &proposer, payload).unwrap();
            let proposal = get_proposal(&env, id).unwrap();
            // Only 1 approval (proposer), threshold is 2
            let result = require_can_execute(&env, &proposal);
            assert_eq!(result, Err(MultisigError::ThresholdNotMet));
        });
    }
}
