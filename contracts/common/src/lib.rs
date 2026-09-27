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
    /// Only the current administrator may nominate a successor, and the
    /// nomination does not take effect until [`accept_admin`] is called.
    ///
    /// [`accept_admin`]: AdminHandoff::accept_admin
    pub fn propose_admin(&mut self, caller: u64, successor: u64) -> Result<(), AdminHandoffError> {
        if caller != self.admin {
            return Err(AdminHandoffError::NotAdmin);
        }
        self.pending_admin = Some(successor);
        Ok(())
    }

    /// Complete a pending handoff, transferring administrator rights to the
    /// nominated successor.
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

/// A narrowly scoped maintenance capability that can be granted to an
/// operator.
///
/// Each capability maps to a defined set of contract entrypoints (see
/// [`OperatorCapability::entrypoints`]); there is deliberately no broad
/// administrative catch-all, so an operator holding one capability cannot
/// invoke entrypoints outside that capability's scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorCapability {
    /// Pause and unpause contract operations.
    Pause,
    /// Update the oracle staleness window.
    OracleConfig,
    /// Update protocol fee parameters.
    FeeConfig,
    /// Rotate signing keys used by the contract.
    KeyRotation,
}

impl OperatorCapability {
    /// Every capability, in a stable order.
    pub const ALL: [OperatorCapability; 4] = [
        OperatorCapability::Pause,
        OperatorCapability::OracleConfig,
        OperatorCapability::FeeConfig,
        OperatorCapability::KeyRotation,
    ];

    /// The contract entrypoints this capability authorizes.
    ///
    /// The returned set is exhaustive: an operator granted this capability
    /// may invoke exactly these entrypoints and nothing else.
    pub fn entrypoints(&self) -> &'static [&'static str] {
        match self {
            OperatorCapability::Pause => &["pause", "unpause"],
            OperatorCapability::OracleConfig => &["set_oracle_staleness"],
            OperatorCapability::FeeConfig => &["set_protocol_fee"],
            OperatorCapability::KeyRotation => &["rotate_signing_key"],
        }
    }

    /// Whether this capability authorizes `entrypoint`.
    pub fn authorizes(&self, entrypoint: &str) -> bool {
        self.entrypoints().contains(&entrypoint)
    }
}

/// Error returned when a capability grant or revocation is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityError {
    /// The caller is not authorized to grant or revoke capabilities.
    Unauthorized,
    /// The operator already holds the capability.
    AlreadyGranted,
    /// The operator does not hold the capability.
    NotGranted,
    /// The operator is not permitted to invoke the entrypoint.
    NotPermitted,
}

/// Auditable record of a capability grant or revocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvent {
    /// A capability was granted to an operator.
    Granted {
        /// The operator that received the capability.
        operator: u64,
        /// The capability that was granted.
        capability: OperatorCapability,
    },
    /// A capability was revoked from an operator.
    Revoked {
        /// The operator that lost the capability.
        operator: u64,
        /// The capability that was revoked.
        capability: OperatorCapability,
    },
}

/// Registry of scoped operator capabilities.
///
/// Grants and revocations are authorized against the current administrator
/// and emit a [`CapabilityEvent`] so every change is auditable. Invocation
/// checks enforce least privilege: an operator may only invoke entrypoints
/// covered by a capability it actually holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRegistry {
    /// The address authorized to grant and revoke capabilities.
    pub admin: u64,
    /// Granted capabilities, as `(operator, capability)` pairs.
    pub grants: Vec<(u64, OperatorCapability)>,
    /// Audit log of every grant and revocation, in order.
    pub events: Vec<CapabilityEvent>,
}

impl CapabilityRegistry {
    /// Create an empty registry administered by `admin`.
    pub fn new(admin: u64) -> Self {
        Self {
            admin,
            grants: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Whether `operator` currently holds `capability`.
    pub fn has_capability(&self, operator: u64, capability: OperatorCapability) -> bool {
        self.grants.contains(&(operator, capability))
    }

    /// Grant `capability` to `operator`.
    ///
    /// Only the administrator may grant capabilities, and the grant is
    /// recorded as an auditable [`CapabilityEvent::Granted`].
    pub fn grant(
        &mut self,
        caller: u64,
        operator: u64,
        capability: OperatorCapability,
    ) -> Result<(), CapabilityError> {
        if caller != self.admin {
            return Err(CapabilityError::Unauthorized);
        }
        if self.has_capability(operator, capability) {
            return Err(CapabilityError::AlreadyGranted);
        }
        self.grants.push((operator, capability));
        self.events.push(CapabilityEvent::Granted {
            operator,
            capability,
        });
        Ok(())
    }

    /// Revoke `capability` from `operator`.
    ///
    /// Only the administrator may revoke capabilities, and the revocation is
    /// recorded as an auditable [`CapabilityEvent::Revoked`].
    pub fn revoke(
        &mut self,
        caller: u64,
        operator: u64,
        capability: OperatorCapability,
    ) -> Result<(), CapabilityError> {
        if caller != self.admin {
            return Err(CapabilityError::Unauthorized);
        }
        let index = self
            .grants
            .iter()
            .position(|&(op, cap)| op == operator && cap == capability)
            .ok_or(CapabilityError::NotGranted)?;
        self.grants.remove(index);
        self.events.push(CapabilityEvent::Revoked {
            operator,
            capability,
        });
        Ok(())
    }

    /// Authorize `operator` to invoke `entrypoint`.
    ///
    /// Succeeds only when the operator holds a capability whose scope covers
    /// `entrypoint`, enforcing least privilege across capabilities.
    pub fn authorize_entrypoint(
        &self,
        operator: u64,
        entrypoint: &str,
    ) -> Result<(), CapabilityError> {
        let permitted = self.grants.iter().any(|&(op, cap)| {
            op == operator && cap.authorizes(entrypoint)
        });
        if permitted {
            Ok(())
        } else {
            Err(CapabilityError::NotPermitted)
        }
    }
}
