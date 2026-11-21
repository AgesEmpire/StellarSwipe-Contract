//! Issue #1431: revision rules for proposals during active voting.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, String};

use crate::distribution::DistributionRecipients;
use crate::proposal_revisions::{
    ProposalAmendment, INITIAL_REVISION_VERSION, MAX_PROPOSAL_REVISIONS, MIN_REVOTE_WINDOW_SECS,
};
use crate::proposals::{ProposalCategory, ProposalType};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError, GovernanceVoteType};

const SUPPLY: i128 = 1_000_000_000;
const DELAY: u64 = 60;
const PERIOD: u64 = 7 * 86_400;

struct Ctx {
    env: Env,
    contract_id: Address,
    recipients: DistributionRecipients,
    id: u64,
}

fn setup() -> Ctx {
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
    let c = GovernanceContractClient::new(&env, &contract_id);
    c.initialize(
        &admin,
        &String::from_str(&env, "Gov"),
        &String::from_str(&env, "GOV"),
        &7u32,
        &SUPPLY,
        &recipients,
    );
    c.stake(&recipients.community_rewards, &120_000_000);
    c.stake(&recipients.public_sale, &10_000);
    let id = c.create_proposal(
        &recipients.community_rewards,
        &ProposalType::FeatureToggle(String::from_str(&env, "flag"), true),
        &String::from_str(&env, "title"),
        &String::from_str(&env, "description"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    Ctx {
        env,
        contract_id,
        recipients,
        id,
    }
}

fn client(ctx: &Ctx) -> GovernanceContractClient<'_> {
    GovernanceContractClient::new(&ctx.env, &ctx.contract_id)
}

#[test]
fn immutable_fields_are_rejected_after_voting_starts() {
    let ctx = setup();
    let c = client(&ctx);
    let env = &ctx.env;
    let proposer = &ctx.recipients.community_rewards;
    env.ledger().set_timestamp(DELAY);
    c.cast_vote(&ctx.id, proposer, &GovernanceVoteType::For);
    let before = c.proposal(&ctx.id);

    let attempts = [
        ProposalAmendment::ProposalType(ProposalType::FeatureToggle(
            String::from_str(env, "flag"),
            false,
        )),
        ProposalAmendment::ExecutionPayload(Bytes::from_slice(env, &[1, 2, 3])),
        ProposalAmendment::Category(ProposalCategory::ContractUpgrade),
        ProposalAmendment::QuadraticVoting(true),
        ProposalAmendment::VotingWindow(DELAY, DELAY + 2 * PERIOD),
    ];
    for amendment in attempts.iter() {
        assert_eq!(
            c.try_revise_proposal(&ctx.id, proposer, amendment),
            Err(Ok(GovernanceError::ImmutableProposalField))
        );
    }

    // Nothing changed: same proposal, same votes, same version.
    assert_eq!(c.proposal(&ctx.id), before);
    assert_eq!(
        c.get_proposal_revision_version(&ctx.id),
        INITIAL_REVISION_VERSION
    );
    assert!(c.get_proposal_revision(&ctx.id).is_none());
}

#[test]
fn mutable_field_revision_bumps_version_and_resets_votes() {
    let ctx = setup();
    let c = client(&ctx);
    let env = &ctx.env;
    let proposer = &ctx.recipients.community_rewards;
    let voter = &ctx.recipients.public_sale;
    env.ledger().set_timestamp(DELAY);
    c.cast_vote(&ctx.id, proposer, &GovernanceVoteType::For);
    c.cast_vote(&ctx.id, voter, &GovernanceVoteType::Against);
    let before = c.proposal(&ctx.id);
    assert_eq!(before.voter_list.len(), 2);

    let new_description = String::from_str(env, "clarified description");
    let version = c.revise_proposal(
        &ctx.id,
        proposer,
        &ProposalAmendment::Description(new_description.clone()),
    );
    assert_eq!(version, INITIAL_REVISION_VERSION + 1);

    let after = c.proposal(&ctx.id);
    assert_eq!(after.description, new_description);
    assert_eq!(after.votes_for, 0);
    assert_eq!(after.votes_against, 0);
    assert_eq!(after.votes_abstain, 0);
    assert_eq!(after.voter_list.len(), 0);
    // Immutable fields are untouched.
    assert_eq!(after.proposal_type, before.proposal_type);
    assert_eq!(after.execution_payload, before.execution_payload);
    assert_eq!(after.voting_ends, before.voting_ends);
    let rev = c.get_proposal_revision(&ctx.id).unwrap();
    assert_eq!(rev.version, version);
    assert_eq!(rev.vote_resets, 1);

    // Snapshot preserved: voters re-vote with the same power as before.
    c.cast_vote(&ctx.id, proposer, &GovernanceVoteType::For);
    c.cast_vote(&ctx.id, voter, &GovernanceVoteType::Against);
    let revoted = c.proposal(&ctx.id);
    assert_eq!(revoted.votes_for, before.votes_for);
    assert_eq!(revoted.votes_against, before.votes_against);

    // Title is also mutable.
    let v2 = c.revise_proposal(
        &ctx.id,
        proposer,
        &ProposalAmendment::Title(String::from_str(env, "better title")),
    );
    assert_eq!(v2, version + 1);
}

#[test]
fn revision_before_any_vote_does_not_reset() {
    let ctx = setup();
    let c = client(&ctx);
    let proposer = &ctx.recipients.community_rewards;
    c.revise_proposal(
        &ctx.id,
        proposer,
        &ProposalAmendment::Title(String::from_str(&ctx.env, "new title")),
    );
    assert_eq!(c.get_proposal_revision(&ctx.id).unwrap().vote_resets, 0);
}

#[test]
fn revision_rules_reject_invalid_attempts() {
    let ctx = setup();
    let c = client(&ctx);
    let env = &ctx.env;
    let proposer = &ctx.recipients.community_rewards;

    // Only the proposer may revise.
    assert_eq!(
        c.try_revise_proposal(
            &ctx.id,
            &ctx.recipients.public_sale,
            &ProposalAmendment::Title(String::from_str(env, "hijack")),
        ),
        Err(Ok(GovernanceError::Unauthorized))
    );
    // Unchanged content is not a revision.
    assert_eq!(
        c.try_revise_proposal(
            &ctx.id,
            proposer,
            &ProposalAmendment::Title(String::from_str(env, "title")),
        ),
        Err(Ok(GovernanceError::InvalidProposal))
    );

    // A vote reset needs a realistic re-vote window.
    env.ledger().set_timestamp(DELAY);
    c.cast_vote(&ctx.id, proposer, &GovernanceVoteType::For);
    env.ledger()
        .set_timestamp(DELAY + PERIOD - MIN_REVOTE_WINDOW_SECS + 1);
    assert_eq!(
        c.try_revise_proposal(
            &ctx.id,
            proposer,
            &ProposalAmendment::Description(String::from_str(env, "late change")),
        ),
        Err(Ok(GovernanceError::RevisionWindowClosed))
    );
    assert_eq!(c.proposal(&ctx.id).voter_list.len(), 1);

    // No revisions once voting has ended.
    env.ledger().set_timestamp(DELAY + PERIOD);
    assert_eq!(
        c.try_revise_proposal(
            &ctx.id,
            proposer,
            &ProposalAmendment::Description(String::from_str(env, "too late")),
        ),
        Err(Ok(GovernanceError::VotingEnded))
    );
}

#[test]
fn revision_count_is_bounded() {
    let ctx = setup();
    let c = client(&ctx);
    let env = &ctx.env;
    let proposer = &ctx.recipients.community_rewards;
    let titles = ["t1", "t2", "t3", "t4"];
    for title in titles.iter().take(MAX_PROPOSAL_REVISIONS as usize) {
        c.revise_proposal(
            &ctx.id,
            proposer,
            &ProposalAmendment::Title(String::from_str(env, title)),
        );
    }
    assert_eq!(
        c.get_proposal_revision_version(&ctx.id),
        INITIAL_REVISION_VERSION + MAX_PROPOSAL_REVISIONS
    );
    assert_eq!(
        c.try_revise_proposal(
            &ctx.id,
            proposer,
            &ProposalAmendment::Title(String::from_str(env, "overflow")),
        ),
        Err(Ok(GovernanceError::RevisionLimitReached))
    );
}
