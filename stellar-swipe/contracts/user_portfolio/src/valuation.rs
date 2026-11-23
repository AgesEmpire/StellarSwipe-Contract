//! Portfolio valuation records.
//!
//! - #1375: every stored valuation carries the `source_round` (oracle round id)
//!   it was derived from, and a write is accepted only when its round strictly
//!   advances past the stored one. Lower or repeated rounds — e.g. a delayed
//!   keeper transaction landing after a newer one, or a retried submission —
//!   are rejected with [`PortfolioError::StaleValuationRound`] and cannot
//!   overwrite the current valuation. Rounds start at 1; round 0 is never
//!   accepted (matching the oracle's own `accept_round` rule).
//! - #1376: position values are summed with `checked_add`. A sum that leaves the
//!   `i128` range fails with [`PortfolioError::ValuationOverflow`] instead of
//!   wrapping or saturating to a misleading total.
//!
//! Every check runs before the single storage write, so a rejected update
//! leaves the stored valuation (value *and* round) untouched.
//!
//! The record is intentionally not removed by `close_portfolio`: keeping the
//! last accepted round preserves monotonicity, so a stale round cannot be
//! replayed after the portfolio is closed and reopened.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

use crate::storage::DataKey;
use crate::PortfolioError;

/// Latest accepted valuation of a user's portfolio.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValuationRecord {
    /// Checked sum of the submitted position values.
    pub total_value: i128,
    /// Oracle round the position values were priced at. Strictly increasing.
    pub source_round: u64,
    /// Number of position values aggregated into `total_value`.
    pub position_count: u32,
    /// Ledger timestamp at which this valuation was accepted.
    pub updated_at: u64,
}

/// Sum `values` with checked arithmetic (#1376).
///
/// Returns `ValuationOverflow` if any partial sum leaves the `i128` range, in
/// either direction. An empty input sums to `0`.
pub fn aggregate_position_values(values: &Vec<i128>) -> Result<i128, PortfolioError> {
    let mut total: i128 = 0;
    for v in values.iter() {
        total = total
            .checked_add(v)
            .ok_or(PortfolioError::ValuationOverflow)?;
    }
    Ok(total)
}

pub fn get_valuation(env: &Env, user: &Address) -> Option<ValuationRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::PortfolioValuation(user.clone()))
}

/// Last accepted source round for `user` (`0` if none).
pub fn last_round(env: &Env, user: &Address) -> u64 {
    get_valuation(env, user)
        .map(|v| v.source_round)
        .unwrap_or(0)
}

/// Validate and store a new valuation for `user`.
///
/// Caller authorization is the entrypoint's responsibility.
pub fn update_valuation(
    env: &Env,
    user: &Address,
    source_round: u64,
    position_values: &Vec<i128>,
) -> Result<ValuationRecord, PortfolioError> {
    // #1375: only strictly advancing rounds may replace the current valuation.
    if source_round <= last_round(env, user) {
        return Err(PortfolioError::StaleValuationRound);
    }
    // #1376: checked aggregation; overflow aborts before anything is written.
    let total_value = aggregate_position_values(position_values)?;

    let record = ValuationRecord {
        total_value,
        source_round,
        position_count: position_values.len(),
        updated_at: env.ledger().timestamp(),
    };
    env.storage()
        .persistent()
        .set(&DataKey::PortfolioValuation(user.clone()), &record);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "user_portfolio"),
            Symbol::new(env, "valuation_updated"),
        ),
        (user.clone(), source_round, total_value),
    );

    Ok(record)
}
