#![allow(dead_code)]
use soroban_sdk::{contracttype, symbol_short, Address, Env};

use crate::errors::AutoTradeError;
use crate::storage::Signal;

// ==========================
// Types
// ==========================
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionResult {
    pub executed_amount: i128,
    pub executed_price: i128,
}

// ==========================
// Balance Check
// ==========================
pub fn has_sufficient_balance(env: &Env, user: &Address, _asset: &u32, amount: i128) -> bool {
    let key = (user.clone(), symbol_short!("balance"));
    let balance: i128 = env.storage().temporary().get(&key).unwrap_or(0);
    balance >= amount
}

pub fn get_available_liquidity(env: &Env, signal: &Signal, amount: i128) -> i128 {
    let key = (symbol_short!("liquidity"), signal.signal_id);
    env.storage().temporary().get(&key).unwrap_or(amount)
}

pub fn get_current_price(env: &Env, signal: &Signal) -> i128 {
    let key = (symbol_short!("price"), signal.signal_id);
    env.storage().temporary().get(&key).unwrap_or(signal.price)
}

// ==========================
// Market Order
// ==========================
pub fn execute_market_order(
    env: &Env,
    _user: &Address,
    signal: &Signal,
    amount: i128,
) -> Result<ExecutionResult, AutoTradeError> {
    let now = env.ledger().timestamp();

    if now >= signal.expiry {
        return Err(AutoTradeError::SignalExpired);
    }

    let available_liquidity = get_available_liquidity(env, signal, amount);

    if available_liquidity <= 0 {
        return Err(AutoTradeError::InsufficientLiquidity);
    }

    let executed_amount = core::cmp::min(amount, available_liquidity);

    Ok(ExecutionResult {
        executed_amount,
        executed_price: signal.price,
    })
}

// ==========================
// Limit Order
// ==========================
pub fn execute_limit_order(
    env: &Env,
    _user: &Address,
    signal: &Signal,
    amount: i128,
) -> Result<ExecutionResult, AutoTradeError> {
    let now = env.ledger().timestamp();

    if now >= signal.expiry {
        return Err(AutoTradeError::SignalExpired);
    }

    let market_price = get_current_price(env, signal);

    if market_price > signal.price {
        return Ok(ExecutionResult {
            executed_amount: 0,
            executed_price: 0,
        });
    }

    Ok(ExecutionResult {
        executed_amount: amount,
        executed_price: signal.price,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as TestAddress, Ledger};
    use soroban_sdk::{contract, symbol_short, Address, Env};

    #[contract]
    struct TestContract;

    fn setup_env() -> Env {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        env
    }

    fn setup_signal(env: &Env, id: u64) -> Signal {
        Signal {
            signal_id: id,
            price: 100,
            expiry: env.ledger().timestamp() + 1_000,
            base_asset: 1,
        }
    }

    /// Generate deterministic test addresses
    fn test_user(_env: &Env, _n: u8) -> Address {
        // Use Soroban TestAddress generator
        <Address as TestAddress>::generate(_env)
    }

    #[test]
    fn market_order_full_fill() {
        let env = setup_env();
        let user = test_user(&env, 1);
        let contract_addr = env.register(TestContract, ());

        let signal = setup_signal(&env, 1);

        env.as_contract(&contract_addr, || {
            // Initialize liquidity in storage
            let key = (symbol_short!("liquidity"), 1u64);
            env.storage().temporary().set(&key, &500i128);

            let res = execute_market_order(&env, &user, &signal, 400).unwrap();
            assert_eq!(res.executed_amount, 400);
            assert_eq!(res.executed_price, 100);
        });
    }

    #[test]
    fn market_order_partial_fill() {
        let env = setup_env();
        let user = test_user(&env, 2);
        let contract_addr = env.register(TestContract, ());

        let signal = setup_signal(&env, 2);

        env.as_contract(&contract_addr, || {
            let key = (symbol_short!("liquidity"), 2u64);
            env.storage().temporary().set(&key, &100i128);

            let res = execute_market_order(&env, &user, &signal, 300).unwrap();
            assert_eq!(res.executed_amount, 100);
            assert_eq!(res.executed_price, 100);
        });
    }

    #[test]
    fn limit_order_not_filled() {
        let env = setup_env();
        let user = test_user(&env, 3);
        let contract_addr = env.register(TestContract, ());

        let signal = setup_signal(&env, 3);

        env.as_contract(&contract_addr, || {
            let key = (symbol_short!("price"), 3u64);
            env.storage().temporary().set(&key, &150i128);

            let res = execute_limit_order(&env, &user, &signal, 200).unwrap();
            assert_eq!(res.executed_amount, 0);
            assert_eq!(res.executed_price, 0);
        });
    }

    #[test]
    fn expired_signal_rejected() {
        let env = setup_env();
        let user = test_user(&env, 4);
        let contract_addr = env.register(TestContract, ());

        let signal = Signal {
            signal_id: 4,
            price: 100,
            expiry: env.ledger().timestamp() - 1, // expired
            base_asset: 1,
        };

        env.as_contract(&contract_addr, || {
            let err = execute_market_order(&env, &user, &signal, 100).unwrap_err();
            assert_eq!(err, AutoTradeError::SignalExpired);
        });
    }
}
