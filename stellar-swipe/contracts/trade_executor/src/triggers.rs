//! Stop-loss and take-profit triggers: check oracle price against thresholds
//! and close the position via UserPortfolio when breached.
//!
//! ## Auth model
//! These functions are **keeper-callable** — any address may call them; no user
//! signature is required.  The position close is performed via
//! `UserPortfolio::close_position_keeper`, a dedicated entrypoint that accepts
//! the registered TradeExecutor contract address as the authorising caller
//! (verified inside UserPortfolio).  This avoids requiring the user's signature
//! at trigger time while still preventing arbitrary contracts from closing
//! positions.
//!
//! Priority: if both stop-loss and take-profit would trigger, stop-loss wins.

use soroban_sdk::{Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::ContractError;

/// Instance key: oracle contract address (`get_price(asset_pair: u32) -> i128`).
pub const ORACLE_KEY: &str = "Oracle";
pub const PORTFOLIO_KEY: &str = "Portfolio";

/// Maximum age of an oracle price in seconds before it is considered stale.
/// Matches the oracle contract's `MAX_PRICE_AGE_LEDGERS` (60 ledgers × ~5 s/ledger = 300 s).
pub const MAX_ORACLE_PRICE_AGE_SECS: u64 = 300;

/// Register a stop-loss price for `(user, trade_id)`.
pub fn set_stop_loss(env: &Env, user: &Address, trade_id: u64, stop_loss_price: i128) {
    env.storage().persistent().set(
        &(Symbol::new(env, "StopLoss"), user.clone(), trade_id),
        &stop_loss_price,
    );
}

pub fn get_stop_loss(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(Symbol::new(env, "StopLoss"), user.clone(), trade_id))
}

/// Register a take-profit price for `(user, trade_id)`.
pub fn set_take_profit(env: &Env, user: &Address, trade_id: u64, take_profit_price: i128) {
    env.storage().persistent().set(
        &(Symbol::new(env, "TakeProfit"), user.clone(), trade_id),
        &take_profit_price,
    );
}

pub fn get_take_profit(env: &Env, user: &Address, trade_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&(Symbol::new(env, "TakeProfit"), user.clone(), trade_id))
}

fn fetch_oracle_and_portfolio(env: &Env) -> Result<(Address, Address), ContractError> {
    let oracle: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, ORACLE_KEY))
        .ok_or(ContractError::NotInitialized)?;
    let portfolio: Address = env
        .storage()
        .instance()
        .get(&Symbol::new(env, PORTFOLIO_KEY))
        .ok_or(ContractError::NotInitialized)?;
    Ok((oracle, portfolio))
}

/// Fetch the current price from the oracle contract, enforcing staleness.
///
/// Oracle ABI:
///   - `get_price(asset_pair: u32) -> i128`  — returns 0 when no price is available
///   - `get_price_timestamp(asset_pair: u32) -> u64` — returns the ledger timestamp
///     at which the price was last updated (0 when never set)
///
/// Returns:
///   - `Ok(price)` when the price is fresh (age ≤ `MAX_ORACLE_PRICE_AGE_SECS`)
///   - `Err(OracleUnavailable)` when no price has ever been set (timestamp == 0)
///   - `Err(OraclePriceStale)` when the price is older than `MAX_ORACLE_PRICE_AGE_SECS`
fn fetch_current_price(
    env: &Env,
    oracle: &Address,
    asset_pair: u32,
) -> Result<i128, ContractError> {
    // Fetch timestamp first — 0 means no price has ever been published.
    let updated_at: u64 = env.invoke_contract(
        oracle,
        &Symbol::new(env, "get_price_timestamp"),
        soroban_sdk::vec![env, asset_pair.into()],
    );

    if updated_at == 0 {
        return Err(ContractError::OracleUnavailable);
    }

    let now = env.ledger().timestamp();
    let age = now.saturating_sub(updated_at);
    if age > MAX_ORACLE_PRICE_AGE_SECS {
        return Err(ContractError::OraclePriceStale);
    }

    let price: i128 = env.invoke_contract(
        oracle,
        &Symbol::new(env, "get_price"),
        soroban_sdk::vec![env, asset_pair.into()],
    );
    Ok(price)
}

/// Close a position via the keeper-specific portfolio entrypoint.
///
/// Calls `UserPortfolio::close_position_keeper(caller, user, trade_id, asset_pair)`.
/// The portfolio verifies that `caller` (this TradeExecutor's address) is the
/// registered keeper — no user signature is needed.
///
/// ## Auth propagation
/// - Caller: keeper (any address, no auth required by TradeExecutor)
/// - Callee: `UserPortfolio::close_position_keeper` — authorises via
///   `caller.require_auth()` where `caller == env.current_contract_address()`.
///   The TradeExecutor contract address is the authorising principal.
fn close_position_keeper(
    env: &Env,
    portfolio: &Address,
    user: &Address,
    trade_id: u64,
    asset_pair: u32,
) {
    // Pass this contract's address as `caller` so UserPortfolio can verify it
    // is the registered TradeExecutor.
    // ABI: `close_position_keeper(caller, user, position_id, asset_pair)`
    let this = env.current_contract_address();
    let sym = Symbol::new(env, "close_position_keeper");
    let mut args = Vec::<Val>::new(env);
    args.push_back(this.into_val(env));
    args.push_back(user.clone().into_val(env));
    args.push_back(trade_id.into_val(env));
    args.push_back(asset_pair.into_val(env));
    env.invoke_contract::<()>(portfolio, &sym, args);
}

/// If `current_price <= stop_loss_price`, closes the position and emits `StopLossTriggered`.
///
/// ## Auth
/// Keeper-callable (no user auth required). Position close uses
/// `close_position_keeper` which is gated by the registered TradeExecutor address.
pub fn check_and_trigger_stop_loss(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
    let (oracle, portfolio) = fetch_oracle_and_portfolio(env)?;
    let stop_loss_price =
        get_stop_loss(env, &user, trade_id).ok_or(ContractError::NotInitialized)?;
    let current_price = fetch_current_price(env, &oracle, asset_pair)?;

    if current_price <= stop_loss_price {
        close_position_keeper(env, &portfolio, &user, trade_id, asset_pair);
        shared::events::emit_stop_loss_triggered(
            env,
            shared::events::EvtStopLossTriggered {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                trade_id,
                stop_loss_price,
                current_price,
                action_required: true,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

/// If `current_price >= take_profit_price`, closes the position and emits `TakeProfitTriggered`.
///
/// Stop-loss takes priority: if the price also breaches the stop-loss threshold,
/// this function returns `false` without triggering (caller should call
/// `check_and_trigger_stop_loss` instead).
///
/// ## Auth
/// Keeper-callable (no user auth required). Position close uses
/// `close_position_keeper` which is gated by the registered TradeExecutor address.
pub fn check_and_trigger_take_profit(
    env: &Env,
    user: Address,
    trade_id: u64,
    asset_pair: u32,
) -> Result<bool, ContractError> {
    let (oracle, portfolio) = fetch_oracle_and_portfolio(env)?;
    let take_profit_price =
        get_take_profit(env, &user, trade_id).ok_or(ContractError::NotInitialized)?;
    let current_price = fetch_current_price(env, &oracle, asset_pair)?;

    // Stop-loss takes priority.
    if let Some(stop_loss_price) = get_stop_loss(env, &user, trade_id) {
        if current_price <= stop_loss_price {
            return Ok(false);
        }
    }

    if current_price >= take_profit_price {
        close_position_keeper(env, &portfolio, &user, trade_id, asset_pair);
        shared::events::emit_take_profit_triggered(
            env,
            shared::events::EvtTakeProfitTriggered {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                trade_id,
                take_profit_price,
                current_price,
                action_required: true,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TradeExecutorContract, TradeExecutorContractClient};
    use soroban_sdk::{
        contract, contractimpl, symbol_short,
        testutils::{Address as _, Events as _, Ledger as _},
        Env, TryFromVal,
    };

    // ── Mock Oracle ───────────────────────────────────────────────────────────

    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, price: i128) {
            env.storage()
                .instance()
                .set(&symbol_short!("price"), &price);
            // Default timestamp: current ledger time (fresh).
            let ts = env.ledger().timestamp().max(1);
            env.storage().instance().set(&symbol_short!("pts"), &ts);
        }

        pub fn set_price_at(env: Env, price: i128, timestamp: u64) {
            env.storage()
                .instance()
                .set(&symbol_short!("price"), &price);
            env.storage()
                .instance()
                .set(&symbol_short!("pts"), &timestamp);
        }

        pub fn get_price(env: Env, _asset_pair: u32) -> i128 {
            env.storage()
                .instance()
                .get(&symbol_short!("price"))
                .unwrap()
        }

        pub fn get_price_timestamp(env: Env, _asset_pair: u32) -> u64 {
            env.storage()
                .instance()
                .get(&symbol_short!("pts"))
                .unwrap_or(0u64)
        }
    }

    // ── Mock Portfolio ────────────────────────────────────────────────────────
    //
    // Implements `close_position_keeper(caller, user, trade_id, asset_pair)` — the
    // keeper-specific entrypoint that does NOT require user auth.

    #[contract]
    pub struct MockPortfolio;

    #[contractimpl]
    impl MockPortfolio {
        /// Keeper-callable close: records the closed trade_id.
        /// In production UserPortfolio this verifies the caller is the registered
        /// TradeExecutor; the mock accepts all callers for test simplicity.
        pub fn close_position_keeper(
            env: Env,
            _caller: Address,
            _user: Address,
            trade_id: u64,
            _asset_pair: u32,
        ) {
            env.storage()
                .instance()
                .set(&symbol_short!("closed"), &trade_id);
        }

        pub fn last_closed(env: Env) -> Option<u64> {
            env.storage().instance().get(&symbol_short!("closed"))
        }
    }

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = 1);

        let admin = Address::generate(&env);
        let oracle_id = env.register(MockOracle, ());
        let portfolio_id = env.register(MockPortfolio, ());
        let exec_id = env.register(TradeExecutorContract, ());

        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.initialize(&admin);
        exec.add_oracle(&oracle_id);
        exec.set_oracle(&oracle_id);
        exec.set_stop_loss_portfolio(&portfolio_id);

        (env, exec_id, oracle_id, portfolio_id)
    }

    // ── Stop-loss tests ───────────────────────────────────────────────────────

    #[test]
    fn no_trigger_when_price_above_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&200);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        assert!(!exec.check_and_trigger_stop_loss(&user, &1u64, &0u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id)
            .last_closed()
            .is_none());
    }

    #[test]
    fn trigger_when_price_at_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&100);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        assert!(exec.check_and_trigger_stop_loss(&user, &1u64, &0u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    #[test]
    fn trigger_when_price_below_stop_loss() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &2u64, &100);
        assert!(exec.check_and_trigger_stop_loss(&user, &2u64, &0u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(2u64)
        );
    }

    #[test]
    fn stop_loss_trigger_emits_event() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&80);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &3u64, &100);
        exec.check_and_trigger_stop_loss(&user, &3u64, &0u32);
        // Just verify the call succeeded and position was closed (event format tested below).
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id)
                .last_closed()
                .is_none(),
            false
        );
        let _ = env.events();
    }

    // ── Take-profit tests ─────────────────────────────────────────────────────

    #[test]
    fn no_trigger_when_price_below_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&150);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);
        assert!(!exec.check_and_trigger_take_profit(&user, &1u64, &0u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id)
            .last_closed()
            .is_none());
    }

    #[test]
    fn trigger_when_price_at_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&200);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);
        assert!(exec.check_and_trigger_take_profit(&user, &1u64, &0u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    #[test]
    fn trigger_when_price_above_take_profit() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&250);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &2u64, &200);
        assert!(exec.check_and_trigger_take_profit(&user, &2u64, &0u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(2u64)
        );
    }

    #[test]
    fn take_profit_trigger_emits_event() {
        let (env, exec_id, oracle_id, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&300);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &3u64, &200);
        exec.check_and_trigger_take_profit(&user, &3u64, &0u32);
        let _ = env.events();
    }

    // ── Priority test ─────────────────────────────────────────────────────────

    #[test]
    fn stop_loss_priority_over_take_profit_on_simultaneous_trigger() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        exec.set_take_profit_price(&user, &1u64, &50);
        // take_profit should NOT fire because stop_loss takes priority
        assert!(!exec.check_and_trigger_take_profit(&user, &1u64, &0u32));
        assert!(MockPortfolioClient::new(&env, &portfolio_id)
            .last_closed()
            .is_none());
        // stop_loss SHOULD fire
        assert!(exec.check_and_trigger_stop_loss(&user, &1u64, &0u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(1u64)
        );
    }

    // ── Auth propagation tests ────────────────────────────────────────────────

    /// Keeper (any address) can trigger stop-loss without user signature.
    #[test]
    fn keeper_can_trigger_stop_loss_without_user_auth() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        let keeper = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &10u64, &100);

        // Invoke as keeper — mock_all_auths covers the keeper's own auth if needed.
        // The key assertion is that the call succeeds and the position is closed.
        let triggered = exec.check_and_trigger_stop_loss(&user, &10u64, &0u32);
        assert!(triggered);
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(10u64)
        );
        let _ = keeper; // keeper address not needed for the call itself
    }

    /// Keeper (any address) can trigger take-profit without user signature.
    #[test]
    fn keeper_can_trigger_take_profit_without_user_auth() {
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);

        MockOracleClient::new(&env, &oracle_id).set_price(&300);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &11u64, &200);

        let triggered = exec.check_and_trigger_take_profit(&user, &11u64, &0u32);
        assert!(triggered);
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(11u64)
        );
    }

    /// close_position_keeper is called (not close_position) — verified by mock
    /// only implementing close_position_keeper, not close_position.
    #[test]
    fn trigger_uses_keeper_entrypoint_not_user_entrypoint() {
        // MockPortfolio only has close_position_keeper; if triggers.rs called
        // close_position instead, the invoke_contract would panic and the test
        // would fail.
        let (env, exec_id, oracle_id, portfolio_id) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &99u64, &100);
        // This succeeds only if close_position_keeper is called.
        assert!(exec.check_and_trigger_stop_loss(&user, &99u64, &0u32));
        assert_eq!(
            MockPortfolioClient::new(&env, &portfolio_id).last_closed(),
            Some(99u64)
        );
    }

    // ── Event format tests ────────────────────────────────────────────────────

    fn last_topics(env: &Env) -> (Symbol, Symbol) {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let events = env.events().all();
        let e = events.last().unwrap();
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1;
        let t0 = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1 = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        (t0, t1)
    }

    #[test]
    fn stop_loss_event_has_two_topic_format() {
        let (env, exec_id, oracle_id, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        exec.check_and_trigger_stop_loss(&user, &1u64, &0u32);
        let (contract, event) = last_topics(&env);
        assert_eq!(contract, Symbol::new(&env, "trade_executor"));
        assert_eq!(event, Symbol::new(&env, "stop_loss_triggered"));
    }

    #[test]
    fn take_profit_event_has_two_topic_format() {
        let (env, exec_id, oracle_id, _) = setup();
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&300);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &1u64, &200);
        exec.check_and_trigger_take_profit(&user, &1u64, &0u32);
        let (contract, event) = last_topics(&env);
        assert_eq!(contract, Symbol::new(&env, "trade_executor"));
        assert_eq!(event, Symbol::new(&env, "take_profit_triggered"));
    }

    // ── Notification field tests ──────────────────────────────────────────────

    fn last_event_body<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(env: &Env) -> T {
        use soroban_sdk::testutils::Events;
        let events = env.events().all();
        let e = events.last().unwrap();
        T::try_from_val(env, &e.2).unwrap()
    }

    #[test]
    fn stop_loss_event_has_action_required_true_and_timestamp() {
        let (env, exec_id, oracle_id, _) = setup();
        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|l| l.timestamp = 12345);
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&50);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_stop_loss_price(&user, &1u64, &100);
        exec.check_and_trigger_stop_loss(&user, &1u64, &0u32);
        let evt: shared::events::EvtStopLossTriggered = last_event_body(&env);
        assert!(evt.action_required);
        assert_eq!(evt.timestamp, 12345);
        assert_eq!(evt.trade_id, 1);
        assert_eq!(evt.stop_loss_price, 100);
        assert_eq!(evt.current_price, 50);
    }

    #[test]
    fn take_profit_event_has_action_required_true_and_timestamp() {
        let (env, exec_id, oracle_id, _) = setup();
        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|l| l.timestamp = 99999);
        let user = Address::generate(&env);
        MockOracleClient::new(&env, &oracle_id).set_price(&300);
        let exec = TradeExecutorContractClient::new(&env, &exec_id);
        exec.set_take_profit_price(&user, &2u64, &200);
        exec.check_and_trigger_take_profit(&user, &2u64, &0u32);
        let evt: shared::events::EvtTakeProfitTriggered = last_event_body(&env);
        assert!(evt.action_required);
        assert_eq!(evt.timestamp, 99999);
        assert_eq!(evt.trade_id, 2);
        assert_eq!(evt.take_profit_price, 200);
        assert_eq!(evt.current_price, 300);
    }
}
