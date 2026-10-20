//! Keeper registry and signature verification for trigger-execution requests.
//!
//! Registered keepers are the only addresses permitted to invoke
//! `check_and_trigger_conditionals`. Adding/removing keeper keys requires
//! admin authority. The actual trigger-condition evaluation still happens
//! on-chain; a valid keeper signature is a prerequisite, not a substitute.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

use crate::errors::AutoTradeError;

// ── Storage key ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum KeeperKey {
    /// Stores the Vec<Address> of all registered keeper addresses.
    Registry,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn load_keepers(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&KeeperKey::Registry)
        .unwrap_or_else(|| Vec::new(env))
}

fn save_keepers(env: &Env, keepers: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&KeeperKey::Registry, keepers);
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Register a new keeper address (admin only).
pub fn add_keeper(env: &Env, admin: &Address, keeper: Address) -> Result<(), AutoTradeError> {
    crate::admin::require_admin(env, admin)?;
    let mut keepers = load_keepers(env);
    if !keepers.contains(keeper.clone()) {
        keepers.push_back(keeper.clone());
        save_keepers(env, &keepers);
        env.events()
            .publish((symbol_short!("keeper"), symbol_short!("added")), keeper);
    }
    Ok(())
}

/// Remove a keeper address (admin only).
pub fn remove_keeper(env: &Env, admin: &Address, keeper: &Address) -> Result<(), AutoTradeError> {
    crate::admin::require_admin(env, admin)?;
    let mut keepers = load_keepers(env);
    if let Some(pos) = keepers.first_index_of(keeper.clone()) {
        keepers.remove(pos);
        save_keepers(env, &keepers);
        env.events().publish(
            (symbol_short!("keeper"), symbol_short!("removed")),
            keeper.clone(),
        );
    }
    Ok(())
}

/// Returns the full list of registered keeper addresses.
pub fn list_keepers(env: &Env) -> Vec<Address> {
    load_keepers(env)
}

/// Verify that `caller` is a registered keeper.
///
/// - Requires the caller to `require_auth()` (Soroban's built-in signature
///   check), confirming possession of the corresponding private key.
/// - Returns `AutoTradeError::Unauthorized` if the address is not registered.
///
/// Call this at the start of any keeper-relayed entrypoint before performing
/// trigger-condition evaluation.
pub fn require_registered_keeper(env: &Env, caller: &Address) -> Result<(), AutoTradeError> {
    caller.require_auth();
    let keepers = load_keepers(env);
    if !keepers.contains(caller.clone()) {
        return Err(AutoTradeError::Unauthorized);
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::init_admin;
    use crate::AutoTradeContract;
    use soroban_sdk::{contract, testutils::Address as _, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let cid = env.register(AutoTradeContract, ());
        let admin = Address::generate(&env);
        env.as_contract(&cid, || init_admin(&env, admin.clone()));
        (env, cid, admin)
    }

    #[test]
    fn test_add_and_verify_keeper() {
        let (env, cid, admin) = setup();
        let keeper = Address::generate(&env);

        env.as_contract(&cid, || {
            add_keeper(&env, &admin, keeper.clone()).unwrap();
            // Registered keeper: verification must succeed
            require_registered_keeper(&env, &keeper).unwrap();
        });
    }

    #[test]
    fn test_unregistered_keeper_rejected() {
        let (env, cid, _admin) = setup();
        let rogue = Address::generate(&env);

        env.as_contract(&cid, || {
            let result = require_registered_keeper(&env, &rogue);
            assert_eq!(result, Err(AutoTradeError::Unauthorized));
        });
    }

    #[test]
    fn test_remove_keeper_then_rejected() {
        let (env, cid, admin) = setup();
        let keeper = Address::generate(&env);

        // Admin ops each run in their own frame: two `require_auth` calls on
        // the same address inside one frame raise "frame is already authorized".
        env.as_contract(&cid, || {
            add_keeper(&env, &admin, keeper.clone()).unwrap();
        });
        env.as_contract(&cid, || {
            remove_keeper(&env, &admin, &keeper).unwrap();
            let result = require_registered_keeper(&env, &keeper);
            assert_eq!(result, Err(AutoTradeError::Unauthorized));
        });
    }

    #[test]
    fn test_no_duplicate_keepers() {
        let (env, cid, admin) = setup();
        let keeper = Address::generate(&env);

        env.as_contract(&cid, || {
            add_keeper(&env, &admin, keeper.clone()).unwrap();
        });
        env.as_contract(&cid, || {
            add_keeper(&env, &admin, keeper.clone()).unwrap(); // second add is no-op
            assert_eq!(list_keepers(&env).len(), 1);
        });
    }

    #[test]
    fn test_registered_keeper_trigger_valid_condition() {
        // Verifies that a registered keeper can invoke the trigger check path
        // and that on-chain condition evaluation still gates actual execution.
        // (Here we just confirm the keeper passes the auth gate; the condition
        //  evaluation gate is tested in conditional::tests.)
        let (env, cid, admin) = setup();
        let keeper = Address::generate(&env);

        env.as_contract(&cid, || {
            add_keeper(&env, &admin, keeper.clone()).unwrap();
            // Passes keeper gate — the triggered list is empty because no
            // conditional orders exist, not because the keeper was rejected.
            require_registered_keeper(&env, &keeper).unwrap();
            let triggered = crate::conditional::check_and_trigger(&env);
            assert_eq!(triggered.len(), 0);
        });
    }
}
