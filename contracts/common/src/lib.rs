//! Shared types and helpers for the protocol contracts.

use serde::{Deserialize, Serialize};

/// Numeric type used by a protocol parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    /// Unsigned integer value.
    Uint,
    /// Signed integer value.
    Int,
    /// Fixed-point decimal value.
    Decimal,
}

/// Who is allowed to update a protocol parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePermission {
    /// Only governance may update the parameter.
    Governance,
    /// Only the protocol admin may update the parameter.
    Admin,
    /// The parameter is immutable after initialization.
    Immutable,
}

/// Error returned when a parameter update violates its schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamSchemaError {
    /// The supplied value is below the schema minimum.
    BelowMinimum,
    /// The supplied value is above the schema maximum.
    AboveMaximum,
    /// The supplied value does not match the schema precision.
    PrecisionMismatch,
    /// The supplied unit does not match the schema unit.
    UnitMismatch,
    /// The caller is not permitted to update the parameter.
    Unauthorized,
}

/// Shared schema describing a configurable protocol parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamSchema {
    /// Stable identifier of the parameter.
    pub key: &'static str,
    /// Numeric type of the parameter value.
    pub param_type: ParamType,
    /// Unit the value is expressed in (e.g. "bps", "seconds").
    pub unit: &'static str,
    /// Number of decimal places the value is expressed with.
    pub precision: u32,
    /// Inclusive minimum value, expressed in `unit`.
    pub min: i128,
    /// Inclusive maximum value, expressed in `unit`.
    pub max: i128,
    /// Who may update the parameter.
    pub permission: UpdatePermission,
}

impl ParamSchema {
    /// Validate a candidate value against this schema.
    ///
    /// `value` is expressed in the schema's smallest unit (i.e. already
    /// scaled by `10^precision`), and `unit` is the unit the caller supplied.
    pub fn validate(&self, value: i128, unit: &str) -> Result<(), ParamSchemaError> {
        if unit != self.unit {
            return Err(ParamSchemaError::UnitMismatch);
        }
        if value < self.min {
            return Err(ParamSchemaError::BelowMinimum);
        }
        if value > self.max {
            return Err(ParamSchemaError::AboveMaximum);
        }
        Ok(())
    }

    /// Validate a candidate value together with its declared precision.
    pub fn validate_with_precision(
        &self,
        value: i128,
        unit: &str,
        precision: u32,
    ) -> Result<(), ParamSchemaError> {
        if precision != self.precision {
            return Err(ParamSchemaError::PrecisionMismatch);
        }
        self.validate(value, unit)
    }
}

/// Schema entry for every configurable protocol parameter.
///
/// Values are expressed in the schema's smallest unit, so a parameter with
/// `precision = 2` and `unit = "percent"` stores `50` for `0.50%`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolParamSchema {
    /// All configurable parameters, in a stable order.
    pub params: Vec<ParamSchema>,
}

impl ProtocolParamSchema {
    /// Build the canonical schema for all configurable protocol parameters.
    pub fn canonical() -> Self {
        Self {
            params: vec![
                ParamSchema {
                    key: "min_collateral_ratio",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 2,
                    min: 10_000,
                    max: 100_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "liquidation_penalty",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 2,
                    min: 0,
                    max: 5_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "interest_rate",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 4,
                    min: 0,
                    max: 10_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "max_oracle_staleness",
                    param_type: ParamType::Uint,
                    unit: "seconds",
                    precision: 0,
                    min: 1,
                    max: 86_400,
                    permission: UpdatePermission::Admin,
                },
                ParamSchema {
                    key: "protocol_fee",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 2,
                    min: 0,
                    max: 1_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "chain_id",
                    param_type: ParamType::Uint,
                    unit: "id",
                    precision: 0,
                    min: 1,
                    max: i128::MAX,
                    permission: UpdatePermission::Immutable,
                },
            ],
        }
    }

    /// Look up the schema entry for a parameter key.
    pub fn get(&self, key: &str) -> Option<&ParamSchema> {
        self.params.iter().find(|p| p.key == key)
    }

    /// Validate an update against the schema entry for `key`.
    pub fn validate_update(
        &self,
        key: &str,
        value: i128,
        unit: &str,
        precision: u32,
    ) -> Result<(), ParamSchemaError> {
        let schema = self.get(key).ok_or(ParamSchemaError::UnitMismatch)?;
        schema.validate_with_precision(value, unit, precision)
    }

    /// Render the schema as documentation for clients.
    pub fn to_documentation(&self) -> String {
        let mut out = String::from("# Protocol Parameter Schema\n\n");
        out.push_str("| key | type | unit | precision | min | max | permission |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        for p in &self.params {
            out.push_str(&format!(
                "| {} | {:?} | {} | {} | {} | {} | {:?} |\n",
                p.key, p.param_type, p.unit, p.precision, p.min, p.max, p.permission
            ));
        }
        out
    }
}

/// Maximum supported depth of nested cross-contract invocations.
///
/// A call chain deeper than this is rejected before any irreversible effect
/// (state write, transfer, or event emission) is performed.
pub const MAX_INVOCATION_DEPTH: u32 = 8;

/// Error returned when a cross-contract invocation is rejected by the
/// recursion/cycle policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationCycleError {
    /// The call would re-enter a contract already present in the active chain.
    ReentrantCall,
    /// The call would exceed [`MAX_INVOCATION_DEPTH`].
    DepthExceeded,
}

/// Guard enforcing the cross-contract invocation depth and cycle policy.
///
/// Call sites push the callee contract id before dispatching a cross-contract
/// call and pop it once the call returns. Pushing a contract id that is already
/// present in the active chain is rejected as a cycle, and pushing beyond
/// [`MAX_INVOCATION_DEPTH`] is rejected as a depth violation. Both checks run
/// before the caller performs any irreversible effect.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationGuard {
    /// Contract ids currently active in the call chain, outermost first.
    active: Vec<u64>,
}

impl InvocationGuard {
    /// Create an empty guard for a fresh top-level invocation.
    pub fn new() -> Self {
        Self { active: Vec::new() }
    }

    /// Current depth of the active call chain.
    pub fn depth(&self) -> u32 {
        self.active.len() as u32
    }

    /// Whether `contract_id` is already active in the call chain.
    pub fn is_active(&self, contract_id: u64) -> bool {
        self.active.contains(&contract_id)
    }

    /// Check whether a cross-contract call to `contract_id` is permitted.
    ///
    /// Returns [`InvocationCycleError::ReentrantCall`] for a direct or indirect
    /// cycle and [`InvocationCycleError::DepthExceeded`] when the call would
    /// exceed [`MAX_INVOCATION_DEPTH`]. This performs no mutation, so callers
    /// can reject a cycle before any irreversible effect.
    pub fn check(&self, contract_id: u64) -> Result<(), InvocationCycleError> {
        if self.is_active(contract_id) {
            return Err(InvocationCycleError::ReentrantCall);
        }
        if self.depth() >= MAX_INVOCATION_DEPTH {
            return Err(InvocationCycleError::DepthExceeded);
        }
        Ok(())
    }

    /// Enter a cross-contract call, enforcing the cycle and depth policy.
    ///
    /// On success the callee is recorded as active; call [`Self::exit`] once
    /// the call returns. On error the guard is left unchanged.
    pub fn enter(&mut self, contract_id: u64) -> Result<(), InvocationCycleError> {
        self.check(contract_id)?;
        self.active.push(contract_id);
        Ok(())
    }

    /// Leave a cross-contract call previously entered with [`Self::enter`].
    pub fn exit(&mut self, contract_id: u64) {
        if self.active.last() == Some(&contract_id) {
            self.active.pop();
        }
    }
}

/// Version of the normalized TTL renewal outcome event schema.
///
/// Bump this whenever the fields of [`TtlRenewalOutcome`] change so that
/// off-chain consumers can detect incompatible event payloads.
pub const TTL_RENEWAL_OUTCOME_EVENT_VERSION: u32 = 1;

/// Normalized outcome of a single storage TTL renewal attempt.
///
/// Every shared TTL caller emits this event instead of ad-hoc per-caller
/// emissions, so off-chain indexers observe one deterministic shape for
/// skipped, renewed, and failed renewals.
///
/// Fields:
/// - `version`: schema version, see [`TTL_RENEWAL_OUTCOME_EVENT_VERSION`].
/// - `key`: storage key whose TTL was considered for renewal.
/// - `result`: normalized outcome of the attempt.
/// - `current_ttl`: TTL (in ledgers) observed before the attempt.
/// - `threshold`: TTL (in ledgers) below which renewal is attempted.
/// - `extend_to`: TTL (in ledgers) requested when a renewal is performed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtlRenewalOutcome {
    /// Schema version of this event payload.
    pub version: u32,
    /// Storage key whose TTL was considered for renewal.
    pub key: String,
    /// Normalized outcome of the renewal attempt.
    pub result: TtlRenewalResult,
    /// TTL (in ledgers) observed before the attempt.
    pub current_ttl: u32,
    /// TTL (in ledgers) below which renewal is attempted.
    pub threshold: u32,
    /// TTL (in ledgers) requested when a renewal is performed.
    pub extend_to: u32,
}

/// Normalized result of a TTL renewal attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtlRenewalResult {
    /// The key's TTL was already above the threshold; no renewal performed.
    Skipped,
    /// The key's TTL was renewed to `extend_to`.
    Renewed,
    /// The renewal attempt failed.
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_cycle_is_rejected() {
        // A -> A: re-entering the same contract is a direct cycle.
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        assert_eq!(guard.enter(1), Err(InvocationCycleError::ReentrantCall));
    }

    #[test]
    fn indirect_cycle_is_rejected() {
        // A -> B -> A: re-entering an ancestor is an indirect cycle.
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        guard.enter(2).unwrap();
        assert_eq!(guard.enter(1), Err(InvocationCycleError::ReentrantCall));
    }

    #[test]
    fn non_cyclic_nested_call_succeeds() {
        // A -> B -> C is a valid non-cyclic nested call chain.
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        guard.enter(2).unwrap();
        guard.enter(3).unwrap();
        assert_eq!(guard.depth(), 3);
        guard.exit(3);
        guard.exit(2);
        guard.exit(1);
        assert_eq!(guard.depth(), 0);
    }

    #[test]
    fn depth_limit_is_enforced() {
        let mut guard = InvocationGuard::new();
        for id in 0..MAX_INVOCATION_DEPTH as u64 {
            guard.enter(id).unwrap();
        }
        assert_eq!(
            guard.enter(MAX_INVOCATION_DEPTH as u64),
            Err(InvocationCycleError::DepthExceeded)
        );
    }

    #[test]
    fn rejected_cycle_leaves_guard_unchanged() {
        let mut guard = InvocationGuard::new();
        guard.enter(1).unwrap();
        let before = guard.clone();
        assert!(guard.enter(1).is_err());
        assert_eq!(guard, before);
    }
}
