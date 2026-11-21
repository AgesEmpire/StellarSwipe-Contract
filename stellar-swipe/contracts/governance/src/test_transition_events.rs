//! Issue #1288: complete, replayable proposal transition events.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{Address, Bytes, Env, String, TryFromVal, Val};

use crate::distribution::DistributionRecipients;
use crate::proposals::{ProposalCategory, ProposalStatus, ProposalType};
use crate::{GovernanceContract, GovernanceContractClient, GovernanceError, GovernanceVoteType, ProposalTransitionEvent};

const SUPPLY: i128 = 1_000_000_000;

fn transitions(env: &Env) -> std::vec::Vec<ProposalTransitionEvent> {
    let mut out = std::vec::Vec::new();
    for (_, _topics, data) in env.events().all() {
        // The event's `pstate` topic is paired with the structured payload;
        // decoding the payload is an exact filter for transition events.
        if let Ok(event) = ProposalTransitionEvent::try_from_val(env, &data) {
            out.push(event);
        }
    }
    out
}

#[test]
fn lifecycle_events_reconstruct_proposal_state() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let id = env.register(GovernanceContract, ());
    let c = GovernanceContractClient::new(&env, &id);
    let admin = Address::generate(&env);
    let proposer = Address::generate(&env);
    c.initialize(
        &admin,
        &String::from_str(&env, "Gov"),
        &String::from_str(&env, "GOV"),
        &7u32,
        &SUPPLY,
        &DistributionRecipients {
            team: Address::generate(&env),
            early_investors: Address::generate(&env),
            community_rewards: proposer.clone(),
            treasury: Address::generate(&env),
            public_sale: Address::generate(&env),
        },
    );
    c.stake(&proposer, &120_000_000);
    env.ledger().set_timestamp(100);
    let proposal_id = c.create_proposal(
        &proposer,
        &ProposalType::FeatureToggle(String::from_str(&env, "flag"), true),
        &String::from_str(&env, "title"),
        &String::from_str(&env, "description"),
        &Bytes::new(&env),
        &ProposalCategory::General,
        &false,
    );
    // Test host keeps events per-ledger, so collect after each ledger step.
    let mut events = transitions(&env);
    env.ledger().set_timestamp(200);
    c.cast_vote(&proposal_id, &proposer, &GovernanceVoteType::For);
    events.extend(transitions(&env));
    env.ledger().set_timestamp(700_000);
    c.finalize_proposal(&proposal_id);
    events.extend(transitions(&env));

    let kinds: std::vec::Vec<String> = events.iter().map(|e| e.kind.clone()).collect();
    assert_eq!(
        kinds,
        std::vec![
            String::from_str(&env, "created"),
            String::from_str(&env, "voting_started"),
            String::from_str(&env, "voting_completed"),
            String::from_str(&env, "executed"),
        ]
    );

    // Replay: creation starts from no state; each later event must continue
    // from the state established by the previous event.
    let mut state = ProposalStatus::Uninitialized;
    for event in &events {
        assert_eq!(event.proposal_id, proposal_id);
        assert_eq!(event.previous_status, state);
        assert!(event.actor.to_string().len() > 0);
        assert!(event.ledger_time > 0);
        state = event.new_status.clone();
    }
    assert_eq!(state, ProposalStatus::Executed);

    // Invalid transitions return Err, so the host rolls back all writes,
    // including events; the replay sequence above remains unchanged.

}

// Keep the `Val` import used by the TryFromVal-based decoder explicit.
#[allow(dead_code)]
fn _val_type(_: Val) {}
