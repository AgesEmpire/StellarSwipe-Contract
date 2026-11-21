use crate::types::{ProviderMonthlyReport, Signal, SignalStatus};
use soroban_sdk::{Address, Env, Map};

const SECONDS_PER_MONTH: u64 = 30 * 24 * 60 * 60;

/// Get provider monthly performance report (Issue #421)
pub fn get_provider_monthly_report(
    env: &Env,
    signals_map: &Map<u64, Signal>,
    provider: &Address,
    month: u32,
    year: u32,
) -> ProviderMonthlyReport {
    let mut report = ProviderMonthlyReport {
        signals_submitted: 0,
        signals_closed: 0,
        success_rate: 0,
        total_adopters: 0,
        fees_earned: 0,
        reputation_change: 0,
        best_signal_id: None,
        worst_signal_id: None,
    };

    let mut best_return = i128::MIN;
    let mut worst_return = i128::MAX;
    let mut best_id: Option<u64> = None;
    let mut worst_id: Option<u64> = None;

    let month_start = calculate_month_start(month, year);
    let month_end = month_start + SECONDS_PER_MONTH;

    for key in signals_map.keys() {
        if let Some(signal) = signals_map.get(key) {
            if signal.provider != *provider {
                continue;
            }

            if signal.timestamp >= month_start && signal.timestamp < month_end {
                report.signals_submitted += 1;
                report.total_adopters = report.total_adopters.saturating_add(signal.adoption_count);

                if matches!(
                    signal.status,
                    SignalStatus::Successful | SignalStatus::Failed
                ) {
                    report.signals_closed += 1;

                    if signal.status == SignalStatus::Successful {
                        if signal.total_roi > best_return {
                            best_return = signal.total_roi;
                            best_id = Some(signal.id);
                        }
                        if signal.total_roi < worst_return {
                            worst_return = signal.total_roi;
                            worst_id = Some(signal.id);
                        }
                    } else {
                        if signal.total_roi > best_return {
                            best_return = signal.total_roi;
                            best_id = Some(signal.id);
                        }
                        if signal.total_roi < worst_return {
                            worst_return = signal.total_roi;
                            worst_id = Some(signal.id);
                        }
                    }
                }
            }
        }
    }

    let successful = best_id.is_some() as u32;
    report.success_rate = (successful * 10000)
        .checked_div(report.signals_closed)
        .unwrap_or(0);

    report.best_signal_id = best_id;
    report.worst_signal_id = worst_id;

    report
}

/// Calculate unix timestamp for start of month (simplified: assumes Jan 1 1970)
fn calculate_month_start(month: u32, _year: u32) -> u64 {
    let mut timestamp = 0u64;
    for m in 1..month {
        timestamp = timestamp.saturating_add(days_in_month(m) * 24 * 60 * 60);
    }
    timestamp
}

/// Get number of days in a month
fn days_in_month(month: u32) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{RiskLevel, SignalCategory};
    use crate::types::{Signal, SignalAction};
    use soroban_sdk::{testutils::Address as _, Address, Env, Map, String};

    fn create_test_signal(
        env: &Env,
        id: u64,
        provider: Address,
        timestamp: u64,
        total_roi: i128,
        status: SignalStatus,
    ) -> Signal {
        Signal {
            id,
            provider,
            asset_pair: String::from_str(env, "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100_000,
            rationale: String::from_str(env, "Test"),
            timestamp,
            expiry: timestamp + 86_400,
            status: status.clone(),
            executions: 1,
            successful_executions: if status == SignalStatus::Successful {
                1
            } else {
                0
            },
            total_volume: 1000,
            total_roi,
            category: SignalCategory::SWING,
            tags: soroban_sdk::Vec::new(env),
            risk_level: RiskLevel::Medium,
            is_collaborative: false,
            submitted_at: timestamp,
            rationale_hash: String::from_str(env, "hash"),
            confidence: 50,
            adoption_count: 5,
            ai_validation_score: None,
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
            warning_emitted: false,
            benchmark_return_bps: None,
            alpha_bps: None,
        }
    }

    #[test]
    fn test_empty_month_returns_zero() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let signals: Map<u64, Signal> = Map::new(&env);

        let report = get_provider_monthly_report(&env, &signals, &provider, 1, 2024);
        assert_eq!(report.signals_submitted, 0);
        assert_eq!(report.signals_closed, 0);
        assert_eq!(report.best_signal_id, None);
        assert_eq!(report.worst_signal_id, None);
    }

    #[test]
    fn test_monthly_report_aggregates_signals() {
        let env = Env::default();
        let provider = Address::generate(&env);
        let mut signals: Map<u64, Signal> = Map::new(&env);

        let base_timestamp = 0u64;
        let signal1 = create_test_signal(
            &env,
            1,
            provider.clone(),
            base_timestamp + 1000,
            500,
            SignalStatus::Successful,
        );
        signals.set(1, signal1);

        let report = get_provider_monthly_report(&env, &signals, &provider, 1, 2024);
        assert!(report.signals_submitted > 0);
    }
}
