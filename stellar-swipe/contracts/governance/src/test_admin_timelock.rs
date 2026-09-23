extern crate std;

use crate::distribution::DistributionRecipients;
use crate::proposals::GovernanceConfig;
use crate::{Authority, GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger as _};
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
    let result = c.try_treasury_spend_timelocked(
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

// ── queue_committee_override ─────────────────────────────────────────────────

#[test]
fn override_committee_decision_timelocked_rejects_without_queue() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let result = c.try_committee_override_timelocked(&admin, &0u64, &1u64, &1u64);
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

// ── Issue #942: admin-timelock enforcement latch ─────────────────────────────
//
// Once `enforce_admin_timelock` is latched on, every category (b) admin entry
// point must reject a *direct* call with `TimelockBypassBlocked`; the action
// can then only go through its `queue_*` + `*_timelocked` pair. Category (c)
// functions (emergency pause, low-risk operational setters) stay callable.

/// Assert that a `try_*` client call was rejected by the admin-timelock
/// enforcement latch (`GovernanceError::TimelockBypassBlocked`).
fn assert_bypass<R: core::fmt::Debug, E: core::fmt::Debug>(
    r: Result<Result<R, E>, Result<GovernanceError, soroban_sdk::InvokeError>>,
) {
    match r {
        Err(Ok(GovernanceError::TimelockBypassBlocked)) => {}
        other => panic!("expected TimelockBypassBlocked, got {other:?}"),
    }
}

#[test]
fn enforce_admin_timelock_is_latched_and_readable() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);

    assert!(!c.is_admin_timelock_enforced());
    c.enforce_admin_timelock(&admin);
    assert!(c.is_admin_timelock_enforced());

    // Idempotent — enabling again is a no-op, not an error.
    c.enforce_admin_timelock(&admin);
    assert!(c.is_admin_timelock_enforced());
}

#[test]
fn enforce_admin_timelock_rejects_non_admin() {
    let (env, contract_id, _admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let stranger = Address::generate(&env);

    let result = c.try_enforce_admin_timelock(&stranger);
    assert_eq!(result, Err(Ok(GovernanceError::Unauthorized)));
    assert!(!c.is_admin_timelock_enforced());
}

#[test]
fn direct_set_treasury_asset_allowed_before_enforcement() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");

    // Enforcement off (default) — direct call still works.
    assert!(c.try_set_treasury_asset(&admin, &usdc, &1_000i128).is_ok());
}

#[test]
fn direct_set_treasury_asset_blocked_after_enforcement() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");

    c.enforce_admin_timelock(&admin);

    assert_bypass(c.try_set_treasury_asset(&admin, &usdc, &1_000i128));
}

#[test]
fn timelocked_path_still_works_after_enforcement() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    let usdc = asset(&env, "USDC");

    c.enforce_admin_timelock(&admin);

    // The queue + timelocked-execute pair is unaffected by the latch.
    let id = c.queue_set_treasury_asset(&admin, &usdc, &1_000i128);
    env.ledger().set_timestamp(TWO_DAYS + 1);
    assert!(c
        .try_set_treasury_asset_timelocked(&admin, &id, &usdc, &1_000i128)
        .is_ok());
}

#[test]
fn enforcement_blocks_every_category_b_direct_entry_point() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    c.enforce_admin_timelock(&admin);

    let usdc = asset(&env, "USDC");
    let other = Address::generate(&env);
    let cfg = GovernanceConfig {
        min_proposal_threshold: 100,
        voting_period: 86_400,
        voting_delay: 0,
        quorum_threshold: 1_000,
        approval_threshold: 6_667,
        execution_delay: 0,
        discussion_duration: 0,
    };

    assert_bypass(c.try_set_treasury_asset(&admin, &usdc, &1i128));
    assert_bypass(c.try_execute_treasury_spend(
        &admin,
        &other,
        &1i128,
        &usdc,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "p"),
        &None::<u64>,
    ));
    assert_bypass(c.try_create_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &10i128,
        &5i128,
        &0u64,
        &1_000u64,
        &false,
    ));
    assert_bypass(c.try_approve_treasury_budget(
        &admin,
        &String::from_str(&env, "ops"),
        &1u64,
        &5i128,
    ));
    assert_bypass(c.try_create_recurring_payment(
        &admin,
        &other,
        &1i128,
        &usdc,
        &86_400u64,
        &String::from_str(&env, "ops"),
        &String::from_str(&env, "p"),
        &None::<u64>,
        &None::<u64>,
    ));
    assert_bypass(c.try_configure_governance(&admin, &cfg));
    assert_bypass(c.try_set_category_thresholds(
        &admin,
        &crate::proposals::ProposalCategory::General,
        &crate::proposals::CategoryThreshold {
            quorum_bps: 1_000,
            supermajority_bps: 5_000,
        },
    ));
    assert_bypass(c.try_set_guardian(&admin, &other));
    assert_bypass(c.try_grant_capability(&admin, &other, &crate::capabilities::Capability::Pause));
    assert_bypass(c.try_revoke_capability(&admin, &other, &crate::capabilities::Capability::Pause));
    assert_bypass(c.try_create_committee(
        &admin,
        &String::from_str(&env, "Treasury"),
        &String::from_str(&env, "d"),
        &Vec::<Address>::new(&env),
        &other,
        &5u32,
        &Vec::<Authority>::new(&env),
        &None::<u32>,
    ));
    assert_bypass(c.try_dissolve_committee(&admin, &1u64));
    assert_bypass(c.try_override_committee_decision(&admin, &1u64, &1u64));
    assert_bypass(c.try_set_rebalance_target(&admin, &usdc, &5_000i128));
    assert_bypass(c.try_update_timelock_delay(
        &admin,
        &crate::timelock::ActionType::TreasurySpend,
        &7_200u64,
    ));
    assert_bypass(
        c.try_create_vesting_schedule(&admin, &other, &1_000i128, &0u64, &0u64, &86_400u64),
    );
    assert_bypass(c.try_enter_shadow_mode(
        &admin,
        &soroban_sdk::Bytes::from_array(&env, &[7u8; 32]),
        &3_600u64,
    ));
    assert_bypass(c.try_promote_from_shadow_mode(&admin));
}

#[test]
fn enforcement_leaves_category_c_functions_callable() {
    let (env, contract_id, admin, _) = setup_with_timelock();
    let c = client(&env, &contract_id);
    c.enforce_admin_timelock(&admin);

    // Emergency pause must stay instant.
    assert!(c.try_set_contract_paused(&admin, &true).is_ok());
    assert!(c.try_set_contract_paused(&admin, &false).is_ok());

    // Pause-target registration (pause infrastructure) stays callable.
    assert!(c
        .try_register_pause_target(&admin, &Address::generate(&env))
        .is_ok());

    // Key-rotation proposal (does not execute the change) stays callable.
    assert!(c
        .try_propose_key_rotation(&admin, &Address::generate(&env))
        .is_ok());
}
