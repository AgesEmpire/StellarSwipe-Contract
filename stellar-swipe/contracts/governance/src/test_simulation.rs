/// Tests: dry-run execution simulation for governance proposals (#917).
///
/// Covers:
///   1. Successful simulation of each ProposalType
///   2. Simulating a proposal that would fail (insufficient treasury balance)
///   3. Simulating a non-existent proposal returns ProposalNotFound
///   4. Simulation does NOT mutate stored state
extern crate std;

use crate::distribution::DistributionRecipients;
use crate::proposals::{
    ProposalCategory, ProposalStatus, ProposalType, SimulationEffect,
};
use crate::shadow_mode::ShadowModeResult;
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, Address, Bytes, Env, String, Symbol, TryFromVal, Val, Vec};

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

fn stake_tokens(c: &GovernanceContractClient, user: &Address, amount: i128) {
    c.stake(user, &amount);
}

// -- #917: simulate a SignalProposal

#[test]
fn simulate_signal_proposal_returns_effects_without_mutating() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "dry-run test")),
        &String::from_str(&env, "Dry Run"),
        &String::from_str(&env, "Testing simulation"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let result = c.simulate_proposal(&pid);
    assert!(result.success, "signal proposal simulation should succeed");
    assert_eq!(
        result.error,
        String::from_str(&env, ""),
        "no error expected"
    );
    assert!(
        result.effects.len() >= 1,
        "expected at least 1 effect, got {}",
        result.effects.len()
    );
    let effect = result.effects.get(0).unwrap();
    assert_eq!(effect.key, String::from_str(&env, "signal"));

    let stored = c.proposal(&pid);
    assert_eq!(
        stored.status,
        ProposalStatus::Pending,
        "simulation must not mutate proposal status"
    );
}

// -- #917: simulate a ParameterChange proposal

#[test]
fn simulate_parameter_change_proposal_reports_current_and_proposed() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let param_name = String::from_str(&env, "max_fee");
    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::ParameterChange(param_name.clone(), 0i128, 500i128),
        &String::from_str(&env, "Set max fee"),
        &String::from_str(&env, "Change max fee to 500"),
        &Bytes::new(&env),
        &ProposalCategory::ParameterChange,
        &false,
    );

    let result = c.simulate_proposal(&pid);
    assert!(result.success, "parameter change simulation should succeed");

    let found = result
        .effects
        .iter()
        .any(|eff| eff.key == String::from_str(&env, "parameter:max_fee"));
    assert!(found, "expected effect key 'parameter:max_fee'");
}

// -- #917: TreasurySpend with insufficient balance reports failure

#[test]
fn simulate_treasury_spend_insufficient_balance_reports_failure() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let asset = stellar_swipe_common::Asset {
        code: String::from_str(&env, "USDC"),
        issuer: None,
    };
    // create_proposal requires the treasury to already hold >= 10x the spend
    // amount at proposal time; fund it here, then drain it below so the
    // balance is insufficient by the time simulation runs.
    c.set_treasury_asset(&admin, &asset, &10_000i128);
    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::TreasurySpend(
            Address::generate(&env),
            1_000i128,
            asset.clone(),
            String::from_str(&env, "test spend"),
        ),
        &String::from_str(&env, "Spend USDC"),
        &String::from_str(&env, "Attempt to spend from empty treasury"),
        &Bytes::new(&env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );
    c.set_treasury_asset(&admin, &asset, &0i128);

    let result = c.simulate_proposal(&pid);
    assert!(!result.success, "simulation should report failure");
    assert!(
        result.effects.len() == 0,
        "no effects expected when simulation fails early"
    );
}

// -- #917: non-existent proposal returns ProposalNotFound

#[test]
fn simulate_nonexistent_proposal_returns_error() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let result = c.try_simulate_proposal(&999_999u64);
    assert_eq!(
        result,
        Err(Ok(GovernanceError::ProposalNotFound)),
        "simulating a non-existent proposal must return ProposalNotFound"
    );
}

// -- #917: FeatureToggle proposal

#[test]
fn simulate_feature_toggle_reports_effect() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let feature = String::from_str(&env, "flash_loans");
    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::FeatureToggle(feature.clone(), true),
        &String::from_str(&env, "Enable flash loans"),
        &String::from_str(&env, "Toggle feature flag"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let result = c.simulate_proposal(&pid);
    assert!(result.success, "feature toggle simulation should succeed");

    let found = result
        .effects
        .iter()
        .any(|eff| eff.key == String::from_str(&env, "feature:flash_loans"));
    assert!(found, "expected effect key 'feature:flash_loans'");
}

// -- #917: simulation is read-only (multiple calls produce same result)

#[test]
fn simulation_is_read_only_no_storage_writes() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::SignalProposal(String::from_str(&env, "read-only test")),
        &String::from_str(&env, "T"),
        &String::from_str(&env, "D"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let r1 = c.simulate_proposal(&pid);
    assert!(r1.success);

    let r2 = c.simulate_proposal(&pid);
    assert!(r2.success);
    assert_eq!(
        r1.effects.len(),
        r2.effects.len(),
        "repeated simulations must return the same number of effects"
    );

    let stored = c.proposal(&pid);
    assert_eq!(
        stored.status,
        ProposalStatus::Pending,
        "simulation must not change proposal status even after multiple calls"
    );
}

// ── Shadow-mode event emission tests ────────────────────────────────────────

/// Helper: verify that a `shadow/simres` event was emitted and return
/// the decoded [`ShadowModeResult`] data.
fn find_shadow_sim_result_event(env: &Env) -> ShadowModeResult {
    let events = env.events().all();
    let mut found_data: Option<ShadowModeResult> = None;
    for (_, topics, data) in events.iter() {
        let t0 = topics
            .get(0)
            .and_then(|v: Val| Symbol::try_from_val(env, &v).ok());
        let t1 = topics
            .get(1)
            .and_then(|v: Val| Symbol::try_from_val(env, &v).ok());
        if t0 == Some(Symbol::new(env, "shadow")) && t1 == Some(Symbol::new(env, "simres")) {
            let decoded = ShadowModeResult::try_from_val(env, &data).unwrap();
            found_data = Some(decoded);
            break;
        }
    }
    found_data.expect("shadow/simres event must be emitted")
}

#[test]
fn successful_simulation_emits_shadow_mode_result_event() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::FeatureToggle(String::from_str(&env, "flash_loans"), true),
        &String::from_str(&env, "Enable flash loans"),
        &String::from_str(&env, "Toggle feature flag"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );

    let result = c.simulate_proposal(&pid);
    assert!(result.success, "simulation should succeed");

    // Verify the ShadowModeResult event.
    let evt = find_shadow_sim_result_event(&env);
    assert_eq!(evt.proposal_id, pid);
    assert!(evt.success, "event success must be true");
    assert!(
        evt.failure_reason.is_none(),
        "successful simulation must have None failure_reason"
    );
    assert!(
        evt.simulated_state_changes.len() >= 1,
        "simulated_state_changes must be non-empty for a feature toggle"
    );
}

#[test]
fn failed_simulation_emits_shadow_mode_result_event_with_reason() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let asset = stellar_swipe_common::Asset {
        code: String::from_str(&env, "USDC"),
        issuer: None,
    };
    // create_proposal requires the treasury to already hold >= 10x the spend
    // amount at proposal time; fund it here, then drain it below so the
    // balance is insufficient by the time simulation runs.
    c.set_treasury_asset(&admin, &asset, &10_000i128);
    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::TreasurySpend(
            Address::generate(&env),
            1_000i128,
            asset.clone(),
            String::from_str(&env, "test spend"),
        ),
        &String::from_str(&env, "Spend USDC"),
        &String::from_str(&env, "Attempt to spend from empty treasury"),
        &Bytes::new(&env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );
    c.set_treasury_asset(&admin, &asset, &0i128);

    let result = c.simulate_proposal(&pid);
    assert!(!result.success, "simulation should report failure");

    // Verify the ShadowModeResult event for a failed simulation.
    let evt = find_shadow_sim_result_event(&env);
    assert_eq!(evt.proposal_id, pid);
    assert!(!evt.success, "event success must be false");
    assert!(
        evt.failure_reason.is_some(),
        "failed simulation must have a non-null failure_reason"
    );
    assert!(
        !evt.failure_reason.as_ref().unwrap().is_empty(),
        "failure_reason must be a meaningful string"
    );
}

#[test]
fn shadow_mode_result_event_preserves_read_only_behavior() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);
    stake_tokens(&c, &r.community_rewards, 10_000);

    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::ParameterChange(String::from_str(&env, "max_fee"), 0i128, 500i128),
        &String::from_str(&env, "Set max fee"),
        &String::from_str(&env, "Change max fee to 500"),
        &Bytes::new(&env),
        &ProposalCategory::ParameterChange,
        &false,
    );

    // Snapshot proposal state before simulation.
    let before = c.proposal(&pid);
    let before_status = before.status;
    let before_for = before.votes_for;
    let before_against = before.votes_against;

    // Run simulation — this should emit the event but not persist changes.
    let result = c.simulate_proposal(&pid);
    assert!(result.success);

    // Verify the event was emitted with the correct proposal_id.
    let evt = find_shadow_sim_result_event(&env);
    assert_eq!(evt.proposal_id, pid);
    assert!(evt.success);

    // Verify persistent state is unchanged.
    let after = c.proposal(&pid);
    assert_eq!(
        after.status, before_status,
        "proposal status must not change after simulation"
    );
    assert_eq!(
        after.votes_for, before_for,
        "votes_for must not change after simulation"
    );
    assert_eq!(
        after.votes_against, before_against,
        "votes_against must not change after simulation"
    );
}
