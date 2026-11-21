#![cfg(test)]
//! Issue #1125: validator sets approved by governance must wait through an
//! activation delay before they can attest messages, while the previously
//! active set remains usable in the meantime.

use crate::governance::{
    activate_validator_set, cancel_pending_validator_set, create_bridge_proposal,
    get_bridge_validators, get_pending_validator_set, initialize_bridge,
    initialize_bridge_governance, set_validator_activation_delay, sign_bridge_proposal,
    BridgeSecurityConfig, ProposalType,
};
use soroban_sdk::{
    contract,
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};

#[contract]
struct TestContract;

struct Setup {
    env: Env,
    contract_id: Address,
    signers: Vec<Address>,
    validators: Vec<Address>,
    delay: u64,
}

fn setup(delay: u64) -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(10_000);
    let contract_id = env.register(TestContract, ());

    let mut signers = Vec::new(&env);
    let mut validators = Vec::new(&env);
    for _ in 0..3 {
        signers.push_back(Address::generate(&env));
        validators.push_back(Address::generate(&env));
    }

    let security = BridgeSecurityConfig {
        max_transfer_amount: 1_000_000,
        daily_transfer_limit: 10_000_000,
        min_validator_signatures: 2,
        transfer_delay_seconds: 0,
    };

    env.as_contract(&contract_id, || {
        initialize_bridge_governance(&env, 1, signers.clone(), 2).unwrap();
        initialize_bridge(&env, 1, validators.clone(), 2, security).unwrap();
        set_validator_activation_delay(&env, 1, signers.get(0).unwrap(), delay).unwrap();
    });

    Setup {
        env,
        contract_id,
        signers,
        validators,
        delay,
    }
}

/// Approve an `AddValidator` proposal through the governance flow.
fn approve_add_validator(s: &Setup, new_validator: &Address) {
    s.env.as_contract(&s.contract_id, || {
        let proposal_id = create_bridge_proposal(
            &s.env,
            1,
            s.signers.get(0).unwrap(),
            ProposalType::AddValidator(new_validator.clone()),
            String::from_str(&s.env, "Add validator"),
        )
        .unwrap();
        sign_bridge_proposal(&s.env, 1, proposal_id, s.signers.get(1).unwrap()).unwrap();
    });
}

#[test]
fn approved_set_is_pending_until_delay_elapses() {
    let s = setup(600);
    let new_validator = Address::generate(&s.env);
    approve_add_validator(&s, &new_validator);

    let now = s.env.ledger().timestamp();
    s.env.as_contract(&s.contract_id, || {
        // The old set still attests; the approved set is pending.
        let active = get_bridge_validators(&s.env, 1).unwrap();
        assert_eq!(active.len(), 3);
        assert!(!active.contains(&new_validator));

        let pending = get_pending_validator_set(&s.env, 1).unwrap();
        assert_eq!(pending.activate_at, now + s.delay);
        assert_eq!(pending.version, 2);

        // One second before the delay elapses activation is rejected.
        s.env.ledger().set_timestamp(pending.activate_at - 1);
        assert!(activate_validator_set(&s.env, 1).is_err());

        // At exactly the boundary it succeeds.
        s.env.ledger().set_timestamp(pending.activate_at);
        assert_eq!(activate_validator_set(&s.env, 1).unwrap(), 2);

        let active = get_bridge_validators(&s.env, 1).unwrap();
        assert_eq!(active.len(), 4);
        assert!(active.contains(&new_validator));
        assert!(get_pending_validator_set(&s.env, 1).is_none());
    });
}

#[test]
fn replaying_activation_is_rejected() {
    let s = setup(600);
    let new_validator = Address::generate(&s.env);
    approve_add_validator(&s, &new_validator);

    s.env.as_contract(&s.contract_id, || {
        let pending = get_pending_validator_set(&s.env, 1).unwrap();
        s.env.ledger().set_timestamp(pending.activate_at);
        assert_eq!(activate_validator_set(&s.env, 1).unwrap(), 2);
        // No pending set remains, so a replay fails deterministically.
        assert!(activate_validator_set(&s.env, 1).is_err());
        assert_eq!(get_bridge_validators(&s.env, 1).unwrap().len(), 4);
    });
}

#[test]
fn cancelling_pending_rolls_back_to_active_set() {
    let s = setup(600);
    let new_validator = Address::generate(&s.env);
    approve_add_validator(&s, &new_validator);

    s.env.as_contract(&s.contract_id, || {
        cancel_pending_validator_set(&s.env, 1, s.signers.get(2).unwrap()).unwrap();
        assert!(get_pending_validator_set(&s.env, 1).is_none());
        let active = get_bridge_validators(&s.env, 1).unwrap();
        assert_eq!(active.len(), 3);
        assert!(!active.contains(&new_validator));
    });
}

#[test]
fn zero_delay_activates_immediately() {
    let s = setup(0);
    let new_validator = Address::generate(&s.env);
    approve_add_validator(&s, &new_validator);

    s.env.as_contract(&s.contract_id, || {
        assert!(get_pending_validator_set(&s.env, 1).is_none());
        let active = get_bridge_validators(&s.env, 1).unwrap();
        assert_eq!(active.len(), 4);
        assert!(active.contains(&new_validator));
    });
}
