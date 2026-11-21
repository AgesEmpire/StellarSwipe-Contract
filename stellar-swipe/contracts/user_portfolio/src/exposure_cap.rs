//! Enforced per-asset maximum exposure cap (Issue #752).
//!
//! Users may optionally configure a maximum exposure cap per asset (by asset-pair ID).
//! Any copy-trade execution that would push exposure past the cap is rejected
//! with `ExposureCapExceeded`. This feature is opt-in; assets without a cap are
//! unaffected. The existing concentration-risk scoring remains advisory and is
//! not replaced by this enforcement.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

use crate::storage::DataKey;

// ── Storage helpers ────────────────────────────────────────────────────────────

fn cap_key(user: &Address, asset_id: u32) -> DataKey {
    DataKey::UserAssetCap(user.clone(), asset_id)
}

fn exposure_key(user: &Address, asset_id: u32) -> DataKey {
    DataKey::UserAssetExposure(user.clone(), asset_id)
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// User: set a maximum absolute exposure cap for `asset_id`.
/// `cap_amount` must be > 0.  The caller must be `user` (auth enforced in entrypoint).
pub fn set_cap(env: &Env, user: &Address, asset_id: u32, cap_amount: i128) {
    env.storage()
        .persistent()
        .set(&cap_key(user, asset_id), &cap_amount);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("portfolio"), symbol_short!("cap_set")),
        (user.clone(), asset_id, cap_amount),
    );
}

/// User: remove the exposure cap for `asset_id` (no enforcement after removal).
pub fn remove_cap(env: &Env, user: &Address, asset_id: u32) {
    env.storage().persistent().remove(&cap_key(user, asset_id));

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("portfolio"), symbol_short!("cap_rem")),
        (user.clone(), asset_id),
    );
}

/// Returns the configured cap for `(user, asset_id)`, or `None` if not set.
pub fn get_cap(env: &Env, user: &Address, asset_id: u32) -> Option<i128> {
    env.storage()
        .persistent()
        .get::<_, i128>(&cap_key(user, asset_id))
}

/// Returns the user's current tracked exposure for `asset_id`.
pub fn get_exposure(env: &Env, user: &Address, asset_id: u32) -> i128 {
    env.storage()
        .persistent()
        .get::<_, i128>(&exposure_key(user, asset_id))
        .unwrap_or(0)
}

/// Check whether adding `new_amount` to the user's current exposure for `asset_id`
/// would exceed the cap. Returns the error variant to use.
///
/// If no cap is configured for this asset, always returns `Ok(())`.
pub fn check_cap(
    env: &Env,
    user: &Address,
    asset_id: u32,
    new_amount: i128,
) -> Result<(), crate::PortfolioError> {
    let cap = match get_cap(env, user, asset_id) {
        Some(c) => c,
        None => return Ok(()), // No cap → no restriction.
    };

    let current = get_exposure(env, user, asset_id);
    if current.saturating_add(new_amount) > cap {
        return Err(crate::PortfolioError::ExposureCapExceeded);
    }

    Ok(())
}

/// Record an increase in exposure when a capped position is opened.
pub fn add_exposure(env: &Env, user: &Address, asset_id: u32, amount: i128) {
    let current = get_exposure(env, user, asset_id);
    env.storage().persistent().set(
        &exposure_key(user, asset_id),
        &current.saturating_add(amount),
    );
}

/// Record a decrease in exposure when a capped position is closed.
pub fn remove_exposure(env: &Env, user: &Address, asset_id: u32, amount: i128) {
    let current = get_exposure(env, user, asset_id);
    env.storage().persistent().set(
        &exposure_key(user, asset_id),
        &current.saturating_sub(amount),
    );
}
