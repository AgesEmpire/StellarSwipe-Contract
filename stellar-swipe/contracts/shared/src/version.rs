//! Cross-contract version compatibility checks.
//!
//! Each contract stores its version as a `u32` in instance storage.
//! Before any cross-contract call, the caller fetches the callee's version
//! via `get_contract_version` and calls `check_compatible`. If the callee
//! version is below `MIN_COMPATIBLE_VERSION` for that pair, the call is
//! rejected with `ContractError::IncompatibleContractVersion`.
//!
//! # Versioning scheme
//! Versions are monotonically increasing integers. A contract is compatible
//! with any callee whose version is >= its declared `MIN_COMPATIBLE_VERSION`.

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Env};

// ── Per-contract version constants ───────────────────────────────────────────

pub const SIGNAL_REGISTRY_VERSION: u32 = 2;
pub const AUTO_TRADE_VERSION: u32 = 2;
pub const ORACLE_VERSION: u32 = 2;
pub const STAKE_VAULT_VERSION: u32 = 2;

/// Minimum callee version accepted by each caller contract.
/// Callee versions below this are considered incompatible.
pub const MIN_COMPATIBLE_VERSION: u32 = 2;

// ── Storage key ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum VersionKey {
    ContractVersion,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ContractError {
    IncompatibleContractVersion,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Store this contract's version in instance storage.
/// Call once during `initialize`.
pub fn set_contract_version(env: &Env, version: u32) {
    env.storage()
        .instance()
        .set(&VersionKey::ContractVersion, &version);
}

/// Read this contract's stored version (defaults to 1 if never set).
pub fn get_contract_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&VersionKey::ContractVersion)
        .unwrap_or(1)
}

/// Assert that `callee_version` meets `MIN_COMPATIBLE_VERSION`.
///
/// Call this before every cross-contract invocation:
/// ```ignore
/// let callee_ver = get_contract_version(&callee_env);
/// check_compatible(callee_ver)?;
/// ```
pub fn check_compatible(callee_version: u32) -> Result<(), ContractError> {
    if callee_version < MIN_COMPATIBLE_VERSION {
        Err(ContractError::IncompatibleContractVersion)
    } else {
        Ok(())
    }
}

/// Convenience: emit a version-check event (optional, for observability).
pub fn emit_version_checked(env: &Env, callee_version: u32, compatible: bool) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("ver_chk"), callee_version),
        compatible,
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn setup() -> Env {
        Env::default()
    }

    #[test]
    fn test_compatible_version_passes() {
        assert!(check_compatible(MIN_COMPATIBLE_VERSION).is_ok());
        assert!(check_compatible(MIN_COMPATIBLE_VERSION + 5).is_ok());
    }

    #[test]
    fn test_incompatible_version_fails() {
        assert_eq!(
            check_compatible(MIN_COMPATIBLE_VERSION - 1),
            Err(ContractError::IncompatibleContractVersion)
        );
        assert_eq!(
            check_compatible(0),
            Err(ContractError::IncompatibleContractVersion)
        );
    }

    #[test]
    fn test_set_and_get_version() {
        let env = setup();
        let contract_addr = env.register(TestContract, ());

        env.as_contract(&contract_addr, || {
            // Default before set
            assert_eq!(get_contract_version(&env), 1);

            set_contract_version(&env, SIGNAL_REGISTRY_VERSION);
            assert_eq!(get_contract_version(&env), SIGNAL_REGISTRY_VERSION);
        });
    }

    #[test]
    fn test_cross_contract_call_blocked_for_old_callee() {
        // Simulate: caller is v2, callee is v1 (legacy, incompatible)
        let callee_version = 1u32;
        let result = check_compatible(callee_version);
        assert_eq!(result, Err(ContractError::IncompatibleContractVersion));
    }

    #[test]
    fn test_cross_contract_call_allowed_for_current_callee() {
        let callee_version = AUTO_TRADE_VERSION;
        assert!(check_compatible(callee_version).is_ok());
    }
}
