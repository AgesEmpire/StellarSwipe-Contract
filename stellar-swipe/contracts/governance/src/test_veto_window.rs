//! Issue #1287: explicit guardian veto window.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, String};

use crate::distribution::DistributionRecipients;
use crate::proposals::{GovernanceConfig, ProposalCategory, ProposalStatus, ProposalType};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError, GovernanceVoteType};

const SUPPLY: i128 = 1_000_000_000;
const VOTING_DELAY: u64 = 60;
const VOTING_PERIOD: u64 = 7 * 86_400;
const VETO_WINDOW: u64 = 1_000;

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

fn config() -> GovernanceConfig {
    GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: VOTING_PERIOD,
        voting_delay: VOTING_DELAY,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 0,
        discussion_duration: 0,
        veto_window: VETO_WINDOW,
    }
}

fn passed_proposal(env: &Env, c: &GovernanceContractClient<'_>, proposer: &Address) -> u64 {
    c.stake(proposer, &120_000_000);
    let id = c.create_proposal(
        proposer,
        &ProposalType::FeatureToggle(String::from_str(env, "flag"), true),
        &String::from_str(env, "title"),
        &String::from_str(env, "description"),
        &Bytes::new(env),
        &ProposalCategory::General,
        &false,
    );
    env.ledger().set_timestamp(VOTING_DELAY + 1);
    c.cast_vote(&id, proposer, &GovernanceVoteType::For);
    env.ledger().set_timestamp(VOTING_DELAY + VOTING_PERIOD);
    assert_eq!(c.finalize_proposal(&id), ProposalStatus::Succeeded);
    id
}

#[test]
fn veto_window_is_recorded_and_guardian_can_veto_during_it() {
    let (env, contract_id, admin, recipients) = setup();
    let c = client(&env, &contract_id);
    c.initialize(
        &admin,
        &String::from_str(&env, "Gov"),
        &String::from_str(&env, "GOV"),
        &7u32,
        &SUPPLY,
        &recipients,
    );
    c.configure_governance(&admin, &config());
    let guardian = Address::generate(&env);
    c.set_guardian(&admin, &guardian);

    let id = passed_proposal(&env, &c, &recipients.community_rewards);
    let expected_end = VOTING_DELAY + VOTING_PERIOD + VETO_WINDOW;
    assert_eq!(c.proposal(&id).veto_ends_at, expected_end);

    // Execution is rejected while the window is open (including its first ledger).
    assert_eq!(
        c.try_execute_proposal(&id, &admin),
        Err(Ok(GovernanceError::VetoWindowOpen))
    );
    env.ledger().set_timestamp(expected_end - 1);
    assert_eq!(c.veto_proposal(&id, &guardian), ProposalStatus::Vetoed);
    assert_eq!(c.proposal(&id).status, ProposalStatus::Vetoed);
    // A vetoed proposal can never execute, even after the window closes.
    env.ledger().set_timestamp(expected_end + 1);
    assert_eq!(
        c.try_execute_proposal(&id, &admin),
        Err(Ok(GovernanceError::ProposalVetoed))
    );
}

#[test]
fn veto_before_during_and_after_window_and_authorization() {
    let (env, contract_id, admin, recipients) = setup();
    let c = client(&env, &contract_id);
    c.initialize(
        &admin,
        &String::from_str(&env, "Gov"),
        &String::from_str(&env, "GOV"),
        &7u32,
        &SUPPLY,
        &recipients,
    );
    c.configure_governance(&admin, &config());
    let guardian = Address::generate(&env);
    let other = Address::generate(&env);
    c.set_guardian(&admin, &guardian);

    let id = passed_proposal(&env, &c, &recipients.community_rewards);
    let end = VOTING_DELAY + VOTING_PERIOD + VETO_WINDOW;

    env.ledger().set_timestamp(VOTING_DELAY + VOTING_PERIOD - 1);
    assert_eq!(
        c.try_veto_proposal(&id, &guardian),
        Err(Ok(GovernanceError::VetoWindowOpen))
    );
    env.ledger().set_timestamp(end - 1);
    assert_eq!(
        c.try_veto_proposal(&id, &other),
        Err(Ok(GovernanceError::NotGuardian))
    );
    // At the boundary the window has closed: veto rejected, execution allowed.
    env.ledger().set_timestamp(end);
    assert_eq!(
        c.try_veto_proposal(&id, &guardian),
        Err(Ok(GovernanceError::VetoWindowOpen))
    );
    assert_eq!(c.execute_proposal(&id, &admin), ProposalStatus::Executed);
}
