//! Capability-based authorization model (Issue #860).
//!
//! Replaces ad-hoc admin checks with specific, named capabilities for each
//! privileged action. Each capability represents the authority to perform a
//! specific class of operations.
//!
//! # Capabilities
//!
//! | Capability | Description |
//! |---|---|
//! | `Pause` | Emergency pause/unpause the contract |
//! | `Upgrade` | Upgrade contract Wasm |
//! | `Treasury` | Withdraw from treasury, manage funds |
//! | `ParameterChange` | Modify fee rates, thresholds, config |
//! | `Emergency` | Emergency actions (slash, halt, etc.) |
//! | `SuperAdmin` | Super-admin: grant/revoke any capability |
//!
//! # Usage
//!
//! ```ignore
//! use shared::capabilities::{self, Capability};
//!
//! fn admin_pause(env: Env, caller: Address) -> Result<(), MyError> {
//!     capabilities::require_capability(&env, &caller, Capability::Pause)?;
//!     set_paused(&env, true);
//!     Ok(())
//! }
//! ```

use crate::access_control;
use soroban_sdk::{contracterror, contracttype, Address, Env, Vec};

/// Named capabilities for privileged actions.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum Capability {
    /// Emergency pause/unpause.
    Pause = 0,
    /// Upgrade contract Wasm / version.
    Upgrade = 1,
    /// Treasury withdrawals and fund management.
    Treasury = 2,
    /// Parameter changes (fee rates, thresholds, config).
    ParameterChange = 3,
    /// Emergency actions (slash, halt, circuit breakers).
    Emergency = 4,
    /// Super-admin: can grant/revoke any capability to/from any address.
    SuperAdmin = 5,
}

impl Capability {
    /// All defined capabilities.
    pub fn all() -> [Capability; 6] {
        [
            Capability::Pause,
            Capability::Upgrade,
            Capability::Treasury,
            Capability::ParameterChange,
            Capability::Emergency,
            Capability::SuperAdmin,
        ]
    }
}

/// Storage key for capability grants.
#[contracttype]
#[derive(Clone)]
pub enum CapabilityStorageKey {
    /// Maps (Capability, Address) -> bool (granted or not).
    Grant(Capability, Address),
}

/// Grant `capability` to `address`. Only the SuperAdmin may call this.
pub fn grant_capability(env: &Env, caller: &Address, address: &Address, capability: Capability) {
    require_capability(env, caller, Capability::SuperAdmin);
    env.storage()
        .instance()
        .set(&CapabilityStorageKey::Grant(capability, address.clone()), &true);
}

/// Revoke `capability` from `address`. Only the SuperAdmin may call this.
pub fn revoke_capability(env: &Env, caller: &Address, address: &Address, capability: Capability) {
    require_capability(env, caller, Capability::SuperAdmin);
    env.storage()
        .instance()
        .remove(&CapabilityStorageKey::Grant(capability, address.clone()));
}

/// Check whether `address` holds `capability`.
pub fn has_capability(env: &Env, address: &Address, capability: Capability) -> bool {
    // SuperAdmin always has every capability.
    if env
        .storage()
        .instance()
        .get::<_, bool>(&CapabilityStorageKey::Grant(
            Capability::SuperAdmin,
            address.clone(),
        ))
        .unwrap_or(false)
    {
        return true;
    }
    // Also check legacy admin role (Admin has all capabilities for backward compat).
    if capability != Capability::SuperAdmin {
        let role = access_control::get_role(env, address);
        if role == access_control::Role::Admin {
            return true;
        }
    }
    env.storage()
        .instance()
        .get::<_, bool>(&CapabilityStorageKey::Grant(capability, address.clone()))
        .unwrap_or(false)
}

/// Require `address` to hold `capability`.
pub fn require_capability(
    env: &Env,
    address: &Address,
    capability: Capability,
) -> Result<(), CapabilityError> {
    if has_capability(env, address, capability) {
        Ok(())
    } else {
        Err(CapabilityError::InsufficientCapability)
    }
}

/// List all capabilities granted to `address`.
pub fn list_capabilities(env: &Env, address: &Address) -> Vec<Capability> {
    let mut caps = Vec::new(env);
    for cap in Capability::all().iter() {
        if has_capability(env, address, *cap) {
            caps.push_back(*cap);
        }
    }
    caps
}

/// Error returned when a caller lacks the required capability.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CapabilityError {
    /// The caller does not hold the required capability.
    InsufficientCapability = 1,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_control::{set_role, Role};
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};

    #[contract]
    struct TestCaps;

    #[contractimpl]
    impl TestCaps {
        pub fn init(env: Env, admin: Address) {
            set_role(&env, &admin, Role::Admin);
            grant_capability(&env, &admin, &admin, Capability::SuperAdmin);
        }
        pub fn check(env: Env, caller: Address, cap: Capability) -> bool {
            has_capability(&env, &caller, cap)
        }
        pub fn do_grant(env: Env, caller: Address, target: Address, cap: Capability) {
            grant_capability(&env, &caller, &target, cap);
        }
        pub fn do_revoke(env: Env, caller: Address, target: Address, cap: Capability) {
            revoke_capability(&env, &caller, &target, cap);
        }
    }

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        let contract_id = env.register(TestCaps, ());
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            TestCaps::init(env.clone(), admin.clone());
        });
        (env, admin, user)
    }

    #[test]
    fn admin_has_all_caps_by_default() {
        let (env, admin, _user) = setup();
        let contract_id = env.register(TestCaps, ());
        env.as_contract(&contract_id, || {
            assert!(has_capability(&env, &admin, Capability::Pause));
            assert!(has_capability(&env, &admin, Capability::Upgrade));
            assert!(has_capability(&env, &admin, Capability::Treasury));
            assert!(has_capability(&env, &admin, Capability::ParameterChange));
            assert!(has_capability(&env, &admin, Capability::Emergency));
        });
    }

    #[test]
    fn user_lacks_capabilities_by_default() {
        let (env, _admin, user) = setup();
        let contract_id = env.register(TestCaps, ());
        env.as_contract(&contract_id, || {
            assert!(!has_capability(&env, &user, Capability::Pause));
            assert!(!has_capability(&env, &user, Capability::Upgrade));
        });
    }

    #[test]
    fn grant_and_revoke() {
        let (env, admin, user) = setup();
        let contract_id = env.register(TestCaps, ());
        env.as_contract(&contract_id, || {
            grant_capability(&env, &admin, &user, Capability::Pause);
            assert!(has_capability(&env, &user, Capability::Pause));
            assert!(!has_capability(&env, &user, Capability::Upgrade));

            revoke_capability(&env, &admin, &user, Capability::Pause);
            assert!(!has_capability(&env, &user, Capability::Pause));
        });
    }

    #[test]
    fn require_capability_works() {
        let (env, admin, user) = setup();
        let contract_id = env.register(TestCaps, ());
        env.as_contract(&contract_id, || {
            assert!(require_capability(&env, &admin, Capability::Pause).is_ok());
            assert_eq!(
                require_capability(&env, &user, Capability::Pause),
                Err(CapabilityError::InsufficientCapability)
            );
        });
    }

    #[test]
    fn moderator_does_not_have_all_caps() {
        let env = Env::default();
        let contract_id = env.register(TestCaps, ());
        let mod_addr = Address::generate(&env);
        env.as_contract(&contract_id, || {
            set_role(&env, &mod_addr, Role::Moderator);
            assert!(!has_capability(&env, &mod_addr, Capability::Pause));
            assert!(!has_capability(&env, &mod_addr, Capability::Upgrade));
        });
    }
}
