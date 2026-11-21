#![cfg(test)]
// Unit tests for atomic (all-or-nothing) `batch_execute_atomic` (#793).
//
// Verifies that:
// - `atomic = false` behaves identically to the existing best-effort `batch_execute`.
// - `atomic = true` with all trades succeeding commits normally.
// - `atomic = true` with a mixed batch panics and rolls back *every* trade's
//   state, including ones that individually succeeded earlier in the batch.
// - The `mark_atomic_rollback` marking logic is correct in isolation.

extern crate std;

use crate::{
    errors::ContractError, mark_atomic_rollback, risk_gates::DEFAULT_ESTIMATED_COPY_TRADE_FEE,
    BatchTradeInput, BatchTradeResult, TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, token::StellarAssetClient,
    Address, Env, Vec,
};

// ── Mock UserPortfolio (mirrors test_batch_execute.rs) ───────────────────────

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

fn funded_user(env: &Env, token: &Address, n: i128) -> Address {
    let user = Address::generate(env);
    StellarAssetClient::new(env, token)
        .mint(&user, &(n * (AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE)));
    user
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// `atomic = true`, every trade succeeds: returns normally, nothing rolled back.
#[test]
fn atomic_all_success_commits_and_is_not_marked_rolled_back() {
    let (env, exec_id, portfolio_id) = setup();
    let token = sac(&env);

    let user1 = funded_user(&env, &token, 1);
    let user2 = funded_user(&env, &token, 1);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    trades.push_back(BatchTradeInput {
        user: user1.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });
    trades.push_back(BatchTradeInput {
        user: user2.clone(),
        token: token.clone(),
        amount: AMOUNT,
    });

    let results = env
        .as_contract(&exec_id, || {
            TradeExecutorContract::batch_execute_atomic(env.clone(), trades, true)
        })
        .unwrap();

    assert_eq!(results.len(), 2);
    for i in 0..2 {
        let r = results.get(i).unwrap();
        assert!(r.ok);
        assert!(!r.atomic_rollback, "no rollback occurred; must stay false");
    }

    let pf = MockPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(pf.get_open_position_count(&user1), 1);
    assert_eq!(pf.get_open_position_count(&user2), 1);
}

/// `atomic = true`, first trade succeeds, second fails: the whole call panics,
/// and — critically — the first trade's *already-applied* state change is
/// gone afterward, because Soroban reverts every storage effect from a
/// panicking top-level invocation, not just the failing sub-step.
#[test]
fn atomic_mixed_batch_panics_and_rolls_back_the_successful_trade_too() {
    let (env, exec_id, portfolio_id) = setup();
    let token = sac(&env);

    let user_ok = funded_user(&env, &token, 1);
    let user_fail = Address::generate(&env); // no balance -> InsufficientBalance

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

    // Must go through the real client (not a direct associated-function call)
    // so the panic is mediated by Soroban's actual invocation/rollback
    // machinery rather than just unwinding the test's own call stack.
    let client = TradeExecutorContractClient::new(&env, &exec_id);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.batch_execute_atomic(&trades, &true);
    }));
    assert!(
        result.is_err(),
        "atomic batch with a failing trade must panic"
    );

    // The first trade individually succeeded partway through the call, but
    // since the whole invocation panicked, that state change must be gone.
    let pf = MockPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(
        pf.get_open_position_count(&user_ok),
        0,
        "the successful trade's position must have been rolled back along with the failed one"
    );
}

/// `atomic = false` via the new entry point behaves exactly like the
/// pre-existing best-effort `batch_execute`: partial success is preserved.
#[test]
fn non_atomic_via_new_entrypoint_preserves_best_effort_semantics() {
    let (env, exec_id, portfolio_id) = setup();
    let token = sac(&env);

    let user_ok = funded_user(&env, &token, 1);
    let user_fail = Address::generate(&env);

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
            TradeExecutorContract::batch_execute_atomic(env.clone(), trades, false)
        })
        .unwrap();

    assert!(results.get(0).unwrap().ok, "first trade must succeed");
    assert!(!results.get(1).unwrap().ok, "second trade must fail");
    assert!(!results.get(0).unwrap().atomic_rollback);
    assert!(!results.get(1).unwrap().atomic_rollback);

    assert_eq!(
        MockPortfolioClient::new(&env, &portfolio_id).get_open_position_count(&user_ok),
        1,
        "non-atomic mode must still commit the successful trade"
    );
}

/// `mark_atomic_rollback` in isolation: no panic involved, just the pure
/// marking logic that `batch_execute_impl` uses right before panicking.
#[test]
fn mark_atomic_rollback_flags_every_entry_only_when_any_failed() {
    let env = Env::default();

    let all_ok: Vec<BatchTradeResult> = Vec::from_array(
        &env,
        [
            BatchTradeResult {
                ok: true,
                error_code: 0,
                atomic_rollback: false,
            },
            BatchTradeResult {
                ok: true,
                error_code: 0,
                atomic_rollback: false,
            },
        ],
    );
    let marked = mark_atomic_rollback(&env, &all_ok);
    assert!(marked.iter().all(|r| !r.atomic_rollback));

    let mixed: Vec<BatchTradeResult> = Vec::from_array(
        &env,
        [
            BatchTradeResult {
                ok: true,
                error_code: 0,
                atomic_rollback: false,
            },
            BatchTradeResult {
                ok: false,
                error_code: ContractError::InsufficientBalance as u32,
                atomic_rollback: false,
            },
        ],
    );
    let marked = mark_atomic_rollback(&env, &mixed);
    assert!(
        marked.iter().all(|r| r.atomic_rollback),
        "every entry, including the successful one, must be marked once any trade failed"
    );
    // Underlying ok/error_code must be preserved, only the marker changes.
    assert!(marked.get(0).unwrap().ok);
    assert!(!marked.get(1).unwrap().ok);
    assert_eq!(
        marked.get(1).unwrap().error_code,
        ContractError::InsufficientBalance as u32
    );
}
