//! Fee collector contract.
//!
//! Collects protocol fees and exposes a tightly-scoped, auditable path for
//! sweeping negligible dust balances out of the collector.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// Balances strictly below this threshold are considered "dust" and are the
/// only balances eligible for the audited sweep path. Any balance at or above
/// this value is an ordinary fee balance and MUST NOT be sweepable.
pub const DUST_THRESHOLD: i128 = 1_000;

/// Number of ledgers a newly proposed payout recipient must wait before it can
/// be activated. This delay prevents an accidental or compromised admin update
/// from immediately redirecting accumulated fees.
pub const RECIPIENT_ROTATION_DELAY: u32 = 17_280;

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const BALANCE_KEY: Symbol = symbol_short!("BALANCE");
const SOURCE_KEY: Symbol = symbol_short!("SOURCE");
const CLOSED_KEY: Symbol = symbol_short!("CLOSED");
const SNAPSHOT_KEY: Symbol = symbol_short!("SNAPSHOT");
const RECIPIENT_KEY: Symbol = symbol_short!("RECIPIENT");
const PENDING_RECIPIENT_KEY: Symbol = symbol_short!("PEND_RECIP");
const PENDING_ACTIVATION_KEY: Symbol = symbol_short!("PEND_ACT");

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
    NoPendingRecipient = 9,
    RecipientNotYetActive = 10,
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

/// Auditable record of a payout recipient rotation.
///
/// Emitted when a new recipient is proposed (with the ledger at which it may
/// be activated) and when a pending proposal is cancelled, so operators can
/// discover and audit recipient changes before they take effect.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientRotation {
    pub recipient: Address,
    pub activation_ledger: u32,
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

    /// Propose a new payout recipient.
    ///
    /// Only the configured admin may propose a recipient. The proposal is
    /// stored as pending along with the ledger at which it becomes activatable
    /// (`current_ledger + RECIPIENT_ROTATION_DELAY`). Payouts continue to use
    /// the currently active recipient until the pending one is activated, so a
    /// compromised admin cannot immediately redirect accumulated fees. The
    /// proposal is recorded in a [`RecipientRotation`] event.
    pub fn propose_recipient(
        env: Env,
        caller: Address,
        recipient: Address,
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

        let activation_ledger = env.ledger().sequence() + RECIPIENT_ROTATION_DELAY;

        env.storage()
            .instance()
            .set(&PENDING_RECIPIENT_KEY, &recipient);
        env.storage()
            .instance()
            .set(&PENDING_ACTIVATION_KEY, &activation_ledger);

        env.events().publish(
            (symbol_short!("recip_rot"),),
            RecipientRotation {
                recipient,
                activation_ledger,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Cancel a pending payout recipient proposal.
    ///
    /// Only the configured admin may cancel. Cancellation clears the pending
    /// recipient and its activation ledger and is recorded in a
    /// [`RecipientRotation`] event (with `activation_ledger` set to 0) so the
    /// cancellation is fully auditable.
    pub fn cancel_pending_recipient(
        env: Env,
        caller: Address,
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

        let recipient: Address = env
            .storage()
            .instance()
            .get(&PENDING_RECIPIENT_KEY)
            .ok_or(FeeCollectorError::NoPendingRecipient)?;

        env.storage().instance().remove(&PENDING_RECIPIENT_KEY);
        env.storage().instance().remove(&PENDING_ACTIVATION_KEY);

        env.events().publish(
            (symbol_short!("recip_rot"),),
            RecipientRotation {
                recipient,
                activation_ledger: 0,
                actor: caller,
            },
        );

        Ok(())
    }

    /// Activate a pending payout recipient once its delay has elapsed.
    ///
    /// Only the configured admin may activate. Activation is rejected until
    /// the current ledger reaches the stored activation ledger, enforcing the
    /// rotation delay. On success the pending recipient becomes the active
    /// recipient and the pending state is cleared.
    pub fn activate_recipient(env: Env, caller: Address) -> Result<(), FeeCollectorError> {
        caller.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .ok_or(FeeCollectorError::NotInitialized)?;
        if caller != admin {
            return Err(FeeCollectorError::Unauthorized);
        }

        let recipient: Address = env
            .storage()
            .instance()
            .get(&PENDING_RECIPIENT_KEY)
            .ok_or(FeeCollectorError::NoPendingRecipient)?;
        let activation_ledger: u32 = env
            .storage()
            .instance()
            .get(&PENDING_ACTIVATION_KEY)
            .ok_or(FeeCollectorError::NoPendingRecipient)?;

        if env.ledger().sequence() < activation_ledger {
            return Err(FeeCollectorError::RecipientNotYetActive);
        }

        env.storage().instance().set(&RECIPIENT_KEY, &recipient);
        env.storage().instance().remove(&PENDING_RECIPIENT_KEY);
        env.storage().instance().remove(&PENDING_ACTIVATION_KEY);

        Ok(())
    }

    /// The currently active payout recipient, if any.
    ///
    /// Payouts must only ever use this recipient; the pending recipient is
    /// never returned here.
    pub fn active_recipient(env: Env) -> Option<Address> {
        env.storage().instance().get(&RECIPIENT_KEY)
    }

    /// The pending payout recipient and the ledger at which it may be
    /// activated, if a proposal is outstanding.
    pub fn pending_recipient(env: Env) -> Option<(Address, u32)> {
        let recipient: Option<Address> = env.storage().instance().get(&PENDING_RECIPIENT_KEY);
        let activation_ledger: Option<u32> = env.storage().instance().get(&PENDING_ACTIVATION_KEY);
        match (recipient, activation_ledger) {
            (Some(recipient), Some(activation_ledger)) => Some((recipient, activation_ledger)),
            _ => None,
        }
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
}
