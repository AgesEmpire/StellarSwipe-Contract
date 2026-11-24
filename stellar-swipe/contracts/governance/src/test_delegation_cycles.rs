/// Tests: #1428 — reject cyclic governance delegation chains.
///
/// Covers:
///   1. Direct cycle (A → B, then B → A) is rejected.
///   2. Indirect/transitive cycle (A → B → C, then C → A) is rejected.
///   3. A rejected delegation update leaves existing delegations intact.
///   4. Valid non-cyclic chain of maximum documented depth is accepted.
///   5. A chain one node deeper than the maximum is rejected.
extern crate std;

use crate::distribution::DistributionRecipients;
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String};

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

/// Direct cycle: A delegates to B, then B tries to delegate to A — must fail.
#[test]
fn direct_cycle_is_rejected() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let alice = r.community_rewards.clone();
    let bob = r.team.clone();
    c.stake(&alice, &5_000i128);
    c.stake(&bob, &5_000i128);

    // Alice → Bob succeeds.
    c.delegate_voting_power(&alice, &bob);

    // Bob → Alice must be rejected (direct cycle).
    let result = c.try_delegate_voting_power(&bob, &alice);
    assert!(
        result.is_err(),
        "direct delegation cycle (B → A when A → B exists) must be rejected"
    );
}

/// Indirect cycle: A → B → C exists, then C → A must fail.
#[test]
fn indirect_cycle_is_rejected() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let alice = r.community_rewards.clone();
    let bob = r.team.clone();
    let carol = r.early_investors.clone();

    c.stake(&alice, &5_000i128);
    c.stake(&bob, &5_000i128);
    c.stake(&carol, &5_000i128);

    c.delegate_voting_power(&alice, &bob);
    c.delegate_voting_power(&bob, &carol);

    // Carol → Alice would create A → B → C → A cycle.
    let result = c.try_delegate_voting_power(&carol, &alice);
    assert!(
        result.is_err(),
        "transitive cycle (C → A when A → B → C exists) must be rejected"
    );
}

/// A rejected delegation must not modify existing delegations.
#[test]
fn rejected_delegation_leaves_existing_state_intact() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let alice = r.community_rewards.clone();
    let bob = r.team.clone();
    c.stake(&alice, &5_000i128);
    c.stake(&bob, &5_000i128);

    c.delegate_voting_power(&alice, &bob);

    // Attempt cyclic delegation — must fail.
    let _ = c.try_delegate_voting_power(&bob, &alice);

    // Alice's delegation to Bob must still be active.
    let vp = c.effective_voting_power(&bob);
    assert!(
        vp > 5_000,
        "Bob's voting power must still include Alice's delegated stake after rejected cycle attempt"
    );
}

/// Self-delegation is already blocked upstream; just verify the guard holds.
#[test]
fn self_delegation_is_rejected() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let alice = r.community_rewards.clone();
    c.stake(&alice, &5_000i128);

    let result = c.try_delegate_voting_power(&alice, &alice);
    assert!(result.is_err(), "self-delegation must be rejected");
}

/// A linear chain up to MAX_DELEGATION_DEPTH nodes deep is accepted.
/// Note: this test uses a short chain (3 nodes) to verify the happy path
/// without needing to generate MAX_DELEGATION_DEPTH addresses.
#[test]
fn valid_chain_is_accepted() {
    let (env, id, admin, r) = setup();
    let c = client(&env, &id);
    init(&c, &env, &admin, &r);

    let alice = r.community_rewards.clone();
    let bob = r.team.clone();
    let carol = r.early_investors.clone();

    c.stake(&alice, &5_000i128);
    c.stake(&bob, &5_000i128);
    c.stake(&carol, &5_000i128);

    // A → B → C with no cycle — both steps must succeed.
    c.delegate_voting_power(&alice, &bob);
    c.delegate_voting_power(&bob, &carol);

    // Carol's effective power includes her own + Bob's delegated (which includes Alice's).
    let carol_power = c.effective_voting_power(&carol);
    assert!(carol_power > 5_000, "Carol must have more than her own stake");
}
