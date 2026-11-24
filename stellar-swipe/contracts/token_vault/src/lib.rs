#![no_std]
//! Token vault: deposit, swap and withdraw SEP-41 tokens.
//!
//! - Issue #1524: an admin-managed allowlist of supported token contracts,
//!   kept in persistent storage. `deposit` and `swap` reject tokens that are
//!   not allowlisted; `withdraw` stays open so users can always exit.
//! - Issue #1525: a storage-based reentrancy lock guards `deposit`, `swap`
//!   and `withdraw`; a reentrant call fails with
//!   [`VaultError::ReentrancyGuard`].
//!
//! Swaps are priced with a constant-product curve against admin-funded
//! reserves, and settle against users' internal vault balances.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token, Address, Env};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Locked,
    AllowedToken(Address),
    Balance(Address, Address),
    Reserve(Address),
}

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VaultError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    TokenNotAllowed = 4,
    InsufficientBalance = 5,
    InsufficientLiquidity = 6,
    SlippageExceeded = 7,
    IdenticalTokens = 8,
    ReentrancyGuard = 9,
    Overflow = 10,
}

#[contract]
pub struct TokenVault;

fn admin(env: &Env) -> Result<Address, VaultError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(VaultError::NotInitialized)
}

fn require_allowed(env: &Env, token: &Address) -> Result<(), VaultError> {
    if TokenVault::is_token_allowed(env.clone(), token.clone()) {
        Ok(())
    } else {
        Err(VaultError::TokenNotAllowed)
    }
}

/// Runs `f` while holding the reentrancy lock: the flag is set before `f`
/// and cleared after it, whatever `f` returns.
fn guarded<T>(env: &Env, f: impl FnOnce() -> Result<T, VaultError>) -> Result<T, VaultError> {
    let storage = env.storage().instance();
    if storage.get(&DataKey::Locked).unwrap_or(false) {
        return Err(VaultError::ReentrancyGuard);
    }
    storage.set(&DataKey::Locked, &true);
    let result = f();
    storage.remove(&DataKey::Locked);
    result
}

fn get_i128(env: &Env, key: &DataKey) -> i128 {
    env.storage().persistent().get(key).unwrap_or(0)
}

fn set_i128(env: &Env, key: &DataKey, value: i128) {
    env.storage().persistent().set(key, &value);
}

#[contractimpl]
impl TokenVault {
    pub fn initialize(env: Env, admin: Address) -> Result<(), VaultError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(VaultError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Admin: add `token` to the allowlist.
    pub fn add_allowed_token(env: Env, token: Address) -> Result<(), VaultError> {
        admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::AllowedToken(token), &true);
        Ok(())
    }

    /// Admin: remove `token` from the allowlist.
    pub fn remove_allowed_token(env: Env, token: Address) -> Result<(), VaultError> {
        admin(&env)?.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::AllowedToken(token));
        Ok(())
    }

    pub fn is_token_allowed(env: Env, token: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::AllowedToken(token))
            .unwrap_or(false)
    }

    /// Admin: seed swap liquidity for an allowlisted `token`.
    pub fn fund_reserve(env: Env, token: Address, amount: i128) -> Result<(), VaultError> {
        let admin = admin(&env)?;
        admin.require_auth();
        if amount <= 0 {
            return Err(VaultError::InvalidAmount);
        }
        require_allowed(&env, &token)?;
        token::Client::new(&env, &token).transfer(&admin, &env.current_contract_address(), &amount);
        let key = DataKey::Reserve(token);
        let reserve = get_i128(&env, &key)
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?;
        set_i128(&env, &key, reserve);
        Ok(())
    }

    pub fn deposit(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        user.require_auth();
        guarded(&env, || {
            if amount <= 0 {
                return Err(VaultError::InvalidAmount);
            }
            require_allowed(&env, &token)?;
            token::Client::new(&env, &token).transfer(
                &user,
                &env.current_contract_address(),
                &amount,
            );
            let key = DataKey::Balance(user.clone(), token.clone());
            let balance = get_i128(&env, &key)
                .checked_add(amount)
                .ok_or(VaultError::Overflow)?;
            set_i128(&env, &key, balance);
            Ok(())
        })
    }

    /// Swap `amount_in` of the caller's vault balance of `from_token` into
    /// `to_token`, returning the amount credited.
    pub fn swap(
        env: Env,
        user: Address,
        from_token: Address,
        to_token: Address,
        amount_in: i128,
        min_out: i128,
    ) -> Result<i128, VaultError> {
        user.require_auth();
        guarded(&env, || {
            if amount_in <= 0 || min_out < 0 {
                return Err(VaultError::InvalidAmount);
            }
            if from_token == to_token {
                return Err(VaultError::IdenticalTokens);
            }
            require_allowed(&env, &from_token)?;
            require_allowed(&env, &to_token)?;

            let from_key = DataKey::Balance(user.clone(), from_token.clone());
            let from_balance = get_i128(&env, &from_key);
            if from_balance < amount_in {
                return Err(VaultError::InsufficientBalance);
            }

            let reserve_in_key = DataKey::Reserve(from_token.clone());
            let reserve_out_key = DataKey::Reserve(to_token.clone());
            let reserve_in = get_i128(&env, &reserve_in_key);
            let reserve_out = get_i128(&env, &reserve_out_key);
            if reserve_in <= 0 || reserve_out <= 0 {
                return Err(VaultError::InsufficientLiquidity);
            }

            let new_reserve_in = reserve_in
                .checked_add(amount_in)
                .ok_or(VaultError::Overflow)?;
            let amount_out = reserve_out
                .checked_mul(amount_in)
                .ok_or(VaultError::Overflow)?
                / new_reserve_in;
            if amount_out <= 0 {
                return Err(VaultError::InsufficientLiquidity);
            }
            if amount_out < min_out {
                return Err(VaultError::SlippageExceeded);
            }

            let to_key = DataKey::Balance(user.clone(), to_token.clone());
            let to_balance = get_i128(&env, &to_key)
                .checked_add(amount_out)
                .ok_or(VaultError::Overflow)?;
            set_i128(&env, &from_key, from_balance - amount_in);
            set_i128(&env, &to_key, to_balance);
            set_i128(&env, &reserve_in_key, new_reserve_in);
            set_i128(&env, &reserve_out_key, reserve_out - amount_out);
            Ok(amount_out)
        })
    }

    /// Withdraw `amount` of `token` from the caller's vault balance. Not
    /// gated by the allowlist, so delisted tokens can still be withdrawn.
    pub fn withdraw(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        user.require_auth();
        guarded(&env, || {
            if amount <= 0 {
                return Err(VaultError::InvalidAmount);
            }
            let key = DataKey::Balance(user.clone(), token.clone());
            let balance = get_i128(&env, &key);
            if balance < amount {
                return Err(VaultError::InsufficientBalance);
            }
            set_i128(&env, &key, balance - amount);
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &user,
                &amount,
            );
            Ok(())
        })
    }

    pub fn balance(env: Env, user: Address, token: Address) -> i128 {
        get_i128(&env, &DataKey::Balance(user, token))
    }

    pub fn reserve(env: Env, token: Address) -> i128 {
        get_i128(&env, &DataKey::Reserve(token))
    }

    pub fn is_locked(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Locked)
            .unwrap_or(false)
    }
}
