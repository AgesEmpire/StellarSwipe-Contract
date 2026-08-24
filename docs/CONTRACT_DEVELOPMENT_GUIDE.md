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
    signal_client.register_signal(&provider, &signal_data);
}
```

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_initialization

# Run with output
cargo test -- --nocapture

# Run with coverage
cargo tarpaulin --out Html
```

---

## Deploying Contracts

### Build Contract

```bash
# Build for deployment
cargo build --target wasm32-unknown-unknown --release

# Optimize WASM
soroban contract optimize \
  --wasm target/wasm32-unknown-unknown/release/contract.wasm
```

### Deploy to Testnet

```bash
# Configure network
soroban network add testnet \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015"

# Generate identity
soroban keys generate deployer

# Fund account
curl "https://friendbot.stellar.org?addr=$(soroban keys address deployer)"

# Deploy contract
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/contract.wasm \
  --source deployer \
  --network testnet
```

### Initialize Contract

```bash
# Invoke initialize function
soroban contract invoke \
  --id CONTRACT_ID \
  --source deployer \
  --network testnet \
  -- initialize \
  --admin ADMIN_ADDRESS
```

### Verify Deployment

```bash
# Check contract info
soroban contract info \
  --id CONTRACT_ID \
  --network testnet

# Test contract call
soroban contract invoke \
  --id CONTRACT_ID \
  --source deployer \
  --network testnet \
  -- get_info
```

---

## Best Practices

### Code Quality

**1. Use Clippy**:
```bash
cargo clippy --all-targets --all-features
```

**2. Format Code**:
```bash
cargo fmt
```

**3. Documentation**:
```rust
/// Transfers tokens from one account to another
///
/// # Arguments
/// * `from` - Source address
/// * `to` - Destination address
/// * `amount` - Amount to transfer
///
/// # Returns
/// * `Result<(), Error>` - Success or error
pub fn transfer(
    env: Env,
    from: Address,
    to: Address,
    amount: i128
) -> Result<(), Error> {
    // Implementation
}
```

### Security

**1. Input Validation**:
```rust
pub fn set_value(env: Env, value: i128) -> Result<(), Error> {
    if value < 0 {
        return Err(Error::InvalidValue);
    }
    if value > MAX_VALUE {
        return Err(Error::ValueTooLarge);
    }
    // Proceed
}
```

**2. Access Control**:
```rust
fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin = get_admin(env);
    if caller != &admin {
        return Err(Error::Unauthorized);
    }
    Ok(())
}
```

**3. Reentrancy Protection**:
```rust
pub fn withdraw(env: Env, caller: Address) -> Result<(), Error> {
    // Check
    let balance = get_balance(&env, &caller);
    
    // Effect
    set_balance(&env, &caller, 0);
    
    // Interaction
    transfer_tokens(&env, &caller, balance)?;
    
    Ok(())
}
```

### Gas Optimization

**1. Minimize Storage Operations**:
```rust
// Bad: Multiple reads
let value1 = env.storage().get(&key);
let value2 = env.storage().get(&key);

// Good: Single read
let value = env.storage().get(&key);
```

**2. Use Efficient Data Structures**:
```rust
// Use Vec for ordered data
let items: Vec<Item> = Vec::new(&env);

// Use Map for key-value pairs
let balances: Map<Address, i128> = Map::new(&env);
```

**3. Batch Operations**:
```rust
// Bad: Loop with multiple calls
for user in users {
    transfer(&env, &user, amount);
}

// Good: Batch transfer
batch_transfer(&env, users, amounts);
```

---

## Common Patterns

### Upgradeable Contracts

```rust
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
    let admin = get_admin(&env);
    admin.require_auth();
    
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}
```

### Pausable Contracts

```rust
pub fn pause(env: Env) {
    require_admin(&env)?;
    env.storage().instance().set(&DataKey::Paused, &true);
}

pub fn unpause(env: Env) {
    require_admin(&env)?;
    env.storage().instance().set(&DataKey::Paused, &false);
}

fn require_not_paused(env: &Env) -> Result<(), Error> {
    let paused = env.storage().instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    
    if paused {
        return Err(Error::ContractPaused);
    }
    Ok(())
}
```

### Time-Locked Operations

```rust
pub fn propose_change(env: Env, change: Change) -> u64 {
    let proposal_id = get_next_id(&env);
    let execute_after = env.ledger().timestamp() + TIMELOCK_PERIOD;
    
    env.storage().persistent().set(
        &DataKey::Proposal(proposal_id),
        &Proposal {
            change,
            execute_after,
            executed: false,
        }
    );
    
    proposal_id
}

pub fn execute_change(env: Env, proposal_id: u64) -> Result<(), Error> {
    let proposal = get_proposal(&env, proposal_id)?;
    
    if env.ledger().timestamp() < proposal.execute_after {
        return Err(Error::TimelockNotExpired);
    }
    
    // Execute change
    apply_change(&env, &proposal.change)?;
    
    Ok(())
}
```

---

## Troubleshooting

### Common Issues

**Issue 1: Contract Size Too Large**
```
Error: Contract WASM size exceeds limit
```

**Solution**:
- Remove unused dependencies
- Optimize code
- Use `cargo-bloat` to identify large dependencies
```bash
cargo install cargo-bloat
cargo bloat --release --target wasm32-unknown-unknown
```

**Issue 2: Storage Access Errors**
```
Error: Storage key not found
```

**Solution**:
- Always check if key exists before accessing
```rust
let value = env.storage().persistent()
    .get(&key)
    .unwrap_or(default_value);
```

**Issue 3: Authorization Failures**
```
Error: Authorization failed
```

**Solution**:
- Ensure `require_auth()` is called
- Check authorization context
```rust
caller.require_auth();
```

### Debugging Tips

**1. Use Logging**:
```rust
#[cfg(test)]
use soroban_sdk::log;

log!(&env, "Debug value: {}", value);
```

**2. Test Incrementally**:
- Write tests for each function
- Test edge cases
- Use `#[should_panic]` for error cases

**3. Check Events**:
```rust
let events = env.events().all();
assert_eq!(events.len(), 1);
```

---

## Additional Resources

### Documentation
- [Soroban Docs](https://soroban.stellar.org)
- [Stellar Docs](https://developers.stellar.org)
- [Rust Book](https://doc.rust-lang.org/book/)

### Tools
- [Soroban CLI](https://github.com/stellar/soroban-cli)
- [Stellar Laboratory](https://laboratory.stellar.org)
- [Stellar Expert](https://stellar.expert)

### Community
- [Stellar Discord](https://discord.gg/stellar)
- [Stellar Stack Exchange](https://stellar.stackexchange.com)
- [GitHub Discussions](https://github.com/stellar/soroban-examples/discussions)

---

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01  
**Maintained By**: StellarSwipe Core Team
