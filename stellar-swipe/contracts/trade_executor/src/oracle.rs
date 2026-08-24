//! Oracle whitelist management for the trade executor.

use soroban_sdk::{Address, Env};

use crate::{ContractError, StorageKey};

pub fn require_admin(env: &Env) -> Result<Address, ContractError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(ContractError::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}

pub fn is_whitelisted(env: &Env, oracle: &Address) -> bool {
    env.storage()
        .instance()
        .get(&StorageKey::OracleWhitelisted(oracle.clone()))
        .unwrap_or(false)
}

pub fn count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::OracleWhitelistCount)
        .unwrap_or(0)
}

pub fn add(env: &Env, oracle: Address) -> Result<(), ContractError> {
    require_admin(env)?;

    let key = StorageKey::OracleWhitelisted(oracle);
    if env
        .storage()
        .instance()
        .get::<_, bool>(&key)
        .unwrap_or(false)
    {
        return Ok(());
    }

    let next = count(env).saturating_add(1);
    env.storage().instance().set(&key, &true);
    env.storage()
        .instance()
        .set(&StorageKey::OracleWhitelistCount, &next);

    Ok(())
}

pub fn remove(env: &Env, oracle: Address) -> Result<(), ContractError> {
    require_admin(env)?;

    let key = StorageKey::OracleWhitelisted(oracle);
    if !env
        .storage()
        .instance()
        .get::<_, bool>(&key)
        .unwrap_or(false)
    {
        return Ok(());
    }

    let current = count(env);
    if current <= 1 {
        return Err(ContractError::CannotRemoveLastOracle);
    }

    env.storage().instance().remove(&key);
    env.storage()
        .instance()
        .set(&StorageKey::OracleWhitelistCount, &(current - 1));

    Ok(())
}

pub fn require_whitelisted(env: &Env, oracle: &Address) -> Result<(), ContractError> {
    if is_whitelisted(env, oracle) {
        Ok(())
    } else {
        Err(ContractError::OracleNotWhitelisted)
    }
}
