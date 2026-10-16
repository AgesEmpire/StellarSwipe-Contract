//! Shared types and helpers for the protocol contracts.

pub mod decimal;

use serde::{Deserialize, Serialize};

/// Error returned when a bounded vector fails validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundedVecError {
    /// The encoded element count exceeds the configured maximum.
    TooManyElements,
    /// The encoded byte length exceeds the configured maximum.
    TooLarge,
    /// The encoded byte length does not match the element count.
    LengthMismatch,
}

/// Decode a Soroban vector from its encoded representation, validating the
/// element count and encoded size before any allocation occurs.
///
/// `encoded` is the raw byte payload of the vector, `element_size` is the
/// fixed encoded size of a single element, `max_elements` bounds the number
/// of elements, and `max_bytes` bounds the total encoded size. Validation is
/// performed up front so malformed or oversized inputs fail without any
/// partial state changes.
pub fn decode_bounded_vec(
    encoded: &[u8],
    element_size: usize,
    max_elements: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, BoundedVecError> {
    if element_size == 0 {
        return Err(BoundedVecError::LengthMismatch);
    }
    if encoded.len() > max_bytes {
        return Err(BoundedVecError::TooLarge);
    }
    if encoded.len() % element_size != 0 {
        return Err(BoundedVecError::LengthMismatch);
    }
    let count = encoded.len() / element_size;
    if count > max_elements {
        return Err(BoundedVecError::TooManyElements);
    }
    Ok(encoded.to_vec())
}

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
    /// The parameter key is not present in the schema (issue #1270).
    UnknownParameter,
    /// The caller lacks the authority required by the parameter's permission
    /// (issue #1270).
    MissingAuthority,
    /// The parameter is immutable after initialization and cannot be updated
    /// (issue #1270).
    ImmutableParameter,
    /// The value cannot be represented by the parameter's numeric type, e.g. a
    /// negative value for [`ParamType::Uint`] (issue #1270).
    UnsupportedParamType,
    /// The supplied precision exceeds the supported maximum (issue #1271).
    UnsupportedPrecision,
}

/// Maximum decimal precision supported for a protocol parameter (issue #1271).
///
/// Precision scales a stored integer value by `10^precision`, so it is bounded
/// to keep scaling, conversion, and overflow behavior predictable across host
/// versions. Precision values above this bound are rejected when a schema is
/// constructed or loaded, before any conversion or state mutation occurs.
pub const MAX_PARAM_PRECISION: u32 = 18;

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
    ///
    /// An out-of-range precision is reported as
    /// [`ParamSchemaError::UnsupportedPrecision`] rather than the looser
    /// [`ParamSchemaError::PrecisionMismatch`], so callers can distinguish
    /// "unsupported precision" from "precision differs from the schema"
    /// (issue #1271).
    pub fn validate_with_precision(
        &self,
        value: i128,
        unit: &str,
        precision: u32,
    ) -> Result<(), ParamSchemaError> {
        Self::validate_precision(precision)?;
        if precision != self.precision {
            return Err(ParamSchemaError::PrecisionMismatch);
        }
        self.validate(value, unit)
    }

    /// Validate that `precision` lies within the supported range
    /// `0..=[MAX_PARAM_PRECISION]` (issue #1271).
    ///
    /// This is checked whenever a schema is constructed, loaded, or used to
    /// validate an update, so an unsupported precision fails with a stable
    /// error before any conversion or state mutation.
    pub fn validate_precision(precision: u32) -> Result<(), ParamSchemaError> {
        if precision > MAX_PARAM_PRECISION {
            return Err(ParamSchemaError::UnsupportedPrecision);
        }
        Ok(())
    }

    /// Construct a schema entry, rejecting an unsupported precision up front.
    ///
    /// Returns the assembled [`ParamSchema`] on success so an out-of-range
    /// precision cannot be persisted (issue #1271).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: &'static str,
        param_type: ParamType,
        unit: &'static str,
        precision: u32,
        min: i128,
        max: i128,
        permission: UpdatePermission,
    ) -> Result<Self, ParamSchemaError> {
        Self::validate_precision(precision)?;
        Ok(Self {
            key,
            param_type,
            unit,
            precision,
            min,
            max,
            permission,
        })
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
    ///
    /// An unknown key is reported as [`ParamSchemaError::UnknownParameter`]
    /// (issue #1270) so it is distinguishable from a unit mismatch instead of
    /// being silently attributed to the wrong reason.
    pub fn validate_update(
        &self,
        key: &str,
        value: i128,
        unit: &str,
        precision: u32,
    ) -> Result<(), ParamSchemaError> {
        let schema = self.get(key).ok_or(ParamSchemaError::UnknownParameter)?;
        // A value the parameter's numeric type cannot represent is rejected
        // with a reason of its own rather than a range error (issue #1270).
        if schema.param_type == ParamType::Uint && value < 0 {
            return Err(ParamSchemaError::UnsupportedParamType);
        }
        schema.validate_with_precision(value, unit, precision)
    }

    /// Enforce the update permission recorded for `key` (issue #1270).
    ///
    /// The caller reports which authorities it holds; the schema's recorded
    /// [`UpdatePermission`] decides whether that is sufficient:
    /// - [`UpdatePermission::Immutable`] is never updatable.
    /// - [`UpdatePermission::Governance`] requires `is_governance`.
    /// - [`UpdatePermission::Admin`] requires `is_admin`.
    ///
    /// A caller that holds neither authority, or that targets an unknown key,
    /// receives a specific error rather than a generic rejection.
    pub fn check_update_permission(
        &self,
        key: &str,
        is_governance: bool,
        is_admin: bool,
    ) -> Result<(), ParamSchemaError> {
        let schema = self.get(key).ok_or(ParamSchemaError::UnknownParameter)?;
        match schema.permission {
            UpdatePermission::Immutable => Err(ParamSchemaError::ImmutableParameter),
            UpdatePermission::Governance => {
                if is_governance {
                    Ok(())
                } else {
                    Err(ParamSchemaError::MissingAuthority)
                }
            }
            UpdatePermission::Admin => {
                if is_admin {
                    Ok(())
                } else {
                    Err(ParamSchemaError::MissingAuthority)
                }
            }
        }
    }

    /// Validate a complete configuration update: authority, key existence,
    /// numeric type, unit, precision, and range (issues #1270, #1271).
    ///
    /// Permission is checked first so an unauthorized caller learns that fact
    /// before any value-based validation is performed.
    pub fn validate_config_update(
        &self,
        key: &str,
        value: i128,
        unit: &str,
        precision: u32,
        is_governance: bool,
        is_admin: bool,
    ) -> Result<(), ParamSchemaError> {
        self.check_update_permission(key, is_governance, is_admin)?;
        self.validate_update(key, value, unit, precision)
    }

    /// Validate that every entry in the schema uses a supported precision
    /// (issue #1271).
    ///
    /// Schemas are validated as a whole when they are constructed or loaded,
    /// so an out-of-range precision anywhere in the set is rejected before it
    /// can be deployed.
    pub fn validate_precisions(&self) -> Result<(), ParamSchemaError> {
        for p in &self.params {
            ParamSchema::validate_precision(p.precision)?;
        }
        Ok(())
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
    /// S

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

#[cfg(test)]
mod descriptive_config_update_error_tests {
    use super::*;

    fn schema() -> ProtocolParamSchema {
        ProtocolParamSchema::canonical()
    }

    // Issue #1270: every rejection condition reports its own typed error.

    #[test]
    fn unknown_parameter_key_is_reported_by_name() {
        assert_eq!(
            schema().validate_update("no_such_param", 1, "percent", 2),
            Err(ParamSchemaError::UnknownParameter)
        );
        assert_eq!(
            schema().check_update_permission("no_such_param", true, true),
            Err(ParamSchemaError::UnknownParameter)
        );
    }

    #[test]
    fn immutable_parameter_is_never_updatable() {
        // `chain_id` is immutable, so even full authority is refused.
        assert_eq!(
            schema().check_update_permission("chain_id", true, true),
            Err(ParamSchemaError::ImmutableParameter)
        );
        assert_eq!(
            schema().validate_config_update("chain_id", 1, "id", 0, true, true),
            Err(ParamSchemaError::ImmutableParameter)
        );
    }

    #[test]
    fn governance_parameter_requires_governance_authority() {
        // `min_collateral_ratio` is governance-governed.
        assert_eq!(
            schema().check_update_permission("min_collateral_ratio", false, true),
            Err(ParamSchemaError::MissingAuthority)
        );
        assert!(schema()
            .check_update_permission("min_collateral_ratio", true, false)
            .is_ok());
    }

    #[test]
    fn admin_parameter_requires_admin_authority() {
        // `max_oracle_staleness` is admin-governed, so governance alone fails.
        assert_eq!(
            schema().check_update_permission("max_oracle_staleness", true, false),
            Err(ParamSchemaError::MissingAuthority)
        );
        assert!(schema()
            .check_update_permission("max_oracle_staleness", false, true)
            .is_ok());
    }

    #[test]
    fn unit_precision_and_range_rejections_are_distinct() {
        // `liquidation_penalty` is Decimal, "percent", precision 2, 0..=5_000.
        let s = schema();
        assert_eq!(
            s.validate_update("liquidation_penalty", 1, "bps", 2),
            Err(ParamSchemaError::UnitMismatch)
        );
        assert_eq!(
            s.validate_update("liquidation_penalty", 1, "percent", 4),
            Err(ParamSchemaError::PrecisionMismatch)
        );
        assert_eq!(
            s.validate_update("liquidation_penalty", -1, "percent", 2),
            Err(ParamSchemaError::BelowMinimum)
        );
        assert_eq!(
            s.validate_update("liquidation_penalty", 5_001, "percent", 2),
            Err(ParamSchemaError::AboveMaximum)
        );
        // Boundaries are inclusive, so neither edge is rejected.
        assert!(s.validate_update("liquidation_penalty", 0, "percent", 2).is_ok());
        assert!(s
            .validate_update("liquidation_penalty", 5_000, "percent", 2)
            .is_ok());
    }

    #[test]
    fn negative_value_for_unsigned_parameter_is_rejected_by_type() {
        // `max_oracle_staleness` is Uint, so a negative value is a type
        // error rather than merely out of range.
        assert_eq!(
            schema().validate_update("max_oracle_staleness", -1, "seconds", 0),
            Err(ParamSchemaError::UnsupportedParamType)
        );
    }

    #[test]
    fn permission_is_checked_before_value_validation() {
        // Missing authority is reported even when the value would also fail.
        assert_eq!(
            schema().validate_config_update(
                "min_collateral_ratio",
                999_999,
                "wrong_unit",
                99,
                false,
                false
            ),
            Err(ParamSchemaError::MissingAuthority)
        );
    }

    #[test]
    fn valid_config_update_is_accepted() {
        // 150.00% collateral ratio at precision 2 as governance.
        assert!(schema()
            .validate_config_update("min_collateral_ratio", 15_000, "percent", 2, true, false)
            .is_ok());
    }
}

#[cfg(test)]
mod parameter_precision_bound_tests {
    use super::*;

    // Issue #1271: the supported precision range is documented and enforced.

    #[test]
    fn lowest_supported_precision_is_accepted() {
        assert_eq!(ParamSchema::validate_precision(0), Ok(()));
    }

    #[test]
    fn highest_supported_precision_is_accepted() {
        assert_eq!(
            ParamSchema::validate_precision(MAX_PARAM_PRECISION),
            Ok(())
        );
    }

    #[test]
    fn precision_just_above_bound_is_rejected() {
        assert_eq!(
            ParamSchema::validate_precision(MAX_PARAM_PRECISION + 1),
            Err(ParamSchemaError::UnsupportedPrecision)
        );
    }

    #[test]
    fn update_with_out_of_range_precision_reports_precision_error() {
        let s = ProtocolParamSchema::canonical();
        assert_eq!(
            s.validate_update("liquidation_penalty", 1, "percent", MAX_PARAM_PRECISION + 1),
            Err(ParamSchemaError::UnsupportedPrecision)
        );
    }

    #[test]
    fn schema_construction_rejects_out_of_range_precision() {
        assert_eq!(
            ParamSchema::new(
                "bad_param",
                ParamType::Decimal,
                "percent",
                MAX_PARAM_PRECISION + 1,
                0,
                100,
                UpdatePermission::Governance
            ),
            Err(ParamSchemaError::UnsupportedPrecision)
        );
        // The same construction at the bound succeeds.
        assert!(ParamSchema::new(
            "ok_param",
            ParamType::Decimal,
            "percent",
            MAX_PARAM_PRECISION,
            0,
            100,
            UpdatePermission::Governance
        )
        .is_ok());
    }

    #[test]
    fn canonical_schema_uses_only_supported_precisions() {
        assert!(ProtocolParamSchema::canonical().validate_precisions().is_ok());
        // A schema carrying an unsupported precision fails as a whole.
        let mut s = ProtocolParamSchema::canonical();
        s.params[0].precision = MAX_PARAM_PRECISION + 1;
        assert_eq!(
            s.validate_precisions(),
            Err(ParamSchemaError::UnsupportedPrecision)
        );
    }
}
