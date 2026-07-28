//! Token metadata and decimal validation (Issue #880).
//!
//! Deposits, withdrawals and reward math must agree on what a "unit" of an
//! asset is. Stellar classic assets are 7-decimal, but Soroban token contracts
//! may declare anything up to [`MAX_DECIMALS`]. Entrypoints validate metadata
//! once, up front, with [`validate`] and then use [`rescale`] to convert an
//! amount between the token's own precision and the protocol's internal
//! [`PROTOCOL_DECIMALS`] precision.

use soroban_sdk::{contracttype, String};

/// Decimals used for all internal accounting (Stellar classic precision).
pub const PROTOCOL_DECIMALS: u32 = 7;

/// Largest decimal exponent the protocol will accept from a token contract.
/// Above this, rescaling to protocol precision cannot be done without
/// overflowing `i128` for realistic balances.
pub const MAX_DECIMALS: u32 = 18;

/// Stellar asset code length bounds.
const SYMBOL_MIN_LEN: u32 = 1;
const SYMBOL_MAX_LEN: u32 = 12;
const NAME_MAX_LEN: u32 = 64;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenMetadataError {
    /// `decimals` exceeds [`MAX_DECIMALS`].
    UnsupportedDecimals,
    /// Symbol is empty or longer than 12 characters.
    InvalidSymbol,
    /// Name is empty or longer than 64 characters.
    InvalidName,
    /// Metadata does not match the value previously registered for this asset.
    MetadataMismatch,
}

/// Metadata as reported by a token contract.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenMetadata {
    pub symbol: String,
    pub name: String,
    pub decimals: u32,
}

/// Validates token metadata before it is used for accounting.
pub fn validate(metadata: &TokenMetadata) -> Result<(), TokenMetadataError> {
    if metadata.decimals > MAX_DECIMALS {
        return Err(TokenMetadataError::UnsupportedDecimals);
    }
    let symbol_len = metadata.symbol.len();
    if symbol_len < SYMBOL_MIN_LEN || symbol_len > SYMBOL_MAX_LEN {
        return Err(TokenMetadataError::InvalidSymbol);
    }
    let name_len = metadata.name.len();
    if name_len == 0 || name_len > NAME_MAX_LEN {
        return Err(TokenMetadataError::InvalidName);
    }
    Ok(())
}

/// Validates `observed` and checks it still matches what was registered for the
/// asset. Re-reading metadata on every entrypoint is what catches a token that
/// changed its decimals between a deposit and the matching withdrawal.
pub fn validate_matches(
    observed: &TokenMetadata,
    registered: &TokenMetadata,
) -> Result<(), TokenMetadataError> {
    validate(observed)?;
    if observed != registered {
        return Err(TokenMetadataError::MetadataMismatch);
    }
    Ok(())
}

/// Converts `amount` from `from_decimals` precision to `to_decimals` precision.
///
/// Scaling down truncates toward zero — the protocol never rounds in the user's
/// favour. Returns `None` on overflow so callers surface an error rather than
/// silently wrapping.
pub fn rescale(amount: i128, from_decimals: u32, to_decimals: u32) -> Option<i128> {
    if from_decimals > MAX_DECIMALS || to_decimals > MAX_DECIMALS {
        return None;
    }
    if from_decimals == to_decimals {
        return Some(amount);
    }
    if to_decimals > from_decimals {
        let factor = 10i128.checked_pow(to_decimals - from_decimals)?;
        amount.checked_mul(factor)
    } else {
        let factor = 10i128.checked_pow(from_decimals - to_decimals)?;
        Some(amount / factor)
    }
}

/// Converts an amount in the token's own precision to protocol precision.
pub fn to_protocol_amount(amount: i128, token_decimals: u32) -> Option<i128> {
    rescale(amount, token_decimals, PROTOCOL_DECIMALS)
}

/// Converts an internal protocol amount back to the token's own precision.
pub fn from_protocol_amount(amount: i128, token_decimals: u32) -> Option<i128> {
    rescale(amount, PROTOCOL_DECIMALS, token_decimals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn metadata(env: &Env, symbol: &str, name: &str, decimals: u32) -> TokenMetadata {
        TokenMetadata {
            symbol: String::from_str(env, symbol),
            name: String::from_str(env, name),
            decimals,
        }
    }

    #[test]
    fn accepts_well_formed_metadata() {
        let env = Env::default();
        assert!(validate(&metadata(&env, "USDC", "USD Coin", 7)).is_ok());
        assert!(validate(&metadata(&env, "XLM", "Stellar Lumens", 7)).is_ok());
        assert!(validate(&metadata(&env, "WETH", "Wrapped Ether", 18)).is_ok());
    }

    #[test]
    fn rejects_unsupported_decimals() {
        let env = Env::default();
        assert_eq!(
            validate(&metadata(&env, "BAD", "Too Precise", 19)),
            Err(TokenMetadataError::UnsupportedDecimals)
        );
    }

    #[test]
    fn rejects_bad_symbol_and_name() {
        let env = Env::default();
        assert_eq!(
            validate(&metadata(&env, "", "Empty Symbol", 7)),
            Err(TokenMetadataError::InvalidSymbol)
        );
        assert_eq!(
            validate(&metadata(&env, "WAYTOOLONGCODE", "Long Symbol", 7)),
            Err(TokenMetadataError::InvalidSymbol)
        );
        assert_eq!(
            validate(&metadata(&env, "OK", "", 7)),
            Err(TokenMetadataError::InvalidName)
        );
    }

    #[test]
    fn detects_metadata_drift() {
        let env = Env::default();
        let registered = metadata(&env, "USDC", "USD Coin", 7);
        let same = metadata(&env, "USDC", "USD Coin", 7);
        let drifted = metadata(&env, "USDC", "USD Coin", 6);

        assert!(validate_matches(&same, &registered).is_ok());
        assert_eq!(
            validate_matches(&drifted, &registered),
            Err(TokenMetadataError::MetadataMismatch)
        );
    }

    #[test]
    fn rescale_is_identity_at_equal_precision() {
        assert_eq!(rescale(1_234, 7, 7), Some(1_234));
    }

    #[test]
    fn rescale_up_and_down() {
        // 1.0 token at 6 decimals -> 7 decimals
        assert_eq!(to_protocol_amount(1_000_000, 6), Some(10_000_000));
        // 1.0 token at 18 decimals -> 7 decimals
        assert_eq!(
            to_protocol_amount(1_000_000_000_000_000_000, 18),
            Some(10_000_000)
        );
        // Round-trip back out
        assert_eq!(from_protocol_amount(10_000_000, 6), Some(1_000_000));
    }

    #[test]
    fn rescale_truncates_toward_zero() {
        // Sub-unit dust is dropped, never rounded up.
        assert_eq!(to_protocol_amount(1_999, 18), Some(0));
        assert_eq!(rescale(-15, 8, 7), Some(-1));
    }

    #[test]
    fn rescale_rejects_overflow_and_bad_decimals() {
        assert_eq!(rescale(i128::MAX, 7, 18), None);
        assert_eq!(rescale(1, 0, 19), None);
        assert_eq!(rescale(1, 19, 7), None);
    }
}
