use crate::errors::OracleError;
use crate::reputation::{get_oracle_stats, slash_oracle, SlashReason};
use crate::types::ExternalPrice;
use common::AssetPair;
use soroban_sdk::{crypto::Ed25519Signature, xdr::ToXdr, Address, Env, Vec};

pub fn process_external_prices(env: &Env, prices: Vec<ExternalPrice>) -> Result<i128, OracleError> {
    if prices.is_empty() {
        return Err(OracleError::InsufficientOracles);
    }

    let mut weighted_sum: i128 = 0;
    let mut total_weight: i128 = 0;

    for data in prices.iter() {
        // 1. Signature Verification
        // Construct the message: (AssetPair + Price + RoundID)
        let mut msg = data.asset_pair.to_xdr(env);
        msg.extend_from_array(&data.price.to_xdr(env).to_array());
        msg.extend_from_array(&data.round_id.to_xdr(env).to_array());

        let sig_verify = env.crypto().ed25519_verify(
            &data.oracle_address.to_xdr(env),
            &msg,
            &data.signature.clone().into(),
        );

        if sig_verify.is_err() {
            slash_oracle(env, &data.oracle_address, SlashReason::SignatureFailure);
            continue; // Skip invalid signatures
        }

        // 2. Freshness Check
        if env.ledger().timestamp().saturating_sub(data.timestamp) > 300 {
            continue;
        }

        // 3. Weighting
        let weight = get_oracle_stats(env, &data.oracle_address).weight as i128;
        if weight > 0 {
            weighted_sum += data
                .price
                .checked_mul(weight)
                .ok_or(OracleError::Overflow)?;
            total_weight += weight;
        }
    }

    if total_weight == 0 {
        return Err(OracleError::NoOracleData);
    }

    Ok(weighted_sum / total_weight)
}
