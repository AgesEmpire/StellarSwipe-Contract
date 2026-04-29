#![allow(dead_code)]
use crate::stake::{can_submit_signal, StakeInfo, DEFAULT_MINIMUM_STAKE};
use crate::validation::{check_duplicate_signal, validate_rationale_hash_string, check_price_reasonableness};
use soroban_sdk::{contracttype, Address, Env, Map, String};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Buy,
    Sell,
    Hold,
}

#[contracttype]
#[derive(Clone)]
pub struct Signal {
    pub provider: Address,
    pub asset_pair: String,
    pub action: Action,
    pub price: i128,
    pub rationale: String,
    pub rationale_hash: String,
    pub timestamp: u64,
    pub expiry: u64,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    NoStake,
    BelowMinimumStake,
    InvalidAssetPair,
    InvalidPrice,
    EmptyRationale,
    MissingRationale,
    PriceUnreasonable,
    DuplicateSignal(u64),
}

#[allow(clippy::too_many_arguments, clippy::manual_range_contains)]
pub fn submit_signal(
    env: &Env,
    storage: &mut Map<u64, Signal>,
    provider_stakes: &Map<Address, StakeInfo>,
    provider: &Address,
    asset_pair: String,
    action: Action,
    price: i128,
    rationale: String,
    rationale_hash: String,
    oracle_address: Option<&Address>,
    asset_pair_id: u32,
) -> Result<u64, Error> {
    // Verify provider stake
    can_submit_signal(provider_stakes, provider).map_err(|_| Error::NoStake)?;
    let stake_info = provider_stakes.get(provider.clone()).unwrap();
    if stake_info.amount < DEFAULT_MINIMUM_STAKE {
        return Err(Error::BelowMinimumStake);
    }

    // Validate asset pair
    let asset_bytes = asset_pair.to_bytes();
    let has_slash = asset_bytes.iter().any(|b| b == b'/');
    let len = asset_bytes.len();
    if !has_slash || len < 5 || len > 20 {
        return Err(Error::InvalidAssetPair);
    }

    // Validate price
    if price <= 0 {
        return Err(Error::InvalidPrice);
    }

    // Validate rationale
    let rationale_len = rationale.to_bytes().len();
    if rationale_len == 0 || rationale_len > 500 {
        return Err(Error::EmptyRationale);
    }

    // Validate rationale hash (must be present and not all zeros)
    validate_rationale_hash_string(env, &rationale_hash)
        .map_err(|e| match e {
            crate::validation::RationaleHashError::MissingRationale => Error::MissingRationale,
            crate::validation::RationaleHashError::ZeroHash => Error::MissingRationale,
        })?;

    // Check price reasonableness against oracle
    // If oracle is unavailable, the check is skipped (returns Ok(None))
    match check_price_reasonableness(env, price, oracle_address, asset_pair_id) {
        Ok(Some(_oracle_price)) => {
            // Price is reasonable, continue
        }
        Ok(None) => {
            // Oracle unavailable, skip check
            // In a real implementation, we would emit a PriceCheckSkipped event here
        }
        Err(crate::validation::PriceReasonablenessError::PriceUnreasonable) => {
            return Err(Error::PriceUnreasonable);
        }
    }

    // Check for duplicate signals
    check_duplicate_signal(env, storage, provider, &asset_pair, &action, price)
        .map_err(|e| match e {
            crate::validation::DuplicateCheckError::DuplicateSignal(id) => Error::DuplicateSignal(id),
        })?;

    // Generate signal ID
    let now = env.ledger().timestamp();
    let next_id = storage.len() as u64 + 1;

    // Set expiry (24 hours default)
    let expiry = now + 86400;

    // Store the signal
    let signal = Signal {
        provider: provider.clone(),
        asset_pair,
        action,
        price,
        rationale,
        rationale_hash,
        timestamp: now,
        expiry,
    };
    storage.set(next_id, signal);

    Ok(next_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stake::{stake, StakeInfo, DEFAULT_MINIMUM_STAKE};
    use soroban_sdk::{testutils::Address as TestAddress, Env, Map};

    fn sdk_string(env: &Env, s: &str) -> String {
        #[allow(deprecated)]
        String::from_slice(env, s)
    }

    fn setup_env() -> Env {
        Env::default()
    }

    fn sample_provider(env: &Env) -> Address {
        <Address as TestAddress>::generate(env)
    }

    #[test]
    fn test_submit_signal_success() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        let signal_id = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish on XLM"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None, // No oracle
            1,
        )
        .unwrap();

        assert_eq!(signal_id, 1);
        let stored = signals.get(signal_id).unwrap();
        assert_eq!(stored.provider, provider);
        assert_eq!(
            stored.asset_pair.to_bytes(),
            sdk_string(&env, "XLM/USDC").to_bytes()
        );
        assert_eq!(stored.action, Action::Buy);
        assert_eq!(stored.price, 120_000_000);
        assert_eq!(
            stored.rationale.to_bytes(),
            sdk_string(&env, "Bullish on XLM").to_bytes()
        );
        assert_eq!(
            stored.rationale_hash.to_bytes(),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG").to_bytes()
        );
    }

    #[test]
    fn test_submit_signal_no_stake() {
        let env = setup_env();
        let stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish on XLM"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );

        assert_eq!(res, Err(Error::NoStake));
    }

    #[test]
    fn test_submit_signal_invalid_price() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            0,
            sdk_string(&env, "Bullish on XLM"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );

        assert_eq!(res, Err(Error::InvalidPrice));
    }

    #[test]
    fn test_submit_signal_empty_rationale() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            sdk_string(&env, ""),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );

        assert_eq!(res, Err(Error::EmptyRationale));
    }

    #[test]
    fn test_submit_signal_missing_rationale_hash() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            sdk_string(&env, "Bullish on XLM"),
            sdk_string(&env, ""),
            None,
            1,
        );

        assert_eq!(res, Err(Error::MissingRationale));
    }

    #[test]
    fn test_submit_signal_zero_rationale_hash() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        // Create a string of 32 zero bytes
        #[allow(deprecated)]
        let zero_hash = String::from_slice(
            &env,
            "\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"
        );

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            sdk_string(&env, "Bullish on XLM"),
            zero_hash,
            None,
            1,
        );

        assert_eq!(res, Err(Error::MissingRationale));
    }

    #[test]
    fn test_submit_signal_duplicate() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        let signal_id = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        )
        .unwrap();

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );

        assert_eq!(res, Err(Error::DuplicateSignal(signal_id)));
    }

    #[test]
    fn test_submit_signal_below_minimum_stake() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        let below_min = DEFAULT_MINIMUM_STAKE / 2;

        let low_stake = StakeInfo {
            amount: below_min,
            locked_until: 0,
            last_signal_time: 0,
        };
        stakes.set(provider.clone(), low_stake);

        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            100_000_000,
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );

        assert_eq!(res, Err(Error::NoStake));
    }

    #[test]
    fn test_submit_signal_invalid_asset_pair() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        // Missing slash
        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLMUSDC"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );
        assert_eq!(res, Err(Error::InvalidAssetPair));

        // Too short
        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "X/US"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );
        assert_eq!(res, Err(Error::InvalidAssetPair));

        // Too long
        let res = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC_EXTRA_LONG_PAIR"),
            Action::Buy,
            120_000_000,
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );
        assert_eq!(res, Err(Error::InvalidAssetPair));
    }

    #[test]
    fn test_submit_signal_price_check_no_oracle() {
        let env = setup_env();
        let mut stakes: Map<Address, StakeInfo> = Map::new(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut stakes, &provider, DEFAULT_MINIMUM_STAKE).unwrap();

        // No oracle provided - price check should be skipped
        let signal_id = submit_signal(
            &env,
            &mut signals,
            &stakes,
            &provider,
            sdk_string(&env, "XLM/USDC"),
            Action::Buy,
            1_000_000_000, // 10x a typical price - would fail with oracle
            sdk_string(&env, "Bullish"),
            sdk_string(&env, "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG"),
            None,
            1,
        );

        assert!(signal_id.is_ok());
    }
}
