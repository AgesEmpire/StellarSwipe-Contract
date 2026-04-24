use soroban_sdk::Env;
use soroban_sdk::Vec;

use crate::errors::OracleError;
use crate::types::ExternalPrice;

/// Aggregate external oracle reports (simplified average; signature verification is out of scope here).
pub fn process_external_prices(env: &Env, prices: Vec<ExternalPrice>) -> Result<i128, OracleError> {
    if prices.is_empty() {
        return Err(OracleError::InsufficientOracles);
    }

    let mut sum: i128 = 0;
    let mut count: i128 = 0;
    for p in prices.iter() {
        if p.price > 0 {
            sum = sum.checked_add(p.price).ok_or(OracleError::Overflow)?;
            count += 1;
        }
    }

    if count == 0 {
        return Err(OracleError::InsufficientOracles);
    }

    Ok(sum / count)
}
