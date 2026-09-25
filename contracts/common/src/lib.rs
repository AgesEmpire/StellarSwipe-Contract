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

/// A single planned change produced by a migration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationOp {
    /// A new key will be written.
    Insert,
    /// An existing key will be overwritten.
    Update,
    /// An existing key will be removed.
    Delete,
}

/// A planned key change, with the before/after values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedChange {
    /// Storage key affected by the change.
    pub key: String,
    /// Operation that would be applied.
    pub op: MigrationOp,
    /// Value before the migration (`None` for inserts).
    pub before: Option<String>,
    /// Value after the migration (`None` for deletes).
    pub after: Option<String>,
}

/// A migration invariant that failed during a dry-run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantFailure {
    /// Stable identifier of the invariant that failed.
    pub invariant: String,
    /// Human-readable explanation of the failure.
    pub detail: String,
}

/// Deterministic, machine-readable report produced by a migration dry-run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationDryRunReport {
    /// Always `true`; a dry-run never commits state.
    pub dry_run: bool,
    /// Checksum of the input snapshot.
    pub snapshot_checksum: String,
    /// Planned key changes, in canonical key order.
    pub changes: Vec<PlannedChange>,
    /// Number of planned inserts.
    pub inserts: usize,
    /// Number of planned updates.
    pub updates: usize,
    /// Number of planned deletes.
    pub deletes: usize,
    /// Invariants that failed; empty when the migration is clean.
    pub invariant_failures: Vec<InvariantFailure>,
}

impl MigrationDryRunReport {
    /// Whether the dry-run completed without any invariant failures.
    pub fn is_clean(&self) -> bool {
        self.invariant_failures.is_empty()
    }

    /// Render the report as deterministic, machine-readable JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("migration dry-run report is always serializable")
    }
}

/// A migration that can be dry-run against a captured ledger snapshot.
///
/// Implementations must be pure: `plan` only reads the snapshot and returns
/// the changes it *would* apply, never mutating ledger state.
pub trait StorageMigration {
    /// Compute the planned changes for `snapshot` without committing them.
    fn plan(&self, snapshot: &LedgerSnapshot) -> Vec<PlannedChange>;

    /// Check invariants over the planned changes.
    ///
    /// The default implementation reports no failures.
    fn check_invariants(
        &self,
        _snapshot: &LedgerSnapshot,
        _changes: &[PlannedChange],
    ) -> Vec<InvariantFailure> {
        Vec::new()
    }
}

/// Run a migration in dry-run mode against a captured ledger snapshot.
///
/// This is read-only: it never writes to ledger state. The returned report is
/// deterministic for a given snapshot and migration, and can be serialized to
/// stable JSON via [`MigrationDryRunReport::to_json`].
pub fn dry_run_migration<M: StorageMigration>(
    migration: &M,
    snapshot: &LedgerSnapshot,
) -> MigrationDryRunReport {
    let mut changes = migration.plan(snapshot);
    changes.sort_by(|a, b| a.key.cmp(&b.key));

    let mut inserts = 0;
    let mut updates = 0;
    let mut deletes = 0;
    for change in &changes {
        match change.op {
            MigrationOp::Insert => inserts += 1,
            MigrationOp::Update => updates += 1,
            MigrationOp::Delete => deletes += 1,
        }
    }

    let invariant_failures = migration.check_invariants(snapshot, &changes);

    MigrationDryRunReport {
        dry_run: true,
        snapshot_checksum: state_checksum(&snapshot.entries),
        changes,
        inserts,
        updates,
        deletes,
        invariant_failures,
    }
}

/// Compute a deterministic checksum over the provided state entries.
///
/// The caller is responsible for supplying `entries` in a canonical order.
/// Equivalent states (same entries, same order) always yield the same digest,
/// regardless of unrelated storage or map iteration order.
///
/// Returns the digest as a lowercase hexadecimal string.
pub fn state_checksum(entries: &[StateEntry]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CHECKSUM_DOMAIN);

    for entry in entries {
        let key = entry.key.as_bytes();
        hasher.update((key.len() as u32).to_be_bytes());
        hasher.update(key);

        let value = entry.value.as_bytes();
        hasher.update((value.len() as u32).to_be_bytes());
        hasher.update(value);
    }

    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Validates and normalizes an address for the given `kind`.
///
/// Normalization currently returns the address unchanged, but centralizing
/// it here lets callers

/* … truncated 11497 chars — edit only what you need near the top … */
