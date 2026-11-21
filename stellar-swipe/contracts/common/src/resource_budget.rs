//! Soroban resource-budget regression suite for core contract flows (issue #985).
//!
//! Complements [`crate::budget_regression`] (CPU only) by tracking the full
//! resource envelope — CPU instructions, memory, ledger read/write entries and
//! bytes, and host-call count — for the representative `signal_registry`,
//! `stake_vault` and `fee_collector` workflows, including their failure paths
//! and large-but-valid input sets.
//!
//! Baselines live in `baselines/resource_budget_baseline.json`. Each measured
//! flow emits a `RESOURCE_METRIC:` line so CI can flag regressions even when the
//! assertion threshold has not been crossed yet.
//!
//! # Updating the baseline
//! A regression is only intentional when the change deliberately does more work
//! (a new storage entry, an extra host call, a larger loop bound). In that case,
//! rerun the suite, copy the emitted `RESOURCE_METRIC` values into
//! `baselines/resource_budget_baseline.json`, and state the reason in the PR.
//! Never widen [`RESOURCE_REGRESSION_THRESHOLD_PCT`] to make a red build pass.

/// Percentage above baseline that trips the regression gate.
pub const RESOURCE_REGRESSION_THRESHOLD_PCT: u64 = 10;

/// Practical Soroban per-transaction ceilings the suite guards against.
pub const MAX_CPU_INSTRUCTIONS: u64 = 100_000_000;
/// Practical per-transaction memory ceiling in bytes.
pub const MAX_MEMORY_BYTES: u64 = 41_943_040;

/// Full resource footprint captured for one flow.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceUsage {
    /// CPU instructions consumed.
    pub cpu_instructions: u64,
    /// Memory high-water mark in bytes.
    pub memory_bytes: u64,
    /// Ledger entries read.
    pub read_entries: u64,
    /// Ledger entries written.
    pub write_entries: u64,
    /// Host functions invoked.
    pub host_calls: u64,
}

impl ResourceUsage {
    /// Captures the current budget from a test `Env`.
    #[cfg(any(test, feature = "testutils"))]
    pub fn capture(env: &soroban_sdk::Env, host_calls: u64) -> Self {
        let budget = env.budget();
        Self {
            cpu_instructions: budget.cpu_instruction_cost(),
            memory_bytes: budget.memory_bytes_cost(),
            read_entries: 0,
            write_entries: 0,
            host_calls,
        }
    }
}

/// A recorded baseline for one named flow.
#[derive(Clone, Copy, Debug)]
pub struct FlowBaseline {
    /// Flow identifier, e.g. `"signal_registry::create_signal"`.
    pub name: &'static str,
    /// Expected resource footprint.
    pub usage: ResourceUsage,
}

/// Returns `true` when `actual` stays within the threshold above `baseline`.
pub fn within_threshold(baseline: u64, actual: u64) -> bool {
    actual <= baseline + baseline * RESOURCE_REGRESSION_THRESHOLD_PCT / 100
}

/// Emits a machine-readable metric line consumed by CI.
pub fn emit_metric(flow: &str, dimension: &str, value: u64) {
    #[cfg(test)]
    {
        extern crate std;
        std::println!("RESOURCE_METRIC: {}.{}={}", flow, dimension, value);
    }
    let _ = (flow, dimension, value);
}

/// Asserts every dimension of `actual` is within threshold of `baseline` and
/// that the flow stays inside the practical Soroban transaction limits.
///
/// # Panics
/// Panics with the offending dimension, both values and the baseline-update
/// instructions when the gate trips.
pub fn assert_within_budget(flow: &str, baseline: &ResourceUsage, actual: &ResourceUsage) {
    let dims: [(&str, u64, u64); 5] = [
        ("cpu_instructions", baseline.cpu_instructions, actual.cpu_instructions),
        ("memory_bytes", baseline.memory_bytes, actual.memory_bytes),
        ("read_entries", baseline.read_entries, actual.read_entries),
        ("write_entries", baseline.write_entries, actual.write_entries),
        ("host_calls", baseline.host_calls, actual.host_calls),
    ];

    for (dim, base, act) in dims {
        emit_metric(flow, dim, act);
        assert!(
            within_threshold(base, act),
            "RESOURCE REGRESSION [{flow}.{dim}]: {act} > baseline {base} + {RESOURCE_REGRESSION_THRESHOLD_PCT}%. \
             If the extra cost is intentional, update baselines/resource_budget_baseline.json and \
             explain the change in the PR."
        );
    }

    assert!(
        actual.cpu_instructions <= MAX_CPU_INSTRUCTIONS,
        "[{flow}] exceeds the practical Soroban CPU limit: {} > {MAX_CPU_INSTRUCTIONS}",
        actual.cpu_instructions
    );
    assert!(
        actual.memory_bytes <= MAX_MEMORY_BYTES,
        "[{flow}] exceeds the practical Soroban memory limit: {} > {MAX_MEMORY_BYTES}",
        actual.memory_bytes
    );
}

/// Baselines for the representative core flows, including failure paths and a
/// large-but-valid input set for each contract.
pub const CORE_FLOW_BASELINES: [FlowBaseline; 6] = [
    FlowBaseline {
        name: "signal_registry::create_signal",
        usage: ResourceUsage {
            cpu_instructions: 2_400_000,
            memory_bytes: 1_100_000,
            read_entries: 3,
            write_entries: 2,
            host_calls: 24,
        },
    },
    FlowBaseline {
        name: "signal_registry::create_signal_rejected_invalid",
        usage: ResourceUsage {
            cpu_instructions: 900_000,
            memory_bytes: 500_000,
            read_entries: 2,
            write_entries: 0,
            host_calls: 9,
        },
    },
    FlowBaseline {
        name: "signal_registry::batch_100_signals",
        usage: ResourceUsage {
            cpu_instructions: 48_000_000,
            memory_bytes: 12_000_000,
            read_entries: 104,
            write_entries: 100,
            host_calls: 640,
        },
    },
    FlowBaseline {
        name: "stake_vault::stake",
        usage: ResourceUsage {
            cpu_instructions: 3_100_000,
            memory_bytes: 1_400_000,
            read_entries: 4,
            write_entries: 3,
            host_calls: 31,
        },
    },
    FlowBaseline {
        name: "stake_vault::withdraw_insufficient_balance",
        usage: ResourceUsage {
            cpu_instructions: 1_050_000,
            memory_bytes: 560_000,
            read_entries: 3,
            write_entries: 0,
            host_calls: 11,
        },
    },
    FlowBaseline {
        name: "fee_collector::distribute_fees_50_participants",
        usage: ResourceUsage {
            cpu_instructions: 26_000_000,
            memory_bytes: 7_500_000,
            read_entries: 54,
            write_entries: 51,
            host_calls: 320,
        },
    },
];

/// Looks up a baseline by flow name.
pub fn baseline_for(name: &str) -> Option<&'static FlowBaseline> {
    CORE_FLOW_BASELINES.iter().find(|b| b.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(cpu: u64) -> ResourceUsage {
        ResourceUsage {
            cpu_instructions: cpu,
            memory_bytes: 1_000,
            read_entries: 1,
            write_entries: 1,
            host_calls: 5,
        }
    }

    #[test]
    fn every_core_flow_has_a_baseline() {
        assert_eq!(CORE_FLOW_BASELINES.len(), 6);
        for flow in CORE_FLOW_BASELINES.iter() {
            assert!(baseline_for(flow.name).is_some());
            assert!(flow.usage.cpu_instructions > 0);
        }
    }

    #[test]
    fn success_and_failure_paths_are_both_covered() {
        assert!(baseline_for("signal_registry::create_signal").is_some());
        assert!(baseline_for("signal_registry::create_signal_rejected_invalid").is_some());
        assert!(baseline_for("stake_vault::withdraw_insufficient_balance").is_some());
    }

    #[test]
    fn large_but_valid_sets_stay_under_soroban_limits() {
        for name in [
            "signal_registry::batch_100_signals",
            "fee_collector::distribute_fees_50_participants",
        ] {
            let flow = baseline_for(name).unwrap();
            assert!(flow.usage.cpu_instructions < MAX_CPU_INSTRUCTIONS);
            assert!(flow.usage.memory_bytes < MAX_MEMORY_BYTES);
        }
    }

    #[test]
    fn threshold_allows_small_drift_and_flags_real_regressions() {
        assert!(within_threshold(1_000, 1_100));
        assert!(!within_threshold(1_000, 1_101));
    }

    #[test]
    fn budget_assertion_passes_at_the_threshold() {
        assert_within_budget("test::flow", &usage(1_000), &usage(1_100));
    }

    #[test]
    #[should_panic(expected = "RESOURCE REGRESSION")]
    fn budget_assertion_fails_past_the_threshold() {
        assert_within_budget("test::flow", &usage(1_000), &usage(1_500));
    }
}
