use soroban_sdk::Env;
use soroban_sdk::Vec;

use crate::errors::OracleError;
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
pub fn process_external_prices(env: &Env, prices: Vec<ExternalPrice>) -> Result<i128, OracleError> {
    if prices.is_empty() {
        return Err(OracleError::InsufficientOracles);
    }

    let now = env.ledger().timestamp();
    let mut sum: i128 = 0;
    let mut count: i128 = 0;
    let mut any_stale = false;
    for p in prices.iter() {
        let window = staleness::get_staleness_window(env, &p.asset_pair);
        if now.saturating_sub(p.timestamp) >= window {
            any_stale = true;
            continue;
        }
        let normalized = rescale_price(p.price, p.decimals, CANONICAL_DECIMALS)
            .ok_or(OracleError::ConversionOverflow)?;
        if normalized > 0 {
            sum = sum.checked_add(normalized).ok_or(OracleError::Overflow)?;
            count += 1;
        }
    }

    if count == 0 {
        if any_stale {
            return Err(OracleError::StalePrice);
        }
        return Err(OracleError::InsufficientOracles);
    }

    Ok(sum / count)
}
