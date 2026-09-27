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
}
