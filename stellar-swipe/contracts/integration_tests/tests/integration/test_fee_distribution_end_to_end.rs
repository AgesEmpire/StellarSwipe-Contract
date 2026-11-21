//! End-to-end fee distribution integration test (issue #951).
//!
//! Exercises the full `fee_collector` distribution cycle against the real
//! Soroban test environment (not a mock):
//!
//!   fees collected (two copy-trade executions)
//!     -> provider shares recorded (50% / 30% / 15%, three providers)
//!     -> provider shares paid out from the treasury (queue -> timelock -> withdraw)
//!     -> provider balances updated
//!     -> treasury retains the remaining 5% protocol cut
//!
//! Every step goes through `fee_collector`'s public contract API — no crate-
//! internal test helpers — so a bug in the composition of these steps (an
//! off-by-one in the bps math, a step dropped from the sequence) would show
//! up here even though each step already has unit coverage in isolation.

extern crate std;

#[path = "fixtures/mod.rs"]
mod fixtures;

use fee_collector::ReportPeriod;
use fixtures::{FeeCollectorBuilder, FeeCollectorFixture};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env, String,
};
use stellar_swipe_common::Asset;

const SECONDS_PER_DAY: u64 = 86_400;

fn trade_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "XLM"),
        issuer: None,
    }
}

/// Builds the shared fee-collector fixture (Issue #1404): `fee_collector`
/// wired to the 1:1 oracle stub (`collect_fee` routes trade volume through
/// the oracle to a USD base value; 1:1 keeps the fee math legible), burn and
/// revenue share disabled so 100% of the net fee lands in the treasury, the
/// max fee rate (1%) so two modest trades add up to a round total, and two
/// funded traders.
fn setup() -> FeeCollectorFixture {
    FeeCollectorBuilder::new()
        .at_timestamp(10 * SECONDS_PER_DAY)
        .burn_rate_bps(0)
        .revenue_share_rate_bps(0)
        .fee_rate_bps(100)
        .fund_trader(10_000_000)
        .fund_trader(10_000_000)
        .build()
        .expect("fee collector fixture")
}

#[test]
fn test_fee_distribution_end_to_end() {
    let fixture = setup();
    let env = fixture.env.clone();
    let client = &fixture.client;
    let token = fixture.token.clone();
    let admin = fixture.admin.clone();
    let trader_a = fixture.traders[0].clone();
    let trader_b = fixture.traders[1].clone();
    let asset = trade_asset(&env);

    // ── 1. Fees collected from two copy-trade executions ────────────────────
    // Each trader's very first `collect_fee` call is waived by protocol design
    // (first-trade-free); prime that waiver with a throwaway call so the trade
    // that follows is the one that actually charges a fee.
    client.collect_fee(&trader_a, &token, &500_000, &asset);
    client.collect_fee(&trader_b, &token, &500_000, &asset);

    let fee_a = client.collect_fee(&trader_a, &token, &500_000, &asset);
    let fee_b = client.collect_fee(&trader_b, &token, &500_000, &asset);
    assert_eq!(fee_a, 5_000, "trade A fee should be 1% of 500_000");
    assert_eq!(fee_b, 5_000, "trade B fee should be 1% of 500_000");

    let total_fees = fee_a + fee_b;
    assert_eq!(total_fees, 10_000);
    assert_eq!(
        client.treasury_balance(&token),
        total_fees,
        "burn and revenue-share are disabled, so 100% of collected fees land in treasury"
    );

    // ── 2. Provider shares: 50% / 30% / 15% (5% retained by the protocol) ───
    let provider_a = Address::generate(&env);
    let provider_b = Address::generate(&env);
    let provider_c = Address::generate(&env);

    let share_a = total_fees * 50 / 100; // 5_000
    let share_b = total_fees * 30 / 100; // 3_000
    let share_c = total_fees * 15 / 100; // 1_500
    let treasury_cut = total_fees - share_a - share_b - share_c; // 500

    // Record the earnings-report bucket for each provider before any time
    // travel below, so `AllTime` reporting reflects the full distribution
    // regardless of which day each withdrawal timelock lands on.
    client.record_provider_fee_share(&admin, &provider_a, &share_a);
    client.record_provider_fee_share(&admin, &provider_b, &share_b);
    client.record_provider_fee_share(&admin, &provider_c, &share_c);

    // ── 3. Provider shares distributed from the treasury ────────────────────
    // `withdraw_treasury_fees` is timelocked 24h behind `queue_withdrawal` and
    // only one withdrawal may be queued at a time, so providers are paid out
    // sequentially, advancing the ledger past each timelock in turn.
    for (provider, share) in [
        (&provider_a, share_a),
        (&provider_b, share_b),
        (&provider_c, share_c),
    ] {
        client.queue_withdrawal(provider, &token, &share);
        env.ledger()
            .with_mut(|l| l.timestamp += SECONDS_PER_DAY + 1);
        client.withdraw_treasury_fees(provider, &token, &share);
    }

    // ── 4. Provider balances updated, matching their percentage exactly ─────
    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&provider_a), share_a);
    assert_eq!(token_client.balance(&provider_b), share_b);
    assert_eq!(token_client.balance(&provider_c), share_c);

    // Rounding: distributed shares + treasury retention == total collected
    // fees exactly — no dust lost or created.
    assert_eq!(share_a + share_b + share_c + treasury_cut, total_fees);

    // ── 5. Treasury retains its share ────────────────────────────────────────
    assert_eq!(
        client.treasury_balance(&token),
        treasury_cut,
        "treasury balance must equal total_fees - sum(provider_shares)"
    );

    // ── 6. Earnings report matches what was actually paid out ───────────────
    let report_a = client.get_provider_earnings_report(&provider_a, &ReportPeriod::AllTime);
    let report_b = client.get_provider_earnings_report(&provider_b, &ReportPeriod::AllTime);
    let report_c = client.get_provider_earnings_report(&provider_c, &ReportPeriod::AllTime);
    assert_eq!(report_a.fee_shares_earned, share_a);
    assert_eq!(report_b.fee_shares_earned, share_b);
    assert_eq!(report_c.fee_shares_earned, share_c);
    assert_eq!(report_a.total_earned, share_a);
    assert_eq!(report_b.total_earned, share_b);
    assert_eq!(report_c.total_earned, share_c);
}
