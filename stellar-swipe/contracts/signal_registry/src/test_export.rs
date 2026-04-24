#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, Env, String};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (Env, Address, SignalRegistryClient) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000); // fixed base timestamp

    #[allow(deprecated)]
    let contract_id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    (env, admin, client)
}

fn create_signal_now(
    env: &Env,
    client: &SignalRegistryClient,
    provider: &Address,
    pair: &str,
) -> u64 {
    let now = env.ledger().timestamp();
    client.create_signal(
        provider,
        &String::from_str(env, pair),
        &SignalAction::Buy,
        &1_000_000,
        &String::from_str(env, "Test rationale"),
        &(now + 3600),
    )
}

fn execute_trade(
    env: &Env,
    client: &SignalRegistryClient,
    signal_id: u64,
    executor: &Address,
    profit: bool,
) {
    let (entry, exit) = if profit {
        (100_000i128, 110_000i128)  // +10%
    } else {
        (100_000i128, 92_000i128)   // -8%
    };
    client.record_trade_execution(executor, &signal_id, &entry, &exit, &1_000_000);
    let _ = env;
}

// ---------------------------------------------------------------------------
// Byte helpers for assertions
// ---------------------------------------------------------------------------

fn bytes_contains(haystack: &soroban_sdk::Bytes, needle: &[u8]) -> bool {
    let len = haystack.len() as usize;
    let nlen = needle.len();
    if nlen == 0 || nlen > len {
        return false;
    }
    let mut h = alloc::vec![0u8; len];
    for i in 0..len {
        h[i] = haystack.get(i as u32).unwrap();
    }
    h.windows(nlen).any(|w| w == needle)
}

fn bytes_starts_with(haystack: &soroban_sdk::Bytes, prefix: &[u8]) -> bool {
    if haystack.len() < prefix.len() as u32 {
        return false;
    }
    for (i, &b) in prefix.iter().enumerate() {
        if haystack.get(i as u32).unwrap() != b {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Signal export — CSV
// ---------------------------------------------------------------------------

#[test]
fn test_export_signals_csv_header() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    let result = client.export_signals(&provider, &0, &None).unwrap();

    assert!(bytes_starts_with(
        &result,
        b"signal_id,timestamp,asset_pair,action,price,rationale,executions,total_roi,status\n"
    ));
}

#[test]
fn test_export_signals_csv_empty_returns_header_only() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    let result = client.export_signals(&provider, &0, &None).unwrap();
    // Should have header but no data rows — length equals header line
    let header =
        b"signal_id,timestamp,asset_pair,action,price,rationale,executions,total_roi,status\n";
    assert_eq!(result.len(), header.len() as u32);
}

#[test]
fn test_export_signals_csv_contains_signal_data() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    let sig_id = create_signal_now(&env, &client, &provider, "XLM/USDC");
    let executor = Address::generate(&env);
    execute_trade(&env, &client, sig_id, &executor, true);

    let result = client.export_signals(&provider, &0, &None).unwrap();

    // Must contain the asset pair
    assert!(bytes_contains(&result, b"XLM/USDC"));
    // Must contain BUY
    assert!(bytes_contains(&result, b"BUY"));
    // Must contain the signal id (1)
    assert!(bytes_contains(&result, b"1,"));
}

#[test]
fn test_export_signals_csv_multiple_signals() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    create_signal_now(&env, &client, &provider, "XLM/USDC");
    create_signal_now(&env, &client, &provider, "BTC/USDC");
    create_signal_now(&env, &client, &provider, "ETH/USDC");

    let result = client.export_signals(&provider, &0, &None).unwrap();

    assert!(bytes_contains(&result, b"XLM/USDC"));
    assert!(bytes_contains(&result, b"BTC/USDC"));
    assert!(bytes_contains(&result, b"ETH/USDC"));
}

#[test]
fn test_export_signals_csv_only_own_signals() {
    let (env, _admin, client) = setup();
    let provider_a = Address::generate(&env);
    let provider_b = Address::generate(&env);

    create_signal_now(&env, &client, &provider_a, "XLM/USDC");
    create_signal_now(&env, &client, &provider_b, "BTC/USDC");

    let result_a = client.export_signals(&provider_a, &0, &None).unwrap();
    let result_b = client.export_signals(&provider_b, &0, &None).unwrap();

    assert!(bytes_contains(&result_a, b"XLM/USDC"));
    assert!(!bytes_contains(&result_a, b"BTC/USDC"));

    assert!(bytes_contains(&result_b, b"BTC/USDC"));
    assert!(!bytes_contains(&result_b, b"XLM/USDC"));
}

// ---------------------------------------------------------------------------
// Signal export — JSON
// ---------------------------------------------------------------------------

#[test]
fn test_export_signals_json_empty_returns_array() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    let result = client.export_signals(&provider, &1, &None).unwrap();

    assert!(bytes_starts_with(&result, b"["));
    assert_eq!(result.get(result.len() - 1).unwrap(), b']');
}

#[test]
fn test_export_signals_json_contains_fields() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    create_signal_now(&env, &client, &provider, "XLM/USDC");

    let result = client.export_signals(&provider, &1, &None).unwrap();

    assert!(bytes_contains(&result, b"signal_id"));
    assert!(bytes_contains(&result, b"asset_pair"));
    assert!(bytes_contains(&result, b"action"));
    assert!(bytes_contains(&result, b"XLM/USDC"));
    assert!(bytes_contains(&result, b"total_roi_pct"));
    assert!(bytes_contains(&result, b"status"));
}

// ---------------------------------------------------------------------------
// Trade export
// ---------------------------------------------------------------------------

#[test]
fn test_export_trades_csv_header() {
    let (env, _admin, client) = setup();
    let executor = Address::generate(&env);

    let result = client.export_trades(&executor, &0, &None).unwrap();

    assert!(bytes_starts_with(
        &result,
        b"trade_id,timestamp,signal_id,asset_pair,volume,entry_price,exit_price,roi_bps,pnl\n"
    ));
}

#[test]
fn test_export_trades_csv_empty_returns_header_only() {
    let (env, _admin, client) = setup();
    let executor = Address::generate(&env);

    let result = client.export_trades(&executor, &0, &None).unwrap();
    let header =
        b"trade_id,timestamp,signal_id,asset_pair,volume,entry_price,exit_price,roi_bps,pnl\n";
    assert_eq!(result.len(), header.len() as u32);
}

#[test]
fn test_export_trades_csv_contains_trade_data() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    let sig_id = create_signal_now(&env, &client, &provider, "XLM/USDC");
    execute_trade(&env, &client, sig_id, &executor, true);

    let result = client.export_trades(&executor, &0, &None).unwrap();

    assert!(bytes_contains(&result, b"XLM/USDC"));
    assert!(bytes_contains(&result, b"1000000")); // volume
}

#[test]
fn test_export_trades_json_structure() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    let sig_id = create_signal_now(&env, &client, &provider, "XLM/USDC");
    execute_trade(&env, &client, sig_id, &executor, true);

    let result = client.export_trades(&executor, &1, &None).unwrap();

    assert!(bytes_starts_with(&result, b"["));
    assert!(bytes_contains(&result, b"trade_id"));
    assert!(bytes_contains(&result, b"roi_bps"));
    assert!(bytes_contains(&result, b"roi_pct"));
    assert!(bytes_contains(&result, b"pnl"));
}

#[test]
fn test_export_trades_only_own_trades() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor_a = Address::generate(&env);
    let executor_b = Address::generate(&env);

    let sig1 = create_signal_now(&env, &client, &provider, "XLM/USDC");
    let sig2 = create_signal_now(&env, &client, &provider, "BTC/USDC");

    execute_trade(&env, &client, sig1, &executor_a, true);
    execute_trade(&env, &client, sig2, &executor_b, false);

    let result_a = client.export_trades(&executor_a, &0, &None).unwrap();
    let result_b = client.export_trades(&executor_b, &0, &None).unwrap();

    // executor_a only traded XLM/USDC
    assert!(bytes_contains(&result_a, b"XLM/USDC"));
    assert!(!bytes_contains(&result_a, b"BTC/USDC"));

    // executor_b only traded BTC/USDC
    assert!(bytes_contains(&result_b, b"BTC/USDC"));
    assert!(!bytes_contains(&result_b, b"XLM/USDC"));
}

// ---------------------------------------------------------------------------
// Performance export
// ---------------------------------------------------------------------------

#[test]
fn test_export_performance_json_fields() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    create_signal_now(&env, &client, &provider, "XLM/USDC");

    let result = client.export_performance(&provider, &1, &None).unwrap();

    assert!(bytes_contains(&result, b"total_signals"));
    assert!(bytes_contains(&result, b"successful_signals"));
    assert!(bytes_contains(&result, b"failed_signals"));
    assert!(bytes_contains(&result, b"success_rate"));
    assert!(bytes_contains(&result, b"total_roi_pct"));
    assert!(bytes_contains(&result, b"total_volume"));
    assert!(bytes_contains(&result, b"best_pair"));
    assert!(bytes_contains(&result, b"worst_pair"));
    assert!(bytes_contains(&result, b"avg_signal_lifetime_hours"));
}

#[test]
fn test_export_performance_csv_fields() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    create_signal_now(&env, &client, &provider, "XLM/USDC");

    let result = client.export_performance(&provider, &0, &None).unwrap();

    assert!(bytes_starts_with(&result, b"metric,value\n"));
    assert!(bytes_contains(&result, b"total_signals"));
    assert!(bytes_contains(&result, b"success_rate"));
    assert!(bytes_contains(&result, b"total_roi_pct"));
}

#[test]
fn test_export_performance_correct_counts() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    // Create 3 signals, execute trades to drive to Successful/Failed
    let s1 = create_signal_now(&env, &client, &provider, "XLM/USDC");
    let s2 = create_signal_now(&env, &client, &provider, "BTC/USDC");
    let s3 = create_signal_now(&env, &client, &provider, "ETH/USDC");

    // s1: +10% → Successful
    client.record_trade_execution(&executor, &s1, &100_000, &110_000, &1_000_000);
    // s2: +5% → Successful
    client.record_trade_execution(&executor, &s2, &100_000, &105_000, &500_000);
    // s3: -8% → stays Active (above -5% threshold)
    client.record_trade_execution(&executor, &s3, &100_000, &92_000, &200_000);

    let result = client.export_performance(&provider, &1, &None).unwrap();

    // "total_signals":3
    assert!(bytes_contains(&result, b"\"total_signals\":3"));
    // At least 2 successful
    assert!(bytes_contains(&result, b"\"successful_signals\":2"));
}

// ---------------------------------------------------------------------------
// Portfolio export
// ---------------------------------------------------------------------------

#[test]
fn test_export_portfolio_json_fields() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    create_signal_now(&env, &client, &provider, "XLM/USDC");

    let result = client.export_portfolio(&provider, &None).unwrap();

    assert!(bytes_contains(&result, b"total_signals"));
    assert!(bytes_contains(&result, b"active_signals"));
    assert!(bytes_contains(&result, b"total_trades"));
    assert!(bytes_contains(&result, b"total_volume"));
    assert!(bytes_contains(&result, b"total_roi_pct"));
}

// ---------------------------------------------------------------------------
// Date range filtering
// ---------------------------------------------------------------------------

#[test]
fn test_export_signals_date_range_filters_correctly() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    let base_ts: u64 = 1_700_000_000;

    // Signal at base
    create_signal_now(&env, &client, &provider, "XLM/USDC");

    // Advance time by 10 days and create another signal
    env.ledger().set_timestamp(base_ts + 10 * 24 * 3600);
    create_signal_now(&env, &client, &provider, "BTC/USDC");

    // Export only signals from the first 5 days
    let range = (base_ts, base_ts + 5 * 24 * 3600);
    let result = client
        .export_signals(&provider, &0, &Some(range))
        .unwrap();

    // Only XLM/USDC should appear (created at base_ts)
    assert!(bytes_contains(&result, b"XLM/USDC"));
    assert!(!bytes_contains(&result, b"BTC/USDC"));
}

#[test]
fn test_export_signals_date_range_no_results_returns_header() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    create_signal_now(&env, &client, &provider, "XLM/USDC");

    // Range that doesn't include any signals (far future)
    let range = (9_000_000_000, 9_999_999_999);
    let result = client
        .export_signals(&provider, &0, &Some(range))
        .unwrap();

    // CSV returns header-only
    let header =
        b"signal_id,timestamp,asset_pair,action,price,rationale,executions,total_roi,status\n";
    assert_eq!(result.len(), header.len() as u32);
}

#[test]
fn test_export_trades_date_range_filters_correctly() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    let base_ts: u64 = 1_700_000_000;

    let sig1 = create_signal_now(&env, &client, &provider, "XLM/USDC");
    execute_trade(&env, &client, sig1, &executor, true); // trade at base_ts

    // Advance 20 days and do another trade
    env.ledger().set_timestamp(base_ts + 20 * 24 * 3600);
    let sig2 = create_signal_now(&env, &client, &provider, "BTC/USDC");
    execute_trade(&env, &client, sig2, &executor, false);

    // Export only first 10 days
    let range = (base_ts, base_ts + 10 * 24 * 3600);
    let result = client.export_trades(&executor, &0, &Some(range)).unwrap();

    assert!(bytes_contains(&result, b"XLM/USDC"));
    assert!(!bytes_contains(&result, b"BTC/USDC"));
}

// ---------------------------------------------------------------------------
// Unsupported format
// ---------------------------------------------------------------------------

#[test]
fn test_export_unsupported_format_returns_error() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);

    let result = client.try_export_signals(&provider, &99, &None);
    assert!(result.is_err());

    let result = client.try_export_trades(&provider, &99, &None);
    assert!(result.is_err());

    let result = client.try_export_performance(&provider, &99, &None);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// ROI and PnL correctness
// ---------------------------------------------------------------------------

#[test]
fn test_export_trades_pnl_calculation() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    let sig = create_signal_now(&env, &client, &provider, "XLM/USDC");
    // Buy at 100_000, exit at 110_000 = +10% = +1000 bps
    // Volume = 1_000_000
    // PnL = 1_000_000 * 1000 / 10000 = 100_000
    client.record_trade_execution(&executor, &sig, &100_000, &110_000, &1_000_000);

    let result = client.export_trades(&executor, &1, &None).unwrap();

    assert!(bytes_contains(&result, b"\"roi_bps\":1000"));
    assert!(bytes_contains(&result, b"\"pnl\":100000"));
}

#[test]
fn test_export_signals_roi_format() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    let sig = create_signal_now(&env, &client, &provider, "XLM/USDC");
    // +5% = +500 bps
    client.record_trade_execution(&executor, &sig, &100_000, &105_000, &1_000_000);

    let result = client.export_signals(&provider, &0, &None).unwrap();

    // CSV should contain "+5.00%"
    assert!(bytes_contains(&result, b"+5.00%"));
}

// ---------------------------------------------------------------------------
// End-to-end workflow — 10 signals + trades
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_10_signals_full_export() {
    let (env, _admin, client) = setup();
    let provider = Address::generate(&env);
    let executor = Address::generate(&env);

    let pairs = [
        "XLM/USDC", "BTC/USDC", "ETH/USDC", "SOL/USDC", "ADA/USDC",
        "DOT/USDC", "AVAX/USDC", "MATIC/USDC", "LINK/USDC", "UNI/USDC",
    ];

    for (i, pair) in pairs.iter().enumerate() {
        let sig = create_signal_now(&env, &client, &provider, pair);
        // Alternate profit/loss
        execute_trade(&env, &client, sig, &executor, i % 2 == 0);
    }

    // Signal CSV export
    let sig_csv = client.export_signals(&provider, &0, &None).unwrap();
    for pair in &pairs {
        assert!(bytes_contains(&sig_csv, pair.as_bytes()));
    }

    // Trade CSV export
    let trade_csv = client.export_trades(&executor, &0, &None).unwrap();
    // All 10 trades should be in the file
    for pair in &pairs {
        assert!(bytes_contains(&trade_csv, pair.as_bytes()));
    }

    // Performance JSON
    let perf_json = client.export_performance(&provider, &1, &None).unwrap();
    assert!(bytes_contains(&perf_json, b"\"total_signals\":10"));
    assert!(bytes_contains(&perf_json, b"\"total_trades\":10"));

    // Portfolio JSON
    let portfolio = client.export_portfolio(&provider, &None).unwrap();
    assert!(bytes_contains(&portfolio, b"\"total_signals\":10"));
}