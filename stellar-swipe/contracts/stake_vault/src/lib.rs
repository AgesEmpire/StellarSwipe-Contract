#![no_std]

pub mod migration;

use migration::{MigrationKey, StakeInfoV2};
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol};

/// Temporary-storage key for the reentrancy lock on `withdraw_stake`.
const EXECUTION_LOCK: &str = "WithdrawLock";

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    StakeToken,
}

#[contracttype]
#[derive(Debug, PartialEq)]
pub enum StakeVaultError {
    NotInitialized,
    Unauthorized,
    NoStake,
    StakeLocked,
    ReentrancyDetected,
}

#[contract]
pub struct StakeVaultContract;

#[contractimpl]
impl StakeVaultContract {
    /// One-time initialization. Stores admin and the SEP-41 stake token address.
    pub fn initialize(env: Env, admin: Address, stake_token: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&StorageKey::StakeToken, &stake_token);
    }

    /// Withdraw all unlocked stake for `staker`.
    ///
    /// ## External call map
    /// | # | Callee | Purpose |
    /// |---|--------|---------|
    /// | 1 | SEP-41 token SAC | `transfer(contract → staker, amount)` |
    ///
    /// The token transfer is the only cross-contract call. A malicious token
    /// contract could attempt to call back into `withdraw_stake` before the
    /// balance is zeroed. The EXECUTION_LOCK guard prevents this.
    pub fn withdraw_stake(env: Env, staker: Address) -> Result<i128, StakeVaultError> {
        staker.require_auth();

        // ── Reentrancy guard ──────────────────────────────────────────────────
        let lock_key = Symbol::new(&env, EXECUTION_LOCK);
        if env
            .storage()
            .temporary()
            .get::<_, bool>(&lock_key)
            .unwrap_or(false)
        {
            return Err(StakeVaultError::ReentrancyDetected);
        }
        env.storage().temporary().set(&lock_key, &true);

        let result = Self::do_withdraw(&env, &staker);

        env.storage().temporary().remove(&lock_key);
        result
    }

    fn do_withdraw(env: &Env, staker: &Address) -> Result<i128, StakeVaultError> {
        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        // Load V2 stake record.
        let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(env));

        let info = stakes
            .get(staker.clone())
            .ok_or(StakeVaultError::NoStake)?;

        if info.balance == 0 {
            return Err(StakeVaultError::NoStake);
        }

        let now = env.ledger().timestamp();
        if now < info.locked_until {
            return Err(StakeVaultError::StakeLocked);
        }

        let amount = info.balance;

        // Zero the balance before the token transfer (checks-effects-interactions).
        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance: 0,
                locked_until: info.locked_until,
                last_updated: now,
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);

        // Cross-contract call: transfer tokens back to staker.
        token::Client::new(env, &token).transfer(
            &env.current_contract_address(),
            staker,
            &amount,
        );

        Ok(amount)
    }

    /// Read the current stake balance for `staker` (0 if no record).
    pub fn get_stake(env: Env, staker: Address) -> i128 {
        let stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));
        stakes.get(staker).map(|s| s.balance).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests;
