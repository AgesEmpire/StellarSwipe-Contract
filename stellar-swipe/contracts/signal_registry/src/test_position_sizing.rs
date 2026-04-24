#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Env};

use crate::{
    position_sizing::{
        calculate_kelly_fraction, calculate_volatility, get_sizing_config, get_price_history,
        record_price, set_sizing_config, PositionSizingConfig, SizingMethod,
        DEFAULT_VOLATILITY_BPS, MAX_VOLATILITY_BPS, MIN_POSITION_SIZE,
    },
    risk::{set_asset_price, update_position},
    AutoTradeContract,
};

// Helper contract registration
struct TestContract;

soroban_sdk::contractimpl!(TestContract, ());

fn setup_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

fn make_contract(env: &Env) -> soroban_sdk::Address {
    env.register(AutoTradeContract, ())
}

// ---------------------------------------------------------------------------
// Volatility calculation tests
// ---------------------------------------------------------------------------

#[test]
fn test_volatility_no_history_returns_default() {
    let env = setup_env();
    let contract = make_contract(&env);
    env.as_contract(&contract, || {
        let vol = calculate_volatility(&env, 1, 30);
        assert_eq!(vol, DEFAULT_VOLATILITY_BPS);
    });
}

#[test]
fn test_volatility_single_price_returns_default() {
    let env = setup_env();
    let contract = make_contract(&env);
    env.as_contract(&contract, || {
        record_price(&env, 1, 100_000);
        let vol = calculate_volatility(&env, 1, 30);
        assert_eq!(vol, DEFAULT_VOLATILITY_BPS); // needs ≥2 prices
    });
}

#[test]
fn test_volatility_constant_prices_is_zero() {
    let env = setup_env();
    let contract = make_contract(&env);
    env.as_contract(&contract, || {
        // Constant price → zero returns → zero variance → zero volatility
        for _ in 0..10 {
            record_price(&env, 1, 100_000);
        }
        let vol = calculate_volatility(&env, 1, 10);
        assert_eq!(vol, 0);
    });
}

#[test]
fn test_volatility_increases_with_price_swings() {
    let env = setup_env();
    let contract = make_contract(&env);
    env.as_contract(&contract, || {
        // Low-volatility asset: small swings
        let prices_low = [100, 101, 100, 101, 100u64];
        for p in &prices_low {
            record_price(&env, 1, *p as i128);
        }
        let low_vol = calculate_volatility(&env, 1, 10);

        // High-volatility asset: large swings
        for p in &[100i128, 130, 80, 140, 70] {
            record_price(&env, 2, *p);
        }
        let high_vol = calculate_volatility(&env, 2, 10);

        assert!(
            high_vol > low_vol,
            "high_vol={} should exceed low_vol={}",
            high_vol,
            low_vol
        );
    });
}

#[test]
fn test_volatility_30_day_window() {
    let env = setup_env();
    let contract = make_contract(&env);
    env.as_contract(&contract, || {
        // Simulate 31 prices for XLM/USDC-like asset (asset_id = 10)
        // Alternating +2% / -2% gives a predictable volatility
        let mut price: i128 = 120_000; // 0.12 USDC in stroops * 1e6
        for i in 0..31 {
            record_price(&env, 10, price);
            if i % 2 == 0 {
                price = price * 102 / 100;
            } else {
                price = price * 98 / 100;
            }
        }
        let vol = calculate_volatility(&env, 10, 30);
        // Expect something roughly around 200 bps (2% daily swings)
        assert!(vol > 0, "volatility should be positive");
        assert!(vol < DEFAULT_VOLATILITY_BPS, "alternating 2% swings should be below default 20%");
    });
}

#[test]
fn test_price_history_ring_buffer_wraps() {
    let env = setup_env();
    let contract = make_contract(&env);
    env.as_contract(&contract, || {
        // Write 65 prices (> MAX_HISTORY_SLOTS = 60)
        for i in 0..65i128 {
            record_price(&env, 5, i * 1000 + 1000);
        }
        // Should still return at most 60 prices
        let hist = get_price_history(&env, 5, 60);
        assert_eq!(hist.len(), 60);
    });
}

// ---------------------------------------------------------------------------
// Kelly Criterion tests
// ---------------------------------------------------------------------------

#[test]
fn test_kelly_zero_avg_win_returns_zero() {
    let kelly = calculate_kelly_fraction(6000, 0, 300);
    assert_eq!(kelly, 0);
}

#[test]
fn test_kelly_negative_expectancy_returns_zero() {
    // Win 40% of the time, avg win = 500 bps, avg loss = 1000 bps
    // kelly = (4000 * 500 - 6000 * 1000) / 500 = (2_000_000 - 6_000_000) / 500 < 0
    let kelly = calculate_kelly_fraction(4000, 500, 1000);
    assert_eq!(kelly, 0);
}

#[test]
fn test_kelly_positive_expectancy() {
    // Win 60%, avg win = 1000 bps, avg loss = 500 bps
    // kelly = (6000 * 1000 - 4000 * 500) / 1000 = (6_000_000 - 2_000_000) / 1000 = 4000 bps
    let kelly = calculate_kelly_fraction(6000, 1000, 500);
    assert_eq!(kelly, 4000);
}

#[test]
fn test_kelly_clamped_to_10000() {
    // Extreme win rate should not produce > 10000 bps
    let kelly = calculate_kelly_fraction(9900, 5000, 100);
    assert!(kelly <= 10_000);
}

#[test]
fn test_kelly_even_odds_50pct() {
    // Win 50%, avg win = 1000, avg loss = 1000
    // kelly = (5000 * 1000 - 5000 * 1000) / 1000 = 0
    let kelly = calculate_kelly_fraction(5000, 1000, 1000);
    assert_eq!(kelly, 0);
}

// ---------------------------------------------------------------------------
// PositionSizingConfig storage
// ---------------------------------------------------------------------------

#[test]
fn test_default_config_stored_when_none_set() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        let config = get_sizing_config(&env, &user);
        assert_eq!(config, PositionSizingConfig::default());
    });
}

#[test]
fn test_set_and_get_sizing_config() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        let config = PositionSizingConfig {
            method: SizingMethod::Kelly,
            risk_per_trade_bps: 150,
            max_position_pct_bps: 1500,
            kelly_multiplier: 25,
            target_volatility_bps: 300,
            base_position_pct_bps: 800,
        };
        set_sizing_config(&env, &user, &config);
        let retrieved = get_sizing_config(&env, &user);
        assert_eq!(retrieved, config);
    });
}

#[test]
#[should_panic]
fn test_invalid_max_position_pct_rejected() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        let config = PositionSizingConfig {
            max_position_pct_bps: 15_000, // > 10000 — invalid
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);
    });
}

// ---------------------------------------------------------------------------
// FixedPercentage sizing
// ---------------------------------------------------------------------------

#[test]
fn test_fixed_pct_sizing_scales_inversely_with_volatility() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // Give user a portfolio of 10_000
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100); // 100 * 100 / 100 = 10000

        let config = PositionSizingConfig {
            method: SizingMethod::FixedPercentage,
            risk_per_trade_bps: 200, // 2%
            max_position_pct_bps: 5000,
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Asset with low volatility (200 bps)
        for p in &[100i128, 102, 100, 102, 100] {
            record_price(&env, 1, *p);
        }
        // Asset with high volatility (roughly 1000 bps)
        for p in &[100i128, 110, 90, 115, 85] {
            record_price(&env, 2, *p);
        }

        let rec_low = crate::position_sizing::calculate_position_size(
            &env, &user, 1, 0, 0, 0,
        ).unwrap();
        let rec_high = crate::position_sizing::calculate_position_size(
            &env, &user, 2, 0, 0, 0,
        ).unwrap();

        // Lower volatility → larger position
        assert!(
            rec_low.recommended_size >= rec_high.recommended_size,
            "low_vol_size={} should be >= high_vol_size={}",
            rec_low.recommended_size,
            rec_high.recommended_size
        );
    });
}

#[test]
fn test_fixed_pct_example_calculation() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // Portfolio = 10_000 units
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100);

        let config = PositionSizingConfig {
            method: SizingMethod::FixedPercentage,
            risk_per_trade_bps: 200,    // 2%
            max_position_pct_bps: 10_000, // no cap
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Manually inject a known volatility via recorded prices
        // 5% daily swings ≈ 500 bps volatility
        for p in &[100i128, 105, 100, 105, 100, 105] {
            record_price(&env, 3, *p);
        }
        let vol = calculate_volatility(&env, 3, 10);
        // Expected: portfolio(10_000) * risk(200) / vol
        // Expected size ≈ 10_000 * 200 / vol
        let expected_approx = 10_000 * 200 / vol;

        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 3, 0, 0, 0,
        ).unwrap();

        // Allow 1% tolerance due to integer math
        let diff = (rec.recommended_size - expected_approx).abs();
        assert!(
            diff <= expected_approx / 100 + 1,
            "expected ~{}, got {}",
            expected_approx,
            rec.recommended_size
        );
    });
}

// ---------------------------------------------------------------------------
// Kelly sizing
// ---------------------------------------------------------------------------

#[test]
fn test_kelly_sizing_with_good_stats() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // Portfolio = 10_000
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100);

        let config = PositionSizingConfig {
            method: SizingMethod::Kelly,
            kelly_multiplier: 50, // half-Kelly
            max_position_pct_bps: 5000,
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Win rate 60%, avg win 1000 bps, avg loss 500 bps → kelly_f = 4000 bps
        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 1, 6000, 1000, 500,
        ).unwrap();

        // size = 10_000 * 4000 * 50 / (10000 * 100) = 10000 * 200000 / 1000000 = 2000
        assert_eq!(rec.recommended_size, 2000);
    });
}

#[test]
fn test_kelly_sizing_negative_expectancy_returns_min() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 1000, 100);

        let config = PositionSizingConfig {
            method: SizingMethod::Kelly,
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Negative expectancy
        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 1, 3000, 500, 1000,
        ).unwrap();

        assert_eq!(rec.recommended_size, MIN_POSITION_SIZE);
    });
}

// ---------------------------------------------------------------------------
// VolatilityScaled sizing
// ---------------------------------------------------------------------------

#[test]
fn test_volatility_scaled_larger_when_vol_low() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100);

        let config = PositionSizingConfig {
            method: SizingMethod::VolatilityScaled,
            target_volatility_bps: 500,  // target 5%
            base_position_pct_bps: 1000, // 10% base
            max_position_pct_bps: 10_000,
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Low vol: prices barely move
        for p in &[100i128, 101, 100, 101, 100] {
            record_price(&env, 10, *p);
        }
        // High vol: prices swing wildly
        for p in &[100i128, 120, 80, 125, 75] {
            record_price(&env, 11, *p);
        }

        let low = crate::position_sizing::calculate_position_size(
            &env, &user, 10, 0, 0, 0,
        ).unwrap();
        let high = crate::position_sizing::calculate_position_size(
            &env, &user, 11, 0, 0, 0,
        ).unwrap();

        // When actual vol < target vol → scaled up → larger position
        assert!(
            low.recommended_size >= high.recommended_size,
            "low_vol={} high_vol={}",
            low.recommended_size,
            high.recommended_size
        );
    });
}

#[test]
fn test_volatility_scaled_exact_calculation() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // Portfolio = 10_000
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100);

        let config = PositionSizingConfig {
            method: SizingMethod::VolatilityScaled,
            target_volatility_bps: 500,   // 5%
            base_position_pct_bps: 1000,  // 10% base → base_size = 1000
            max_position_pct_bps: 10_000,
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Record prices that give a known volatility
        for p in &[100i128, 105, 100, 105, 100, 105] {
            record_price(&env, 20, *p);
        }
        let actual_vol = calculate_volatility(&env, 20, 10);

        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 20, 0, 0, 0,
        ).unwrap();

        // base_size = 10_000 * 1000 / 10_000 = 1000
        // expected = 1000 * 500 / actual_vol
        let expected = 1000 * 500 / actual_vol;
        let diff = (rec.recommended_size - expected).abs();
        assert!(
            diff <= expected / 100 + 1,
            "expected ~{}, got {}",
            expected,
            rec.recommended_size
        );
    });
}

// ---------------------------------------------------------------------------
// Max size cap tests
// ---------------------------------------------------------------------------

#[test]
fn test_max_position_cap_enforced() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // Portfolio = 10_000
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100);

        let config = PositionSizingConfig {
            method: SizingMethod::FixedPercentage,
            risk_per_trade_bps: 200,
            max_position_pct_bps: 500, // max 5% = 500
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Very low volatility → raw size would exceed cap
        for _ in 0..10 {
            record_price(&env, 30, 100_000);
        }
        // vol = 0 here so MAX_VOLATILITY_BPS is used
        // raw = 10000 * 200 / 10000 = 200 — already under cap in this case
        // Let's force a scenario with extremely small volatility manually
        // by giving tiny price movement
        for p in &[1_000_000i128, 1_000_001] {
            record_price(&env, 31, *p);
        }

        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 31, 0, 0, 0,
        ).unwrap();

        assert!(
            rec.recommended_size <= rec.max_size,
            "recommended={} must be <= max={}",
            rec.recommended_size,
            rec.max_size
        );
    });
}

#[test]
fn test_was_capped_flag_set_when_size_exceeds_max() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // Large portfolio, tight cap
        set_asset_price(&env, 99, 1000);
        update_position(&env, &user, 99, 1000, 1000); // portfolio = 10_000

        let config = PositionSizingConfig {
            method: SizingMethod::VolatilityScaled,
            target_volatility_bps: 2000,  // target 20%
            base_position_pct_bps: 5000,  // 50% base
            max_position_pct_bps: 100,    // cap at 1% — forces capping
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        // Very low volatility so the scaled size will be huge
        for p in &[100i128, 101, 100] {
            record_price(&env, 40, *p);
        }

        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 40, 0, 0, 0,
        ).unwrap();

        // The recommended size should equal max_size and was_capped = true
        assert_eq!(rec.recommended_size, rec.max_size);
        assert!(rec.was_capped);
    });
}

// ---------------------------------------------------------------------------
// Edge case tests
// ---------------------------------------------------------------------------

#[test]
fn test_zero_portfolio_returns_min_position() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        // No positions → portfolio_value = 0
        for p in &[100i128, 110, 100] {
            record_price(&env, 1, *p);
        }

        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 1, 6000, 1000, 500,
        ).unwrap();

        assert_eq!(rec.recommended_size, MIN_POSITION_SIZE);
    });
}

#[test]
fn test_zero_volatility_assigns_max_volatility_floor() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 10000, 100);

        // Constant prices → zero volatility
        for _ in 0..5 {
            record_price(&env, 50, 100_000);
        }

        let config = PositionSizingConfig {
            method: SizingMethod::FixedPercentage,
            risk_per_trade_bps: 200,
            max_position_pct_bps: 5000,
            ..PositionSizingConfig::default()
        };
        set_sizing_config(&env, &user, &config);

        let rec = crate::position_sizing::calculate_position_size(
            &env, &user, 50, 0, 0, 0,
        ).unwrap();

        // Should not panic, should return a sane (minimum) size
        assert!(rec.recommended_size >= MIN_POSITION_SIZE);
        // The volatility field should reflect MAX_VOLATILITY_BPS
        assert_eq!(rec.volatility_bps, MAX_VOLATILITY_BPS);
    });
}

#[test]
fn test_balance_cap_applied() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100); // portfolio = 10_000

        for p in &[100i128, 110, 100, 110] {
            record_price(&env, 1, *p);
        }

        let available = 50i128; // very tight balance
        let size = crate::position_sizing::get_position_size_for_trade(
            &env, &user, 1, 0, 0, 0, available,
        ).unwrap();

        assert!(size <= available);
        assert!(size >= MIN_POSITION_SIZE);
    });
}

// ---------------------------------------------------------------------------
// Public API tests (via contract client)
// ---------------------------------------------------------------------------

#[test]
fn test_public_get_set_sizing_config() {
    let env = setup_env();
    let contract = make_contract(&env);
    let client = crate::AutoTradeContractClient::new(&env, &contract);
    let user = soroban_sdk::Address::generate(&env);

    let config = PositionSizingConfig {
        method: SizingMethod::VolatilityScaled,
        risk_per_trade_bps: 150,
        max_position_pct_bps: 2000,
        kelly_multiplier: 25,
        target_volatility_bps: 400,
        base_position_pct_bps: 1000,
    };

    client.set_sizing_config(&user, &config);
    let got = client.get_sizing_config(&user);
    assert_eq!(got, config);
}

#[test]
fn test_public_record_price_and_get_volatility() {
    let env = setup_env();
    let contract = make_contract(&env);
    let client = crate::AutoTradeContractClient::new(&env, &contract);

    for p in &[100i128, 105, 100, 108, 95, 110] {
        client.record_asset_price(&42u32, p);
    }

    let vol = client.get_asset_volatility(&42u32, &10u32);
    assert!(vol > 0, "should have non-zero volatility after recording prices");
}

#[test]
fn test_public_get_sizing_recommendation() {
    let env = setup_env();
    let contract = make_contract(&env);
    let client = crate::AutoTradeContractClient::new(&env, &contract);
    let user = soroban_sdk::Address::generate(&env);

    // Record prices for the asset
    for p in &[100i128, 105, 100, 107, 98] {
        client.record_asset_price(&7u32, p);
    }

    // Give user a portfolio
    env.as_contract(&contract, || {
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 5000, 100);
    });

    let rec = client
        .get_sizing_recommendation(&user, &7u32, &0i128, &0i128, &0i128)
        .unwrap();

    assert!(rec.portfolio_value > 0);
    assert!(rec.recommended_size >= MIN_POSITION_SIZE);
    assert!(rec.max_size >= rec.recommended_size);
}

#[test]
fn test_public_get_price_history() {
    let env = setup_env();
    let contract = make_contract(&env);
    let client = crate::AutoTradeContractClient::new(&env, &contract);

    for p in &[1000i128, 1100, 1050, 1200] {
        client.record_asset_price(&8u32, p);
    }

    let hist = client.get_price_history(&8u32, &10u32);
    assert_eq!(hist.len(), 4);
}

// ---------------------------------------------------------------------------
// Multi-method comparison test
// ---------------------------------------------------------------------------

#[test]
fn test_all_methods_produce_valid_sizes() {
    let env = setup_env();
    let contract = make_contract(&env);
    let user = soroban_sdk::Address::generate(&env);
    env.as_contract(&contract, || {
        set_asset_price(&env, 99, 100);
        update_position(&env, &user, 99, 100_00, 100); // portfolio = 10_000

        for p in &[100i128, 105, 98, 107, 102] {
            record_price(&env, 77, *p);
        }

        for method in [
            SizingMethod::FixedPercentage,
            SizingMethod::Kelly,
            SizingMethod::VolatilityScaled,
        ] {
            let config = PositionSizingConfig {
                method,
                ..PositionSizingConfig::default()
            };
            set_sizing_config(&env, &user, &config);

            let rec = crate::position_sizing::calculate_position_size(
                &env, &user, 77, 6000, 1000, 400,
            ).unwrap();

            assert!(
                rec.recommended_size >= MIN_POSITION_SIZE,
                "method produced size below minimum"
            );
            assert!(
                rec.recommended_size <= rec.max_size,
                "method produced size above max"
            );
        }
    });
}