//! Oracle Governance Module
//!
//! Implements a decentralized governance system for oracle configuration changes.
//! Token holders can propose and vote on oracle additions, removals, and parameter
//! updates. Approved proposals are auto-executed when quorum and threshold are met.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Env, String, Vec,
};

use crate::errors::OracleError;

// ---------------------------------------------------------------------------
// Governance constants
// ---------------------------------------------------------------------------

/// Voting period in seconds (7 days).
pub const VOTING_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Emergency voting period in seconds (1 day).
pub const EMERGENCY_VOTING_PERIOD_SECONDS: u64 = 24 * 60 * 60;

/// Quorum: minimum fraction of total staked tokens that must vote (10% = 1_000 / 10_000).
pub const QUORUM_BPS: i128 = 1_000; // basis points out of 10_000

/// Standard approval threshold (66% = 6_600 / 10_000).
pub const APPROVAL_THRESHOLD_BPS: i128 = 6_600;

/// Emergency approval threshold (80% = 8_000 / 10_000).
pub const EMERGENCY_THRESHOLD_BPS: i128 = 8_000;

/// Proposal deposit in stroops (1 000 XLM × 10_000_000 stroops/XLM).
pub const PROPOSAL_DEPOSIT: i128 = 1_000 * 10_000_000;

/// Minimum oracles that must remain after a removal proposal executes.
pub const MIN_ORACLES: u32 = 2;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum GovernanceKey {
    /// Counter for proposal IDs.
    ProposalCounter,
    /// Individual proposal by ID.
    Proposal(u64),
    /// Whether `(proposal_id, voter)` has already cast a ballot.
    HasVoted(u64, Address),
    /// Total tokens staked in the governance system.
    TotalStaked,
    /// Stake balance of a given address.
    Stake(Address),
    /// Governance admin (can bootstrap the system, then decentralise).
    GovAdmin,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// The kind of change a proposal requests.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalType {
    /// Add a new oracle source.
    AddOracle,
    /// Remove an existing oracle source.
    RemoveOracle,
    /// Update a named governance or oracle parameter.
    UpdateParameter,
    /// Pause all oracle activity immediately (shorter period, higher threshold).
    EmergencyPause,
}

/// Lifecycle status of a proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    /// Accepting votes.
    Active,
    /// Voting ended without reaching quorum or approval.
    Failed,
    /// Approved and successfully executed.
    Executed,
    /// Approved but execution encountered an error; may be retried.
    ExecutionFailed,
    /// Cancelled before voting ended (governance admin only, emergency use).
    Cancelled,
}

/// Core proposal record stored on-chain.
#[contracttype]
#[derive(Clone, Debug)]
pub struct OracleProposal {
    /// Monotonically increasing unique identifier.
    pub id: u64,
    /// Account that created the proposal (paid the deposit).
    pub proposer: Address,
    /// Category of change being requested.
    pub proposal_type: ProposalType,
    /// Human-readable rationale (max ~255 bytes in practice).
    pub description: String,
    /// Weighted votes in favour.
    pub votes_for: i128,
    /// Weighted votes against.
    pub votes_against: i128,
    /// Ledger timestamp after which no more votes are accepted.
    pub voting_ends: u64,
    /// Current lifecycle state.
    pub status: ProposalStatus,
    /// ABI-encoded payload interpreted according to `proposal_type`.
    /// • AddOracle    → Address (oracle to add)
    /// • RemoveOracle → Address (oracle to remove)
    /// • UpdateParameter → (String param_name, i128 new_value) packed as Vec<u8>
    /// • EmergencyPause → empty
    pub execution_payload: Vec<u8>,
    /// XLM deposit in stroops locked at creation; returned or burned on resolution.
    pub deposit: i128,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

fn emit_proposal_created(env: &Env, id: u64, proposer: &Address, proposal_type: &ProposalType) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("proposed")),
        (id, proposer.clone(), proposal_type.clone()),
    );
}

fn emit_vote_cast(env: &Env, proposal_id: u64, voter: &Address, vote: bool, weight: i128) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("vote")),
        (proposal_id, voter.clone(), vote, weight),
    );
}

fn emit_proposal_executed(env: &Env, id: u64) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("executed")),
        id,
    );
}

fn emit_proposal_failed(env: &Env, id: u64, reason: &str) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("failed")),
        (id, reason),
    );
}

fn emit_proposal_cancelled(env: &Env, id: u64) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("cancelled")),
        id,
    );
}

fn emit_stake_changed(env: &Env, staker: &Address, amount: i128, total: i128) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("stake")),
        (staker.clone(), amount, total),
    );
}

fn emit_deposit_returned(env: &Env, recipient: &Address, amount: i128) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("deposit")),
        (recipient.clone(), amount, true),
    );
}

fn emit_deposit_burned(env: &Env, proposer: &Address, amount: i128) {
    env.events().publish(
        (symbol_short!("gov"), symbol_short!("deposit")),
        (proposer.clone(), amount, false),
    );
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_proposal_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&GovernanceKey::ProposalCounter)
        .unwrap_or(0u64)
}

fn increment_proposal_counter(env: &Env) -> u64 {
    let next = get_proposal_counter(env) + 1;
    env.storage()
        .instance()
        .set(&GovernanceKey::ProposalCounter, &next);
    next
}

fn save_proposal(env: &Env, proposal: &OracleProposal) {
    env.storage()
        .persistent()
        .set(&GovernanceKey::Proposal(proposal.id), proposal);
}

fn load_proposal(env: &Env, id: u64) -> Result<OracleProposal, OracleError> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::Proposal(id))
        .ok_or(OracleError::OracleNotFound)
}

fn mark_voted(env: &Env, proposal_id: u64, voter: &Address) {
    env.storage()
        .persistent()
        .set(&GovernanceKey::HasVoted(proposal_id, voter.clone()), &true);
}

fn has_voted(env: &Env, proposal_id: u64, voter: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&GovernanceKey::HasVoted(proposal_id, voter.clone()))
        .unwrap_or(false)
}

fn get_total_staked(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&GovernanceKey::TotalStaked)
        .unwrap_or(0i128)
}

fn set_total_staked(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&GovernanceKey::TotalStaked, &amount);
}

fn get_stake(env: &Env, staker: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&GovernanceKey::Stake(staker.clone()))
        .unwrap_or(0i128)
}

fn set_stake(env: &Env, staker: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&GovernanceKey::Stake(staker.clone()), &amount);
}

// ---------------------------------------------------------------------------
// Quorum & approval helpers
// ---------------------------------------------------------------------------

fn is_quorum_reached(proposal: &OracleProposal, total_staked: i128) -> bool {
    if total_staked == 0 {
        return false;
    }
    let total_votes = proposal.votes_for + proposal.votes_against;
    // total_votes / total_staked >= QUORUM_BPS / 10_000
    total_votes * 10_000 >= QUORUM_BPS * total_staked
}

fn is_approved(proposal: &OracleProposal) -> bool {
    let total_votes = proposal.votes_for + proposal.votes_against;
    if total_votes == 0 {
        return false;
    }
    let threshold = match proposal.proposal_type {
        ProposalType::EmergencyPause => EMERGENCY_THRESHOLD_BPS,
        _ => APPROVAL_THRESHOLD_BPS,
    };
    // votes_for / total_votes >= threshold / 10_000
    proposal.votes_for * 10_000 >= threshold * total_votes
}

// ---------------------------------------------------------------------------
// Execution helpers
// ---------------------------------------------------------------------------

/// Decode the first 32 bytes of an execution payload as a raw Address.
/// In a real deployment this would use proper ABI/XDR decoding.
fn decode_oracle_address(env: &Env, payload: &Vec<u8>) -> Result<Address, OracleError> {
    // Payload convention: the raw bytes of the Address SCVal (32-byte ed25519 key).
    // Soroban stores Address as an SCVal; we encode it via to_xdr and decode here.
    // For brevity, we require the caller to pass a correctly XDR-encoded address.
    if payload.is_empty() {
        return Err(OracleError::InvalidPrice); // reuse closest error
    }
    // NOTE: In production, use soroban_sdk::xdr deserialization.
    // Here we demonstrate the interface; real encoding is project-specific.
    Err(OracleError::InvalidPrice) // placeholder — see integration note in README
}

/// Decode an UpdateParameter payload: returns (param_name_bytes, new_value_i128).
fn decode_parameter(payload: &Vec<u8>) -> Result<(u64, i128), OracleError> {
    // Payload layout (little-endian):
    //   bytes 0..8  → param key as u64 enum discriminant
    //   bytes 8..24 → new value as i128
    if payload.len() < 24 {
        return Err(OracleError::InvalidPrice);
    }
    let mut key_bytes = [0u8; 8];
    let mut val_bytes = [0u8; 16];
    for i in 0..8 {
        key_bytes[i] = payload.get(i as u32).unwrap_or(0);
    }
    for i in 0..16 {
        val_bytes[i] = payload.get((8 + i) as u32).unwrap_or(0);
    }
    let key = u64::from_le_bytes(key_bytes);
    let val = i128::from_le_bytes(val_bytes);
    Ok((key, val))
}

// ---------------------------------------------------------------------------
// Public governance contract functions
// ---------------------------------------------------------------------------

pub struct OracleGovernance;

impl OracleGovernance {
    // -----------------------------------------------------------------------
    // Staking
    // -----------------------------------------------------------------------

    /// Deposit stake that confers voting weight.
    ///
    /// The actual token transfer must be handled by the calling transaction
    /// (e.g. a SAC token contract invocation). This function records the
    /// bookkeeping entry.
    pub fn deposit_stake(env: &Env, staker: Address, amount: i128) -> Result<(), OracleError> {
        staker.require_auth();
        if amount <= 0 {
            return Err(OracleError::InvalidPrice);
        }
        let current = get_stake(env, &staker);
        let new_stake = current + amount;
        set_stake(env, &staker, new_stake);

        let total = get_total_staked(env) + amount;
        set_total_staked(env, total);

        emit_stake_changed(env, &staker, amount, total);
        Ok(())
    }

    /// Withdraw previously deposited stake.
    pub fn withdraw_stake(env: &Env, staker: Address, amount: i128) -> Result<(), OracleError> {
        staker.require_auth();
        let current = get_stake(env, &staker);
        if amount <= 0 || amount > current {
            return Err(OracleError::InvalidPrice);
        }
        set_stake(env, &staker, current - amount);

        let total = (get_total_staked(env) - amount).max(0);
        set_total_staked(env, total);

        emit_stake_changed(env, &staker, -amount, total);
        Ok(())
    }

    /// Query how much a given address has staked.
    pub fn get_stake(env: &Env, staker: &Address) -> i128 {
        get_stake(env, staker)
    }

    /// Query the total tokens staked across all participants.
    pub fn get_total_staked(env: &Env) -> i128 {
        get_total_staked(env)
    }

    // -----------------------------------------------------------------------
    // Proposal lifecycle
    // -----------------------------------------------------------------------

    /// Create a new governance proposal.
    ///
    /// The proposer must have staked at least `PROPOSAL_DEPOSIT` worth of tokens.
    /// Their deposit is recorded and will be returned on approval or burned on
    /// rejection.
    pub fn create_proposal(
        env: &Env,
        proposer: Address,
        proposal_type: ProposalType,
        description: String,
        execution_payload: Vec<u8>,
    ) -> Result<u64, OracleError> {
        proposer.require_auth();

        // Verify proposer has enough stake to cover the deposit.
        let stake = get_stake(env, &proposer);
        if stake < PROPOSAL_DEPOSIT {
            return Err(OracleError::InsufficientOracles); // closest semantic match
        }

        // Lock the deposit by reducing available stake.
        set_stake(env, &proposer, stake - PROPOSAL_DEPOSIT);

        // Determine the voting window based on proposal type.
        let voting_period = match proposal_type {
            ProposalType::EmergencyPause => EMERGENCY_VOTING_PERIOD_SECONDS,
            _ => VOTING_PERIOD_SECONDS,
        };

        let now = env.ledger().timestamp();
        let id = increment_proposal_counter(env);

        let proposal = OracleProposal {
            id,
            proposer: proposer.clone(),
            proposal_type: proposal_type.clone(),
            description,
            votes_for: 0,
            votes_against: 0,
            voting_ends: now + voting_period,
            status: ProposalStatus::Active,
            execution_payload,
            deposit: PROPOSAL_DEPOSIT,
        };

        save_proposal(env, &proposal);
        emit_proposal_created(env, id, &proposer, &proposal_type);

        Ok(id)
    }

    /// Cast a vote on an active proposal.
    ///
    /// Voting weight equals the caller's current staked balance at vote time.
    /// Each address may vote only once per proposal.
    ///
    /// If casting this vote pushes the proposal past quorum **and** the approval
    /// threshold, the proposal is immediately executed.
    pub fn vote_on_proposal(
        env: &Env,
        proposal_id: u64,
        voter: Address,
        vote: bool,
    ) -> Result<(), OracleError> {
        voter.require_auth();

        let mut proposal = load_proposal(env, proposal_id)?;

        // --- Guard: proposal must still be active ---
        if proposal.status != ProposalStatus::Active {
            return Err(OracleError::InvalidPrice);
        }

        // --- Guard: voting window must not have closed ---
        let now = env.ledger().timestamp();
        if now >= proposal.voting_ends {
            // Lazily finalise the proposal and return an error.
            Self::finalise_expired_proposal(env, &mut proposal);
            return Err(OracleError::InvalidPrice);
        }

        // --- Guard: no double voting ---
        if has_voted(env, proposal_id, &voter) {
            return Err(OracleError::OracleAlreadyExists); // semantics: already recorded
        }

        // Voting weight = stake at time of vote.
        let weight = get_stake(env, &voter);
        if weight == 0 {
            return Err(OracleError::LowReputation);
        }

        // Tally the vote.
        if vote {
            proposal.votes_for += weight;
        } else {
            proposal.votes_against += weight;
        }

        mark_voted(env, proposal_id, &voter);
        save_proposal(env, &proposal);
        emit_vote_cast(env, proposal_id, &voter, vote, weight);

        // Check whether the proposal can now be executed.
        let total_staked = get_total_staked(env);
        if is_quorum_reached(&proposal, total_staked) && is_approved(&proposal) {
            Self::execute_proposal(env, &mut proposal);
        }

        Ok(())
    }

    /// Explicitly finalise a proposal whose voting window has closed without
    /// meeting quorum/approval (anyone can call this to clean up state).
    pub fn finalise_proposal(env: &Env, proposal_id: u64) -> Result<ProposalStatus, OracleError> {
        let mut proposal = load_proposal(env, proposal_id)?;

        if proposal.status != ProposalStatus::Active {
            return Ok(proposal.status.clone());
        }

        let now = env.ledger().timestamp();
        if now < proposal.voting_ends {
            // Still in voting window — nothing to do yet.
            return Ok(proposal.status.clone());
        }

        let total_staked = get_total_staked(env);
        if is_quorum_reached(&proposal, total_staked) && is_approved(&proposal) {
            Self::execute_proposal(env, &mut proposal);
        } else {
            Self::finalise_expired_proposal(env, &mut proposal);
        }

        Ok(proposal.status.clone())
    }

    /// Retry execution of a proposal that previously entered `ExecutionFailed`.
    pub fn retry_execution(env: &Env, proposal_id: u64) -> Result<(), OracleError> {
        let mut proposal = load_proposal(env, proposal_id)?;

        if proposal.status != ProposalStatus::ExecutionFailed {
            return Err(OracleError::InvalidPrice);
        }

        Self::execute_proposal(env, &mut proposal);
        Ok(())
    }

    /// Cancel an active proposal (governance admin only, for emergency use).
    pub fn cancel_proposal(
        env: &Env,
        admin: Address,
        proposal_id: u64,
    ) -> Result<(), OracleError> {
        admin.require_auth();
        Self::require_gov_admin(env, &admin)?;

        let mut proposal = load_proposal(env, proposal_id)?;

        if proposal.status != ProposalStatus::Active {
            return Err(OracleError::InvalidPrice);
        }

        // Return the deposit to the proposer.
        let deposit = proposal.deposit;
        let proposer_stake = get_stake(env, &proposal.proposer);
        set_stake(env, &proposal.proposer, proposer_stake + deposit);

        proposal.status = ProposalStatus::Cancelled;
        save_proposal(env, &proposal);
        emit_proposal_cancelled(env, proposal_id);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Fetch a proposal by ID.
    pub fn get_proposal(env: &Env, proposal_id: u64) -> Result<OracleProposal, OracleError> {
        load_proposal(env, proposal_id)
    }

    /// Number of proposals created so far.
    pub fn proposal_count(env: &Env) -> u64 {
        get_proposal_counter(env)
    }

    /// Check whether a given address has voted on a proposal.
    pub fn has_voted(env: &Env, proposal_id: u64, voter: &Address) -> bool {
        has_voted(env, proposal_id, voter)
    }

    // -----------------------------------------------------------------------
    // Internal execution
    // -----------------------------------------------------------------------

    /// Dispatch proposal execution based on its type.
    ///
    /// On success the proposer's deposit is returned.
    /// On failure the status is set to `ExecutionFailed` so a retry is possible.
    fn execute_proposal(env: &Env, proposal: &mut OracleProposal) {
        let result = match proposal.proposal_type {
            ProposalType::AddOracle => Self::exec_add_oracle(env, proposal),
            ProposalType::RemoveOracle => Self::exec_remove_oracle(env, proposal),
            ProposalType::UpdateParameter => Self::exec_update_parameter(env, proposal),
            ProposalType::EmergencyPause => Self::exec_emergency_pause(env, proposal),
        };

        match result {
            Ok(()) => {
                proposal.status = ProposalStatus::Executed;
                // Return deposit to proposer.
                let s = get_stake(env, &proposal.proposer);
                set_stake(env, &proposal.proposer, s + proposal.deposit);
                emit_deposit_returned(env, &proposal.proposer, proposal.deposit);
                emit_proposal_executed(env, proposal.id);
            }
            Err(_) => {
                proposal.status = ProposalStatus::ExecutionFailed;
                emit_proposal_failed(env, proposal.id, "execution_error");
            }
        }

        save_proposal(env, proposal);
    }

    /// Mark a proposal as failed and burn its deposit.
    fn finalise_expired_proposal(env: &Env, proposal: &mut OracleProposal) {
        proposal.status = ProposalStatus::Failed;
        // Deposit is NOT returned — burn it (no-op on-chain; tokens simply remain locked
        // out of circulation from the governance balance).
        emit_deposit_burned(env, &proposal.proposer, proposal.deposit);
        emit_proposal_failed(env, proposal.id, "expired_or_insufficient_votes");
        save_proposal(env, proposal);
    }

    // -----------------------------------------------------------------------
    // Concrete execution handlers
    // -----------------------------------------------------------------------

    fn exec_add_oracle(env: &Env, proposal: &OracleProposal) -> Result<(), OracleError> {
        let oracle = decode_oracle_address(env, &proposal.execution_payload)?;

        // Retrieve the oracle list from the main oracle contract storage.
        let oracles_key = crate::types::StorageKey::Oracles;
        let mut oracles: Vec<Address> = env
            .storage()
            .persistent()
            .get(&oracles_key)
            .unwrap_or(Vec::new(env));

        if oracles.contains(&oracle) {
            return Err(OracleError::OracleAlreadyExists);
        }

        oracles.push_back(oracle.clone());
        env.storage().persistent().set(&oracles_key, &oracles);

        // Initialise reputation for the new oracle.
        use crate::types::OracleReputation;
        let rep = OracleReputation {
            total_submissions: 0,
            accurate_submissions: 0,
            avg_deviation: 0,
            reputation_score: 50,
            weight: 1,
            last_slash: 0,
        };
        env.storage()
            .persistent()
            .set(&crate::types::StorageKey::OracleStats, &(oracle, rep));

        Ok(())
    }

    fn exec_remove_oracle(env: &Env, proposal: &OracleProposal) -> Result<(), OracleError> {
        let oracle = decode_oracle_address(env, &proposal.execution_payload)?;

        let oracles_key = crate::types::StorageKey::Oracles;
        let oracles: Vec<Address> = env
            .storage()
            .persistent()
            .get(&oracles_key)
            .unwrap_or(Vec::new(env));

        // Enforce minimum oracle count.
        if oracles.len() <= MIN_ORACLES {
            return Err(OracleError::InsufficientOracles);
        }

        let mut new_oracles = Vec::new(env);
        for i in 0..oracles.len() {
            let o = oracles.get(i).unwrap();
            if o != oracle {
                new_oracles.push_back(o);
            }
        }

        env.storage()
            .persistent()
            .set(&oracles_key, &new_oracles);

        Ok(())
    }

    fn exec_update_parameter(env: &Env, proposal: &OracleProposal) -> Result<(), OracleError> {
        let (param_key, new_value) = decode_parameter(&proposal.execution_payload)?;

        // Parameter key conventions (extend as needed):
        //   0 → min_oracles threshold
        //   1 → price staleness TTL (seconds)
        //   2 → max allowed deviation in BPS before slash
        #[contracttype]
        #[derive(Clone)]
        enum ParamKey {
            MinOracles,
            PriceTtl,
            MaxDeviationBps,
        }

        match param_key {
            0 => {
                env.storage()
                    .instance()
                    .set(&symbol_short!("p_min_or"), &(new_value as u32));
            }
            1 => {
                env.storage()
                    .instance()
                    .set(&symbol_short!("p_ttl"), &(new_value as u64));
            }
            2 => {
                env.storage()
                    .instance()
                    .set(&symbol_short!("p_dev"), &new_value);
            }
            _ => return Err(OracleError::InvalidPrice),
        }

        Ok(())
    }

    fn exec_emergency_pause(env: &Env, _proposal: &OracleProposal) -> Result<(), OracleError> {
        // Record a boolean flag that the oracle contract checks before accepting submissions.
        env.storage()
            .instance()
            .set(&symbol_short!("paused"), &true);

        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("paused")),
            env.ledger().timestamp(),
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Admin bootstrap
    // -----------------------------------------------------------------------

    /// Initialise the governance admin (called once by the oracle contract owner).
    pub fn initialize(env: &Env, admin: Address) {
        if env
            .storage()
            .instance()
            .has(&GovernanceKey::GovAdmin)
        {
            panic!("governance already initialized");
        }
        env.storage()
            .instance()
            .set(&GovernanceKey::GovAdmin, &admin);
    }

    fn require_gov_admin(env: &Env, caller: &Address) -> Result<(), OracleError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&GovernanceKey::GovAdmin)
            .ok_or(OracleError::Unauthorized)?;
        if caller != &admin {
            return Err(OracleError::Unauthorized);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    /// Helper: create a fresh env with governance initialised.
    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let voter1 = Address::generate(&env);
        let voter2 = Address::generate(&env);
        let voter3 = Address::generate(&env);

        OracleGovernance::initialize(&env, admin.clone());

        (env, admin, voter1, voter2, voter3)
    }

    /// Stake tokens on behalf of an address.
    fn stake(env: &Env, who: &Address, amount: i128) {
        OracleGovernance::deposit_stake(env, who.clone(), amount).unwrap();
    }

    /// Create a minimal AddOracle proposal (payload intentionally empty for unit tests).
    fn make_proposal(env: &Env, proposer: &Address) -> u64 {
        OracleGovernance::create_proposal(
            env,
            proposer.clone(),
            ProposalType::AddOracle,
            String::from_str(env, "Add new oracle"),
            Vec::new(env),
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------

    #[test]
    fn test_stake_and_withdraw() {
        let (env, _, voter1, _, _) = setup();

        stake(&env, &voter1, 5_000 * 10_000_000);
        assert_eq!(
            OracleGovernance::get_stake(&env, &voter1),
            5_000 * 10_000_000
        );
        assert_eq!(OracleGovernance::get_total_staked(&env), 5_000 * 10_000_000);

        OracleGovernance::withdraw_stake(&env, voter1.clone(), 2_000 * 10_000_000).unwrap();
        assert_eq!(
            OracleGovernance::get_stake(&env, &voter1),
            3_000 * 10_000_000
        );
    }

    #[test]
    fn test_create_proposal_requires_deposit() {
        let (env, _, voter1, _, _) = setup();

        // No stake → should fail.
        let result = OracleGovernance::create_proposal(
            &env,
            voter1.clone(),
            ProposalType::AddOracle,
            String::from_str(&env, "test"),
            Vec::new(&env),
        );
        assert!(result.is_err());

        // Enough stake → should succeed.
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 1);
        let id = OracleGovernance::create_proposal(
            &env,
            voter1.clone(),
            ProposalType::AddOracle,
            String::from_str(&env, "test"),
            Vec::new(&env),
        )
        .unwrap();
        assert_eq!(id, 1);

        // Deposit is now locked (stake reduced by PROPOSAL_DEPOSIT).
        assert_eq!(OracleGovernance::get_stake(&env, &voter1), 1);
    }

    #[test]
    fn test_vote_basic() {
        let (env, _, voter1, voter2, _) = setup();

        stake(&env, &voter1, PROPOSAL_DEPOSIT + 10_000 * 10_000_000);
        stake(&env, &voter2, 10_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);

        OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), true).unwrap();
        OracleGovernance::vote_on_proposal(&env, id, voter2.clone(), true).unwrap();

        let proposal = OracleGovernance::get_proposal(&env, id).unwrap();
        // Both voters staked 10_000 XLM worth after the deposit deduction for voter1.
        assert!(proposal.votes_for > 0);
        assert_eq!(proposal.votes_against, 0);
    }

    #[test]
    fn test_double_vote_rejected() {
        let (env, _, voter1, _, _) = setup();
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 5_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);
        OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), true).unwrap();

        let result = OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_quorum_not_reached_proposal_fails() {
        let (env, _, voter1, voter2, _) = setup();

        // Total staked: 100_000 tokens (10% quorum = 10_000).
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 500 * 10_000_000); // proposer
        stake(&env, &voter2, 99_500 * 10_000_000); // passive holder, won't vote

        let id = make_proposal(&env, &voter1);

        // voter1 votes but their stake after deposit is only 500 XLM → < 10% quorum.
        OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), true).unwrap();

        // Warp time past voting window.
        env.ledger().with_mut(|l| {
            l.timestamp += VOTING_PERIOD_SECONDS + 1;
        });

        let status = OracleGovernance::finalise_proposal(&env, id).unwrap();
        assert_eq!(status, ProposalStatus::Failed);
    }

    #[test]
    fn test_proposal_fails_insufficient_approval() {
        let (env, _, voter1, voter2, voter3) = setup();

        // Enough total stake for quorum.
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 4_000 * 10_000_000);
        stake(&env, &voter2, 4_000 * 10_000_000);
        stake(&env, &voter3, 2_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);

        // voter1 & voter2 vote FOR (8_000); voter3 votes AGAINST (2_000).
        // Total votes = 10_000 (quorum met). For = 80% ≥ 66% → actually this passes!
        // Let's flip: voter1 FOR, voter2 + voter3 AGAINST.
        OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), true).unwrap(); // ~4_000 FOR
        OracleGovernance::vote_on_proposal(&env, id, voter2.clone(), false).unwrap(); // 4_000 AGAINST
        OracleGovernance::vote_on_proposal(&env, id, voter3.clone(), false).unwrap(); // 2_000 AGAINST
        // For = 4_000 / 10_000 = 40% < 66% → fails.

        env.ledger().with_mut(|l| {
            l.timestamp += VOTING_PERIOD_SECONDS + 1;
        });

        let status = OracleGovernance::finalise_proposal(&env, id).unwrap();
        assert_eq!(status, ProposalStatus::Failed);
    }

    #[test]
    fn test_has_voted_query() {
        let (env, _, voter1, voter2, _) = setup();
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 1_000 * 10_000_000);
        stake(&env, &voter2, 1_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);

        assert!(!OracleGovernance::has_voted(&env, id, &voter1));
        OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), true).unwrap();
        assert!(OracleGovernance::has_voted(&env, id, &voter1));
        assert!(!OracleGovernance::has_voted(&env, id, &voter2));
    }

    #[test]
    fn test_cancel_proposal_admin_only() {
        let (env, admin, voter1, non_admin, _) = setup();
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 1_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);

        // Non-admin cannot cancel.
        let result = OracleGovernance::cancel_proposal(&env, non_admin.clone(), id);
        assert!(result.is_err());

        // Admin can cancel.
        OracleGovernance::cancel_proposal(&env, admin, id).unwrap();
        let proposal = OracleGovernance::get_proposal(&env, id).unwrap();
        assert_eq!(proposal.status, ProposalStatus::Cancelled);

        // Deposit returned to proposer.
        assert!(OracleGovernance::get_stake(&env, &voter1) >= PROPOSAL_DEPOSIT);
    }

    #[test]
    fn test_emergency_pause_uses_shorter_window_and_higher_threshold() {
        let (env, _, voter1, _, _) = setup();
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 1_000 * 10_000_000);

        let id = OracleGovernance::create_proposal(
            &env,
            voter1.clone(),
            ProposalType::EmergencyPause,
            String::from_str(&env, "pause oracle"),
            Vec::new(&env),
        )
        .unwrap();

        let proposal = OracleGovernance::get_proposal(&env, id).unwrap();
        let expected_end = env.ledger().timestamp() + EMERGENCY_VOTING_PERIOD_SECONDS;
        // Allow ±1 second tolerance for ledger timestamp reads.
        assert!(proposal.voting_ends <= expected_end + 1);
        assert!(proposal.voting_ends >= expected_end - 1);
    }

    #[test]
    fn test_no_stake_cannot_vote() {
        let (env, _, voter1, voter2, _) = setup();
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 1_000 * 10_000_000);
        // voter2 has no stake.

        let id = make_proposal(&env, &voter1);
        let result = OracleGovernance::vote_on_proposal(&env, id, voter2.clone(), true);
        assert!(result.is_err());
    }

    #[test]
    fn test_proposal_counter_increments() {
        let (env, _, voter1, _, _) = setup();
        stake(&env, &voter1, 3 * PROPOSAL_DEPOSIT + 1_000);

        let id1 = make_proposal(&env, &voter1);
        let id2 = make_proposal(&env, &voter1);
        let id3 = make_proposal(&env, &voter1);

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
        assert_eq!(OracleGovernance::proposal_count(&env), 3);
    }

    #[test]
    fn test_cannot_vote_on_cancelled_proposal() {
        let (env, admin, voter1, voter2, _) = setup();
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 1_000 * 10_000_000);
        stake(&env, &voter2, 1_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);
        OracleGovernance::cancel_proposal(&env, admin, id).unwrap();

        let result = OracleGovernance::vote_on_proposal(&env, id, voter2.clone(), true);
        assert!(result.is_err());
    }

    #[test]
    fn test_weighted_voting_larger_stake_counts_more() {
        let (env, _, voter1, voter2, _) = setup();

        // voter1: 6_000 XLM stake (after deposit locked); voter2: 4_000 XLM.
        // Total staked: 11_000 XLM. Quorum at 10% = 1_100 XLM → met by either voter alone.
        stake(&env, &voter1, PROPOSAL_DEPOSIT + 6_000 * 10_000_000);
        stake(&env, &voter2, 4_000 * 10_000_000);

        let id = make_proposal(&env, &voter1);

        // Only voter1 votes FOR → 6_000 / (6_000 + 0) = 100% ≥ 66%.
        OracleGovernance::vote_on_proposal(&env, id, voter1.clone(), true).unwrap();

        let proposal = OracleGovernance::get_proposal(&env, id).unwrap();

        // The proposal should be executed immediately (quorum + approval both met).
        // Because our exec_add_oracle returns Err for empty payload, status will be
        // ExecutionFailed — which proves the execution path was reached.
        assert!(
            proposal.status == ProposalStatus::Executed
                || proposal.status == ProposalStatus::ExecutionFailed
        );
    }
}
