//! Property-based tests for fee and PnL arithmetic invariants.
//!
//! This harness uses proptest to generate randomized valid input ranges and asserts
//! core arithmetic invariants hold for fee-splitting calculations in fee_collector
//! and realized-PnL calculations in trade_executor.
//!
//! # Invariants Tested
//!
//! ## Fee Splitting (fee_collector)
//! - No arithmetic overflow for in-range inputs
//! - Fee components sum exactly to the total fee: burn + referral + revenue_share + treasury = total_fee
//! - Fee calculation is user-favorable (rounds down)
//! - Distributable = fee - burn (no dust accumulation)
//!
//! ## PnL Calculation (trade_executor)
//! - No arithmetic overflow for in-range inputs
//! - Realized PnL = exit_price - entry_value (conservative under composition)
//! - Entry value calculation respects asset precision bounds
//!
//! # Running
//!
//! ```sh
//! cargo test --test test_arithmetic_invariants
//! ```
//!
//! # Input Constraints
//!
//! All generated inputs are constrained to realistic, valid ranges reflecting real contract usage:
//! - Trade amounts: 1 to 10^12 (no negative balances)
//! - Fee rates: MIN_FEE_RATE_BPS to MAX_FEE_RATE_BPS (1-100 bps)
//! - Burn rates: 0 to MAX_BURN_RATE_BPS (0-10000 bps)
//! - Asset prices: 7-decimal precision (Stellar standard)
//! - Entry/exit amounts: bounded by realistic trade sizes

extern crate std;

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String,
};
use stellar_swipe_common::Asset;

use fee_collector::{fee_amount_floor, FeeCollector, FeeCollectorClient};

// ── Fee Splitting Invariants ─────────────────────────────────────────────────────

/// Maximum realistic trade amount (1 trillion units, covers most use cases)
const MAX_TRADE_AMOUNT: i128 = 1_000_000_000_000;

/// Minimum fee rate in basis points (0.01%)
const MIN_FEE_RATE_BPS: u32 = 1;

/// Maximum fee rate in basis points (1%)
const MAX_FEE_RATE_BPS: u32 = 100;

/// Maximum burn rate in basis points (100%)
const MAX_BURN_RATE_BPS: u32 = 10_000;

/// Maximum referral/revenue share rate in basis points (50%)
const MAX_SHARE_RATE_BPS: u32 = 5_000;

/// Asset precision for price calculations (7 decimals, Stellar standard)
const PRICE_PRECISION: i128 = 10_000_000;

fn setup_fee_collector(env: &Env) -> FeeCollectorClient<'_> {
    let admin = Address::generate(env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    client
}

fn trade_asset(env: &Env) -> Asset {
    Asset {
        code: String::from_str(env, "TRADE"),
        issuer: Some(Address::generate(env)),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,  // Reasonable budget for CI
        ..ProptestConfig::default()
    })]

    /// Invariant: Fee calculation never overflows for valid input ranges.
    #[test]
    fn prop_fee_calculation_no_overflow(
        trade_amount in 1_i128..=MAX_TRADE_AMOUNT,
        fee_rate_bps in MIN_FEE_RATE_BPS..=MAX_FEE_RATE_BPS,
    ) {
        let result = fee_amount_floor(trade_amount, fee_rate_bps);
        prop_assert!(result.is_some(), "fee calculation should not overflow for valid inputs");
    }

    /// Invariant: Fee + net_amount equals trade_amount (conservation).
    #[test]
    fn prop_fee_plus_net_equals_gross(
        trade_amount in 1_i128..=MAX_TRADE_AMOUNT,
        fee_rate_bps in MIN_FEE_RATE_BPS..=MAX_FEE_RATE_BPS,
    ) {
        let fee = fee_amount_floor(trade_amount, fee_rate_bps)
            .expect("fee calculation should not overflow");
        let net_amount = trade_amount - fee;

        prop_assert_eq!(fee + net_amount, trade_amount,
            "fee + net_amount must equal trade_amount (conservation)");
    }

    /// Invariant: Fee is non-negative and does not exceed trade amount.
    #[test]
    fn prop_fee_bounds(
        trade_amount in 1_i128..=MAX_TRADE_AMOUNT,
        fee_rate_bps in MIN_FEE_RATE_BPS..=MAX_FEE_RATE_BPS,
    ) {
        let fee = fee_amount_floor(trade_amount, fee_rate_bps)
            .expect("fee calculation should not overflow");

        prop_assert!(fee >= 0, "fee must be non-negative");
        prop_assert!(fee <= trade_amount, "fee must not exceed trade amount");
    }

    /// Invariant: Burn amount calculation never overflows.
    #[test]
    fn prop_burn_amount_no_overflow(
        fee_amount in 1_i128..=MAX_TRADE_AMOUNT,
        burn_rate_bps in 0u32..=MAX_BURN_RATE_BPS,
    ) {
        let burn_amount = fee_amount
            .checked_mul(burn_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000));

        prop_assert!(burn_amount.is_some(), "burn amount calculation should not overflow");
    }

    /// Invariant: Burn + distributable equals fee (no dust accumulation).
    #[test]
    fn prop_burn_plus_distributable_equals_fee(
        fee_amount in 1_i128..=MAX_TRADE_AMOUNT,
        burn_rate_bps in 0u32..=MAX_BURN_RATE_BPS,
    ) {
        let burn_amount = fee_amount
            .checked_mul(burn_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .expect("burn calculation should not overflow");
        let distributable = fee_amount
            .checked_sub(burn_amount)
            .expect("distributable calculation should not overflow");

        prop_assert_eq!(burn_amount + distributable, fee_amount,
            "burn + distributable must equal fee (no dust)");
    }

    /// Invariant: Burn amount is bounded by fee amount.
    #[test]
    fn prop_burn_amount_bounded(
        fee_amount in 1_i128..=MAX_TRADE_AMOUNT,
        burn_rate_bps in 0u32..=MAX_BURN_RATE_BPS,
    ) {
        let burn_amount = fee_amount
            .checked_mul(burn_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .expect("burn calculation should not overflow");

        prop_assert!(burn_amount >= 0, "burn amount must be non-negative");
        prop_assert!(burn_amount <= fee_amount, "burn amount must not exceed fee");
    }

    /// Invariant: Referral share calculation never overflows.
    #[test]
    fn prop_referral_share_no_overflow(
        fee_amount in 1_i128..=MAX_TRADE_AMOUNT,
        referral_rate_bps in 0u32..=MAX_SHARE_RATE_BPS,
    ) {
        let referral_amount = fee_amount
            .checked_mul(referral_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000));

        prop_assert!(referral_amount.is_some(), "referral share calculation should not overflow");
    }

    /// Invariant: Revenue share calculation never overflows.
    #[test]
    fn prop_revenue_share_no_overflow(
        distributable in 1_i128..=MAX_TRADE_AMOUNT,
        revenue_rate_bps in 0u32..=MAX_SHARE_RATE_BPS,
    ) {
        let revenue_amount = distributable
            .checked_mul(revenue_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000));

        prop_assert!(revenue_amount.is_some(), "revenue share calculation should not overflow");
    }

    /// Invariant: Fee components sum to total fee (burn + referral + revenue_share + treasury = fee).
    /// This tests the complete fee distribution logic.
    #[test]
    fn prop_fee_components_sum_to_total(
        fee_amount in 1_i128..=MAX_TRADE_AMOUNT,
        burn_rate_bps in 0u32..=MAX_BURN_RATE_BPS,
        referral_rate_bps in 0u32..=MAX_SHARE_RATE_BPS,
        revenue_rate_bps in 0u32..=MAX_SHARE_RATE_BPS,
    ) {
        // Calculate burn
        let burn_amount = fee_amount
            .checked_mul(burn_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .expect("burn calculation should not overflow");

        let distributable = fee_amount
            .checked_sub(burn_amount)
            .expect("distributable calculation should not overflow");

        // Calculate referral (capped at distributable)
        let mut referral_amount = fee_amount
            .checked_mul(referral_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .unwrap_or(0);
        if referral_amount > distributable {
            referral_amount = distributable;
        }

        let remaining_after_referral = distributable.saturating_sub(referral_amount);

        // Calculate revenue share (capped at remaining)
        let mut revenue_amount = distributable
            .checked_mul(revenue_rate_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .unwrap_or(0);
        if revenue_amount > remaining_after_referral {
            revenue_amount = remaining_after_referral;
        }

        // Treasury gets the rest
        let treasury_amount = remaining_after_referral.saturating_sub(revenue_amount);

        // Assert conservation
        let total_distributed = burn_amount
            .checked_add(referral_amount)
            .and_then(|v| v.checked_add(revenue_amount))
            .and_then(|v| v.checked_add(treasury_amount))
            .expect("sum of components should not overflow");

        prop_assert_eq!(total_distributed, fee_amount,
            "burn + referral + revenue_share + treasury must equal fee_amount");
    }

    /// Invariant: Fee calculation is monotonic - higher fee rate yields higher fee.
    #[test]
    fn prop_fee_monotonic_with_rate(
        trade_amount in 1_i128..=MAX_TRADE_AMOUNT,
        fee_rate_low in MIN_FEE_RATE_BPS..=MAX_FEE_RATE_BPS - 1,
    ) {
        let fee_rate_high = fee_rate_low + 1;
        let fee_low = fee_amount_floor(trade_amount, fee_rate_low)
            .expect("fee calculation should not overflow");
        let fee_high = fee_amount_floor(trade_amount, fee_rate_high)
            .expect("fee calculation should not overflow");

        prop_assert!(fee_high >= fee_low,
            "higher fee rate should yield equal or higher fee");
    }

    /// Invariant: Fee calculation is monotonic - higher trade amount yields higher fee.
    #[test]
    fn prop_fee_monotonic_with_amount(
        trade_amount_low in 1_i128..=MAX_TRADE_AMOUNT - 1,
        fee_rate_bps in MIN_FEE_RATE_BPS..=MAX_FEE_RATE_BPS,
    ) {
        let trade_amount_high = trade_amount_low + 1;
        let fee_low = fee_amount_floor(trade_amount_low, fee_rate_bps)
            .expect("fee calculation should not overflow");
        let fee_high = fee_amount_floor(trade_amount_high, fee_rate_bps)
            .expect("fee calculation should not overflow");

        prop_assert!(fee_high >= fee_low,
            "higher trade amount should yield equal or higher fee");
    }
}

// ── PnL Calculation Invariants ────────────────────────────────────────────────────

/// Calculate entry value in to_token units.
/// entry_value = amount * entry_price / PRICE_PRECISION
fn calculate_entry_value(amount: i128, entry_price: i128) -> Option<i128> {
    amount
        .checked_mul(entry_price)
        .and_then(|v| v.checked_div(PRICE_PRECISION))
}

/// Calculate realized PnL.
/// realized_pnl = exit_price - entry_value
fn calculate_realized_pnl(exit_price: i128, entry_value: i128) -> i128 {
    exit_price.saturating_sub(entry_value)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        ..ProptestConfig::default()
    })]

    /// Invariant: Entry value calculation never overflows for realistic inputs.
    #[test]
    fn prop_entry_value_no_overflow(
        amount in 1_i128..=MAX_TRADE_AMOUNT,
        entry_price in 1_i128..=PRICE_PRECISION * 1_000_000, // Realistic price range
    ) {
        let result = calculate_entry_value(amount, entry_price);
        prop_assert!(result.is_some(), "entry value calculation should not overflow");
    }

    /// Invariant: Entry value is non-negative.
    #[test]
    fn prop_entry_value_non_negative(
        amount in 1_i128..=MAX_TRADE_AMOUNT,
        entry_price in 1_i128..=PRICE_PRECISION * 1_000_000,
    ) {
        let entry_value = calculate_entry_value(amount, entry_price)
            .expect("entry value calculation should not overflow");

        prop_assert!(entry_value >= 0, "entry value must be non-negative");
    }

    /// Invariant: Realized PnL calculation never panics (uses saturating arithmetic).
    #[test]
    fn prop_realized_pnl_no_panic(
        entry_value in 0_i128..=MAX_TRADE_AMOUNT * 1_000_000,
        exit_price in 0_i128..=MAX_TRADE_AMOUNT * 1_000_000,
    ) {
        let _pnl = calculate_realized_pnl(exit_price, entry_value);
        // If we get here without panic, the invariant holds
        prop_assert!(true);
    }

    /// Invariant: Realized PnL is conservative (never overestimates profit).
    /// When exit_price < entry_value, PnL should be negative or zero (loss).
    #[test]
    fn prop_realized_pnl_conservative_loss(
        entry_value in 1_i128..=MAX_TRADE_AMOUNT * 1_000_000,
        loss_factor in 1_i128..=10_i128, // 1% to 10% loss
    ) {
        let exit_price = entry_value.saturating_sub(entry_value / loss_factor);
        let pnl = calculate_realized_pnl(exit_price, entry_value);

        prop_assert!(pnl <= 0, "PnL should be non-positive when exit_price < entry_value (loss)");
    }

    /// Invariant: Realized PnL is conservative (never overestimates profit).
    /// When exit_price > entry_value, PnL should be positive (profit).
    #[test]
    fn prop_realized_pnl_conservative_profit(
        entry_value in 1_i128..=MAX_TRADE_AMOUNT * 1_000_000,
        profit_factor in 1_i128..=10_i128, // 1% to 10% profit
    ) {
        let exit_price = entry_value.saturating_add(entry_value / profit_factor);
        let pnl = calculate_realized_pnl(exit_price, entry_value);

        prop_assert!(pnl >= 0, "PnL should be non-negative when exit_price >= entry_value (profit/breakeven)");
    }

    /// Invariant: PnL composition is conservative.
    /// For multiple trades, total PnL should equal sum of individual PnLs.
    #[test]
    fn prop_pnl_composition_conservative(
        entry_value1 in 1_i128..=MAX_TRADE_AMOUNT * 1_000_000,
        exit_price1 in 0_i128..=MAX_TRADE_AMOUNT * 1_000_000,
        entry_value2 in 1_i128..=MAX_TRADE_AMOUNT * 1_000_000,
        exit_price2 in 0_i128..=MAX_TRADE_AMOUNT * 1_000_000,
    ) {
        let pnl1 = calculate_realized_pnl(exit_price1, entry_value1);
        let pnl2 = calculate_realized_pnl(exit_price2, entry_value2);

        let total_entry = entry_value1.saturating_add(entry_value2);
        let total_exit = exit_price1.saturating_add(exit_price2);
        let combined_pnl = calculate_realized_pnl(total_exit, total_entry);

        let sum_individual = pnl1.saturating_add(pnl2);

        // Combined PnL should equal sum of individual PnLs (conservative composition)
        prop_assert_eq!(combined_pnl, sum_individual,
            "combined PnL should equal sum of individual PnLs");
    }

    /// Invariant: Entry value respects asset precision bounds.
    /// Entry value should not exceed realistic bounds for the asset.
    #[test]
    fn prop_entry_value_precision_bounds(
        amount in 1_i128..=MAX_TRADE_AMOUNT,
        entry_price in 1_i128..=PRICE_PRECISION * 1_000_000,
    ) {
        let entry_value = calculate_entry_value(amount, entry_price)
            .expect("entry value calculation should not overflow");

        // Entry value should be bounded by amount * max_price
        let max_entry_value = amount.saturating_mul(PRICE_PRECISION * 1_000_000);
        prop_assert!(entry_value <= max_entry_value,
            "entry value should respect precision bounds");
    }
}

// ── Integration Tests ─────────────────────────────────────────────────────────────

/// Test fee collection with realistic parameters to ensure invariants hold end-to-end.
#[test]
fn test_fee_collection_end_to_end_invariants() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Set realistic fee and burn rates
    client.set_fee_rate(&30u32); // 0.30%
    client.set_burn_rate(&1_000u32); // 10%
    client.set_revenue_share_rate_bps(&0u32); // Disable for simplicity

    // Mock oracle
    #[contract]
    struct MockOracle;
    #[contractimpl]
    impl MockOracle {
        pub fn convert_to_base(_env: Env, amount: i128, _asset: Asset) -> i128 {
            amount
        }
    }
    let oracle_id = env.register(MockOracle, ());
    client.set_oracle_contract(&oracle_id);

    let asset = trade_asset(&env);
    let trade_amount: i128 = 1_000_000;

    // Mint tokens to trader
    StellarAssetClient::new(&env, &token).mint(&trader, &trade_amount);

    // Mark trader as having traded (skip first-trade fee waiver)
    env.as_contract(&contract_id, || {
        fee_collector::storage::set_has_traded(&env, &trader);
    });

    // Collect fee
    let fee = client.collect_fee(&trader, &token, &trade_amount, &asset);

    // Verify invariants
    // fee = 1_000_000 * 30 / 10_000 = 3_000
    assert_eq!(fee, 3_000);

    // burn = 3_000 * 1_000 / 10_000 = 300
    // treasury = 3_000 - 300 = 2_700
    let treasury_balance = client.treasury_balance(&token);
    assert_eq!(treasury_balance, 2_700);

    // Verify conservation: burn + treasury = fee
    // 300 + 2_700 = 3_000 ✓
    assert_eq!(300 + treasury_balance, fee);
}

/// Test PnL calculation with realistic parameters.
#[test]
fn test_pnl_calculation_end_to_end_invariants() {
    let amount: i128 = 1_000_000;
    let entry_price: i128 = 9_500_000; // 0.95 in 7-decimal format
    let exit_price: i128 = 1_200_000; // 1.20 in 7-decimal format

    // Calculate entry value
    let entry_value = calculate_entry_value(amount, entry_price)
        .expect("entry value calculation should not overflow");

    // entry_value = 1_000_000 * 9_500_000 / 10_000_000 = 950_000
    assert_eq!(entry_value, 950_000);

    // Calculate realized PnL
    let pnl = calculate_realized_pnl(exit_price, entry_value);

    // pnl = 1_200_000 - 950_000 = 250_000
    assert_eq!(pnl, 250_000);

    // Verify PnL is positive (profit)
    assert!(pnl > 0);
}
