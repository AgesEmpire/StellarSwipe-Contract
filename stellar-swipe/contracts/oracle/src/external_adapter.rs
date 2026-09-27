use soroban_sdk::Env;
use soroban_sdk::{Address, Vec};

use crate::errors::OracleError;
use crate::observation::{self, Admission};
use crate::staleness;
use crate::storage::rescale_price;
use crate::types::ExternalPrice;

const CANONICAL_DECIMALS: u32 = 7;

/// Aggregate external oracle reports, normalizing each price to canonical
/// 7-decimal precision before averaging so feeds with different native
/// precisions are consumed deterministically.
///
/// Issue #normalization: reports older than the configured staleness window for their
/// asset pair are dropped before aggregation; if every report is stale the
/// call is rejected with `OracleError::StalePrice` rather than silently
/// falling back to insufficient-sources.
///
/// Issue #1213: every report's timestamp is validated against the ledger
/// timestamp before any report is used; a single future-dated report rejects
/// the batch with `OracleError::FutureTimestamp`.
///
/// Issue #1212: reports are identified by `(oracle_address, round_id)`. Each
/// oracle contributes at most one sample — its highest round in the batch —
/// so replayed or repeated reports cannot add weight to the average.
pub fn process_external_prices(env: &Env, prices: Vec<ExternalPrice>) -> Result<i128, OracleError> {
    if prices.is_empty() {
        return Err(OracleError::InsufficientOracles);
    }

    for p in prices.iter() {
        observation::ensure_not_future(env, p.timestamp)?;
    }

    let now = env.ledger().timestamp();
    // (oracle, round_id, normalized price) — one entry per oracle.
    let mut accepted: Vec<(Address, u64, i128)> = Vec::new(env);
    let mut any_stale = false;
    for p in prices.iter() {
        let window = staleness::get_staleness_window(env, &p.asset_pair);
        if now.saturating_sub(p.timestamp) >= window {
            any_stale = true;
            continue;
        }
        let normalized = rescale_price(p.price, p.decimals, CANONICAL_DECIMALS)
            .ok_or(OracleError::ConversionOverflow)?;
        if normalized <= 0 {
            continue;
        }
        match observation::admit(
            accepted.iter().map(|(oracle, round, _)| (oracle, round)),
            &p.oracle_address,
            p.round_id,
        ) {
            Ok(Admission::Append) => {
                accepted.push_back((p.oracle_address.clone(), p.round_id, normalized))
            }
            Ok(Admission::Replace(i)) => {
                accepted.set(i, (p.oracle_address.clone(), p.round_id, normalized))
            }
            // Replayed or superseded report: never counted.
            Err(_) => continue,
        }
    }

    if accepted.is_empty() {
        if any_stale {
            return Err(OracleError::StalePrice);
        }
        return Err(OracleError::InsufficientOracles);
    }

    let mut sum: i128 = 0;
    for (_, _, price) in accepted.iter() {
        sum = sum.checked_add(price).ok_or(OracleError::Overflow)?;
    }
    Ok(sum / accepted.len() as i128)
}
