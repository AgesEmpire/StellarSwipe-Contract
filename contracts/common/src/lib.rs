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
    pub key: &'static str,
    /// Normalized outcome of the attempt.
    pub result: TtlRenewalResult,
    /// TTL (in ledgers) observed before the attempt.
    pub current_ttl: u32,
    /// TTL (in ledgers) below which renewal is attempted.
    pub threshold: u32,
    /// TTL (in ledgers) requested when a renewal is performed.
    pub extend_to: u32,
}

/// Normalized result of a storage TTL renewal attempt.
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

/// Maximum number of ledgers a rotated bridge validator set may remain
/// active alongside its successor.
///
/// A bounded overlap keeps messages signed just before a rotation verifiable
/// while the new set is being established, without allowing signatures from
/// arbitrarily old sets to be accepted indefinitely.
pub const BRIDGE_VALIDATOR_SET_MAX_OVERLAP_LEDGERS: u32 = 17_280;

/// Error returned when a bridge validator-set rotation is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeValidatorSetError {
    /// The activation ledger is not strictly after the current set's activation.
    NonMonotonicActivation,
    /// The retirement ledger is not strictly after the activation ledger.
    InvalidRetirementBoundary,
    /// The overlap between the outgoing and incoming sets exceeds the bound.
    OverlapTooLong,
    /// The rotation would leave no active validator set.
    NoActiveSet,
}

/// Explicit activation and retirement boundaries for a bridge validator set.
///
/// A set is active for the inclusive ledger range
/// `[activation_ledger, retirement_ledger]`. The outgoing set's
/// `retirement_ledger` and the incoming set's `activation_ledger` define the
/// overlap window during which both sets may verify messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeValidatorSetBoundary {
    /// Ledger at which this set becomes active (inclusive).
    pub activation_ledger: u32,
    /// Ledger at which this set is retired (inclusive).
    pub retirement_ledger: u32,
}

impl BridgeValidatorSetBoundary {
    /// Whether `ledger` falls within this set's active range.
    pub fn is_active_at(&self, ledger: u32) -> bool {
        ledger >= self.activation_ledger && ledger <= self.retirement_ledger
    }

    /// Number of ledgers this set remains active for.
    pub fn active_span(&self) -> u32 {
        self.retirement_ledger.saturating_sub(self.activation_ledger)
    }
}

/// Validate a rotation from `current` to `next`.
///
/// Enforces explicit, monotonic activation/retirement boundaries and a bounded
/// overlap so that messages signed before, during, and after the rotation stay
/// verifiable without accepting signatures from unintended sets.
///
/// The overlap is the number of ledgers during which both sets are active,
/// i.e. `current.retirement_ledger - next.activation_ledger + 1` when the
/// ranges intersect. It must not exceed
/// [`BRIDGE_VALIDATOR_SET_MAX_OVERLAP_LEDGERS`].
///
/// Returns the overlap length in ledgers on success.
pub fn validate_bridge_validator_set_rotation(
    current: BridgeValidatorSetBoundary,
    next: BridgeValidatorSetBoundary,
) -> Result<u32, BridgeValidatorSetError> {
    if next.activation_ledger <= current.activation_ledger {
        return Err(BridgeValidatorSetError::NonMonotonicActivation);
    }
    if next.retirement_ledger <= next.activation_ledger {
        return Err(BridgeValidatorSetError::InvalidRetirementBoundary);
    }
    if current.retirement_ledger < current.activation_ledger {
        return Err(BridgeValidatorSetError::InvalidRetirementBoundary);
    }

    // No overlap: the outgoing set retires before the incoming set activates.
    if current.retirement_ledger < next.activation_ledger {
        return Ok(0);
    }

    let overlap = current
        .retirement_ledger
        .saturating_sub(next.activation_ledger)
        .saturating_add(1);
    if overlap > BRIDGE_VALIDATOR_SET_MAX_OVERLAP_LEDGERS {
        return Err(BridgeValidatorSetError::OverlapTooLong);
    }
    Ok(overlap)
}

/// Whether a message signed at `signed_at` is verifiable by a set whose
/// boundary is `boundary`.
///
/// A message is verifiable only when the signing ledger falls within the
/// set's explicit active range, so signatures from unintended sets are
/// rejected.
pub fn is_message_verifiable_by_set(
    boundary: BridgeValidatorSetBoundary,
    signed_at: u32,
) -> bool {
    boundary.is_active_at(signed_at)
}

#[cfg(test)]
mod bridge_validator_set_rotation_tests {
    use super::*;

    fn boundary(activation_ledger: u32, retirement_ledger: u32) -> BridgeValidatorSetBoundary {
        BridgeValidatorSetBoundary {
            activation_ledger,
            retirement_ledger,
        }
    }

    #[test]
    fn message_signed_before_rotation_is_verifiable_by_outgoing_set() {
        let current = boundary(100, 200);
        let next = boundary(180, 300);
        assert_eq!(validate_bridge_validator_set_rotation(current, next), Ok(21));
        // Signed before the incoming set activates: only the outgoing set verifies.
        assert!(is_message_verifiable_by_set(current, 150));
        assert!(!is_message_verifiable_by_set(next, 150));
    }

    #[test]
    fn message_signed_during_overlap_is_verifiable_by_both_sets() {
        let current = boundary(100, 200);
        let next = boundary(180, 300);
        assert_eq!(validate_bridge_validator_set_rotation(current, next), Ok(21));
        // Signed during the overlap window: both sets verify.
        assert!(is_message_verifiable_by_set(current, 190));
        assert!(is_message_verifiable_by_set(next, 190));
    }

    #[test]
    fn message_signed_after_rotation_is_verifiable_by_incoming_set() {
        let current = boundary(100, 200);
        let next = boundary(180, 300);
        assert_eq!(validate_bridge_validator_set_rotation(current, next), Ok(21));
        // Signed after the outgoing set retires: only the incoming set verifies.
        assert!(!is_message_verifiable_by_set(current, 250));
        assert!(is_message_verifiable_by_set(next, 250));
    }

    #[test]
    fn rotation_without_overlap_is_allowed() {
        let current = boundary(100, 150);
        let next = boundary(151, 250);
        assert_eq!(validate_bridge_validator_set_rotation(current, next), Ok(0));
        assert!(!is_message_verifiable_by_set(current, 151));
        assert!(is_message_verifiable_by_set(next, 151));
    }

    #[test]
    fn overlap_beyond_bound_is_rejected() {
        let current = boundary(100, 100 + BRIDGE_VALIDATOR_SET_MAX_OVERLAP_LEDGERS + 5);
        let next = boundary(101, 100 + BRIDGE_VALIDATOR_SET_MAX_OVERLAP_LEDGERS + 10);
        assert_eq!(
            validate_bridge_validator_set_rotation(current, next),
            Err(BridgeValidatorSetError::OverlapTooLong)
        );
    }

    #[test]
    fn non_monotonic_activation_is_rejected() {
        let current = boundary(100, 200);
        let next = boundary(100, 300);
        assert_eq!(
            validate_bridge_validator_set_rotation(current, next),
            Err(BridgeValidatorSetError::NonMonotonicActivation)
        );
    }

    #[test]
    fn invalid_retirement_boundary_is_rejected() {
        let current = boundary(100, 200);
        let next = boundary(180, 180);
        assert_eq!(
            validate_bridge_validator_set_rotation(current, next),
            Err(BridgeValidatorSetError::InvalidRetirementBoundary)
        );
    }
}
