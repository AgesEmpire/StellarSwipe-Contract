/// Issue: Validate strategy parameters against declared min/max bounds.
///
/// Each parameter declares its legal range in configuration state.
/// Updates outside the range are rejected with `AdminError::InvalidParameter`.
use soroban_sdk::{contracttype, Env, Symbol};

use crate::errors::AdminError;

#[contracttype]
#[derive(Clone, Debug)]
pub struct ParamBounds {
    pub min: i128,
    pub max: i128,
}

#[contracttype]
#[derive(Clone)]
enum BoundsKey {
    Bounds(Symbol),
}

/// Admin: declare the legal [min, max] range for a named parameter.
pub fn set_param_bounds(env: &Env, param: Symbol, bounds: ParamBounds) -> Result<(), AdminError> {
    if bounds.min > bounds.max {
        return Err(AdminError::InvalidParameter);
    }
    env.storage()
        .instance()
        .set(&BoundsKey::Bounds(param), &bounds);
    Ok(())
}

/// Returns the declared bounds for `param`, if any.
pub fn get_param_bounds(env: &Env, param: Symbol) -> Option<ParamBounds> {
    env.storage()
        .instance()
        .get(&BoundsKey::Bounds(param))
}

/// Validate `value` against the declared bounds for `param`.
/// Returns `Ok(())` if no bounds are declared (open range) or value is within range.
/// Returns `Err(AdminError::InvalidParameter)` if value is outside [min, max].
pub fn validate_param(env: &Env, param: Symbol, value: i128) -> Result<(), AdminError> {
    if let Some(bounds) = get_param_bounds(env, param) {
        if value < bounds.min || value > bounds.max {
            return Err(AdminError::InvalidParameter);
        }
    }
    Ok(())
}
