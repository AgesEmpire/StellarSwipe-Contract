//! Reward distribution and fee-collector balance management.
//!
//! This module handles the accounting of collected fees and provides a
//! tightly-scoped, auditable path for sweeping negligible "dust" balances
//! out of the collector.

use soroban_sdk::{contractevent, contracttype, Address, Env};

/// Maximum balance (in stroops) that is considered "dust" and therefore
/// eligible for the auditable sweep path. Any balance greater than or equal
/// to this threshold is a normal fee balance and MUST NOT be sweepable
/// through [`sweep_dust`].
pub const DUST_THRESHOLD: i128 = 1_000;

/// Storage key for the collector's current balance.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Balance,
}

/// Emitted whenever a dust sweep is performed, recording the destination,
/// the swept amount, and the authorizing caller for audit purposes.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DustSwept {
    #[topic]
    pub caller: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Returns the collector's current balance.
pub fn balance(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::Balance).unwrap_or(0)
}

/// Sets the collector's current balance.
pub fn set_balance(env: &Env, amount: i128) {
    env.storage().instance().set(&DataKey::Balance, &amount);
}

/// Sweeps a negligible dust balance to an explicit destination.
///
/// This is a controlled, auditable path: it may only be invoked by the
/// configured authority, only for balances strictly below [`DUST_THRESHOLD`],
/// and only to a non-zero destination for a positive amount not exceeding the
/// current balance. Ordinary fee balances cannot be moved through this path.
///
/// # Panics
/// - If `caller` is not the configured authority.
/// - If `destination` is the collector itself (invalid destination).
/// - If `amount` is not positive.
/// - If `amount` exceeds the current balance.
/// - If `amount` is not strictly below [`DUST_THRESHOLD`].
/// - If the current balance is not strictly below [`DUST_THRESHOLD`].
pub fn sweep_dust(env: &Env, caller: Address, destination: Address, amount: i128) {
    caller.require_auth();

    let authority: Address = env
        .storage()
        .instance()
        .get(&DataKey::Authority)
        .expect("authority not set");
    if caller != authority {
        panic!("unauthorized: caller is not the authority");
    }

    if destination == env.current_contract_address() {
        panic!("invalid destination");
    }
    if amount <= 0 {
        panic!("amount must be positive");
    }

    let current = balance(env);
    if current >= DUST_THRESHOLD {
        panic!("balance is not dust");
    }
    if amount >= DUST_THRESHOLD {
        panic!("amount is not dust");
    }
    if amount > current {
        panic!("amount exceeds balance");
    }

    set_balance(env, current - amount);

    DustSwept {
        caller,
        destination,
        amount,
    }
    .publish(env);
}
