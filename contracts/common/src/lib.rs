//! Shared utilities for Signal contracts.
//!
//! This crate provides canonical helpers used across the Signal contract
//! suite. The address validation helpers below are the single source of
//! truth for validating and normalizing Soroban addresses (contracts,
//! accounts, and authorized invokers) so that public entrypoints reject
//! malformed or unsupported inputs consistently.

use soroban_sdk::{Address, Env, IntoVal, Symbol, Val, Vec};

/// Stable error codes returned by the canonical address validation helpers.
///
/// These codes are part of the public contract surface and are documented
/// for SDK consumers. They must not be renumbered.
pub mod address_error {
    /// The supplied value is not a valid Soroban address.
    pub const INVALID_ADDRESS: u32 = 1;
    /// The address is valid but of an unsupported type for this context.
    pub const UNSUPPORTED_ADDRESS_TYPE: u32 = 2;
    /// The address is the zero/empty address and is not allowed here.
    pub const ZERO_ADDRESS: u32 = 3;
}

/// The kind of Soroban address being validated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AddressKind {
    /// A deployed contract address.
    Contract = 0,
    /// A user/account address.
    Account = 1,
    /// An address authorized to invoke a contract entrypoint.
    AuthorizedInvoker = 2,
}

/// Result type for address validation helpers.
pub type AddressResult = Result<Address, u32>;

/// Returns `true` when `address` is a well-formed Soroban address.
///
/// This performs the canonical structural check used across the suite. It
/// rejects the zero/empty address, which is never a valid target.
pub fn is_valid_address(env: &Env, address: &Address) -> bool {
    // `Address` is a host type; the only malformed case reachable from the
    // guest is the zero/empty address, which we reject explicitly.
    let _ = env;
    !is_zero_address(address)
}

/// Returns `true` when `address` is the zero/empty address.
pub fn is_zero_address(address: &Address) -> bool {
    // A zero address has no string representation in the host; compare
    // against the canonical empty string form.
    let s = address.to_string();
    s.len() == 0
}

/// Validates `address` as the given `kind`, returning a stable error code
/// on failure.
///
/// This is the canonical entrypoint used by public contract functions to
/// reject malformed or unsupported address inputs consistently.
pub fn validate_address(env: &Env, address: &Address, kind: AddressKind) -> AddressResult {
    if !is_valid_address(env, address) {
        return Err(address_error::INVALID_ADDRESS);
    }
    match kind {
        AddressKind::Contract | AddressKind::Account | AddressKind::AuthorizedInvoker => {
            Ok(address.clone())
        }
    }
}

/// Validates and normalizes an address for the given `kind`.
///
/// Normalization currently returns the address unchanged, but centralizing
/// it here lets callers rely on a single canonical behavior and keeps the
/// door open for future normalization rules without touching entrypoints.
pub fn normalize_address(env: &Env, address: &Address, kind: AddressKind) -> AddressResult {
    validate_address(env, address, kind)
}

/// Convenience helper for validating a contract address.
pub fn validate_contract_address(env: &Env, address: &Address) -> AddressResult {
    validate_address(env, address, AddressKind::Contract)
}

/// Convenience helper for validating an account address.
pub fn validate_account_address(env: &Env, address: &Address) -> AddressResult {
    validate_address(env, address, AddressKind::Account)
}

/// Convenience helper for validating an authorized invoker address.
pub fn validate_authorized_invoker(env: &Env, address: &Address) -> AddressResult {
    validate_address(env, address, AddressKind::AuthorizedInvoker)
}

/// Validates a list of authorized invokers, returning the first error code
/// encountered. Used by entrypoints that accept multiple invokers.
pub fn validate_authorized_invokers(env: &Env, invokers: &Vec<Address>) -> Result<(), u32> {
    for invoker in invokers.iter() {
        validate_authorized_invoker(env, &invoker)?;
    }
    Ok(())
}

/// Invokes `f` with the validated address, panicking with the stable error
/// code when validation fails. Intended for use in public entrypoints where
/// an invalid address should abort the call.
pub fn require_valid_address(env: &Env, address: &Address, kind: AddressKind) -> Address {
    match validate_address(env, address, kind) {
        Ok(addr) => addr,
        Err(code) => panic_with_address_error(env, code),
    }
}

/// Panics with a stable, documented error code for address validation
/// failures.
pub fn panic_with_address_error(env: &Env, code: u32) -> ! {
    let _ = env;
    panic!("address validation error: {}", code)
}

/// Marker used by tests to ensure the helpers remain usable from the host
/// test environment without pulling in contract-specific types.
#[allow(dead_code)]
fn _assert_val_roundtrip(env: &Env, address: &Address) -> Val {
    address.clone().into_val(env)
}

#[allow(dead_code)]
fn _assert_symbol(env: &Env, name: &str) -> Symbol {
    Symbol::new(env, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn valid_contract_address_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert!(is_valid_address(&env, &addr));
        assert_eq!(
            validate_contract_address(&env, &addr),
            Ok(addr.clone())
        );
    }

    #[test]
    fn valid_account_address_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(validate_account_address(&env, &addr), Ok(addr.clone()));
    }

    #[test]
    fn valid_authorized_invoker_is_accepted() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(
            validate_authorized_invoker(&env, &addr),
            Ok(addr.clone())
        );
    }

    #[test]
    fn normalize_returns_same_address() {
        let env = Env::default();
        let addr = Address::generate(&env);
        assert_eq!(
            normalize_address(&env, &addr, AddressKind::Contract),
            Ok(addr.clone())
        );
    }

    #[test]
    fn zero_address_is_rejected() {
        let env = Env::default();
        // The zero/empty address is not a valid target for any kind.
        let zero = Address::from_string(&soroban_sdk::String::from_str(&env, ""));
        assert!(is_zero_address(&zero));
        assert!(!is_valid_address(&env, &zero));
        assert_eq!(
            validate_address(&env, &zero, AddressKind::Contract),
            Err(address_error::INVALID_ADDRESS)
        );
        assert_eq!(
            validate_address(&env, &zero, AddressKind::Account),
            Err(address_error::INVALID_ADDRESS)
        );
        assert_eq!(
            validate_address(&env, &zero, AddressKind::AuthorizedInvoker),
            Err(address_error::INVALID_ADDRESS)
        );
    }

    #[test]
    fn authorized_invokers_list_validates_each_entry() {
        let env = Env::default();
        let mut invokers = Vec::new(&env);
        invokers.push_back(Address::generate(&env));
        invokers.push_back(Address::generate(&env));
        assert_eq!(validate_authorized_invokers(&env, &invokers), Ok(()));
    }

    #[test]
    fn authorized_invokers_list_rejects_zero_entry() {
        let env = Env::default();
        let mut invokers = Vec::new(&env);
        invokers.push_back(Address::generate(&env));
        invokers.push_back(Address::from_string(&soroban_sdk::String::from_str(&env, "")));
        assert_eq!(
            validate_authorized_invokers(&env, &invokers),
            Err(address_error::INVALID_ADDRESS)
        );
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(address_error::INVALID_ADDRESS, 1);
        assert_eq!(address_error::UNSUPPORTED_ADDRESS_TYPE, 2);
        assert_eq!(address_error::ZERO_ADDRESS, 3);
    }
}
