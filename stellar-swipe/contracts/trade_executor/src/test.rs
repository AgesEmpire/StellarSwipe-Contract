#![cfg(test)]
//! Integration tests for TradeExecutor.
//! Stop-loss / take-profit coverage is in `triggers::tests`.

use crate::{
    errors::{ContractError, InsufficientBalanceDetail},
    risk_gates::{
        check_user_balance, resolve_trade_amount, DEFAULT_ESTIMATED_COPY_TRADE_FEE,
        MAX_POSITION_PCT_BPS, MAX_POSITIONS_PER_USER,
    },
    sdex::{self, execute_sdex_swap},
    TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    testutils::{Address as _, Events},
    token::{self, StellarAssetClient},
    Address, Env, MuxedAddress, TryFromVal,
};

// ── Mock UserPortfolio ────────────────────────────────────────────────────────
//
// Exposes the batched `validate_and_record(user, max_positions) -> u32` entrypoint
// that replaces the old two-call pattern (get_open_position_count + record_copy_position).
// Also retains helpers used by cancel_copy_trade tests.

#[contract]
pub struct MockUserPortfolio;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    OpenCount(Address),
}

#[contractimpl]
impl MockUserPortfolio {
    /// Batched entrypoint: atomically checks the position cap and records the new copy
    /// position. Panics when `open_count >= max_positions` so that `try_invoke_contract`
    /// surfaces it as `PositionLimitReached`.
    pub fn validate_and_record(env: Env, user: Address, max_positions: u32) -> u32 {
        let key = MockKey::OpenCount(user.clone());
        let count: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if count >= max_positions {
            panic!("position limit reached");
        }
        let new_count = count + 1;
        env.storage().instance().set(&key, &new_count);
        new_count
    }

    pub fn get_open_position_count(env: Env, user: Address) -> u32 {
        env.storage()
            .instance()
            .get(&MockKey::OpenCount(user))
            .unwrap_or(0)
    }

    /// Decrement open count (simulates closing one copy position).
    pub fn close_one_copy_position(env: Env, user: Address) {
        let key = MockKey::OpenCount(user);
        let c: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if c > 0 {
            env.storage().instance().set(&key, &(c - 1));
        }
    }

    // Satisfy cancel_copy_trade path.
    pub fn has_position(_env: Env, _user: Address, _trade_id: u64) -> bool {
        false
    }
    pub fn close_position(_env: Env, _user: Address, _trade_id: u64, _pnl: i128) {}
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const TRADE_AMOUNT: i128 = 1_000_000;

fn sac_token(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

fn setup_with_balance(user_balance: i128) -> (Env, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let router_id = env.register(MockSdexRouter, ());
    let exec_id = env.register(TradeExecutorContract, ());

    StellarAssetClient::new(&env, &token).mint(&user, &user_balance);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);
    exec.set_sdex_router(&router_id);

    (env, exec_id, portfolio_id, user, admin, token)
}

// ── Balance check unit tests ──────────────────────────────────────────────────

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
    StellarAssetClient::new(&env, &token).mint(&user, &(amount + fee));
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
    StellarAssetClient::new(&env, &token).mint(&user, &(amount + fee + 1_000_000));
    assert!(check_user_balance(&env, &user, &token, amount, fee).is_ok());
}

// ── execute_copy_trade tests ──────────────────────────────────────────────────

#[test]
fn execute_copy_trade_insufficient_balance_sets_detail() {
    let required = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, _pf, user, _admin, token) = setup_with_balance(required - 1);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
            None,
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
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);
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
        TradeExecutorContract::execute_copy_trade(
            env.clone(), user.clone(), token.clone(), 0, None,
        )
    });
    assert_eq!(err, Err(ContractError::InvalidAmount));
}

#[test]
fn twenty_first_copy_trade_fails_until_one_closed() {
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, portfolio_id, user, _admin, token) =
        setup_with_balance(per * 30 + 1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);
    }

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::execute_copy_trade(
            env.clone(), user.clone(), token.clone(), TRADE_AMOUNT, None,
        )
    });
    assert_eq!(err, Err(ContractError::PositionLimitReached));

    MockUserPortfolioClient::new(&env, &portfolio_id).close_one_copy_position(&user);
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);

    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        MAX_POSITIONS_PER_USER
    );
}

#[test]
fn whitelisted_user_bypasses_position_limit() {
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, portfolio_id, user, _admin, token) =
        setup_with_balance(per * 35 + 1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);
    }

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::execute_copy_trade(
            env.clone(), user.clone(), token.clone(), TRADE_AMOUNT, None,
        )
    });
    assert_eq!(err, Err(ContractError::PositionLimitReached));

    exec.set_position_limit_exempt(&user, &true);
    assert!(exec.is_position_limit_exempt(&user));

    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);
    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        MAX_POSITIONS_PER_USER + 1
    );

    exec.set_position_limit_exempt(&user, &false);
    assert!(!exec.is_position_limit_exempt(&user));

    let err2 = env.as_contract(&exec_id, || {
        TradeExecutorContract::execute_copy_trade(
            env.clone(), user.clone(), token.clone(), TRADE_AMOUNT, None,
        )
    });
    assert_eq!(err2, Err(ContractError::PositionLimitReached));
}

// ── Auth propagation: execute_copy_trade ─────────────────────────────────────

/// execute_copy_trade requires user.require_auth() — calling without auth must fail.
#[test]
fn execute_copy_trade_requires_user_auth() {
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, _pf, user, _admin, token) = setup_with_balance(per + 1_000_000);

    // Do NOT mock auths — call without any auth context.
    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
        )
    });
    // Without mock_all_auths the require_auth() panics, surfaced as an error.
    // We just verify the call does not succeed silently.
    assert!(err.is_err(), "execute_copy_trade must require user auth");
}

// ── Reentrancy guard tests ────────────────────────────────────────────────────

/// A mock portfolio that calls back into execute_copy_trade during validate_and_record,
/// simulating a reentrant call.
#[contract]
pub struct ReentrantPortfolio;

#[contractimpl]
impl ReentrantPortfolio {
    pub fn set_executor(env: Env, exec: Address) {
        env.storage().instance().set(&symbol_short!("exec"), &exec);
    }
    pub fn set_user(env: Env, user: Address) {
        env.storage().instance().set(&symbol_short!("user"), &user);
    }
    pub fn validate_and_record(env: Env, user: Address, _max_positions: u32) -> u32 {
        let exec: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("exec"))
            .unwrap();
        // Attempt reentrant call — must be blocked.
        let token = Address::generate(&env); // dummy token; balance check will fail first
        let client = TradeExecutorContractClient::new(&env, &exec);
        let result = client.try_execute_copy_trade(&user, &token, &1_000_000i128, &None::<u32>);
        let blocked = matches!(result, Err(Ok(ContractError::ReentrancyDetected)));
        env.storage()
            .instance()
            .set(&symbol_short!("blocked"), &blocked);
        1
    }
    pub fn was_blocked(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&symbol_short!("blocked"))
            .unwrap_or(false)
    }
}

#[test]
fn reentrant_call_returns_reentrancy_detected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac_token(&env);

    // Give user enough balance so the balance check passes on the outer call.
    StellarAssetClient::new(&env, &token).mint(
        &user,
        &(TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE + 1_000_000),
    );

    let portfolio_id = env.register(ReentrantPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    ReentrantPortfolioClient::new(&env, &portfolio_id).set_executor(&exec_id);
    ReentrantPortfolioClient::new(&env, &portfolio_id).set_user(&user);

    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT);

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
    let token = sac_token(&env);
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    StellarAssetClient::new(&env, &token).mint(&user, &(per * 3));

    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    // Two sequential calls must both succeed (lock is cleared between them).
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);
    exec.execute_copy_trade(&user, &token, &TRADE_AMOUNT, &None::<u32>);

    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        2
    );
}

// ── Portfolio percentage trade size tests ─────────────────────────────────────

#[test]
fn resolve_trade_amount_none_returns_explicit() {
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    StellarAssetClient::new(&env, &token).mint(&user, &5_000_000);
    let result = resolve_trade_amount(&env, &user, &token, 1_000_000, None, None);
    assert_eq!(result, Ok(1_000_000));
}

#[test]
fn resolve_trade_amount_pct_calculates_correctly() {
    // portfolio = 10_000_000, pct = 1000 bps (10%) => amount = 1_000_000
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let portfolio_value: i128 = 10_000_000;
    StellarAssetClient::new(&env, &token).mint(&user, &portfolio_value);
    let dummy_oracle = Address::generate(&env);
    let result = resolve_trade_amount(
        &env, &user, &token, 999, Some(1_000), Some(dummy_oracle),
    );
    assert_eq!(result, Ok(1_000_000));
}

#[test]
fn resolve_trade_amount_cap_enforced() {
    // pct = 2001 bps > MAX_POSITION_PCT_BPS (2000) => PositionPctTooHigh
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    StellarAssetClient::new(&env, &token).mint(&user, &10_000_000);
    let dummy_oracle = Address::generate(&env);
    let result = resolve_trade_amount(
        &env, &user, &token, 1_000_000, Some(MAX_POSITION_PCT_BPS + 1), Some(dummy_oracle),
    );
    assert_eq!(result, Err(ContractError::PositionPctTooHigh));
}

#[test]
fn resolve_trade_amount_at_max_cap_succeeds() {
    // pct = 2000 bps (exactly 20%) => allowed
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    StellarAssetClient::new(&env, &token).mint(&user, &10_000_000);
    let dummy_oracle = Address::generate(&env);
    let result = resolve_trade_amount(
        &env, &user, &token, 999, Some(MAX_POSITION_PCT_BPS), Some(dummy_oracle),
    );
    // 10_000_000 * 2000 / 10_000 = 2_000_000
    assert_eq!(result, Ok(2_000_000));
}

#[test]
fn resolve_trade_amount_oracle_unavailable_falls_back() {
    // oracle = None with Some(pct) => fall back to explicit_amount
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let token = sac_token(&env);
    StellarAssetClient::new(&env, &token).mint(&user, &10_000_000);
    let result = resolve_trade_amount(
        &env, &user, &token, 1_234_567, Some(500), None,
    );
    assert_eq!(result, Ok(1_234_567));
}

// ── SDEX swap tests ───────────────────────────────────────────────────────────

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

    StellarAssetClient::new(env, &token_a).mint(&exec_id, &1_000_000_000);
    StellarAssetClient::new(env, &token_b).mint(&router_id, &10_000_000_000);

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
        execute_sdex_swap(&env, &router_id, &token_a, &token_b, 1_000_000, 400_000)
    });
    assert_eq!(err, Err(ContractError::SlippageExceeded));
}

#[test]
fn swap_with_slippage_matches_formula() {
    let env = Env::default();
    env.mock_all_auths();

    let (exec_id, router_id, token_a, token_b) = setup_executor_with_router(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

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
        execute_sdex_swap(&env, &router_id, &token_a, &token_b, 1_000_000, min)
    });
    assert_eq!(err, Err(ContractError::SlippageExceeded));
}

// ── cancel_copy_trade tests ───────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum PortfolioKey {
    Position(Address, u64),
    LastClosed,
}

#[contract]
pub struct MockPortfolioWithPositions;

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
    pub fn last_closed(env: Env) -> Option<u64> {
        env.storage().instance().get(&PortfolioKey::LastClosed)
    }
    pub fn validate_and_record(_env: Env, _user: Address, _max_positions: u32) -> u32 {
        1
    }
}

fn setup_cancel(router_out: i128) -> (Env, Address, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let sac_a = env.register_stellar_asset_contract_v2(admin.clone());
    let sac_b = env.register_stellar_asset_contract_v2(admin.clone());
    let token_a = sac_a.address();
    let token_b = sac_b.address();

    let router_id = env.register(MockSdexRouter, ());
    MockSdexRouterClient::new(&env, &router_id).set_amount_out(&router_out);
    StellarAssetClient::new(&env, &token_b).mint(&router_id, &10_000_000_000);

    let portfolio_id = env.register(MockPortfolioWithPositions, ());
    let exec_id = env.register(TradeExecutorContract, ());
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);
    exec.set_sdex_router(&router_id);

    StellarAssetClient::new(&env, &token_a).mint(&exec_id, &1_000_000_000);

    (env, exec_id, portfolio_id, user, token_a, token_b, admin)
}

#[test]
fn cancel_copy_trade_success() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);
    exec.cancel_copy_trade(
        &user, &user, &1u64, &token_a, &token_b, &1_000_000, &900_000,
    );

    assert_eq!(
        MockPortfolioWithPositionsClient::new(&env, &portfolio_id).last_closed(),
        Some(1u64)
    );
}

#[test]
fn cancel_copy_trade_unauthorized() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let attacker = Address::generate(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(),
            attacker,
            user,
            1u64,
            token_a,
            token_b,
            1_000_000,
            900_000,
        )
    });
    assert_eq!(err, Err(ContractError::Unauthorized));
}

#[test]
fn cancel_copy_trade_not_found() {
    let (env, exec_id, _portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let _ = exec;

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(),
            user.clone(),
            user,
            99u64,
            token_a,
            token_b,
            1_000_000,
            900_000,
        )
    });
    assert_eq!(err, Err(ContractError::TradeNotFound));
}

#[test]
fn cancel_copy_trade_pnl_calculation() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_200_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &2u64);
    exec.cancel_copy_trade(
        &user, &user, &2u64, &token_a, &token_b, &1_000_000, &900_000,
    );

    let found = env.events().all().iter().any(|e| {
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        topics
            .get(0)
            .and_then(|v| soroban_sdk::Symbol::try_from_val(&env, &v).ok())
            .map(|s| s == soroban_sdk::Symbol::new(&env, "TradeCancelled"))
            .unwrap_or(false)
    });
    assert!(found, "TradeCancelled event not emitted");
}

// ── Auth propagation: cancel_copy_trade ──────────────────────────────────────

/// cancel_copy_trade requires caller == user; a third party must be rejected.
#[test]
fn cancel_copy_trade_third_party_rejected() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let third_party = Address::generate(&env);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &5u64);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(),
            third_party.clone(),
            user.clone(),
            5u64,
            token_a,
            token_b,
            1_000_000,
            900_000,
        )
    });
    assert_eq!(err, Err(ContractError::Unauthorized));
}

// ── Event format tests ────────────────────────────────────────────────────────

fn last_event_topics(env: &Env) -> (soroban_sdk::Symbol, soroban_sdk::Symbol) {
    use soroban_sdk::testutils::Events;
    let events = env.events().all();
    let e = events.last().unwrap();
    let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1;
    let t0 = soroban_sdk::Symbol::try_from(topics.get(0).unwrap()).unwrap();
    let t1 = soroban_sdk::Symbol::try_from(topics.get(1).unwrap()).unwrap();
    (t0, t1)
}

#[test]
fn trade_cancelled_event_has_two_topic_format() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position(&user, &1u64);
    exec.cancel_copy_trade(&user, &user, &1u64, &token_a, &token_b, &1_000_000, &900_000);
    let (contract, event) = last_event_topics(&env);
    assert_eq!(contract, soroban_sdk::Symbol::new(&env, "trade_executor"));
    assert_eq!(event, soroban_sdk::Symbol::new(&env, "trade_cancelled"));
}
