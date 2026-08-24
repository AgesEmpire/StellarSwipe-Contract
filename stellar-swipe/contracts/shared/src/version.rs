//! Cross-contract version compatibility checks.
//!
//! Each contract stores its version as a `u32` in instance storage via
//! [`set_contract_version`]. Before any cross-contract call, the caller fetches
//! the callee's version and calls [`require_compatible`] (panics on
//! incompatibility) or [`check_compatible`] (returns `Result`).
//!
//! # Versioning scheme
//! Versions are monotonically increasing integers. Compatibility is
//! per-callee-kind: each contract kind declares a minimum acceptable callee
//! version in [`min_version_for`].

use soroban_sdk::{contracterror, contracttype, panic_with_error, symbol_short, Env};

// ── Per-contract version constants ───────────────────────────────────────────

pub const SIGNAL_REGISTRY_VERSION: u32 = 2;
pub const AUTO_TRADE_VERSION: u32 = 2;
pub const ORACLE_VERSION: u32 = 2;
pub const STAKE_VAULT_VERSION: u32 = 2;
pub const FEE_COLLECTOR_VERSION: u32 = 2;

/// Identifies which contract kind we are checking compatibility against.
/// Add new variants here as new contract pairs are introduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractKind {
    SignalRegistry,
    AutoTrade,
    Oracle,
    StakeVault,
    FeeCollector,
}

/// Returns the minimum acceptable version for a given callee contract kind.
/// Versions below this will be rejected as incompatible.
pub fn min_version_for(kind: ContractKind) -> u32 {
    match kind {
        ContractKind::SignalRegistry => 2,
        ContractKind::AutoTrade => 2,
        ContractKind::Oracle => 2,
        ContractKind::StakeVault => 2,
        ContractKind::FeeCollector => 2,
    }
}

// ── Storage key ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum VersionKey {
    ContractVersion,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VersionError {
    IncompatibleContractVersion = 1,
    /// An `upgrade()` call proposed a `new_version` that is not strictly
    /// greater than the contract's currently stored version.
    DowngradeRejected = 2,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Store this contract's version in instance storage. Call once during `initialize`.
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

/// Returns `Ok(())` if `callee_version >= min_version_for(kind)`, otherwise
/// `Err(VersionError::IncompatibleContractVersion)`.
///
/// Use this when you want to propagate the error up the call chain.
pub fn check_compatible(callee_version: u32, kind: ContractKind) -> Result<(), VersionError> {
    if callee_version < min_version_for(kind) {
        Err(VersionError::IncompatibleContractVersion)
    } else {
        Ok(())
    }
}

/// Panics (via `soroban_sdk::panic_with_error!`) if the callee version is
/// incompatible. Use this inside `#[contractimpl]` methods where you want the
/// SDK to encode the error into the invocation result automatically.
pub fn require_compatible(env: &Env, callee_version: u32, kind: ContractKind) {
    if callee_version < min_version_for(kind) {
        panic_with_error!(env, VersionError::IncompatibleContractVersion);
    }
}

/// Convenience: emit a version-check event for observability.
pub fn emit_version_checked(env: &Env, callee_version: u32, compatible: bool) {
    env.events()
        .publish((symbol_short!("ver_chk"), callee_version), compatible);
}

/// Guard an `upgrade()` call: the proposed `new_version` must be strictly
/// greater than `current_version`.
///
/// Upgrades must move forward — re-deploying the same version or an older
/// one is rejected. This prevents an admin key compromise (or a scripting
/// mistake) from silently rolling a contract back to a version with a known
/// bug or an incompatible storage layout. Pair with [`set_contract_version`]
/// after a successful [`soroban_sdk::deploy::Deployer::update_current_contract_wasm`]
/// call so the stored version and the deployed code stay in lockstep.
pub fn guard_upgrade(current_version: u32, new_version: u32) -> Result<(), VersionError> {
    if new_version <= current_version {
        Err(VersionError::DowngradeRejected)
    } else {
        Ok(())
    }
}

/// Emit an event recording a completed contract upgrade (old → new version).
pub fn emit_contract_upgraded(env: &Env, old_version: u32, new_version: u32) {
    env.events()
        .publish((symbol_short!("upgraded"),), (old_version, new_version));
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

    // --- check_compatible ---

    #[test]
    fn compatible_version_passes() {
        for kind in [
            ContractKind::SignalRegistry,
            ContractKind::AutoTrade,
            ContractKind::Oracle,
            ContractKind::StakeVault,
            ContractKind::FeeCollector,
        ] {
            let min = min_version_for(kind);
            assert!(check_compatible(min, kind).is_ok());
            assert!(check_compatible(min + 5, kind).is_ok());
        }
    }

    #[test]
    fn incompatible_version_fails() {
        for kind in [
            ContractKind::SignalRegistry,
            ContractKind::AutoTrade,
            ContractKind::Oracle,
            ContractKind::StakeVault,
            ContractKind::FeeCollector,
        ] {
            let min = min_version_for(kind);
            if min > 0 {
                assert_eq!(
                    check_compatible(min - 1, kind),
                    Err(VersionError::IncompatibleContractVersion)
                );
            }
            assert_eq!(
                check_compatible(0, kind),
                Err(VersionError::IncompatibleContractVersion)
            );
        }
    }

    // --- set/get version ---

    #[test]
    fn set_and_get_version_roundtrip() {
        let env = setup();
        let addr = env.register(TestContract, ());
        env.as_contract(&addr, || {
            assert_eq!(get_contract_version(&env), 1); // default
            set_contract_version(&env, SIGNAL_REGISTRY_VERSION);
            assert_eq!(get_contract_version(&env), SIGNAL_REGISTRY_VERSION);
        });
    }

    // --- cross-contract simulation ---

    #[test]
    fn old_callee_blocked() {
        // callee stuck on v1, caller requires min v2
        assert_eq!(
            check_compatible(1, ContractKind::SignalRegistry),
            Err(VersionError::IncompatibleContractVersion)
        );
    }

    #[test]
    fn current_callee_allowed() {
        assert!(check_compatible(AUTO_TRADE_VERSION, ContractKind::AutoTrade).is_ok());
    }

    // --- require_compatible (panic path) ---

    #[test]
    fn require_compatible_does_not_panic_for_valid_version() {
        let env = setup();
        let addr = env.register(TestContract, ());
        env.as_contract(&addr, || {
            // Should not panic
            require_compatible(&env, SIGNAL_REGISTRY_VERSION, ContractKind::SignalRegistry);
        });
    }

    #[test]
    #[should_panic]
    fn require_compatible_panics_for_incompatible_version() {
        let env = setup();
        let addr = env.register(TestContract, ());
        env.as_contract(&addr, || {
            require_compatible(&env, 0, ContractKind::SignalRegistry);
        });
    }

    // --- guard_upgrade ---

    #[test]
    fn guard_upgrade_allows_strictly_increasing_version() {
        assert!(guard_upgrade(1, 2).is_ok());
        assert!(guard_upgrade(2, 100).is_ok());
    }

    #[test]
    fn guard_upgrade_rejects_same_version() {
        assert_eq!(guard_upgrade(3, 3), Err(VersionError::DowngradeRejected));
    }

    #[test]
    fn guard_upgrade_rejects_downgrade() {
        assert_eq!(guard_upgrade(5, 4), Err(VersionError::DowngradeRejected));
        assert_eq!(guard_upgrade(5, 0), Err(VersionError::DowngradeRejected));
    }
}
