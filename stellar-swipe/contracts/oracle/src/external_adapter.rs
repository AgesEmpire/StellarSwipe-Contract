use soroban_sdk::Env;
use soroban_sdk::{Address, Vec};

use crate::errors::OracleError;
use crate::heartbeat;
use crate::observation::{self, Admission};
use crate::staleness;
use crate::storage::{self, normalize_price_checked, CANONICAL_DECIMALS};
use crate::types::ExternalPrice;

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
///
/// Issue #1405: decimals are validated at ingestion. A report whose
/// `decimals` exceeds `MAX_FEED_DECIMALS`, or differs from the scale
/// configured for its feed via `set_feed_decimals`, rejects the batch with
/// `OracleError::UnsupportedPrecision` before any report is aggregated.
/// Normalization to the canonical precision uses checked arithmetic and
/// rejects the batch with `OracleError::ConversionOverflow` on overflow.
///
/// Issue #1406: reports from a source that exceeded its configured heartbeat
/// are dropped; if no report survives and at least one was dropped for its
/// heartbeat the batch is rejected with `OracleError::HeartbeatExpired`.
pub fn process_external_prices(env: &Env, prices: Vec<ExternalPrice>) -> Result<i128, OracleError> {
    if prices.is_empty() {
        return Err(OracleError::InsufficientOracles);
    }

    for p in prices.iter() {
        observation::ensure_not_future(env, p.timestamp)?;
        storage::validate_decimals(p.decimals)?;
        if let Some(configured) = storage::get_feed_decimals(env, &p.asset_pair) {
            if configured != p.decimals {
                return Err(OracleError::UnsupportedPrecision);
            }
        }
    }

    let now = env.ledger().timestamp();
    // (oracle, round_id, normalized price) — one entry per oracle.
    let mut accepted: Vec<(Address, u64, i128)> = Vec::new(env);
    let mut any_stale = false;
    let mut any_heartbeat_expired = false;
    for p in prices.iter() {
        let window = staleness::get_staleness_window(env, &p.asset_pair);
        if now.saturating_sub(p.timestamp) >= window {
            any_stale = true;
            continue;
        }
        if !heartbeat::is_within_heartbeat(env, &p.asset_pair, &p.oracle_address, p.timestamp) {
            any_heartbeat_expired = true;
            continue;
        }
        let normalized = normalize_price_checked(p.price, p.decimals, CANONICAL_DECIMALS)?;
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
        if any_heartbeat_expired {
            return Err(OracleError::HeartbeatExpired);
        }
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
