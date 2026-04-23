#![cfg(test)]

use crate::{
 feature/copy-trade-balance-check
    errors::{ContractError, InsufficientBalanceDetail},
    risk_gates::{
        check_user_balance, DEFAULT_ESTIMATED_COPY_TRADE_FEE, MAX_POSITIONS_PER_USER,
    },

    errors::ContractError,
 feature/position-limit-copy-trade
    risk_gates::MAX_POSITIONS_PER_USER,
 main
    TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::Address as _,
feature/copy-trade-balance-check
    token::StellarAssetClient,

main
    Address, Env,
};

/// Minimal UserPortfolio: open count + hooks expected by [`TradeExecutorContract::execute_copy_trade`].
#[contract]
pub struct MockUserPortfolio;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    OpenCount(Address),
}

#[contractimpl]
impl MockUserPortfolio {
    pub fn get_open_position_count(env: Env, user: Address) -> u32 {
        env.storage()
            .instance()
            .get(&MockKey::OpenCount(user))
            .unwrap_or(0)
    }

    pub fn record_copy_position(env: Env, user: Address) {
        let key = MockKey::OpenCount(user.clone());
        let c: u32 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(c + 1));
    }

feature/copy-trade-balance-check

    /// Decrement open count (simulates closing one copy position). main
    pub fn close_one_copy_position(env: Env, user: Address) {
        let key = MockKey::OpenCount(user);
        let c: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if c > 0 {
            env.storage().instance().set(&key, &(c - 1));
        }
    }
}

feature/copy-trade-balance-check
const TRADE_AMOUNT: i128 = 1_000_000;

fn sac_token(env: &Env) -> Address {
    let issuer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    sac.address()
}

fn setup_with_balance(user_balance: i128) -> (Env, Address, Address, Address, Address, Address) {
fn setup() -> (Env, Address, Address, Address, Address) { main
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env); feature/copy-trade-balance-check
    let token = sac_token(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    StellarAssetClient::new(&env, &token).mint(&user, &user_balance);


    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

 main
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

feature/copy-trade-balance-check
    (env, exec_id, portfolio_id, user, admin, token)
}

#[test]
fn check_user_balance_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let amount: i128 = 100;
    let fee: i128 = 10;
    let required = amount + fee;
    StellarAssetClient::new(&env, &token).mint(&user, &(required - 1));

    let err = check_user_balance(&env, &user, &token, amount, fee);
    assert_eq!(
        err,
        Err(InsufficientBalanceDetail {
            required,
            available: required - 1,
        })
    );
}

#[test]
fn check_user_balance_exactly_sufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let amount: i128 = 100;
    let fee: i128 = 10;
    let required = amount + fee;
    StellarAssetClient::new(&env, &token).mint(&user, &required);

    assert!(check_user_balance(&env, &user, &token, amount, fee).is_ok());
}

#[test]
fn check_user_balance_more_than_sufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let amount: i128 = 100;
    let fee: i128 = 10;
    let required = amount + fee;
    StellarAssetClient::new(&env, &token).mint(&user, &(required + 1_000_000));

    assert!(check_user_balance(&env, &user, &token, amount, fee).is_ok());
}

#[test]
fn execute_copy_trade_insufficient_balance_sets_detail() {
    let required = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, _pf, user, _admin, token) = setup_with_balance(required - 1);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
        )
    });
    assert_eq!(err, Err(ContractError::InsufficientBalance));

    let detail = exec.get_insufficient_balance_detail(&user).unwrap();
    assert_eq!(
        detail,
        InsufficientBalanceDetail {
            required,
            available: required - 1,
        }
    );
}

#[test]
fn execute_copy_trade_sufficient_balance_invokes_portfolio() {
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(per + 1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT);
    assert!(exec.get_insufficient_balance_detail(&user).is_none());
    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        1
    );
}

#[test]
fn execute_copy_trade_zero_amount_invalid() {
    let (env, exec_id, _pf, user, _admin, token) = setup_with_balance(1_000_000_000);
    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            0,
        )
    });
    assert_eq!(err, Err(ContractError::InvalidAmount));
  
    (env, exec_id, portfolio_id, user, admin)
 main
}

#[test]
fn twenty_first_copy_trade_fails_until_one_closed() {
 feature/copy-trade-balance-check
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, portfolio_id, user, _admin, token) =
        setup_with_balance(per * 30 + 1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT);
    }

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
        )

    let (env, exec_id, portfolio_id, user, _admin) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user);
    }

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(env.clone(), user.clone())
 main
    });
    assert_eq!(err, Err(ContractError::PositionLimitReached));

    MockUserPortfolioClient::new(&env, &portfolio_id).close_one_copy_position(&user);

 feature/copy-trade-balance-check
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT);

    exec.execute_copy_trade(&user);
main

    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), MAX_POSITIONS_PER_USER);
}

#[test]
fn whitelisted_user_bypasses_position_limit() {
 feature/copy-trade-balance-check
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, portfolio_id, user, _admin, token) =
        setup_with_balance(per * 35 + 1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT);
    }

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
        )

    let (env, exec_id, portfolio_id, user, _admin) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user);
    }

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(env.clone(), user.clone())
 main
    });
    assert_eq!(err, Err(ContractError::PositionLimitReached));

    exec.set_position_limit_exempt(&user, &true);
    assert!(exec.is_position_limit_exempt(&user));

feature/copy-trade-balance-check
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT);
    exec.execute_copy_trade(&user);
main

    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), MAX_POSITIONS_PER_USER + 1);

    exec.set_position_limit_exempt(&user, &false);
    assert!(!exec.is_position_limit_exempt(&user));

    let err2 = env.as_contract(&exec_id, || {
 feature/copy-trade-balance-check
        crate::TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
        )
    });
    assert_eq!(err2, Err(ContractError::PositionLimitReached));

        crate::TradeExecutorContract::execute_copy_trade(env.clone(), user.clone())
    });
    assert_eq!(err2, Err(ContractError::PositionLimitReached));
  
    sdex::{self, execute_sdex_swap},
    TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl,
    symbol_short,
    testutils::Address as _,
    token::{self, StellarAssetClient},
    Address, Env, MuxedAddress,
};

/// Mock SDEX / aggregator: pulls input SAC via `transfer_from`, sends output SAC via `transfer`.
/// Configurable `amount_out` (default: `amount_in` if unset) simulates different fill levels.
#[contract]
pub struct MockSdexRouter;

#[contractimpl]
impl MockSdexRouter {
    pub fn set_amount_out(env: Env, out: i128) {
        env.storage().instance().set(&symbol_short!("amtout"), &out);
    }

    pub fn swap(
        env: Env,
        pull_from: Address,
        from_token: Address,
        to_token: Address,
        amount_in: i128,
        _min_out: i128,
        recipient: Address,
    ) -> i128 {
        let router = env.current_contract_address();
        let from_c = token::Client::new(&env, &from_token);
        from_c.transfer_from(&router, &pull_from, &router, &amount_in);

        let amount_out: i128 = env
            .storage()
            .instance()
            .get(&symbol_short!("amtout"))
            .unwrap_or(amount_in);

        let to_c = token::Client::new(&env, &to_token);
        let to_mux: MuxedAddress = recipient.into();
        to_c.transfer(&router, &to_mux, &amount_out);

        amount_out
    }
}

fn setup_executor_with_router(env: &Env) -> (Address, Address, Address, Address) {
    let admin = Address::generate(env);
    let sac_a = env.register_stellar_asset_contract_v2(admin.clone());
    let sac_b = env.register_stellar_asset_contract_v2(admin.clone());
    let token_a = sac_a.address();
    let token_b = sac_b.address();

    let router_id = env.register(MockSdexRouter, ());
    let exec_id = env.register(TradeExecutorContract, ());
    let exec = TradeExecutorContractClient::new(env, &exec_id);

    exec.initialize(&admin);
    exec.set_sdex_router(&router_id);

    // Input liquidity on executor; output liquidity on router (pool).
    let a_client = StellarAssetClient::new(env, &token_a);
    let b_client = StellarAssetClient::new(env, &token_b);
    a_client.mint(&exec_id, &1_000_000_000);
    b_client.mint(&router_id, &10_000_000_000);

    (exec_id, router_id, token_a, token_b)
}

#[test]
fn min_received_from_slippage_one_percent() {
    let amount: i128 = 1_000_000;
    let min = sdex::min_received_from_slippage(amount, 100).unwrap();
    assert_eq!(min, 990_000);
}

#[test]
fn swap_returns_actual_received() {
    let env = Env::default();
    env.mock_all_auths();

    let (exec_id, router_id, token_a, token_b) = setup_executor_with_router(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockSdexRouterClient::new(&env, &router_id).set_amount_out(&500_000);

    let out = exec.swap(&token_a, &token_b, &1_000_000, &400_000);
    assert_eq!(out, 500_000);
}

#[test]
fn swap_reverts_when_balance_below_min() {
    let env = Env::default();
    env.mock_all_auths();

    let (exec_id, router_id, token_a, token_b) = setup_executor_with_router(&env);

    MockSdexRouterClient::new(&env, &router_id).set_amount_out(&300_000);

    let err = env.as_contract(&exec_id, || {
        execute_sdex_swap(
            &env,
            &router_id,
            &token_a,
            &token_b,
            1_000_000,
            400_000,
        )
    });
    assert_eq!(err, Err(ContractError::SlippageExceeded));
}

#[test]
fn swap_with_slippage_matches_formula() {
    let env = Env::default();
    env.mock_all_auths();

    let (exec_id, router_id, token_a, token_b) = setup_executor_with_router(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    // 1% slippage => min = 990_000
    MockSdexRouterClient::new(&env, &router_id).set_amount_out(&995_000);

    let out = exec.swap_with_slippage(&token_a, &token_b, &1_000_000, &100);
    assert_eq!(out, 995_000);
}

#[test]
fn swap_with_slippage_reverts_when_exceeded() {
    let env = Env::default();
    env.mock_all_auths();

    let (exec_id, router_id, token_a, token_b) = setup_executor_with_router(&env);

    MockSdexRouterClient::new(&env, &router_id).set_amount_out(&980_000);

    let min = sdex::min_received_from_slippage(1_000_000, 100).unwrap();
    let err = env.as_contract(&exec_id, || {
        execute_sdex_swap(
            &env,
            &router_id,
            &token_a,
            &token_b,
            1_000_000,
            min,
        )
    });
    assert_eq!(err, Err(ContractError::SlippageExceeded));
main
 main
}

 feature/cancel-copy-trade
// ── cancel_copy_trade tests ───────────────────────────────────────────────────

/// Mock portfolio that tracks positions and supports has_position / close_position.
#[contract]
pub struct MockPortfolioWithPositions;

#[contracttype]
#[derive(Clone)]
enum PortfolioKey {
    Position(Address, u64),
    LastClosed,
}

#[contractimpl]
impl MockPortfolioWithPositions {
    pub fn add_position(env: Env, user: Address, trade_id: u64) {
        env.storage()
            .instance()
            .set(&PortfolioKey::Position(user, trade_id), &true);
    }
    pub fn has_position(env: Env, user: Address, trade_id: u64) -> bool {
        env.storage()
            .instance()
            .get(&PortfolioKey::Position(user, trade_id))
            .unwrap_or(false)
    }
    pub fn close_position(env: Env, user: Address, trade_id: u64, _pnl: i128) {
        env.storage()
            .instance()
            .remove(&PortfolioKey::Position(user, trade_id));
        env.storage()
            .instance()
            .set(&PortfolioKey::LastClosed, &trade_id);
    }
    /// Auth-propagation-safe entrypoint: executor passes itself as first arg.
    pub fn close_position_by_executor(env: Env, _executor: Address, user: Address, trade_id: u64, _pnl: i128) {
        env.storage()
            .instance()
            .remove(&PortfolioKey::Position(user, trade_id));
        env.storage()
            .instance()
            .set(&PortfolioKey::LastClosed, &trade_id);
    }
    pub fn last_closed(env: Env) -> Option<u64> {
        env.storage().instance().get(&PortfolioKey::LastClosed)
    }
    // Satisfy execute_copy_trade path (unused in cancel tests).
    pub fn get_open_position_count(_env: Env, _user: Address) -> u32 { 0 }
    pub fn record_copy_position(_env: Env, _user: Address) {}
}

// ── Auth propagation tests ────────────────────────────────────────────────────

/// cancel_copy_trade: executor calls close_position_by_executor (not close_position).
/// Verifies the position is closed without requiring user auth on the portfolio call.
#[test]
fn cancel_copy_trade_uses_executor_close_entrypoint() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &42u64);
    exec.cancel_copy_trade(&user, &user, &42u64, &token_a, &token_b, &1_000_000, &900_000);

    assert_eq!(
        MockPortfolioWithPositionsClient::new(&env, &portfolio_id).last_closed(),
        Some(42u64)
    );
}

/// cancel_copy_trade: attacker cannot close another user's position.
#[test]
fn cancel_copy_trade_attacker_cannot_close_other_user_position() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let attacker = Address::generate(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(), attacker.clone(), user.clone(), 1u64,
            token_a.clone(), token_b.clone(), 1_000_000, 900_000,
        )
    });
    assert_eq!(err, Err(ContractError::Unauthorized));
    assert!(MockPortfolioWithPositionsClient::new(&env, &portfolio_id).has_position(&user, &1u64));
}

fn setup_cancel(
    router_out: i128,
) -> (Env, Address, Address, Address, Address, Address, Address) {

// ── Reentrancy guard tests ────────────────────────────────────────────────────

/// A mock portfolio that calls back into execute_copy_trade during record_copy_position,
/// simulating a reentrant call.
#[contract]
pub struct ReentrantPortfolio;

#[contractimpl]
impl ReentrantPortfolio {
    pub fn set_executor(env: Env, exec: Address) {
        env.storage().instance().set(&soroban_sdk::symbol_short!("exec"), &exec);
    }
    pub fn set_user(env: Env, user: Address) {
        env.storage().instance().set(&soroban_sdk::symbol_short!("user"), &user);
    }
    pub fn get_open_position_count(_env: Env, _user: Address) -> u32 {
        0
    }
    pub fn record_copy_position(env: Env, user: Address) {
        // Attempt reentrant call back into execute_copy_trade.
        let exec: Address = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("exec"))
            .unwrap();
        let client = TradeExecutorContractClient::new(&env, &exec);
        // This reentrant call should return ReentrancyDetected.
        let result = client.try_execute_copy_trade(&user);
        // Store whether it was blocked so the test can assert.
        let blocked = matches!(
            result,
            Err(Ok(ContractError::ReentrancyDetected))
        );
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("blocked"), &blocked);
    }
    pub fn was_blocked(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("blocked"))
            .unwrap_or(false)
    }
}

#[test]
fn reentrant_call_returns_reentrancy_detected() {
  main
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

 feature/cancel-copy-trade
    let sac_a = env.register_stellar_asset_contract_v2(admin.clone());
    let sac_b = env.register_stellar_asset_contract_v2(admin.clone());
    let token_a = sac_a.address();
    let token_b = sac_b.address();

    soroban_sdk::token::StellarAssetClient::new(&env, &token_a).mint(&env.current_contract_address(), &1_000_000_000);
    soroban_sdk::token::StellarAssetClient::new(&env, &token_b).mint(&Address::generate(&env), &0); // ensure SAC exists

    let router_id = env.register(MockSdexRouter, ());
    MockSdexRouterClient::new(&env, &router_id).set_amount_out(&router_out);
    soroban_sdk::token::StellarAssetClient::new(&env, &token_b).mint(&router_id, &10_000_000_000);

    let portfolio_id = env.register(MockPortfolioWithPositions, ());

    let portfolio_id = env.register(ReentrantPortfolio, ());
 main
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);
 feature/cancel-copy-trade
    exec.set_sdex_router(&router_id);

    soroban_sdk::token::StellarAssetClient::new(&env, &token_a).mint(&exec_id, &1_000_000_000);

    (env, exec_id, portfolio_id, user, token_a, token_b, router_id)
}

/// User can cancel their own open position.
#[test]
fn cancel_copy_trade_success() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);

    exec.cancel_copy_trade(&user, &user, &1u64, &token_a, &token_b, &1_000_000, &900_000);

    assert_eq!(
        MockPortfolioWithPositionsClient::new(&env, &portfolio_id).last_closed(),
        Some(1u64)
    );
}

/// Caller != user returns Unauthorized.
#[test]
fn cancel_copy_trade_unauthorized() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let attacker = Address::generate(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(), attacker, user, 1u64, token_a, token_b, 1_000_000, 900_000,
        )
    });
    assert_eq!(err, Err(ContractError::Unauthorized));
}

/// Non-existent trade returns TradeNotFound.
#[test]
fn cancel_copy_trade_not_found() {
    let (env, exec_id, _portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(), user.clone(), user, 99u64, token_a, token_b, 1_000_000, 900_000,
        )
    });
    assert_eq!(err, Err(ContractError::TradeNotFound));
}

/// P&L is correctly computed as exit_price - amount.
#[test]
fn cancel_copy_trade_pnl_calculation() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_200_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &2u64);

    exec.cancel_copy_trade(&user, &user, &2u64, &token_a, &token_b, &1_000_000, &900_000);

    // Verify TradeCancelled event was emitted.
    let found = env.events().all().iter().any(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        topics.get(0)
            .and_then(|v| soroban_sdk::Symbol::try_from(v).ok())
            .map(|s| s == soroban_sdk::Symbol::new(&env, "TradeCancelled"))
            .unwrap_or(false)
    });
    assert!(found, "TradeCancelled event not emitted");


    // Wire the reentrant portfolio with the executor address and user.
    ReentrantPortfolioClient::new(&env, &portfolio_id).set_executor(&exec_id);
    ReentrantPortfolioClient::new(&env, &portfolio_id).set_user(&user);

    // First call succeeds; the reentrant call inside record_copy_position is blocked.
    exec.execute_copy_trade(&user);

    assert!(
        ReentrantPortfolioClient::new(&env, &portfolio_id).was_blocked(),
        "reentrant call was not blocked with ReentrancyDetected"
    );
}

#[test]
fn lock_cleared_after_successful_execution() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    // Two sequential calls must both succeed (lock is cleared between them).
    exec.execute_copy_trade(&user);
    exec.execute_copy_trade(&user);

    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        2
    );
 main
}

/// trade_cancelled event uses (trade_executor, trade_cancelled) two-topic format.
#[test]
fn event_trade_cancelled_two_topic_format() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);
    exec.cancel_copy_trade(&user, &user, &1u64, &token_a, &token_b, &1_000_000, &900_000);

    let found = env.events().all().iter().any(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        let t0 = topics.get(0).and_then(|v| soroban_sdk::Symbol::try_from(v).ok());
        let t1 = topics.get(1).and_then(|v| soroban_sdk::Symbol::try_from(v).ok());
        t0.map(|s| s == soroban_sdk::Symbol::new(&env, "trade_executor")).unwrap_or(false)
            && t1.map(|s| s == soroban_sdk::Symbol::new(&env, "trade_cancelled")).unwrap_or(false)
    });
    assert!(found, "trade_cancelled must use (trade_executor, trade_cancelled) topics");
}
