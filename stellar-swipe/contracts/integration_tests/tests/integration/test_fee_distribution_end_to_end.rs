//! End-to-end integration test for the fee_collector distribution cycle
//! (Issue #800): fees collected → epoch closed → provider shares
//! distributed → provider balances updated → treasury share retained.
//!
//! `fee_collector/src/test.rs` only unit-tests individual pipeline steps in
//! isolation; this test runs the full cycle against a real Soroban test
//! environment (no mocking of the contract under test) to catch composition
//! bugs — e.g. an off-by-one in basis-point math, or a missed step between
//! fee collection and provider settlement — that per-function unit tests
//! cannot see.
//!
//! Cycle modeled here, using the contract's real, already-existing
//! primitives:
//!   1. Two copy-trade executions call `collect_fee`, each splitting its fee
//!      between the treasury and the revenue-share pool (`RevenueShareRateBps`).
//!   2. The revenue-share pool — the fee revenue earmarked for signal
//!      providers — is allocated across three providers with a 50/30/20
//!      split via `record_provider_fee_share`, the entry point the contract
//!      documents as "called by the fee distribution system... when
//!      allocating fee shares to a signal provider".
//!   3. `trigger_revenue_share_snapshot` closes the epoch, clearing the pool.
//!   4. Each provider's credited balance is read back via
//!      `get_provider_earnings_report` and reconciled against the treasury's
//!      retained share, with no fee-revenue dust unaccounted for.

extern crate std;

use fee_collector::{FeeCollector, FeeCollectorClient, ReportPeriod};
use soroban_sdk::{
    contract, contractimpl,
    testutils::Address as _,
    token::StellarAssetClient,
    Address, Env, String,
};
use stellar_swipe_common::Asset;

// Minimal oracle stub: 1:1 conversion, sufficient for `collect_fee`'s
// volume-tracking oracle call (`fee_collector`'s own `MockOracleContract`
// test helper is private to its crate, so integration tests need their own).
#[contract]
struct OracleStub;

#[contractimpl]
impl OracleStub {
    pub fn convert_to_base(_env: Env, amount: i128, _asset: Asset) -> i128 {
        amount
    }
}

fn trade_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

/// Fee collection is waived on a trader's first trade (Issue #428). Each
/// trader here "warms up" with a trivial trade before the real copy-trade
/// execution that is meant to count toward this test's totals.
fn warm_up(client: &FeeCollectorClient<'_>, trader: &Address, token: &Address, asset: &Asset) {
    let waived = client.collect_fee(trader, token, &1i128, asset);
    assert_eq!(waived, 0i128, "first trade per trader must be fee-free");
}

#[test]
fn test_fee_distribution_end_to_end() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader_a = Address::generate(&env);
    let trader_b = Address::generate(&env);
    let provider_a = Address::generate(&env);
    let provider_b = Address::generate(&env);
    let provider_c = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let oracle_id = env.register(OracleStub, ());
    client.set_oracle_contract(&oracle_id);
    let asset = trade_asset(&env);

    // Fixed 1% fee, no burn, so every stroop of `distributable` fee revenue
    // is accounted for by exactly the treasury + the revenue-share pool.
    client.set_fee_rate(&100u32);
    client.set_burn_rate(&0u32);
    client.set_revenue_share_rate_bps(&8_000u32); // 80% of fees go to providers

    StellarAssetClient::new(&env, &token).mint(&trader_a, &(1_000_000 * 10_000_000));
    StellarAssetClient::new(&env, &token).mint(&trader_b, &(1_000_000 * 10_000_000));

    warm_up(&client, &trader_a, &token, &asset);
    warm_up(&client, &trader_b, &token, &asset);

    // ── Step (b): two copy-trade executions ─────────────────────────────────
    let fee_1 = client.collect_fee(&trader_a, &token, &100_000_000i128, &asset);
    let fee_2 = client.collect_fee(&trader_b, &token, &200_000_000i128, &asset);
    assert_eq!(fee_1, 1_000_000i128);
    assert_eq!(fee_2, 2_000_000i128);
    let total_fees = fee_1 + fee_2;

    // Revenue-share pool and treasury both accumulate deterministically from
    // the 80/20 split applied at collection time.
    let pool_before_distribution = client.get_revenue_share_pool(&token);
    assert_eq!(pool_before_distribution, total_fees * 8_000 / 10_000);
    let treasury_after_collection = client.treasury_balance(&token);
    assert_eq!(treasury_after_collection, total_fees - pool_before_distribution);

    // ── Step (a)/(the "three providers"): split the pool 50/30/20 ───────────
    let share_a = pool_before_distribution * 5_000 / 10_000;
    let share_b = pool_before_distribution * 3_000 / 10_000;
    let share_c = pool_before_distribution * 2_000 / 10_000;
    // No dust: bps split of the pool must sum back to the whole pool exactly.
    assert_eq!(share_a + share_b + share_c, pool_before_distribution);

    client.record_provider_fee_share(&admin, &provider_a, &share_a);
    client.record_provider_fee_share(&admin, &provider_b, &share_b);
    client.record_provider_fee_share(&admin, &provider_c, &share_c);

    // ── Step (c): close the epoch ────────────────────────────────────────────
    client.trigger_revenue_share_snapshot(&admin, &token);
    assert_eq!(
        client.get_revenue_share_pool(&token),
        0i128,
        "epoch close must clear the pool for the next cycle"
    );

    // ── Step (d): provider balances updated, within the expected split ──────
    let report_a = client.get_provider_earnings_report(&provider_a, &ReportPeriod::Daily);
    let report_b = client.get_provider_earnings_report(&provider_b, &ReportPeriod::Daily);
    let report_c = client.get_provider_earnings_report(&provider_c, &ReportPeriod::Daily);
    assert_eq!(report_a.fee_shares_earned, share_a);
    assert_eq!(report_b.fee_shares_earned, share_b);
    assert_eq!(report_c.fee_shares_earned, share_c);
    assert_eq!(report_a.total_earned, share_a);

    // ── Step (e): treasury retains its share; nothing is lost ───────────────
    let treasury_final = client.treasury_balance(&token);
    assert_eq!(
        treasury_final, treasury_after_collection,
        "distributing provider shares must not touch the treasury balance"
    );
    let distributed_to_providers = report_a.fee_shares_earned
        + report_b.fee_shares_earned
        + report_c.fee_shares_earned;
    assert_eq!(
        distributed_to_providers + treasury_final,
        total_fees,
        "provider shares + treasury retention must equal total fees collected, no dust lost"
    );
}

/// Single-provider variant covering the exact-split boundary: with only one
/// claimant the entire pool goes to them and the reconciliation still holds.
#[test]
fn test_fee_distribution_single_provider_no_rounding_dust() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let provider = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let oracle_id = env.register(OracleStub, ());
    client.set_oracle_contract(&oracle_id);
    let asset = trade_asset(&env);

    client.set_fee_rate(&100u32);
    client.set_burn_rate(&0u32);
    client.set_revenue_share_rate_bps(&8_000u32);

    StellarAssetClient::new(&env, &token).mint(&trader, &(1_000_000 * 10_000_000));
    warm_up(&client, &trader, &token, &asset);

    let fee = client.collect_fee(&trader, &token, &100_000_000i128, &asset);
    assert_eq!(fee, 1_000_000i128);

    let pool = client.get_revenue_share_pool(&token);
    let treasury_after_collection = client.treasury_balance(&token);

    client.record_provider_fee_share(&admin, &provider, &pool);
    client.trigger_revenue_share_snapshot(&admin, &token);

    let report = client.get_provider_earnings_report(&provider, &ReportPeriod::Daily);
    assert_eq!(report.fee_shares_earned, pool);
    assert_eq!(client.treasury_balance(&token), treasury_after_collection);
    assert_eq!(report.fee_shares_earned + client.treasury_balance(&token), fee);
}
