#![no_std]

mod errors;

pub mod sdex;

use errors::ContractError;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

use sdex::{execute_sdex_swap, min_received_from_slippage};

#[contracttype]
#[derive(Clone)]
enum StorageKey {
    Admin,
    SdexRouter,
}

#[contract]
pub struct TradeExecutorContract;

#[contractimpl]
impl TradeExecutorContract {
    /// One-time init; stores admin who may configure the SDEX router address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    /// Set the router contract invoked by [`sdex::execute_sdex_swap`].
    pub fn set_sdex_router(env: Env, router: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&StorageKey::SdexRouter, &router);
    }

    /// Read configured router (for off-chain tooling).
    pub fn get_sdex_router(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::SdexRouter)
    }

    /// Swap using a caller-supplied minimum output (already includes slippage tolerance).
    pub fn swap(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
    ) -> Result<i128, ContractError> {
        let router = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;
        execute_sdex_swap(
            &env,
            &router,
            &from_token,
            &to_token,
            amount,
            min_received,
        )
    }

    /// Swap with `min_received = amount * (10000 - max_slippage_bps) / 10000`.
    pub fn swap_with_slippage(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        max_slippage_bps: u32,
    ) -> Result<i128, ContractError> {
        let min_received =
            min_received_from_slippage(amount, max_slippage_bps).ok_or(ContractError::InvalidAmount)?;
        Self::swap(env, from_token, to_token, amount, min_received)
    }
}

#[cfg(test)]
mod test;
