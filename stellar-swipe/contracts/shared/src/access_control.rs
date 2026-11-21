//! Shared access-control role hierarchy (Issue #678).
//!
//! Provides three ordered role levels — Admin, Moderator, Viewer — plus a
//! `require_role` helper so contracts can enforce scoped permissions without
//! reinventing role storage in every module.
//!
//! The top-level admin (the existing contract admin) may grant/revoke any role
//! for any address. Only the admin can change roles.

use soroban_sdk::{contracterror, contracttype, Address, Env};

// ── Storage key ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum RoleStorageKey {
    /// Role assigned to an address.
    Role(Address),
}

// ── Role enum ─────────────────────────────────────────────────────────────────

/// Ordered access-control levels. Higher ordinal = more privileges.
#[contracttype]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Role {
    /// Unassigned / no special permissions. Lowest level.
    None = 0,
    /// Read-only access: can view data but not mutate state.
    Viewer = 1,
    /// Operational moderator: can perform maintenance actions (e.g. pause)
    /// but cannot change fee parameters or admin config.
    Moderator = 2,
    /// Full administrative access. Top level.
    Admin = 3,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RoleError {
    /// The caller did not meet the minimum required role for this action.
    InsufficientRole = 1,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return the role currently assigned to `address`, defaulting to [`Role::None`].
pub fn get_role(env: &Env, address: &Address) -> Role {
    env.storage()
        .instance()
        .get(&RoleStorageKey::Role(address.clone()))
        .unwrap_or(Role::None)
}

/// Assign a role to `address`. Only the contract admin should call this.
/// Passing [`Role::None`] effectively revokes all special permissions.
pub fn set_role(env: &Env, address: &Address, role: Role) {
    env.storage()
        .instance()
        .set(&RoleStorageKey::Role(address.clone()), &role);
}

/// Check that the caller (`address`) has at least `min_role`.
/// Returns `Ok(())` if the caller's role ≥ `min_role`, otherwise
/// `Err(RoleError::InsufficientRole)`.
pub fn require_role(env: &Env, address: &Address, min_role: Role) -> Result<(), RoleError> {
    let actual = get_role(env, address);
    if actual < min_role {
        return Err(RoleError::InsufficientRole);
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, testutils::Address as _, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        let contract_id = env.register(TestContract, ());
        (env, contract_id)
    }

    // --- get_role / set_role ---

    #[test]
    fn default_role_is_none() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            assert_eq!(get_role(&env, &addr), Role::None);
        });
    }

    #[test]
    fn set_and_get_role_roundtrip() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Viewer);
            assert_eq!(get_role(&env, &addr), Role::Viewer);

            set_role(&env, &addr, Role::Moderator);
            assert_eq!(get_role(&env, &addr), Role::Moderator);

            set_role(&env, &addr, Role::Admin);
            assert_eq!(get_role(&env, &addr), Role::Admin);
        });
    }

    #[test]
    fn set_role_none_revokes() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Moderator);
            assert_eq!(get_role(&env, &addr), Role::Moderator);

            set_role(&env, &addr, Role::None);
            assert_eq!(get_role(&env, &addr), Role::None);
        });
    }

    #[test]
    fn different_addresses_have_independent_roles() {
        let (env, cid) = setup();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &a, Role::Moderator);
            set_role(&env, &b, Role::Viewer);
            assert_eq!(get_role(&env, &a), Role::Moderator);
            assert_eq!(get_role(&env, &b), Role::Viewer);
        });
    }

    // --- require_role ---

    #[test]
    fn require_role_admin_succeeds_for_admin() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Admin);
            assert!(require_role(&env, &addr, Role::Admin).is_ok());
        });
    }

    #[test]
    fn require_role_viewer_succeeds_for_viewer() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Viewer);
            assert!(require_role(&env, &addr, Role::Viewer).is_ok());
        });
    }

    #[test]
    fn require_role_admin_fails_for_viewer() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Viewer);
            assert_eq!(
                require_role(&env, &addr, Role::Admin),
                Err(RoleError::InsufficientRole)
            );
        });
    }

    #[test]
    fn require_role_moderator_fails_for_viewer() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Viewer);
            assert_eq!(
                require_role(&env, &addr, Role::Moderator),
                Err(RoleError::InsufficientRole)
            );
        });
    }

    #[test]
    fn require_role_moderator_succeeds_for_admin() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Admin);
            assert!(require_role(&env, &addr, Role::Moderator).is_ok());
        });
    }

    #[test]
    fn higher_role_passes_lower_requirement() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            set_role(&env, &addr, Role::Admin);
            assert!(require_role(&env, &addr, Role::Viewer).is_ok());
            assert!(require_role(&env, &addr, Role::Moderator).is_ok());
        });
    }

    #[test]
    fn none_role_fails_any_non_none_requirement() {
        let (env, cid) = setup();
        let addr = Address::generate(&env);
        env.as_contract(&cid, || {
            assert_eq!(
                require_role(&env, &addr, Role::Viewer),
                Err(RoleError::InsufficientRole)
            );
            assert_eq!(
                require_role(&env, &addr, Role::Moderator),
                Err(RoleError::InsufficientRole)
            );
            assert_eq!(
                require_role(&env, &addr, Role::Admin),
                Err(RoleError::InsufficientRole)
            );
        });
    }
}
