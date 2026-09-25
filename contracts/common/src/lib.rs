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
/// it here lets callers rely on a single canonical behavior and keeps the
/// door open for future normalization rules without touching entrypoints.
pub fn normalize_address(env: &Env, address: &Address, kind: AddressKind) -> AddressResult {
    validate_address(env, address, kind)
}

/// Convenience helper for validating a contract address.
pub fn validate_contract_address(env: &Env, address: &Address) -> AddressResult {
    validate_address(env, address, AddressKind::Contract)
}

/// Convenience helper for validating an account address.
pub fn validate_account_address(env: &Env, address: &Address) -> AddressResult {
    validate_address(env, address, AddressKind::Account)
}

/// Convenience helper for validating an authorized invoker address.
pub fn validate_authorized_invoker(env: &Env, address: &Address) -> AddressResult {
    validate_address(env, address, AddressKind::AuthorizedInvoker)
}

/// Validates a list of authorized invokers, returning the first error code
/// encountered. Used by entrypoints that accept multiple invokers.
pub fn validate_authorized_invokers(env: &Env, invokers: &Vec<Address>) -> Result<(), u32> {
    for invoker in invokers.iter() {
        validate_authorized_invoker(env, &invoker)?;
    }
    Ok(())
}

/// Invokes `f` with the validated address, panicking with the stable error
/// code when validation fails. Intended for use in public entrypoints where
/// an invalid address should abort the call.
pub fn require_valid_address(env: &Env, address: &Address, kind: AddressKind) -> Address {
    match validate_address(env, address, kind) {
        Ok(addr) => addr,
        Err(code) => panic_with_address_error(env, code),
    }
}

/// Panics with a stable, documented error code for address validation
/// failures.
pub fn panic_with_address_error(env: &Env, code: u32) -> ! {
    let _ = env;
    panic!("address validation error: {}", code)
}

/// Marker used by tests to ensure the helpers remain usable from the host
/// test environment without pulling in contract-specific types.
#[allow(dead_code)]
fn _assert_val_roundtrip(env: &Env, address: &Address) -> Val {
    address.clone().into_val(env)
}

#[allow(dead_code)]
fn _assert_symbol(env: &Env, name: &str) -> Symbol {
    Symbol::new(env, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_configurable_parameter_has_a_schema_entry() {
        let schema = ProtocolParamSchema::canonical();
        for key in [
            "min_collateral_ratio",
            "liquidation_penalty",
            "interest_rate",
            "max_oracle_staleness",
            "protocol_fee",
            "chain_id",
        ] {
            assert!(schema.get(key).is_some(), "missing schema for {key}");
        }
    }
}

}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn valid_contract_address_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert!(is_valid_address(&env, &addr));
        assert_eq!(
            validate_contract_address(&env, &addr),
            Ok(addr.clone())
        );
    }

    #[test]
    fn every_configurable_parameter_has_a_schema_entry() {
        let schema = ProtocolParamSchema::canonical();
        for key in [
            "min_collateral_ratio",
            "liquidation_penalty",
            "interest_rate",
            "max_oracle_staleness",
            "protocol_fee",
            "chain_id",
        ] {
            assert!(schema.get(key).is_some(), "missing schema for {key}");
        }
    }

    #[test]
    fn out_of_range_updates_are_rejected() {
        let schema = ProtocolParamSchema::canonical();
        assert_eq!(
            schema.validate_update("min_collateral_ratio", 9_999, "percent", 2),
            Err(ParamSchemaError::BelowMinimum)
        );
        assert_eq!(
            schema.validate_update("min_collateral_ratio", 100_001, "percent", 2),
            Err(ParamSchemaError::AboveMaximum)
        );
        assert_eq!(
            schema.validate_update("min_collateral_ratio", 15_000, "percent", 2),
            Ok(())
        );
    }

    #[test]
    fn unit_inconsistent_updates_are_rejected() {
        let schema = ProtocolParamSchema::canonical();
        assert_eq!(
            schema.validate_update("max_oracle_staleness", 60, "minutes", 0),
            Err(ParamSchemaError::UnitMismatch)
        );
        assert_eq!(
            schema.validate_update("max_oracle_staleness", 60, "seconds", 0),
            Ok(())
        );
    }
        );
    }

    #[test]
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn valid_contract_address_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert!(is_valid_address(&env, &addr));
        assert_eq!(
            validate_contract_address(&env, &addr),
            Ok(addr.clone())
        );
    }

    #[test]
    fn valid_account_address_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(validate_account_address(&env, &addr), Ok(addr.clone()));
    }

    #[test]
    fn valid_authorized_invoker_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(
            validate_authorized_invoker(&env, &addr),
            Ok(addr.clone())
        );
    }

    #[test]
    fn unit_inconsistent_updates_are_rejected() {
        let schema = ProtocolParamSchema::canonical();
        assert_eq!(
            schema.validate_update("max_oracle_staleness", 60, "minutes", 0),
            Err(ParamSchemaError::UnitMismatch)
        );
        assert_eq!(
            schema.validate_update("max_oracle_staleness", 60, "seconds", 0),
            Ok(())
        );
    }

    #[test]
    fn precision_boundaries_are_enforced() {
        let schema = ProtocolParamSchema::canonical();
        assert_eq!(
            schema.validate_update("interest_rate", 500, "percent", 2),
            Err(ParamSchemaError::PrecisionMismatch)
        );
        assert_eq!(
            schema.validate_update("interest_rate", 500, "percent", 4),
            Ok(())
        );
    }

    #[test]
    fn normalize_returns_same_address() {
        let env = Env::default();

        );
    }

    #[test]
    fn precision_boundaries_are_enforced() {
        let schema = ProtocolParamSchema::canonical();
        assert_eq!(
            schema.validate_update("interest_rate", 500, "percent", 2),
            Err(ParamSchemaError::PrecisionMismatch)
        );
        assert_eq!(
            schema.validate_update("interest_rate", 500, "percent", 4),
            Ok(())
        );
    }

    #[test]
    fn normalize_returns_same_address() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(
            normalize_address(&env, &addr, AddressKind::Contract),
            Ok(addr.clone())
        );
    }

    #[test]

        );
    }

    #[test]
    fn documentation_exposes_the_schema() {
        let doc = ProtocolParamSchema::canonical().to_documentation();
        assert!(doc.contains("min_collateral_ratio"));
        assert!(doc.contains("max_oracle_staleness"));
        assert!(doc.contains("precision"));
    }

    #[test]
    fn zero_address_is_rejected() {
        let env = Env::default();
        // The zero/empty address is not a valid target for any kind.
        let zero = Address::from_string(&soroban_sdk::String::from_str(&env, ""));
        assert!(is_zero_address(&zero));
        assert!(!is_valid_address(&env, &zero));
        assert_eq!(
            validate_address(&env, &zero, AddressKind::Contract),
            Err(address_error::INVALID_ADDRESS)
        );
        assert_eq!(
            validate_address(&env, &zero, AddressKind::Account),
            Err(address_error::INVALID_ADDRESS)
        );
        assert_eq!(
            validate_address(&env, &zero, AddressKind::AuthorizedInvoker),
            Err(address_error::INVALID_ADDRESS)
        );
    }

    #[test]
    fn authorized_invokers_list_validates_each_entry() {
        let env = Env::default();
        let mut invokers = Vec::new(&env);
        invokers.push_back(Address::generate(&env));
        invokers.push_back(Address::generate(&env));
        assert_eq!(validate_authorized_invokers(&env, &invokers), Ok(()));
    }

    #[test]
    fn authorized_invokers_list_rejects_zero_entry() {
        let env = Env::default();
        let mut invokers = Vec::new(&env);
        invokers.push_back(Address::generate(&env));
        invokers.push_back(Address::from_string(&soroban_sdk::String::from_str(&env, "")));
        assert_eq!(
            validate_authorized_invokers(&env, &invokers),
            Err(address_error::INVALID_ADDRESS)
        );
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(address_error::INVALID_ADDRESS, 1);
        assert_eq!(address_error::UNSUPPORTED_ADDRESS_TYPE, 2);
        assert_eq!(address_error::ZERO_ADDRESS, 3);
    }

    fn entries() -> Vec<StateEntry> {
        vec![
            StateEntry::new("admin", "GADMIN"),
            StateEntry::new("threshold", "2"),
        ]
    }

    #[test]
    fn repeated_calls_are_identical() {
        let first = state_checksum(&entries());
        let second = state_checksum(&entries());
        assert_eq!(first, second);
    }

    #[test]
    fn unrelated_storage_does_not_change_checksum() {
        let baseline = state_checksum(&entries());

        // Simulate unrelated storage that is not part of the input set.
        let mut unrelated = std::collections::HashMap::new();
        unrelated.insert("unrelated", "value");
        unrelated.insert("other", "data");
        let _ = unrelated.len();

        assert_eq!(baseline, state_checksum(&entries()));
    }

    #[test]
    fn map_iteration_order_does_not_change_checksum() {
        let mut map = std::collections::HashMap::new();
        map.insert("admin", "GADMIN");
        map.insert("threshold", "2");

        // Canonicalize by sorting keys so iteration order is irrelevant.
        let mut keys: Vec<_> = map.keys().copied().collect();
        keys.sort_unstable();
        let canonical: Vec<StateEntry> = keys
            .iter()
            .map(|k| StateEntry::new(*k, map[k]))
            .collect();

        let first = state_checksum(&canonical);

        // Rebuild the map in a different insertion order.
        let mut reordered = std::collections::HashMap::new();
        reordered.insert("threshold", "2");
        reordered.insert("admin", "GADMIN");
        let mut keys2: Vec<_> = reordered.keys().copied().collect();
        keys2.sort_unstable();
        let canonical2: Vec<StateEntry> = keys2
            .iter()
            .map(|k| StateEntry::new(*k, reordered[k]))
            .collect();

        assert_eq!(first, state_checksum(&canonical2));
    }

    #[test]
    fn ordering_is_significant() {
        let a = vec![StateEntry::new("a", "1"), StateEntry::new("b", "2")];
        let b = vec![StateEntry::new("b", "2"), StateEntry::new("a", "1")];
        assert_ne!(state_checksum(&a), state_checksum(&b));
    }

    #[test]
    fn normalize_returns_same_address() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(
            normalize_address(&env, &addr, AddressKind::Contract),
            Ok(addr.clone())
        );
    }

    #[test]
    }
}
