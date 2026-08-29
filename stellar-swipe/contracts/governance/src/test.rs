extern crate std;

use crate::distribution::{
    DistributionRecipients, EARLY_INVESTOR_VESTING_DURATION, TEAM_CLIFF_DURATION,
    TEAM_VESTING_DURATION, YEAR_SECONDS,
};
use crate::proposals::{
    GovernanceConfig, ProposalCategory, ProposalStatus, ProposalType,
    VoteType as GovernanceVoteType,
};
use crate::{
    Authority, CommitteeAction, CommitteeElectionStatus, CrossCommitteeStatus, DecisionStatus,
    EmergencyActionAuthority, EmergencyActionPayload, GovernanceContract, GovernanceContractClient,
    GovernanceError, ParameterAdjustmentAuthority, ReputationConfig, ReputationTier,
    RewardConfigUpdateAction, StalenessLevel, TreasurySpendAction, TreasurySpendAuthority,
    VoteType,
};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, Address, Bytes, Env, Map, String, Symbol, Vec};
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

fn fund_user(env: &Env, contract_id: &Address, user: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        crate::add_balance(env, user, amount).unwrap();
    });
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
    // governance approval must exist before spending
    client.approve_treasury_budget(
        &admin,
        &String::from_str(&env, "operations"),
        &114u64,
        &600i128,
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
    // governance must approve before recurring payments can be scheduled or executed
    client.approve_treasury_budget(&admin, &String::from_str(&env, "grants"), &1u64, &500i128);
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
    client.approve_treasury_budget(
        &admin,
        &String::from_str(&env, "operations"),
        &2u64,
        &500i128,
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
    // governance must approve the "technical" budget cap before any spend
    client.approve_treasury_budget(
        &admin,
        &String::from_str(&env, "technical"),
        &1u64,
        &20_000i128,
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

    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &0u32, &0i128);

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
    let result = client.finalize_committee_election(&admin, &committee.id);
    assert_eq!(result.status, CommitteeElectionStatus::Succeeded);
    assert_eq!(result.winners.len(), 3);
    assert_eq!(result.winners.get(0).unwrap(), candidate_one);
    assert_eq!(result.valid_votes, 3);
    assert_eq!(result.rejected_votes, 0);

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
    client.stake(&recipients.public_sale, &40_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "liquidity_reward_bps"), 100, 120),
        &String::from_str(&env, "Adjust reward"),
        &String::from_str(&env, "Increase by 20%"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(
        &proposal_id,
        &recipients.public_sale,
        &GovernanceVoteType::For,
    );

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
        discussion_duration: 0,
    };
    client.configure_governance(&admin, &cfg);
    client.initialize_timelock(&admin, &3_600u64, &(7 * 86_400u64), &admin);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &40_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::FeatureToggle(String::from_str(&env, "new_signal_ui"), true),
        &String::from_str(&env, "Enable feature"),
        &String::from_str(&env, "toggle"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(
        &proposal_id,
        &recipients.public_sale,
        &GovernanceVoteType::For,
    );

    env.ledger().set_timestamp(8 * 86_400);
    assert_eq!(
        client.finalize_proposal(&proposal_id),
        ProposalStatus::Succeeded
    );

    let action_id = client.queue_action(&proposal_id);
    let early = client.try_execute_queued_action(&action_id, &admin);
    assert_eq!(early, Err(Ok(GovernanceError::InvalidDuration)));

    client.cancel_queued_action(&action_id, &admin);
    let analytics = client.timelock_analytics();
    assert_eq!(analytics.total_cancelled, 1);
}

fn queue_underfunded_treasury_spend_action(
    env: &Env,
    client: &GovernanceContractClient<'_>,
    admin: &Address,
    recipients: &DistributionRecipients,
) -> (u64, Asset) {
    let cfg = GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: 7 * 86_400,
        voting_delay: 60,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 60,
        discussion_duration: 0,
    };
    client.configure_governance(admin, &cfg);
    client.initialize_timelock(admin, &3_600u64, &(7 * 86_400u64), admin);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &40_000_000i128);

    // Fund the treasury so the proposal passes creation-time validation, then
    // drain it again after queueing. This models the real-world "contract
    // state issue" the emergency path exists for: the treasury had funds when
    // the proposal was approved, but no longer does by the time the timelock
    // delay elapses and the action becomes executable.
    let spend_asset = asset(env, "USDC");
    client.set_treasury_asset(admin, &spend_asset, &10_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::TreasurySpend(
            recipients.public_sale.clone(),
            500i128,
            spend_asset.clone(),
            String::from_str(env, "payout"),
        ),
        &String::from_str(env, "Fund payout"),
        &String::from_str(env, "treasury spend"),
        &Bytes::new(env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(
        &proposal_id,
        &recipients.public_sale,
        &GovernanceVoteType::For,
    );

    env.ledger().set_timestamp(8 * 86_400);
    assert_eq!(
        client.finalize_proposal(&proposal_id),
        ProposalStatus::Succeeded
    );

    let action_id = client.queue_action(&proposal_id);
    client.set_treasury_asset(admin, &spend_asset, &0i128);
    (action_id, spend_asset)
}

#[test]
fn emergency_unblock_rejects_ineligible_and_unauthorized_callers() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let (action_id, _asset) =
        queue_underfunded_treasury_spend_action(&env, &client, &admin, &recipients);

    // Not yet overdue: emergency recovery has not opened up yet.
    let too_early = client.try_emergency_unblock_action(&action_id, &admin);
    assert_eq!(too_early, Err(Ok(GovernanceError::InvalidDuration)));

    // Past the normal execution window but still inside the grace period.
    env.ledger().set_timestamp(10 * 86_400);
    let still_in_grace = client.try_emergency_unblock_action(&action_id, &admin);
    assert_eq!(still_in_grace, Err(Ok(GovernanceError::InvalidDuration)));

    // Guardian-only: any other caller is rejected even once the window passes.
    env.ledger().set_timestamp(11 * 86_400);
    let not_guardian = client.try_emergency_unblock_action(&action_id, &recipients.public_sale);
    assert_eq!(not_guardian, Err(Ok(GovernanceError::Unauthorized)));
}

#[test]
fn emergency_unblock_retries_a_stuck_action_and_rejects_duplicate_execution() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let (action_id, spend_asset) =
        queue_underfunded_treasury_spend_action(&env, &client, &admin, &recipients);

    // Treasury has no funds yet, so the normal execution attempt fails and the
    // action sits in the queue unexecuted rather than disappearing.
    env.ledger().set_timestamp(10 * 86_400);
    let failed = client.try_execute_queued_action(&action_id, &admin);
    assert_eq!(failed, Err(Ok(GovernanceError::InsufficientBalance)));
    assert!(!client.queued_action(&action_id).executed);

    // Past the execution window plus the grace period, the action is eligible
    // for emergency recovery, but retrying while still underfunded fails the
    // same way the normal path did.
    env.ledger().set_timestamp(11 * 86_400);
    let retry_still_stuck = client.try_emergency_unblock_action(&action_id, &admin);
    assert_eq!(
        retry_still_stuck,
        Err(Ok(GovernanceError::InsufficientBalance))
    );
    assert!(!client.queued_action(&action_id).executed);

    // Once the underlying contract state issue is resolved, the guardian's
    // emergency retry succeeds exactly once.
    client.set_treasury_asset(&admin, &spend_asset, &500i128);
    client.emergency_unblock_action(&action_id, &admin);

    let recovered_action = client.queued_action(&action_id);
    assert!(recovered_action.executed);

    let proposal = client.proposal(&recovered_action.proposal_id);
    assert_eq!(proposal.status, ProposalStatus::Executed);

    // Duplicate-execution protection: a second emergency call on an already
    // executed action is rejected rather than spending the treasury twice.
    let duplicate = client.try_emergency_unblock_action(&action_id, &admin);
    assert_eq!(duplicate, Err(Ok(GovernanceError::InvalidCommitteeAction)));
}

#[test]
fn governance_reputation_tracks_activity() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &40_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Community sentiment")),
        &String::from_str(&env, "Signal"),
        &String::from_str(&env, "Record governance sentiment"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(
        &proposal_id,
        &recipients.public_sale,
        &GovernanceVoteType::For,
    );

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
    client.stake(&recipients.public_sale, &40_000_000i128);

    let new_wasm_hash = Bytes::from_array(&env, &[1u8; 32]);
    let migration_notes_hash = Bytes::from_array(&env, &[2u8; 32]);
    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ContractUpgrade(String::from_str(&env, "auto_trade"), new_wasm_hash.clone()),
        &String::from_str(&env, "Upgrade auto_trade contract"),
        &String::from_str(&env, "Deploy new version"),
        &migration_notes_hash,
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(
        &proposal_id,
        &recipients.public_sale,
        &GovernanceVoteType::For,
    );

    env.ledger().set_timestamp(8 * 86_400);
    let status = client.finalize_proposal(&proposal_id);
    assert_eq!(status, ProposalStatus::Succeeded);

    // Check event was emitted
    use soroban_sdk::testutils::Events;
    use soroban_sdk::TryFromVal;

    let events = env.events().all();
    let found = events.iter().any(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        let t0 = topics
            .get(0)
            .and_then(|v| Symbol::try_from_val(&env, &v).ok());
        let t1 = topics
            .get(1)
            .and_then(|v| Symbol::try_from_val(&env, &v).ok());
        t0 == Some(Symbol::new(&env, "upgrade")) && t1 == Some(Symbol::new(&env, "announced"))
    });
    assert!(
        found,
        "upgrade announcement event must be emitted on successful finalize"
    );
}

// ── Reputation decay & stale-score tests ─────────────────────────────────

#[test]
fn reputation_tier_computed_correctly_from_participation() {
    let (env, contract_id, admin, _recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &_recipients);

    let user = Address::generate(&env);
    fund_user(&env, &contract_id, &user, 200_000i128);
    client.stake(&user, &100_000i128);
    for i in 0..60u64 {
        env.ledger().set_timestamp(100 + i * 1000);
        let _pid = client.create_proposal(
            &user,
            &ProposalType::ParameterChange(String::from_str(&env, "test_param"), 1000, 1100),
            &String::from_str(&env, "Proposal"),
            &String::from_str(&env, "desc"),
            &Bytes::new(&env),
            &ProposalCategory::General,
            &false,
        );
    }
    // After 60 proposals, tier should be Silver (>=50 actions)
    let rep = client.governance_reputation(&user);
    assert_eq!(rep.tier, ReputationTier::Silver);
}

#[test]
fn decay_applied_after_grace_period() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);
    env.mock_all_auths();

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &40_000_000i128);

    env.ledger().set_timestamp(100);
    let pid = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "test_param"), 1000, 1100),
        &String::from_str(&env, "Test"),
        &String::from_str(&env, "desc"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(170);
    client.cast_vote(
        &pid,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(&pid, &recipients.public_sale, &GovernanceVoteType::For);
    env.ledger().set_timestamp(8 * 86_400);
    client.finalize_proposal(&pid);

    let rep_before = client.governance_reputation(&recipients.community_rewards);
    assert!(rep_before.reputation_score > 0);

    // Advance well past the Bronze grace period (30 days) so decay is measurable.
    env.ledger().set_timestamp(8 * 86_400 + 100 * 86_400);

    let rep_after = client.governance_reputation(&recipients.community_rewards);
    assert!(
        rep_after.reputation_score < rep_before.reputation_score,
        "Reputation should decay after grace period: before={}, after={}",
        rep_before.reputation_score,
        rep_after.reputation_score,
    );
}

#[test]
fn no_decay_within_grace_period() {
    let (env, contract_id, admin, _recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &_recipients);
    env.mock_all_auths();

    let user = Address::generate(&env);
    fund_user(&env, &contract_id, &user, 200_000i128);
    client.stake(&user, &100_000i128);

    env.ledger().set_timestamp(100);
    let _pid = client.create_proposal(
        &user,
        &ProposalType::ParameterChange(String::from_str(&env, "test_param"), 1000, 1100),
        &String::from_str(&env, "Test"),
        &String::from_str(&env, "desc"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let rep_before = client.governance_reputation(&user);

    // Advance 15 days (within Bronze 30-day grace period)
    env.ledger().set_timestamp(100 + 15 * 86_400);
    let rep_after = client.governance_reputation(&user);

    assert_eq!(
        rep_after.reputation_score, rep_before.reputation_score,
        "Reputation should NOT decay within grace period"
    );
}

#[test]
fn staleness_level_detected_correctly() {
    let (env, contract_id, admin, _recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &_recipients);
    env.mock_all_auths();

    let user = Address::generate(&env);
    fund_user(&env, &contract_id, &user, 200_000i128);
    client.stake(&user, &100_000i128);

    // Set initial activity timestamp
    env.ledger().set_timestamp(1000);
    let _ = client.create_proposal(
        &user,
        &ProposalType::ParameterChange(String::from_str(&env, "test_param"), 1000, 1100),
        &String::from_str(&env, "Test"),
        &String::from_str(&env, "desc"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // Still Active within 30 days
    let staleness = client.check_reputation_staleness(&user);
    assert_eq!(staleness, StalenessLevel::Active);

    // Aging: 31-90 days
    env.ledger().set_timestamp(1000 + 60 * 86_400);
    let staleness = client.check_reputation_staleness(&user);
    assert_eq!(staleness, StalenessLevel::Aging);

    // Stale: 91-180 days
    env.ledger().set_timestamp(1000 + 120 * 86_400);
    let staleness = client.check_reputation_staleness(&user);
    assert_eq!(staleness, StalenessLevel::Stale);

    // Critical: >180 days
    env.ledger().set_timestamp(1000 + 250 * 86_400);
    let staleness = client.check_reputation_staleness(&user);
    assert_eq!(staleness, StalenessLevel::Critical);
}

#[test]
fn refresh_stale_reputation_recalculates_score() {
    let (env, contract_id, admin, _recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &_recipients);
    env.mock_all_auths();

    let user = Address::generate(&env);
    fund_user(&env, &contract_id, &user, 200_000i128);
    client.stake(&user, &100_000i128);

    env.ledger().set_timestamp(100);
    let _pid = client.create_proposal(
        &user,
        &ProposalType::ParameterChange(String::from_str(&env, "test_param"), 1000, 1100),
        &String::from_str(&env, "Test"),
        &String::from_str(&env, "desc"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // Go 100 days into the future - reputation should be decayed
    env.ledger().set_timestamp(100 + 100 * 86_400);

    // Call refresh
    let fresh_score = client.refresh_reputation(&user);
    let rep = client.governance_reputation(&user);
    assert_eq!(fresh_score, rep.reputation_score);
    assert_eq!(rep.staleness_override, StalenessLevel::Auto);
}

#[test]
fn reputation_config_can_be_updated_by_admin() {
    let (env, contract_id, admin, _recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &_recipients);
    env.mock_all_auths();

    // Default config
    let config = client.reputation_config();
    assert!(config.decay_enabled);
    assert!(config.stale_penalty_enabled);

    // Admin disables decay
    let updated = ReputationConfig {
        decay_enabled: false,
        stale_penalty_enabled: true,
        default_tier: ReputationTier::Bronze,
    };
    let result = client.update_reputation_config(&admin, &updated);
    assert_eq!(result.decay_enabled, false);

    // Verify it's stored
    let config = client.reputation_config();
    assert!(!config.decay_enabled);
    assert!(config.stale_penalty_enabled);
}

#[test]
fn detect_staleness_logic() {
    use crate::reputation::detect_staleness;
    let env = Env::default();
    let now = 20_000_000u64;

    env.ledger().set_timestamp(now);

    // Active: last activity was 10 days ago
    assert_eq!(
        detect_staleness(&env, now - 10 * 86_400),
        StalenessLevel::Active
    );

    // Aging: last activity was 60 days ago
    assert_eq!(
        detect_staleness(&env, now - 60 * 86_400),
        StalenessLevel::Aging
    );

    // Stale: last activity was 120 days ago
    assert_eq!(
        detect_staleness(&env, now - 120 * 86_400),
        StalenessLevel::Stale
    );

    // Critical: last activity was 200 days ago
    assert_eq!(
        detect_staleness(&env, now - 200 * 86_400),
        StalenessLevel::Critical
    );
}

#[cfg(test)]
mod event_format_tests {
    use super::*;
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{Symbol, TryFromVal};

    fn last_topics(env: &Env) -> (Symbol, Symbol) {
        let events = env.events().all();
        let e = events.last().unwrap();
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1;
        let t0 = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1 = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        (t0, t1)
    }

    fn setup_gov(env: &Env) -> (Address, Address, GovernanceContractClient<'_>) {
        let admin = Address::generate(env);
        let id = env.register(GovernanceContract, ());
        let client = GovernanceContractClient::new(env, &id);
        let recipients = DistributionRecipients {
            team: Address::generate(env),
            early_investors: Address::generate(env),
            community_rewards: Address::generate(env),
            treasury: Address::generate(env),
            public_sale: Address::generate(env),
        };
        client.initialize(
            &admin,
            &soroban_sdk::String::from_str(env, "StellarSwipe"),
            &soroban_sdk::String::from_str(env, "SSW"),
            &7u32,
            &1_000_000_000i128,
            &recipients,
        );
        (admin, id, client)
    }

    #[test]
    fn stake_changed_event_has_two_topic_format() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id, client) = setup_gov(&env);
        let user = Address::generate(&env);
        fund_user(&env, &contract_id, &user, 200_000i128);
        client.stake(&user, &100i128);
        // Find stake_changed event
        let found = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            let t0 = topics
                .get(0)
                .and_then(|v| Symbol::try_from_val(&env, &v).ok());
            let t1 = topics
                .get(1)
                .and_then(|v| Symbol::try_from_val(&env, &v).ok());
            t0 == Some(Symbol::new(&env, "governance"))
                && t1 == Some(Symbol::new(&env, "stake_changed"))
        });
        assert!(
            found,
            "stake_changed must use (governance, stake_changed) topics"
        );
    }

    #[test]
    fn vesting_released_event_has_two_topic_format() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, client) = setup_gov(&env);
        let beneficiary = Address::generate(&env);
        env.ledger().with_mut(|l| l.timestamp = 0);
        client.create_vesting_schedule(&admin, &beneficiary, &1_000i128, &0u64, &0u64, &1u64);
        env.ledger().with_mut(|l| l.timestamp = 10);
        client.release_vested_tokens(&beneficiary);
        let (contract, event) = last_topics(&env);
        assert_eq!(contract, Symbol::new(&env, "governance"));
        assert_eq!(event, Symbol::new(&env, "vesting_released"));
    }
}

// ── Committee election: quorum, invalid-vote, and success tests ──────────

/// Helper: build a committee with EmergencyAction authority and return
/// (committee, chair) so election tests don't repeat boilerplate.
fn make_election_committee<'a>(
    env: &'a Env,
    client: &GovernanceContractClient<'a>,
    admin: &Address,
) -> (crate::Committee, Address) {
    let mems = members(env, 5);
    let chair = mems.get(0).unwrap();
    let authorities = soroban_sdk::vec![
        env,
        Authority::EmergencyAction(EmergencyActionAuthority {
            action_types: soroban_sdk::vec![env, String::from_str(env, "incident")],
        })
    ];
    let committee = client.create_committee(
        admin,
        &String::from_str(env, "Test Committee"),
        &String::from_str(env, "Election test committee"),
        &mems,
        &chair,
        &5u32,
        &authorities,
        &Some(90u32),
    );
    (committee, chair)
}

#[test]
fn election_fails_when_voter_participation_below_quorum() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Give voters staked balance
    client.stake(&recipients.community_rewards, &100_000_000);
    client.stake(&recipients.public_sale, &50_000_000);
    client.stake(&recipients.treasury, &40_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);

    // Require at least 3 participating voters — we'll only cast 1 vote
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &3u32, &0i128);

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee.id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee.id, &candidate_three, &recipients.treasury);

    // Only 1 voter participates (below quorum of 3)
    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &candidate_one);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee.id);

    assert_eq!(
        result.status,
        CommitteeElectionStatus::FailedQuorumParticipation
    );
    assert_eq!(result.winners.len(), 0);
    assert_eq!(result.valid_votes, 1);
    assert_eq!(result.rejected_votes, 0);

    // Committee membership must be unchanged
    let updated = client.committee(&committee.id);
    assert_eq!(
        updated.members.len(),
        5,
        "existing members must be preserved on quorum failure"
    );
}

#[test]
fn election_fails_when_stake_weight_below_quorum_threshold() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Small stakes — well below our threshold
    client.stake(&recipients.community_rewards, &1_000);
    client.stake(&recipients.public_sale, &1_000);
    client.stake(&recipients.treasury, &1_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);

    // Quorum stake threshold: 500_000 (far above what's staked)
    client.start_committee_election(
        &admin,
        &committee.id,
        &3u32,
        &7u32,
        &0u32,        // no participation quorum
        &500_000i128, // high stake-weight quorum
    );

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee.id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee.id, &candidate_three, &recipients.treasury);

    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &candidate_one);
    client.vote_in_committee_election(&committee.id, &recipients.public_sale, &candidate_two);
    client.vote_in_committee_election(&committee.id, &recipients.treasury, &candidate_three);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee.id);

    assert_eq!(result.status, CommitteeElectionStatus::FailedQuorumStake);
    assert_eq!(result.winners.len(), 0);
    // Total stake weight = 3 × 1_000 = 3_000 — well below 500_000
    assert!(result.total_stake_weight < 500_000);

    // Committee membership must be unchanged
    let updated = client.committee(&committee.id);
    assert_eq!(
        updated.members.len(),
        5,
        "existing members must be preserved on stake quorum failure"
    );
}

#[test]
fn vote_for_unknown_candidate_is_rejected_with_invalid_vote_error() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &100_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &0u32, &0i128);

    let candidate_one = Address::generate(&env);
    let unknown_candidate = Address::generate(&env); // not nominated

    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);

    // Attempt to vote for a candidate not on the ballot
    let result = client.try_vote_in_committee_election(
        &committee.id,
        &recipients.community_rewards,
        &unknown_candidate,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidElectionVote)));

    // Election votes map must be unmodified (no phantom vote recorded)
    let election = client.committee_election(&committee.id);
    assert_eq!(
        election.votes.len(),
        0,
        "no vote should be recorded for an invalid candidate"
    );
}

#[test]
fn vote_from_unstaked_address_is_rejected_with_invalid_vote_error() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Do NOT stake recipients.community_rewards
    client.stake(&recipients.public_sale, &50_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &0u32, &0i128);

    let candidate_one = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.public_sale);

    // Unstaked voter — must be rejected cleanly
    let result = client.try_vote_in_committee_election(
        &committee.id,
        &recipients.community_rewards, // no staked balance
        &candidate_one,
    );
    assert_eq!(result, Err(Ok(GovernanceError::InvalidElectionVote)));

    // Election state must be unmodified
    let election = client.committee_election(&committee.id);
    assert_eq!(
        election.votes.len(),
        0,
        "unstaked voter must not be recorded in election votes"
    );
}

#[test]
fn duplicate_vote_is_rejected_without_corrupting_election_state() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &100_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &0u32, &0i128);

    let candidate_one = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);

    // First vote — should succeed
    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &candidate_one);

    // Second vote from same voter — must be rejected
    let result = client.try_vote_in_committee_election(
        &committee.id,
        &recipients.community_rewards,
        &candidate_one,
    );
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyVoted)));

    // Exactly 1 vote must be recorded, not 2
    let election = client.committee_election(&committee.id);
    assert_eq!(
        election.votes.len(),
        1,
        "only one vote should be recorded per voter"
    );
}

#[test]
fn election_result_exposes_rejected_vote_count() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Give several voters staked balances
    client.stake(&recipients.community_rewards, &100_000_000);
    client.stake(&recipients.public_sale, &50_000_000);
    client.stake(&recipients.treasury, &40_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);
    // No quorum requirements so we can observe the count freely
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &0u32, &0i128);

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee.id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee.id, &candidate_three, &recipients.treasury);

    // All three cast valid votes
    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &candidate_one);
    client.vote_in_committee_election(&committee.id, &recipients.public_sale, &candidate_two);
    client.vote_in_committee_election(&committee.id, &recipients.treasury, &candidate_three);

    env.ledger().set_timestamp(8 * 86_400);
    let result = client.finalize_committee_election(&admin, &committee.id);

    assert_eq!(result.status, CommitteeElectionStatus::Succeeded);
    assert_eq!(result.valid_votes, 3);
    assert_eq!(result.rejected_votes, 0);
    assert_eq!(result.winners.len(), 3);
    assert!(result.total_stake_weight > 0);
}

#[test]
fn successful_election_with_both_quorum_thresholds_met() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &100_000_000);
    client.stake(&recipients.public_sale, &50_000_000);
    client.stake(&recipients.treasury, &40_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);

    // Both quorum conditions; total staked = 190_000_000 so 100_000_000 threshold is met
    client.start_committee_election(
        &admin,
        &committee.id,
        &3u32,
        &7u32,
        &2u32,            // need at least 2 voters
        &100_000_000i128, // need at least 100_000_000 total stake weight
    );

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
    let result = client.finalize_committee_election(&admin, &committee.id);

    assert_eq!(result.status, CommitteeElectionStatus::Succeeded);
    assert_eq!(result.valid_votes, 3);
    assert!(result.total_stake_weight >= 100_000_000);
    assert_eq!(result.winners.len(), 3);
    // Top vote-getter (candidate_one with 150_000_000 weight) becomes chair
    assert_eq!(result.winners.get(0).unwrap(), candidate_one);

    let updated = client.committee(&committee.id);
    assert_eq!(updated.chair, candidate_one);
    assert_eq!(updated.members.len(), 3);
}

#[test]
fn election_can_be_restarted_after_quorum_failure() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &100_000_000);
    client.stake(&recipients.public_sale, &50_000_000);
    client.stake(&recipients.treasury, &40_000_000);

    let (committee, _) = make_election_committee(&env, &client, &admin);

    // First election — strict participation quorum that won't be met
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &5u32, &0i128);

    let candidate_one = Address::generate(&env);
    let candidate_two = Address::generate(&env);
    let candidate_three = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &candidate_one, &recipients.community_rewards);
    client.nominate_for_committee(&committee.id, &candidate_two, &recipients.public_sale);
    client.nominate_for_committee(&committee.id, &candidate_three, &recipients.treasury);

    // Only 1 vote cast — quorum requires 5
    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &candidate_one);

    env.ledger().set_timestamp(8 * 86_400);
    let first_result = client.finalize_committee_election(&admin, &committee.id);
    assert_eq!(
        first_result.status,
        CommitteeElectionStatus::FailedQuorumParticipation
    );

    // After a failed election the record is cleared; a new election can be started
    // (advance time so the new election isn't blocked by a running election)
    env.ledger().set_timestamp(9 * 86_400);
    client.start_committee_election(&admin, &committee.id, &3u32, &7u32, &1u32, &0i128);

    let new_c1 = Address::generate(&env);
    let new_c2 = Address::generate(&env);
    let new_c3 = Address::generate(&env);
    client.nominate_for_committee(&committee.id, &new_c1, &recipients.community_rewards);
    client.nominate_for_committee(&committee.id, &new_c2, &recipients.public_sale);
    client.nominate_for_committee(&committee.id, &new_c3, &recipients.treasury);

    client.vote_in_committee_election(&committee.id, &recipients.community_rewards, &new_c1);
    client.vote_in_committee_election(&committee.id, &recipients.public_sale, &new_c1);
    client.vote_in_committee_election(&committee.id, &recipients.treasury, &new_c2);

    env.ledger().set_timestamp(17 * 86_400);
    let second_result = client.finalize_committee_election(&admin, &committee.id);
    assert_eq!(second_result.status, CommitteeElectionStatus::Succeeded);
    assert_eq!(second_result.winners.get(0).unwrap(), new_c1);
}

// ── Treasury budget-cap guardrail integration tests ──────────────────────

/// Spending without calling `approve_treasury_budget` first returns
/// `BudgetApprovalRequired`.
#[test]
fn spend_without_approval_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    client.set_treasury_asset(&admin, &xlm, &1_000i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &500i128,
        &500i128,
        &0u64,
        &100u64,
        &false,
    );
    // No approve_treasury_budget call

    let result = client.try_execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &100i128,
        &xlm,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "test"),
        &None,
    );
    assert_eq!(result, Err(Ok(GovernanceError::BudgetApprovalRequired)));
}

/// A spend that would push cumulative drawn past the approved cap returns
/// `ApprovedCapExceeded`.
#[test]
fn spend_exceeding_approved_cap_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    client.set_treasury_asset(&admin, &xlm, &1_000i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &500i128,
        &500i128,
        &0u64,
        &100u64,
        &false,
    );
    // Governance approves only 200 out of 500 allocated
    client.approve_treasury_budget(&admin, &String::from_str(&env, "ops"), &10u64, &200i128);

    // First draw of 150 succeeds (drawn: 150 ≤ cap 200)
    client.execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &150i128,
        &xlm,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "hosting"),
        &Some(10u64),
    );

    // Second draw of 100 would push drawn to 250 > cap 200
    let result = client.try_execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &100i128,
        &xlm,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "extra"),
        &Some(10u64),
    );
    assert_eq!(result, Err(Ok(GovernanceError::ApprovedCapExceeded)));
}

/// `approve_treasury_budget` with a cap greater than `allocated` fails.
#[test]
fn approve_cap_exceeding_allocated_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.create_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &200i128,
        &200i128,
        &0u64,
        &100u64,
        &false,
    );

    let result = client.try_approve_treasury_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &5u64,
        &500i128, // 500 > allocated 200
    );
    assert_eq!(result, Err(Ok(GovernanceError::BudgetExceeded)));
}

/// Re-approving a category with a new proposal resets `total_drawn`,
/// allowing further spending under the fresh cap.
#[test]
fn re_approval_resets_drawn_and_allows_new_spending() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let xlm = asset(&env, "XLM");
    client.set_treasury_asset(&admin, &xlm, &1_000i128);
    client.create_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &500i128,
        &500i128,
        &0u64,
        &100u64,
        &false,
    );
    client.approve_treasury_budget(&admin, &String::from_str(&env, "ops"), &20u64, &300i128);

    // Draw down to the cap
    client.execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &300i128,
        &xlm,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "batch"),
        &Some(20u64),
    );

    // Any further spend is rejected
    let rejected = client.try_execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &1i128,
        &xlm,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "over"),
        &Some(20u64),
    );
    assert_eq!(rejected, Err(Ok(GovernanceError::ApprovedCapExceeded)));

    // New governance proposal approves another 150
    client.approve_treasury_budget(&admin, &String::from_str(&env, "ops"), &21u64, &150i128);

    // Spending is now allowed again
    let spend = client.execute_treasury_spend(
        &admin,
        &Address::generate(&env),
        &100i128,
        &xlm,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "renewed"),
        &Some(21u64),
    );
    assert_eq!(spend.amount, 100);

    let treasury = client.treasury();
    let approval = treasury
        .approved_budgets
        .get(String::from_str(&env, "ops"))
        .unwrap();
    assert_eq!(approval.proposal_id, 21);
    assert_eq!(approval.total_drawn, 100);
}

/// `approve_treasury_budget` on a non-existent category returns `BudgetNotFound`.
#[test]
fn approve_nonexistent_budget_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let result = client.try_approve_treasury_budget(
        &admin,
        &String::from_str(&env, "ghost"),
        &99u64,
        &100i128,
    );
    assert_eq!(result, Err(Ok(GovernanceError::BudgetNotFound)));
}

/// A recurring payment is deactivated when the approved cap is exhausted.
#[test]
fn recurring_payment_paused_when_approved_cap_exhausted() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let usdc = asset(&env, "USDC");
    client.set_treasury_asset(&admin, &usdc, &1_000i128);
    // Budget allows 400; governance only approved 150 (covers one 100-unit payment)
    client.create_budget(
        &admin,
        &String::from_str(&env, "grants"),
        &400i128,
        &200i128,
        &0u64,
        &100u64,
        &true,
    );
    client.approve_treasury_budget(&admin, &String::from_str(&env, "grants"), &30u64, &150i128);
    client.create_recurring_payment(
        &admin,
        &Address::generate(&env),
        &100i128,
        &usdc,
        &10u64,
        &String::from_str(&env, "grants"),
        &String::from_str(&env, "stipend"),
        &None,
        &Some(200u64),
    );

    // t=10: first payment — drawn 0 → 100 ≤ cap 150 ✓
    env.ledger().set_timestamp(10);
    assert_eq!(client.process_recurring_payments(&admin), 1);

    // t=20: second payment would push drawn to 200 > cap 150 → deactivated
    env.ledger().set_timestamp(20);
    assert_eq!(client.process_recurring_payments(&admin), 0);

    let treasury = client.treasury();
    assert!(!treasury.recurring_payments.get(0).unwrap().active);
}

/// Non-admin cannot call `approve_treasury_budget`.
#[test]
fn non_admin_cannot_approve_budget() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.create_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &500i128,
        &500i128,
        &0u64,
        &100u64,
        &false,
    );

    let non_admin = Address::generate(&env);
    let result = client.try_approve_treasury_budget(
        &non_admin,
        &String::from_str(&env, "ops"),
        &1u64,
        &100i128,
    );
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));
}

// ── Conviction Calibration tests ─────────────────────────────────────

#[test]
fn conviction_calibration_default_is_noop() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = client.conviction_calibration();
    assert_eq!(config.penalty_threshold_days, 0);
    assert_eq!(config.penalty_multiplier, 1);
    assert_eq!(config.reward_bonus_pct, 0);
    assert_eq!(config.max_conviction_cap, 0);
}

#[test]
fn conviction_calibration_admin_can_set_config() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 7,
        penalty_multiplier: 2,
        reward_bonus_pct: 10,
        max_conviction_cap: 50_000,
        decay_rate_bps: 50,
    };
    client.set_conviction_calibration(&admin, &config);

    let stored = client.conviction_calibration();
    assert_eq!(stored.penalty_threshold_days, 7);
    assert_eq!(stored.penalty_multiplier, 2);
    assert_eq!(stored.reward_bonus_pct, 10);
    assert_eq!(stored.max_conviction_cap, 50_000);
}

#[test]
fn conviction_calibration_non_admin_cannot_set_config() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let fake_admin = Address::generate(&env);
    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 3,
        penalty_multiplier: 4,
        reward_bonus_pct: 5,
        max_conviction_cap: 10_000,
        decay_rate_bps: 50,
    };

    let result = client.try_set_conviction_calibration(&fake_admin, &config);
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));
}

#[test]
fn conviction_calibration_rejects_invalid_multiplier() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 5,
        penalty_multiplier: 0,
        reward_bonus_pct: 0,
        max_conviction_cap: 0,
        decay_rate_bps: 50,
    };
    let result = client.try_set_conviction_calibration(&admin, &config);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCalibrationConfig)));
}
#[test]
fn conviction_calibration_rejects_invalid_reward_bonus() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 5,
        penalty_multiplier: 2,
        reward_bonus_pct: 101,
        max_conviction_cap: 0,
        decay_rate_bps: 50,
    };
    let result = client.try_set_conviction_calibration(&admin, &config);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCalibrationConfig)));
}

#[test]
fn conviction_calibration_penalty_short_votes() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 10,
        penalty_multiplier: 2,
        reward_bonus_pct: 0,
        max_conviction_cap: 0,
        decay_rate_bps: 50,
    };
    client.set_conviction_calibration(&admin, &config);

    let pool_id = client.create_conviction_pool(&admin, &100_000i128, &1_000i128, &86_400u64);
    let proposal_id = client.create_conviction_proposal(
        &pool_id,
        &recipients.community_rewards,
        &String::from_str(&env, "Test penalty"),
        &10_000i128,
        &recipients.public_sale,
    );

    client.vote_conviction(
        &pool_id,
        &proposal_id,
        &recipients.community_rewards,
        &1_000i128,
    );

    env.ledger().set_timestamp(5 * 86_400);
    let conviction = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert!(conviction > 0);
    assert_eq!(conviction, 1);
}

#[test]
fn conviction_calibration_reward_long_votes() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 10,
        penalty_multiplier: 1,
        reward_bonus_pct: 20,
        max_conviction_cap: 0,
        decay_rate_bps: 50,
    };
    client.set_conviction_calibration(&admin, &config);

    let pool_id = client.create_conviction_pool(&admin, &100_000i128, &1_000i128, &86_400u64);
    let proposal_id = client.create_conviction_proposal(
        &pool_id,
        &recipients.community_rewards,
        &String::from_str(&env, "Test reward"),
        &10_000i128,
        &recipients.public_sale,
    );

    // A large enough commitment that decay_rate_bps's 5%/day compounding
    // (see #837 conviction_voting overflow fix) still leaves a meaningful,
    // non-zero conviction after 15 days for the reward bonus to act on.
    client.vote_conviction(
        &pool_id,
        &proposal_id,
        &recipients.community_rewards,
        &100_000i128,
    );

    env.ledger().set_timestamp(15 * 86_400);
    let conviction = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert!(conviction >= 100);
}

#[test]
fn conviction_calibration_caps_max_conviction() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 0,
        penalty_multiplier: 1,
        reward_bonus_pct: 0,
        max_conviction_cap: 5,
        decay_rate_bps: 50,
    };
    client.set_conviction_calibration(&admin, &config);

    let pool_id = client.create_conviction_pool(&admin, &100_000i128, &1_000i128, &86_400u64);
    let proposal_id = client.create_conviction_proposal(
        &pool_id,
        &recipients.community_rewards,
        &String::from_str(&env, "Test cap"),
        &10_000i128,
        &recipients.public_sale,
    );

    // Large enough that pre-cap conviction (even after 100 days of 5%/day
    // decay) comfortably exceeds max_conviction_cap, so this actually
    // exercises the clamp rather than passing vacuously.
    client.vote_conviction(
        &pool_id,
        &proposal_id,
        &recipients.community_rewards,
        &1_000_000i128,
    );

    env.ledger().set_timestamp(100 * 86_400);
    let conviction = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert_eq!(conviction, 5);
}

#[test]
fn conviction_calibration_combination_penalty_and_reward() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 7,
        penalty_multiplier: 3,
        reward_bonus_pct: 10,
        max_conviction_cap: 0,
        decay_rate_bps: 50,
    };
    client.set_conviction_calibration(&admin, &config);

    let pool_id = client.create_conviction_pool(&admin, &100_000i128, &1_000i128, &86_400u64);
    let proposal_id = client.create_conviction_proposal(
        &pool_id,
        &recipients.community_rewards,
        &String::from_str(&env, "Test combo"),
        &10_000i128,
        &recipients.public_sale,
    );

    // Large enough that penalty vs. reward stays distinguishable from
    // decay_rate_bps's default 5%/day compounding decay over these windows.
    client.vote_conviction(
        &pool_id,
        &proposal_id,
        &recipients.community_rewards,
        &100_000i128,
    );

    env.ledger().set_timestamp(3 * 86_400);
    let conviction_short = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert_eq!(conviction_short, 57);

    env.ledger().set_timestamp(14 * 86_400);
    let conviction_long = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert!(conviction_long >= 100);
    assert!(conviction_long > conviction_short);
}

#[test]
fn conviction_calibration_zero_threshold_disables_penalty() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 0,
        penalty_multiplier: 2,
        reward_bonus_pct: 0,
        max_conviction_cap: 0,
        decay_rate_bps: 50,
    };
    client.set_conviction_calibration(&admin, &config);

    let pool_id = client.create_conviction_pool(&admin, &100_000i128, &1_000i128, &86_400u64);
    let proposal_id = client.create_conviction_proposal(
        &pool_id,
        &recipients.community_rewards,
        &String::from_str(&env, "Test no penalty"),
        &10_000i128,
        &recipients.public_sale,
    );

    client.vote_conviction(
        &pool_id,
        &proposal_id,
        &recipients.community_rewards,
        &10_000i128,
    );

    env.ledger().set_timestamp(86_400);
    let conviction = client.update_proposal_conviction(&pool_id, &proposal_id);
    assert_eq!(conviction, 9);
}

#[test]
fn voting_power_uses_snapshot_not_live_balance() {
    // Arrange: two voters with pre-existing stake
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let late_staker = recipients.public_sale.clone();
    client.stake(&recipients.community_rewards, &120_000_000i128);
    // late_staker has 0 staked at proposal creation time

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Snapshot test")),
        &String::from_str(&env, "Snapshot"),
        &String::from_str(
            &env,
            "Voting power must be snapshotted at proposal creation",
        ),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // late_staker stakes AFTER proposal creation — should not gain voting power on this proposal
    client.stake(&late_staker, &50_000_000i128);
    assert_eq!(client.staked_balance(&late_staker), 50_000_000);

    // Advance into the voting window
    env.ledger().set_timestamp(70);

    // community_rewards can vote (was snapshotted with 120_000_000)
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.votes_for, 120_000_000);

    // late_staker had 0 power at snapshot time — must be rejected with NoVotingPower
    let result = client.try_cast_vote(&proposal_id, &late_staker, &GovernanceVoteType::For);
    assert_eq!(result, Err(Ok(GovernanceError::NoVotingPower)));
}

#[test]
fn test_reputation_leaderboard_limit() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    use crate::reputation::MAX_REPUTATION_USERS;

    // Populate MAX_REPUTATION_USERS users
    for _ in 0..MAX_REPUTATION_USERS {
        let user = Address::generate(&env);
        client.refresh_reputation(&user);
    }

    // Leaderboard query at limit should succeed
    let res = client.try_reputation_leaderboard(&10);
    assert!(res.is_ok());

    // Add one more user (over limit)
    let user = Address::generate(&env);
    client.refresh_reputation(&user);

    // Leaderboard query should now fail with IterationLimitExceeded (mapped to InvalidCommitteeAction)
    let res = client.try_reputation_leaderboard(&10);
    assert_eq!(res, Err(Ok(GovernanceError::IterationLimitExceeded)));
}

// ── Proposal self-withdrawal tests ──────────────────────────────────────────

#[test]
fn proposer_can_withdraw_proposal_before_voting_opens() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "liquidity_reward_bps"), 100, 120),
        &String::from_str(&env, "Adjust reward"),
        &String::from_str(&env, "Increase by 20%"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // Still at t=0, well before voting_starts (t=60). Proposal is Pending.
    let status = client.withdraw_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(status, ProposalStatus::Withdrawn);

    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.status, ProposalStatus::Withdrawn);
}

#[test]
fn non_proposer_cannot_withdraw_others_proposal() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Test signal")),
        &String::from_str(&env, "Signal vote"),
        &String::from_str(&env, "Community signal"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // public_sale is NOT the proposer — must be rejected.
    let result = client.try_withdraw_proposal(&proposal_id, &recipients.public_sale);
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));

    // Original proposal must remain unchanged.
    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.status, ProposalStatus::Pending);
}

#[test]
fn cannot_withdraw_after_voting_has_started() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &40_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "liquidity_reward_bps"), 100, 120),
        &String::from_str(&env, "Adjust reward"),
        &String::from_str(&env, "Increase by 20%"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // Advance past voting_starts (t=60) and cast a vote → status becomes Active.
    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );

    // Now try to withdraw — must be rejected because status is Active.
    let result = client.try_withdraw_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalNotActive)));
}

#[test]
fn withdrawn_proposal_refunds_deposit() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposer = &recipients.community_rewards;
    let balance_before = client.balance(proposer);

    let proposal_id = client.create_proposal(
        proposer,
        &ProposalType::SignalProposal(String::from_str(&env, "Test")),
        &String::from_str(&env, "Deposit test"),
        &String::from_str(&env, "Verify deposit refund"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // Deposit was locked — balance should be reduced.
    let balance_after_create = client.balance(proposer);
    assert!(
        balance_after_create < balance_before,
        "balance must decrease after proposal creation due to deposit lock"
    );

    // Withdraw the proposal.
    client.withdraw_proposal(&proposal_id, proposer);

    // Deposit must be refunded — balance should return to original.
    let balance_after_withdraw = client.balance(proposer);
    assert_eq!(
        balance_after_withdraw, balance_before,
        "deposit must be fully refunded on self-withdrawal"
    );
}

#[test]
fn withdraw_proposal_emits_event() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Event test")),
        &String::from_str(&env, "Event check"),
        &String::from_str(&env, "Verify event emission"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    client.withdraw_proposal(&proposal_id, &recipients.community_rewards);

    // Verify that a (gov, propwdr) event was emitted.
    use soroban_sdk::TryFromVal;
    let events = env.events().all();
    let found = events.iter().any(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        let t0 = topics
            .get(0)
            .and_then(|v| Symbol::try_from_val(&env, &v).ok());
        let t1 = topics
            .get(1)
            .and_then(|v| Symbol::try_from_val(&env, &v).ok());
        t0 == Some(Symbol::new(&env, "gov")) && t1 == Some(Symbol::new(&env, "propwdr"))
    });
    assert!(found, "withdraw_proposal must emit (gov, propwdr) event");
}

#[test]
fn cannot_withdraw_already_withdrawn_proposal() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Double withdraw")),
        &String::from_str(&env, "Double withdraw test"),
        &String::from_str(&env, "Cannot withdraw twice"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // First withdrawal — succeeds.
    let status = client.withdraw_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(status, ProposalStatus::Withdrawn);

    // Second withdrawal — must be rejected (status is Withdrawn, not Pending).
    let result = client.try_withdraw_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalAlreadyWithdrawn)));
}

#[test]
fn withdrawn_proposals_excluded_from_voting_eligible_listings() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    // Create two proposals.
    let pid1 = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Keep this")),
        &String::from_str(&env, "Active proposal"),
        &String::from_str(&env, "Should remain"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );
    let pid2 = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Withdraw this")),
        &String::from_str(&env, "Withdrawn proposal"),
        &String::from_str(&env, "Should be withdrawn"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // Withdraw the second proposal.
    client.withdraw_proposal(&pid2, &recipients.community_rewards);

    // get_all_proposals should still return both, but the withdrawn one
    // has Withdrawn status (not Pending/Active) so it's excluded from
    // voting-eligible listings by status check.
    let all = client.proposals();
    assert_eq!(all.len(), 2);
    let p1 = all.get(0).unwrap();
    let p2 = all.get(1).unwrap();
    assert_eq!(p1.status, ProposalStatus::Pending);
    assert_eq!(p2.status, ProposalStatus::Withdrawn);
}

#[test]
fn cancelled_proposal_cannot_be_withdrawn() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "Cancel test")),
        &String::from_str(&env, "Cancel first"),
        &String::from_str(&env, "Then try withdraw"),
        &Bytes::new(&env),
        &crate::proposals::ProposalCategory::General,
        &false,
    );

    // Cancel the proposal first.
    client.cancel_proposal(&proposal_id, &recipients.community_rewards);

    // Now try to withdraw — must be rejected (status is Cancelled, not Pending).
    let result = client.try_withdraw_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalNotActive)));
}

// ── Decay rate validation tests ─────────────────────────────────────────────

#[test]
fn conviction_decay_rate_zero_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Attempt to set decay rate to 0 (below MIN_DECAY_RATE)
    let result = client.try_set_conviction_decay_rate(&admin, &0u64);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidDecayRate)));
}

#[test]
fn conviction_decay_rate_1000_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Attempt to set decay rate to 1000 (above MAX_DECAY_RATE)
    let result = client.try_set_conviction_decay_rate(&admin, &1000u64);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidDecayRate)));
}

#[test]
fn conviction_decay_rate_valid_min_accepted() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // MIN_DECAY_RATE (1) should be accepted
    let result = client.try_set_conviction_decay_rate(&admin, &1u64);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn conviction_decay_rate_valid_max_accepted() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // MAX_DECAY_RATE (999) should be accepted
    let result = client.try_set_conviction_decay_rate(&admin, &999u64);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn conviction_decay_rate_via_calibration_rejects_invalid() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    // Attempt to set calibration with decay_rate_bps = 0
    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 0,
        penalty_multiplier: 1,
        reward_bonus_pct: 0,
        max_conviction_cap: 0,
        decay_rate_bps: 0,
    };
    let result = client.try_set_conviction_calibration(&admin, &config);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidDecayRate)));

    // Attempt to set calibration with decay_rate_bps = 1000
    let config = crate::conviction_voting::ConvictionCalibration {
        penalty_threshold_days: 0,
        penalty_multiplier: 1,
        reward_bonus_pct: 0,
        max_conviction_cap: 0,
        decay_rate_bps: 1000,
    };
    let result = client.try_set_conviction_calibration(&admin, &config);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidDecayRate)));
}

#[test]
fn conviction_decay_rate_non_admin_cannot_set() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let fake_admin = Address::generate(&env);
    let result = client.try_set_conviction_decay_rate(&fake_admin, &50u64);
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));
}

#[test]
fn conviction_score_bounded_across_valid_decay_rates() {
    // Proptest-style verification: for all valid decay rates (1..=999),
    // conviction score should be in (0, max_conviction] after some time.
    use crate::conviction_voting::{
        calculate_conviction, ConvictionCalibration, MAX_DECAY_RATE, MIN_DECAY_RATE,
    };

    let tokens = 10_000i128;
    let time_elapsed = 30u64 * 86_400; // 30 days

    for decay_rate in MIN_DECAY_RATE..=MAX_DECAY_RATE {
        let calibration = ConvictionCalibration {
            penalty_threshold_days: 0,
            penalty_multiplier: 1,
            reward_bonus_pct: 0,
            max_conviction_cap: 0,
            decay_rate_bps: decay_rate,
        };

        let conviction = calculate_conviction(tokens, time_elapsed, &calibration);

        // Conviction should be non-negative
        assert!(
            conviction >= 0,
            "Conviction should be non-negative for decay_rate={}",
            decay_rate
        );

        // With 10,000 tokens and 30 days, sqrt(30) ≈ 5.47, so base conviction ≈ 54
        // After decay, it should be less than or equal to the base (no decay case)
        // The maximum possible conviction without decay would be tokens * sqrt(days) / 1000
        let max_possible = tokens * (30i128).checked_mul(1).unwrap_or(1) / 1000;
        assert!(
            conviction <= max_possible + 100,
            "Conviction {} exceeds max possible {} for decay_rate={}",
            conviction,
            max_possible,
            decay_rate
        );
    }
}

#[test]
fn conviction_decay_does_not_collapse_for_large_day_counts() {
    // Regression test: `decay_factor.saturating_pow(days)` and
    // `1000.saturating_pow(days)` both blew past i128::MAX after ~13 days
    // for any decay rate, and dividing the two saturated i128::MAX values
    // collapsed conviction to ~1 regardless of the true ratio or token
    // amount. `conviction_score_bounded_across_valid_decay_rates` above
    // didn't catch this because its bounds were loose enough for the
    // collapsed value to still pass. A correctly-computed decay must
    // actually track the configured rate, not converge on a constant.
    use crate::conviction_voting::{calculate_conviction, ConvictionCalibration};

    let days = 50u64;
    let time_elapsed = days * 86_400;
    let light_decay = ConvictionCalibration {
        penalty_threshold_days: 0,
        penalty_multiplier: 1,
        reward_bonus_pct: 0,
        max_conviction_cap: 0,
        decay_rate_bps: 1,
    };
    let heavy_decay = ConvictionCalibration {
        decay_rate_bps: 900,
        ..light_decay.clone()
    };

    let with_light_decay = calculate_conviction(1_000_000, time_elapsed, &light_decay);
    let with_heavy_decay = calculate_conviction(1_000_000, time_elapsed, &heavy_decay);

    assert!(
        with_light_decay > 6_000,
        "1 bps/day decay over {days} days should barely reduce conviction, got {with_light_decay}"
    );
    assert!(
        with_light_decay > with_heavy_decay,
        "light decay ({with_light_decay}) should exceed heavy decay ({with_heavy_decay}) after {days} days"
    );

    // No panic for a far larger day count than the old
    // `saturating_pow`-based implementation could handle at all.
    let huge_time_elapsed = 100_000u64 * 86_400;
    let result = calculate_conviction(1_000_000, huge_time_elapsed, &heavy_decay);
    assert!(result >= 0);
}

// ── Issue #796: Treasury spend proposal execution expiry ──────────────────────

/// Shared setup for the execution-expiry tests below: initializes the
/// contract, configures a non-zero `execution_delay` (so `finalize_proposal`
/// does not auto-execute the proposal), funds the treasury, stakes voting
/// power for two holders, creates a `TreasurySpend` proposal, votes it to
/// `Succeeded`, and returns the proposal id plus the recipient/asset used.
fn setup_succeeded_treasury_spend_proposal(
    env: &Env,
    client: &GovernanceContractClient<'_>,
    admin: &Address,
    recipients: &DistributionRecipients,
) -> (u64, Address, Asset) {
    let cfg = GovernanceConfig {
        min_proposal_threshold: 1_000,
        voting_period: 7 * 86_400,
        voting_delay: 60,
        quorum_threshold: 1_000,
        approval_threshold: 5_000,
        execution_delay: 60,
        discussion_duration: 0,
    };
    client.configure_governance(admin, &cfg);

    client.stake(&recipients.community_rewards, &120_000_000i128);
    client.stake(&recipients.public_sale, &40_000_000i128);

    let spend_asset = asset(env, "USDC");
    client.set_treasury_asset(admin, &spend_asset, &10_000i128);
    let payout_recipient = Address::generate(env);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::TreasurySpend(
            payout_recipient.clone(),
            500i128,
            spend_asset.clone(),
            String::from_str(env, "payout"),
        ),
        &String::from_str(env, "Fund payout"),
        &String::from_str(env, "treasury spend"),
        &Bytes::new(env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    client.cast_vote(
        &proposal_id,
        &recipients.public_sale,
        &GovernanceVoteType::For,
    );

    // voting_ends = 60 (delay) + 7 * 86_400 (period) = 604_860.
    env.ledger().set_timestamp(8 * 86_400);
    assert_eq!(
        client.finalize_proposal(&proposal_id),
        ProposalStatus::Succeeded
    );

    (proposal_id, payout_recipient, spend_asset)
}

#[test]
fn treasury_spend_proposal_executes_within_execution_window() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let (proposal_id, payout_recipient, spend_asset) =
        setup_succeeded_treasury_spend_proposal(&env, &client, &admin, &recipients);

    // Still well inside the 7-day execution window (voting_ends + 7 days).
    let status = client.execute_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(status, ProposalStatus::Executed);

    let treasury = client.treasury();
    assert_eq!(treasury.assets.get(spend_asset).unwrap(), 9_500);
    assert_eq!(client.balance(&payout_recipient), 500);
}

#[test]
fn treasury_spend_proposal_execution_past_deadline_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let (proposal_id, _payout_recipient, _spend_asset) =
        setup_succeeded_treasury_spend_proposal(&env, &client, &admin, &recipients);

    // execution_deadline = voting_ends (604_860) + EXECUTION_WINDOW (7 days) = 1_209_660.
    // Jump well past it.
    env.ledger().set_timestamp(1_209_661);

    let result = client.try_execute_proposal(&proposal_id, &recipients.community_rewards);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));

    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.status, ProposalStatus::Expired);
}

#[test]
fn reclaim_expired_proposal_removes_entry_and_is_callable_by_anyone() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let (proposal_id, _payout_recipient, _spend_asset) =
        setup_succeeded_treasury_spend_proposal(&env, &client, &admin, &recipients);

    // Too early: execution window hasn't closed yet.
    let too_early = client.try_reclaim_expired_proposal(&proposal_id, &recipients.public_sale);
    assert_eq!(too_early, Err(Ok(GovernanceError::InvalidDuration)));

    env.ledger().set_timestamp(1_209_661);

    // Any address (not just admin) may reclaim once the deadline has passed.
    let random_caller = Address::generate(&env);
    client.reclaim_expired_proposal(&proposal_id, &random_caller);

    // The proposal entry is gone entirely.
    let missing = client.try_proposal(&proposal_id);
    assert_eq!(missing, Err(Ok(GovernanceError::ProposalNotFound)));

    // It no longer shows up in the active-proposals listing either.
    let active = client.get_active_proposals();
    let mut i = 0;
    let mut found = false;
    while i < active.len() {
        if active.get(i).unwrap().id == proposal_id {
            found = true;
        }
        i += 1;
    }
    assert!(!found);
}

#[test]
fn error_messages_are_non_empty_and_distinct() {
    // A representative sample of raw variants plus at least one alias const
    // (Issue #883). Aliases resolve to their target variant's value, so
    // `ContractPaused` and `Unauthorized` intentionally share a message —
    // see `GovernanceError::message()`'s doc comment for why.
    let samples = [
        GovernanceError::NotInitialized,
        GovernanceError::Unauthorized,
        GovernanceError::ProposalNotFound,
        GovernanceError::VotingEnded,
        GovernanceError::BudgetExceeded,
        GovernanceError::ContractPaused, // alias const → Unauthorized
    ];
    for err in samples.iter() {
        assert!(!err.message().is_empty());
    }
    assert_eq!(
        GovernanceError::Unauthorized.message(),
        GovernanceError::ContractPaused.message(),
        "alias consts share their target variant's runtime value and message"
    );

    let distinct = [
        GovernanceError::NotInitialized,
        GovernanceError::Unauthorized,
        GovernanceError::ProposalNotFound,
        GovernanceError::VotingEnded,
        GovernanceError::BudgetExceeded,
    ];
    for i in 0..distinct.len() {
        for j in (i + 1)..distinct.len() {
            assert_ne!(
                distinct[i].message(),
                distinct[j].message(),
                "expected distinct messages for {:?} and {:?}",
                distinct[i],
                distinct[j]
            );
        }
    }
}

// ── Issue #998: vote accounting resistant to duplicate participation ────────

#[test]
fn duplicate_vote_on_proposal_is_rejected_and_tally_unchanged() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "dedup")),
        &String::from_str(&env, "Dedup"),
        &String::from_str(&env, "A voter may not be counted twice"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );

    let after_first = client.proposal(&proposal_id);
    assert_eq!(after_first.votes_for, 120_000_000);
    assert_eq!(after_first.voter_list.len(), 1);

    // Repeated submission attempt (e.g. a client retry) must be rejected
    // without mutating the tally.
    let result = client.try_cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyVoted)));

    // A duplicate attempt casting the *opposite* choice must also be
    // rejected — the recorded participation, not just the chosen side,
    // is what's checked.
    let opposite = client.try_cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::Against,
    );
    assert_eq!(opposite, Err(Ok(GovernanceError::AlreadyVoted)));

    let after_retries = client.proposal(&proposal_id);
    assert_eq!(after_retries.votes_for, 120_000_000, "tally must be unchanged");
    assert_eq!(after_retries.votes_against, 0, "tally must be unchanged");
    assert_eq!(
        after_retries.voter_list.len(),
        1,
        "exactly one participation record per voter"
    );
}

#[test]
fn late_vote_on_proposal_is_rejected_with_no_tally_change() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &120_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "late")),
        &String::from_str(&env, "Late"),
        &String::from_str(&env, "Votes after voting_ends must be rejected"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // voting_ends = voting_delay(60) + voting_period(7d) = 604_860.
    env.ledger().set_timestamp(604_860);
    let result = client.try_cast_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    assert_eq!(result, Err(Ok(GovernanceError::VotingEnded)));

    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.votes_for, 0);
    assert_eq!(proposal.votes_against, 0);
    assert_eq!(proposal.votes_abstain, 0);
    assert_eq!(proposal.voter_list.len(), 0);
}

#[test]
fn delegation_change_after_snapshot_does_not_alter_counted_or_future_votes() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let delegator = recipients.community_rewards.clone();
    let delegate = recipients.public_sale.clone();
    client.stake(&delegator, &100_000_000i128);
    client.stake(&delegate, &40_000_000i128);

    // Snapshot taken here: delegator's own power (100_000_000) and
    // delegate's own power (40_000_000) — no delegation active yet.
    let proposal_id = client.create_proposal(
        &delegate,
        &ProposalType::SignalProposal(String::from_str(&env, "deleg")),
        &String::from_str(&env, "Delegation"),
        &String::from_str(
            &env,
            "Delegation changes after snapshot must not alter counted votes",
        ),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // Delegate voting power *after* the proposal (and its snapshot) was
    // created — this must not retroactively inflate the delegate's weight
    // on this proposal, nor strip the delegator's own frozen weight.
    client.delegate_voting_power(&delegator, &delegate);

    env.ledger().set_timestamp(70);

    // The delegate votes with only their pre-delegation snapshotted power,
    // not the live effective power (which now also includes the delegated
    // 100_000_000).
    client.cast_vote(&proposal_id, &delegate, &GovernanceVoteType::For);
    let after_delegate_vote = client.proposal(&proposal_id);
    assert_eq!(
        after_delegate_vote.votes_for, 40_000_000,
        "delegate must be counted with their snapshotted power only, \
         not power delegated after proposal creation"
    );

    // The delegator can still cast their own frozen vote on this proposal —
    // an active delegation at *vote time* does not retroactively erase the
    // snapshot recorded at proposal creation.
    client.cast_vote(&proposal_id, &delegator, &GovernanceVoteType::Against);
    let after_both_votes = client.proposal(&proposal_id);
    assert_eq!(after_both_votes.votes_for, 40_000_000);
    assert_eq!(
        after_both_votes.votes_against, 100_000_000,
        "delegator's own snapshotted power must still be usable"
    );
    assert_eq!(after_both_votes.voter_list.len(), 2);
}

#[test]
fn reputation_weighted_vote_duplicate_is_rejected_and_tally_unchanged() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &50_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "rep-dedup")),
        &String::from_str(&env, "Rep dedup"),
        &String::from_str(&env, "Reputation-weighted votes must dedup too"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    env.ledger().set_timestamp(70);
    client.cast_reputation_weighted_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    let after_first = client.proposal(&proposal_id);
    assert_eq!(after_first.votes_for, 50_000_000);

    let result = client.try_cast_reputation_weighted_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyVoted)));

    let after_retry = client.proposal(&proposal_id);
    assert_eq!(after_retry.votes_for, 50_000_000, "tally must be unchanged");
    assert_eq!(after_retry.voter_list.len(), 1);
}

#[test]
fn reputation_weighted_vote_after_voting_ends_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &50_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "rep-late")),
        &String::from_str(&env, "Rep late"),
        &String::from_str(
            &env,
            "Reputation-weighted votes must respect the voting window",
        ),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // Past voting_ends (604_860): must be rejected, not silently accepted.
    env.ledger().set_timestamp(604_860);
    let result = client.try_cast_reputation_weighted_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    assert_eq!(result, Err(Ok(GovernanceError::VotingEnded)));

    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.votes_for, 0);
    assert_eq!(proposal.voter_list.len(), 0);
}

#[test]
fn reputation_weighted_vote_on_cancelled_proposal_is_rejected() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    client.stake(&recipients.community_rewards, &50_000_000i128);

    let proposal_id = client.create_proposal(
        &recipients.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "rep-cancel")),
        &String::from_str(&env, "Rep cancel"),
        &String::from_str(
            &env,
            "Reputation-weighted votes must not land on inactive proposals",
        ),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // Proposer cancels while still Pending (before the voting window ends).
    client.cancel_proposal(&proposal_id, &recipients.community_rewards);

    env.ledger().set_timestamp(70);
    let result = client.try_cast_reputation_weighted_vote(
        &proposal_id,
        &recipients.community_rewards,
        &GovernanceVoteType::For,
    );
    assert_eq!(result, Err(Ok(GovernanceError::ProposalNotActive)));

    let proposal = client.proposal(&proposal_id);
    assert_eq!(proposal.votes_for, 0);
    assert_eq!(proposal.voter_list.len(), 0);
}

#[test]
fn reputation_weighted_vote_uses_snapshotted_power_not_live_delegation() {
    let (env, contract_id, admin, recipients) = setup();
    let client = client(&env, &contract_id);
    initialize(&client, &env, &admin, &recipients);

    let delegator = recipients.community_rewards.clone();
    let delegate = recipients.public_sale.clone();
    client.stake(&delegator, &100_000_000i128);
    client.stake(&delegate, &40_000_000i128);

    // Snapshot taken at creation: delegate's own power is 40_000_000, with
    // no delegation active yet.
    let proposal_id = client.create_proposal(
        &delegate,
        &ProposalType::SignalProposal(String::from_str(&env, "rep-snapshot")),
        &String::from_str(&env, "Rep snapshot"),
        &String::from_str(
            &env,
            "Reputation-weighted votes must use the frozen snapshot",
        ),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    // Delegating *after* the snapshot must not let the delegate inflate
    // their counted weight on this proposal by picking up live power.
    client.delegate_voting_power(&delegator, &delegate);

    env.ledger().set_timestamp(70);
    client.cast_reputation_weighted_vote(&proposal_id, &delegate, &GovernanceVoteType::For);

    let proposal = client.proposal(&proposal_id);
    assert_eq!(
        proposal.votes_for, 40_000_000,
        "reputation-weighted vote must use the pre-delegation snapshot, \
         not the live effective power (which now includes the delegated \
         100_000_000)"
    );
}
