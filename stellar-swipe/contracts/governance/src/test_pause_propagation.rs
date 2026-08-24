/// Tests: governance pause state propagates consistently across dependent modules.
///
/// An admin pause issued via `set_contract_paused(true)` must block:
///   1. Proposal creation
///   2. Vote casting
///   3. Proposal finalization
///   4. Proposal execution
///   5. Staking actions (stake / unstake)
///   6. Timelock queuing
///   7. Timelock execution
///   8. Batch timelock execution
///
/// Unpausing (set_contract_paused(false)) must restore all of the above.
/// Read-only operations (health_check, proposal, balance) remain unaffected.
extern crate std;

use crate::distribution::DistributionRecipients;
use crate::proposals::{ProposalCategory, ProposalStatus, ProposalType};
use crate::timelock::ActionType;
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, String, Vec};

// ── helpers ──────────────────────────────────────────────────────────────────

const SUPPLY: i128 = 1_000_000_000;

fn setup() -> (Env, Address, Address, DistributionRecipients) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let recipients = DistributionRecipients {
        team: Address::generate(&env),
        early_investors: Address::generate(&env),
        community_rewards: Address::generate(&env),
        treasury: Address::generate(&env),
        public_sale: Address::generate(&env),
    };
    (env, contract_id, admin, recipients)
}

fn client<'a>(env: &'a Env, id: &'a Address) -> GovernanceContractClient<'a> {
    GovernanceContractClient::new(env, id)
}

fn init(c: &GovernanceContractClient<'_>, env: &Env, admin: &Address, r: &DistributionRecipients) {
    c.initialize(
        admin,
        &String::from_str(env, "StellarSwipe Gov"),
        &String::from_str(env, "SSG"),
        &7u32,
        &SUPPLY,
        r,
    );
}

fn prevent_auto_execution(c: &GovernanceContractClient<'_>, admin: &Address) {
    c.configure_governance(
        admin,
        &crate::proposals::GovernanceConfig {
            min_proposal_threshold: 1_000,
            voting_period: 7 * 86_400,
            voting_delay: 60,
            quorum_threshold: 1_000,
            approval_threshold: 5_000,
            execution_delay: 3_600,
            discussion_duration: 0,
        },
    );
}

/// Stake enough for a user to have proposal-creation voting power.
/// community_rewards holder starts with 300_000_000 balance.
fn stake_tokens(c: &GovernanceContractClient<'_>, user: &Address, amount: i128) {
    c.stake(user, &amount);
}

fn stake_for_quorum(c: &GovernanceContractClient<'_>, user: &Address) {
    c.stake(user, &120_000_000i128);
}

fn make_proposal(c: &GovernanceContractClient<'_>, env: &Env, proposer: &Address) -> u64 {
    c.create_proposal(
        proposer,
        &ProposalType::SignalProposal(String::from_str(env, "test")),
        &String::from_str(env, "Title"),
        &String::from_str(env, "Description"),
        &Bytes::new(env),
        &ProposalCategory::General,
        &false,
    )
}

// ── Module 1: proposal lifecycle blocked by pause ────────────────────────────

/// Pausing the contract blocks proposal creation.
#[test]
fn paused_blocks_create_proposal() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    // Give the proposer some staked voting power
    stake_tokens(&c, &r.community_rewards, 10_000);

    // Pause
    c.set_contract_paused(&admin, &true);

    let result = c.try_create_proposal(
        &r.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "paused")),
        &String::from_str(&env, "T"),
        &String::from_str(&env, "D"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

/// Pausing the contract blocks vote casting.
#[test]
fn paused_blocks_cast_vote() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    stake_tokens(&c, &r.community_rewards, 10_000);

    // Create proposal while unpaused
    let proposal_id = make_proposal(&c, &env, &r.community_rewards);

    // Advance time so voting starts
    env.ledger().set_timestamp(1_100);

    // Pause
    c.set_contract_paused(&admin, &true);

    let voter = Address::generate(&env);
    let result = c.try_cast_vote(&proposal_id, &voter, &crate::proposals::VoteType::For);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

/// Pausing blocks proposal finalization.
#[test]
fn paused_blocks_finalize_proposal() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    stake_tokens(&c, &r.community_rewards, 10_000);
    let proposal_id = make_proposal(&c, &env, &r.community_rewards);

    // Advance past voting period
    env.ledger().set_timestamp(1_000 + 7 * 24 * 60 * 60 + 120);

    c.set_contract_paused(&admin, &true);

    let result = c.try_finalize_proposal(&proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

/// Pausing blocks explicit proposal execution.
#[test]
fn paused_blocks_execute_proposal() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    stake_tokens(&c, &r.community_rewards, 10_000);
    let proposal_id = make_proposal(&c, &env, &r.community_rewards);

    c.set_contract_paused(&admin, &true);

    let result = c.try_execute_proposal(&proposal_id, &r.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

// ── Module 2: staking blocked by pause ───────────────────────────────────────

/// Pausing blocks stake().
#[test]
fn paused_blocks_stake() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    c.set_contract_paused(&admin, &true);

    let result = c.try_stake(&r.community_rewards, &1_000i128);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

/// Pausing blocks unstake().
#[test]
fn paused_blocks_unstake() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    // Stake while unpaused so there is something to unstake
    stake_tokens(&c, &r.community_rewards, 5_000);

    c.set_contract_paused(&admin, &true);

    let result = c.try_unstake(&r.community_rewards, &5_000i128);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

// ── Module 3: timelock blocked by pause ──────────────────────────────────────

/// Pausing blocks queue_action().
#[test]
fn paused_blocks_queue_action() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    prevent_auto_execution(&c, &admin);

    // Set up a succeeded proposal so queue_action would otherwise work
    stake_for_quorum(&c, &r.community_rewards);
    let proposal_id = make_proposal(&c, &env, &r.community_rewards);

    // Advance past voting, cast a winning vote, finalize
    env.ledger().set_timestamp(1_100);
    c.cast_vote(
        &proposal_id,
        &r.community_rewards,
        &crate::proposals::VoteType::For,
    );
    env.ledger().set_timestamp(1_000 + 7 * 24 * 60 * 60 + 120);
    assert_eq!(c.finalize_proposal(&proposal_id), ProposalStatus::Succeeded);

    // Initialize timelock
    let guardian = Address::generate(&env);
    c.initialize_timelock(&admin, &3_600u64, &(7 * 86_400u64), &guardian);

    // Now pause
    c.set_contract_paused(&admin, &true);

    let result = c.try_queue_action(&proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

/// Pausing blocks execute_queued_action().
#[test]
fn paused_blocks_execute_queued_action() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    prevent_auto_execution(&c, &admin);

    // Build a queued action while unpaused
    stake_for_quorum(&c, &r.community_rewards);
    let proposal_id = make_proposal(&c, &env, &r.community_rewards);

    env.ledger().set_timestamp(1_100);
    c.cast_vote(
        &proposal_id,
        &r.community_rewards,
        &crate::proposals::VoteType::For,
    );
    env.ledger().set_timestamp(1_000 + 7 * 24 * 60 * 60 + 120);
    assert_eq!(c.finalize_proposal(&proposal_id), ProposalStatus::Succeeded);

    let guardian = Address::generate(&env);
    c.initialize_timelock(&admin, &3_600u64, &(7 * 86_400u64), &guardian);

    let action_id = c.queue_action(&proposal_id);

    // Advance past minimum timelock delay
    env.ledger()
        .set_timestamp(1_000 + 7 * 24 * 60 * 60 + 120 + 3_601);

    // Now pause
    c.set_contract_paused(&admin, &true);

    let result = c.try_execute_queued_action(&action_id, &r.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

/// Pausing blocks execute_multiple_actions().
#[test]
fn paused_blocks_execute_multiple_actions() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    c.set_contract_paused(&admin, &true);

    let ids: Vec<u64> = Vec::new(&env);
    let result = c.try_execute_multiple_actions(&ids, &r.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ContractPaused)));
}

// ── Module 4: read-only ops unaffected by pause ───────────────────────────────

/// health_check and balance reads work while paused.
#[test]
fn paused_does_not_block_reads() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    c.set_contract_paused(&admin, &true);

    // health_check reflects paused state
    let health = c.health_check();
    assert!(health.is_paused);
    assert!(health.is_initialized);

    // balance reads still work
    let bal = c.balance(&r.community_rewards);
    assert!(bal > 0);
}

// ── Module 5: unpause restores operations ─────────────────────────────────────

/// Unpausing re-enables proposal creation and staking (cross-module round-trip).
#[test]
fn unpause_restores_proposal_and_staking() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    // Pause, verify staking blocked
    c.set_contract_paused(&admin, &true);
    assert_eq!(
        c.try_stake(&r.community_rewards, &1_000i128),
        Err(Ok(GovernanceError::ContractPaused))
    );

    // Unpause
    c.set_contract_paused(&admin, &false);

    // Staking now works
    c.stake(&r.community_rewards, &1_000i128);
    assert_eq!(c.staked_balance(&r.community_rewards), 1_000);

    // Proposal creation now works
    let pid = make_proposal(&c, &env, &r.community_rewards);
    assert!(pid > 0);

    // health_check reflects unpaused
    let health = c.health_check();
    assert!(!health.is_paused);
}

/// Unpause restores timelock queuing after it was blocked.
#[test]
fn unpause_restores_timelock_queue() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    prevent_auto_execution(&c, &admin);

    // Build a succeeded proposal
    stake_for_quorum(&c, &r.community_rewards);
    let proposal_id = make_proposal(&c, &env, &r.community_rewards);
    env.ledger().set_timestamp(1_100);
    c.cast_vote(
        &proposal_id,
        &r.community_rewards,
        &crate::proposals::VoteType::For,
    );
    env.ledger().set_timestamp(1_000 + 7 * 24 * 60 * 60 + 120);
    assert_eq!(c.finalize_proposal(&proposal_id), ProposalStatus::Succeeded);

    let guardian = Address::generate(&env);
    c.initialize_timelock(&admin, &3_600u64, &(7 * 86_400u64), &guardian);

    // Pause, verify queue blocked
    c.set_contract_paused(&admin, &true);
    assert_eq!(
        c.try_queue_action(&proposal_id),
        Err(Ok(GovernanceError::ContractPaused))
    );

    // Unpause → queue_action now succeeds
    c.set_contract_paused(&admin, &false);
    let action_id = c.queue_action(&proposal_id);
    assert!(action_id > 0);
}
