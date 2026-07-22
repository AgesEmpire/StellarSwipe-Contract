#![cfg(test)]
//! Unit tests for DCA copy trading (Issue #360).
//!
//! Covers:
//! - Full DCA completion (all intervals execute, DCAPlanCompleted emitted)
//! - Signal expiry cancellation (DCAPlanCancelled with reason=0)
//! - Manual cancellation (DCAPlanCancelled with reason=1)
//! - Interval timing enforcement (IntervalNotDue)
//! - Duplicate plan rejection (DCAPlanAlreadyExists)

use crate::{
    dca::{self, DCAPlan},
    errors::ContractError,
    StorageKey,
};
use shared::events::{EvtDCAPlanCancelled, EvtDCAPlanCompleted};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Events, Ledger},
    Address, Env,
};

// ── Minimal contract needed to run as_contract ────────────────────────────────

#[contract]
struct TestContract;

#[contractimpl]
impl TestContract {}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(TestContract, ());
    (env, id)
}

fn set_ledger(env: &Env, seq: u32) {
    env.ledger().with_mut(|l| l.sequence_number = seq);
}

/// A no-op execute_fn that always succeeds.
fn ok_exec(_amount: i128) -> Result<(), ContractError> {
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn full_dca_completion_executes_all_intervals_and_emits_completed() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 100);

        // Create a 3-interval plan, 10 ledgers apart, no expiry.
        dca::execute_dca_copy_trade(&env, &user, 1, 300, 3, 10, 0).unwrap();

        // Interval 1 — immediately due.
        let done = dca::execute_dca_interval(&env, &user, 1, ok_exec).unwrap();
        assert!(!done);

        // Interval 2 — advance ledger.
        set_ledger(&env, 110);
        let done = dca::execute_dca_interval(&env, &user, 1, ok_exec).unwrap();
        assert!(!done);

        // Interval 3 — plan completes.
        set_ledger(&env, 120);
        let done = dca::execute_dca_interval(&env, &user, 1, ok_exec).unwrap();
        assert!(done);

        // Plan should be gone.
        assert_eq!(
            dca::load_plan(&env, &user, 1).unwrap_err(),
            ContractError::DCAPlanNotFound
        );

        // Check events: 3 × DCAIntervalExecuted + 1 × DCAPlanCompleted.
        let all = env.events().all();
        // Last event is DCAPlanCompleted.
        let (_, _, completed_val) = all.last().unwrap();
        let completed: EvtDCAPlanCompleted =
            soroban_sdk::TryFromVal::try_from_val(&env, &completed_val).unwrap();
        assert_eq!(completed.user, user);
        assert_eq!(completed.signal_id, 1);
        assert_eq!(completed.total_amount, 300);
    });
}

#[test]
fn each_interval_executes_correct_amount() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 0);
        dca::execute_dca_copy_trade(&env, &user, 42, 1000, 4, 5, 0).unwrap();

        let mut amounts = soroban_sdk::Vec::new(&env);
        for i in 0u32..4 {
            set_ledger(&env, i * 5);
            dca::execute_dca_interval(&env, &user, 42, |amt| {
                amounts.push_back(amt);
                Ok(())
            })
            .unwrap();
        }

        // Each interval should be 250 (1000 / 4).
        for i in 0..4 {
            assert_eq!(amounts.get(i).unwrap(), 250);
        }
    });
}

#[test]
fn signal_expiry_cancels_plan_and_emits_cancelled_reason_0() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 100);
        // Signal expires at ledger 110.
        dca::execute_dca_copy_trade(&env, &user, 7, 200, 2, 5, 110).unwrap();

        // First interval at ledger 100 — OK.
        dca::execute_dca_interval(&env, &user, 7, ok_exec).unwrap();

        // Advance past expiry.
        set_ledger(&env, 115);
        let err = dca::execute_dca_interval(&env, &user, 7, ok_exec).unwrap_err();
        assert_eq!(err, ContractError::SignalExpired);

        // Plan removed.
        assert_eq!(
            dca::load_plan(&env, &user, 7).unwrap_err(),
            ContractError::DCAPlanNotFound
        );

        // Last event is DCAPlanCancelled with reason=0.
        let all = env.events().all();
        let (_, _, val) = all.last().unwrap();
        let cancelled: EvtDCAPlanCancelled =
            soroban_sdk::TryFromVal::try_from_val(&env, &val).unwrap();
        assert_eq!(cancelled.user, user);
        assert_eq!(cancelled.signal_id, 7);
        assert_eq!(cancelled.intervals_completed, 1);
        assert_eq!(cancelled.reason, 0);
    });
}

#[test]
fn manual_cancellation_emits_cancelled_reason_1() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 50);
        dca::execute_dca_copy_trade(&env, &user, 99, 500, 5, 10, 0).unwrap();

        // Execute one interval.
        dca::execute_dca_interval(&env, &user, 99, ok_exec).unwrap();

        // Manual cancel. 4 of 5 intervals remain at 100 each == 400 refund.
        let mut refunded = None;
        dca::cancel_dca_plan(&env, &user, 99, |amount| {
            refunded = Some(amount);
            Ok(())
        })
        .unwrap();
        assert_eq!(refunded, Some(400));

        // Plan removed.
        assert_eq!(
            dca::load_plan(&env, &user, 99).unwrap_err(),
            ContractError::DCAPlanNotFound
        );

        // Last event is DCAPlanCancelled with reason=1.
        let all = env.events().all();
        let (_, _, val) = all.last().unwrap();
        let cancelled: EvtDCAPlanCancelled =
            soroban_sdk::TryFromVal::try_from_val(&env, &val).unwrap();
        assert_eq!(cancelled.reason, 1);
        assert_eq!(cancelled.intervals_completed, 1);
        assert_eq!(cancelled.refund_amount, 400);
    });
}

// ── Issue #790: proportional refund on cancellation ────────────────────────────

#[test]
fn cancel_with_zero_executions_refunds_full_amount() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 0);
        // 10-interval plan, 100 per interval, none executed yet.
        dca::execute_dca_copy_trade(&env, &user, 10, 1000, 10, 5, 0).unwrap();

        let mut refunded = None;
        dca::cancel_dca_plan(&env, &user, 10, |amount| {
            refunded = Some(amount);
            Ok(())
        })
        .unwrap();
        assert_eq!(refunded, Some(1000));

        let all = env.events().all();
        let (_, _, val) = all.last().unwrap();
        let cancelled: EvtDCAPlanCancelled =
            soroban_sdk::TryFromVal::try_from_val(&env, &val).unwrap();
        assert_eq!(cancelled.intervals_completed, 0);
        assert_eq!(cancelled.refund_amount, 1000);
    });
}

#[test]
fn cancel_after_partial_execution_refunds_remaining_intervals_only() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 0);
        // 10-interval plan, 100 per interval.
        dca::execute_dca_copy_trade(&env, &user, 11, 1000, 10, 5, 0).unwrap();

        // Execute 3 of the 10 intervals.
        for i in 0u32..3 {
            set_ledger(&env, i * 5);
            dca::execute_dca_interval(&env, &user, 11, ok_exec).unwrap();
        }

        // 7 intervals remain unexecuted: 7 * 100 = 700.
        let mut refunded = None;
        dca::cancel_dca_plan(&env, &user, 11, |amount| {
            refunded = Some(amount);
            Ok(())
        })
        .unwrap();
        assert_eq!(refunded, Some(700));

        let all = env.events().all();
        let (_, _, val) = all.last().unwrap();
        let cancelled: EvtDCAPlanCancelled =
            soroban_sdk::TryFromVal::try_from_val(&env, &val).unwrap();
        assert_eq!(cancelled.intervals_completed, 3);
        assert_eq!(cancelled.refund_amount, 700);
    });
}

#[test]
fn cancel_on_final_interval_after_full_execution_refunds_nothing() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 0);
        // 3-interval plan, 100 per interval — execute all of them.
        dca::execute_dca_copy_trade(&env, &user, 12, 300, 3, 5, 0).unwrap();
        for i in 0u32..3 {
            set_ledger(&env, i * 5);
            let done = dca::execute_dca_interval(&env, &user, 12, ok_exec).unwrap();
            if i == 2 {
                assert!(done);
            }
        }

        // The plan is already removed by the final interval's own completion
        // (see full_dca_completion_executes_all_intervals_and_emits_completed),
        // so cancelling afterwards must report the plan as not found rather
        // than double-refund.
        let refund_fn_called = core::cell::Cell::new(false);
        let result = dca::cancel_dca_plan(&env, &user, 12, |_amount| {
            refund_fn_called.set(true);
            Ok(())
        });
        assert_eq!(result, Err(ContractError::DCAPlanNotFound));
        assert!(!refund_fn_called.get());
    });
}

#[test]
fn interval_not_due_returns_error() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 100);
        dca::execute_dca_copy_trade(&env, &user, 5, 200, 2, 20, 0).unwrap();

        // First interval is due immediately.
        dca::execute_dca_interval(&env, &user, 5, ok_exec).unwrap();

        // Try again before 20 ledgers have passed.
        set_ledger(&env, 110);
        let err = dca::execute_dca_interval(&env, &user, 5, ok_exec).unwrap_err();
        assert_eq!(err, ContractError::IntervalNotDue);
    });
}

#[test]
fn duplicate_plan_rejected() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 0);
        dca::execute_dca_copy_trade(&env, &user, 3, 100, 2, 5, 0).unwrap();
        let err = dca::execute_dca_copy_trade(&env, &user, 3, 100, 2, 5, 0).unwrap_err();
        assert_eq!(err, ContractError::DCAPlanAlreadyExists);
    });
}

#[test]
fn create_plan_with_already_expired_signal_fails() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_ledger(&env, 200);
        // Expiry in the past.
        let err = dca::execute_dca_copy_trade(&env, &user, 8, 100, 2, 5, 100).unwrap_err();
        assert_eq!(err, ContractError::SignalExpired);
    });
}

#[test]
fn cancel_nonexistent_plan_returns_not_found() {
    let (env, contract_id) = setup();
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let err = dca::cancel_dca_plan(&env, &user, 999, ok_exec).unwrap_err();
        assert_eq!(err, ContractError::DCAPlanNotFound);
    });
}
