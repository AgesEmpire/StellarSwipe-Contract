# Contract Development Guide

## Introduction

This guide provides comprehensive instructions for developing, testing, and deploying smart contracts for the StellarSwipe protocol on Stellar's Soroban platform.

---

## Table of Contents

1. [Development Environment Setup](#development-environment-setup)
2. [Project Structure](#project-structure)
3. [Writing Contracts](#writing-contracts)
4. [Testing Contracts](#testing-contracts)
5. [Deploying Contracts](#deploying-contracts)
6. [Best Practices](#best-practices)
7. [Common Patterns](#common-patterns)
8. [Troubleshooting](#troubleshooting)
9. [Contract Interface Semver Compatibility Policy](#contract-interface-semver-compatibility-policy)

---

## Development Environment Setup

### Prerequisites

**Required Software**:
- Rust 1.70 or later
- Soroban CLI
- Stellar CLI
- Git
- Code editor (VS Code recommended)

### Installation Steps

#### 1. Install Rust

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add wasm target
rustup target add wasm32-unknown-unknown
```

#### 2. Install Soroban CLI

```bash
cargo install --locked soroban-cli
```

#### 3. Install Stellar CLI

```bash
cargo install --locked stellar-cli
```

#### 4. Verify Installation

```bash
soroban --version
stellar --version
rustc --version
```

### IDE Setup

**VS Code Extensions**:
- rust-analyzer
- CodeLLDB (for debugging)
- Better TOML
- Soroban snippets

**VS Code Settings** (`.vscode/settings.json`):
```json
{
  "rust-analyzer.cargo.target": "wasm32-unknown-unknown",
  "rust-analyzer.checkOnSave.command": "clippy",
  "editor.formatOnSave": true
}
```

---

## Project Structure

### Standard Layout

```
stellar-swipe-contract/
├── contracts/
│   ├── signal_registry/
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── types.rs
│   │   │   ├── storage.rs
│   │   │   └── test.rs
│   │   └── Cargo.toml
│   ├── stake_vault/
│   └── fee_collector/
├── docs/
├── tests/
│   ├── integration/
│   └── e2e/
├── scripts/
│   ├── deploy.sh
│   └── test.sh
├── Cargo.toml
└── README.md
```

### Contract Structure

**Typical Contract Layout**:
```
contract/
├── src/
│   ├── lib.rs          # Main contract code
│   ├── types.rs        # Data structures
│   ├── storage.rs      # Storage helpers
│   ├── events.rs       # Event definitions
│   ├── errors.rs       # Error types
│   └── test.rs         # Unit tests
└── Cargo.toml          # Dependencies
```

---

## Writing Contracts

### Basic Contract Template

```rust
#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

#[contract]
pub struct MyContract;

#[contractimpl]
impl MyContract {
    /// Initialize the contract
    pub fn initialize(env: Env, admin: Address) {
        // Initialization logic
        env.storage().instance().set(&DataKey::Admin, &admin);
    }
    
    /// Example function
    pub fn do_something(env: Env, caller: Address, value: i128) -> i128 {
        // Verify caller
        caller.require_auth();
        
        // Business logic
        let result = value * 2;
        
        // Emit event
        env.events().publish(("action_performed",), (caller, result));
        
        result
    }
}

#[cfg(test)]
mod test {
    use super::*;
    
    #[test]
    fn test_do_something() {
        let env = Env::default();
        let contract_id = env.register_contract(None, MyContract);
        let client = MyContractClient::new(&env, &contract_id);
        
        let result = client.do_something(&user, &100);
        assert_eq!(result, 200);
    }
}
```

### Data Types

**Primitive Types**:
```rust
use soroban_sdk::{
    Address,    // Stellar address
    String,     // String type
    Symbol,     // Symbol (short string)
    Bytes,      // Byte array
    Vec,        // Vector
    Map,        // Map/Dictionary
};
```

**Custom Types**:
```rust
use soroban_sdk::contracttype;

#[derive(Clone)]
#[contracttype]
pub struct Signal {
    pub id: u64,
    pub provider: Address,
    pub price: i128,
    pub timestamp: u64,
}

#[contracttype]
pub enum SignalStatus {
    Active,
    Completed,
    Cancelled,
}
```

### Storage Operations

**Storage Types**:
- **Persistent**: Long-term storage
- **Temporary**: Short-term storage (cheaper)
- **Instance**: Contract-level configuration

**Storage Examples**:
```rust
use soroban_sdk::storage::Storage;

// Persistent storage
env.storage().persistent().set(&key, &value);
let value = env.storage().persistent().get(&key);

// Temporary storage
env.storage().temporary().set(&key, &value, 100); // TTL: 100 ledgers

// Instance storage
env.storage().instance().set(&key, &value);
```

### Access Control

**Authorization Pattern**:
```rust
pub fn restricted_function(env: Env, caller: Address) {
    // Require caller authorization
    caller.require_auth();
    
    // Check if caller is admin
    let admin: Address = env.storage().instance()
        .get(&DataKey::Admin)
        .unwrap();
    
    if caller != admin {
        panic!("Unauthorized");
    }
    
    // Proceed with function logic
}
```

### Events

**Emitting Events**:
```rust
// Simple event
env.events().publish(("signal_created",), signal_id);

// Event with multiple topics
env.events().publish(
    ("signal_updated", signal_id),
    (provider, new_status)
);

// Structured event
#[contracttype]
pub struct SignalCreatedEvent {
    pub signal_id: u64,
    pub provider: Address,
    pub timestamp: u64,
}

env.events().publish(
    ("signal_created",),
    SignalCreatedEvent {
        signal_id,
        provider,
        timestamp: env.ledger().timestamp(),
    }
);
```

### Error Handling

**Custom Errors**:
```rust
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyInitialized = 1,
    NotAuthorized = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
}

// Usage
pub fn transfer(env: Env, amount: i128) -> Result<(), Error> {
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    
    // Transfer logic
    Ok(())
}
```

---

## Testing Contracts

### Unit Tests

**Basic Test Structure**:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{Env, testutils::Address as _};
    
    #[test]
    fn test_initialization() {
        let env = Env::default();
        let contract_id = env.register_contract(None, MyContract);
        let client = MyContractClient::new(&env, &contract_id);
        
        let admin = Address::generate(&env);
        client.initialize(&admin);
        
        // Assertions
        assert_eq!(client.get_admin(), admin);
    }
    
    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_unauthorized_access() {
        let env = Env::default();
        let contract_id = env.register_contract(None, MyContract);
        let client = MyContractClient::new(&env, &contract_id);
        
        let unauthorized = Address::generate(&env);
        client.admin_only_function(&unauthorized);
    }
}
```

**Testing with Mock Data**:
```rust
#[test]
fn test_with_mock_data() {
    let env = Env::default();
    env.mock_all_auths(); // Mock all authorizations
    
    let contract_id = env.register_contract(None, MyContract);
    let client = MyContractClient::new(&env, &contract_id);
    
    // Create mock addresses
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    
    // Test logic
    client.transfer(&user1, &user2, &1000);
}
```

### Integration Tests

**Multi-Contract Testing**:
```rust
#[test]
fn test_contract_interaction() {
    let env = Env::default();
    
    // Deploy multiple contracts
    let signal_registry_id = env.register_contract(None, SignalRegistry);
    let stake_vault_id = env.register_contract(None, StakeVault);
    
    let signal_client = SignalRegistryClient::new(&env, &signal_registry_id);
    let stake_client = StakeVaultClient::new(&env, &stake_vault_id);
    
    // Test interaction
    let provider = Address::generate(&env);
    stake_client.stake(&provider, &10000);
    signal_client.register_signal(&provider, &1000);
}
```

---

## Contract Interface Semver Compatibility Policy

This section defines the semantic versioning rules for the **public interface** of every
Soroban contract in this repository. The public interface is the contract's ABI as
consumed by external callers and indexers, and it is versioned independently of internal
implementation details.

### What counts as the public interface

- **Entrypoints**: exported `#[contractimpl]` function names, their parameter order and
  types, and their return types.
- **Argument types**: `#[contracttype]` structs/enums used as parameters, including field
  names, field order, and field types.
- **Return values**: the return type of each entrypoint, including nested `#[contracttype]`
  shapes and `Result<T, E>` error types.
- **Events**: event topic tuples and the payload `#[contracttype]` shape published via
  `env.events().publish(...)`.
- **Errors**: `#[contracterror]` enum variants and their assigned numeric codes.
- **Storage schemas**: `DataKey` variants and the `#[contracttype]` values stored under
  them, including persistent/temporary/instance placement.

### Versioning scheme

The interface version is a `MAJOR.MINOR.PATCH` triple recorded in contract metadata
(see below). It is bumped according to the classification table that follows.

### Breaking vs non-breaking changes

| Change | Classification | Version bump |
| --- | --- | --- |
| Remove or rename a public entrypoint | Breaking | MAJOR |
| Add a required parameter to an entrypoint | Breaking | MAJOR |
| Change a parameter or return type incompatibly | Breaking | MAJOR |
| Reorder `#[contracttype]` struct fields | Breaking | MAJOR |
| Remove or rename a `#[contracttype]` field | Breaking | MAJOR |
| Change an event topic tuple or payload shape | Breaking | MAJOR |
| Remove or renumber a `#[contracterror]` variant | Breaking | MAJOR |
| Change a `DataKey` variant or its stored value type | Breaking | MAJOR |
| Change storage tier (persistent/temporary/instance) for a key | Breaking | MAJOR |
| Add a new public entrypoint | Non-breaking | MINOR |
| Add an optional parameter with a default | Non-breaking | MINOR |
| Add a new `#[contracttype]` field at the end | Non-breaking | MINOR |
| Add a new event | Non-breaking | MINOR |
| Add a new `#[contracterror]` variant with a new code | Non-breaking | MINOR |
| Add a new `DataKey` variant | Non-breaking | MINOR |
| Internal refactor with no ABI change | Non-breaking | PATCH |
| Documentation or comment-only change | Non-breaking | PATCH |

Any change not listed above must be reviewed by a maintainer and classified explicitly
before merge.

### Recording the interface version in contract metadata

Each contract exposes its interface version through a dedicated read-only entrypoint so
that tooling and indexers can discover it on-chain:

```rust
/// Returns the semantic version of this contract's public interface.
pub fn interface_version(env: Env) -> String {
    String::from_str(&env, "1.0.0")
}
```

When the interface version is bumped, update the string literal in this entrypoint in the
same pull request as the interface change. The value MUST match the classification table
above.

### CI detection of unsupported breaking changes

CI compares the current interface against the prior release and fails the build when a
breaking change is detected without a corresponding MAJOR bump. The check runs as part of
the contract CI job:

```bash
# scripts/check-interface-compat.sh
# Fails if the public interface changed in a breaking way without a MAJOR bump.
./scripts/check-interface-compat.sh --base "$PRIOR_RELEASE_TAG" --head HEAD
```

A breaking change is only accepted when the `interface_version` entrypoint has been bumped
to a new MAJOR value in the same change set. Otherwise CI reports the offending entrypoint,
type, event, error, or storage key and blocks the merge.

### Maintainer compatibility review checklist

Before approving any change that touches a contract's public interface, confirm:

- [ ] Every entrypoint, argument type, return value, event, error, and storage key change
      has been classified as breaking or non-breaking using the table above.
- [ ] The `interface_version` entrypoint reflects the correct MAJOR/MINOR/PATCH bump.
- [ ] Breaking changes are accompanied by a MAJOR bump and a migration note for callers.
- [ ] Non-breaking additions are additive only (no reordering, renaming, or removal).
- [ ] CI interface-compatibility check passes against the prior release tag.
- [ ] Downstream consumers (indexers, SDKs, frontends) are notified of breaking changes.

---

## Deploying Contracts

### Build for Production

```bash
# Build optimized WASM
cargo build --target wasm32-unknown-unknown --release

# Optimize WASM
soroban contract optimize \
  --wasm target/wasm32-unknown-unknown/release/my_contract.wasm
```

### Deploy to Testnet

```bash
# Deploy contract
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/my_contract.wasm \
  --source admin \
  --network testnet
```

### Deploy to Mainnet

```bash
# Deploy with verification
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/my_contract.wasm \
  --source admin \
  --network mainnet
```

---

## Best Practices

### 1. Always Verify Authorization

```rust
// Good
pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
    from.require_auth();
    // Transfer logic
}

// Bad - no authorization check
pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
    // Transfer logic without auth
}
```

### 2. Use Custom Errors

```rust
// Good
return Err(Error::InsufficientBalance);

// Bad
panic!("Insufficient balance");
```

### 3. Emit Events for State Changes

```rust
// Good
env.storage().persistent().set(&key, &value);
env.events().publish(("state_changed",), (key, value));

// Bad - no event
env.storage().persistent().set(&key, &value);
```

### 4. Validate Inputs

```rust
// Good
if amount <= 0 {
    return Err(Error::InvalidAmount);
}

// Bad - no validation
// Proceed with amount directly
```

### 5. Use Appropriate Storage Types

```rust
// Good - temporary for short-lived data
env.storage().temporary().set(&key, &value, 100);

// Good - persistent for long-term data
env.storage().persistent().set(&key, &value);
```

---

## Common Patterns

### Pattern: Initialization Guard

```rust
pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
    if env.storage().instance().has(&DataKey::Admin) {
        return Err(Error::AlreadyInitialized);
    }
    
    env.storage().instance().set(&DataKey::Admin, &admin);
    Ok(())
}
```

### Pattern: Access Control

```rust
fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin: Address = env.storage().instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotAuthorized)?;
    
    if caller != &admin {
        return Err(Error::NotAuthorized);
    }
    
    Ok(())
}
```

### Pattern: Pagination

```rust
pub fn get_items(env: Env, start: u32, limit: u32) -> Vec<Item> {
    let mut items = Vec::new(&env);
    let total = get_total_count(&env);
    
    let end = (start + limit).min(total);
    for i in start..end {
        if let Some(item) = get_item(&env, i) {
            items.push_back(item);
        }
    }
    
    items
}
```

---

## Troubleshooting

### Common Issues

**Issue: Contract fails to deploy**
- Check WASM size (must be < 64KB)
- Verify network configuration
- Ensure sufficient balance for deployment

**Issue: Transaction fails with "Unauthorized"**
- Verify `require_auth()` is called
- Check caller has proper permissions
- Ensure auth is mocked in tests

**Issue: Storage not persisting**
- Check storage type (persistent vs temporary)
- Verify TTL settings
- Ensure key is correct

**Issue: Events not appearing**
- Verify event is published
- Check event topic format
- Ensure indexer is running

### Debugging Tips

```bash
# Enable debug logging
RUST_LOG=debug soroban contract invoke ...

# Check contract state
soroban contract read --id <contract-id> --key <key>

# Simulate transaction
soroban contract invoke --id <contract-id> --fn <function> -- --help
```

---

## Additional Resources

- [Soroban Documentation](https://soroban.stellar.org/docs)
- [Stellar Developer Docs](https://developers.stellar.org)
- [Soroban Examples](https://github.com/stellar/soroban-examples)
- [Stellar Discord](https://discord.gg/stellar)

---

**Last Updated**: 2024
**Maintainers**: StellarSwipe Team
