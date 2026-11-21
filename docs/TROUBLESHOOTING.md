# Troubleshooting Guide

## Introduction

This guide helps you diagnose and resolve common issues when developing with or integrating the StellarSwipe protocol.

---

## Table of Contents

1. [Connection Issues](#connection-issues)
2. [Contract Interaction Errors](#contract-interaction-errors)
3. [Transaction Failures](#transaction-failures)
4. [Build and Deployment Issues](#build-and-deployment-issues)
5. [Performance Problems](#performance-problems)
6. [Common Error Messages](#common-error-messages)

---

## Connection Issues

### Problem: Cannot Connect to Network

**Symptoms**:
```
Error: Network request failed
Error: Connection timeout
```

**Solutions**:

1. **Check Network URL**:
```typescript
// Verify correct RPC URL
const TESTNET_URL = 'https://soroban-testnet.stellar.org';
const MAINNET_URL = 'https://soroban-mainnet.stellar.org';

// Test connection
async function testConnection() {
    try {
        const server = new SorobanRpc.Server(TESTNET_URL);
        const health = await server.getHealth();
        console.log('Connection successful:', health);
    } catch (error) {
        console.error('Connection failed:', error);
    }
}
```

2. **Check Network Status**:
- Visit [Stellar Status Page](https://status.stellar.org)
- Check if testnet/mainnet is operational

3. **Verify Firewall Settings**:
```bash
# Test connectivity
curl https://soroban-testnet.stellar.org/health

# Check DNS resolution
nslookup soroban-testnet.stellar.org
```

4. **Use Alternative RPC Endpoints**:
```typescript
const RPC_ENDPOINTS = [
    'https://soroban-testnet.stellar.org',
    'https://rpc-testnet.stellar.org',
    // Add backup endpoints
];

async function connectWithFallback() {
    for (const endpoint of RPC_ENDPOINTS) {
        try {
            const server = new SorobanRpc.Server(endpoint);
            await server.getHealth();
            return server;
        } catch (error) {
            console.log(`Failed to connect to ${endpoint}`);
        }
    }
    throw new Error('All RPC endpoints failed');
}
```

### Problem: Slow Response Times

**Symptoms**:
- Requests taking >5 seconds
- Timeouts on contract calls

**Solutions**:

1. **Implement Caching**:
```typescript
const cache = new Map();
const CACHE_TTL = 60000; // 1 minute

async function getCachedData(key: string, fetchFn: () => Promise<any>) {
    const cached = cache.get(key);
    
    if (cached && Date.now() - cached.timestamp < CACHE_TTL) {
        return cached.data;
    }
    
    const data = await fetchFn();
    cache.set(key, { data, timestamp: Date.now() });
    
    return data;
}
```

2. **Use Connection Pooling**:
```typescript
class ConnectionPool {
    private servers: SorobanRpc.Server[] = [];
    private currentIndex = 0;
    
    constructor(urls: string[], poolSize: number = 3) {
        for (let i = 0; i < poolSize; i++) {
            this.servers.push(new SorobanRpc.Server(urls[i % urls.length]));
        }
    }
    
    getServer(): SorobanRpc.Server {
        const server = this.servers[this.currentIndex];
        this.currentIndex = (this.currentIndex + 1) % this.servers.length;
        return server;
    }
}
```

3. **Optimize Queries**:
```typescript
// Bad: Multiple sequential calls
const signal1 = await getSignal(1);
const signal2 = await getSignal(2);
const signal3 = await getSignal(3);

// Good: Parallel calls
const [signal1, signal2, signal3] = await Promise.all([
    getSignal(1),
    getSignal(2),
    getSignal(3)
]);
```

---

## Contract Interaction Errors

### Problem: Contract Not Found

**Error**:
```
Error: Contract not found: CXXX...
```

**Solutions**:

1. **Verify Contract ID**:
```typescript
function validateContractId(contractId: string): boolean {
    // Check format
    if (!contractId.startsWith('C')) {
        console.error('Invalid contract ID format');
        return false;
    }
    
    // Check length (56 characters)
    if (contractId.length !== 56) {
        console.error('Invalid contract ID length');
        return false;
    }
    
    return true;
}
```

2. **Check Network**:
```typescript
// Ensure you're on the correct network
const network = process.env.NETWORK || 'testnet';
const contractId = network === 'testnet' 
    ? TESTNET_CONTRACT_ID 
    : MAINNET_CONTRACT_ID;
```

3. **Verify Deployment**:
```bash
# Check if contract exists
soroban contract info \
  --id CXXX... \
  --network testnet
```

### Problem: Function Not Found

**Error**:
```
Error: Function 'get_signal' not found
```

**Solutions**:

1. **Check Function Name**:
```typescript
// Verify exact function name (case-sensitive)
const correctName = 'get_signal';  // ✓
const wrongName = 'getSignal';     // ✗
const wrongName2 = 'get_Signal';   // ✗
```

2. **Check Contract Version**:
```typescript
async function checkContractVersion() {
    const contract = new Contract(CONTRACT_ID);
    const version = await contract.call('get_version');
    console.log('Contract version:', version);
    
    if (version < REQUIRED_VERSION) {
        throw new Error('Contract version too old');
    }
}
```

3. **List Available Functions**:
```bash
# Get contract spec
soroban contract inspect \
  --wasm target/wasm32-unknown-unknown/release/contract.wasm
```

### Problem: Invalid Parameters

**Error**:
```
Error: Invalid parameter type
Error: Parameter count mismatch
```

**Solutions**:

1. **Validate Parameters**:
```typescript
function validateSignalParams(params: any): boolean {
    const required = ['provider', 'asset_pair', 'entry_price'];
    
    for (const field of required) {
        if (!(field in params)) {
            console.error(`Missing required field: ${field}`);
            return false;
        }
    }
    
    // Type validation
    if (typeof params.entry_price !== 'number') {
        console.error('entry_price must be a number');
        return false;
    }
    
    return true;
}
```

2. **Check Parameter Types**:
```typescript
// Ensure correct types
const params = {
    signal_id: Number(signalId),  // Convert to number
    provider: String(address),     // Convert to string
    amount: BigInt(amount)         // Convert to BigInt if needed
};
```

---

## Transaction Failures

### Problem: Insufficient Balance

**Error**:
```
Error: Insufficient balance for transaction
```

**Solutions**:

1. **Check Account Balance**:
```typescript
async function checkBalance(address: string) {
    const server = new SorobanRpc.Server(RPC_URL);
    const account = await server.getAccount(address);
    
    console.log('Balance:', account.balances);
    
    // Check if sufficient for transaction
    const xlmBalance = account.balances.find(b => b.asset_type === 'native');
    const minRequired = 1; // 1 XLM minimum
    
    if (parseFloat(xlmBalance.balance) < minRequired) {
        throw new Error('Insufficient XLM balance');
    }
}
```

2. **Fund Account (Testnet)**:
```bash
# Use Friendbot for testnet
curl "https://friendbot.stellar.org?addr=GXXX..."
```

3. **Estimate Transaction Cost**:
```typescript
async function estimateTransactionCost(transaction: Transaction) {
    const server = new SorobanRpc.Server(RPC_URL);
    
    const simulation = await server.simulateTransaction(transaction);
    
    console.log('Estimated cost:', {
        fee: simulation.minResourceFee,
        cpuInstructions: simulation.cost.cpuInsns,
        memoryBytes: simulation.cost.memBytes
    });
    
    return simulation.minResourceFee;
}
```

### Problem: Transaction Timeout

**Error**:
```
Error: Transaction timed out
```

**Solutions**:

1. **Increase Timeout**:
```typescript
const transaction = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: Networks.TESTNET
})
.addOperation(operation)
.setTimeout(60) // Increase from 30 to 60 seconds
.build();
```

2. **Check Transaction Status**:
```typescript
async function waitForTransaction(hash: string, maxAttempts: number = 10) {
    const server = new SorobanRpc.Server(RPC_URL);
    
    for (let i = 0; i < maxAttempts; i++) {
        try {
            const tx = await server.getTransaction(hash);
            
            if (tx.status === 'SUCCESS') {
                return tx;
            } else if (tx.status === 'FAILED') {
                throw new Error('Transaction failed');
            }
        } catch (error) {
            // Transaction not yet processed
        }
        
        await new Promise(resolve => setTimeout(resolve, 2000));
    }
    
    throw new Error('Transaction timeout');
}
```

### Problem: Authorization Failed

**Error**:
```
Error: Authorization required
Error: Signature verification failed
```

**Solutions**:

1. **Verify Signature**:
```typescript
// Ensure transaction is signed
transaction.sign(keypair);

// Verify signature
const isValid = transaction.signatures.length > 0;
if (!isValid) {
    throw new Error('Transaction not signed');
}
```

2. **Check Authorization Context**:
```typescript
// For contract calls, ensure proper authorization
const operation = contract.call('restricted_function', {
    caller: userAddress
});

// User must authorize this operation
// In frontend, use wallet to sign
```

3. **Debug Authorization**:
```typescript
async function debugAuthorization(transaction: Transaction) {
    console.log('Transaction signatures:', transaction.signatures.length);
    console.log('Required signers:', transaction.operations.length);
    
    // Simulate to check authorization
    const simulation = await server.simulateTransaction(transaction);
    console.log('Simulation result:', simulation);
}
```

---

## Build and Deployment Issues

### Problem: Build Fails

**Error**:
```
error: could not compile `contract`
```

**Solutions**:

1. **Check Rust Version**:
```bash
# Update Rust
rustup update

# Verify version
rustc --version  # Should be 1.70+
```

2. **Clean and Rebuild**:
```bash
# Clean build artifacts
cargo clean

# Rebuild
cargo build --target wasm32-unknown-unknown --release
```

3. **Check Dependencies**:
```bash
# Update dependencies
cargo update

# Check for conflicts
cargo tree
```

4. **Fix Common Errors**:
```rust
// Error: trait bounds not satisfied
// Solution: Add required trait bounds
impl<T: Clone + Debug> MyStruct<T> {
    // ...
}

// Error: cannot find type in this scope
// Solution: Add import
use soroban_sdk::Address;
```

### Problem: Contract Size Too Large

**Error**:
```
Error: Contract WASM exceeds size limit
```

**Solutions**:

1. **Optimize Build**:
```toml
# Cargo.toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
```

2. **Remove Unused Dependencies**:
```bash
# Find large dependencies
cargo bloat --release --target wasm32-unknown-unknown

# Remove unused
cargo clean
cargo build --release
```

3. **Use Soroban Optimizer**:
```bash
soroban contract optimize \
  --wasm target/wasm32-unknown-unknown/release/contract.wasm
```

### Problem: Deployment Fails

**Error**:
```
Error: Failed to deploy contract
```

**Solutions**:

1. **Check Account Funding**:
```bash
# Verify account has XLM
stellar account show GXXX...
```

2. **Verify Network Configuration**:
```bash
# List networks
soroban network ls

# Add network if missing
soroban network add testnet \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015"
```

3. **Check WASM File**:
```bash
# Verify WASM file exists
ls -lh target/wasm32-unknown-unknown/release/*.wasm

# Check file size
du -h target/wasm32-unknown-unknown/release/contract.wasm
```

---

## Performance Problems

### Problem: Slow Contract Calls

**Symptoms**:
- Contract calls taking >3 seconds
- High latency

**Solutions**:

1. **Profile Contract**:
```rust
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;
    
    #[test]
    fn benchmark_function() {
        let start = Instant::now();
        
        // Call function
        my_function();
        
        let duration = start.elapsed();
        println!("Time: {:?}", duration);
    }
}
```

2. **Optimize Storage Access**:
```rust
// Bad: Multiple reads
let value1 = env.storage().get(&key);
let value2 = env.storage().get(&key);

// Good: Single read
let value = env.storage().get(&key);
```

3. **Use Batch Operations**:
```rust
// Bad: Loop with individual operations
for item in items {
    process_item(&env, item);
}

// Good: Batch processing
process_items_batch(&env, items);
```

### Problem: High Gas Costs

**Symptoms**:
- Transactions costing more than expected
- Users complaining about fees

**Solutions**:

1. **Analyze Gas Usage**:
```typescript
async function analyzeGasUsage(transaction: Transaction) {
    const simulation = await server.simulateTransaction(transaction);
    
    console.log('Gas analysis:', {
        cpuInstructions: simulation.cost.cpuInsns,
        memoryBytes: simulation.cost.memBytes,
        fee: simulation.minResourceFee
    });
    
    return simulation;
}
```

2. **Optimize Contract Code**:
```rust
// Use efficient data structures
use soroban_sdk::Vec;  // Instead of custom vector

// Minimize storage operations
// Cache frequently accessed data
```

3. **Batch Transactions**:
```typescript
// Combine multiple operations
const transaction = new TransactionBuilder(account, { fee: BASE_FEE })
    .addOperation(operation1)
    .addOperation(operation2)
    .addOperation(operation3)
    .build();
```

---

## Common Error Messages

### Error: "HostError: Error(WasmVm, InvalidAction)"

**Cause**: Contract panic or assertion failure

**Solution**:
```rust
// Add proper error handling
pub fn my_function(env: Env) -> Result<(), Error> {
    // Instead of panic!
    if condition {
        return Err(Error::InvalidCondition);
    }
    
    Ok(())
}
```

### Error: "HostError: Error(Storage, MissingValue)"

**Cause**: Trying to access non-existent storage key

**Solution**:
```rust
// Check if key exists
if env.storage().has(&key) {
    let value = env.storage().get(&key).unwrap();
} else {
    // Handle missing key
    return Err(Error::KeyNotFound);
}

// Or use default value
let value = env.storage().get(&key).unwrap_or(default_value);
```

### Error: "Transaction malformed"

**Cause**: Invalid transaction structure

**Solution**:
```typescript
// Verify transaction structure
console.log('Operations:', transaction.operations.length);
console.log('Signatures:', transaction.signatures.length);
console.log('Sequence:', transaction.sequence);

// Rebuild transaction if needed
const rebuilt = TransactionBuilder.fromXDR(
    transaction.toXDR(),
    Networks.TESTNET
);
```

---

## Getting Help

### Debug Checklist

- [ ] Check error message carefully
- [ ] Verify network connection
- [ ] Confirm contract ID is correct
- [ ] Validate function parameters
- [ ] Check account balance
- [ ] Review transaction signatures
- [ ] Test on testnet first
- [ ] Check Stellar status page

### Resources

**Documentation**:
- [Soroban Docs](https://soroban.stellar.org)
- [Stellar Docs](https://developers.stellar.org)
- [StellarSwipe Docs](./README.md)

**Community**:
- [Stellar Discord](https://discord.gg/stellar)
- [Stack Exchange](https://stellar.stackexchange.com)
- [GitHub Issues](https://github.com/stellar/soroban-cli/issues)

**Tools**:
- [Stellar Laboratory](https://laboratory.stellar.org)
- [Stellar Expert](https://stellar.expert)
- [Status Page](https://status.stellar.org)

### Reporting Issues

When reporting issues, include:

1. **Error Message**: Full error text
2. **Code Sample**: Minimal reproducible example
3. **Environment**: OS, versions, network
4. **Steps**: How to reproduce
5. **Expected**: What should happen
6. **Actual**: What actually happens

**Template**:
```markdown
## Issue Description
[Brief description]

## Error Message
```
[Full error text]
```

## Code Sample
```typescript
[Minimal code to reproduce]
```

## Environment
- OS: macOS 13.0
- Node: 18.0.0
- Soroban CLI: 20.0.0
- Network: Testnet

## Steps to Reproduce
1. [Step 1]
2. [Step 2]
3. [Step 3]

## Expected Behavior
[What should happen]

## Actual Behavior
[What actually happens]
```

---

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01  
**Maintained By**: StellarSwipe Support Team
