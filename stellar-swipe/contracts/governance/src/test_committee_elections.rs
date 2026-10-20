/// Committee-election test suite.
///
/// Covers:
/// - quorum failure (participation count)
/// - quorum failure (stake-weight threshold)
/// - invalid vote rejection (un-nominated candidate)
/// - invalid vote rejection (unstaked voter)
/// - duplicate-vote rejection (AlreadyVoted)
/// - invalid election config (bad positions, zero duration, negative threshold)
/// - post-failure state reset (new election can be started immediately)
/// - finalization before deadline is rejected
/// - successful election with all quorum params set
extern crate std;

use crate::committees::ElectionStatus;
use crate::distribution::DistributionRecipients;
use crate::{
    Authority, CommitteeElectionStatus, EmergencyActionAuthority, GovernanceContract,
    GovernanceContractClient, GovernanceError,
};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String, Vec};

// ─── helpers ───────────────────────────────────────────────────────────────

const SUPPLY: i128 = 1_000_000_000;

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

fn client<'a>(env: &'a Env, contract_id: &'a Address) -> GovernanceContractClient<'a> {
    GovernanceContractClient::new(env, contract_id)
}

fn fund_user(env: &Env, contract_id: &Address, user: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        crate::add_balance(env, user, amount).unwrap();
    });
}

fn initialize(
    client: &GovernanceContractClient<'_>,
    env: &Env,
    admin: &Address,
    recipients: &DistributionRecipients,
) {
    client.initialize(
        admin,
        &String::from_str(env, "StellarSwipe Gov"),
        &String::from_str(env, "SSG"),
        &7u32,
        &SUPPLY,
        recipients,
    );
}

/// Create a minimal committee and return its id.
fn create_test_committee(client: &GovernanceContractClient<'_>, env: &Env, admin: &Address) -> u64 {
    let members: Vec<Address> = {
        let mut v = Vec::new(env);
        for _ in 0..5u32 {
            v.push_back(Address::generate(env));
        }
        v
    };
    let chair = members.get(0).unwrap();
    let authorities = soroban_sdk::vec![
        env,
        Authority::EmergencyAction(EmergencyActionAuthority {
            action_types: soroban_sdk::vec![env, String::from_str(env, "incident")],
        })
    ];
    client
        .create_committee(
            admin,
            &String::from_str(env, "Test Committee"),
            &String::from_str(env, "Used in election tests"),
            &members,
            &chair,
            &5u32,
            &authorities,
            &Some(90u32),
        )
        .id
}

// ─── config-validation tests ────────────────────────────────────────────────

/// `positions_available < 3` must be rejected with `InvalidCommitteeConfig`.
#[test]
fn reject_election_with_too_few_positions() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    let committee_id = create_test_committee(&client, &env, &admin);

    let result = client.try_start_committee_election(
        &admin,
        &committee_id,
        &2u32, // < 3 — invalid
        &7u32,
        &0u32,
        &0i128,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCommitteeConfig)));
}

/// `duration_days == 0` must be rejected with `InvalidCommitteeConfig`.
#[test]
fn reject_election_with_zero_duration() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    let committee_id = create_test_committee(&client, &env, &admin);

    let result = client.try_start_committee_election(
        &admin,
        &committee_id,
        &3u32,
        &0u32, // zero duration — invalid
        &0u32,
        &0i128,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCommitteeConfig)));
}

/// Negative `quorum_stake_threshold` must be rejected with `InvalidCommitteeConfig`.
#[test]
fn reject_election_with_negative_quorum_stake_threshold() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    let committee_id = create_test_committee(&client, &env, &admin);

    let result = client.try_start_committee_election(
        &admin,
        &committee_id,
        &3u32,
        &7u32,
        &0u32,
        &-1i128, // negative — invalid
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCommitteeConfig)));
}

// ─── invalid-vote tests ─────────────────────────────────────────────────────

/// Casting a vote for a candidate who was never nominated returns
/// `InvalidElectionVote` and does NOT advance `election.votes`.
#[test]
fn reject_vote_for_unnominated_candidate() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Give the voter some stake
    client.stake(&recipients.community_rewards, &10_000_000);

    let committee_id = create_test_committee(&client, &env, &admin);
    client.start_committee_election(&admin, &committee_id, &3u32, &7u32, &0u32, &0i128);

    // Nominate candidate_one but vote for an address that was never nominated
    let candidate_one = Address::generate(&env);
    let phantom_candidate = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &candidate_one, &recipients.community_rewards);

    let result = client.try_vote_in_committee_election(
        &committee_id,
        &recipients.community_rewards,
        &phantom_candidate, // not on ballot
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidElectionVote)));

    // Verify: election votes map must still be empty
    let election = client.committee_election(&committee_id);
    assert_eq!(
        election.votes.len(),
        0,
        "invalid ballot must not be recorded in election.votes"
    );
}

/// A voter with zero staked balance is rejected with `InvalidElectionVote`
/// and the election state remains unmodified.
#[test]
fn reject_vote_from_unstaked_voter() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Stake so we can nominate
    client.stake(&recipients.community_rewards, &10_000_000);

    let committee_id = create_test_committee(&client, &env, &admin);
    client.start_committee_election(&admin, &committee_id, &3u32, &7u32, &0u32, &0i128);

    let candidate_one = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &candidate_one, &recipients.community_rewards);

    // unstaked_voter has no staked balance at all
    let unstaked_voter = Address::generate(&env);
    let result =
        client.try_vote_in_committee_election(&committee_id, &unstaked_voter, &candidate_one);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidElectionVote)));

    let election = client.committee_election(&committee_id);
    assert_eq!(
        election.votes.len(),
        0,
        "ballot from unstaked voter must not be recorded"
    );
}

/// A voter who has already cast a ballot receives `AlreadyVoted` and the
/// earlier ballot is preserved unchanged.
#[test]
fn reject_duplicate_vote_in_election() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &10_000_000);

    let committee_id = create_test_committee(&client, &env, &admin);
    client.start_committee_election(&admin, &committee_id, &3u32, &7u32, &0u32, &0i128);

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee_id, &candidate_two, &recipients.community_rewards);

    // First vote succeeds
    client.vote_in_committee_election(&committee_id, &recipients.community_rewards, &candidate_one);

    // Second vote must be rejected
    let result = client.try_vote_in_committee_election(
        &committee_id,
        &recipients.community_rewards,
        &candidate_two,
    );
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyVoted)));

    // Still exactly one ballot recorded
    let election = client.committee_election(&committee_id);
    assert_eq!(election.votes.len(), 1);
}

// ─── finalization-before-deadline test ──────────────────────────────────────

/// Calling `finalize_committee_election` before the deadline is rejected with
/// `CommitteeElectionNotActive`.
#[test]
fn reject_finalize_before_deadline() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let committee_id = create_test_committee(&client, &env, &admin);
    client.start_committee_election(&admin, &committee_id, &3u32, &7u32, &0u32, &0i128);

    // Advance only 3 days — election lasts 7
    env.ledger().set_timestamp(3 * 86_400);

    let result = client.try_finalize_committee_election(&admin, &committee_id);
    assert_eq!(result, Err(Ok(GovernanceError::CommitteeElectionNotActive)));
}

// ─── quorum-failure tests ───────────────────────────────────────────────────

/// When fewer unique valid voters participate than `min_participation`, the
/// election must return `FailedQuorumParticipation`, leave the committee
/// membership unchanged, and clear the election record so a new one can start.
#[test]
fn election_fails_when_participation_quorum_not_met() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Three voters, each staked
    client.stake(&recipients.community_rewards, &50_000_000);
    client.stake(&recipients.public_sale, &30_000_000);
    client.stake(&recipients.treasury, &20_000_000);

    let committee_id = create_test_committee(&client, &env, &admin);
    // Require 5 unique voters, but we only have 3
    client.start_committee_election(
        &admin,
        &committee_id,
        &3u32,
        &7u32,
        &5u32,  // min_participation = 5
        &0i128, // stake quorum disabled
    );

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee_id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee_id, &candidate_three, &recipients.treasury);

    // Only 3 votes cast — below min_participation
    client.vote_in_committee_election(&committee_id, &recipients.community_rewards, &candidate_one);
    client.vote_in_committee_election(&committee_id, &recipients.public_sale, &candidate_two);
    client.vote_in_committee_election(&committee_id, &recipients.treasury, &candidate_three);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee_id);

    assert_eq!(
        result.status,
        CommitteeElectionStatus::FailedQuorumParticipation,
        "expected FailedQuorumParticipation"
    );
    assert_eq!(result.winners.len(), 0, "no winners on quorum failure");
    assert_eq!(result.valid_votes, 3, "3 valid votes counted");
    assert_eq!(result.rejected_votes, 0, "no rejections");

    // Committee membership must be unchanged (5 original members)
    let committee = client.committee(&committee_id);
    assert_eq!(
        committee.members.len(),
        5,
        "committee members must be unchanged after quorum failure"
    );

    // Election record must be cleared — a new election can start immediately
    let new_election =
        client.start_committee_election(&admin, &committee_id, &3u32, &7u32, &1u32, &0i128);
    assert!(
        new_election.election_end > new_election.election_start,
        "new election should be schedulable after a failed one"
    );
}

/// When valid votes accumulate insufficient total stake weight, the election
/// returns `FailedQuorumStake` and keeps the existing committee intact.
#[test]
fn election_fails_when_stake_quorum_not_met() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Very small stakes so total weight stays below our threshold
    client.stake(&recipients.community_rewards, &1_000);
    client.stake(&recipients.public_sale, &1_000);
    client.stake(&recipients.treasury, &1_000);

    let committee_id = create_test_committee(&client, &env, &admin);
    client.start_committee_election(
        &admin,
        &committee_id,
        &3u32,
        &7u32,
        &1u32,          // participation quorum met (1 voter)
        &1_000_000i128, // stake quorum NOT met (total will be 3_000)
    );

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee_id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee_id, &candidate_three, &recipients.treasury);

    client.vote_in_committee_election(&committee_id, &recipients.community_rewards, &candidate_one);
    client.vote_in_committee_election(&committee_id, &recipients.public_sale, &candidate_two);
    client.vote_in_committee_election(&committee_id, &recipients.treasury, &candidate_three);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee_id);

    assert_eq!(
        result.status,
        CommitteeElectionStatus::FailedQuorumStake,
        "expected FailedQuorumStake"
    );
    assert_eq!(result.winners.len(), 0);
    assert_eq!(result.valid_votes, 3);
    // Total stake weight = 3 × 1_000 = 3_000 — returned for diagnostics
    assert_eq!(result.total_stake_weight, 3_000);

    let committee = client.committee(&committee_id);
    assert_eq!(
        committee.members.len(),
        5,
        "committee members must be unchanged after stake-quorum failure"
    );
}

// ─── invalid-at-finalization test ───────────────────────────────────────────

/// Votes cast by an address that unstakes between voting and finalization are
/// counted as `rejected_votes` and excluded from quorum math.
#[test]
fn finalization_excludes_votes_from_voters_who_unstaked() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // voter_a stays staked; voter_b unstakes before finalization
    client.stake(&recipients.community_rewards, &50_000_000); // voter_a
    client.stake(&recipients.public_sale, &20_000_000); // voter_b (will unstake)
    client.stake(&recipients.treasury, &30_000_000); // voter_c

    let committee_id = create_test_committee(&client, &env, &admin);
    // Require 3 valid voters (only 2 will be valid at finalization)
    client.start_committee_election(
        &admin,
        &committee_id,
        &3u32,
        &7u32,
        &3u32, // min_participation = 3; voter_b's vote becomes invalid post-unstake
        &0i128,
    );

    let c1 = Address::generate(&env);
    let c2 = Address::generate(&env);
    let c3 = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &c1, &recipients.community_rewards);
    client.nominate_for_committee(&committee_id, &c2, &recipients.public_sale);
    client.nominate_for_committee(&committee_id, &c3, &recipients.treasury);

    client.vote_in_committee_election(&committee_id, &recipients.community_rewards, &c1);
    client.vote_in_committee_election(&committee_id, &recipients.public_sale, &c2); // will be invalidated
    client.vote_in_committee_election(&committee_id, &recipients.treasury, &c3);

    // voter_b fully unstakes before the election ends
    client.unstake(&recipients.public_sale, &20_000_000);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee_id);

    // Only 2 valid votes after voter_b's unstake — quorum of 3 not met
    assert_eq!(
        result.status,
        CommitteeElectionStatus::FailedQuorumParticipation
    );
    assert_eq!(result.valid_votes, 2, "voter_b's ballot must be excluded");
    assert_eq!(
        result.rejected_votes, 1,
        "voter_b's ballot must appear in rejected_votes"
    );
}

// ─── successful election test ────────────────────────────────────────────────

/// A fully valid election with all quorum thresholds set succeeds: winners are
/// installed, chair is updated, and `ElectionResult` fields are correct.
#[test]
fn successful_election_with_quorum_checks_installs_winners() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Four staked voters; total weight = 200_000_000
    client.stake(&recipients.community_rewards, &100_000_000);
    client.stake(&recipients.public_sale, &50_000_000);
    client.stake(&recipients.treasury, &30_000_000);
    fund_user(&env, &contract_id, &recipients.team, 20_000_000);
    client.stake(&recipients.team, &20_000_000);

    let committee_id = create_test_committee(&client, &env, &admin);
    client.start_committee_election(
        &admin,
        &committee_id,
        &3u32,
        &7u32,
        &3u32,           // min_participation: need at least 3 voters
        &50_000_000i128, // stake quorum: at least 50M staked weight
    );

    let c1 = Address::generate(&env);
    let c2 = Address::generate(&env);
    let c3 = Address::generate(&env);
    client.nominate_for_committee(&committee_id, &c1, &recipients.community_rewards);
    client.nominate_for_committee(&committee_id, &c2, &recipients.public_sale);
    client.nominate_for_committee(&committee_id, &c3, &recipients.treasury);

    // c1 gets 3 votes (highest), c2 gets 1 vote, c3 gets 1 vote
    client.vote_in_committee_election(&committee_id, &recipients.community_rewards, &c1);
    client.vote_in_committee_election(&committee_id, &recipients.public_sale, &c1);
    client.vote_in_committee_election(&committee_id, &recipients.treasury, &c2);
    client.vote_in_committee_election(&committee_id, &recipients.team, &c3);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee_id);

    assert_eq!(
        result.status,
        CommitteeElectionStatus::Succeeded,
        "election must succeed when both quorum thresholds are met"
    );
    assert_eq!(result.winners.len(), 3);
    assert_eq!(
        result.winners.get(0).unwrap(),
        c1,
        "highest-stake-weighted candidate should be first winner"
    );
    assert_eq!(result.valid_votes, 4);
    assert_eq!(result.rejected_votes, 0);
    // total_stake_weight >= 50_000_000 threshold
    assert!(result.total_stake_weight >= 50_000_000);

    // Committee is updated with new members and chair
    let updated = client.committee(&committee_id);
    assert_eq!(updated.members.len(), 3);
    assert_eq!(updated.chair, c1, "top-voted candidate becomes chair");
}
