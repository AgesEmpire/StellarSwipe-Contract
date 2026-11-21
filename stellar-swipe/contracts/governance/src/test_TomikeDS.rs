extern crate std;

use crate::distribution::DistributionRecipients;
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String};

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

fn init(
    client: &GovernanceContractClient<'_>,
    env: &Env,
    admin: &Address,
    r: &DistributionRecipients,
) {
    client.initialize(
        admin,
        &String::from_str(env, "StellarSwipe Gov"),
        &String::from_str(env, "SSG"),
        &7u32,
        &SUPPLY,
        r,
    );
}

/// Give `user` a staked balance so they have voting power.
fn stake(env: &Env, contract_id: &Address, user: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        crate::add_staked_balance(env, user, amount).unwrap();
        crate::track_holder(env, user);
    });
}

// ── Issue #586: Vote delegation ───────────────────────────────────────────────

#[test]
fn delegate_voting_power_transfers_weight_to_delegate() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let delegator = Address::generate(&env);
    let delegate = Address::generate(&env);

    stake(&env, &id, &delegator, 5_000i128);
    stake(&env, &id, &delegate, 1_000i128);

    // Before delegation both have their own power.
    assert_eq!(client.effective_voting_power(&delegator), 5_000);
    assert_eq!(client.effective_voting_power(&delegate), 1_000);

    client.delegate_voting_power(&delegator, &delegate);

    // After delegation: delegator loses its own power (it's delegated away),
    // and delegate gains it.
    assert_eq!(client.effective_voting_power(&delegator), 0);
    assert_eq!(client.effective_voting_power(&delegate), 6_000); // own + delegated
}

#[test]
fn revoke_delegation_restores_delegator_weight() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let delegator = Address::generate(&env);
    let delegate = Address::generate(&env);

    stake(&env, &id, &delegator, 4_000i128);
    stake(&env, &id, &delegate, 500i128);

    client.delegate_voting_power(&delegator, &delegate);
    assert_eq!(client.effective_voting_power(&delegator), 0);

    client.undelegate_voting_power(&delegator);

    // Power is restored to delegator after revocation.
    assert_eq!(client.effective_voting_power(&delegator), 4_000);
    assert_eq!(client.effective_voting_power(&delegate), 500);
}

#[test]
fn delegator_cannot_delegate_to_self() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let user = Address::generate(&env);
    stake(&env, &id, &user, 1_000i128);

    let result = client.try_delegate_voting_power(&user, &user);
    assert_eq!(
        result,
        Err(Ok(GovernanceError::InvalidProposal)),
        "self-delegation should be rejected"
    );
}

#[test]
fn delegator_cannot_re_delegate_while_active() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let delegator = Address::generate(&env);
    let delegate_a = Address::generate(&env);
    let delegate_b = Address::generate(&env);

    stake(&env, &id, &delegator, 2_000i128);

    client.delegate_voting_power(&delegator, &delegate_a);

    // Attempting to delegate again while active must fail.
    let result = client.try_delegate_voting_power(&delegator, &delegate_b);
    assert_eq!(
        result,
        Err(Ok(GovernanceError::InvalidCommitteeAction)),
        "re-delegation while active should be rejected"
    );
}

#[test]
fn undelegate_without_prior_delegation_returns_error() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let user = Address::generate(&env);
    let result = client.try_undelegate_voting_power(&user);
    assert!(
        result.is_err(),
        "undelegating without prior delegation should fail"
    );
}

#[test]
fn effective_voting_power_aggregates_multiple_delegators() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let delegate = Address::generate(&env);
    let d1 = Address::generate(&env);
    let d2 = Address::generate(&env);

    stake(&env, &id, &delegate, 100i128);
    stake(&env, &id, &d1, 3_000i128);
    stake(&env, &id, &d2, 2_000i128);

    client.delegate_voting_power(&d1, &delegate);
    client.delegate_voting_power(&d2, &delegate);

    // delegate has own (100) + d1 (3000) + d2 (2000) = 5100
    assert_eq!(client.effective_voting_power(&delegate), 5_100);
}

// ── Issue #584: Double-init guard (governance contract) ───────────────────────

#[test]
fn governance_double_init_returns_error() {
    let (env, id, admin, r) = setup();
    let client = GovernanceContractClient::new(&env, &id);
    init(&client, &env, &admin, &r);

    let result = client.try_initialize(
        &admin,
        &String::from_str(&env, "StellarSwipe Gov"),
        &String::from_str(&env, "SSG"),
        &7u32,
        &SUPPLY,
        &r,
    );
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyInitialized)));
}

// ── Issue #585: Event topics (spot-check canonical constants compile) ─────────

#[test]
fn canonical_event_topic_constants_are_accessible() {
    // Verify that the shared::event_topics constants can be referenced and
    // produce the expected Symbols (regression guard for renaming accidents).
    use soroban_sdk::symbol_short;
    assert_eq!(
        shared::event_topics::TOPIC_GOVERNANCE(),
        symbol_short!("gov")
    );
    assert_eq!(
        shared::event_topics::TOPIC_SHADOW_DISCREP(),
        symbol_short!("discrep")
    );
    assert_eq!(
        shared::event_topics::TOPIC_GOV_PROP_NEW(),
        symbol_short!("propnew")
    );
}
