#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, Map};

pub const DEFAULT_MINIMUM_STAKE: i128 = 100_000_000; // 100 XLM
pub const UNSTAKE_LOCK_PERIOD: u64 = 7 * 24 * 60 * 60; // 7 days in seconds

#[contracttype]
#[derive(Clone)]
pub struct StakeInfo {
    pub amount: i128,
    pub last_signal_time: u64,
    pub locked_until: u64,
}

#[derive(Debug, PartialEq)]
pub enum ContractError {
    InvalidStakeAmount,
    NoStakeFound,
    StakeLocked,
    InsufficientStake,
    BelowMinimumStake,
}

/// Stake XLM for a provider
pub fn stake(
    _env: &Env,
    storage: &mut Map<Address, StakeInfo>,
    provider: &Address,
    amount: i128,
) -> Result<(), ContractError> {
    if amount <= 0 {
        return Err(ContractError::InvalidStakeAmount);
    }

    let mut info = storage.get(provider.clone()).unwrap_or(StakeInfo {
        amount: 0,
        last_signal_time: 0,
        locked_until: 0,
    });

    info.amount += amount;

    if info.amount < DEFAULT_MINIMUM_STAKE {
        return Err(ContractError::BelowMinimumStake);
    }

    storage.set(provider.clone(), info);
    Ok(())
}

/// Unstake XLM (only after lock period)
pub fn unstake(
    env: &Env,
    storage: &mut Map<Address, StakeInfo>,
    provider: &Address,
) -> Result<i128, ContractError> {
    let mut info = storage
        .get(provider.clone())
        .ok_or(ContractError::NoStakeFound)?;
    let now = env.ledger().timestamp();

    if now < info.locked_until {
        return Err(ContractError::StakeLocked);
    }

    if info.amount <= 0 {
        return Err(ContractError::InsufficientStake);
    }

    let amount = info.amount;
    info.amount = 0;
    storage.set(provider.clone(), info);

    Ok(amount)
}

/// Record that a signal was submitted
/// Updates last_signal_time and locks stake for UNSTAKE_LOCK_PERIOD
pub fn record_signal(
    env: &Env,
    storage: &mut Map<Address, StakeInfo>,
    provider: &Address,
) -> Result<(), ContractError> {
    let mut info = storage
        .get(provider.clone())
        .ok_or(ContractError::NoStakeFound)?;
    let now = env.ledger().timestamp();

    info.last_signal_time = now;
    info.locked_until = now + UNSTAKE_LOCK_PERIOD;

    storage.set(provider.clone(), info);
    Ok(())
}

/// Check if a provider can submit a signal
pub fn can_submit_signal(
    storage: &Map<Address, StakeInfo>,
    provider: &Address,
) -> Result<(), ContractError> {
    let info = storage
        .get(provider.clone())
        .ok_or(ContractError::NoStakeFound)?;

    if info.amount < DEFAULT_MINIMUM_STAKE {
        return Err(ContractError::BelowMinimumStake);
    }

    Ok(())
}

/// Get stake information for a provider from instance storage.
pub fn get_stake_info(env: &Env, provider: &Address) -> Option<StakeInfo> {
    let map: Map<Address, StakeInfo> = env
        .storage()
        .instance()
        .get(&crate::StorageKey::ProviderStakes)
        .unwrap_or(Map::new(env));
    map.get(provider.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{testutils::Address as TestAddress, Address, Env, Map};

    fn setup_env() -> Env {
        Env::default()
    }

    fn sample_provider(env: &Env) -> Address {
        <Address as TestAddress>::generate(env)
    }

    #[test]
    fn test_stake_accumulation_and_minimum() {
        let env = setup_env();
        let mut storage: Map<Address, StakeInfo> = Map::new(&env);
        let provider = sample_provider(&env);

        // First stake below minimum fails
        assert_eq!(
            stake(&env, &mut storage, &provider, 50_000_000),
            Err(ContractError::BelowMinimumStake)
        );

        // Stake meeting minimum succeeds
        assert!(stake(&env, &mut storage, &provider, 100_000_000).is_ok());
        let info = storage.get(provider.clone()).unwrap();
        assert_eq!(info.amount, 100_000_000);

        // Additional stake accumulates
        assert!(stake(&env, &mut storage, &provider, 50_000_000).is_ok());
        let info = storage.get(provider.clone()).unwrap();
        assert_eq!(info.amount, 150_000_000);
    }

    #[test]
    fn test_unstake_before_and_after_lock() {
        let env = setup_env();
        let mut storage: Map<Address, StakeInfo> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut storage, &provider, 100_000_000).unwrap();

        // Locked because no signal yet (locked_until = 0, so actually allowed)
        let unstake_result = unstake(&env, &mut storage, &provider);
        assert_eq!(unstake_result.unwrap(), 100_000_000);

        // Re-stake and simulate signal submission
        stake(&env, &mut storage, &provider, 100_000_000).unwrap();
        record_signal(&env, &mut storage, &provider).unwrap();

        // Attempt unstake immediately should fail
        assert_eq!(
            unstake(&env, &mut storage, &provider),
            Err(ContractError::StakeLocked)
        );

        // Move timestamp beyond lock period
        env.ledger()
            .set_timestamp(env.ledger().timestamp() + UNSTAKE_LOCK_PERIOD + 1);

        // Now unstake should succeed
        let amount = unstake(&env, &mut storage, &provider).unwrap();
        assert_eq!(amount, 100_000_000);
    }

    #[test]
    fn test_record_signal_updates_lock() {
        let env = setup_env();
        let mut storage: Map<Address, StakeInfo> = Map::new(&env);
        let provider = sample_provider(&env);

        stake(&env, &mut storage, &provider, 100_000_000).unwrap();
        let before = env.ledger().timestamp();

        record_signal(&env, &mut storage, &provider).unwrap();
        let info = storage.get(provider.clone()).unwrap();

        assert_eq!(info.last_signal_time, before);
        assert_eq!(info.locked_until, before + UNSTAKE_LOCK_PERIOD);
    }

    #[test]
    fn test_can_submit_signal() {
        let env = setup_env();
        let mut storage: Map<Address, StakeInfo> = Map::new(&env);
        let provider = sample_provider(&env);

        // No stake yet
        assert_eq!(
            can_submit_signal(&storage, &provider),
            Err(ContractError::NoStakeFound)
        );

        stake(&env, &mut storage, &provider, 100_000_000).unwrap();
        assert!(can_submit_signal(&storage, &provider).is_ok());
    }
}
