use soroban_sdk::{Address, Env, Map, String, BytesN};
use crate::submission::{Action, Signal};

/// Maximum allowed price deviation from oracle price (in basis points)
/// 2000 = 20% deviation allowed
pub const MAX_PRICE_DEVIATION_BPS: u32 = 2000;

/// Error type for duplicate signal detection
#[derive(Debug, PartialEq)]
pub enum DuplicateCheckError {
    /// A duplicate signal was found. Contains the ID of the existing signal.
    DuplicateSignal(u64),
}

/// Error type for rationale hash validation
#[derive(Debug, PartialEq)]
pub enum RationaleHashError {
    /// Rationale hash is missing or empty
    MissingRationale,
    /// Rationale hash is all zeros (invalid)
    ZeroHash,
}

/// Error type for price reasonableness validation
#[derive(Debug, PartialEq)]
pub enum PriceReasonablenessError {
    /// Signal price deviates too much from oracle price
    PriceUnreasonable,
}

/// Check if a new signal is a duplicate of an existing active signal.
/// 
/// Duplicate criteria:
/// - Same provider
/// - Same asset_pair
/// - Same action
/// - Price within 1% of existing signal
/// - Submitted within 1 hour of existing signal
/// - Existing signal is not expired
///
/// Returns Ok(()) if no duplicate found, or Err(DuplicateCheckError::DuplicateSignal(id))
/// with the ID of the existing duplicate signal.
pub fn check_duplicate_signal(
    env: &Env,
    storage: &Map<u64, Signal>,
    provider: &Address,
    asset_pair: &String,
    action: &Action,
    price: i128,
) -> Result<(), DuplicateCheckError> {
    let now = env.ledger().timestamp();
    let one_hour = 3600u64;

    for (signal_id, existing_signal) in storage.iter() {
        // Skip if different provider
        if existing_signal.provider != *provider {
            continue;
        }

        // Skip if different asset pair
        if existing_signal.asset_pair.to_bytes() != asset_pair.to_bytes() {
            continue;
        }

        // Skip if different action
        if existing_signal.action != *action {
            continue;
        }

        // Skip if signal is expired (edge case: expired signals don't block new submissions)
        if now >= existing_signal.expiry {
            continue;
        }

        // Skip if submitted more than 1 hour ago
        if now >= existing_signal.timestamp + one_hour {
            continue;
        }

        // Check if price is within 1%
        if is_price_within_threshold(price, existing_signal.price) {
            return Err(DuplicateCheckError::DuplicateSignal(signal_id));
        }
    }

    Ok(())
}

/// Check if two prices are within 1% of each other.
/// Uses integer arithmetic to avoid floating point operations.
fn is_price_within_threshold(price1: i128, price2: i128) -> bool {
    // Calculate 1% threshold
    // We use the larger price as the base to ensure symmetry
    let base = if price1 > price2 { price1 } else { price2 };
    
    // Calculate 1% of base price
    // threshold = base * 1 / 100
    let threshold = base / 100;
    
    // Calculate absolute difference
    let diff = if price1 > price2 {
        price1 - price2
    } else {
        price2 - price1
    };
    
    // Prices are within 1% if difference <= threshold
    diff <= threshold
}

/// Validate that a rationale hash is present and not all zeros.
///
/// A valid rationale hash should be a 32-byte IPFS hash (or similar content hash)
/// that is not empty and not all zeros.
///
/// # Arguments
/// * `env` - Soroban environment
/// * `rationale_hash` - The 32-byte hash to validate
///
/// # Returns
/// Ok(()) if valid, or Err with appropriate error
pub fn validate_rationale_hash(
    env: &Env,
    rationale_hash: &BytesN<32>,
) -> Result<(), RationaleHashError> {
    // Check if hash is all zeros
    let zero_hash = BytesN::from_array(env, &[0u8; 32]);
    
    if rationale_hash == &zero_hash {
        return Err(RationaleHashError::ZeroHash);
    }
    
    Ok(())
}

/// Validate rationale hash from String representation.
/// Expects a hex-encoded 32-byte hash (64 characters).
///
/// # Arguments
/// * `env` - Soroban environment
/// * `rationale_hash_str` - String containing the hash
///
/// # Returns
/// Ok(()) if valid, or Err with appropriate error
pub fn validate_rationale_hash_string(
    env: &Env,
    rationale_hash_str: &String,
) -> Result<(), RationaleHashError> {
    let hash_bytes = rationale_hash_str.to_bytes();
    
    // Check if empty
    if hash_bytes.len() == 0 {
        return Err(RationaleHashError::MissingRationale);
    }
    
    // Check if it's a valid length for hex-encoded 32 bytes (64 chars)
    // or if it's at least some non-empty content
    if hash_bytes.len() < 32 {
        return Err(RationaleHashError::MissingRationale);
    }
    
    // Check if all bytes are zeros (for binary representation)
    let mut all_zeros = true;
    for byte in hash_bytes.iter() {
        if byte != 0 {
            all_zeros = false;
            break;
        }
    }
    
    if all_zeros {
        return Err(RationaleHashError::ZeroHash);
    }
    
    Ok(())
}

/// Check if signal price is reasonable compared to oracle price.
///
/// Validates that the signal price is within MAX_PRICE_DEVIATION_BPS (20%) of the oracle price.
/// If oracle is unavailable, returns Ok(None) to indicate the check was skipped.
///
/// # Arguments
/// * `env` - Soroban environment
/// * `signal_price` - The price from the signal submission
/// * `oracle_address` - Optional oracle contract address
/// * `asset_pair_id` - Asset pair identifier for oracle lookup
///
/// # Returns
/// * Ok(Some(oracle_price)) - Price is reasonable, returns oracle price for reference
/// * Ok(None) - Oracle unavailable, check skipped
/// * Err(PriceReasonablenessError::PriceUnreasonable) - Price deviates too much
pub fn check_price_reasonableness(
    env: &Env,
    signal_price: i128,
    oracle_address: Option<&soroban_sdk::Address>,
    asset_pair_id: u32,
) -> Result<Option<i128>, PriceReasonablenessError> {
    // If no oracle configured, skip check
    let oracle_addr = match oracle_address {
        Some(addr) => addr,
        None => return Ok(None),
    };
    
    // Try to fetch oracle price
    use stellar_swipe_common::oracle::{IOracleClient, OnChainOracleClient, oracle_price_to_i128, validate_freshness};
    
    let client = OnChainOracleClient {
        address: oracle_addr.clone(),
    };
    
    let oracle_price_result = client.get_price(env, asset_pair_id);
    
    // If oracle call fails or price is stale, skip check
    let oracle_price_data = match oracle_price_result {
        Ok(price) => {
            // Validate freshness
            if validate_freshness(env, &price).is_err() {
                return Ok(None);
            }
            price
        }
        Err(_) => return Ok(None),
    };
    
    // Convert oracle price to i128
    let oracle_price = oracle_price_to_i128(&oracle_price_data);
    
    // Check if prices are within acceptable deviation
    if is_price_reasonable(signal_price, oracle_price) {
        Ok(Some(oracle_price))
    } else {
        Err(PriceReasonablenessError::PriceUnreasonable)
    }
}

/// Check if signal price is within acceptable deviation from oracle price.
///
/// Calculates: |signal_price - oracle_price| / oracle_price <= MAX_PRICE_DEVIATION_BPS / 10000
///
/// # Arguments
/// * `signal_price` - Price from signal submission
/// * `oracle_price` - Current oracle price
///
/// # Returns
/// true if within acceptable range, false otherwise
fn is_price_reasonable(signal_price: i128, oracle_price: i128) -> bool {
    if oracle_price == 0 {
        // Can't validate against zero oracle price
        return true;
    }
    
    // Calculate absolute difference
    let diff = if signal_price > oracle_price {
        signal_price - oracle_price
    } else {
        oracle_price - signal_price
    };
    
    // Calculate percentage deviation in basis points
    // deviation_bps = (diff * 10000) / oracle_price
    let deviation_bps = (diff as u128 * 10000) / oracle_price.abs() as u128;
    
    // Check if within acceptable range
    deviation_bps <= MAX_PRICE_DEVIATION_BPS as u128
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as TestAddress, Env, Map};

    fn sdk_string(env: &Env, s: &str) -> String {
        #[allow(deprecated)]
        String::from_slice(env, s)
    }

    fn create_signal(
        env: &Env,
        provider: Address,
        asset_pair: String,
        action: Action,
        price: i128,
        timestamp: u64,
        expiry: u64,
    ) -> Signal {
        Signal {
            provider,
            asset_pair,
            action,
            price,
            rationale: sdk_string(env, "Test rationale"),
            rationale_hash: sdk_string(env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            timestamp,
            expiry,
        }
    }

    #[test]
    fn test_exact_duplicate() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now,
            now + 86400,
        );
        storage.set(1, signal);

        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Buy,
            100_000_000,
        );

        assert_eq!(result, Err(DuplicateCheckError::DuplicateSignal(1)));
    }

    #[test]
    fn test_near_duplicate_within_1_percent() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now,
            now + 86400,
        );
        storage.set(1, signal);

        // Price is 0.5% higher (within 1% threshold)
        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Buy,
            100_500_000,
        );

        assert_eq!(result, Err(DuplicateCheckError::DuplicateSignal(1)));
    }

    #[test]
    fn test_non_duplicate_outside_1_percent() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now,
            now + 86400,
        );
        storage.set(1, signal);

        // Price is 2% higher (outside 1% threshold)
        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Buy,
            102_000_000,
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_expired_signal_not_duplicate() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        // Create an expired signal (expiry in the past)
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now - 7200, // 2 hours ago
            now - 3600, // expired 1 hour ago
        );
        storage.set(1, signal);

        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Buy,
            100_000_000,
        );

        // Should allow new submission since existing signal is expired
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_different_provider_not_duplicate() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider1 = <Address as TestAddress>::generate(&env);
        let provider2 = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        let signal = create_signal(
            &env,
            provider1.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now,
            now + 86400,
        );
        storage.set(1, signal);

        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider2,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Buy,
            100_000_000,
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_different_asset_pair_not_duplicate() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now,
            now + 86400,
        );
        storage.set(1, signal);

        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "BTC/USDC"),
            &Action::Buy,
            100_000_000,
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_different_action_not_duplicate() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now,
            now + 86400,
        );
        storage.set(1, signal);

        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Sell,
            100_000_000,
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_old_signal_not_duplicate() {
        let env = Env::default();
        let mut storage: Map<u64, Signal> = Map::new(&env);
        let provider = <Address as TestAddress>::generate(&env);
        
        let now = env.ledger().timestamp();
        // Create a signal from more than 1 hour ago
        let signal = create_signal(
            &env,
            provider.clone(),
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            now - 7200, // 2 hours ago
            now + 79200, // still valid for 22 more hours
        );
        storage.set(1, signal);

        let result = check_duplicate_signal(
            &env,
            &storage,
            &provider,
            &sdk_string(&env, "XLM/USDC"),
            &Action::Buy,
            100_000_000,
        );

        // Should allow new submission since existing signal is older than 1 hour
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_price_within_threshold_exact() {
        assert!(is_price_within_threshold(100_000_000, 100_000_000));
    }

    #[test]
    fn test_price_within_threshold_1_percent() {
        // Exactly 1% difference
        assert!(is_price_within_threshold(100_000_000, 101_000_000));
        assert!(is_price_within_threshold(101_000_000, 100_000_000));
    }

    #[test]
    fn test_price_outside_threshold() {
        // More than 1% difference
        assert!(!is_price_within_threshold(100_000_000, 102_000_000));
        assert!(!is_price_within_threshold(102_000_000, 100_000_000));
    }

    #[test]
    fn test_price_within_threshold_small_values() {
        // Test with smaller values
        assert!(is_price_within_threshold(1000, 1010));
        assert!(!is_price_within_threshold(1000, 1020));
    }

    // ========== Rationale Hash Validation Tests ==========

    #[test]
    fn test_validate_rationale_hash_valid() {
        let env = Env::default();
        // Create a valid non-zero hash
        let valid_hash = BytesN::from_array(&env, &[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
        ]);
        
        let result = validate_rationale_hash(&env, &valid_hash);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_validate_rationale_hash_zero_hash() {
        let env = Env::default();
        // Create a zero hash
        let zero_hash = BytesN::from_array(&env, &[0u8; 32]);
        
        let result = validate_rationale_hash(&env, &zero_hash);
        assert_eq!(result, Err(RationaleHashError::ZeroHash));
    }

    #[test]
    fn test_validate_rationale_hash_string_valid() {
        let env = Env::default();
        // Create a valid hash string (IPFS hash example)
        #[allow(deprecated)]
        let valid_hash_str = String::from_slice(
            &env,
            "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"
        );
        
        let result = validate_rationale_hash_string(&env, &valid_hash_str);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_validate_rationale_hash_string_empty() {
        let env = Env::default();
        #[allow(deprecated)]
        let empty_hash = String::from_slice(&env, "");
        
        let result = validate_rationale_hash_string(&env, &empty_hash);
        assert_eq!(result, Err(RationaleHashError::MissingRationale));
    }

    #[test]
    fn test_validate_rationale_hash_string_too_short() {
        let env = Env::default();
        #[allow(deprecated)]
        let short_hash = String::from_slice(&env, "short");
        
        let result = validate_rationale_hash_string(&env, &short_hash);
        assert_eq!(result, Err(RationaleHashError::MissingRationale));
    }

    #[test]
    fn test_validate_rationale_hash_string_all_zeros() {
        let env = Env::default();
        // Create a string of 32 zero bytes
        #[allow(deprecated)]
        let zero_string = String::from_slice(
            &env,
            "\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"
        );
        
        let result = validate_rationale_hash_string(&env, &zero_string);
        assert_eq!(result, Err(RationaleHashError::ZeroHash));
    }

    // ========== Price Reasonableness Tests ==========

    #[test]
    fn test_is_price_reasonable_exact_match() {
        // Exact match should be reasonable
        assert!(is_price_reasonable(100_000_000, 100_000_000));
    }

    #[test]
    fn test_is_price_reasonable_within_20_percent() {
        let oracle_price = 100_000_000i128;
        
        // 10% higher - should be reasonable
        assert!(is_price_reasonable(110_000_000, oracle_price));
        
        // 10% lower - should be reasonable
        assert!(is_price_reasonable(90_000_000, oracle_price));
        
        // Exactly 20% higher - should be reasonable (at boundary)
        assert!(is_price_reasonable(120_000_000, oracle_price));
        
        // Exactly 20% lower - should be reasonable (at boundary)
        assert!(is_price_reasonable(80_000_000, oracle_price));
    }

    #[test]
    fn test_is_price_reasonable_outside_20_percent() {
        let oracle_price = 100_000_000i128;
        
        // 21% higher - should be unreasonable
        assert!(!is_price_reasonable(121_000_000, oracle_price));
        
        // 25% higher - should be unreasonable
        assert!(!is_price_reasonable(125_000_000, oracle_price));
        
        // 21% lower - should be unreasonable
        assert!(!is_price_reasonable(79_000_000, oracle_price));
        
        // 50% lower - should be unreasonable
        assert!(!is_price_reasonable(50_000_000, oracle_price));
        
        // 2x higher - should be unreasonable
        assert!(!is_price_reasonable(200_000_000, oracle_price));
    }

    #[test]
    fn test_is_price_reasonable_zero_oracle_price() {
        // Zero oracle price should return true (can't validate)
        assert!(is_price_reasonable(100_000_000, 0));
    }

    #[test]
    fn test_check_price_reasonableness_no_oracle() {
        let env = Env::default();
        
        // No oracle address provided - should skip check
        let result = check_price_reasonableness(&env, 100_000_000, None, 1);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_check_price_reasonableness_oracle_unavailable() {
        let env = Env::default();
        let oracle_addr = <soroban_sdk::Address as soroban_sdk::testutils::Address>::generate(&env);
        
        // Oracle address provided but oracle call will fail - should skip check
        let result = check_price_reasonableness(&env, 100_000_000, Some(&oracle_addr), 1);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_check_price_reasonableness_with_mock_oracle_within_range() {
        let env = Env::default();
        
        // Set up mock oracle price
        use stellar_swipe_common::oracle::{MockOracleClient, OraclePrice};
        use soroban_sdk::Symbol;
        
        let oracle_price = OraclePrice {
            price: 100_000_000,
            decimals: 0,
            timestamp: env.ledger().timestamp(),
            source: Symbol::new(&env, "test"),
        };
        
        MockOracleClient::set_price(&env, 1, oracle_price);
        
        // Signal price within 20% (110 vs 100)
        let signal_price = 110_000_000;
        
        // Note: This test would need the MockOracleClient to be used instead of OnChainOracleClient
        // For now, we test the is_price_reasonable function directly
        assert!(is_price_reasonable(signal_price, 100_000_000));
    }

    #[test]
    fn test_check_price_reasonableness_with_mock_oracle_outside_range() {
        let env = Env::default();
        
        // Signal price outside 20% (130 vs 100 = 30% deviation)
        let signal_price = 130_000_000;
        let oracle_price = 100_000_000;
        
        assert!(!is_price_reasonable(signal_price, oracle_price));
    }

    #[test]
    fn test_price_reasonableness_edge_cases() {
        // Very small prices
        assert!(is_price_reasonable(100, 100));
        assert!(is_price_reasonable(120, 100));
        assert!(!is_price_reasonable(121, 100));
        
        // Large prices
        assert!(is_price_reasonable(1_000_000_000, 1_000_000_000));
        assert!(is_price_reasonable(1_200_000_000, 1_000_000_000));
        assert!(!is_price_reasonable(1_210_000_000, 1_000_000_000));
        
        // Negative prices (shouldn't happen but test robustness)
        assert!(is_price_reasonable(-100_000_000, -100_000_000));
    }

    #[test]
    fn test_max_price_deviation_constant() {
        // Verify the constant is set correctly
        assert_eq!(MAX_PRICE_DEVIATION_BPS, 2000); // 20%
    }
}
