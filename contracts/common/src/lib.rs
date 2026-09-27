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

/// A versioned Soroban event schema.
///
/// The `version` is bumped whenever the payload layout changes in a way that
/// indexers must be able to detect. The `fields` list is the canonical,
/// ordered list of payload field names; ordering is significant and must not
/// depend on map iteration order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSchema {
    /// Stable event name (e.g. `"ttl_renewal_outcome"`).
    pub name: &'static str,
    /// Monotonic schema version for this event.
    pub version: u32,
    /// Canonical, ordered payload field names.
    pub fields: Vec<&'static str>,
}

impl EventSchema {
    /// Compute the deterministic schema hash for this event.
    ///
    /// The hash is derived purely from the event name, version, and the
    /// canonical field list, so it is stable across builds and platforms.
    /// Any change to the name, version, or field list (including field order)
    /// alters the hash predictably.
    pub fn schema_hash(&self) -> u64 {
        // FNV-1a 64-bit: deterministic, dependency-free, and stable across
        // builds. Inputs are length-prefixed so that distinct field lists
        // cannot collide by concatenation.
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut hash = FNV_OFFSET;
        let mut write = |bytes: &[u8]| {
            for b in bytes {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
            }
        };

        write(self.name.as_bytes());
        write(&self.version.to_le_bytes());
        write(&(self.fields.len() as u64).to_le_bytes());
        for field in &self.fields {
            write(&(field.len() as u64).to_le_bytes());
            write(field.as_bytes());
        }
        hash
    }
}

/// Canonical schema for the TTL renewal outcome event.
pub const TTL_RENEWAL_OUTCOME_SCHEMA: EventSchema = EventSchema {
    name: "ttl_renewal_outcome",
    version: 1,
    fields: vec!["key", "renewed", "new_ttl", "schema_hash"],
};

/// Compute the canonical schema hash for the TTL renewal outcome event.
pub fn ttl_renewal_outcome_schema_hash() -> u64 {
    TTL_RENEWAL_OUTCOME_SCHEMA.schema_hash()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_renewal_outcome_schema_hash_is_canonical() {
        // Fixture test: locks the canonical hash value. If this changes, the
        // event schema changed and indexers must be updated.
        assert_eq!(ttl_renewal_outcome_schema_hash(), 0x9d3c_1f6a_2b7e_4c05);
    }

    #[test]
    fn schema_hash_is_stable_and_order_sensitive() {
        let base = EventSchema {
            name: "demo",
            version: 1,
            fields: vec!["a", "b"],
        };
        assert_eq!(base.schema_hash(), base.schema_hash());

        let reordered = EventSchema {
            name: "demo",
            version: 1,
            fields: vec!["b", "a"],
        };
        assert_ne!(base.schema_hash(), reordered.schema_hash());

        let bumped = EventSchema {
            name: "demo",
            version: 2,
            fields: vec!["a", "b"],
        };
        assert_ne!(base.schema_hash(), bumped.schema_hash());
    }
}
