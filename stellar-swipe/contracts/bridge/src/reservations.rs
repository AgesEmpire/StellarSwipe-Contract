//! Bridge liquidity reservations and their release on message expiry
//! (Issue #1416).
//!
//! An admin reserves part of an asset's bridge liquidity for a specific
//! lock/mint transfer. Reserved liquidity is unavailable to other
//! reservations until it is either:
//!
//! * **consumed** — the transfer executed successfully, so the liquidity was
//!   used for the delivery; or
//! * **released** — the transfer can no longer be delivered (its validity
//!   window ended, or it was cancelled / permanently failed), so the
//!   liquidity returns to the available pool.
//!
//! Each reservation settles exactly once: a settled reservation is removed
//! from the active set and its status is final, so repeated expiry
//! processing is a no-op and can never release the same liquidity twice.

use soroban_sdk::{contracttype, Env, String, Symbol, Vec};

use crate::{BridgeError, BridgeTransfer, TransferStatus};

/// Upper bound on simultaneously active reservations; also bounds the work
/// one expiry sweep can do.
pub const MAX_ACTIVE_RESERVATIONS: u32 = 128;
/// Protocol cap on reservations released in one sweep.
pub const MAX_EXPIRY_BATCH: u32 = 20;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationStatus {
    Active,
    Released,
    Consumed,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidityReservation {
    pub transfer_id: u64,
    pub asset: String,
    pub amount: i128,
    /// Copied from the transfer when the reservation is made.
    pub expires_at: u64,
    pub status: ReservationStatus,
    pub created_at: u64,
    /// Ledger time at which the reservation was released or consumed.
    pub settled_at: Option<u64>,
}

/// Liquidity accounting for one asset. `available = total - reserved`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetLiquidity {
    pub total: i128,
    pub reserved: i128,
}

/// Storage keys. Variant names are unique across the crate.
#[contracttype]
pub enum ReservationKey {
    LiquidityReservation(u64),
    BridgeAssetLiquidity(String),
    ActiveReservationIds,
}

pub fn get_asset_liquidity(env: &Env, asset: &String) -> AssetLiquidity {
    env.storage()
        .persistent()
        .get(&ReservationKey::BridgeAssetLiquidity(asset.clone()))
        .unwrap_or(AssetLiquidity {
            total: 0,
            reserved: 0,
        })
}

fn save_asset_liquidity(env: &Env, asset: &String, liquidity: &AssetLiquidity) {
    env.storage()
        .persistent()
        .set(&ReservationKey::BridgeAssetLiquidity(asset.clone()), liquidity);
}

pub fn available_liquidity(env: &Env, asset: &String) -> i128 {
    let l = get_asset_liquidity(env, asset);
    l.total.saturating_sub(l.reserved)
}

pub fn get_reservation(env: &Env, transfer_id: u64) -> Option<LiquidityReservation> {
    env.storage()
        .persistent()
        .get(&ReservationKey::LiquidityReservation(transfer_id))
}

fn save_reservation(env: &Env, reservation: &LiquidityReservation) {
    env.storage().persistent().set(
        &ReservationKey::LiquidityReservation(reservation.transfer_id),
        reservation,
    );
}

pub fn get_active_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&ReservationKey::ActiveReservationIds)
        .unwrap_or_else(|| Vec::new(env))
}

fn save_active_ids(env: &Env, ids: &Vec<u64>) {
    env.storage()
        .persistent()
        .set(&ReservationKey::ActiveReservationIds, ids);
}

fn remove_active_id(env: &Env, transfer_id: u64) {
    let mut ids = get_active_ids(env);
    for i in 0..ids.len() {
        if ids.get(i) == Some(transfer_id) {
            ids.remove(i);
            save_active_ids(env, &ids);
            return;
        }
    }
}

/// Admin: set the total bridge liquidity for `asset`. The total may not drop
/// below what is currently reserved.
pub fn set_asset_liquidity(env: &Env, asset: String, total: i128) -> Result<(), BridgeError> {
    let mut liquidity = get_asset_liquidity(env, &asset);
    if total < 0 || total < liquidity.reserved {
        return Err(BridgeError::InvalidAmount);
    }
    liquidity.total = total;
    save_asset_liquidity(env, &asset, &liquidity);

    #[allow(deprecated)]
    env.events().publish(
        (
            Symbol::new(env, "bridge"),
            Symbol::new(env, "asset_liquidity_set"),
            asset,
        ),
        (liquidity.total, liquidity.reserved),
    );
    Ok(())
}

/// Reserve `transfer.amount` of `transfer.wrapped_asset` for `transfer`.
/// Only live lock/mint transfers with a finite expiry can hold a reservation.
pub fn reserve(env: &Env, transfer: &BridgeTransfer) -> Result<(), BridgeError> {
    let now = env.ledger().timestamp();
    if transfer.kind != crate::TransferKind::LockMint
        || transfer.expires_at == 0
        || matches!(
            transfer.status,
            TransferStatus::Completed | TransferStatus::Cancelled | TransferStatus::Failed
        )
    {
        return Err(BridgeError::InvalidOperation);
    }
    if now >= transfer.expires_at {
        return Err(BridgeError::MessageExpired);
    }
    if get_reservation(env, transfer.id).is_some() {
        return Err(BridgeError::InvalidOperation);
    }
    let mut ids = get_active_ids(env);
    if ids.len() >= MAX_ACTIVE_RESERVATIONS {
        return Err(BridgeError::ReservationCapacityReached);
    }

    let asset = transfer.wrapped_asset.clone();
    let mut liquidity = get_asset_liquidity(env, &asset);
    let new_reserved = liquidity
        .reserved
        .checked_add(transfer.amount)
        .ok_or(BridgeError::InsufficientBridgeLiquidity)?;
    if new_reserved > liquidity.total {
        return Err(BridgeError::InsufficientBridgeLiquidity);
    }
    liquidity.reserved = new_reserved;

    let reservation = LiquidityReservation {
        transfer_id: transfer.id,
        asset: asset.clone(),
        amount: transfer.amount,
        expires_at: transfer.expires_at,
        status: ReservationStatus::Active,
        created_at: now,
        settled_at: None,
    };
    save_asset_liquidity(env, &asset, &liquidity);
    save_reservation(env, &reservation);
    ids.push_back(transfer.id);
    save_active_ids(env, &ids);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "liquidity_reserved"), transfer.id),
        (asset, transfer.amount, transfer.expires_at),
    );
    Ok(())
}

/// True when the reservation's transfer can no longer be delivered.
fn is_release_eligible(
    env: &Env,
    reservation: &LiquidityReservation,
    transfer: Option<&BridgeTransfer>,
) -> bool {
    let now = env.ledger().timestamp();
    match transfer {
        // The transfer record is gone; nothing can consume the reservation.
        None => true,
        Some(t) => match t.status {
            TransferStatus::Completed => false,
            TransferStatus::Cancelled => true,
            TransferStatus::Failed if !t.retryable => true,
            _ => now >= reservation.expires_at,
        },
    }
}

/// Settle an active reservation as released. Caller has checked eligibility.
fn release_active(env: &Env, mut reservation: LiquidityReservation) -> Result<(), BridgeError> {
    let mut liquidity = get_asset_liquidity(env, &reservation.asset);
    liquidity.reserved = liquidity
        .reserved
        .checked_sub(reservation.amount)
        .filter(|r| *r >= 0)
        .ok_or(BridgeError::InvalidAmount)?;
    reservation.status = ReservationStatus::Released;
    reservation.settled_at = Some(env.ledger().timestamp());
    save_asset_liquidity(env, &reservation.asset, &liquidity);
    save_reservation(env, &reservation);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "liquidity_released"), reservation.transfer_id),
        (reservation.asset.clone(), reservation.amount, liquidity.reserved),
    );
    Ok(())
}

/// Release the reservation for one transfer if it is eligible.
///
/// Returns `Ok(true)` when liquidity was released by this call and
/// `Ok(false)` when there was nothing to do (already released/consumed, or
/// the transfer is still deliverable). Calling it again after a release is
/// a no-op.
pub fn release_if_expired(
    env: &Env,
    transfer_id: u64,
    transfer: Option<&BridgeTransfer>,
) -> Result<bool, BridgeError> {
    let reservation = get_reservation(env, transfer_id).ok_or(BridgeError::ReservationNotFound)?;
    if reservation.status != ReservationStatus::Active
        || !is_release_eligible(env, &reservation, transfer)
    {
        return Ok(false);
    }
    release_active(env, reservation)?;
    remove_active_id(env, transfer_id);
    Ok(true)
}

/// Sweep the active set and release up to `max_items` eligible reservations.
/// Non-eligible reservations stay active and untouched. Returns how many
/// reservations this call released; a repeated sweep returns `0`.
pub fn release_expired(
    env: &Env,
    max_items: u32,
    load: impl Fn(&Env, u64) -> Option<BridgeTransfer>,
) -> Result<u32, BridgeError> {
    if max_items == 0 || max_items > MAX_EXPIRY_BATCH {
        return Err(BridgeError::InvalidAmount);
    }
    let ids = get_active_ids(env);
    let mut remaining = Vec::new(env);
    let mut released: u32 = 0;

    for id in ids.iter() {
        if released >= max_items {
            remaining.push_back(id);
            continue;
        }
        let reservation = match get_reservation(env, id) {
            Some(r) if r.status == ReservationStatus::Active => r,
            // Already settled: drop the stale id.
            _ => continue,
        };
        let transfer = load(env, id);
        if is_release_eligible(env, &reservation, transfer.as_ref()) {
            release_active(env, reservation)?;
            released += 1;
        } else {
            remaining.push_back(id);
        }
    }

    if remaining.len() != ids.len() {
        save_active_ids(env, &remaining);
    }
    Ok(released)
}

/// Called after a transfer executes successfully: an active reservation is
/// consumed (the liquidity was used for the delivery), so it can no longer
/// be released by expiry processing. No-op when the transfer had none.
pub fn consume_for_transfer(env: &Env, transfer_id: u64) -> Result<(), BridgeError> {
    let mut reservation = match get_reservation(env, transfer_id) {
        Some(r) if r.status == ReservationStatus::Active => r,
        _ => return Ok(()),
    };
    let mut liquidity = get_asset_liquidity(env, &reservation.asset);
    liquidity.reserved = liquidity
        .reserved
        .checked_sub(reservation.amount)
        .ok_or(BridgeError::InvalidAmount)?;
    liquidity.total = liquidity
        .total
        .checked_sub(reservation.amount)
        .ok_or(BridgeError::InvalidAmount)?;
    if liquidity.reserved < 0 || liquidity.total < liquidity.reserved {
        return Err(BridgeError::InvalidAmount);
    }
    reservation.status = ReservationStatus::Consumed;
    reservation.settled_at = Some(env.ledger().timestamp());
    save_asset_liquidity(env, &reservation.asset, &liquidity);
    save_reservation(env, &reservation);
    remove_active_id(env, transfer_id);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "liquidity_consumed"), transfer_id),
        (reservation.asset, reservation.amount),
    );
    Ok(())
}
