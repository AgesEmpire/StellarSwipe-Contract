use crate::types::{ProviderPerformance, Signal, SignalAction, SignalStatus, TradeExecution};

/// ROI calculation constants
const BASIS_POINTS_100_PERCENT: i128 = 10000;
const SUCCESS_THRESHOLD_BPS: i128 = 200; // 2% in basis points
const FAILURE_THRESHOLD_BPS: i128 = -500; // -5% in basis points
const MIN_ROI_BPS: i128 = -10000; // -100% cap

/// Calculate ROI in basis points from entry and exit prices
///
/// # Arguments
/// * `entry_price` - Entry price for the trade
/// * `exit_price` - Exit price for the trade
/// * `action` - Buy or Sell signal action
///
/// # Returns
/// ROI in basis points (10000 = 100%). Capped at -100% minimum.
///
/// # Panics
/// Panics if entry_price is 0 (division by zero)
pub fn calculate_roi(entry_price: i128, exit_price: i128, action: &SignalAction) -> i128 {
    if entry_price == 0 {
        panic!("entry price cannot be zero");
    }

    // Calculate price difference based on action
    let price_diff = match action {
        SignalAction::Buy => exit_price - entry_price,
        SignalAction::Sell => entry_price - exit_price, // Inverted for sell signals
    };

    // Calculate ROI: (price_diff / entry_price) * 10000
    let roi = price_diff
        .checked_mul(BASIS_POINTS_100_PERCENT)
        .expect("ROI calculation overflow")
        .checked_div(entry_price)
        .expect("division by zero in ROI calculation");

    // Cap negative ROI at -100%
    if roi < MIN_ROI_BPS {
        MIN_ROI_BPS
    } else {
        roi
    }
}

/// Update signal statistics with a new trade execution
///
/// # Arguments
/// * `signal` - Mutable reference to the signal to update
/// * `trade` - The trade execution details
pub fn update_signal_stats(signal: &mut Signal, trade: &TradeExecution) {
    // Increment execution count
    signal.executions = signal
        .executions
        .checked_add(1)
        .expect("executions overflow");

    // Increment successful validations if ROI > 0
    if trade.roi > 0 {
        signal.successful_executions = signal
            .successful_executions
            .checked_add(1)
            .expect("successful executions overflow");
    }

    // Add trade volume
    signal.total_volume = signal
        .total_volume
        .checked_add(trade.volume)
        .expect("total volume overflow");

    // Add trade ROI
    signal.total_roi = signal
        .total_roi
        .checked_add(trade.roi)
        .expect("total ROI overflow");
}

/// Evaluate signal status based on performance criteria
///
/// # Success/Failure Criteria:
/// - Successful: avg ROI > 2%
/// - Failed: avg ROI < -5% OR expired with 0 executions
/// - Active: Everything else
///
/// # Arguments
/// * `signal` - The signal to evaluate
/// * `now` - Current timestamp
///
/// # Returns
/// The appropriate signal status
pub fn evaluate_signal_status(signal: &Signal, now: u64) -> SignalStatus {
    // Check if signal expired with no executions -> Failed
    if signal.expiry < now && signal.executions == 0 {
        return SignalStatus::Failed;
    }

    // If no executions yet, maintain current status
    if signal.executions == 0 {
        return signal.status.clone();
    }

    // Calculate average ROI
    let avg_roi = signal.total_roi / (signal.executions as i128);

    // Evaluate against thresholds
    if avg_roi > SUCCESS_THRESHOLD_BPS {
        SignalStatus::Successful
    } else if avg_roi < FAILURE_THRESHOLD_BPS {
        SignalStatus::Failed
    } else {
        // Maintain Active status if within thresholds
        SignalStatus::Active
    }
}

/// Get the average ROI for a signal
///
/// # Arguments
/// * `signal` - The signal to calculate average ROI for
///
/// # Returns
/// Average ROI in basis points, or 0 if no executions
pub fn get_signal_average_roi(signal: &Signal) -> i128 {
    if signal.executions == 0 {
        0
    } else {
        signal.total_roi / (signal.executions as i128)
    }
}

/// Update provider performance statistics when a signal status changes
///
/// # Arguments
/// * `provider_stats` - Mutable reference to provider performance stats
/// * `old_status` - Previous signal status
/// * `new_status` - New signal status
/// * `signal_roi` - Average ROI of the signal (in basis points)
/// * `signal_volume` - Total volume of the signal
pub fn update_provider_performance(
    provider_stats: &mut ProviderPerformance,
    old_status: &SignalStatus,
    new_status: &SignalStatus,
    signal_roi: i128,
    signal_volume: i128,
) {
    // Only update when transitioning to a terminal state
    let is_terminal_transition = matches!(
        (old_status, new_status),
        (SignalStatus::Active, SignalStatus::Successful)
            | (SignalStatus::Active, SignalStatus::Failed)
            | (SignalStatus::Pending, SignalStatus::Successful)
            | (SignalStatus::Pending, SignalStatus::Failed)
    );

    if !is_terminal_transition {
        return;
    }

    // Increment total signals on first terminal state
    provider_stats.total_signals = provider_stats
        .total_signals
        .checked_add(1)
        .expect("total signals overflow");

    // Update success/failure counts
    match new_status {
        SignalStatus::Successful => {
            provider_stats.successful_signals = provider_stats
                .successful_signals
                .checked_add(1)
                .expect("successful signals overflow");
        }
        SignalStatus::Failed => {
            provider_stats.failed_signals = provider_stats
                .failed_signals
                .checked_add(1)
                .expect("failed signals overflow");
        }
        _ => {}
    }

    // Recalculate success rate: (successful_signals / total_signals) * 10000
    if provider_stats.total_signals > 0 {
        provider_stats.success_rate = ((provider_stats.successful_signals as i128)
            * BASIS_POINTS_100_PERCENT
            / (provider_stats.total_signals as i128)) as u32;
    }

    // Update average return as rolling average
    // Formula: new_avg = ((old_avg * (n-1)) + new_value) / n
    let n = provider_stats.total_signals as i128;
    if n > 0 {
        let old_total = provider_stats.avg_return.checked_mul(n - 1).unwrap_or(0);
        let new_total = old_total.checked_add(signal_roi).unwrap_or(old_total);
        provider_stats.avg_return = new_total / n;
    }

    // Add signal volume to total
    provider_stats.total_volume = provider_stats
        .total_volume
        .checked_add(signal_volume)
        .expect("total volume overflow");
}

/// Check if a status change should trigger provider stats update
pub fn should_update_provider_stats(old_status: &SignalStatus, new_status: &SignalStatus) -> bool {
    old_status != new_status
        && matches!(new_status, SignalStatus::Successful | SignalStatus::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_calculate_roi_buy_profit() {
        let roi = calculate_roi(100, 105, &SignalAction::Buy);
        assert_eq!(roi, 500); // 5% = 500 basis points
    }

    #[test]
    fn test_calculate_roi_buy_loss() {
        let roi = calculate_roi(100, 98, &SignalAction::Buy);
        assert_eq!(roi, -200); // -2% = -200 basis points
    }

    #[test]
    fn test_calculate_roi_sell_profit() {
        let roi = calculate_roi(100, 95, &SignalAction::Sell);
        assert_eq!(roi, 500); // 5% profit on sell = 500 basis points
    }

    #[test]
    fn test_calculate_roi_capped_at_negative_100_percent() {
        let roi = calculate_roi(100, 0, &SignalAction::Buy);
        assert_eq!(roi, -10000); // Capped at -100%
    }

    #[test]
    fn test_evaluate_status_expired_no_executions() {
        let signal = Signal {
            id: 1,
            provider: soroban_sdk::Address::generate(&soroban_sdk::Env::default()),
            asset_pair: soroban_sdk::String::from_str(&soroban_sdk::Env::default(), "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100,
            rationale: soroban_sdk::String::from_str(&soroban_sdk::Env::default(), "Test"),
            timestamp: 1000,
            expiry: 2000,
            status: SignalStatus::Active,
            executions: 0,
            total_volume: 0,
            total_roi: 0,
            category: crate::categories::SignalCategory::SWING,
            risk_level: crate::categories::RiskLevel::Medium,
            is_collaborative: false,
            tags: soroban_sdk::vec![&soroban_sdk::Env::default()],
            successful_executions: 0,
            submitted_at: 1000,
            rationale_hash: soroban_sdk::String::from_str(&soroban_sdk::Env::default(), "Test"),
            confidence: 50,
            adoption_count: 0,
        };

        let status = evaluate_signal_status(&signal, 2001);
        assert_eq!(status, SignalStatus::Failed);
    }

    #[test]
    fn test_get_signal_average_roi_zero_executions() {
        let signal = Signal {
            id: 1,
            provider: soroban_sdk::Address::generate(&soroban_sdk::Env::default()),
            asset_pair: soroban_sdk::String::from_str(&soroban_sdk::Env::default(), "XLM/USDC"),
            action: SignalAction::Buy,
            price: 100,
            rationale: soroban_sdk::String::from_str(&soroban_sdk::Env::default(), "Test"),
            timestamp: 1000,
            expiry: 2000,
            status: SignalStatus::Active,
            executions: 0,
            total_volume: 0,
            total_roi: 0,
            category: crate::categories::SignalCategory::SWING,
            risk_level: crate::categories::RiskLevel::Medium,
            is_collaborative: false,
            tags: soroban_sdk::vec![&soroban_sdk::Env::default()],
            successful_executions: 0,
            submitted_at: 1000,
            rationale_hash: soroban_sdk::String::from_str(&soroban_sdk::Env::default(), "Test"),
            confidence: 50,
            adoption_count: 0,
        };

        assert_eq!(get_signal_average_roi(&signal), 0);
    }
}
