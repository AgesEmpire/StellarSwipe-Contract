#![no_std]

pub mod migration;

use migration::{MigrationKey, StakeInfoV2};
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol};

/// Temporary-storage key for the reentrancy lock on `withdraw_stake`.
const EXECUTION_LOCK: &str = "WithdrawLock";

/// 24 hours in seconds — grace period for providers to top up stake.
const GRACE_PERIOD_SECS: u64 = 86_400;

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    StakeToken,
    /// Minimum stake required for a provider to submit signals.
    MinimumStake,
    /// Timestamp when a provider's stake first dropped below minimum.
    /// `None` means stake is currently at or above minimum.
    StakeBelowMinSince(Address),
}

#[contracttype]
#[derive(Debug, PartialEq)]
pub enum StakeVaultError {
    NotInitialized,
    Unauthorized,
    NoStake,
    StakeLocked,
    ReentrancyDetected,
    /// Provider stake is below minimum and grace period has expired.
    StakeBelowMinimum,
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

    /// Admin: set the minimum stake required for signal submission.
    pub fn set_minimum_stake(env: Env, minimum: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::MinimumStake, &minimum);
    }

    pub fn get_minimum_stake(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0)
    }

    /// Called when a provider's stake drops below the minimum (e.g. after slashing).
    ///
    /// - Records the timestamp of the drop (grace period start).
    /// - Emits `ProviderStakeBelowMinimum` event.
    /// - Existing signals remain valid; only new submissions are blocked.
    pub fn notify_stake_below_minimum(env: Env, provider: Address) {
        let minimum: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0);

        let current_stake = Self::get_stake(env.clone(), provider.clone());

        // Only record if actually below minimum and not already recorded.
        if current_stake >= minimum {
            return;
        }

        let key = StorageKey::StakeBelowMinSince(provider.clone());
        if !env.storage().persistent().has(&key) {
            let now = env.ledger().timestamp();
            env.storage().persistent().set(&key, &now);

            env.events().publish(
                (
                    Symbol::new(&env, "stake_vault"),
                    Symbol::new(&env, "stake_below_min"),
                ),
                (provider, current_stake, minimum),
            );
        }
    }

    /// Check whether `provider` is allowed to submit new signals.
    ///
    /// Returns `Ok(())` if:
    /// - stake is at or above minimum, OR
    /// - stake is below minimum but still within the 24h grace period.
    ///
    /// Returns `Err(StakeBelowMinimum)` if grace period has expired.
    pub fn check_signal_submission_allowed(
        env: Env,
        provider: Address,
    ) -> Result<(), StakeVaultError> {
        let minimum: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0);

        let current_stake = Self::get_stake(env.clone(), provider.clone());

        if current_stake >= minimum {
            // Stake restored — clear any recorded drop timestamp.
            let key = StorageKey::StakeBelowMinSince(provider);
            env.storage().persistent().remove(&key);
            return Ok(());
        }

        // Stake is below minimum — check grace period.
        let key = StorageKey::StakeBelowMinSince(provider.clone());
        let below_since: u64 = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.ledger().timestamp());

        let now = env.ledger().timestamp();
        if now.saturating_sub(below_since) > GRACE_PERIOD_SECS {
            Err(StakeVaultError::StakeBelowMinimum)
        } else {
            Ok(())
        }
    }

    /// Returns the timestamp when the provider's stake first dropped below minimum,
    /// or `None` if the stake is currently at or above minimum.
    pub fn get_stake_below_min_since(env: Env, provider: Address) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&StorageKey::StakeBelowMinSince(provider))
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
