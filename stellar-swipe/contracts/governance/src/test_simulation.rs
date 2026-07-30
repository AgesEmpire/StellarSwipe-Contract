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
    ProposalCategory, ProposalStatus, ProposalType, SimulationEffect, SimulationResult,
};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Bytes, Env, String, Vec};

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

fn client(env: &Env, id: &Address) -> GovernanceContractClient {
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
    assert_eq!(result.error, String::from_str(&env, ""), "no error expected");
    assert!(
        result.effects.len() >= 1,
        "expected at least 1 effect, got {}",
        result.effects.len()
    );
    let effect = result.effects.get(0).unwrap();
    assert_eq!(effect.key, String::from_str(&env, "signal"));

    let stored = c.get_proposal(&pid);
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

    let found = result.effects.iter().any(|eff| {
        eff.key == String::from_str(&env, "parameter:max_fee")
    });
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
    let pid = c.create_proposal(
        &r.community_rewards,
        &ProposalType::TreasurySpend(
            Address::generate(&env),
            1_000i128,
            asset,
            String::from_str(&env, "test spend"),
        ),
        &String::from_str(&env, "Spend USDC"),
        &String::from_str(&env, "Attempt to spend from empty treasury"),
        &Bytes::new(&env),
        &ProposalCategory::TreasuryTransfer,
        &false,
    );

    let result = c.simulate_proposal(&pid);
    assert!(!result.success, "simulation should report failure");
    assert!(result.effects.len() == 0, "no effects expected when simulation fails early");
}

// -- #917: non-existent proposal returns ProposalNotFound

#[test]
fn simulate_nonexistent_proposal_returns_error() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let result: Result<SimulationResult, GovernanceError> =
        c.try_simulate_proposal(&999_999u64);
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

    let found = result.effects.iter().any(|eff| {
        eff.key == String::from_str(&env, "feature:flash_loans")
    });
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

    let stored = c.get_proposal(&pid);
    assert_eq!(
        stored.status,
        ProposalStatus::Pending,
        "simulation must not change proposal status even after multiple calls"
    );
}
