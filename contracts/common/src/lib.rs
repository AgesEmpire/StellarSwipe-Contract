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

/// Error returned when a two-step administrator handoff is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminHandoffError {
    /// The caller is not the current administrator.
    NotAdmin,
    /// The caller is not the nominated successor.
    NotPendingAdmin,
    /// No successor has been nominated yet.
    NoPendingAdmin,
}

/// Two-step administrator handoff state for a core contract.
///
/// The current administrator nominates a successor with [`propose_admin`],
/// which does **not** change the administrator. The nominated address must
/// then call [`accept_admin`] to complete the handoff. This prevents an
/// accidental or unilateral transfer from taking effect immediately.
///
/// [`propose_admin`]: AdminHandoff::propose_admin
/// [`accept_admin`]: AdminHandoff::accept_admin
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminHandoff {
    /// The address currently holding administrator rights.
    pub admin: u64,
    /// The address nominated to become the next administrator, if any.
    pub pending_admin: Option<u64>,
}

impl AdminHandoff {
    /// Create a new handoff state with `admin` as the current administrator.
    pub fn new(admin: u64) -> Self {
        Self {
            admin,
            pending_admin: None,
        }
    }

    /// Nominate `successor` as the next administrator.
    ///
    /// Only the current administrator may nominate, and the nomination does
    /// not change the administrator until the successor accepts.
    pub fn propose_admin(&mut self, caller: u64, successor: u64) -> Result<(), AdminHandoffError> {
        if caller != self.admin {
            return Err(AdminHandoffError::NotAdmin);
        }
        self.pending_admin = Some(successor);
        Ok(())
    }

    /// Accept the pending nomination, transferring administrator rights.
    ///
    /// Only the nominated successor may accept. On success the successor
    /// becomes the administrator and the pending nomination is cleared.
    pub fn accept_admin(&mut self, caller: u64) -> Result<(), AdminHandoffError> {
        match self.pending_admin {
            None => Err(AdminHandoffError::NoPendingAdmin),
            Some(pending) if pending != caller => Err(AdminHandoffError::NotPendingAdmin),
            Some(pending) => {
                self.admin = pending;
                self.pending_admin = None;
                Ok(())
            }
        }
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
    pub chain: Vec<u64>,
}

impl InvocationGuard {
    /// Create an empty invocation guard.
    pub fn new() -> Self {
        Self { chain: Vec::new() }
    }

    /// Push `contract_id` onto the active chain.
    ///
    /// Returns [`InvocationCycleError::ReentrantCall`] if the contract is
    /// already active, or [`InvocationCycleError::DepthExceeded`] if the chain
    /// is already at [`MAX_INVOCATION_DEPTH`].
    pub fn push(&mut self, contract_id: u64) -> Result<(), InvocationCycleError> {
        if self.chain.contains(&contract_id) {
            return Err(InvocationCycleError::ReentrantCall);
        }
        if self.chain.len() as u32 >= MAX_INVOCATION_DEPTH {
            return Err(InvocationCycleError::DepthExceeded);
        }
        self.chain.push(contract_id);
        Ok(())
    }

    /// Pop the most recently pushed contract id from the active chain.
    pub fn pop(&mut self) -> Option<u64> {
        self.chain.pop()
    }

    /// Current depth of the active call chain.
    pub fn depth(&self) -> u32 {
        self.chain.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_requires_current_admin() {
        let mut handoff = AdminHandoff::new(1);
        assert_eq!(handoff.propose_admin(2, 3), Err(AdminHandoffError::NotAdmin));
        assert_eq!(handoff.admin, 1);
        assert_eq!(handoff.pending_admin, None);
    }

    #[test]
    fn propose_does_not_change_admin() {
        let mut handoff = AdminHandoff::new(1);
        assert_eq!(handoff.propose_admin(1, 2), Ok(()));
        assert_eq!(handoff.admin, 1);
        assert_eq!(handoff.pending_admin, Some(2));
    }

    #[test]
    fn accept_requires_pending_admin() {
        let mut handoff = AdminHandoff::new(1);
        assert_eq!(handoff.propose_admin(1, 2), Ok(()));
        assert_eq!(handoff.accept_admin(3), Err(AdminHandoffError::NotPendingAdmin));
        assert_eq!(handoff.admin, 1);
        assert_eq!(handoff.pending_admin, Some(2));
    }

    #[test]
    fn accept_without_nomination_fails() {
        let mut handoff = AdminHandoff::new(1);
        assert_eq!(handoff.accept_admin(1), Err(AdminHandoffError::NoPendingAdmin));
        assert_eq!(handoff.admin, 1);
    }

    #[test]
    fn accept_transfers_admin() {
        let mut handoff = AdminHandoff::new(1);
        assert_eq!(handoff.propose_admin(1, 2), Ok(()));
        assert_eq!(handoff.accept_admin(2), Ok(()));
        assert_eq!(handoff.admin, 2);
        assert_eq!(handoff.pending_admin, None);
    }

    #[test]
    fn old_admin_cannot_bypass_handoff() {
        let mut handoff = AdminHandoff::new(1);
        assert_eq!(handoff.propose_admin(1, 2), Ok(()));
        // The previous admin can no longer act as admin once the handoff is
        // pending, and cannot accept on behalf of the successor.
        assert_eq!(handoff.accept_admin(1), Err(AdminHandoffError::NotPendingAdmin));
        assert_eq!(handoff.admin, 1);
    }
}
