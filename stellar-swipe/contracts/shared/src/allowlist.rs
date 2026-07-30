//! Cross-contract call allowlist (Issue: cross-contract allowlist security).
//!
//! Provides a centralized allowlist model for restricting which contracts can
//! invoke sensitive cross-contract entrypoints. Contracts register trusted
//! counterpart addresses via `add_allowed_contract` and enforce the allowlist
//! in privileged flows via `require_allowed_contract`.

use soroban_sdk::{contracterror, contracttype, Address, Env, Vec};

// ── Storage key ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum AllowlistStorageKey {
    /// List of allowed contract addresses for a specific entrypoint.
    AllowedContracts(Address),
    /// Global list of all allowed contracts (used for admin queries).
    GlobalAllowedContracts,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AllowlistError {
    /// The caller contract is not in the allowlist for this entrypoint.
    ContractNotAllowed = 1,
    /// Attempted to add a contract that is already in the allowlist.
    ContractAlreadyAllowed = 2,
    /// Attempted to remove a contract that is not in the allowlist.
    ContractNotInAllowlist = 3,
    /// Allowlist is at maximum capacity.
    AllowlistFull = 4,
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of contracts allowed in a single allowlist.
/// Prevents unbounded storage growth and keeps lookups reasonably fast.
pub const MAX_ALLOWLIST_SIZE: u32 = 50;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns the list of allowed contracts for the given entrypoint.
fn get_allowed_contracts(env: &Env, entrypoint: &Address) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&AllowlistStorageKey::AllowedContracts(entrypoint.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Save the allowed contracts list for the given entrypoint.
fn set_allowed_contracts(env: &Env, entrypoint: &Address, contracts: &Vec<Address>) {
    env.storage().instance().set(
        &AllowlistStorageKey::AllowedContracts(entrypoint.clone()),
        contracts,
    );
}

/// Check if a contract is in the allowlist.
fn is_in_allowlist(contracts: &Vec<Address>, target: &Address) -> bool {
    for i in 0..contracts.len() {
        if contracts.get(i).unwrap() == *target {
            return true;
        }
    }
    false
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Add a contract to the allowlist for the specified entrypoint.
/// Only the admin should call this. Returns `ContractAlreadyAllowed` if the
/// contract is already in the list, or `AllowlistFull` if the list is at capacity.
pub fn add_allowed_contract(
    env: &Env,
    entrypoint: &Address,
    contract: &Address,
) -> Result<(), AllowlistError> {
    let mut contracts = get_allowed_contracts(env, entrypoint);
    
    if is_in_allowlist(&contracts, contract) {
        return Err(AllowlistError::ContractAlreadyAllowed);
    }
    
    if contracts.len() >= MAX_ALLOWLIST_SIZE {
        return Err(AllowlistError::AllowlistFull);
    }
    
    contracts.push_back(contract.clone());
    set_allowed_contracts(env, entrypoint, &contracts);
    emit_contract_allowed(env, entrypoint, contract);
    Ok(())
}

/// Remove a contract from the allowlist for the specified entrypoint.
/// Only the admin should call this. Returns `ContractNotInAllowlist` if the
/// contract is not in the list.
pub fn remove_allowed_contract(
    env: &Env,
    entrypoint: &Address,
    contract: &Address,
) -> Result<(), AllowlistError> {
    let mut contracts = get_allowed_contracts(env, entrypoint);
    
    if !is_in_allowlist(&contracts, contract) {
        return Err(AllowlistError::ContractNotInAllowlist);
    }
    
    let mut new_contracts = Vec::new(env);
    for i in 0..contracts.len() {
        let addr = contracts.get(i).unwrap();
        if addr != *contract {
            new_contracts.push_back(addr);
        }
    }
    
    set_allowed_contracts(env, entrypoint, &new_contracts);
    emit_contract_removed(env, entrypoint, contract);
    Ok(())
}

/// Check that the calling contract is in the allowlist for this entrypoint.
/// Returns `Ok(())` if allowed, otherwise `Err(AllowlistError::ContractNotAllowed)`.
/// Use this in sensitive entrypoints to enforce the allowlist.
pub fn require_allowed_contract(
    env: &Env,
    entrypoint: &Address,
    caller: &Address,
) -> Result<(), AllowlistError> {
    let contracts = get_allowed_contracts(env, entrypoint);
    if is_in_allowlist(&contracts, caller) {
        Ok(())
    } else {
        Err(AllowlistError::ContractNotAllowed)
    }
}

/// Get all allowed contracts for a specific entrypoint (for admin queries).
pub fn get_allowlist(env: &Env, entrypoint: &Address) -> Vec<Address> {
    get_allowed_contracts(env, entrypoint)
}

/// Check if a contract is allowed without requiring auth (read-only query).
pub fn is_contract_allowed(env: &Env, entrypoint: &Address, contract: &Address) -> bool {
    let contracts = get_allowed_contracts(env, entrypoint);
    is_in_allowlist(&contracts, contract)
}

// ── Events ────────────────────────────────────────────────────────────────────

fn emit_contract_allowed(env: &Env, entrypoint: &Address, contract: &Address) {
    env.events().publish(
        (soroban_sdk::symbol_short!("allow_add"), entrypoint.clone()),
        contract.clone(),
    );
}

fn emit_contract_removed(env: &Env, entrypoint: &Address, contract: &Address) {
    env.events().publish(
        (soroban_sdk::symbol_short!("allow_rm"), entrypoint.clone()),
        contract.clone(),
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, testutils::Address as _, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        let contract_id = env.register(TestContract, ());
        let entrypoint = Address::generate(&env);
        let allowed = Address::generate(&env);
        (env, contract_id, entrypoint, allowed)
    }

    #[test]
    fn add_allowed_contract_succeeds() {
        let (env, cid, entrypoint, allowed) = setup();
        env.as_contract(&cid, || {
            assert!(add_allowed_contract(&env, &entrypoint, &allowed).is_ok());
            assert!(is_contract_allowed(&env, &entrypoint, &allowed));
        });
    }

    #[test]
    fn add_duplicate_contract_fails() {
        let (env, cid, entrypoint, allowed) = setup();
        env.as_contract(&cid, || {
            add_allowed_contract(&env, &entrypoint, &allowed).unwrap();
            assert_eq!(
                add_allowed_contract(&env, &entrypoint, &allowed),
                Err(AllowlistError::ContractAlreadyAllowed)
            );
        });
    }

    #[test]
    fn remove_allowed_contract_succeeds() {
        let (env, cid, entrypoint, allowed) = setup();
        env.as_contract(&cid, || {
            add_allowed_contract(&env, &entrypoint, &allowed).unwrap();
            assert!(remove_allowed_contract(&env, &entrypoint, &allowed).is_ok());
            assert!(!is_contract_allowed(&env, &entrypoint, &allowed));
        });
    }

    #[test]
    fn remove_non_existent_contract_fails() {
        let (env, cid, entrypoint, allowed) = setup();
        env.as_contract(&cid, || {
            assert_eq!(
                remove_allowed_contract(&env, &entrypoint, &allowed),
                Err(AllowlistError::ContractNotInAllowlist)
            );
        });
    }

    #[test]
    fn require_allowed_contract_passes_for_allowed() {
        let (env, cid, entrypoint, allowed) = setup();
        env.as_contract(&cid, || {
            add_allowed_contract(&env, &entrypoint, &allowed).unwrap();
            assert!(require_allowed_contract(&env, &entrypoint, &allowed).is_ok());
        });
    }

    #[test]
    fn require_allowed_contract_fails_for_unapproved() {
        let (env, cid, entrypoint, _) = setup();
        let unapproved = Address::generate(&env);
        env.as_contract(&cid, || {
            assert_eq!(
                require_allowed_contract(&env, &entrypoint, &unapproved),
                Err(AllowlistError::ContractNotAllowed)
            );
        });
    }

    #[test]
    fn allowlist_respects_max_capacity() {
        let (env, cid, entrypoint, _) = setup();
        env.as_contract(&cid, || {
            // Add MAX_ALLOWLIST_SIZE contracts
            for _ in 0..MAX_ALLOWLIST_SIZE {
                let addr = Address::generate(&env);
                add_allowed_contract(&env, &entrypoint, &addr).unwrap();
            }
            // Next one should fail
            let overflow = Address::generate(&env);
            assert_eq!(
                add_allowed_contract(&env, &entrypoint, &overflow),
                Err(AllowlistError::AllowlistFull)
            );
        });
    }

    #[test]
    fn get_allowlist_returns_all_added_contracts() {
        let (env, cid, entrypoint, _) = setup();
        env.as_contract(&cid, || {
            let addr1 = Address::generate(&env);
            let addr2 = Address::generate(&env);
            add_allowed_contract(&env, &entrypoint, &addr1).unwrap();
            add_allowed_contract(&env, &entrypoint, &addr2).unwrap();
            
            let list = get_allowlist(&env, &entrypoint);
            assert_eq!(list.len(), 2);
            assert!(is_in_allowlist(&list, &addr1));
            assert!(is_in_allowlist(&list, &addr2));
        });
    }
}
