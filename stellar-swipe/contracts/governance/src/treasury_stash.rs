//! Stash-account isolation for treasury funds (Issue #1044).
//!
//! Treasury reserves ("stash") and day-to-day "operational" balances live in
//! **separate state buckets**.  Deposits target exactly one bucket and never
//! touch the other.  Value only crosses between buckets through
//! [`move_stash_to_operational`] / [`move_operational_to_stash`], and only when
//! a matching governance authorization ([`authorize_transfer`]) covers the
//! asset, direction, and cumulative amount.  Any transfer without such an
//! authorization fails with [`GovernanceError::Unauthorized`].
//!
//! ```text
//! deposit_stash ─────► [ stash ] ──move_stash_to_operational──► [ operational ]
//! deposit_operational ─────────────────────────────────────────►      ▲  │
//!                          ▲                                          │  │
//!                          └────────────── move_operational_to_stash ─┘  ▼
//!                                                              deposit_operational
//! ```

use soroban_sdk::{contracttype, Env, Map, Vec};
use stellar_swipe_common::Asset;

use crate::errors::GovernanceError;
use crate::{checked_add, checked_sub};

/// Direction a [`StashTransferAuth`] permits value to move.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StashDirection {
    /// Stash reserve → operational spending balance.
    StashToOperational,
    /// Operational balance → stash reserve (top-up).
    OperationalToStash,
}

/// A governance-approved authorization to move a bounded cumulative amount of a
/// single asset between the stash and operational buckets in one direction.
///
/// Re-authorizing the same `asset` **replaces** the prior record and resets
/// `moved` to zero.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StashTransferAuth {
    pub asset: Asset,
    /// Governance proposal that approved this authorization.
    pub proposal_id: u64,
    /// Cumulative cap on value moved under this authorization.
    pub max_amount: i128,
    /// Amount already moved under this authorization.
    pub moved: i128,
    /// The only direction this authorization permits.
    pub direction: StashDirection,
    /// Ledger timestamp the authorization was recorded.
    pub approved_at: u64,
}

/// Isolated treasury fund buckets.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StashLedger {
    /// Long-term reserves. Only `deposit_stash` and an authorized
    /// operational→stash move can increase this.
    pub stash: Map<Asset, i128>,
    /// Day-to-day spending balances, kept fully separate from `stash`.
    pub operational: Map<Asset, i128>,
    /// Active transfer authorizations, keyed by asset code.
    pub authorizations: Map<Asset, StashTransferAuth>,
    /// Lifetime value moved stash→operational per asset (audit trail).
    pub lifetime_released: Map<Asset, i128>,
    /// Lifetime value moved operational→stash per asset (audit trail).
    pub lifetime_topped_up: Map<Asset, i128>,
    /// Every asset ever seen by this ledger.
    pub tracked_assets: Vec<Asset>,
}

/// An empty stash ledger.
pub fn empty_stash_ledger(env: &Env) -> StashLedger {
    StashLedger {
        stash: Map::new(env),
        operational: Map::new(env),
        authorizations: Map::new(env),
        lifetime_released: Map::new(env),
        lifetime_topped_up: Map::new(env),
        tracked_assets: Vec::new(env),
    }
}

fn track_asset(ledger: &mut StashLedger, asset: &Asset) {
    let mut i = 0;
    while i < ledger.tracked_assets.len() {
        if ledger.tracked_assets.get(i).unwrap() == *asset {
            return;
        }
        i += 1;
    }
    ledger.tracked_assets.push_back(asset.clone());
}

// ── Balance reads ─────────────────────────────────────────────────────────────

/// Current stash (reserve) balance for `asset`.
pub fn stash_balance(ledger: &StashLedger, asset: &Asset) -> i128 {
    ledger.stash.get(asset.clone()).unwrap_or(0)
}

/// Current operational balance for `asset`.
pub fn operational_balance(ledger: &StashLedger, asset: &Asset) -> i128 {
    ledger.operational.get(asset.clone()).unwrap_or(0)
}

/// The active transfer authorization for `asset`, if any.
pub fn get_authorization(ledger: &StashLedger, asset: &Asset) -> Option<StashTransferAuth> {
    ledger.authorizations.get(asset.clone())
}

// ── Deposits (single-bucket) ─────────────────────────────────────────────────

/// Credit `amount` of `asset` to the **stash** bucket only.
pub fn deposit_stash(
    env: &Env,
    ledger: &mut StashLedger,
    asset: Asset,
    amount: i128,
) -> Result<i128, GovernanceError> {
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    track_asset(ledger, &asset);
    let new_balance = checked_add(stash_balance(ledger, &asset), amount)?;
    ledger.stash.set(asset.clone(), new_balance);
    emit(env, "stash_dep", &asset, amount, new_balance);
    Ok(new_balance)
}

/// Credit `amount` of `asset` to the **operational** bucket only.
pub fn deposit_operational(
    env: &Env,
    ledger: &mut StashLedger,
    asset: Asset,
    amount: i128,
) -> Result<i128, GovernanceError> {
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    track_asset(ledger, &asset);
    let new_balance = checked_add(operational_balance(ledger, &asset), amount)?;
    ledger.operational.set(asset.clone(), new_balance);
    emit(env, "op_dep", &asset, amount, new_balance);
    Ok(new_balance)
}

// ── Authorization ────────────────────────────────────────────────────────────

/// Governance: authorize bounded transfers of `asset` in one `direction`.
///
/// `admin` must have authenticated the enclosing call.  Replaces any existing
/// authorization for `asset` and resets its `moved` counter.
#[allow(clippy::too_many_arguments)]
pub fn authorize_transfer(
    env: &Env,
    ledger: &mut StashLedger,
    asset: Asset,
    proposal_id: u64,
    max_amount: i128,
    direction: StashDirection,
    now: u64,
) -> Result<StashTransferAuth, GovernanceError> {
    if max_amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    track_asset(ledger, &asset);
    let auth = StashTransferAuth {
        asset: asset.clone(),
        proposal_id,
        max_amount,
        moved: 0,
        direction,
        approved_at: now,
    };
    ledger.authorizations.set(asset.clone(), auth.clone());

    #[allow(deprecated)]
    env.events().publish(
        (
            soroban_sdk::symbol_short!("stash"),
            soroban_sdk::symbol_short!("authorize"),
        ),
        (asset, proposal_id, max_amount, direction, now),
    );
    Ok(auth)
}

/// Revoke the active authorization for `asset` (no-op if none exists).
pub fn revoke_authorization(env: &Env, ledger: &mut StashLedger, asset: Asset) {
    if ledger.authorizations.contains_key(asset.clone()) {
        ledger.authorizations.remove(asset.clone());
        #[allow(deprecated)]
        env.events().publish(
            (
                soroban_sdk::symbol_short!("stash"),
                soroban_sdk::symbol_short!("revoke"),
            ),
            asset,
        );
    }
}

// ── Authorized cross-bucket moves ────────────────────────────────────────────

fn consume_authorization(
    ledger: &mut StashLedger,
    asset: &Asset,
    amount: i128,
    direction: StashDirection,
) -> Result<StashTransferAuth, GovernanceError> {
    let mut auth = ledger
        .authorizations
        .get(asset.clone())
        .ok_or(GovernanceError::Unauthorized)?;
    if auth.direction != direction {
        return Err(GovernanceError::Unauthorized);
    }
    let new_moved = checked_add(auth.moved, amount)?;
    if new_moved > auth.max_amount {
        return Err(GovernanceError::ApprovedCapExceeded);
    }
    auth.moved = new_moved;
    ledger.authorizations.set(asset.clone(), auth.clone());
    Ok(auth)
}

/// Move `amount` of `asset` from the stash bucket to the operational bucket.
///
/// # Errors
/// * [`GovernanceError::InvalidAmount`] — `amount <= 0`.
/// * [`GovernanceError::Unauthorized`] — no authorization, or it does not cover
///   the `StashToOperational` direction.
/// * [`GovernanceError::ApprovedCapExceeded`] — would exceed the authorized cap.
/// * [`GovernanceError::InsufficientBalance`] — stash balance is too low.
pub fn move_stash_to_operational(
    env: &Env,
    ledger: &mut StashLedger,
    asset: Asset,
    amount: i128,
) -> Result<(), GovernanceError> {
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let auth = consume_authorization(ledger, &asset, amount, StashDirection::StashToOperational)?;

    let stash_bal = stash_balance(ledger, &asset);
    if stash_bal < amount {
        return Err(GovernanceError::InsufficientBalance);
    }
    let op_bal = operational_balance(ledger, &asset);

    ledger
        .stash
        .set(asset.clone(), checked_sub(stash_bal, amount)?);
    ledger
        .operational
        .set(asset.clone(), checked_add(op_bal, amount)?);

    let released = checked_add(
        ledger.lifetime_released.get(asset.clone()).unwrap_or(0),
        amount,
    )?;
    ledger.lifetime_released.set(asset.clone(), released);

    #[allow(deprecated)]
    env.events().publish(
        (
            soroban_sdk::symbol_short!("stash"),
            soroban_sdk::symbol_short!("release"),
        ),
        (asset, amount, auth.proposal_id, auth.moved, auth.max_amount),
    );
    Ok(())
}

/// Move `amount` of `asset` from the operational bucket to the stash bucket.
///
/// Mirrors [`move_stash_to_operational`]; requires an authorization covering
/// the `OperationalToStash` direction.
pub fn move_operational_to_stash(
    env: &Env,
    ledger: &mut StashLedger,
    asset: Asset,
    amount: i128,
) -> Result<(), GovernanceError> {
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let auth = consume_authorization(ledger, &asset, amount, StashDirection::OperationalToStash)?;

    let op_bal = operational_balance(ledger, &asset);
    if op_bal < amount {
        return Err(GovernanceError::InsufficientBalance);
    }
    let stash_bal = stash_balance(ledger, &asset);

    ledger
        .operational
        .set(asset.clone(), checked_sub(op_bal, amount)?);
    ledger
        .stash
        .set(asset.clone(), checked_add(stash_bal, amount)?);

    let topped = checked_add(
        ledger.lifetime_topped_up.get(asset.clone()).unwrap_or(0),
        amount,
    )?;
    ledger.lifetime_topped_up.set(asset.clone(), topped);

    #[allow(deprecated)]
    env.events().publish(
        (
            soroban_sdk::symbol_short!("stash"),
            soroban_sdk::symbol_short!("topup"),
        ),
        (asset, amount, auth.proposal_id, auth.moved, auth.max_amount),
    );
    Ok(())
}

// ── Accounting checks ────────────────────────────────────────────────────────

/// Total value held for `asset` across **both** buckets.
pub fn total_balance(ledger: &StashLedger, asset: &Asset) -> i128 {
    stash_balance(ledger, asset).saturating_add(operational_balance(ledger, asset))
}

/// Verify, for every tracked asset, that
/// `stash + operational == deposits ± authorized net movement` — i.e. no value
/// was created or destroyed and nothing leaked between buckets outside an
/// authorized move.
///
/// `expected_total` supplies the sum of all deposits per asset; the check is
/// `stash + operational == expected_total(asset)`.
pub fn assert_conservation(ledger: &StashLedger, expected_total: &Map<Asset, i128>) {
    let mut i = 0;
    while i < ledger.tracked_assets.len() {
        let asset = ledger.tracked_assets.get(i).unwrap();
        let expected = expected_total.get(asset.clone()).unwrap_or(0);
        assert_eq!(
            total_balance(ledger, &asset),
            expected,
            "stash/operational conservation violated for a tracked asset"
        );
        i += 1;
    }
}

fn emit(env: &Env, kind: &str, asset: &Asset, amount: i128, new_balance: i128) {
    #[allow(deprecated)]
    env.events().publish(
        (
            soroban_sdk::symbol_short!("stash"),
            soroban_sdk::Symbol::new(env, kind),
        ),
        (asset.clone(), amount, new_balance),
    );
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{Env, String};

    fn asset(env: &Env, code: &str) -> Asset {
        Asset {
            code: String::from_str(env, code),
            issuer: None,
        }
    }

    fn totals(env: &Env) -> Map<Asset, i128> {
        Map::new(env)
    }

    #[test]
    fn deposits_target_a_single_bucket() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");

        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        deposit_operational(&env, &mut l, xlm.clone(), 250).unwrap();

        assert_eq!(stash_balance(&l, &xlm), 1_000);
        assert_eq!(operational_balance(&l, &xlm), 250);
        assert_eq!(total_balance(&l, &xlm), 1_250);
    }

    #[test]
    fn deposits_reject_non_positive_amounts() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        assert_eq!(
            deposit_stash(&env, &mut l, xlm.clone(), 0),
            Err(GovernanceError::InvalidAmount)
        );
        assert_eq!(
            deposit_operational(&env, &mut l, xlm, -5),
            Err(GovernanceError::InvalidAmount)
        );
    }

    #[test]
    fn transfer_without_authorization_fails() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();

        assert_eq!(
            move_stash_to_operational(&env, &mut l, xlm.clone(), 100),
            Err(GovernanceError::Unauthorized)
        );
        // Balances untouched.
        assert_eq!(stash_balance(&l, &xlm), 1_000);
        assert_eq!(operational_balance(&l, &xlm), 0);
    }

    #[test]
    fn wrong_direction_authorization_is_rejected() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            7,
            500,
            StashDirection::OperationalToStash,
            0,
        )
        .unwrap();

        // Authorization covers the opposite direction only.
        assert_eq!(
            move_stash_to_operational(&env, &mut l, xlm.clone(), 100),
            Err(GovernanceError::Unauthorized)
        );
        assert_eq!(stash_balance(&l, &xlm), 1_000);
    }

    #[test]
    fn wrong_asset_authorization_does_not_apply() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        let usdc = asset(&env, "USDC");
        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        deposit_stash(&env, &mut l, usdc.clone(), 1_000).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm,
            7,
            500,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();

        assert_eq!(
            move_stash_to_operational(&env, &mut l, usdc.clone(), 100),
            Err(GovernanceError::Unauthorized)
        );
        assert_eq!(stash_balance(&l, &usdc), 1_000);
    }

    #[test]
    fn authorized_move_transfers_between_buckets_only() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        deposit_operational(&env, &mut l, xlm.clone(), 200).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            7,
            500,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();

        move_stash_to_operational(&env, &mut l, xlm.clone(), 300).unwrap();

        assert_eq!(stash_balance(&l, &xlm), 700);
        assert_eq!(operational_balance(&l, &xlm), 500);
        assert_eq!(total_balance(&l, &xlm), 1_200); // unchanged
        assert_eq!(l.lifetime_released.get(xlm.clone()).unwrap(), 300);

        let auth = get_authorization(&l, &xlm).unwrap();
        assert_eq!(auth.moved, 300);
    }

    #[test]
    fn cumulative_cap_is_enforced_across_moves() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            7,
            500,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();

        move_stash_to_operational(&env, &mut l, xlm.clone(), 400).unwrap();
        // 400 + 200 = 600 > cap 500
        assert_eq!(
            move_stash_to_operational(&env, &mut l, xlm.clone(), 200),
            Err(GovernanceError::ApprovedCapExceeded)
        );
        // The remaining 100 of headroom still works.
        move_stash_to_operational(&env, &mut l, xlm.clone(), 100).unwrap();
        assert_eq!(stash_balance(&l, &xlm), 500);
        assert_eq!(operational_balance(&l, &xlm), 500);
    }

    #[test]
    fn move_fails_when_bucket_balance_insufficient() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 100).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            7,
            10_000,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();

        assert_eq!(
            move_stash_to_operational(&env, &mut l, xlm.clone(), 500),
            Err(GovernanceError::InsufficientBalance)
        );
        assert_eq!(stash_balance(&l, &xlm), 100);
    }

    #[test]
    fn re_authorization_resets_moved_counter() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 2_000).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            7,
            500,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();
        move_stash_to_operational(&env, &mut l, xlm.clone(), 500).unwrap();
        assert_eq!(
            move_stash_to_operational(&env, &mut l, xlm.clone(), 1),
            Err(GovernanceError::ApprovedCapExceeded)
        );

        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            8,
            300,
            StashDirection::StashToOperational,
            10,
        )
        .unwrap();
        move_stash_to_operational(&env, &mut l, xlm.clone(), 300).unwrap();
        assert_eq!(stash_balance(&l, &xlm), 1_200);
        assert_eq!(operational_balance(&l, &xlm), 800);
    }

    #[test]
    fn revoked_authorization_blocks_further_moves() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            7,
            500,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();
        move_stash_to_operational(&env, &mut l, xlm.clone(), 100).unwrap();
        revoke_authorization(&env, &mut l, xlm.clone());
        assert_eq!(
            move_stash_to_operational(&env, &mut l, xlm.clone(), 100),
            Err(GovernanceError::Unauthorized)
        );
    }

    #[test]
    fn round_trip_conserves_value_and_keeps_buckets_separate() {
        let env = Env::default();
        let mut l = empty_stash_ledger(&env);
        let xlm = asset(&env, "XLM");
        let usdc = asset(&env, "USDC");

        deposit_stash(&env, &mut l, xlm.clone(), 1_000).unwrap();
        deposit_operational(&env, &mut l, xlm.clone(), 400).unwrap();
        deposit_stash(&env, &mut l, usdc.clone(), 5_000).unwrap();

        let mut expected: Map<Asset, i128> = totals(&env);
        expected.set(xlm.clone(), 1_400);
        expected.set(usdc.clone(), 5_000);

        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            1,
            1_000,
            StashDirection::StashToOperational,
            0,
        )
        .unwrap();
        move_stash_to_operational(&env, &mut l, xlm.clone(), 600).unwrap();

        authorize_transfer(
            &env,
            &mut l,
            xlm.clone(),
            2,
            1_000,
            StashDirection::OperationalToStash,
            1,
        )
        .unwrap();
        move_operational_to_stash(&env, &mut l, xlm.clone(), 250).unwrap();

        // USDC never had an authorization and never moved.
        assert_eq!(stash_balance(&l, &usdc), 5_000);
        assert_eq!(operational_balance(&l, &usdc), 0);

        // XLM buckets shifted but total is preserved.
        assert_eq!(stash_balance(&l, &xlm), 1_000 - 600 + 250);
        assert_eq!(operational_balance(&l, &xlm), 400 + 600 - 250);

        assert_conservation(&l, &expected);
    }
}
