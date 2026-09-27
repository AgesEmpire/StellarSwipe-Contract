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
    pub key: String,
    /// Normalized outcome of the renewal attempt.
    pub result: TtlRenewalResult,
    /// TTL observed before the attempt, in ledgers.
    pub current_ttl: u32,
    /// TTL below which renewal is attempted, in ledgers.
    pub threshold: u32,
    /// TTL requested when a renewal is performed, in ledgers.
    pub extend_to: u32,
}

/// Normalized result of a storage TTL renewal attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtlRenewalResult {
    /// The entry was already expired; renewal is impossible.
    Expired,
    /// The entry's TTL was above the threshold; no renewal was needed.
    Skipped,
    /// The entry's TTL was extended to `extend_to`.
    Renewed,
    /// The renewal attempt failed.
    Failed,
}

impl TtlRenewalOutcome {
    /// Build a normalized outcome for a renewal attempt.
    ///
    /// The result is derived deterministically from the observed TTL:
    /// an already-expired entry (`current_ttl == 0`) yields
    /// [`TtlRenewalResult::Expired`], an entry at or above `threshold`
    /// yields [`TtlRenewalResult::Skipped`], and anything else yields
    /// [`TtlRenewalResult::Renewed`].
    pub fn new(
        key: impl Into<String>,
        current_ttl: u32,
        threshold: u32,
        extend_to: u32,
    ) -> Self {
        let result = if current_ttl == 0 {
            TtlRenewalResult::Expired
        } else if current_ttl >= threshold {
            TtlRenewalResult::Skipped
        } else {
            TtlRenewalResult::Renewed
        };
        Self {
            version: TTL_RENEWAL_OUTCOME_EVENT_VERSION,
            key: key.into(),
            result,
            current_ttl,
            threshold,
            extend_to,
        }
    }

    /// Build a normalized outcome for a failed renewal attempt.
    pub fn failed(
        key: impl Into<String>,
        current_ttl: u32,
        threshold: u32,
        extend_to: u32,
    ) -> Self {
        Self {
            version: TTL_RENEWAL_OUTCOME_EVENT_VERSION,
            key: key.into(),
            result: TtlRenewalResult::Failed,
            current_ttl,
            threshold,
            extend_to,
        }
    }

    /// Whether this outcome represents an actual TTL extension.
    pub fn is_renewed(&self) -> bool {
        self.result == TtlRenewalResult::Renewed
    }
}

/// A single key/value entry in contract storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateEntry {
    /// Storage key.
    pub key: String,
    /// Storage value.
    pub value: String,
}

/// A captured, read-only view of contract storage.
///
/// A snapshot is the sole input to a migration dry-run: it is never mutated,
/// so running a dry-run cannot write to ledger state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerSnapshot {
    /// Storage entries in canonical (sorted) key order.
    pub entries: Vec<StateEntry>,
}

impl LedgerSnapshot {
    /// Build a snapshot, sorting entries into canonical key order so that
    /// downstream output is deterministic regardless of input order.
    pub fn new(mut entries: Vec<StateEntry>) -> Self {
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        Self { entries }
    }

    /// Look up the value stored under `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.value.as_str())
    }
}
