//! Contract-level feature flag registry for gradual entrypoint rollout.
//!
//! Admin sets named boolean flags via `set_feature_flag`.  Individual
//! entrypoints call `require_feature_enabled` before executing new code
//! paths, returning [`ContractError::FeatureDisabled`] when the flag is off.
//!
//! Toggling a flag does NOT affect unrelated entrypoints: each flag is
//! independent and stored under its own `StorageKey::FeatureFlag(name)` key.

use soroban_sdk::{symbol_short, Env, String, Symbol};

use crate::errors::ContractError;
use crate::StorageKey;

/// Flag name for the copy-trade market execution code path.
pub const FEAT_COPY_TRADE: &str = "copy_trade";
/// Flag name for the DCA interval execution code path.
pub const FEAT_DCA: &str = "dca";

// ── Storage helpers ───────────────────────────────────────────────────────────

pub fn is_flag_enabled(env: &Env, name: &String) -> bool {
    env.storage()
        .instance()
        .get(&StorageKey::FeatureFlag(name.clone()))
        .unwrap_or(true) // absent = enabled by default (backwards compatible)
}

pub fn set_flag(env: &Env, name: String, enabled: bool) {
    let old = is_flag_enabled(env, &name);
    env.storage()
        .instance()
        .set(&StorageKey::FeatureFlag(name.clone()), &enabled);
    emit_flag_changed(env, name, old, enabled);
}

// ── Guard used at entrypoint boundaries ──────────────────────────────────────

pub fn require_feature_enabled(env: &Env, name: &str) -> Result<(), ContractError> {
    let key = String::from_str(env, name);
    if is_flag_enabled(env, &key) {
        Ok(())
    } else {
        Err(ContractError::FeatureDisabled)
    }
}

// ── Event emission ────────────────────────────────────────────────────────────

fn emit_flag_changed(env: &Env, name: String, old_enabled: bool, new_enabled: bool) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("feat_flag"), Symbol::new(env, "changed")),
        (name, old_enabled, new_enabled),
    );
}
