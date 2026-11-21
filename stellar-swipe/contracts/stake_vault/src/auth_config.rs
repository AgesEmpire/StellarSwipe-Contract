//! Authorization configuration preservation across upgrades (Issue #1398).
//!
//! The vault's authorization state is spread over several instance entries:
//!
//! | Entry | Role |
//! |---|---|
//! | `StorageKey::Admin` | administrator (required) |
//! | `StorageKey::GovernanceAddress` | governance guardian allowed to pause (#865) |
//! | `MultisigStorageKey::Config` | N-of-M signers for privileged actions |
//! | `StorageKey::EmergencyMultiSigConfig` | N-of-M approvers for emergency unstake (#754) |
//!
//! Two layouts are supported:
//!
//! * **Legacy** — only `Admin` is present (deployments that predate
//!   governance, multisig and emergency-unstake configuration).
//! * **Current** — `Admin` plus any combination of the optional entries.
//!
//! Every upgrade path captures an [`AuthConfigSnapshot`] *before* the WASM is
//! swapped, explicitly rejecting malformed state (wrong-typed entries,
//! zero/oversized thresholds, duplicate principals, out-of-range penalties)
//! instead of carrying it forward. The snapshot is persisted so the new code
//! can call `verify_auth_config` after the upgrade and prove that principals
//! and role assignments were retained exactly.

use soroban_sdk::{contracttype, Address, Env, IntoVal, TryFromVal, Val, Vec};

use shared::multisig::{self, MultisigConfig, MultisigStorageKey};

use crate::{emergency_unstake::EmergencyMultiSigConfig, StakeVaultError, StorageKey};

/// Exact copy of every authorization principal and role assignment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthConfigSnapshot {
    pub admin: Address,
    pub governance: Option<Address>,
    pub multisig: Option<MultisigConfig>,
    pub emergency: Option<EmergencyMultiSigConfig>,
}

#[contracttype]
#[derive(Clone)]
pub enum AuthConfigKey {
    /// Snapshot recorded by the most recent upgrade.
    PreUpgradeSnapshot,
}

/// Decode an optional instance entry, rejecting values of the wrong type
/// rather than trapping (or silently accepting them).
fn read_typed<K, T>(env: &Env, key: &K) -> Result<Option<T>, ()>
where
    K: IntoVal<Env, Val>,
    T: TryFromVal<Env, Val>,
{
    match env.storage().instance().get::<K, Val>(key) {
        None => Ok(None),
        Some(raw) => T::try_from_val(env, &raw).map(Some).map_err(|_| ()),
    }
}

fn has_duplicates(list: &Vec<Address>) -> bool {
    let len = list.len();
    let mut i: u32 = 0;
    while i < len {
        let a = list.get(i).unwrap();
        let mut j = i + 1;
        while j < len {
            if a == list.get(j).unwrap() {
                return true;
            }
            j += 1;
        }
        i += 1;
    }
    false
}

/// Validate a snapshot. Rejects:
/// * a multisig config that fails `shared::multisig::validate_config`
///   (empty signers, threshold 0 or above signer count, > `MAX_SIGNERS`,
///   duplicate signers) → `InvalidMultiSigConfig`;
/// * an emergency config with `required` 0 / above the approver count or
///   duplicate approvers → `InvalidMultiSigConfig`, or a penalty above
///   100% → `InvalidEmergencyPenalty`.
pub fn validate(snapshot: &AuthConfigSnapshot) -> Result<(), StakeVaultError> {
    if let Some(cfg) = &snapshot.multisig {
        multisig::validate_config(cfg).map_err(|_| StakeVaultError::InvalidMultiSigConfig)?;
    }
    if let Some(cfg) = &snapshot.emergency {
        if cfg.required == 0 || cfg.required > cfg.admins.len() || has_duplicates(&cfg.admins) {
            return Err(StakeVaultError::InvalidMultiSigConfig);
        }
        if cfg.penalty_bps > 10_000 {
            return Err(StakeVaultError::InvalidEmergencyPenalty);
        }
    }
    Ok(())
}

/// Read the live authorization configuration (legacy or current layout) and
/// validate it.
///
/// # Errors
/// * `NotInitialized` — no administrator is recorded.
/// * `Unauthorized` — the administrator or governance entry is not an `Address`.
/// * `InvalidMultiSigConfig` / `InvalidEmergencyPenalty` — see [`validate`].
pub fn capture(env: &Env) -> Result<AuthConfigSnapshot, StakeVaultError> {
    let admin: Address = read_typed(env, &StorageKey::Admin)
        .map_err(|_| StakeVaultError::Unauthorized)?
        .ok_or(StakeVaultError::NotInitialized)?;
    let governance: Option<Address> = read_typed(env, &StorageKey::GovernanceAddress)
        .map_err(|_| StakeVaultError::Unauthorized)?;
    let multisig: Option<MultisigConfig> = read_typed(env, &MultisigStorageKey::Config)
        .map_err(|_| StakeVaultError::InvalidMultiSigConfig)?;
    let emergency: Option<EmergencyMultiSigConfig> =
        read_typed(env, &StorageKey::EmergencyMultiSigConfig)
            .map_err(|_| StakeVaultError::InvalidMultiSigConfig)?;

    let snapshot = AuthConfigSnapshot {
        admin,
        governance,
        multisig,
        emergency,
    };
    validate(&snapshot)?;
    Ok(snapshot)
}

/// Capture + validate the live config and persist it as the pre-upgrade
/// reference. Called by every upgrade entrypoint before the WASM swap.
pub fn record_pre_upgrade(env: &Env) -> Result<AuthConfigSnapshot, StakeVaultError> {
    let snapshot = capture(env)?;
    env.storage()
        .instance()
        .set(&AuthConfigKey::PreUpgradeSnapshot, &snapshot);
    Ok(snapshot)
}

/// The snapshot recorded by the most recent upgrade, if any.
pub fn get_pre_upgrade(env: &Env) -> Option<AuthConfigSnapshot> {
    env.storage()
        .instance()
        .get(&AuthConfigKey::PreUpgradeSnapshot)
}

/// Assert the live config equals `expected` exactly (same admin, governance,
/// signer lists in the same order, thresholds, penalties and timeouts).
///
/// Returns `Unauthorized` on any drift; malformed live state is rejected by
/// [`capture`] first.
pub fn require_unchanged(env: &Env, expected: &AuthConfigSnapshot) -> Result<(), StakeVaultError> {
    let live = capture(env)?;
    if &live != expected {
        return Err(StakeVaultError::Unauthorized);
    }
    Ok(())
}

/// Post-upgrade check against the recorded snapshot. When no upgrade has
/// been recorded yet, the live config is still validated.
pub fn verify_against_recorded(env: &Env) -> Result<AuthConfigSnapshot, StakeVaultError> {
    match get_pre_upgrade(env) {
        Some(expected) => {
            require_unchanged(env, &expected)?;
            Ok(expected)
        }
        None => capture(env),
    }
}
