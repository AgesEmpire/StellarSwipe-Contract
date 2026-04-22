extern crate std;

use crate::distribution::{
    DistributionRecipients, EARLY_INVESTOR_VESTING_DURATION, TEAM_CLIFF_DURATION,
    TEAM_VESTING_DURATION, YEAR_SECONDS,
};
use crate::{
    Authority, CommitteeAction, CrossCommitteeStatus, DecisionStatus, EmergencyActionAuthority,
    EmergencyActionPayload, GovernanceContract, GovernanceContractClient, GovernanceError,
    ParameterAdjustmentAuthority, RewardConfigUpdateAction, TreasurySpendAction,
    TreasurySpendAuthority, VoteType,
};
use crate::proposals::{GovernanceConfig, ProposalStatus, ProposalType, VoteType as GovernanceVoteType};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, Map, String, Vec};
use stellar_swipe_common::Asset;

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

fn asset(env: &Env, code: &str) -> Asset {
    Asset {
        code: String::from_str(env, code),
        issuer: None,
    }
}

fn members(env: &Env, count: u32) -> Vec<Address> {
    let mut members = Vec::new(env);
    let mut index = 0;
    while index < count {
        members.push_back(Address::generate(env));
        index += 1;
    }
    members
}

#[test]
fn initialize_governance_token_with_valid_total_supply() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let metadata = client.get_metadata();
    assert_eq!(metadata.total_supply, SUPPLY);
    assert_eq!(metadata.decimals, 7);
}

#[test]
fn reject_zero_invalid_total_supply() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);

    let result = client.try_initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &0i128,
        &recipients,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidSupply)));
}

#[test]
fn allocate_initial_distribution_correctly_from_one_billion_supply() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let distribution = client.distribution();
    assert_eq!(distribution.allocation.team, 200_000_000);
    assert_eq!(distribution.allocation.early_investors, 150_000_000);
    assert_eq!(distribution.allocation.community_rewards, 300_000_000);
    assert_eq!(distribution.allocation.liquidity_mining, 200_000_000);
    assert_eq!(distribution.allocation.treasury, 100_000_000);
    assert_eq!(distribution.allocation.public_sale, 50_000_000);
    assert_eq!(client.balance(&recipients.community_rewards), 300_000_000);
    assert_eq!(client.balance(&recipients.treasury), 100_000_000);
    assert_eq!(client.balance(&recipients.public_sale), 50_000_000);
}

#[test]
fn create_team_vesting_schedule() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let schedule = client.get_vesting_schedule(&recipients.team);
    assert_eq!(schedule.total_amount, 200_000_000);
    assert_eq!(schedule.cliff_seconds, TEAM_CLIFF_DURATION);
    assert_eq!(schedule.duration_seconds, TEAM_VESTING_DURATION);
}

#[test]
fn enforce_cliff_before_release() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    env.ledger().set_timestamp(TEAM_CLIFF_DURATION - 1);

    let result = client.try_release_vested_tokens(&recipients.team);
    assert_eq!(result, Err(Ok(GovernanceError::CliffNotReached)));
}

#[test]
fn release_vested_tokens_after_cliff_over_time() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    env.ledger()
        .set_timestamp(TEAM_CLIFF_DURATION + (YEAR_SECONDS / 2));

    let released = client.release_vested_tokens(&recipients.team);
    assert_eq!(released, 33_333_333);
    assert_eq!(client.balance(&recipients.team), released);
}

#[test]
fn full_vesting_release_at_end_of_duration() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    env.ledger().set_timestamp(TEAM_VESTING_DURATION);

    let released = client.release_vested_tokens(&recipients.team);
    assert_eq!(released, 200_000_000);
    assert_eq!(client.balance(&recipients.team), 200_000_000);
}

#[test]
fn stake_tokens_updates_balances_and_voting_power() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &50_000_000);
    assert_eq!(client.balance(&recipients.community_rewards), 250_000_000);
    assert_eq!(
        client.staked_balance(&recipients.community_rewards),
        50_000_000
    );
    assert_eq!(
        client.voting_power(&recipients.community_rewards),
        50_000_000
    );
}

#[test]
fn unstake_fails_with_insufficient_staked_balance() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let result = client.try_unstake(&recipients.community_rewards, &1i128);
    assert_eq!(result, Err(Ok(GovernanceError::InsufficientStakedBalance)));
}

#[test]
fn accrue_liquidity_mining_rewards() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let reward = client.accrue_liquidity_rewards(&admin, &recipients.public_sale, &50_000);
    assert_eq!(reward, 500);
    assert_eq!(client.pending_rewards(&recipients.public_sale), 500);
}

#[test]
fn claim_liquidity_mining_rewards() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.accrue_liquidity_rewards(&admin, &recipients.public_sale, &50_000);
    let claimed = client.claim_liquidity_rewards(&recipients.public_sale);
    assert_eq!(claimed, 500);
    assert_eq!(client.pending_rewards(&recipients.public_sale), 0);
    assert_eq!(client.balance(&recipients.public_sale), 50_000_500);
}

#[test]
fn analytics_returns_sane_stats() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    client.stake(&recipients.community_rewards, &100_000_000);
    client.accrue_liquidity_rewards(&admin, &recipients.public_sale, &100_000);
    client.claim_liquidity_rewards(&recipients.public_sale);

    let analytics = client.analytics(&3);
    assert_eq!(analytics.total_holders, 3);
    assert_eq!(analytics.total_staked, 100_000_000);
    assert!(analytics.staking_ratio_bps > 0);
    assert_eq!(analytics.top_holders.len(), 3);
}

#[test]
fn edge_cases_duplicate_schedules_zero_amount_and_over_claim_are_covered() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let duplicate =
        client.try_create_vesting_schedule(&admin, &recipients.team, &10i128, &0u64, &0u64, &10u64);
    assert_eq!(duplicate, Err(Ok(GovernanceError::DuplicateSchedule)));

    let zero_amount = client.try_stake(&recipients.community_rewards, &0i128);
    assert_eq!(zero_amount, Err(Ok(GovernanceError::InvalidAmount)));

    let reward = client.accrue_liquidity_rewards(&admin, &recipients.public_sale, &1_000);
    assert_eq!(reward, 10);
    let below_threshold = client.try_claim_liquidity_rewards(&recipients.public_sale);
    assert_eq!(below_threshold, Err(Ok(GovernanceError::BelowMinimumClaim)));

    env.ledger().set_timestamp(TEAM_CLIFF_DURATION + 1);
    let first_release = client.release_vested_tokens(&recipients.team);
    assert!(first_release > 0);
    let second_release = client.try_release_vested_tokens(&recipients.team);
    assert_eq!(second_release, Err(Ok(GovernanceError::NothingToRelease)));
}

#[test]
fn early_investor_vesting_releases_fully_at_end() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let schedule = client.get_vesting_schedule(&recipients.early_investors);
    assert_eq!(schedule.duration_seconds, EARLY_INVESTOR_VESTING_DURATION);

    env.ledger().set_timestamp(EARLY_INVESTOR_VESTING_DURATION);
    let released = client.release_vested_tokens(&recipients.early_investors);
    assert_eq!(released, 150_000_000);
}

#[test]
fn active_vote_lock_blocks_unstake() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    client.stake(&recipients.community_rewards, &10_000);
    client.set_vote_lock(&admin, &recipients.community_rewards, &1);

    let result = client.try_unstake(&recipients.community_rewards, &1_000);
    assert_eq!(result, Err(Ok(GovernanceError::ActiveVoteLock)));
}

#[test]
fn treasury_spend_updates_budget_balances_and_history() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    client.set_treasury_asset(&admin, &xlm, &1_000i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "operations"),
        &600i128,
        &300i128,
        &0u64,
        &100u64,
        &false,
    );

    let spend = client.execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &250i128,
        &xlm,
        &String::from_str(&env, "operations"),
        &String::from_str(&env, "hosting"),
        &Some(114u64),
    );

    assert_eq!(spend.id, 1);
    let treasury = client.treasury();
    assert_eq!(treasury.assets.get(xlm).unwrap(), 750);
    assert_eq!(treasury.spending_history.len(), 1);
    let budget = treasury
        .budgets
        .get(String::from_str(&env, "operations"))
        .unwrap();
    assert_eq!(budget.spent, 250);
    assert_eq!(budget.remaining, 350);
}

#[test]
fn recurring_payments_reporting_and_rebalance_are_tracked() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    let usdc = asset(&env, "USDC");
    client.set_treasury_asset(&admin, &xlm, &100i128);
    client.set_treasury_asset(&admin, &usdc, &100i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "grants"),
        &500i128,
        &200i128,
        &0u64,
        &20u64,
        &true,
    );
    client.create_recurring_payment(
        &admin,
        &Address::generate(&env),
        &100i128,
        &usdc,
        &10u64,
        &String::from_str(&env, "grants"),
        &String::from_str(&env, "builder stipend"),
        &None,
        &Some(40u64),
    );

    env.ledger().set_timestamp(10);
    assert_eq!(client.process_recurring_payments(&admin), 1);

    client.set_rebalance_target(&admin, &xlm, &6_000i128);
    client.set_rebalance_target(&admin, &usdc, &4_000i128);
    let mut prices = Map::new(&env);
    prices.set(xlm.clone(), 2);
    prices.set(usdc.clone(), 1);
    let actions = client.rebalance_treasury(&admin, &prices);

    assert_eq!(actions.len(), 2);
    let report = client.treasury_report();
    assert_eq!(report.total_spends, 1);
    assert_eq!(report.total_spent, 100);
    assert_eq!(report.active_recurring_payments, 1);
    assert_eq!(report.monthly_burn_rate, 100);
    assert_eq!(report.runway_months, 2);
    assert_eq!(report.total_value_usd, 200);
    assert_eq!(report.last_rebalance, 10);
}

#[test]
fn recurring_payment_is_paused_when_balance_is_insufficient() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let usdc = asset(&env, "USDC");
    client.set_treasury_asset(&admin, &usdc, &50i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "operations"),
        &500i128,
        &500i128,
        &0u64,
        &20u64,
        &true,
    );
    client.create_recurring_payment(
        &admin,
        &Address::generate(&env),
        &100i128,
        &usdc,
        &10u64,
        &String::from_str(&env, "operations"),
        &String::from_str(&env, "salary"),
        &None,
        &Some(40u64),
    );

    env.ledger().set_timestamp(10);
    assert_eq!(client.process_recurring_payments(&admin), 0);

    let treasury = client.treasury();
    assert_eq!(treasury.spending_history.len(), 0);
    assert!(!treasury.recurring_payments.get(0).unwrap().active);
}

#[test]
fn treasury_report_defaults_to_infinite_runway_without_recent_spend() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    client.set_treasury_asset(&admin, &xlm, &250i128);

    let report = client.treasury_report();
    assert_eq!(report.total_spends, 0);
    assert_eq!(report.total_spent, 0);
    assert_eq!(report.monthly_burn_rate, 0);
    assert_eq!(report.runway_months, 999);
}

#[test]
fn committee_executes_delegated_treasury_spend_and_reports_metrics() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    client.set_treasury_asset(&admin, &xlm, &20_000i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "technical"),
        &20_000i128,
        &10_000i128,
        &0u64,
        &(365u64 * 86_400),
        &true,
    );

    let committee_members = members(&env, 5);
    let chair = committee_members.get(0).unwrap();
    let authorities = soroban_sdk::vec![
        &env,
        Authority::TreasurySpend(TreasurySpendAuthority {
            max_amount: 10_000,
            category: String::from_str(&env, "technical"),
        })
    ];

    let committee = client.create_committee(
        &admin,
        &String::from_str(&env, "Technical Committee"),
        &String::from_str(&env, "Delegated engineering treasury decisions"),
        &committee_members,
        &chair,
        &5u32,
        &authorities,
        &Some(30u32),
    );

    env.ledger().set_timestamp(86_400);
    let decision = client.propose_committee_decision(
        &committee.id,
        &chair,
        &String::from_str(&env, "Fund audit work"),
        &CommitteeAction::TreasurySpend(TreasurySpendAction {
            recipient: recipients.team.clone(),
            amount: 5_000,
            asset: xlm.clone(),
            category: String::from_str(&env, "technical"),
            purpose: String::from_str(&env, "security audit"),
        }),
    );

    let against_voter = committee_members.get(1).unwrap();
    let for_voter_one = committee_members.get(2).unwrap();
    let for_voter_two = committee_members.get(3).unwrap();

    client.vote_on_committee_decision(
        &committee.id,
        &decision.decision_id,
        &against_voter,
        &VoteType::Against,
    );
    client.vote_on_committee_decision(&committee.id, &decision.decision_id, &chair, &VoteType::For);
    client.vote_on_committee_decision(
        &committee.id,
        &decision.decision_id,
        &for_voter_one,
        &VoteType::For,
    );
    let approved = client.vote_on_committee_decision(
        &committee.id,
        &decision.decision_id,
        &for_voter_two,
        &VoteType::For,
    );
    assert_eq!(approved.status, DecisionStatus::Approved);
    assert_eq!(approved.votes_for, 3);
    assert_eq!(approved.votes_against, 1);

    env.ledger().set_timestamp(86_400 + 600);
    let executed = client.execute_committee_decision(&committee.id, &decision.decision_id, &chair);
    assert_eq!(executed.status, DecisionStatus::Executed);

    let treasury = client.treasury();
    assert_eq!(treasury.assets.get(xlm).unwrap(), 15_000);
    assert_eq!(treasury.spending_history.len(), 1);

    client.set_committee_approval_rating(&admin, &committee.id, &9_100u32);
    env.ledger().set_timestamp(31 * 86_400);
    let report = client.committee_report(&committee.id);
    assert_eq!(report.total_decisions, 1);
    assert_eq!(report.execution_rate, 10_000);
    assert_eq!(report.avg_decision_time, 600);
    assert_eq!(report.community_approval, 9_100);
    assert!(report.days_active >= 30);
}

#[test]
fn committee_can_adjust_reward_config_within_delegated_limits() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let committee_members = members(&env, 5);
    let chair = committee_members.get(0).unwrap();
    let parameters = soroban_sdk::vec![
        &env,
        String::from_str(&env, "liquidity_reward_bps"),
        String::from_str(&env, "min_claim_threshold")
    ];
    let authorities = soroban_sdk::vec![
        &env,
        Authority::ParameterAdjustment(ParameterAdjustmentAuthority {
            parameters,
            max_change_pct: 10,
        })
    ];

    let committee = client.create_committee(
        &admin,
        &String::from_str(&env, "Risk Committee"),
        &String::from_str(&env, "Adjusts bounded incentive parameters"),
        &committee_members,
        &chair,
        &5u32,
        &authorities,
        &Some(60u32),
    );

    let decision = client.propose_committee_decision(
        &committee.id,
        &chair,
        &String::from_str(&env, "Tune liquidity rewards"),
        &CommitteeAction::RewardConfigUpdate(RewardConfigUpdateAction {
            reward_bps: 105,
            min_claim_threshold: 105,
        }),
    );

    client.vote_on_committee_decision(&committee.id, &decision.decision_id, &chair, &VoteType::For);
    client.vote_on_committee_decision(
        &committee.id,
        &decision.decision_id,
        &committee_members.get(1).unwrap(),
        &VoteType::For,
    );
    client.vote_on_committee_decision(
        &committee.id,
        &decision.decision_id,
        &committee_members.get(2).unwrap(),
        &VoteType::For,
    );

    client.execute_committee_decision(&committee.id, &decision.decision_id, &chair);
    let distribution = client.distribution();
    assert_eq!(distribution.liquidity_reward_bps, 105);
    assert_eq!(distribution.min_claim_threshold, 105);
}

#[test]
fn committee_election_replaces_members_and_updates_chair() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &100_000_000);
    client.stake(&recipients.public_sale, &50_000_000);
    client.stake(&recipients.treasury, &40_000_000);

    let committee_members = members(&env, 5);
    let chair = committee_members.get(0).unwrap();
    let authorities = soroban_sdk::vec![
        &env,
        Authority::EmergencyAction(EmergencyActionAuthority {
            action_types: soroban_sdk::vec![&env, String::from_str(&env, "incident")]
        })
    ];

    let committee = client.create_committee(
        &admin,
        &String::from_str(&env, "Operations Committee"),
        &String::from_str(&env, "Coordinates incident response"),
        &committee_members,
        &chair,
        &5u32,
        &authorities,
        &Some(90u32),
    );

    client.start_committee_election(&admin, &committee.id, &3u32, &7u32);

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);

    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee.id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee.id, &candidate_three, &recipients.treasury);

    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &candidate_one);
    client.vote_in_committee_election(&committee.id, &recipients.public_sale, &candidate_one);
    client.vote_in_committee_election(&committee.id, &recipients.treasury, &candidate_two);

    env.ledger().set_timestamp(8 * 86_400);
    let winners = client.finalize_committee_election(&admin, &committee.id);
    assert_eq!(winners.len(), 3);
    assert_eq!(winners.get(0).unwrap(), candidate_one);

    let updated = client.committee(&committee.id);
    assert_eq!(updated.members.len(), 3);
    assert_eq!(updated.chair, candidate_one);
}

#[test]
fn committee_override_and_cross_committee_approval_are_tracked() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let requester_members = members(&env, 5);
    let approver_members = members(&env, 5);
    let requester_chair = requester_members.get(0).unwrap();
    let approver_chair = approver_members.get(0).unwrap();

    let requester_committee = client.create_committee(
        &admin,
        &String::from_str(&env, "Technical Committee"),
        &String::from_str(&env, "Requests cross-functional review"),
        &requester_members,
        &requester_chair,
        &5u32,
        &soroban_sdk::vec![
            &env,
            Authority::EmergencyAction(EmergencyActionAuthority {
                action_types: soroban_sdk::vec![&env, String::from_str(&env, "incident")]
            })
        ],
        &Some(30u32),
    );
    let approving_committee = client.create_committee(
        &admin,
        &String::from_str(&env, "Risk Committee"),
        &String::from_str(&env, "Approves cross-committee escalations"),
        &approver_members,
        &approver_chair,
        &5u32,
        &soroban_sdk::vec![
            &env,
            Authority::EmergencyAction(EmergencyActionAuthority {
                action_types: soroban_sdk::vec![&env, String::from_str(&env, "incident")]
            })
        ],
        &Some(30u32),
    );

    let request = client.request_cross_committee_approval(
        &requester_committee.id,
        &requester_chair,
        &soroban_sdk::vec![&env, approving_committee.id],
        &String::from_str(&env, "Approve incident-response rollback"),
    );

    let decision = client.propose_committee_decision(
        &approving_committee.id,
        &approver_chair,
        &String::from_str(&env, "Approve rollback"),
        &CommitteeAction::EmergencyAction(EmergencyActionPayload {
            action_type: String::from_str(&env, "incident"),
            details: String::from_str(&env, "authorizes rollback"),
        }),
    );

    client.vote_on_committee_decision(
        &approving_committee.id,
        &decision.decision_id,
        &approver_chair,
        &VoteType::For,
    );
    client.vote_on_committee_decision(
        &approving_committee.id,
        &decision.decision_id,
        &approver_members.get(1).unwrap(),
        &VoteType::For,
    );
    client.vote_on_committee_decision(
        &approving_committee.id,
        &decision.decision_id,
        &approver_members.get(2).unwrap(),
        &VoteType::For,
    );

    let approved_request = client.approve_cross_committee_request(
        &request.id,
        &approving_committee.id,
        &approver_chair,
        &decision.decision_id,
    );
    assert_eq!(approved_request.status, CrossCommitteeStatus::Approved);

    let overridden =
        client.override_committee_decision(&admin, &approving_committee.id, &decision.decision_id);
    assert_eq!(overridden.status, DecisionStatus::Overridden);

    let report = client.committee_report(&approving_committee.id);
    assert_eq!(report.overridden_count, 1);

    let stored_request = client.cross_committee_request(&request.id);
    assert_eq!(stored_request.status, CrossCommitteeStatus::Approved);
}

#[test]
fn governance_proposal_vote_finalize_and_execute() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &80_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ParameterChange(
            String::from_str(&env, "liquidity_reward_bps"),
            100,
            120,
        ),
        &String::from_str(&env, "Adjust reward"),
        &String::from_str(&env, "Increase by 20%"),
        &Bytes::new(&env),
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&proposal_id, &recipients.public_sale, &GovernanceVoteType::For);

    env.ledger().set_timestamp(8 * 86_400);
    let status = client.finalize_proposal(&proposal_id);
    assert_eq!(status, ProposalStatus::Succeeded);

    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.status, ProposalStatus::Executed);
}

#[test]
fn timelock_queue_execute_and_cancel_flow() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let cfg = GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: 7 * 86_400,
        voting_delay: 60,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 60,
    };
    client.configure_governance(&admin, &cfg);
    client.initialize_timelock(&admin, &3_600u64, &(7 * 86_400u64), &admin);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &80_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::FeatureToggle(String::from_str(&env, "new_signal_ui"), true),
        &String::from_str(&env, "Enable feature"),
        &String::from_str(&env, "toggle"),
        &Bytes::new(&env),
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&proposal_id, &recipients.public_sale, &GovernanceVoteType::For);

    env.ledger().set_timestamp(8 * 86_400);
    assert_eq!(client.finalize_proposal(&proposal_id), ProposalStatus::Succeeded);

    let action_id = client.queue_action(&proposal_id);
    let early = client.try_execute_queued_action(&action_id, &admin);
    assert_eq!(early, Err(Ok(GovernanceError::InvalidDuration)));

    client.cancel_queued_action(&action_id, &admin);
    let analytics = client.timelock_analytics();
    assert_eq!(analytics.total_cancelled, 1);
}

#[test]
fn governance_reputation_tracks_activity() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &80_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Community sentiment")),
        &String::from_str(&env, "Signal"),
        &String::from_str(&env, "Record governance sentiment"),
        &Bytes::new(&env),
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&proposal_id, &recipients.public_sale, &GovernanceVoteType::For);

    env.ledger().set_timestamp(8 * 86_400);
    client.finalize_proposal(&proposal_id);

    let proposer_rep = client.governance_reputation(&recipients.community_rewards);
    let voter_rep = client.governance_reputation(&recipients.public_sale);

    assert!(proposer_rep.participation_history.proposals_created >= 1);
    assert!(voter_rep.participation_history.votes_cast >= 1);
    assert!(proposer_rep.reputation_score > 0);
}

#[test]
fn conviction_voting_accumulates_over_time() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let pool_id = client.create_conviction_pool(&admin, &100_000i128, &1_000i128, &86_400u64);
    let proposal_id = client.create_conviction_proposal(
        &pool_id,
        &recipients.community_rewards,
        &String::from_str(&env, "Fund builder grant"),
        &10_000i128,
        &recipients.public_sale,
    );

    client.vote_conviction(
        &pool_id,
        &proposal_id,
        &recipients.community_rewards,
        &1_000i128,
    );

    env.ledger().set_timestamp(10 * 86_400);
    let conviction = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert!(conviction > 0);

    let analytics = client.analyze_conviction_proposal(&pool_id, &proposal_id);
    assert!(analytics.current_conviction > 0);
    assert_eq!(analytics.total_voters, 1);
}

#[test]
fn upgrade_announcement_event_emitted_on_contract_upgrade_proposal_success() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &80_000_000i128);

    let new_wasm_hash = Bytes::from_array(&env, &[1u8; 32]);
    let migration_notes_hash = Bytes::from_array(&env, &[2u8; 32]);
    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ContractUpgrade(
            String::from_str(&env, "auto_trade"),
            new_wasm_hash.clone(),
        ),
        &String::from_str(&env, "Upgrade auto_trade contract"),
        &String::from_str(&env, "Deploy new version"),
        &migration_notes_hash,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&proposal_id, &recipients.public_sale, &GovernanceVoteType::For);

    env.ledger().set_timestamp(8 * 86_400);
    let status = client.finalize_proposal(&proposal_id);
    assert_eq!(status, ProposalStatus::Succeeded);

    // Check event was emitted
    let events = env.events().all();
    assert_eq!(events.len(), 2); // propnew and upgrade announced
    let upgrade_event = &events[1];
    assert_eq!(upgrade_event.0, (symbol_short!("upgrade"), symbol_short!("announced")));
    let (contract, hash, exec_after, notes) = upgrade_event.1.clone();
    assert_eq!(contract, String::from_str(&env, "auto_trade"));
    assert_eq!(hash, new_wasm_hash);
    assert_eq!(exec_after, 8 * 86_400 + 0); // execution_delay is 0 by default
    assert_eq!(notes, migration_notes_hash);
}
