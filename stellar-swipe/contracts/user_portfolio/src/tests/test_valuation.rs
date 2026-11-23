#![cfg(test)]
//! Portfolio valuation tests:
//! - #1375 stale / repeated / out-of-order source rounds are rejected
//! - #1376 checked aggregation of position values, overflow leaves state unchanged

use crate::valuation::aggregate_position_values;
use crate::{PortfolioError, UserPortfolio, UserPortfolioClient, ValuationRecord};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Env, Vec,
};

struct Ctx {
    env: Env,
    admin: Address,
    oracle: Address,
    user: Address,
    client: UserPortfolioClient<'static>,
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
    let user = Address::generate(&env);
    let id = env.register(UserPortfolio, ());
    let client = UserPortfolioClient::new(&env, &id);
    client.initialize(&admin, &oracle);
    Ctx {
        env,
        admin,
        oracle,
        user,
        client,
    }
}

impl Ctx {
    fn update(&self, round: u64, values: Vec<i128>) -> Result<ValuationRecord, PortfolioError> {
        match self
            .client
            .try_update_valuation(&self.admin, &self.user, &round, &values)
        {
            Ok(Ok(rec)) => Ok(rec),
            Err(Ok(e)) => Err(e),
            other => panic!("unexpected invocation result: {:?}", other),
        }
    }

    fn current(&self) -> Option<ValuationRecord> {
        self.client.get_valuation(&self.user)
    }
}

// ── #1375 source-round monotonicity ─────────────────────────────────────────

#[test]
fn no_valuation_before_first_update() {
    let c = setup();
    assert_eq!(c.current(), None);
}

#[test]
fn first_valuation_stores_value_and_source_round() {
    let c = setup();
    let rec = c.update(1, vec![&c.env, 100, 250]).unwrap();
    assert_eq!(
        rec,
        ValuationRecord {
            total_value: 350,
            source_round: 1,
            position_count: 2,
            updated_at: 1_000,
        }
    );
    assert_eq!(c.current(), Some(rec));
}

#[test]
fn advancing_rounds_replace_valuation() {
    let c = setup();
    c.update(1, vec![&c.env, 100]).unwrap();
    c.env.ledger().with_mut(|l| l.timestamp = 2_000);
    // Rounds need not be contiguous, only strictly increasing.
    let rec = c.update(5, vec![&c.env, 400]).unwrap();
    assert_eq!(rec.source_round, 5);
    assert_eq!(rec.total_value, 400);
    assert_eq!(rec.updated_at, 2_000);
    assert_eq!(c.current(), Some(rec));
}

#[test]
fn repeated_round_is_rejected_and_state_unchanged() {
    let c = setup();
    let before = c.update(3, vec![&c.env, 100]).unwrap();

    // Same round, different value (e.g. a retried or conflicting submission).
    assert_eq!(
        c.update(3, vec![&c.env, 999]),
        Err(PortfolioError::StaleValuationRound)
    );
    // Same round, identical value is also not re-applied.
    assert_eq!(
        c.update(3, vec![&c.env, 100]),
        Err(PortfolioError::StaleValuationRound)
    );
    assert_eq!(c.current(), Some(before));
}

#[test]
fn out_of_order_lower_round_cannot_overwrite() {
    let c = setup();
    c.update(10, vec![&c.env, 1_000]).unwrap();
    let latest = c.update(12, vec![&c.env, 1_200]).unwrap();

    // Delayed submission priced at an older round lands late.
    assert_eq!(
        c.update(11, vec![&c.env, 1_100]),
        Err(PortfolioError::StaleValuationRound)
    );
    assert_eq!(
        c.update(1, vec![&c.env, 5]),
        Err(PortfolioError::StaleValuationRound)
    );
    assert_eq!(c.current(), Some(latest));

    // A genuinely newer round is still accepted afterwards.
    let next = c.update(13, vec![&c.env, 1_300]).unwrap();
    assert_eq!(c.current(), Some(next));
}

#[test]
fn round_zero_is_never_accepted() {
    let c = setup();
    assert_eq!(
        c.update(0, vec![&c.env, 1]),
        Err(PortfolioError::StaleValuationRound)
    );
    assert_eq!(c.current(), None);
}

#[test]
fn rounds_are_tracked_per_user() {
    let c = setup();
    let other = Address::generate(&c.env);
    c.update(50, vec![&c.env, 1]).unwrap();
    // Another user's history starts independently.
    let rec = c
        .client
        .update_valuation(&c.admin, &other, &1, &vec![&c.env, 7]);
    assert_eq!(rec.source_round, 1);
    assert_eq!(c.current().unwrap().source_round, 50);
}

#[test]
fn oracle_may_submit_valuations() {
    let c = setup();
    let rec = c
        .client
        .update_valuation(&c.oracle, &c.user, &1, &vec![&c.env, 42]);
    assert_eq!(rec.total_value, 42);
}

#[test]
fn unauthorized_caller_is_rejected() {
    let c = setup();
    let stranger = Address::generate(&c.env);
    let res = c
        .client
        .try_update_valuation(&stranger, &c.user, &1, &vec![&c.env, 42]);
    assert_eq!(res, Err(Ok(PortfolioError::UnauthorizedValuationSource)));
    assert_eq!(c.current(), None);
}

// ── #1376 checked aggregation ───────────────────────────────────────────────

#[test]
fn aggregate_preserves_valid_sums() {
    let env = Env::default();
    assert_eq!(aggregate_position_values(&Vec::new(&env)), Ok(0));
    assert_eq!(aggregate_position_values(&vec![&env, 1, 2, 3]), Ok(6));
    assert_eq!(aggregate_position_values(&vec![&env, -5, 3]), Ok(-2));
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MAX]),
        Ok(i128::MAX)
    );
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MAX - 1, 1]),
        Ok(i128::MAX)
    );
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MIN]),
        Ok(i128::MIN)
    );
    // Opposite extremes cancel without an intermediate overflow.
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MAX, i128::MIN]),
        Ok(-1)
    );
}

#[test]
fn aggregate_reports_overflow_in_both_directions() {
    let env = Env::default();
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MAX, 1]),
        Err(PortfolioError::ValuationOverflow)
    );
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MAX / 2 + 1, i128::MAX / 2 + 1]),
        Err(PortfolioError::ValuationOverflow)
    );
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MIN, -1]),
        Err(PortfolioError::ValuationOverflow)
    );
    // Overflow is detected mid-sequence even if later values would bring the
    // mathematical total back into range.
    assert_eq!(
        aggregate_position_values(&vec![&env, i128::MAX, 1, -1]),
        Err(PortfolioError::ValuationOverflow)
    );
}

#[test]
fn maximum_value_valuation_is_stored_exactly() {
    let c = setup();
    let rec = c.update(1, vec![&c.env, i128::MAX - 10, 4, 6]).unwrap();
    assert_eq!(rec.total_value, i128::MAX);
    assert_eq!(rec.position_count, 3);
    assert_eq!(c.current(), Some(rec));
}

#[test]
fn overflowing_total_leaves_state_unchanged() {
    let c = setup();
    let before = c.update(1, vec![&c.env, 500]).unwrap();

    assert_eq!(
        c.update(2, vec![&c.env, i128::MAX, 1]),
        Err(PortfolioError::ValuationOverflow)
    );
    assert_eq!(
        c.update(2, vec![&c.env, i128::MIN, i128::MIN]),
        Err(PortfolioError::ValuationOverflow)
    );
    // Neither the value nor the round moved.
    assert_eq!(c.current(), Some(before));

    // The failed round was not consumed: a valid submission for it succeeds.
    let rec = c.update(2, vec![&c.env, 600]).unwrap();
    assert_eq!(rec.source_round, 2);
    assert_eq!(rec.total_value, 600);
}
