extern crate std;

use crate::distribution::DistributionRecipients;
use crate::proposals::GovernanceConfig;
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String, Vec};
use stellar_swipe_common::Asset;

const SUPPLY: i128 = 1_000_000_000;
const TWO_DAYS: u64 = 2 * 86_400;

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

fn initialize_gov(
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

fn setup_with_timelock() -> (Env, Address, Address, DistributionRecipients) {
    let (env, contract_id, admin, recipients) = setup();
    let c = client(&env, &contract_id);
    initialize_gov(&c, &env, &admin, &recipients);
    let guardian = Address::generate(&env);
    c.initialize_timelock(&admin, &3600u64, &(7 * 86_400), &guardian);
    (env, contract_id, admin, recipients)
}

fn asset(env: &Env, code: &str) -> Asset {
    Asset {
        code: String::from_str(env, code),
        issuer: None,
    }
}

// ── queue_set_treasury_asset ─────────────────────────────────────────────────

#[test]
fn queue_set_treasury_asset_returns_id() {
    let (env, contract_id, admin, recipients) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);
    assert!(id > 0);
}

#[test]
fn set_treasury_asset_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let result = c.try_set_treasury_asset_timelocked(&admin, &0u64, &usdc, &1000i128);
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

#[test]
fn set_treasury_asset_timelocked_rejects_before_delay() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);

    // Try to execute immediately — should fail
    let result = c.try_set_treasury_asset_timelocked(&admin, &id, &usdc, &1000i128);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidDuration)));
}

#[test]
fn set_treasury_asset_timelocked_succeeds_after_delay() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);

    env.ledger().set_timestamp(TWO_DAYS + 1);
    let result = c.try_set_treasury_asset_timelocked(&admin, &id, &usdc, &1000i128);
    assert!(result.is_ok());
}

#[test]
fn set_treasury_asset_timelocked_rejects_wrong_caller() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let other = Address::generate(&env);
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);

    env.ledger().set_timestamp(TWO_DAYS + 1);
    let result = c.try_set_treasury_asset_timelocked(&other, &id, &usdc, &1000i128);
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));
}

#[test]
fn set_treasury_asset_timelocked_rejects_double_execute() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);

    env.ledger().set_timestamp(TWO_DAYS + 1);
    let _ = c.set_treasury_asset_timelocked(&admin, &id, &usdc, &1000i128);
    let result = c.try_set_treasury_asset_timelocked(&admin, &id, &usdc, &2000i128);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCommitteeAction)));
}

// ── queue_execute_treasury_spend ─────────────────────────────────────────────

#[test]
fn execute_treasury_spend_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let recipient = Address::generate(&env);
    let usdc = asset(&env, "USDC");
    let result = c.try_execute_treasury_spend_timelocked(
        &admin,
        &0u64,
        &recipient,
        &100i128,
        &usdc,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "test"),
        &None::<u64>,
    );
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

// ── queue_configure_governance ───────────────────────────────────────────────

#[test]
fn configure_governance_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let config = GovernanceConfig {
        min_proposal_threshold: 100,
        voting_period: 86400,
        voting_delay: 0,
        quorum_threshold: 1000,
        approval_threshold: 6667,
        execution_delay: 0,
        discussion_duration: 0,
    };
    let result = c.try_configure_governance_timelocked(&admin, &0u64, &config);
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

#[test]
fn configure_governance_timelocked_succeeds_after_delay() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let config = GovernanceConfig {
        min_proposal_threshold: 100,
        voting_period: 86400,
        voting_delay: 0,
        quorum_threshold: 1000,
        approval_threshold: 6667,
        execution_delay: 0,
        discussion_duration: 0,
    };
    let id = c.queue_configure_governance(&admin, &config);
    env.ledger().set_timestamp(TWO_DAYS + 1);
    let result = c.try_configure_governance_timelocked(&admin, &id, &config);
    assert!(result.is_ok());
}

// ── queue_set_guardian ───────────────────────────────────────────────────────

#[test]
fn set_guardian_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let guardian = Address::generate(&env);
    let result = c.try_set_guardian_timelocked(&admin, &0u64, &guardian);
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

#[test]
fn set_guardian_timelocked_succeeds_after_delay() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let guardian = Address::generate(&env);
    let id = c.queue_set_guardian(&admin, &guardian);
    env.ledger().set_timestamp(TWO_DAYS + 1);
    let result = c.try_set_guardian_timelocked(&admin, &id, &guardian);
    assert!(result.is_ok());
}

// ── queue_create_committee ───────────────────────────────────────────────────

#[test]
fn create_committee_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let result = c.try_create_committee_timelocked(
        &admin,
        &0u64,
        &String::from_str(&env, "Treasury"),
        &String::from_str(&env, "desc"),
        &Vec::<Address>::new(&env),
        &Address::generate(&env),
        &5u32,
        &Vec::<Authority>::new(&env),
        &None::<u32>,
    );
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

// ── queue_dissolve_committee ─────────────────────────────────────────────────

#[test]
fn dissolve_committee_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let result = c.try_dissolve_committee_timelocked(&admin, &0u64, &1u64);
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

// ── queue_override_committee_decision ────────────────────────────────────────

#[test]
fn override_committee_decision_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let result = c.try_override_committee_decision_timelocked(&admin, &0u64, &1u64, &1u64);
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

// ── queue_grant_capability ───────────────────────────────────────────────────

#[test]
fn grant_capability_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let target = Address::generate(&env);
    let result = c.try_grant_capability_timelocked(
        &admin,
        &0u64,
        &target,
        &crate::capabilities::Capability::Pause,
    );
    assert_eq!(result, Err(Ok(GovernanceError::ActionNotFound)));
}

// ── cancel_admin_action ──────────────────────────────────────────────────────

#[test]
fn cancel_admin_action_prevents_execution() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);

    c.cancel_admin_action(&id, &admin);

    env.ledger().set_timestamp(TWO_DAYS + 1);
    let result = c.try_set_treasury_asset_timelocked(&admin, &id, &usdc, &1000i128);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidCommitteeAction)));
}

#[test]
fn cancel_admin_action_rejects_non_admin() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let other = Address::generate(&env);
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);

    let result = c.try_cancel_admin_action(&id, &other);
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));
}

// ── admin_pending_actions ────────────────────────────────────────────────────

#[test]
fn admin_pending_actions_lists_queued() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");
    let _ = c.queue_set_treasury_asset(&admin, &usdc, &1000i128);
    let _ = c.queue_set_guardian(&admin, &Address::generate(&env));

    let pending = c.admin_pending_actions();
    assert_eq!(pending.len(), 2);
}
