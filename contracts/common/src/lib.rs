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

/// Error returned when a storage key does not belong to a registered namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceError {
    /// The key does not carry a namespace prefix.
    MissingNamespace,
    /// The key's namespace prefix is not registered.
    UnknownNamespace,
    /// The namespace was declared more than once in the registry.
    DuplicateNamespace,
}

/// A reserved storage key namespace.
///
/// Every storage key is expected to be of the form `"<namespace>:<rest>"`,
/// where `<namespace>` is one of the entries declared in
/// [`StorageNamespaceRegistry::canonical`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageNamespace {
    /// Namespace prefix, without the trailing `:` separator.
    pub name: &'static str,
    /// Human-readable description of what the namespace owns.
    pub description: &'static str,
}

/// Central registry of every reserved storage key namespace.
///
/// This is the single source of truth for namespace prefixes: contracts must
/// not hard-code their own prefixes, they should validate keys against this
/// registry instead. Declaring the same namespace twice is rejected by
/// [`StorageNamespaceRegistry::validate`], which is exercised by CI tests so
/// duplicate declarations fail the build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageNamespaceRegistry {
    /// All reserved namespaces, in a stable order.
    pub namespaces: Vec<StorageNamespace>,
}

impl StorageNamespaceRegistry {
    /// Build the canonical registry of reserved storage namespaces.
    pub fn canonical() -> Self {
        Self {
            namespaces: vec![
                StorageNamespace {
                    name: "param",
                    description: "Protocol parameter values and their metadata.",
                },
                StorageNamespace {
                    name: "state",
                    description: "Generic contract state entries.",
                },
                StorageNamespace {
                    name: "migration",
                    description: "Migration bookkeeping and applied-version markers.",
                },
                StorageNamespace {
                    name: "oracle",
                    description: "Oracle price feeds and staleness metadata.",
                },
                StorageNamespace {
                    name: "admin",
                    description: "Admin roles, ownership and pause flags.",
                },
            ],
        }
    }

    /// Look up a namespace by its prefix.
    pub fn get(&self, name: &str) -> Option<&StorageNamespace> {
        self.namespaces.iter().find(|n| n.name == name)
    }

    /// Return `true` if `name` is a registered namespace.
    pub fn is_registered(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Validate the registry itself, rejecting duplicate declarations.
    ///
    /// CI runs this against [`StorageNamespaceRegistry::canonical`] so that a
    /// duplicated namespace fails the build instead of silently colliding.
    pub fn validate(&self) -> Result<(), NamespaceError> {
        for (i, ns) in self.namespaces.iter().enumerate() {
            if self.namespaces[i + 1..].iter().any(|other| other.name == ns.name) {
                return Err(NamespaceError::DuplicateNamespace);
            }
        }
        Ok(())
    }

    /// Split a storage key into its namespace prefix and remainder.
    pub fn split_key(key: &str) -> Result<(&str, &str), NamespaceError> {
        match key.split_once(':') {
            Some((namespace, rest)) if !namespace.is_empty() => Ok((namespace, rest)),
            _ => Err(NamespaceError::MissingNamespace),
        }
    }

    /// Validate that `key` belongs to a registered namespace.
    ///
    /// Returns the namespace prefix on success, or a [`NamespaceError`] when
    /// the key is malformed or uses an unknown namespace.
    pub fn validate_key(&self, key: &str) -> Result<&str, NamespaceError> {
        let (namespace, _) = Self::split_key(key)?;
        if self.is_registered(namespace) {
            Ok(namespace)
        } else {
            Err(NamespaceError::UnknownNamespace)
        }
    }

    /// Build a namespaced storage key from a registered namespace and suffix.
    pub fn make_key(&self, namespace: &str, suffix: &str) -> Result<String, NamespaceError> {
        if !self.is_registered(namespace) {
            return Err(NamespaceError::UnknownNamespace);
        }
        Ok(format!("{}:{}", namespace, suffix))
    }
}

/// Legacy storage keys that predate the namespace registry.
///
/// Migration fixtures use this list to assert that old, un-namespaced keys are
/// still readable and can be rewritten into their namespaced equivalents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyKeyFixture {
    /// The old, un-namespaced storage key.
    pub legacy_key: &'static str,
    /// The namespace the key should migrate into.
    pub namespace: &'static str,
    /// The namespaced key the legacy key maps to.
    pub migrated_key: &'static str,
}

impl LegacyKeyFixture {
    /// Canonical fixtures covering the legacy keys seen in production.
    pub fn canonical() -> Vec<Self> {
        vec![
            LegacyKeyFixture {
                legacy_key: "min_collateral_ratio",
                namespace: "param",
                migrated_key: "param:min_collateral_ratio",
            },
            LegacyKeyFixture {
                legacy_key: "liquidation_penalty",
                namespace: "param",
                migrated_key: "param:liquidation_penalty",
            },
            LegacyKeyFixture {
                legacy_key: "interest_rate",
                namespace: "param",
                migrated_key: "param:interest_rate",
            },
            LegacyKeyFixture {
                legacy_key: "max_oracle_staleness",
                namespace: "oracle",
                migrated_key: "oracle:max_oracle_staleness",
            },
            LegacyKeyFixture {
                legacy_key: "protocol_fee",
                namespace: "param",
                migrated_key: "param:protocol_fee",
            },
            LegacyKeyFixture {
                legacy_key: "chain_id",
                namespace: "param",
                migrated_key: "param:chain_id",
            },
            LegacyKeyFixture {
                legacy_key: "admin",
                namespace: "admin",
                migrated_key: "admin:admin",
            },
            LegacyKeyFixture {
                legacy_key: "paused",
                namespace: "admin",
                migrated_key: "admin:paused",
            },
        ]
    }

    /// Validate that every fixture maps into a registered namespace and that
    /// the migrated key round-trips through the registry.
    pub fn validate_all(
        &self,
        registry: &StorageNamespaceRegistry,
    ) -> Result<(), NamespaceError> {
        if !registry.is_registered(self.namespace) {
            return Err(NamespaceError::UnknownNamespace);
        }
        let expected = registry.make_key(self.namespace, self.legacy_key)?;
        if expected != self.migrated_key {
            return Err(NamespaceError::UnknownNamespace);
        }
        registry.validate_key(self.migrated_key).map(|_| ())
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;

    #[test]
    fn canonical_registry_has_no_duplicates() {
        let registry = StorageNamespaceRegistry::canonical();
        assert_eq!(registry.validate(), Ok(()));
    }

    #[test]
    fn duplicate_namespace_is_rejected() {
        let registry = StorageNamespaceRegistry {
            namespaces: vec![
                StorageNamespace {
                    name: "param",
                    description: "first",
                },
                StorageNamespace {
                    name: "param",
                    description: "duplicate",
                },
            ],
        };
        assert_eq!(registry.validate(), Err(NamespaceError::DuplicateNamespace));
    }

    #[test]
    fn unknown_namespace_is_rejected() {
        let registry = StorageNamespaceRegistry::canonical();
        assert_eq!(
            registry.validate_key("bogus:key"),
            Err(NamespaceError::UnknownNamespace)
        );
        assert_eq!(
            registry.validate_key("no_separator"),
            Err(NamespaceError::MissingNamespace)
        );
    }

    #[test]
    fn registered_namespace_is_accepted() {
        let registry = StorageNamespaceRegistry::canonical();
        assert_eq!(registry.validate_key("param:interest_rate"), Ok("param"));
        assert_eq!(registry.make_key("param", "interest_rate").as_deref(), Ok("param:interest_rate"));
        assert_eq!(
            registry.make_key("bogus", "x"),
            Err(NamespaceError::UnknownNamespace)
        );
    }

    #[test]
    fn legacy_fixtures_migrate_into_registered_namespaces() {
        let registry = StorageNamespaceRegistry::canonical();
        for fixture in LegacyKeyFixture::canonical() {
            assert_eq!(fixture.validate_all(&registry), Ok(()));
        }
    }
}
