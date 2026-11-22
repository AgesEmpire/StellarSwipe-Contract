/// Tests: #1429 — bound treasury multi-action proposal execution.
///
/// Covers:
///   1. A batch at the action count limit is accepted.
///   2. A batch one above the action count limit is rejected before any deposit.
///   3. A batch at the work limit is accepted.
///   4. A batch one above the work limit is rejected.
///   5. Non-treasury actions do not count against either limit.
extern crate std;

use crate::distribution::DistributionRecipients;
use crate::proposals::{
    ProposalType, MAX_TREASURY_ACTIONS_PER_BATCH, MAX_TREASURY_BATCH_WORK,
};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String, Vec};
use stellar_swipe_common::Asset;

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

fn init(c: &GovernanceContractClient, env: &Env, admin: &Address, r: &DistributionRecipients) {
    c.initialize(
        admin,
        &String::from_str(env, "StellarSwipe Gov"),
        &String::from_str(env, "SSG"),
        &7u32,
        &SUPPLY,
        r,
    );
}

fn usdc(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "USDC"),
        issuer: None,
    }
}

fn treasury_action(env: &Env, amount: i128) -> ProposalType {
    ProposalType::TreasurySpend(
        Address::generate(env),
        amount,
        usdc(env),
        String::from_str(env, "batch item"),
    )
}

/// A batch exactly at the action-count limit is accepted.
#[test]
fn batch_at_action_count_limit_is_accepted() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let mut actions: Vec<ProposalType> = Vec::new(&env);
    let mut i = 0u32;
    while i < MAX_TREASURY_ACTIONS_PER_BATCH {
        actions.push_back(treasury_action(&env, 1));
        i += 1;
    }

    let result = c.try_validate_treasury_batch(&actions);
    assert!(
        result.is_ok(),
        "batch at action count limit must be accepted"
    );
}

/// A batch one above the action-count limit is rejected.
#[test]
fn batch_one_above_action_count_limit_is_rejected() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let mut actions: Vec<ProposalType> = Vec::new(&env);
    let mut i = 0u32;
    while i < MAX_TREASURY_ACTIONS_PER_BATCH + 1 {
        actions.push_back(treasury_action(&env, 1));
        i += 1;
    }

    let result = c.try_validate_treasury_batch(&actions);
    assert!(
        result.is_err(),
        "batch one above action count limit must be rejected"
    );
}

/// A batch whose aggregate work is exactly at the limit is accepted.
#[test]
fn batch_at_work_limit_is_accepted() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    // Single action spending exactly the work limit.
    let mut actions: Vec<ProposalType> = Vec::new(&env);
    actions.push_back(treasury_action(&env, MAX_TREASURY_BATCH_WORK));

    let result = c.try_validate_treasury_batch(&actions);
    assert!(result.is_ok(), "batch at work limit must be accepted");
}

/// A batch whose aggregate work exceeds the limit is rejected.
#[test]
fn batch_above_work_limit_is_rejected() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let mut actions: Vec<ProposalType> = Vec::new(&env);
    actions.push_back(treasury_action(&env, MAX_TREASURY_BATCH_WORK + 1));

    let result = c.try_validate_treasury_batch(&actions);
    assert!(
        result.is_err(),
        "batch exceeding work limit must be rejected"
    );
}

/// Non-treasury actions do not count against the action-count or work limits.
#[test]
fn non_treasury_actions_do_not_count_against_limits() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    // Fill with signal proposals (non-treasury) well beyond the treasury limit.
    let mut actions: Vec<ProposalType> = Vec::new(&env);
    let mut i = 0u32;
    while i < MAX_TREASURY_ACTIONS_PER_BATCH * 2 {
        actions.push_back(ProposalType::SignalProposal(String::from_str(&env, "sig")));
        i += 1;
    }

    let result = c.try_validate_treasury_batch(&actions);
    assert!(
        result.is_ok(),
        "non-treasury actions must not be counted against treasury limits"
    );
}
