 feature/take-profit-trigger
//! Stop-loss and take-profit triggers: check oracle price against position thresholds
//! and close the position via UserPortfolio when breached.
//!
//! Constraint: uses oracle price only — never SDEX spot — for manipulation resistance.
//! Priority: if both stop-loss and take-profit trigger simultaneously, stop-loss wins.

//! Stop-loss trigger: checks oracle price against a position's stop-loss threshold
//! and closes the position via UserPortfolio when breached.
//!
//! Constraint: uses oracle price only — never SDEX spot — for manipulation resistance.
 main

use soroban_sdk::{symbol_short, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::ContractError;

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Instance key: oracle contract address (`get_price(asset_pair: u32) -> i128`).
pub const ORACLE_KEY: &str = "Oracle";
/// Instance key: user-portfolio contract address (`close_position(user, trade_id, pnl)`).
pub const PORTFOLIO_KEY: &str = "Portfolio";
 feature/take-profit-trigger

/// Persistent key prefix: stop-loss price per (user, trade_id).
pub const SL_KEY: &str = "StopLoss";
 main

// ── Public helpers ────────────────────────────────────────────────────────────

/// Register a stop-loss price for `(user, trade_id)`.
pub fn set_stop_loss(env: &Env, user: &Address, trade_id: u64, stop_loss_price: i128) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("StopLoss"), user.clone(), trade_id), &stop_loss_price);
}

/// Return the registered stop-loss price, if any.
pub fn get_stop_loss(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(symbol_short!("StopLoss"), user.clone(), trade_id))
}

 feature/take-profit-trigger
/// Register a take-profit price for `(user, trade_id)`.
pub fn set_take_profit(env: &Env, user: &Address, trade_id: u64, take_profit_price: i128) {
    env.storage()
        .persistent()
        .set(&(symbol_short!("TakeProfit"), user.clone(), trade_id), &take_profit_price);
}

/// Return the registered take-profit price, if any.
pub fn get_take_profit(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(symbol_short!("TakeProfit"), user.clone(), trade_id))
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn fetch_oracle_and_portfolio(env: &Env) -> Result<(Address, Address), ContractError> {

// ── Core trigger ──────────────────────────────────────────────────────────────

/// Check the oracle price for `asset_pair` against the registered stop-loss for
/// `(user, trade_id)`.  If `current_price <= stop_loss_price`, calls
/// `close_position(user, trade_id, 0)` on the portfolio contract and emits
/// `StopLossTriggered`.
///
/// Returns `Ok(true)` when triggered, `Ok(false)` when price is above threshold.
/// Returns `Err(ContractError::NotInitialized)` when oracle or portfolio are not
/// configured, or when no stop-loss is registered for the position.
pub fn check_and_trigger_stop_loss(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
 main
    let oracle: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, ORACLE_KEY))
        .ok_or(ContractError::NotInitialized)?;
 feature/take-profit-trigger


 main
    let portfolio: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, PORTFOLIO_KEY))
        .ok_or(ContractError::NotInitialized)?;
 feature/take-profit-trigger
    Ok((oracle, portfolio))
}

fn close_position(env: &Env, portfolio: &Address, user: &Address, trade_id: u64) {
    // Use close_position_by_executor so the portfolio accepts this call without user auth.
    // The TradeExecutor (current contract) must be registered as the authorized executor
    // in the UserPortfolio via `set_authorized_executor`.
    let close_sym = Symbol::new(env, "close_position_by_executor");
    let executor = env.current_contract_address();
    let mut args = Vec::<Val>::new(env);
    args.push_back(executor.clone().into_val(env));
    args.push_back(user.clone().into_val(env));
    args.push_back(trade_id.into_val(env));
    args.push_back(0i128.into_val(env));
    env.invoke_contract::<()>(portfolio, &close_sym, args);
}

// ── Core triggers ─────────────────────────────────────────────────────────────

/// Check oracle price against stop-loss for `(user, trade_id)`.
/// If `current_price <= stop_loss_price`, closes the position and emits `StopLossTriggered`.
/// Returns `Ok(true)` when triggered, `Ok(false)` otherwise.
pub fn check_and_trigger_stop_loss(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
    let (oracle, portfolio) = fetch_oracle_and_portfolio(env)?;

 main

    let stop_loss_price: i128 = env
        .storage()
        .persistent()
        .get(&(symbol_short!("StopLoss"), user.clone(), trade_id))
        .ok_or(ContractError::NotInitialized)?;

 feature/take-profit-trigger

    // Fetch oracle price (manipulation-resistant; never SDEX spot).
 main
    let current_price: i128 = env.invoke_contract(
        &oracle,
        &Symbol::new(env, "get_price"),
        soroban_sdk::vec![env, asset_pair.into()],
    );

    if current_price <= stop_loss_price {
 feature/take-profit-trigger
        close_position(env, &portfolio, &user, trade_id);

        // Close the position via UserPortfolio using the executor entrypoint.
        let close_sym = Symbol::new(env, "close_position_by_executor");
        let executor = env.current_contract_address();
        let mut args = Vec::<Val>::new(env);
        args.push_back(executor.clone().into_val(env));
        args.push_back(user.clone().into_val(env));
        args.push_back(trade_id.into_val(env));
        args.push_back(0i128.into_val(env));
        env.invoke_contract::<()>(&portfolio, &close_sym, args);

        // Emit StopLossTriggered event.
 main
        env.events().publish(
            (Symbol::new(env, "StopLossTriggered"), user.clone()),
            (trade_id, stop_loss_price, current_price),
        );
 feature/take-profit-trigger
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Check oracle price against take-profit for `(user, trade_id)`.
/// If `current_price >= take_profit_price`, closes the position and emits `TakeProfitTriggered`.
/// If both stop-loss and take-profit would trigger simultaneously, stop-loss takes priority.
/// Returns `Ok(true)` when triggered, `Ok(false)` otherwise.
pub fn check_and_trigger_take_profit(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
    let (oracle, portfolio) = fetch_oracle_and_portfolio(env)?;

    let take_profit_price: i128 = env
        .storage()
        .persistent()
        .get(&(symbol_short!("TakeProfit"), user.clone(), trade_id))
        .ok_or(ContractError::NotInitialized)?;

    let current_price: i128 = env.invoke_contract(
        &oracle,
        &Symbol::new(env, "get_price"),
        soroban_sdk::vec![env, asset_pair.into()],
    );

    // Stop-loss takes priority: if stop-loss would also trigger, do not take-profit.
    if let Some(stop_loss_price) = get_stop_loss(env, &user, trade_id) {
        if current_price <= stop_loss_price {
            return Ok(false);
        }
    }

    if current_price >= take_profit_price {
        close_position(env, &portfolio, &user, trade_id);
        env.events().publish(
            (Symbol::new(env, "TakeProfitTriggered"), user.clone()),
            (trade_id, take_profit_price, current_price),
        );

 main
        Ok(true)
    } else {
        Ok(false)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
 feature/take-profit-trigger
    use crate::{TradeExecutorContract, TradeExecutorContractClient};

    use crate::{StorageKey, TradeExecutorContract, TradeExecutorContractClient};
  main
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env};

    // ── Mock oracle ───────────────────────────────────────────────────────────

    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, price: i128) {
            env.storage().instance().set(&symbol_short!("price"), &price);
        }
        pub fn get_price(env: Env, _asset_pair: u32) -> i128 {
            env.storage()
                .instance()
                .get(&symbol_short!("price"))
                .unwrap_or(0)
        }
    }

    // ── Mock portfolio ────────────────────────────────────────────────────────

    #[contract]
    pub struct MockPortfolio;

    #[contractimpl]
    impl MockPortfolio {
        pub fn close_position(env: Env, _user: Address, trade_id: u64, _pnl: i128) {
            env.storage()
                .instance()
                .set(&symbol_short!("closed"), &trade_id);
        }
        pub fn last_closed(env: Env) -> Option<u64> {
            env.storage().instance().get(&symbol_short!("closed"))
        }
    }

    // ── Setup helper ──────────────────────────────────────────────────────────

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle_id = env.register(MockOracle, ());
        let portfolio_id = env.register(MockPortfolio, ());
        let exec_id = env.register(TradeExecutorContract, ());

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.initialize(&admin);
        exec.set_oracle(&oracle_id);
        exec.set_stop_loss_portfolio(&portfolio_id);

        (env, exec_id, oracle_id, portfolio_id, admin)
    }

 feature/take-profit-trigger
    // ── Stop-loss tests ───────────────────────────────────────────────────────

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// Price above stop-loss → no trigger, portfolio untouched.
 main
    #[test]
    fn no_trigger_when_price_above_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);
 feature/take-profit-trigger
        MockOracleClient::new(&env, &oracle_id).set_price(&200);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        assert!(!exec.check_and_trigger_stop_loss(&user, &1u64, &0u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id).last_closed().is_none());
    }



        MockOracleClient::new(&env, &oracle_id).set_price(&200);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);

        let triggered = exec.check_and_trigger_stop_loss(&user, &1u64, &0u32);
        assert!(!triggered);
        assert!(MockPortfolioClient::new(&env, &portfolio_id).last_closed().is_none());
    }

    /// Price exactly at stop-loss → triggers.
 main
    #[test]
    fn trigger_when_price_at_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);
 feature/take-profit-trigger
        MockOracleClient::new(&env, &oracle_id).set_price(&100);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        assert!(exec.check_and_trigger_stop_loss(&user, &1u64, &0u32));
        assert_eq!(MockPortfolioClient::new(&env, &portfolio_id).last_closed(), Some(1u64));
    }



        MockOracleClient::new(&env, &oracle_id).set_price(&100);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);

        let triggered = exec.check_and_trigger_stop_loss(&user, &1u64, &0u32);
        assert!(triggered);
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    /// Price below stop-loss → triggers.
 main
    #[test]
    fn trigger_when_price_below_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);
 feature/take-profit-trigger
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &2u64, &100);
        assert!(exec.check_and_trigger_stop_loss(&user, &2u64, &0u32));
        assert_eq!(MockPortfolioClient::new(&env, &portfolio_id).last_closed(), Some(2u64));
    }

    #[test]
    fn stop_loss_trigger_emits_event() {
        let (env, exec_id, oracle_id, _, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&80);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &3u64, &100);
        exec.check_and_trigger_stop_loss(&user, &3u64, &0u32);
        let found = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            topics.get(0).and_then(|v| soroban_sdk::Symbol::try_from(v).ok())
                .map(|s| s == Symbol::new(&env, "StopLossTriggered"))
                .unwrap_or(false)
        });
        assert!(found, "StopLossTriggered event not emitted");
    }

    // ── Take-profit tests ─────────────────────────────────────────────────────

    /// Price below take-profit → no trigger.
    #[test]
    fn no_trigger_when_price_below_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&150);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);
        assert!(!exec.check_and_trigger_take_profit(&user, &1u64, &0u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id).last_closed().is_none());
    }

    /// Price exactly at take-profit → triggers.
    #[test]
    fn trigger_when_price_at_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&200);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);
        assert!(exec.check_and_trigger_take_profit(&user, &1u64, &0u32));
        assert_eq!(MockPortfolioClient::new(&env, &portfolio_id).last_closed(), Some(1u64));
    }

    /// Price above take-profit → triggers.
    #[test]
    fn trigger_when_price_above_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&250);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &2u64, &200);
        assert!(exec.check_and_trigger_take_profit(&user, &2u64, &0u32));
        assert_eq!(MockPortfolioClient::new(&env, &portfolio_id).last_closed(), Some(2u64));
    }

    /// Take-profit emits TakeProfitTriggered event.
    #[test]
    fn take_profit_trigger_emits_event() {
        let (env, exec_id, oracle_id, _, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&300);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &3u64, &200);
        exec.check_and_trigger_take_profit(&user, &3u64, &0u32);
        let found = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            topics.get(0).and_then(|v| soroban_sdk::Symbol::try_from(v).ok())
                .map(|s| s == Symbol::new(&env, "TakeProfitTriggered"))
                .unwrap_or(false)
        });
        assert!(found, "TakeProfitTriggered event not emitted");
    }

    /// Stop-loss takes priority when both would trigger simultaneously.
    #[test]
    fn stop_loss_priority_over_take_profit_on_simultaneous_trigger() {
        let (env, exec_id, oracle_id, portfolio_id, _) = setup();
        let user = Address::generate(&env);

        // Price of 50 is both <= stop_loss(100) and >= take_profit(50) simultaneously.
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        exec.set_take_profit_price(&user, &1u64, &50);

        // Take-profit should NOT trigger because stop-loss takes priority.
        assert!(!exec.check_and_trigger_take_profit(&user, &1u64, &0u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id).last_closed().is_none());

        // Stop-loss SHOULD trigger.
        assert!(exec.check_and_trigger_stop_loss(&user, &1u64, &0u32));
        assert_eq!(MockPortfolioClient::new(&env, &portfolio_id).last_closed(), Some(1u64));


        MockOracleClient::new(&env, &oracle_id).set_price(&50);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &2u64, &100);

        let triggered = exec.check_and_trigger_stop_loss(&user, &2u64, &0u32);
        assert!(triggered);
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(2u64)
        );
    }

    /// Event is emitted on trigger with correct prices.
    #[test]
    fn trigger_emits_stop_loss_event() {
        let (env, exec_id, oracle_id, _, _) = setup();
        let user = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&80);

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &3u64, &100);
        exec.check_and_trigger_stop_loss(&user, &3u64, &0u32);

        let events = env.events().all();
        // Each event is (contract_id, topics, data); find StopLossTriggered by topic symbol.
        let found = events.iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if let Some(first) = topics.get(0) {
                if let Ok(sym) = soroban_sdk::Symbol::try_from(first) {
                    return sym == Symbol::new(&env, "StopLossTriggered");
                }
            }
            false
        });
        assert!(found, "StopLossTriggered event not emitted");
 main
    }
}
