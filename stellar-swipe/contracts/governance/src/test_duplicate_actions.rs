//! Issue #1286: canonical duplicate-action detection.

use soroban_sdk::{vec, Env, String};

use crate::{action_identity, validate_no_duplicate_actions, GovernanceError, ProposalType};

fn toggle(env: &Env, name: &str, enabled: bool) -> ProposalType {
    ProposalType::FeatureToggle(String::from_str(env, name), enabled)
}

#[test]
fn exact_duplicate_action_is_rejected() {
    let env = Env::default();
    let actions = vec![
        &env,
        toggle(&env, "flash_loans", true),
        toggle(&env, "flash_loans", true),
    ];
    assert_eq!(
        validate_no_duplicate_actions(&env, &actions),
        Err(GovernanceError::DuplicateProposalAction)
    );
}

#[test]
fn distinct_parameters_are_not_duplicates() {
    let env = Env::default();
    let actions = vec![
        &env,
        toggle(&env, "flash_loans", true),
        toggle(&env, "flash_loans", false),
        toggle(&env, "flash_loans_2", true),
        ProposalType::ParameterChange(String::from_str(&env, "max_fee"), 100, 200),
        ProposalType::ParameterChange(String::from_str(&env, "max_fee"), 200, 300),
    ];
    assert!(validate_no_duplicate_actions(&env, &actions).is_ok());

    // Canonical identity includes every parameter, not just the variant name.
    assert_ne!(
        action_identity(&env, &actions.get(0).unwrap()),
        action_identity(&env, &actions.get(1).unwrap())
    );
}

#[test]
fn duplicate_detection_is_order_independent() {
    let env = Env::default();
    let a = toggle(&env, "a", true);
    let b = ProposalType::SignalProposal(String::from_str(&env, "hello"));

    let first = vec![&env, a.clone(), b.clone(), a.clone()];
    let second = vec![&env, b.clone(), a.clone(), b.clone()];
    assert_eq!(
        validate_no_duplicate_actions(&env, &first),
        Err(GovernanceError::DuplicateProposalAction)
    );
    assert_eq!(
        validate_no_duplicate_actions(&env, &second),
        Err(GovernanceError::DuplicateProposalAction)
    );
    // Same actions in a different order remain duplicate-free when unique.
    let unique_a = vec![&env, a.clone(), b.clone()];
    let unique_b = vec![&env, b, a];
    assert!(validate_no_duplicate_actions(&env, &unique_a).is_ok());
    assert!(validate_no_duplicate_actions(&env, &unique_b).is_ok());
}
