#![cfg(test)]
//! Latency benchmarks and performance regression guards for copy-trade execution.

use crate::{
    risk_gates::{DEFAULT_ESTIMATED_COPY_TRADE_FEE, MAX_BATCH_SIZE},
    BatchTradeInput, TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, token::StellarAssetClient,
    Address, Env, Vec,
};
use stellar_swipe_common::budget_regression::measure_and_emit;
use stellar_swipe_common::perf::{
    regression_budget_limit, BASELINE_COPY_TRADE_INSTRUCTIONS, REGRESSION_BUDGET_PCT,
};

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
}

const AMOUNT: i128 = 1_000_000;

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let portfolio_id = env.register(MockPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());
    let token = {
        let issuer = Address::generate(&env);
        env.register_stellar_asset_contract_v2(issuer).address()
    };

    let client = TradeExecutorContractClient::new(&env, &exec_id);
    client.initialize(&admin);
    client.set_user_portfolio(&portfolio_id);

    (env, exec_id, token, admin)
}

fn funded_user(env: &Env, token: &Address) -> Address {
    let user = Address::generate(env);
    StellarAssetClient::new(env, token).mint(&user, &(AMOUNT + DEFAULT_ESTIMATED_COPY_TRADE_FEE));
    user
}

#[test]
fn test_single_copy_trade_latency_regression() {
    let (env, exec_id, token, _) = setup();
    let client = TradeExecutorContractClient::new(&env, &exec_id);
    let user = funded_user(&env, &token);

    client.execute_copy_trade(
        &user,
        &token,
        &AMOUNT,
        &None,
        &crate::OrderType::Market,
        &None,
    );

    let instructions = env.cost_estimate().budget().cpu_instruction_cost();
    assert!(
        instructions <= regression_budget_limit(),
        "copy trade used {instructions} instructions (>{REGRESSION_BUDGET_PCT}% budget)"
    );
    assert!(
        instructions <= BASELINE_COPY_TRADE_INSTRUCTIONS * 3,
        "copy trade {instructions} exceeds 3x baseline ({BASELINE_COPY_TRADE_INSTRUCTIONS})"
    );
    measure_and_emit("trade_executor.execute_copy_trade", BASELINE_COPY_TRADE_INSTRUCTIONS, instructions);
}

#[test]
fn test_batch_execute_amortized_latency() {
    let (env, exec_id, token, _) = setup();
    let client = TradeExecutorContractClient::new(&env, &exec_id);

    let mut trades: Vec<BatchTradeInput> = Vec::new(&env);
    for _ in 0..MAX_BATCH_SIZE {
        let user = funded_user(&env, &token);
        trades.push_back(BatchTradeInput {
            user,
            token: token.clone(),
            amount: AMOUNT,
        });
    }

    client.batch_execute(&trades);

    let per_trade = env.cost_estimate().budget().cpu_instruction_cost() / MAX_BATCH_SIZE as u64;
    assert!(
        per_trade <= BASELINE_COPY_TRADE_INSTRUCTIONS * 2,
        "batch amortized cost {per_trade} per trade too high"
    );
}
