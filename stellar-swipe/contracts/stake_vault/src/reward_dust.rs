//! Reward dust tracking and controlled reconciliation (Issue #1135).
//!
//! Sub-unit rounding remainders are accumulated per asset in scaled units
//! ([`DUST_PRECISION`]) and kept **separate** from claimable provider
//! balances.  Whole units of dust can only leave the vault through
//! [`reconcile`], which requires the configured dust authority and emits the
//! before/after totals.  Provider balances are never touched.
//!
//! Conservation invariant (per asset):
//! `recorded_scaled == dust_scaled + reconciled * DUST_PRECISION`

use soroban_sdk::{contracttype, symbol_short, token, Address, Env};

/// Scale factor for sub-unit dust (1 unit == 10^7 scaled dust).
pub const DUST_PRECISION: i128 = 10_000_000;

#[contracttype]
#[derive(Clone)]
pub enum DustKey {
    /// Address allowed to reconcile dust.
    Authority,
    /// Scaled dust currently held for `asset`.
    Dust(Address),
    /// Lifetime scaled dust recorded for `asset`.
    Recorded(Address),
    /// Lifetime whole units reconciled for `asset`.
    Reconciled(Address),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DustError {
    Unauthorized,
    InvalidAmount,
    NothingToReconcile,
    Overflow,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustTotals {
    pub dust_scaled: i128,
    pub recorded_scaled: i128,
    pub reconciled_units: i128,
}

fn get(env: &Env, key: &DustKey) -> i128 {
    env.storage().persistent().get(key).unwrap_or(0)
}

pub fn set_authority(env: &Env, authority: &Address) {
    env.storage().instance().set(&DustKey::Authority, authority);
}

pub fn get_authority(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DustKey::Authority)
}

pub fn totals(env: &Env, asset: &Address) -> DustTotals {
    DustTotals {
        dust_scaled: get(env, &DustKey::Dust(asset.clone())),
        recorded_scaled: get(env, &DustKey::Recorded(asset.clone())),
        reconciled_units: get(env, &DustKey::Reconciled(asset.clone())),
    }
}

/// Invariant: recorded dust == held dust + reconciled dust.
pub fn is_conserved(env: &Env, asset: &Address) -> bool {
    let t = totals(env, asset);
    t.reconciled_units
        .checked_mul(DUST_PRECISION)
        .and_then(|r| r.checked_add(t.dust_scaled))
        == Some(t.recorded_scaled)
}

/// Records `scaled` sub-unit dust for `asset` (never credited to providers).
pub fn record(env: &Env, asset: &Address, scaled: i128) -> Result<DustTotals, DustError> {
    if scaled <= 0 {
        return Err(DustError::InvalidAmount);
    }
    let t = totals(env, asset);
    let dust = t.dust_scaled.checked_add(scaled).ok_or(DustError::Overflow)?;
    let recorded = t.recorded_scaled.checked_add(scaled).ok_or(DustError::Overflow)?;
    env.storage().persistent().set(&DustKey::Dust(asset.clone()), &dust);
    env.storage().persistent().set(&DustKey::Recorded(asset.clone()), &recorded);
    env.events()
        .publish((symbol_short!("dust_rec"), asset.clone()), (scaled, dust));
    Ok(totals(env, asset))
}

/// Moves whole units of held dust for `asset` to `recipient`.
/// Sub-unit remainder stays tracked.  Returns the whole units transferred.
pub fn reconcile(
    env: &Env,
    caller: &Address,
    asset: &Address,
    recipient: &Address,
) -> Result<i128, DustError> {
    let authority = get_authority(env).ok_or(DustError::Unauthorized)?;
    if *caller != authority {
        return Err(DustError::Unauthorized);
    }
    caller.require_auth();

    let before = totals(env, asset);
    let units = before.dust_scaled / DUST_PRECISION;
    if units == 0 {
        return Err(DustError::NothingToReconcile);
    }
    let dust_after = before.dust_scaled - units * DUST_PRECISION;
    let reconciled = before
        .reconciled_units
        .checked_add(units)
        .ok_or(DustError::Overflow)?;

    env.storage().persistent().set(&DustKey::Dust(asset.clone()), &dust_after);
    env.storage()
        .persistent()
        .set(&DustKey::Reconciled(asset.clone()), &reconciled);
    token::Client::new(env, asset).transfer(&env.current_contract_address(), recipient, &units);

    let after = totals(env, asset);
    env.events().publish(
        (symbol_short!("dust_rcn"), asset.clone(), recipient.clone()),
        (
            before.dust_scaled,
            after.dust_scaled,
            before.reconciled_units,
            after.reconciled_units,
            units,
        ),
    );
    Ok(units)
}
