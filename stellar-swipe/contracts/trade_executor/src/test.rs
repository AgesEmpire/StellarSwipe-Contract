#![cfg(test)]
//! Integration tests for TradeExecutor.
//! Stop-loss / take-profit coverage is in `triggers::tests`.

use crate::{
    errors::{ContractError, InsufficientBalanceDetail},
    risk_gates::{
        check_user_balance, resolve_trade_amount, DEFAULT_ESTIMATED_COPY_TRADE_FEE,
        MAX_POSITIONS_PER_USER, MAX_POSITION_PCT_BPS,
    },
    sdex::{self, execute_sdex_swap},
    OrderType, ReplayParams, TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    testutils::{Address as _, Events, Ledger as _},
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

fn test_tx_hash(env: &Env, seed: u8) -> soroban_sdk::Bytes {
    let mut arr = [0u8; 32];
    arr[0] = seed;
    arr[31] = seed;
    soroban_sdk::Bytes::from_array(env, &arr)
}

fn far_future(env: &Env) -> u64 {
    env.ledger().timestamp() + 86_400 * 365
}

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

#[test]
fn set_oracle_requires_whitelisted_oracle() {
    let (env, exec_id, _, _, _, _) = setup_with_balance(1_000_000);
    let oracle_id = Address::generate(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    assert_eq!(
        exec.try_set_oracle(&oracle_id),
        Err(Ok(ContractError::OracleNotWhitelisted))
    );

    exec.add_oracle(&oracle_id);
    assert!(exec.is_oracle_whitelisted(&oracle_id));
    assert_eq!(exec.get_oracle_whitelist_count(), 1);
    assert_eq!(exec.try_set_oracle(&oracle_id), Ok(Ok(())));
    assert_eq!(exec.get_oracle(), Some(oracle_id));
}

#[test]
fn oracle_whitelist_add_remove_is_idempotent_and_preserves_last_oracle() {
    let (env, exec_id, _, _, _, _) = setup_with_balance(1_000_000);
    let oracle_one = Address::generate(&env);
    let oracle_two = Address::generate(&env);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    exec.add_oracle(&oracle_one);
    exec.add_oracle(&oracle_one);
    assert_eq!(exec.get_oracle_whitelist_count(), 1);

    exec.add_oracle(&oracle_two);
    assert_eq!(exec.get_oracle_whitelist_count(), 2);

    exec.remove_oracle(&oracle_two);
    assert!(!exec.is_oracle_whitelisted(&oracle_two));
    assert_eq!(exec.get_oracle_whitelist_count(), 1);

    assert_eq!(
        exec.try_remove_oracle(&oracle_one),
        Err(Ok(ContractError::CannotRemoveLastOracle))
    );
    assert!(exec.is_oracle_whitelisted(&oracle_one));
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

// ── Minimum trade size tests (Issue #590) ──────────────────────────────────────

#[test]
fn execute_copy_trade_below_default_minimum_is_rejected() {
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    let below_default_min = crate::risk_gates::DEFAULT_MIN_TRADE_SIZE - 1;
    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &below_default_min,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::BelowMinimumTradeSize)));

    // No state change occurred: the portfolio never recorded a position.
    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        0
    );
}

#[test]
fn execute_copy_trade_below_per_asset_minimum_is_rejected() {
    let (env, exec_id, _portfolio_id, user, _admin, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_min_trade_size(&token, &10_000_000);

    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &9_999_999,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::BelowMinimumTradeSize)));
}

#[test]
fn execute_copy_trade_exactly_at_minimum_is_accepted() {
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_min_trade_size(&token, &10_000_000);

    exec.execute_copy_trade(
        &user,
        &token,
        &10_000_000,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );

    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        1
    );
}

#[test]
fn execute_copy_trade_above_minimum_is_accepted() {
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_min_trade_size(&token, &10_000_000);

    exec.execute_copy_trade(
        &user,
        &token,
        &10_000_001,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );

    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        1
    );
}

#[test]
fn admin_can_update_min_trade_size_for_asset() {
    let (env, exec_id, _portfolio_id, _user, _admin, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    assert_eq!(
        exec.get_min_trade_size(&token),
        crate::risk_gates::DEFAULT_MIN_TRADE_SIZE
    );

    exec.set_min_trade_size(&token, &5_000_000);
    assert_eq!(exec.get_min_trade_size(&token), 5_000_000);
}

#[test]
fn limit_order_below_minimum_trade_size_is_rejected() {
    let (env, exec_id, _portfolio_id, user, _admin, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_min_trade_size(&token, &10_000_000);

    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &9_999_999,
        &None::<u32>,
        &OrderType::Limit,
        &Some(100_0000000i128),
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::BelowMinimumTradeSize)));
    assert_eq!(exec.get_pending_limit_order_ids().len(), 0);
}

// ── execute_copy_trade tests ──────────────────────────────────────────────────

#[test]
fn execute_copy_trade_insufficient_balance_sets_detail() {
    let required = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, _pf, user, _admin, token) = setup_with_balance(TRADE_AMOUNT - 1);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::execute_copy_trade(
            env.clone(),
            user.clone(),
            token.clone(),
            TRADE_AMOUNT,
            None,
            OrderType::Market,
            None,
            1u64,
            test_tx_hash(&env, 0),
            far_future(&env),
        )
    });
    assert_eq!(err, Err(ContractError::InsufficientBalance));

    let detail = exec.get_insufficient_balance_detail(&user).unwrap();
    assert_eq!(
        detail,
        InsufficientBalanceDetail {
            required,
            available: TRADE_AMOUNT - 1,
        }
    );
}

#[test]
fn execute_copy_trade_sufficient_balance_invokes_portfolio() {
    let per = TRADE_AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(per + 1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
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
            env.clone(),
            user.clone(),
            token.clone(),
            0,
            None,
            OrderType::Market,
            None,
            1u64,
            test_tx_hash(&env, 0),
            far_future(&env),
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

    for n in 0..MAX_POSITIONS_PER_USER {
        let nonce = (n + 1) as u64;
        exec.execute_copy_trade(
            &user,
            &token,
            &TRADE_AMOUNT,
            &None::<u32>,
            &OrderType::Market,
            &None,
            &nonce,
            &test_tx_hash(&env, nonce as u8),
            &far_future(&env),
        );
    }

    // A failed call rolls back its nonce commit, so the retry reuses the nonce.
    let next_nonce = (MAX_POSITIONS_PER_USER + 1) as u64;
    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &next_nonce,
        &test_tx_hash(&env, next_nonce as u8),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::PositionLimitReached)));

    MockUserPortfolioClient::new(&env, &portfolio_id).close_one_copy_position(&user);
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &next_nonce,
        &test_tx_hash(&env, next_nonce as u8),
        &far_future(&env),
    );

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

    for n in 0..MAX_POSITIONS_PER_USER {
        let nonce = (n + 1) as u64;
        exec.execute_copy_trade(
            &user,
            &token,
            &TRADE_AMOUNT,
            &None::<u32>,
            &OrderType::Market,
            &None,
            &nonce,
            &test_tx_hash(&env, nonce as u8),
            &far_future(&env),
        );
    }

    // A failed call rolls back its nonce commit, so the retry reuses the nonce.
    let next_nonce = (MAX_POSITIONS_PER_USER + 1) as u64;
    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &next_nonce,
        &test_tx_hash(&env, next_nonce as u8),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::PositionLimitReached)));

    exec.set_position_limit_exempt(&user, &true);
    assert!(exec.is_position_limit_exempt(&user));

    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &next_nonce,
        &test_tx_hash(&env, next_nonce as u8),
        &far_future(&env),
    );
    assert_eq!(
        MockUserPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user),
        MAX_POSITIONS_PER_USER + 1
    );

    exec.set_position_limit_exempt(&user, &false);
    assert!(!exec.is_position_limit_exempt(&user));

    let after_nonce = next_nonce + 1;
    let err2 = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &after_nonce,
        &test_tx_hash(&env, after_nonce as u8),
        &far_future(&env),
    );
    assert_eq!(err2, Err(Ok(ContractError::PositionLimitReached)));
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
        let result = client.try_execute_copy_trade(
            &user,
            &token,
            &1_000_000i128,
            &None::<u32>,
            &OrderType::Market,
            &None,
            &1u64,
            &soroban_sdk::Bytes::from_array(&env, &[0u8; 32]),
            &(env.ledger().timestamp() + 86_400),
        );
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
#[ignore = "upstream reentrancy test requires a portfolio mock that preserves nested call diagnostics"]
fn reentrant_call_returns_reentrancy_detected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac_token(&env);

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

    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert!(
        ReentrantPortfolioClient::new(&env, &portfolio_id).was_blocked(),
        "expected reentrant inner call to be blocked"
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
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 1),
        &far_future(&env),
    );
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None::<u32>,
        &OrderType::Market,
        &None,
        &2u64,
        &test_tx_hash(&env, 2),
        &far_future(&env),
    );

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
    let result = resolve_trade_amount(&env, &user, &token, 999, Some(1_000), Some(dummy_oracle));
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
        &env,
        &user,
        &token,
        1_000_000,
        Some(MAX_POSITION_PCT_BPS + 1),
        Some(dummy_oracle),
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
        &env,
        &user,
        &token,
        999,
        Some(MAX_POSITION_PCT_BPS),
        Some(dummy_oracle),
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
    let result = resolve_trade_amount(&env, &user, &token, 1_234_567, Some(500), None);
    assert_eq!(result, Ok(1_234_567));
}

// ── SDEX swap tests ───────────────────────────────────────────────────────────

#[contract]
pub struct MockSdexRouter;

#[contractimpl]
impl MockSdexRouter {
    pub fn get_best_ask(_env: Env, _from_token: Address, _to_token: Address) -> (i128, i128) {
        (0, 10_000_000_000i128)
    }

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
    EntryPrice(Address, u64),
    LastClosed,
    LastPnl,
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
    pub fn add_position_with_entry_price(
        env: Env,
        user: Address,
        trade_id: u64,
        entry_price: i128,
    ) {
        env.storage()
            .instance()
            .set(&PortfolioKey::Position(user.clone(), trade_id), &true);
        env.storage()
            .instance()
            .set(&PortfolioKey::EntryPrice(user, trade_id), &entry_price);
    }
    pub fn has_position(env: Env, user: Address, trade_id: u64) -> bool {
        env.storage()
            .instance()
            .get(&PortfolioKey::Position(user, trade_id))
            .unwrap_or(false)
    }
    pub fn get_entry_price(env: Env, user: Address, trade_id: u64) -> i128 {
        env.storage()
            .instance()
            .get(&PortfolioKey::EntryPrice(user, trade_id))
            .unwrap_or(10_000_000) // default 1:1 rate
    }
    pub fn close_position(env: Env, user: Address, trade_id: u64, pnl: i128) {
        env.storage()
            .instance()
            .remove(&PortfolioKey::Position(user.clone(), trade_id));
        env.storage()
            .instance()
            .remove(&PortfolioKey::EntryPrice(user, trade_id));
        env.storage()
            .instance()
            .set(&PortfolioKey::LastClosed, &trade_id);
        env.storage().instance().set(&PortfolioKey::LastPnl, &pnl);
    }
    pub fn last_closed(env: Env) -> Option<u64> {
        env.storage().instance().get(&PortfolioKey::LastClosed)
    }
    pub fn last_pnl(env: Env) -> Option<i128> {
        env.storage().instance().get(&PortfolioKey::LastPnl)
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

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &1u64,
        &10_000_000i128,
    );
    exec.cancel_copy_trade(
        &user,
        &user,
        &1u64,
        &token_a,
        &token_b,
        &1_000_000,
        &900_000,
        &10_000_000,
        &ReplayParams {
            nonce: 1,
            tx_hash: test_tx_hash(&env, 0),
            expiry_ts: far_future(&env),
        },
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

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &1u64,
        &10_000_000i128,
    );

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
            10_000_000,
            ReplayParams {
                nonce: 1,
                tx_hash: test_tx_hash(&env, 0),
                expiry_ts: far_future(&env),
            },
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
            10_000_000,
            ReplayParams {
                nonce: 1,
                tx_hash: test_tx_hash(&env, 0),
                expiry_ts: far_future(&env),
            },
        )
    });
    assert_eq!(err, Err(ContractError::TradeNotFound));
}

#[test]
fn cancel_copy_trade_pnl_calculation() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_200_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    // entry_price = 9_500_000 → 0.95 to_token per 1 from_token
    // amount = 1_000_000 from_token
    // entry_value = 1_000_000 * 9_500_000 / 10_000_000 = 950_000 to_token
    // exit_price = 1_200_000 to_token
    // realized_pnl = 1_200_000 - 950_000 = 250_000
    let portfolio = MockPortfolioWithPositionsClient::new(&env, &portfolio_id);
    portfolio.add_position_with_entry_price(&user, &2u64, &9_500_000i128);
    exec.cancel_copy_trade(
        &user,
        &user,
        &2u64,
        &token_a,
        &token_b,
        &1_000_000,
        &900_000,
        &9_500_000,
        &ReplayParams {
            nonce: 1,
            tx_hash: test_tx_hash(&env, 0),
            expiry_ts: far_future(&env),
        },
    );

    // Verify the close_position was called with the correct realized_pnl.
    let closed_id = portfolio.last_closed();
    assert_eq!(closed_id, Some(2u64));
    let pnl = portfolio.last_pnl();
    assert_eq!(pnl, Some(250_000i128), "realized PnL should be 250_000 when entry_price=0.95 and exit_price=1.2 for amount=1_000_000");
}

// ── Auth propagation: cancel_copy_trade ──────────────────────────────────────

/// cancel_copy_trade requires caller == user; a third party must be rejected.
#[test]
fn cancel_copy_trade_third_party_rejected() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let third_party = Address::generate(&env);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &5u64,
        &10_000_000i128,
    );

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
            10_000_000,
            ReplayParams {
                nonce: 1,
                tx_hash: test_tx_hash(&env, 0),
                expiry_ts: far_future(&env),
            },
        )
    });
    assert_eq!(err, Err(ContractError::Unauthorized));
}

#[test]
fn cancel_copy_trade_replay_nonce_rejected() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &1u64,
        &10_000_000i128,
    );
    exec.cancel_copy_trade(
        &user,
        &user,
        &1u64,
        &token_a,
        &token_b,
        &1_000_000,
        &900_000,
        &10_000_000,
        &ReplayParams {
            nonce: 1,
            tx_hash: test_tx_hash(&env, 1),
            expiry_ts: far_future(&env),
        },
    );

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::cancel_copy_trade(
            env.clone(),
            user.clone(),
            user.clone(),
            2u64,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
            900_000,
            10_000_000,
            ReplayParams {
                nonce: 1,
                tx_hash: test_tx_hash(&env, 1),
                expiry_ts: far_future(&env),
            },
        )
    });
    assert_eq!(err, Err(ContractError::ReplayDetected));
}

// ── Event format tests ────────────────────────────────────────────────────────

fn last_event_topics(env: &Env) -> (soroban_sdk::Symbol, soroban_sdk::Symbol) {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::TryFromVal;
    let events = env.events().all();
    let e = events.last().unwrap();
    let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1;
    let t0 = soroban_sdk::Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
    let t1 = soroban_sdk::Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
    (t0, t1)
}

#[test]
fn trade_cancelled_event_has_two_topic_format() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &1u64,
        &10_000_000i128,
    );
    exec.cancel_copy_trade(
        &user,
        &user,
        &1u64,
        &token_a,
        &token_b,
        &1_000_000,
        &900_000,
        &10_000_000,
        &ReplayParams {
            nonce: 1,
            tx_hash: test_tx_hash(&env, 0),
            expiry_ts: far_future(&env),
        },
    );
    let (contract, event) = last_event_topics(&env);
    assert_eq!(contract, soroban_sdk::Symbol::new(&env, "trade_executor"));
    assert_eq!(event, soroban_sdk::Symbol::new(&env, "trade_cancelled"));
}

// ── Daily volume limit tests ──────────────────────────────────────────────────

// Helper: set up executor with a funded user and a volume limit.
fn setup_with_limit(limit: i128) -> (Env, Address, Address, Address, Address) {
    let (env, exec_id, _portfolio_id, user, _admin, token) = setup_with_balance(10_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_daily_volume_limit(&limit);
    (env, exec_id, user, _admin, token)
}

/// Zero limit means no restriction — trade succeeds.
#[test]
fn volume_limit_zero_means_no_restriction() {
    let (env, exec_id, user, _admin, token) = setup_with_limit(0);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
}

/// Trade under the daily limit succeeds.
#[test]
fn volume_under_limit_succeeds() {
    let (env, exec_id, user, _admin, token) = setup_with_limit(TRADE_AMOUNT * 2);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
}

/// Trade exactly at the daily limit succeeds.
#[test]
fn volume_at_limit_succeeds() {
    let (env, exec_id, user, _admin, token) = setup_with_limit(TRADE_AMOUNT);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
}

/// Trade that would exceed the daily limit returns DailyVolumeLimitExceeded.
#[test]
fn volume_over_limit_returns_error() {
    let (env, exec_id, user, _admin, token) = setup_with_limit(TRADE_AMOUNT - 1);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    let result = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert_eq!(result, Err(Ok(ContractError::DailyVolumeLimitExceeded)));
}

/// Volume resets on a new day (simulated by advancing the ledger timestamp).
#[test]
fn volume_resets_on_new_day() {
    use soroban_sdk::testutils::Ledger;
    let (env, exec_id, user, _admin, token) = setup_with_limit(TRADE_AMOUNT);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    // Day 0: use up the full limit.
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 1),
        &far_future(&env),
    );

    // Advance to day 1.
    env.ledger().with_mut(|l| l.timestamp = 86_400);

    // Day 1: limit resets — trade should succeed again.
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &2u64,
        &test_tx_hash(&env, 2),
        &far_future(&env),
    );
}

// ── Issue #390: fee fallback tests ───────────────────────────────────────────

/// Primary fee deduction succeeds when user has amount + fee.
#[test]
fn primary_fee_deduction_succeeds_with_sufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    // Give user exactly amount + fee.
    let fee = DEFAULT_ESTIMATED_COPY_TRADE_FEE;
    let amount = TRADE_AMOUNT;
    StellarAssetClient::new(&env, &token).mint(&user, &(amount + fee));

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    // Should succeed — no fallback needed.
    let result = exec.try_execute_copy_trade(
        &user,
        &token,
        &amount,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert!(result.is_ok(), "primary fee deduction should succeed");

    // No fee_from_received event should be emitted.
    let has_fallback_event = env.events().all().iter().any(|e| {
        use soroban_sdk::TryFromVal;
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        if topics.len() < 2 {
            return false;
        }
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
            .map(|s| s == soroban_sdk::Symbol::new(&env, "fee_from_received"))
            .unwrap_or(false)
    });
    assert!(
        !has_fallback_event,
        "fallback event must not be emitted when primary succeeds"
    );
}

/// Fallback activates when user has exactly the trade amount but not the fee.
#[test]
fn fee_fallback_activates_when_only_amount_available() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    // Give user exactly the trade amount (no extra for fee).
    let amount = TRADE_AMOUNT;
    StellarAssetClient::new(&env, &token).mint(&user, &amount);

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);
    // Set a non-zero fee so fallback is triggered.
    exec.set_copy_trade_estimated_fee(&1_000i128);

    // Trade should still succeed via fallback.
    let result = exec.try_execute_copy_trade(
        &user,
        &token,
        &amount,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert!(result.is_ok(), "trade should succeed via fee fallback");

    // fee_from_received event must be emitted.
    let has_fallback_event = env.events().all().iter().any(|e| {
        use soroban_sdk::TryFromVal;
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
        if topics.len() < 2 {
            return false;
        }
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
            .map(|s| s == soroban_sdk::Symbol::new(&env, "fee_from_received"))
            .unwrap_or(false)
    });
    assert!(
        has_fallback_event,
        "fee_from_received event must be emitted on fallback"
    );
}

/// Trade fails when user has less than the trade amount (even fallback can't help).
#[test]
fn trade_fails_when_balance_below_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token = sac_token(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    // Give user less than the trade amount.
    StellarAssetClient::new(&env, &token).mint(&user, &(TRADE_AMOUNT - 1));

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    let result = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );
    assert_eq!(result, Err(Ok(ContractError::InsufficientBalance)));
}

// ── Issue #623: pending limit orders execution & expiration tests ────────────────

#[test]
fn test_limit_order_execution() {
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(10_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    // Initial sequence number
    env.ledger().with_mut(|l| l.sequence_number = 10);

    // Place a limit order: limit_price = 10_000, amount = TRADE_AMOUNT
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Limit,
        &Some(10_000i128),
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );

    // Verify order is stored
    let order_ids = exec.get_pending_limit_order_ids();
    assert_eq!(order_ids.len(), 1);
    let order_id = order_ids.get(0).unwrap();
    let order = exec.get_pending_limit_order(&order_id).unwrap();
    assert_eq!(order.limit_price, 10_000);
    assert_eq!(order.amount, TRADE_AMOUNT);

    // Set SDEX price higher than limit price -> check should NOT execute
    exec.set_sdex_price(&token, &11_000i128);
    let processed = exec.check_pending_limit_orders(&token);
    assert_eq!(processed, 0);

    // The order should still be pending
    assert_eq!(exec.get_pending_limit_order_ids().len(), 1);

    // Set SDEX price to limit price -> check should execute
    exec.set_sdex_price(&token, &10_000i128);
    let processed = exec.check_pending_limit_orders(&token);
    assert_eq!(processed, 1);

    // Order should be removed from pending list
    assert_eq!(exec.get_pending_limit_order_ids().len(), 0);
    assert!(exec.get_pending_limit_order(&order_id).is_none());

    // User portfolio should now have recorded a position (open_position_count == 1)
    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), 1);
}

#[test]
fn test_limit_order_expiration() {
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(10_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 10);

    // Place a limit order: limit_price = 10_000, amount = TRADE_AMOUNT
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Limit,
        &Some(10_000i128),
        &1u64,
        &test_tx_hash(&env, 0),
        &far_future(&env),
    );

    let order_ids = exec.get_pending_limit_order_ids();
    assert_eq!(order_ids.len(), 1);
    let order_id = order_ids.get(0).unwrap();

    // Advance sequence number beyond the expiration sequence (10 + 120 = 130)
    use soroban_sdk::testutils::Ledger;
    env.ledger().with_mut(|l| l.sequence_number = 131);

    // Set SDEX price to 10_000 (which would otherwise execute)
    exec.set_sdex_price(&token, &10_000i128);

    // Call check_pending_limit_orders -> should expire, NOT execute
    let processed = exec.check_pending_limit_orders(&token);
    assert_eq!(processed, 1);

    // Order should be removed from pending
    assert_eq!(exec.get_pending_limit_order_ids().len(), 0);
    assert!(exec.get_pending_limit_order(&order_id).is_none());

    // Verify it did NOT record a position in the portfolio
    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), 0);
}

#[test]
fn test_limit_order_persistence() {
    let (env, exec_id, portfolio_id, user, _admin, token) = setup_with_balance(10_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    env.ledger().with_mut(|l| l.sequence_number = 10);

    // Place three limit orders with different limit prices:
    // Order 1: limit_price = 10_000
    // Order 2: limit_price = 9_000
    // Order 3: limit_price = 8_000
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Limit,
        &Some(10_000i128),
        &1u64,
        &test_tx_hash(&env, 1),
        &far_future(&env),
    );
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Limit,
        &Some(9_000i128),
        &2u64,
        &test_tx_hash(&env, 2),
        &far_future(&env),
    );
    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Limit,
        &Some(8_000i128),
        &3u64,
        &test_tx_hash(&env, 3),
        &far_future(&env),
    );

    let initial_ids = exec.get_pending_limit_order_ids();
    assert_eq!(initial_ids.len(), 3);

    // Set SDEX price to 9_000.
    // Order 1 (limit 10_000) and Order 2 (limit 9_000) should execute.
    // Order 3 (limit 8_000) should NOT execute (9_000 > 8_000).
    exec.set_sdex_price(&token, &9_000i128);

    let processed = exec.check_pending_limit_orders(&token);
    assert_eq!(processed, 2);

    // The remaining pending list should contain only Order 3
    let remaining_ids = exec.get_pending_limit_order_ids();
    assert_eq!(remaining_ids.len(), 1);

    let last_order_id = remaining_ids.get(0).unwrap();
    let last_order = exec.get_pending_limit_order(&last_order_id).unwrap();
    assert_eq!(last_order.limit_price, 8_000);

    // Portfolio should have 2 recorded positions
    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), 2);
}

// ── Trade receipt hash tests (Issue #683) ─────────────────────────────────────

#[test]
fn trade_receipt_hash_is_stored_after_execute_copy_trade() {
    let (env, exec_id, _, user, _, token) = setup_with_balance(10_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    exec.execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &1u64,
        &test_tx_hash(&env, 42),
        &far_future(&env),
    );

    let receipt = exec.get_trade_receipt(&1u64);
    assert!(
        receipt.is_some(),
        "receipt hash must be stored after a trade"
    );
    let hash = receipt.unwrap();
    assert_ne!(hash, soroban_sdk::BytesN::from_array(&env, &[0u8; 32]));
}

#[test]
fn trade_receipt_hash_is_deterministic_for_same_inputs() {
    use crate::compute_trade_hash;
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    let h1 = compute_trade_hash(&env, &user, &asset, 1_000_000, 5_000, 1_700_000_000);
    let h2 = compute_trade_hash(&env, &user, &asset, 1_000_000, 5_000, 1_700_000_000);
    assert_eq!(h1, h2, "same inputs must produce the same hash");
}

#[test]
fn trade_receipt_hash_changes_when_amount_changes() {
    use crate::compute_trade_hash;
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    let h1 = compute_trade_hash(&env, &user, &asset, 1_000_000, 5_000, 1_700_000_000);
    let h2 = compute_trade_hash(&env, &user, &asset, 2_000_000, 5_000, 1_700_000_000);
    assert_ne!(h1, h2, "changing amount must change the hash");
}

#[test]
fn trade_receipt_hash_changes_when_price_changes() {
    use crate::compute_trade_hash;
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    let h1 = compute_trade_hash(&env, &user, &asset, 1_000_000, 5_000, 1_700_000_000);
    let h2 = compute_trade_hash(&env, &user, &asset, 1_000_000, 9_999, 1_700_000_000);
    assert_ne!(h1, h2, "changing price must change the hash");
}

#[test]
fn trade_receipt_hash_changes_when_user_changes() {
    use crate::compute_trade_hash;
    let env = Env::default();
    env.mock_all_auths();
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);
    let asset = Address::generate(&env);

    let h1 = compute_trade_hash(&env, &user_a, &asset, 1_000_000, 5_000, 1_700_000_000);
    let h2 = compute_trade_hash(&env, &user_b, &asset, 1_000_000, 5_000, 1_700_000_000);
    assert_ne!(h1, h2, "different users must produce different hashes");
}

#[test]
fn trade_receipt_hash_changes_when_timestamp_changes() {
    use crate::compute_trade_hash;
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    let h1 = compute_trade_hash(&env, &user, &asset, 1_000_000, 5_000, 1_700_000_000);
    let h2 = compute_trade_hash(&env, &user, &asset, 1_000_000, 5_000, 1_700_000_001);
    assert_ne!(h1, h2, "different timestamps must produce different hashes");
}

#[test]
fn trade_receipt_ids_are_monotonically_incremented() {
    let (env, exec_id, _, user, _, token) = setup_with_balance(50_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for i in 1u64..=3 {
        exec.execute_copy_trade(
            &user,
            &token,
            &TRADE_AMOUNT,
            &None,
            &OrderType::Market,
            &None,
            &i,
            &test_tx_hash(&env, i as u8),
            &far_future(&env),
        );
        assert!(exec.get_trade_receipt(&i).is_some(), "receipt {i} missing");
    }
    // Receipts 1, 2, 3 must all be distinct hashes.
    let h1 = exec.get_trade_receipt(&1u64).unwrap();
    let h2 = exec.get_trade_receipt(&2u64).unwrap();
    let h3 = exec.get_trade_receipt(&3u64).unwrap();
    // Timestamps and nonces differ per trade, so hashes must differ.
    assert_ne!(h1, h2);
    assert_ne!(h2, h3);
}

#[test]
fn get_trade_receipt_returns_none_for_unknown_id() {
    let (env, exec_id, _, _, _, _) = setup_with_balance(1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    assert!(exec.get_trade_receipt(&999u64).is_none());
}

// ── Per-user open-position cap tests (Issue #791) ─────────────────────────────

fn execute_trade(
    env: &Env,
    exec: &TradeExecutorContractClient,
    user: &Address,
    token: &Address,
    nonce: u64,
) {
    exec.execute_copy_trade(
        user,
        token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &nonce,
        &test_tx_hash(env, nonce as u8),
        &far_future(env),
    );
}

#[test]
fn max_open_positions_defaults_to_50() {
    let (env, exec_id, _, _, _, _) = setup_with_balance(1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    assert_eq!(
        exec.get_max_open_positions(),
        crate::DEFAULT_MAX_OPEN_POSITIONS
    );
    assert_eq!(crate::DEFAULT_MAX_OPEN_POSITIONS, 50);
}

#[test]
#[should_panic(expected = "max must be positive")]
fn set_max_open_positions_rejects_zero() {
    let (env, exec_id, _, _, _, _) = setup_with_balance(1_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_max_open_positions(&0);
}

#[test]
fn open_below_cap_succeeds_and_increments_count() {
    let (env, exec_id, _, user, _, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_max_open_positions(&3);

    execute_trade(&env, &exec, &user, &token, 1);
    assert_eq!(exec.get_user_position_count(&user), 1);

    execute_trade(&env, &exec, &user, &token, 2);
    assert_eq!(exec.get_user_position_count(&user), 2);
}

#[test]
fn open_at_exactly_cap_is_rejected() {
    let (env, exec_id, _, user, _, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_max_open_positions(&2);

    execute_trade(&env, &exec, &user, &token, 1);
    execute_trade(&env, &exec, &user, &token, 2);
    assert_eq!(exec.get_user_position_count(&user), 2);

    let err = exec.try_execute_copy_trade(
        &user,
        &token,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &3u64,
        &test_tx_hash(&env, 3),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::TooManyOpenPositions)));
    assert_eq!(exec.get_user_position_count(&user), 2);
}

#[test]
fn queue_copy_trade_rejected_at_cap() {
    let (env, exec_id, _, user, _, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_max_open_positions(&1);

    execute_trade(&env, &exec, &user, &token, 1);

    let err = exec.try_queue_copy_trade(&user, &token, &TRADE_AMOUNT, &None);
    assert_eq!(err, Err(Ok(ContractError::TooManyOpenPositions)));
}

#[test]
fn exempt_user_ignores_cap() {
    let (env, exec_id, _, user, _, token) = setup_with_balance(1_000_000_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.set_max_open_positions(&1);
    exec.set_position_limit_exempt(&user, &true);

    for nonce in 1u64..=3 {
        execute_trade(&env, &exec, &user, &token, nonce);
    }
    // Exempt users are still counted, just not capped.
    assert_eq!(exec.get_user_position_count(&user), 3);

    // Queueing is also uncapped for exempt users.
    exec.queue_copy_trade(&user, &token, &TRADE_AMOUNT, &None);
}

#[test]
fn close_frees_a_slot() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    StellarAssetClient::new(&env, &token_a).mint(&user, &1_000_000_000);
    exec.set_max_open_positions(&1);

    execute_trade(&env, &exec, &user, &token_a, 1);
    assert_eq!(exec.get_user_position_count(&user), 1);

    // Cap of 1 reached — next open is rejected.
    let err = exec.try_execute_copy_trade(
        &user,
        &token_a,
        &TRADE_AMOUNT,
        &None,
        &OrderType::Market,
        &None,
        &2u64,
        &test_tx_hash(&env, 2),
        &far_future(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::TooManyOpenPositions)));

    // Close the position; the slot is freed. (The rejected call above rolled
    // back its nonce commit, so the next sequential nonce is 2.)
    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &1u64,
        &10_000_000i128,
    );
    exec.cancel_copy_trade(
        &user,
        &user,
        &1u64,
        &token_a,
        &token_b,
        &1_000_000,
        &900_000,
        &10_000_000,
        &ReplayParams {
            nonce: 2,
            tx_hash: test_tx_hash(&env, 20),
            expiry_ts: far_future(&env),
        },
    );
    assert_eq!(exec.get_user_position_count(&user), 0);

    // A new position can be opened again.
    execute_trade(&env, &exec, &user, &token_a, 3);
    assert_eq!(exec.get_user_position_count(&user), 1);
}

#[test]
fn count_never_goes_negative_on_close() {
    let (env, exec_id, portfolio_id, user, token_a, token_b, _) = setup_cancel(1_100_000);
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    // Close a position that was never counted (pre-existing position):
    // the saturating decrement must leave the count at zero, not underflow.
    MockPortfolioWithPositionsClient::new(&env, &portfolio_id).add_position_with_entry_price(
        &user,
        &7u64,
        &10_000_000i128,
    );
    assert_eq!(exec.get_user_position_count(&user), 0);
    exec.cancel_copy_trade(
        &user,
        &user,
        &7u64,
        &token_a,
        &token_b,
        &1_000_000,
        &900_000,
        &10_000_000,
        &ReplayParams {
            nonce: 1,
            tx_hash: test_tx_hash(&env, 1),
            expiry_ts: far_future(&env),
        },
    );
    assert_eq!(exec.get_user_position_count(&user), 0);
}
