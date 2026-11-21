//! Issue #1289: a single explicit deadline rule for votes and vote changes.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, String};

use crate::distribution::DistributionRecipients;
use crate::proposals::{ProposalCategory, ProposalType};
use crate::voting::require_voting_open;
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError, GovernanceVoteType};

const SUPPLY: i128 = 1_000_000_000;
const DELAY: u64 = 60;
const PERIOD: u64 = 7 * 86_400;

fn setup() -> (Env, Address, Address, DistributionRecipients) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
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

#[test]
fn votes_are_rejected_at_and_after_the_exact_deadline() {
    let (env, contract_id, admin, recipients) = setup();
    let c = GovernanceContractClient::new(&env, &contract_id);
    c.initialize(
        &admin,
        &String::from_str(&env, "Gov"),
        &String::from_str(&env, "GOV"),
        &7u32,
        &SUPPLY,
        &recipients,
    );
    let proposer = &recipients.community_rewards;
    c.stake(proposer, &120_000_000);
    let id = c.create_proposal(
        proposer,
        &ProposalType::FeatureToggle(String::from_str(&env, "flag"), true),
        &String::from_str(&env, "title"),
        &String::from_str(&env, "description"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    let deadline = DELAY + PERIOD;

    // One ledger before the deadline: accepted.
    env.ledger().set_timestamp(deadline - 1);
    c.cast_vote(&id, proposer, &GovernanceVoteType::For);
    let before = c.proposal(&id);
    assert_eq!(before.votes_for, 120_000_000);
    assert_eq!(before.voter_list.len(), 1);

    // Exactly at the deadline: rejected, with no tally or voter changes.
    let late_voter = &recipients.public_sale;
    c.stake(late_voter, &10_000);
    env.ledger().set_timestamp(deadline);
    assert_eq!(
        c.try_cast_vote(&id, late_voter, &GovernanceVoteType::Against),
        Err(Ok(GovernanceError::VotingEnded))
    );
    // One ledger after the deadline: also rejected.
    env.ledger().set_timestamp(deadline + 1);
    assert_eq!(
        c.try_cast_vote(&id, late_voter, &GovernanceVoteType::Abstain),
        Err(Ok(GovernanceError::VotingEnded))
    );
    let after = c.proposal(&id);
    assert_eq!(after.votes_for, before.votes_for);
    assert_eq!(after.votes_against, before.votes_against);
    assert_eq!(after.votes_abstain, before.votes_abstain);
    assert_eq!(after.voter_list.len(), before.voter_list.len());
    assert!(!after.voters.contains_key(late_voter.clone()));
}

#[test]
fn shared_deadline_rule_applies_to_vote_replacements() {
    let (env, _contract_id, _admin, _recipients) = setup();
    // The helper is the single source of truth used by submission and
    // replacement paths (quadratic reallocation, conviction vote changes).
    let proposal = crate::proposals::Proposal {
        id: 1,
        proposer: Address::generate(&env),
        proposal_type: ProposalType::SignalProposal(String::from_str(&env, "msg")),
        title: String::from_str(&env, "t"),
        description: String::from_str(&env, "d"),
        execution_payload: Bytes::new(&env),
        voting_starts: 100,
        voting_ends: 200,
        votes_for: 0,
        votes_against: 0,
        votes_abstain: 0,
        status: crate::proposals::ProposalStatus::Active,
        voters: soroban_sdk::Map::new(&env),
        voter_list: soroban_sdk::Vec::new(&env),
        executed_at: None,
        veto_ends_at: 200,
        vetoed_by: None,
        discussion_ends_at: 0,
        category: ProposalCategory::General,
        use_quadratic_voting: false,
        quadratic_total_supply: 0,
        execution_deadline: 1_000,
    };
    env.ledger().set_timestamp(199);
    assert!(require_voting_open(&env, &proposal).is_ok());
    env.ledger().set_timestamp(200);
    assert_eq!(
        require_voting_open(&env, &proposal),
        Err(GovernanceError::VotingEnded)
    );
    env.ledger().set_timestamp(201);
    assert_eq!(
        require_voting_open(&env, &proposal),
        Err(GovernanceError::VotingEnded)
    );
}
