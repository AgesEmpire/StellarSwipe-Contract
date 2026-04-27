#![cfg(test)]
//! Unit tests for `batch_execute` mixed success/failure scenarios (#258).
//!
//! Verifies that:
//! - Successful trades in a batch are NOT rolled back by failed trades.
//! - The result array accurately reflects each trade's outcome.
//! - The batch size limit is enforced.

use crate::{
    errors::ContractError,
    risk_gates::{DEFAULT_ESTIMATED_COPY_TRADE_FEE, MAX_BATCH_SIZE},
    BatchTradeInput, BatchTradeResult, TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, token::StellarAssetClient,
    Address, Env, Vec,
};

// ── Mock UserPortfolio ────────────────────────────────────────────────────────

#[contract]
pub struct MockPortfolio;

#[contracttype]
#[derive(Clone)]
enum PortfolioKey {
    Count(Address),
}

#[contractimpl]
impl MockPortfolio {
    pub fn validate_and_record(env: Env, user: Address, max_positions: u32) -> u32 {
        let key = PortfolioKey::Count(user.clone());
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
            .get(&PortfolioKey::Count(user))
            .unwrap_or(0)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const AMOUNT: i128 = 1_000_000;

fn sac(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

/// Set up executor + portfolio. Returns `(env, exec_id, portfolio_id)`.
fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let portfolio_id = env.register(MockPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    (env, exec_id, portfolio_id)
}

/// Mint enough tokens for `n` trades (amount + fee each).
fn funded_user(env: &Env, token: &Address, n: i128) -> Address {
    let user = Address::generate(env);
    StellarAssetClient::new(env, token)
        .mint(&user, &(n * (AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE)));
    user
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Batch of 5: trades 1, 3, 5 succeed; trades 2, 4 fail (different error reasons).
/// - Trade 2 fails: InvalidAmount (amount = 0).
/// - Trade 4 fails: InsufficientBalance (user has no tokens).
#[test]
fn batch_mixed_success_failure() {
    let (env, exec_id, portfolio_id) = setup();
    let token = sac(&env);

    // Users for succeeding trades (1, 3, 5) — each funded for 1 trade.
    let user1 = funded_user(&env, &token, 1);
    let user3 = funded_user(&env, &token, 1);
    let user5 = funded_user(&env, &token, 1);

    // User for trade 2: zero amount → InvalidAmount.
    let user2 = Address::generate(&env);

    // User for trade 4: no balance → InsufficientBalance.
    let user4 = Address::generate(&env);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    trades.push_back(BatchTradeInput {
        user: user1.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });
    trades.push_back(BatchTradeInput {
        user: user2.clone(),
        token: token.clone(),
        amount: 0,
    });
    trades.push_back(BatchTradeInput {
        user: user3.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });
    trades.push_back(BatchTradeInput {
        user: user4.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });
    trades.push_back(BatchTradeInput {
        user: user5.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });

    let results = env
        .as_contract(&exec_id, || {
            TradeExecutorContract::batch_execute(env.clone(), trades)
        })
        .unwrap();

    assert_eq!(results.len(), 5);

    // Trade 1 succeeds.
    assert_eq!(
        results.get(0).unwrap(),
        BatchTradeResult {
            ok: true,
            error_code: 0
        }
    );
    // Trade 2 fails: InvalidAmount.
    assert_eq!(
        results.get(1).unwrap(),
        BatchTradeResult {
            ok: false,
            error_code: ContractError::InvalidAmount as u32
        }
    );
    // Trade 3 succeeds.
    assert_eq!(
        results.get(2).unwrap(),
        BatchTradeResult {
            ok: true,
            error_code: 0
        }
    );
    // Trade 4 fails: InsufficientBalance.
    assert_eq!(
        results.get(3).unwrap(),
        BatchTradeResult {
            ok: false,
            error_code: ContractError::InsufficientBalance as u32
        }
    );
    // Trade 5 succeeds.
    assert_eq!(
        results.get(4).unwrap(),
        BatchTradeResult {
            ok: true,
            error_code: 0
        }
    );

    let pf = MockPortfolioClient::new(&env, &portfolio_id);

    // Positions opened for trades 1, 3, 5.
    assert_eq!(
        pf.get_open_position_count(&user1),
        1,
        "trade 1 must have opened a position"
    );
    assert_eq!(
        pf.get_open_position_count(&user3),
        1,
        "trade 3 must have opened a position"
    );
    assert_eq!(
        pf.get_open_position_count(&user5),
        1,
        "trade 5 must have opened a position"
    );

    // No positions opened for trades 2, 4.
    assert_eq!(
        pf.get_open_position_count(&user2),
        0,
        "trade 2 must NOT have opened a position"
    );
    assert_eq!(
        pf.get_open_position_count(&user4),
        0,
        "trade 4 must NOT have opened a position"
    );
}

/// Successful trades are not rolled back when later trades in the same batch fail.
#[test]
fn successful_trades_not_rolled_back_by_later_failures() {
    let (env, exec_id, portfolio_id) = setup();
    let token = sac(&env);

    let user_ok = funded_user(&env, &token, 1);
    let user_fail = Address::generate(&env); // no balance

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    trades.push_back(BatchTradeInput {
        user: user_ok.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });
    trades.push_back(BatchTradeInput {
        user: user_fail.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });

    let results = env
        .as_contract(&exec_id, || {
            TradeExecutorContract::batch_execute(env.clone(), trades)
        })
        .unwrap();

    assert!(results.get(0).unwrap().ok, "first trade must succeed");
    assert!(!results.get(1).unwrap().ok, "second trade must fail");

    // The successful trade's position must still exist.
    assert_eq!(
        MockPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user_ok),
        1
    );
}

/// Result array length matches input batch length.
#[test]
fn result_array_length_matches_input() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    for _ in 0..3 {
        let user = funded_user(&env, &token, 1);
        trades.push_back(BatchTradeInput {
            user,
            token: token.clone(),
            amount: AMOUNT,
        });
    }

    let results = env
        .as_contract(&exec_id, || {
            TradeExecutorContract::batch_execute(env.clone(), trades)
        })
        .unwrap();

    assert_eq!(results.len(), 3);
}

/// Empty batch returns `InvalidAmount`.
#[test]
fn empty_batch_returns_invalid_amount() {
    let (env, exec_id, _) = setup();
    let trades: Vec<BatchTradeInput> = Vec::new(&env);

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::batch_execute(env.clone(), trades)
    });

    assert_eq!(err, Err(ContractError::InvalidAmount));
}

/// Batch exceeding `MAX_BATCH_SIZE` returns `InvalidAmount`.
#[test]
fn oversized_batch_returns_invalid_amount() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    for _ in 0..=(MAX_BATCH_SIZE) {
        let user = Address::generate(&env);
        trades.push_back(BatchTradeInput {
            user,
            token: token.clone(),
            amount: AMOUNT,
        });
    }

    let err = env.as_contract(&exec_id, || {
        TradeExecutorContract::batch_execute(env.clone(), trades)
    });

    assert_eq!(err, Err(ContractError::InvalidAmount));
}

/// Batch at exactly `MAX_BATCH_SIZE` is accepted.
#[test]
fn batch_at_max_size_is_accepted() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    for _ in 0..MAX_BATCH_SIZE {
        let user = funded_user(&env, &token, 1);
        trades.push_back(BatchTradeInput {
            user,
            token: token.clone(),
            amount: AMOUNT,
        });
    }

    let results = env
        .as_contract(&exec_id, || {
            TradeExecutorContract::batch_execute(env.clone(), trades)
        })
        .unwrap();

    assert_eq!(results.len(), MAX_BATCH_SIZE);
}

/// All trades in a batch can fail independently without panicking.
#[test]
fn all_trades_fail_returns_all_error_results() {
    let (env, exec_id, _) = setup();
    let token = sac(&env);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    for _ in 0..3 {
        let user = Address::generate(&env); // no balance
        trades.push_back(BatchTradeInput {
            user,
            token: token.clone(),
            amount: AMOUNT,
        });
    }

    let results = env
        .as_contract(&exec_id, || {
            TradeExecutorContract::batch_execute(env.clone(), trades)
        })
        .unwrap();

    assert_eq!(results.len(), 3);
    for i in 0..3 {
        assert!(!results.get(i).unwrap().ok);
    }
}
