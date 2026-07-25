#![cfg(test)]

use soroban_sdk::{testutils::{Address as _, Ledger as _}, Env};
use crate::governance::{
    create_proposal, execute_proposal, get_proposal,
    GovernanceError, ProposalStatus,
    PROPOSAL_COOLDOWN_SECONDS, PROPOSAL_TIMELOCK_SECONDS, MAX_EXECUTIONS_PER_PROPOSAL,
};

fn setup() -> (Env, soroban_sdk::Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let caller = soroban_sdk::Address::generate(&env);
    (env, caller)
}

// ── Creation ──────────────────────────────────────────────────────────────────

#[test]
fn test_create_proposal_stores_correctly() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        let p = get_proposal(&env, id).unwrap();
        assert_eq!(p.id, id);
        assert_eq!(p.proposer, caller);
        assert_eq!(p.status, ProposalStatus::Pending);
        assert_eq!(p.execution_count, 0);
        assert_eq!(p.last_executed_at, 0);
    });
}

// ── Timelock ──────────────────────────────────────────────────────────────────

#[test]
fn test_execute_before_timelock_fails() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        // Still within timelock window
        let err = execute_proposal(&env, id, &caller).unwrap_err();
        assert_eq!(err, GovernanceError::TimelockNotExpired);
    });
}

#[test]
fn test_execute_after_timelock_succeeds() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        env.ledger().set_timestamp(1_000 + PROPOSAL_TIMELOCK_SECONDS);
        assert!(execute_proposal(&env, id, &caller).is_ok());
    });
}

// ── Rate limiting ─────────────────────────────────────────────────────────────

#[test]
fn test_repeated_execution_within_cooldown_is_rejected() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        env.ledger().set_timestamp(1_000 + PROPOSAL_TIMELOCK_SECONDS);

        // First execution — ok
        assert!(execute_proposal(&env, id, &caller).is_ok());

        // Immediate second attempt — rate limited
        let err = execute_proposal(&env, id, &caller).unwrap_err();
        assert_eq!(err, GovernanceError::ExecutionRateLimited);
    });
}

#[test]
fn test_execution_after_cooldown_succeeds() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        env.ledger().set_timestamp(1_000 + PROPOSAL_TIMELOCK_SECONDS);
        assert!(execute_proposal(&env, id, &caller).is_ok());

        // Advance past cooldown
        env.ledger()
            .set_timestamp(1_000 + PROPOSAL_TIMELOCK_SECONDS + PROPOSAL_COOLDOWN_SECONDS);
        assert!(execute_proposal(&env, id, &caller).is_ok());
    });
}

// ── Execution cap ─────────────────────────────────────────────────────────────

#[test]
fn test_execution_cap_is_enforced() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        let mut t = 1_000 + PROPOSAL_TIMELOCK_SECONDS;

        for _ in 0..MAX_EXECUTIONS_PER_PROPOSAL {
            env.ledger().set_timestamp(t);
            assert!(execute_proposal(&env, id, &caller).is_ok());
            t += PROPOSAL_COOLDOWN_SECONDS;
        }

        // One more — should be blocked
        env.ledger().set_timestamp(t);
        let err = execute_proposal(&env, id, &caller).unwrap_err();
        assert!(
            err == GovernanceError::ExecutionLimitReached
                || err == GovernanceError::ProposalAlreadyExecuted
        );
    });
}

// ── Already executed ──────────────────────────────────────────────────────────

#[test]
fn test_already_executed_proposal_is_rejected() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        let mut t = 1_000 + PROPOSAL_TIMELOCK_SECONDS;

        // Exhaust all executions to reach Executed status
        for _ in 0..MAX_EXECUTIONS_PER_PROPOSAL {
            env.ledger().set_timestamp(t);
            let _ = execute_proposal(&env, id, &caller);
            t += PROPOSAL_COOLDOWN_SECONDS;
        }

        env.ledger().set_timestamp(t);
        let err = execute_proposal(&env, id, &caller).unwrap_err();
        assert_eq!(err, GovernanceError::ProposalAlreadyExecuted);
    });
}

// ── Not found ─────────────────────────────────────────────────────────────────

#[test]
fn test_execute_nonexistent_proposal_fails() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let err = execute_proposal(&env, 999, &caller).unwrap_err();
        assert_eq!(err, GovernanceError::ProposalNotFound);
    });
}

// ── Status transitions ────────────────────────────────────────────────────────

#[test]
fn test_proposal_status_transitions_correctly() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        assert_eq!(get_proposal(&env, id).unwrap().status, ProposalStatus::Pending);

        env.ledger().set_timestamp(1_000 + PROPOSAL_TIMELOCK_SECONDS);
        execute_proposal(&env, id, &caller).unwrap();
        // After first execution (not yet at cap) status is Active
        assert_eq!(get_proposal(&env, id).unwrap().status, ProposalStatus::Active);
    });
}

#[test]
fn test_execution_count_increments() {
    let (env, caller) = setup();
    env.as_contract(&env.register(crate::AutoTradeContract, ()), || {
        let id = create_proposal(&env, caller.clone());
        let mut t = 1_000 + PROPOSAL_TIMELOCK_SECONDS;

        for expected in 1..MAX_EXECUTIONS_PER_PROPOSAL {
            env.ledger().set_timestamp(t);
            execute_proposal(&env, id, &caller).unwrap();
            assert_eq!(get_proposal(&env, id).unwrap().execution_count, expected);
            t += PROPOSAL_COOLDOWN_SECONDS;
        }
    });
}
