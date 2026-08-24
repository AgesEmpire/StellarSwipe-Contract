//! Signal watchlist: users can watch signals without copying them.
//!
//! Storage key: `DataKey::Watchlist(user)` → `Vec<u64>` (signal IDs).
//! Cap: 50 signals per user.
//! Expired signals are removed on access.

use soroban_sdk::{Address, Env, Symbol, Vec};

use crate::storage::DataKey;

pub const WATCHLIST_CAP: u32 = 50;

/// Emit `SignalWatchlisted { user, signal_id }`.
fn emit_signal_watchlisted(env: &Env, user: Address, signal_id: u64) {
    let topics = (Symbol::new(env, "signal_watchlisted"),);
    env.events().publish(topics, (user, signal_id));
}

/// Load the watchlist for `user`, filtering out any signal IDs whose expiry has passed.
///
/// `is_expired_fn` receives a signal_id and returns `true` if the signal is expired.
/// Pass a closure that checks the signal registry (or a stub in tests).
fn load_watchlist<F>(env: &Env, user: &Address, is_expired_fn: F) -> Vec<u64>
where
    F: Fn(u64) -> bool,
{
    let raw: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::Watchlist(user.clone()))
        .unwrap_or(Vec::new(env));

    // Filter expired signals
    let mut filtered = Vec::new(env);
    for id in raw.iter() {
        if !is_expired_fn(id) {
            filtered.push_back(id);
        }
    }
    filtered
}

/// Add `signal_id` to `user`'s watchlist.
///
/// - Deduplicates (no-op if already present).
/// - Enforces cap of [`WATCHLIST_CAP`].
/// - Expired signals are removed on access.
/// - Emits `SignalWatchlisted` event on successful add.
///
/// Returns `Err` if the watchlist is full after removing expired signals.
pub fn add_to_watchlist<F>(
    env: &Env,
    user: Address,
    signal_id: u64,
    is_expired_fn: F,
) -> Result<(), WatchlistError>
where
    F: Fn(u64) -> bool,
{
    if is_expired_fn(signal_id) {
        return Err(WatchlistError::SignalExpired);
    }

    let mut list = load_watchlist(env, &user, &is_expired_fn);

    // Deduplicate
    for existing in list.iter() {
        if existing == signal_id {
            return Ok(());
        }
    }

    if list.len() >= WATCHLIST_CAP {
        return Err(WatchlistError::WatchlistFull);
    }

    list.push_back(signal_id);
    env.storage()
        .persistent()
        .set(&DataKey::Watchlist(user.clone()), &list);

    emit_signal_watchlisted(env, user, signal_id);
    Ok(())
}

/// Remove `signal_id` from `user`'s watchlist.
/// Expired signals are also pruned on access.
pub fn remove_from_watchlist<F>(env: &Env, user: Address, signal_id: u64, is_expired_fn: F)
where
    F: Fn(u64) -> bool,
{
    let list = load_watchlist(env, &user, &is_expired_fn);
    let mut updated = Vec::new(env);
    for id in list.iter() {
        if id != signal_id {
            updated.push_back(id);
        }
    }
    env.storage()
        .persistent()
        .set(&DataKey::Watchlist(user), &updated);
}

/// Return the watchlist for `user`, pruning expired signals.
pub fn get_watchlist<F>(env: &Env, user: Address, is_expired_fn: F) -> Vec<u64>
where
    F: Fn(u64) -> bool,
{
    let list = load_watchlist(env, &user, &is_expired_fn);
    // Persist the pruned list so storage stays clean.
    env.storage()
        .persistent()
        .set(&DataKey::Watchlist(user), &list);
    list
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WatchlistError {
    WatchlistFull,
    SignalExpired,
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};

    #[contract]
    struct WatchlistHarness;

    #[contractimpl]
    impl WatchlistHarness {}

    fn setup() -> (Env, Address) {
        let env = Env::default();
        let contract_id = env.register(WatchlistHarness, ());
        (env, contract_id)
    }

    fn run<F, R>(env: &Env, contract_id: &Address, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        env.as_contract(contract_id, f)
    }

    fn never_expired(_id: u64) -> bool {
        false
    }

    fn always_expired(_id: u64) -> bool {
        true
    }

    fn expired_if(expired_id: u64) -> impl Fn(u64) -> bool {
        move |id| id == expired_id
    }

    #[test]
    fn add_and_get_watchlist() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);

            add_to_watchlist(&env, user.clone(), 1, never_expired).unwrap();
            add_to_watchlist(&env, user.clone(), 2, never_expired).unwrap();

            let list = get_watchlist(&env, user, never_expired);
            assert_eq!(list.len(), 2);
        });
    }

    #[test]
    fn remove_from_watchlist_works() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);

            add_to_watchlist(&env, user.clone(), 1, never_expired).unwrap();
            add_to_watchlist(&env, user.clone(), 2, never_expired).unwrap();
            remove_from_watchlist(&env, user.clone(), 1, never_expired);

            let list = get_watchlist(&env, user, never_expired);
            assert_eq!(list.len(), 1);
            assert_eq!(list.get(0).unwrap(), 2);
        });
    }

    #[test]
    fn cap_enforced() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);

            for i in 0..WATCHLIST_CAP {
                add_to_watchlist(&env, user.clone(), i as u64, never_expired).unwrap();
            }

            let result = add_to_watchlist(&env, user, WATCHLIST_CAP as u64, never_expired);
            assert_eq!(result, Err(WatchlistError::WatchlistFull));
        });
    }

    #[test]
    fn expired_signal_rejected_on_add() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);

            let result = add_to_watchlist(&env, user, 99, always_expired);
            assert_eq!(result, Err(WatchlistError::SignalExpired));
        });
    }

    #[test]
    fn expired_signals_removed_on_access() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);

            add_to_watchlist(&env, user.clone(), 1, never_expired).unwrap();
            add_to_watchlist(&env, user.clone(), 2, never_expired).unwrap();

            let list = get_watchlist(&env, user, expired_if(1));
            assert_eq!(list.len(), 1);
            assert_eq!(list.get(0).unwrap(), 2);
        });
    }

    #[test]
    fn deduplication_no_op() {
        let (env, contract_id) = setup();
        run(&env, &contract_id, || {
            let user = Address::generate(&env);

            add_to_watchlist(&env, user.clone(), 5, never_expired).unwrap();
            add_to_watchlist(&env, user.clone(), 5, never_expired).unwrap();

            let list = get_watchlist(&env, user, never_expired);
            assert_eq!(list.len(), 1);
        });
    }
}
