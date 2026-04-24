extern crate alloc;

use alloc::string::{String as RustString, ToString};
use alloc::vec::Vec as RustVec;
use soroban_sdk::{Address, Bytes, Env, Map};

use crate::errors::ExportError;
use crate::types::{Signal, SignalAction, SignalStatus, TradeExecution};
use crate::StorageKey;
use stellar_swipe_common::{SECONDS_PER_30_DAY_MONTH, SECONDS_PER_DAY, SECONDS_PER_WEEK};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum records in a single export to prevent runaway gas usage.
const MAX_EXPORT_RECORDS: u32 = 500;

/// 7 days in seconds
pub const PRESET_7_DAYS: u64 = SECONDS_PER_WEEK;
/// 30 days in seconds
pub const PRESET_30_DAYS: u64 = SECONDS_PER_30_DAY_MONTH;
/// 365 days in seconds
pub const PRESET_365_DAYS: u64 = 365 * SECONDS_PER_DAY;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ExportFormat {
    Csv,
    Json,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportEntity {
    Signals,
    Trades,
    Performance,
    Portfolio,
}

/// Date range filter (start_ts, end_ts) inclusive, both in Unix seconds UTC.
pub type DateRange = (u64, u64);

// ---------------------------------------------------------------------------
// CSV / JSON helpers (no_std compatible using alloc)
// ---------------------------------------------------------------------------

fn i128_to_str(v: i128) -> RustString {
    v.to_string()
}

fn u64_to_str(v: u64) -> RustString {
    v.to_string()
}

fn u32_to_str(v: u32) -> RustString {
    v.to_string()
}

/// Format basis-point ROI as "+X.XX%" or "-X.XX%"
fn bps_to_pct_str(bps: i128) -> RustString {
    let sign = if bps >= 0 { "+" } else { "-" };
    let abs = bps.unsigned_abs();
    let whole = abs / 100;
    let frac = abs % 100;
    let mut s = RustString::from(sign);
    s.push_str(&whole.to_string());
    s.push('.');
    if frac < 10 {
        s.push('0');
    }
    s.push_str(&frac.to_string());
    s.push('%');
    s
}

fn signal_status_str(status: &SignalStatus) -> &'static str {
    match status {
        SignalStatus::Pending => "Pending",
        SignalStatus::Active => "Active",
        SignalStatus::Executed => "Executed",
        SignalStatus::Expired => "Expired",
        SignalStatus::Successful => "Successful",
        SignalStatus::Failed => "Failed",
    }
}

fn signal_action_str(action: &SignalAction) -> &'static str {
    match action {
        SignalAction::Buy => "BUY",
        SignalAction::Sell => "SELL",
    }
}

/// Escape a string for CSV (wrap in quotes if it contains comma/newline/quote).
fn csv_escape(s: &str) -> RustString {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let mut out = RustString::from('"');
        for c in s.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        RustString::from(s)
    }
}

/// Convert a native Soroban `String` to a Rust `String`.
fn sdk_str_to_rust(s: &soroban_sdk::String) -> RustString {
    let bytes = s.to_array::<512>().unwrap_or([0u8; 512]);
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(512);
    core::str::from_utf8(&bytes[..len])
        .unwrap_or("")
        .to_string()
}

/// Append a `RustString` to a `RustVec<u8>`.
fn push_str(buf: &mut RustVec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
}

/// Convert a `RustVec<u8>` to a Soroban `Bytes`.
fn vec_to_bytes(env: &Env, v: &RustVec<u8>) -> Bytes {
    Bytes::from_slice(env, v)
}

// ---------------------------------------------------------------------------
// Storage helpers — trade executions
// ---------------------------------------------------------------------------

/// Return all `TradeExecution` records for an executor.
pub fn get_executor_trades(env: &Env, executor: &Address) -> alloc::vec::Vec<TradeExecution> {
    let map: Map<u64, TradeExecution> = env
        .storage()
        .instance()
        .get(&StorageKey::TradeExecutions)
        .unwrap_or(Map::new(env));

    let mut trades = alloc::vec::Vec::new();
    for i in 0..map.len() {
        if let Some(key) = map.keys().get(i) {
            if let Some(trade) = map.get(key) {
                if trade.executor == *executor {
                    trades.push(trade);
                }
            }
        }
    }
    trades
}

/// Return all `TradeExecution` records for signals owned by a provider.
pub fn get_provider_trades(env: &Env, provider: &Address) -> alloc::vec::Vec<TradeExecution> {
    let signals_map: Map<u64, Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    let trades_map: Map<u64, TradeExecution> = env
        .storage()
        .instance()
        .get(&StorageKey::TradeExecutions)
        .unwrap_or(Map::new(env));

    let mut result = alloc::vec::Vec::new();
    for i in 0..trades_map.len() {
        if let Some(key) = trades_map.keys().get(i) {
            if let Some(trade) = trades_map.get(key) {
                // Include if the signal belongs to this provider
                if let Some(signal) = signals_map.get(trade.signal_id) {
                    if signal.provider == *provider {
                        result.push(trade);
                    }
                }
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Signal export
// ---------------------------------------------------------------------------

fn collect_provider_signals(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> alloc::vec::Vec<Signal> {
    let map: Map<u64, Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    let mut out = alloc::vec::Vec::new();
    for i in 0..map.len() {
        if let Some(key) = map.keys().get(i) {
            if let Some(signal) = map.get(key) {
                if signal.provider != *provider {
                    continue;
                }
                if let Some((start, end)) = date_range {
                    if signal.timestamp < start || signal.timestamp > end {
                        continue;
                    }
                }
                out.push(signal);
                if out.len() as u32 >= MAX_EXPORT_RECORDS {
                    break;
                }
            }
        }
    }
    out
}

pub fn export_signals_csv(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let signals = collect_provider_signals(env, provider, date_range);

    let mut buf: RustVec<u8> = RustVec::new();
    // Header
    push_str(
        &mut buf,
        "signal_id,timestamp,asset_pair,action,price,rationale,executions,total_roi,status\n",
    );

    for signal in &signals {
        let asset_pair = sdk_str_to_rust(&signal.asset_pair);
        let rationale = sdk_str_to_rust(&signal.rationale);
        let avg_roi = if signal.executions > 0 {
            signal.total_roi / signal.executions as i128
        } else {
            0
        };

        let row = alloc::format!(
            "{},{},{},{},{},{},{},{},{}\n",
            u64_to_str(signal.id),
            u64_to_str(signal.timestamp),
            csv_escape(&asset_pair),
            signal_action_str(&signal.action),
            i128_to_str(signal.price),
            csv_escape(&rationale),
            u32_to_str(signal.executions),
            bps_to_pct_str(avg_roi),
            signal_status_str(&signal.status),
        );
        push_str(&mut buf, &row);
    }

    Ok(vec_to_bytes(env, &buf))
}

pub fn export_signals_json(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let signals = collect_provider_signals(env, provider, date_range);

    let mut buf: RustVec<u8> = RustVec::new();
    push_str(&mut buf, "[");

    for (idx, signal) in signals.iter().enumerate() {
        if idx > 0 {
            push_str(&mut buf, ",");
        }
        let asset_pair = sdk_str_to_rust(&signal.asset_pair);
        let rationale = sdk_str_to_rust(&signal.rationale);
        let avg_roi = if signal.executions > 0 {
            signal.total_roi / signal.executions as i128
        } else {
            0
        };

        let entry = alloc::format!(
            r#"{{"signal_id":{},"timestamp":{},"asset_pair":"{}","action":"{}","price":{},"rationale":"{}","executions":{},"avg_roi_bps":{},"total_roi_pct":"{}","status":"{}"}}"#,
            signal.id,
            signal.timestamp,
            asset_pair.replace('"', "\\\""),
            signal_action_str(&signal.action),
            signal.price,
            rationale.replace('"', "\\\""),
            signal.executions,
            avg_roi,
            bps_to_pct_str(avg_roi),
            signal_status_str(&signal.status),
        );
        push_str(&mut buf, &entry);
    }

    push_str(&mut buf, "]");
    Ok(vec_to_bytes(env, &buf))
}

// ---------------------------------------------------------------------------
// Trade export
// ---------------------------------------------------------------------------

fn collect_trades(
    env: &Env,
    executor: &Address,
    date_range: Option<DateRange>,
) -> alloc::vec::Vec<(u64, TradeExecution, Signal)> {
    let signals_map: Map<u64, Signal> = env
        .storage()
        .instance()
        .get(&StorageKey::Signals)
        .unwrap_or(Map::new(env));

    let trades_map: Map<u64, TradeExecution> = env
        .storage()
        .instance()
        .get(&StorageKey::TradeExecutions)
        .unwrap_or(Map::new(env));

    let mut out = alloc::vec::Vec::new();
    for i in 0..trades_map.len() {
        if let Some(trade_id) = trades_map.keys().get(i) {
            if let Some(trade) = trades_map.get(trade_id) {
                if trade.executor != *executor {
                    continue;
                }
                if let Some((start, end)) = date_range {
                    if trade.timestamp < start || trade.timestamp > end {
                        continue;
                    }
                }
                if let Some(signal) = signals_map.get(trade.signal_id) {
                    out.push((trade_id, trade, signal));
                    if out.len() as u32 >= MAX_EXPORT_RECORDS {
                        break;
                    }
                }
            }
        }
    }
    out
}

pub fn export_trades_csv(
    env: &Env,
    executor: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let trades = collect_trades(env, executor, date_range);

    let mut buf: RustVec<u8> = RustVec::new();
    push_str(
        &mut buf,
        "trade_id,timestamp,signal_id,asset_pair,volume,entry_price,exit_price,roi_bps,pnl\n",
    );

    for (trade_id, trade, signal) in &trades {
        let asset_pair = sdk_str_to_rust(&signal.asset_pair);
        // PnL = volume * roi / 10000
        let pnl = trade
            .volume
            .checked_mul(trade.roi)
            .unwrap_or(i128::MAX)
            .checked_div(10000)
            .unwrap_or(0);

        let row = alloc::format!(
            "{},{},{},{},{},{},{},{},{}\n",
            trade_id,
            trade.timestamp,
            trade.signal_id,
            csv_escape(&asset_pair),
            trade.volume,
            trade.entry_price,
            trade.exit_price,
            trade.roi,
            pnl,
        );
        push_str(&mut buf, &row);
    }

    Ok(vec_to_bytes(env, &buf))
}

pub fn export_trades_json(
    env: &Env,
    executor: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let trades = collect_trades(env, executor, date_range);

    let mut buf: RustVec<u8> = RustVec::new();
    push_str(&mut buf, "[");

    for (idx, (trade_id, trade, signal)) in trades.iter().enumerate() {
        if idx > 0 {
            push_str(&mut buf, ",");
        }
        let asset_pair = sdk_str_to_rust(&signal.asset_pair);
        let pnl = trade
            .volume
            .checked_mul(trade.roi)
            .unwrap_or(i128::MAX)
            .checked_div(10000)
            .unwrap_or(0);

        let entry = alloc::format!(
            r#"{{"trade_id":{},"timestamp":{},"signal_id":{},"asset_pair":"{}","volume":{},"entry_price":{},"exit_price":{},"roi_bps":{},"roi_pct":"{}","pnl":{}}}"#,
            trade_id,
            trade.timestamp,
            trade.signal_id,
            asset_pair.replace('"', "\\\""),
            trade.volume,
            trade.entry_price,
            trade.exit_price,
            trade.roi,
            bps_to_pct_str(trade.roi),
            pnl,
        );
        push_str(&mut buf, &entry);
    }

    push_str(&mut buf, "]");
    Ok(vec_to_bytes(env, &buf))
}

// ---------------------------------------------------------------------------
// Performance summary export
// ---------------------------------------------------------------------------

pub struct PerformanceSummary {
    pub total_signals: u32,
    pub successful_signals: u32,
    pub failed_signals: u32,
    pub success_rate_bps: u32,
    pub total_roi_bps: i128,
    pub total_volume: i128,
    pub total_trades: u32,
    pub best_pair: RustString,
    pub worst_pair: RustString,
    pub avg_signal_lifetime_secs: u64,
}

fn calculate_performance_summary(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> PerformanceSummary {
    let signals = collect_provider_signals(env, provider, date_range);

    let total_signals = signals.len() as u32;
    let mut successful_signals: u32 = 0;
    let mut failed_signals: u32 = 0;
    let mut total_roi_bps: i128 = 0;
    let mut total_volume: i128 = 0;
    let mut total_lifetime_secs: u64 = 0;
    let mut total_trades: u32 = 0;

    // Track ROI per asset pair
    let mut pair_roi: alloc::collections::BTreeMap<RustString, (i128, u32)> =
        alloc::collections::BTreeMap::new();

    for signal in &signals {
        if matches!(signal.status, SignalStatus::Successful) {
            successful_signals += 1;
        }
        if matches!(signal.status, SignalStatus::Failed) {
            failed_signals += 1;
        }

        let avg_roi = if signal.executions > 0 {
            signal.total_roi / signal.executions as i128
        } else {
            0
        };

        total_roi_bps = total_roi_bps.saturating_add(avg_roi);
        total_volume = total_volume.saturating_add(signal.total_volume);
        total_lifetime_secs =
            total_lifetime_secs.saturating_add(signal.expiry.saturating_sub(signal.timestamp));
        total_trades = total_trades.saturating_add(signal.executions);

        let pair_key = sdk_str_to_rust(&signal.asset_pair);
        let entry = pair_roi.entry(pair_key).or_insert((0i128, 0u32));
        entry.0 = entry.0.saturating_add(avg_roi);
        entry.1 = entry.1.saturating_add(1);
    }

    let success_rate_bps = if total_signals > 0 {
        ((successful_signals as u64 * 10000) / total_signals as u64) as u32
    } else {
        0
    };

    let avg_signal_lifetime_secs = if total_signals > 0 {
        total_lifetime_secs / total_signals as u64
    } else {
        0
    };

    // Determine best / worst pair by average ROI
    let mut best_pair = RustString::from("N/A");
    let mut worst_pair = RustString::from("N/A");
    let mut best_roi = i128::MIN;
    let mut worst_roi = i128::MAX;

    for (pair, (roi_sum, count)) in &pair_roi {
        if *count == 0 {
            continue;
        }
        let avg = roi_sum / *count as i128;
        if avg > best_roi {
            best_roi = avg;
            best_pair = pair.clone();
        }
        if avg < worst_roi {
            worst_roi = avg;
            worst_pair = pair.clone();
        }
    }

    PerformanceSummary {
        total_signals,
        successful_signals,
        failed_signals,
        success_rate_bps,
        total_roi_bps,
        total_volume,
        total_trades,
        best_pair,
        worst_pair,
        avg_signal_lifetime_secs,
    }
}

pub fn export_performance_json(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let s = calculate_performance_summary(env, provider, date_range);

    let sr_whole = s.success_rate_bps / 100;
    let sr_frac = s.success_rate_bps % 100;
    let success_rate_str = alloc::format!("{}.{:02}%", sr_whole, sr_frac);

    let avg_lifetime_hours = s.avg_signal_lifetime_secs / 3600;

    let json = alloc::format!(
        r#"{{"total_signals":{},"successful_signals":{},"failed_signals":{},"success_rate":"{}","total_roi_bps":{},"total_roi_pct":"{}","total_volume":{},"total_trades":{},"best_pair":"{}","worst_pair":"{}","avg_signal_lifetime_hours":{}}}"#,
        s.total_signals,
        s.successful_signals,
        s.failed_signals,
        success_rate_str,
        s.total_roi_bps,
        bps_to_pct_str(s.total_roi_bps),
        s.total_volume,
        s.total_trades,
        s.best_pair.replace('"', "\\\""),
        s.worst_pair.replace('"', "\\\""),
        avg_lifetime_hours,
    );

    let mut buf: RustVec<u8> = RustVec::new();
    push_str(&mut buf, &json);
    Ok(vec_to_bytes(env, &buf))
}

pub fn export_performance_csv(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let s = calculate_performance_summary(env, provider, date_range);

    let sr_whole = s.success_rate_bps / 100;
    let sr_frac = s.success_rate_bps % 100;
    let success_rate_str = alloc::format!("{}.{:02}%", sr_whole, sr_frac);

    let mut buf: RustVec<u8> = RustVec::new();
    push_str(&mut buf, "metric,value\n");

    let rows = [
        alloc::format!("total_signals,{}\n", s.total_signals),
        alloc::format!("successful_signals,{}\n", s.successful_signals),
        alloc::format!("failed_signals,{}\n", s.failed_signals),
        alloc::format!("success_rate,{}\n", success_rate_str),
        alloc::format!("total_roi_bps,{}\n", s.total_roi_bps),
        alloc::format!("total_roi_pct,{}\n", bps_to_pct_str(s.total_roi_bps)),
        alloc::format!("total_volume,{}\n", s.total_volume),
        alloc::format!("total_trades,{}\n", s.total_trades),
        alloc::format!("best_pair,{}\n", csv_escape(&s.best_pair)),
        alloc::format!("worst_pair,{}\n", csv_escape(&s.worst_pair)),
        alloc::format!(
            "avg_signal_lifetime_hours,{}\n",
            s.avg_signal_lifetime_secs / 3600
        ),
    ];

    for row in &rows {
        push_str(&mut buf, row);
    }

    Ok(vec_to_bytes(env, &buf))
}

// ---------------------------------------------------------------------------
// Portfolio export
// ---------------------------------------------------------------------------

pub fn export_portfolio_json(
    env: &Env,
    provider: &Address,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    let signals = collect_provider_signals(env, provider, date_range);
    let trades = get_provider_trades(env, provider);

    let total_volume: i128 = signals.iter().map(|s| s.total_volume).sum();
    let total_roi_bps: i128 = signals
        .iter()
        .map(|s| {
            if s.executions > 0 {
                s.total_roi / s.executions as i128
            } else {
                0
            }
        })
        .sum();
    let total_trades = trades.len() as u32;
    let active_signals = signals
        .iter()
        .filter(|s| matches!(s.status, SignalStatus::Active))
        .count() as u32;

    let json = alloc::format!(
        r#"{{"total_signals":{},"active_signals":{},"total_trades":{},"total_volume":{},"total_roi_bps":{},"total_roi_pct":"{}"}}"#,
        signals.len(),
        active_signals,
        total_trades,
        total_volume,
        total_roi_bps,
        bps_to_pct_str(total_roi_bps),
    );

    let mut buf: RustVec<u8> = RustVec::new();
    push_str(&mut buf, &json);
    Ok(vec_to_bytes(env, &buf))
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

pub fn export_data(
    env: &Env,
    requester: &Address,
    entity: ExportEntity,
    format: ExportFormat,
    date_range: Option<DateRange>,
) -> Result<Bytes, ExportError> {
    match (entity, format) {
        (ExportEntity::Signals, ExportFormat::Csv) => {
            export_signals_csv(env, requester, date_range)
        }
        (ExportEntity::Signals, ExportFormat::Json) => {
            export_signals_json(env, requester, date_range)
        }
        (ExportEntity::Trades, ExportFormat::Csv) => export_trades_csv(env, requester, date_range),
        (ExportEntity::Trades, ExportFormat::Json) => {
            export_trades_json(env, requester, date_range)
        }
        (ExportEntity::Performance, ExportFormat::Csv) => {
            export_performance_csv(env, requester, date_range)
        }
        (ExportEntity::Performance, ExportFormat::Json) => {
            export_performance_json(env, requester, date_range)
        }
        (ExportEntity::Portfolio, ExportFormat::Json) => {
            export_portfolio_json(env, requester, date_range)
        }
        (ExportEntity::Portfolio, ExportFormat::Csv) => {
            // Portfolio makes most sense as JSON; CSV is a flat summary
            export_portfolio_json(env, requester, date_range)
        }
    }
}
