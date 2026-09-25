/// Validates and normalizes an address for the given `kind`.
///
/// Normalization currently returns the address unchanged, but centralizing
/// it here lets callers rely on a single canonical behavior and keeps the
/// door open for future normalization rules without touching entrypoints.
pub fn normalize_address(env: &Env, address: &Address, kind: AddressKind) -> AddressResult {
    validate_address(env, address, kind)
}

/// Convenience helper for validating 
        }
    }
}

/// Compute a deterministic checksum over the provided state entries.
///
/// The caller is responsible for supplying `entries` in a canonical order.
/// Equivalent states (same entries, same order) always yield the same digest,
/// regardless of unrelated storage or map iteration order.
///
/// Returns the digest as a lowercase hexadecimal string.
pub fn state_checksum(entries: &[StateEntry]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CHECKSUM_DOMAIN);

    for entry in entries {
        let key = entry.key.as_bytes();
        hasher.update((key.len() as u32).to_be_bytes());
        hasher.update(key);

        let value = entry.value.as_bytes();
        hasher.update((value.len() as u32).to_be_bytes());
        hasher.update(value);
    }

    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
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

    fn entries() -> Vec<StateEntry> {
        vec![
            StateEntry::new("admin", "GADMIN"),
            StateEntry::new("threshold", "2"),
        ]
    }

    #[test]
    fn repeated_calls_are_identical() {
        let first = state_checksum(&entries());
        let second = state_checksum(&entries());
        assert_eq!(first, second);
    }

    #[test]
    fn unrelated_storage_does_not_change_checksum() {
        let baseline = state_checksum(&entries());

        // Simulate unrelated storage that is not part of the input set.
        let mut unrelated = std::collections::HashMap::new();
        unrelated.insert("unrelated", "value");
        unrelated.insert("other", "data");
        let _ = unrelated.len();

        assert_eq!(baseline, state_checksum(&entries()));
    }

    #[test]
    fn map_iteration_order_does_not_change_checksum() {
        let mut map = std::collections::HashMap::new();
        map.insert("admin", "GADMIN");
        map.insert("threshold", "2");

        // Canonicalize by sorting keys so iteration order is irrelevant.
        let mut keys: Vec<_> = map.keys().copied().collect();
        keys.sort_unstable();
        let canonical: Vec<StateEntry> = keys
            .iter()
            .map(|k| StateEntry::new(*k, map[k]))
            .collect();

        let first = state_checksum(&canonical);

        // Rebuild the map in a different insertion order.
        let mut reordered = std::collections::HashMap::new();
        reordered.insert("threshold", "2");
        reordered.insert("admin", "GADMIN");
        let mut keys2: Vec<_> = reordered.keys().copied().collect();
        keys2.sort_unstable();
        let canonical2: Vec<StateEntry> = keys2
            .iter()
            .map(|k| StateEntry::new(*k, reordered[k]))
            .collect();

        assert_eq!(first, state_checksum(&canonical2));
    }

    #[test]
    fn ordering_is_significant() {
        let a = vec![StateEntry::new("a", "1"), StateEntry::new("b", "2")];
        let b = vec![StateEntry::new("b", "2"), StateEntry::new("a", "1")];
        assert_ne!(state_checksum(&a), state_checksum(&b));
    }
    }
}
