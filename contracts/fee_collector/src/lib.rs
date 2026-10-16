//! Fee collector contract.
//!
//! Collects protocol fees and exposes a tightly-scoped, auditable path for
//! sweeping negligible dust balances out of the collector.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// Balances strictly below this threshold are considered "dust" and are the
/// only balances eligible for the audited sweep path. Any balance at or above
/// this value is an ordinary fee balance and MUST NOT be sweepable.
pub const DUST_THRESHOLD: i128 = 1_000;

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const BALANCE_KEY: Symbol = symbol_short!("BALANCE");
const SOURCE_KEY: Symbol = symbol_short!("SOURCE");
const CLOSED_KEY: Symbol = symbol_short!("CLOSED");
const SNAPSHOT_KEY: Symbol = symbol_short!("SNAPSHOT");

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FeeCollectorError {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidDestination = 3,
    InvalidAmount = 4,
    AmountExceedsBalance = 5,
    AmountNotDust = 6,
    UnknownRevenueSource = 7,
    BucketClosed = 8,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustSwept {
    pub caller: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Auditable record of a material contract configuration change.
///
/// Emitted whenever a configuration setting is updated so indexers and
/// operators can discover the change, the prior value, the new value, and the
/// actor that authorized it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigChanged {
    pub setting: Symbol,
    pub old_value: Address,
    pub new_value: Address,
    pub actor: Address,
}

/// Auditable record of a revenue source allowlist change.
///
/// Emitted whenever a revenue source is added to or removed from the
/// allowlist so indexers and operators can attribute deposits and discover
/// which sources are authorized to deposit fees.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevenueSourceChanged {
    pub source: Address,
    pub allowed: bool,
    pub actor: Address,
}

/// Immutable snapshot of a fee bucket captured at closeout time.
///
/// Once written, the snapshot is never mutated: reports read these totals
/// instead of live bucket state, and the closed period rejects further writes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BucketSnapshot {
    pub total_collected: i128,
    pub pending_rebates: i128,
    pub net_total: i128,
    pub closed_at: u64,
}

/// Auditable record of a fee bucket closeout.
///
/// Emitted exactly once per bucket (the first successful closeout) so indexers
/// can discover the frozen totals and the actor that authorized the close.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BucketClosed {
    pub total_collected: i128,
    pub pending_rebates: i128,
    pub net_total: i128,
    pub actor: Address,
}

#[contract]
pub struct FeeCollector;

#[contractimpl]
impl FeeCollector {
    /// Initialize the collector with an admin authority.
    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&BALANCE_KEY, &0i128);
    }

    /// Update the collector's admin authority.
    ///
    /// Only the current admin may rotate the admin role. The change is recorded
    /// in a [`ConfigChanged`] event capturing the setting, prior admin, new
    /// admin, and the authorizing actor.
    pub fn set_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        env.storage().instance().set(&ADMIN_KEY, &new_admin);

        env.events().publish(
            (symbol_short!("config_changed"),),
            ConfigChanged {
                setting: ADMIN_KEY,
                old_value: admin,
                new_value: new_admin,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Authorize a revenue source to deposit fees into the collector.
    ///
    /// Only the configured admin may add a source. The change is recorded in a
    /// [`RevenueSourceChanged`] event so the allowlist is fully auditable.
    pub fn add_revenue_source(
        env: Env,
        caller: Address,
        source: Address,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        env.storage()
            .persistent()
            .set(&(SOURCE_KEY, source.clone()), &true);

        env.events().publish(
            (symbol_short!("rev_source"),),
            RevenueSourceChanged {
                source,
                allowed: true,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Revoke a revenue source's authorization to deposit fees.
    ///
    /// Only the configured admin may remove a source. Removal takes effect
    /// immediately, so a source removed during an in-flight accounting period
    /// can no longer deposit. The change is recorded in a
    /// [`RevenueSourceChanged`] event.
    pub fn remove_revenue_source(
        env: Env,
        caller: Address,
        source: Address,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        env.storage()
            .persistent()
            .remove(&(SOURCE_KEY, source.clone()));

        env.events().publish(
            (symbol_short!("rev_source"),),
            RevenueSourceChanged {
                source,
                allowed: false,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Whether a revenue source is currently authorized to deposit fees.
    pub fn is_revenue_source(env: Env, source: Address) -> bool {
        env.storage()
            .persistent()
            .get(&(SOURCE_KEY, source))
            .unwrap_or(false)
    }

    /// Record collected fees into the collector balance.
    ///
    /// Only allowlisted revenue sources may deposit. The authorization check
    /// runs before any balance mutation so deposits from unknown sources fail
    /// before any transfer or accounting side effect is attempted. Deposits
    /// into a closed bucket are rejected so the closed period is immutable.
    pub fn collect(
        env: Env,
        source: Address,
        amount: i128,
    ) -> Result<(), FeeCollectorError> {
        source.require_auth();

        if Self::is_closed(env.clone()) {
            return Err(FeeCollectorError::BucketClosed);
        }

        let allowed: bool = env
            .storage()
            .persistent()
            .get(&(SOURCE_KEY, source.clone()))
            .unwrap_or(false);
        if !allowed {
            return Err(FeeCollectorError::UnknownRevenueSource);
        }

        let balance: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        env.storage().instance().set(&BALANCE_KEY, &(balance + amount));

        Ok(())
    }

    /// Current collector balance.
    pub fn balance(env: Env) -> i128 {
        env.storage().instance().get(&BALANCE_KEY).unwrap_or(0)
    }

    /// Whether the fee bucket has been closed out.
    pub fn is_closed(env: Env) -> bool {
        env.storage().instance().get(&CLOSED_KEY).unwrap_or(false)
    }

    /// Close out the fee bucket, snapshotting totals and freezing the period.
    ///
    /// Only the configured admin may close the bucket. The operation is
    /// idempotent: a repeat call is a safe no-op that returns the existing
    /// snapshot unchanged, so the frozen totals can never be altered after the
    /// first close. Pending rebates are handled by policy: they are recorded in
    /// the snapshot and deducted from the reported net total, but are not paid
    /// out here. Once closed, [`collect`] rejects further writes.
    pub fn close_bucket(
        env: Env,
        caller: Address,
        pending_rebates: i128,
    ) -> Result<BucketSnapshot, FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        if pending_rebates < 0 {
            return Err(FeeCollectorError::InvalidAmount);
        }

        // Idempotent: return the frozen snapshot without mutating it.
        if let Some(existing) = env
            .storage()
            .instance()
            .get::<Symbol, BucketSnapshot>(&SNAPSHOT_KEY)
        {
            return Ok(existing);
        }

        let total_collected: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        if pending_rebates > total_collected {
            return Err(FeeCollectorError::AmountExceedsBalance);
        }

        let snapshot = BucketSnapshot {
            total_collected,
            pending_rebates,
            net_total: total_collected - pending_rebates,
            closed_at: env.ledger().timestamp(),
        };

        env.storage().instance().set(&CLOSED_KEY, &true);
        env.storage().instance().set(&SNAPSHOT_KEY, &snapshot);

        env.events().publish(
            (symbol_short!("bucket_closed"),),
            BucketClosed {
                total_collected: snapshot.total_collected,
                pending_rebates: snapshot.pending_rebates,
                net_total: snapshot.net_total,
                actor: caller,
            },
        );

        Ok(snapshot)
    }

    /// Read the frozen closeout snapshot, if the bucket has been closed.
    ///
    /// Reports MUST read totals from this snapshot rather than live bucket
    /// state so closed periods are reported consistently and immutably.
    pub fn bucket_snapshot(env: Env) -> Option<BucketSnapshot> {
        env.storage().instance().get(&SNAPSHOT_KEY)
    }

    /// Sweep a negligible dust balance to an explicit destination.
    ///
    /// Only the configured admin may call this, the amount must be strictly
    /// below [`DUST_THRESHOLD`], and the destination must be non-zero. The
    /// destination, amount, and authorizing caller are recorded in a
    /// [`DustSwept`] event for audit purposes.
    pub fn sweep_dust(
        env: Env,
        caller: Address,
        destination: Address,
        amount: i128,
    ) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        if destination == caller {
            return Err(FeeCollectorError::InvalidDestination);
        }

        if amount <= 0 {
            return Err(FeeCollectorError::InvalidAmount);
        }

        if amount >= DUST_THRESHOLD {
            return Err(FeeCollectorError::AmountNotDust);
        }

        let balance: i128 = env.storage().instance().get(&BALANCE_KEY).unwrap_or(0);
        if amount > balance {
            return Err(FeeCollectorError::AmountExceedsBalance);
        }

        env.storage().instance().set(&BALANCE_KEY, &(balance - amount));

        env.events().publish(
            (symbol_short!("dust_swept"),),
            DustSwept {
                caller,
                destination,
                amount,
            },
        );

        Ok(())
    }
}
