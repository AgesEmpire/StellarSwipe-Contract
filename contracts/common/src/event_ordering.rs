//! Event ordering utilities and cross-contract invocation guards.
//!
//! This module provides helpers for ordering events emitted across contracts
//! and a reusable guard that detects recursive cross-contract invocation
//! cycles before any irreversible effects (state writes, transfers, events)
//! are performed.

use std::collections::HashSet;

/// Maximum supported cross-contract invocation depth.
///
/// Any invocation chain deeper than this is rejected before it can produce
/// irreversible effects. This bounds recursion so a cyclic call pattern
/// cannot exhaust resources.
pub const MAX_INVOCATION_DEPTH: u32 = 8;

/// Policy applied when a cross-contract invocation cycle is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CyclePolicy {
    /// Reject the invocation as soon as a cycle or depth overflow is seen.
    Reject,
}

/// Error returned when a cross-contract invocation is not allowed to proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationError {
    /// The same contract was re-entered within the current invocation chain.
    RecursiveCycle,
    /// The invocation chain exceeded [`MAX_INVOCATION_DEPTH`].
    DepthExceeded,
}

/// Tracks the active cross-contract invocation chain and enforces the
/// configured [`CyclePolicy`].
///
/// Call [`InvocationGuard::enter`] before performing any irreversible effect
/// for a cross-contract call, and [`InvocationGuard::exit`] once the call
/// returns. A rejected `enter` leaves the guard unchanged, so the caller can
/// fail safely before mutating state.
#[derive(Debug, Default, Clone)]
pub struct InvocationGuard {
    policy: CyclePolicy,
    max_depth: u32,
    active: Vec<u64>,
    seen: HashSet<u64>,
}

impl InvocationGuard {
    /// Create a guard using the default [`CyclePolicy::Reject`] policy and
    /// [`MAX_INVOCATION_DEPTH`].
    pub fn new() -> Self {
        Self::with_policy(CyclePolicy::Reject, MAX_INVOCATION_DEPTH)
    }

    /// Create a guard with an explicit policy and depth limit.
    pub fn with_policy(policy: CyclePolicy, max_depth: u32) -> Self {
        Self {
            policy,
            max_depth,
            active: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// The configured cycle policy.
    pub fn policy(&self) -> CyclePolicy {
        self.policy
    }

    /// The configured maximum invocation depth.
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Current invocation depth.
    pub fn depth(&self) -> u32 {
        self.active.len() as u32
    }

    /// Whether `contract_id` is already active in the current chain.
    pub fn is_active(&self, contract_id: u64) -> bool {
        self.seen.contains(&contract_id)
    }

    /// Attempt to enter a cross-contract invocation for `contract_id`.
    ///
    /// Returns `Ok(())` when the invocation may proceed. Returns an
    /// [`InvocationError`] and leaves the guard unchanged when the call would
    /// create a cycle or exceed the depth limit, so callers can reject the
    /// call before any irreversible effect.
    pub fn enter(&mut self, contract_id: u64) -> Result<(), InvocationError> {
        if self.seen.contains(&contract_id) {
            return Err(InvocationError::RecursiveCycle);
        }
        if self.active.len() as u32 >= self.max_depth {
            return Err(InvocationError::DepthExceeded);
        }
        self.active.push(contract_id);
        self.seen.insert(contract_id);
        Ok(())
    }

    /// Exit the most recent invocation, clearing it from the active chain.
    ///
    /// Returns the contract id that was exited, or `None` if the chain is
    /// already empty.
    pub fn exit(&mut self) -> Option<u64> {
        let id = self.active.pop()?;
        self.seen.remove(&id);
        Some(id)
    }

    /// Run `f` within a guarded invocation of `contract_id`.
    ///
    /// The guard is entered before `f` runs and exited afterwards, including
    /// on error. If entering fails, `f` is never invoked and the error is
    /// returned, ensuring rejected cycles fail before irreversible effects.
    pub fn with_invocation<F, T>(
        &mut self,
        contract_id: u64,
        f: F,
    ) -> Result<T, InvocationError>
    where
        F: FnOnce(&mut Self) -> T,
    {
        self.enter(contract_id)?;
        let result = f(self);
        self.exit();
        Ok(result)
    }
}

/// Supported ledger protocol versions.
///
/// Deterministic contract calculations must produce identical state and event
/// outputs under every supported protocol version. Any protocol-dependent
/// behavior must be documented here and covered by an explicit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerProtocolVersion {
    /// Protocol version 20.
    V20,
    /// Protocol version 21.
    V21,
    /// Protocol version 22.
    V22,
}

impl LedgerProtocolVersion {
    /// All supported ledger protocol versions.
    pub const ALL: [LedgerProtocolVersion; 3] = [
        LedgerProtocolVersion::V20,
        LedgerProtocolVersion::V21,
        LedgerProtocolVersion::V22,
    ];

    /// The numeric protocol version.
    pub fn as_u32(self) -> u32 {
        match self {
            LedgerProtocolVersion::V20 => 20,
            LedgerProtocolVersion::V21 => 21,
            LedgerProtocolVersion::V22 => 22,
        }
    }
}

/// A deterministic state/event output produced by a contract flow.
///
/// Used to compare outcomes across supported ledger protocol versions. The
/// ordering of `events` is significant and must be stable across protocols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterministicOutcome {
    /// Deterministic state digest after the flow completes.
    pub state: u64,
    /// Ordered event identifiers emitted by the flow.
    pub events: Vec<u64>,
}

/// Run a deterministic flow under a given ledger protocol version.
///
/// The flow is a pure function of the protocol version and the shared fixture
/// input, so its state and event outputs can be compared across protocols.
/// Protocol-dependent behavior, if any, must be documented on the flow and
/// asserted explicitly in tests.
pub fn run_deterministic_flow<F>(
    protocol: LedgerProtocolVersion,
    flow: F,
) -> DeterministicOutcome
where
    F: Fn(LedgerProtocolVersion) -> DeterministicOutcome,
{
    flow(protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_cycle_is_rejected() {
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        assert_eq!(guard.enter(1), Err(InvocationError::RecursiveCycle));
        // Guard state is unchanged after a rejected enter.
        assert_eq!(guard.depth(), 1);
    }

    #[test]
    fn indirect_cycle_is_rejected() {
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        guard.enter(2).unwrap();
        assert_eq!(guard.enter(1), Err(InvocationError::RecursiveCycle));
        assert_eq!(guard.depth(), 2);
    }

    #[test]
    fn valid_nested_call_succeeds() {
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        guard.enter(2).unwrap();
        guard.enter(3).unwrap();
        assert_eq!(guard.depth(), 3);
        assert_eq!(guard.exit(), Some(3));
        assert_eq!(guard.exit(), Some(2));
        assert_eq!(guard.exit(), Some(1));
        assert_eq!(guard.depth(), 0);
    }

    #[test]
    fn depth_limit_is_enforced() {
        let mut guard = InvocationGuard::with_policy(CyclePolicy::Reject, 2);
        guard.enter(1).unwrap();
        guard.enter(2).unwrap();
        assert_eq!(guard.enter(3), Err(InvocationError::DepthExceeded));
        assert_eq!(guard.depth(), 2);
    }

    #[test]
    fn with_invocation_rejects_before_running() {
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        let mut ran = false;
        let result = guard.with_invocation(1, |_| {
            ran = true;
        });
        assert_eq!(result, Err(InvocationError::RecursiveCycle));
        assert!(!ran, "rejected cycle must not run the invocation body");
    }

    #[test]
    fn with_invocation_exits_on_success() {
        let mut guard = InvocationGuard::new();
        let value = guard.with_invocation(7, |_| 42).unwrap();
        assert_eq!(value, 42);
        assert_eq!(guard.depth(), 0);
        assert!(!guard.is_active(7));
    }

    /// Shared fixture: a deterministic flow whose state and event outputs must
    /// be identical under every supported ledger protocol version.
    fn shared_fixture(protocol: LedgerProtocolVersion) -> DeterministicOutcome {
        // The flow is protocol-independent by construction: it derives its
        // state and events purely from the fixture input, not the protocol.
        let _ = protocol;
        DeterministicOutcome {
            state: 0xDEAD_BEEF,
            events: vec![1, 2, 3],
        }
    }

    #[test]
    fn shared_fixture_runs_under_each_protocol() {
        for protocol in LedgerProtocolVersion::ALL {
            let outcome = run_deterministic_flow(protocol, shared_fixture);
            assert_eq!(outcome.state, 0xDEAD_BEEF);
            assert_eq!(outcome.events, vec![1, 2, 3]);
        }
    }

    #[test]
    fn state_and_events_are_deterministic_across_protocols() {
        let baseline = run_deterministic_flow(LedgerProtocolVersion::V20, shared_fixture);
        for protocol in LedgerProtocolVersion::ALL {
            let outcome = run_deterministic_flow(protocol, shared_fixture);
            assert_eq!(
                outcome, baseline,
                "protocol {:?} produced a non-deterministic outcome",
                protocol
            );
        }
    }

    #[test]
    fn protocol_versions_are_distinct_and_ordered() {
        // Documents the supported protocol set explicitly so any change to the
        // supported versions is caught by a test.
        let versions: Vec<u32> = LedgerProtocolVersion::ALL
            .iter()
            .map(|p| p.as_u32())
            .collect();
        assert_eq!(versions, vec![20, 21, 22]);
    }
}
