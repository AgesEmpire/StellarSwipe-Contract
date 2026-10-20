//! Issue #1215: settlement must deliver at least the caller-provided minimum
//! output, and a shortfall must roll back every transfer and accounting write.
//!
//! Exercised through the contract client so the host's rollback-on-`Err`
//! semantics apply exactly as they do on-chain.

use crate::errors::ContractError;
use crate::test::MockPortfolioWithPositions;
use crate::test::{MockPortfolioWithPositionsClient, MockSdexRouter, MockSdexRouterClient};
use crate::{ReplayParams, TradeExecutorContract, TradeExecutorContractClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    Address, Bytes, Env,
};

const AMOUNT_IN: i128 = 1_000_000;
const MIN_OUT: i128 = 900_000;

struct Setup {
    env: Env,
    exec: Address,
    router: Address,
    portfolio: Address,
    user: Address,
    token_a: Address,
    token_b: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let router = env.register(MockSdexRouter, ());
    let portfolio = env.register(MockPortfolioWithPositions, ());
    let exec = env.register(TradeExecutorContract, ());

    let client = TradeExecutorContractClient::new(&env, &exec);
    client.initialize(&admin);
    client.set_sdex_router(&router);
    client.set_user_portfolio(&portfolio);

    StellarAssetClient::new(&env, &token_a).mint(&exec, &1_000_000_000);
    StellarAssetClient::new(&env, &token_b).mint(&router, &10_000_000_000);

    Setup {
        env,
        exec,
        router,
        portfolio,
        user,
        token_a,
        token_b,
    }
}

impl Setup {
    fn client(&self) -> TradeExecutorContractClient<'_> {
        TradeExecutorContractClient::new(&self.env, &self.exec)
    }

    fn set_router_output(&self, out: i128) {
        MockSdexRouterClient::new(&self.env, &self.router).set_amount_out(&out);
    }

    /// (executor token_a, executor token_b, router token_a, router token_b)
    fn balances(&self) -> (i128, i128, i128, i128) {
        let a = TokenClient::new(&self.env, &self.token_a);
        let b = TokenClient::new(&self.env, &self.token_b);
        (
            a.balance(&self.exec),
            b.balance(&self.exec),
            a.balance(&self.router),
            b.balance(&self.router),
        )
    }

    fn replay(&self, nonce: u64) -> ReplayParams {
        let mut hash = [0u8; 32];
        hash[0] = nonce as u8;
        hash[31] = 0xAB;
        ReplayParams {
            nonce,
            tx_hash: Bytes::from_array(&self.env, &hash),
            expiry_ts: self.env.ledger().timestamp() + 86_400,
        }
    }
}

// ── swap: exact / below / zero ───────────────────────────────────────────────

#[test]
fn settlement_at_exact_minimum_succeeds() {
    let s = setup();
    s.set_router_output(MIN_OUT);
    let before = s.balances();

    let out = s
        .client()
        .swap(&s.token_a, &s.token_b, &AMOUNT_IN, &MIN_OUT);

    assert_eq!(out, MIN_OUT);
    let after = s.balances();
    assert_eq!(after.0, before.0 - AMOUNT_IN);
    assert_eq!(after.1, before.1 + MIN_OUT);
    assert_eq!(after.2, before.2 + AMOUNT_IN);
    assert_eq!(after.3, before.3 - MIN_OUT);
}

#[test]
fn settlement_one_below_minimum_reverts_and_rolls_back_transfers() {
    let s = setup();
    s.set_router_output(MIN_OUT - 1);
    let before = s.balances();

    let res = s
        .client()
        .try_swap(&s.token_a, &s.token_b, &AMOUNT_IN, &MIN_OUT);

    assert_eq!(res, Err(Ok(ContractError::SlippageExceeded)));
    // The router already pulled input and paid output inside the call; both
    // transfers must be undone.
    assert_eq!(s.balances(), before);
}

#[test]
fn zero_output_fill_reverts_and_rolls_back_transfers() {
    let s = setup();
    s.set_router_output(0);
    let before = s.balances();

    let res = s
        .client()
        .try_swap(&s.token_a, &s.token_b, &AMOUNT_IN, &MIN_OUT);

    assert_eq!(res, Err(Ok(ContractError::SlippageExceeded)));
    assert_eq!(s.balances(), before);
}

#[test]
fn zero_minimum_output_is_rejected_before_any_transfer() {
    let s = setup();
    s.set_router_output(0);
    let before = s.balances();

    let res = s.client().try_swap(&s.token_a, &s.token_b, &AMOUNT_IN, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));

    let res = s.client().try_swap(&s.token_a, &s.token_b, &AMOUNT_IN, &-1);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));

    assert_eq!(s.balances(), before);
}

#[test]
fn full_slippage_tolerance_cannot_produce_zero_minimum() {
    let s = setup();
    s.set_router_output(0);
    let before = s.balances();

    let res = s
        .client()
        .try_swap_with_slippage(&s.token_a, &s.token_b, &AMOUNT_IN, &10_000);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    assert_eq!(s.balances(), before);
}

// ── cancel_copy_trade: accounting rollback ───────────────────────────────────

#[test]
fn cancel_below_minimum_rolls_back_accounting_and_nonce() {
    let s = setup();
    let portfolio = MockPortfolioWithPositionsClient::new(&s.env, &s.portfolio);
    portfolio.add_position_with_entry_price(&s.user, &1u64, &10_000_000i128);

    let oi_before = s.client().get_open_interest(&s.token_a);
    let count_before = s.client().get_user_position_count(&s.user);
    let balances_before = s.balances();

    s.set_router_output(MIN_OUT - 1);
    let res = s.client().try_cancel_copy_trade(
        &s.user,
        &s.user,
        &1u64,
        &s.token_a,
        &s.token_b,
        &AMOUNT_IN,
        &MIN_OUT,
        &10_000_000,
        &s.replay(1),
    );
    assert_eq!(res, Err(Ok(ContractError::SlippageExceeded)));

    // Position still open, nothing closed, no accounting or balance drift.
    assert!(portfolio.has_position(&s.user, &1u64));
    assert_eq!(portfolio.last_closed(), None);
    assert_eq!(s.client().get_open_interest(&s.token_a), oi_before);
    assert_eq!(s.client().get_user_position_count(&s.user), count_before);
    assert_eq!(s.balances(), balances_before);

    // The replay nonce commit was rolled back too: the same request settles
    // once the output meets the minimum exactly.
    s.set_router_output(MIN_OUT);
    s.client().cancel_copy_trade(
        &s.user,
        &s.user,
        &1u64,
        &s.token_a,
        &s.token_b,
        &AMOUNT_IN,
        &MIN_OUT,
        &10_000_000,
        &s.replay(1),
    );
    assert!(!portfolio.has_position(&s.user, &1u64));
    assert_eq!(portfolio.last_closed(), Some(1u64));
    assert_eq!(s.balances().1, balances_before.1 + MIN_OUT);
}
