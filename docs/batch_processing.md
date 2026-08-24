# Batch Processing System

## Overview

The StellarSwipe batch processing system optimizes contract operations by grouping multiple transactions together, significantly reducing gas costs and improving throughput. This comprehensive guide covers the architecture, usage patterns, and best practices for implementing batch operations.

## Table of Contents

1. [Architecture](#architecture)
2. [Core Components](#core-components)
3. [Batch Execution Modes](#batch-execution-modes)
4. [Usage Guide](#usage-guide)
5. [Performance Optimization](#performance-optimization)
6. [Error Handling](#error-handling)
7. [Best Practices](#best-practices)
8. [Integration Examples](#integration-examples)

---

## Architecture

### System Design

The batch processing system is built on four core pillars:

```
┌─────────────────────────────────────────────────────────┐
│                  Batch Processing Layer                  │
├─────────────────────────────────────────────────────────┤
│                                                           │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │  Aggregator  │  │   Executor   │  │  Optimizer   │  │
│  │              │  │              │  │              │  │
│  │ • Collection │  │ • Processing │  │ • Size Calc  │  │
│  │ • Validation │  │ • Rollback   │  │ • Adjustment │  │
│  │ • Timeout    │  │ • Modes      │  │ • Recommend  │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│                                                           │
│  ┌─────────────────────────────────────────────────┐    │
│  │         Rollback Manager                        │    │
│  │  • Savepoints  • State Snapshots  • Recovery   │    │
│  └─────────────────────────────────────────────────┘    │
│                                                           │
└─────────────────────────────────────────────────────────┘
```

### Key Features

- **Batch Aggregation**: Intelligent collection of operations with size limits and timeouts
- **Multiple Execution Modes**: AllOrNothing, BestEffort, and StopOnError strategies
- **Automatic Rollback**: State management with savepoints for failure recovery
- **Size Optimization**: Dynamic batch sizing based on gas costs and performance
- **Performance Tracking**: Comprehensive metrics and benchmarking tools

---

## Core Components

### 1. BatchAggregator

Collects and manages operations before execution.

```rust
pub struct BatchAggregator<T> {
    pub items: Vec<T>,
    pub batch_id: u64,
    pub created_at: u64,
    pub max_size: u32,
}
```

**Key Methods:**
- `new()`: Create a new aggregator with specified batch ID and max size
- `add()`: Add an item to the batch
- `is_ready()`: Check if batch is ready for processing (full or timeout)
- `size()`: Get current batch size
- `is_empty()`: Check if batch has no items

**Configuration:**
- `MAX_BATCH_SIZE`: 100 items (hard limit)
- `MIN_BATCH_SIZE`: 1 item
- `OPTIMAL_BATCH_SIZE`: 50 items (recommended default)
- `BATCH_TIMEOUT_SECONDS`: 300 seconds (5 minutes)

### 2. BatchExecutor

Processes batches according to specified execution mode.

```rust
pub struct BatchExecutor;
```

**Key Method:**
```rust
pub fn execute_batch<T, R, F>(
    env: &Env,
    items: Vec<T>,
    mode: BatchMode,
    processor: F,
) -> BatchResult<R>
```

**Returns:**
```rust
pub struct BatchResult<T> {
    pub successful: Vec<T>,
    pub failed: Vec<BatchError>,
    pub total_processed: u32,
    pub success_count: u32,
    pub failure_count: u32,
    pub gas_saved: u64,
}
```

### 3. BatchSizeOptimizer

Calculates and adjusts optimal batch sizes.

```rust
pub struct BatchSizeOptimizer {
    pub min_size: u32,
    pub max_size: u32,
    pub optimal_size: u32,
}
```

**Key Methods:**
- `calculate_optimal_size()`: Calculate size based on gas constraints
- `adjust_size()`: Dynamically adjust based on performance metrics
- `recommend_size()`: Get recommended size for operation type

### 4. BatchRollbackManager

Manages state snapshots and rollback operations.

```rust
pub struct BatchRollbackManager;
```

**Key Methods:**
- `create_savepoint()`: Create state snapshot before execution
- `rollback()`: Restore state from savepoint
- `commit()`: Finalize batch changes

---

## Batch Execution Modes

### AllOrNothing Mode

All operations must succeed or the entire batch is rolled back.

**Use Cases:**
- Financial transactions requiring atomicity
- State updates that must be consistent
- Critical operations where partial success is unacceptable

**Example:**
```rust
let result = BatchExecutor::execute_batch(
    &env,
    transfer_items,
    BatchMode::AllOrNothing,
    |env, item| process_transfer(env, item),
);

if result.failure_count > 0 {
    // Entire batch was rolled back
    panic!("Batch failed, all operations reverted");
}
```

**Characteristics:**
- ✅ Guarantees atomicity
- ✅ Maintains consistency
- ❌ Higher gas cost on failure
- ❌ All-or-nothing outcome

### BestEffort Mode

Process all items, continuing even if some fail.

**Use Cases:**
- Bulk notifications or updates
- Non-critical operations
- Operations where partial success is acceptable
- Maximum throughput scenarios

**Example:**
```rust
let result = BatchExecutor::execute_batch(
    &env,
    notification_items,
    BatchMode::BestEffort,
    |env, item| send_notification(env, item),
);

// Process results
for success in result.successful.iter() {
    log_success(&success);
}

for error in result.failed.iter() {
    log_error(&error);
}
```

**Characteristics:**
- ✅ Maximum throughput
- ✅ Partial success possible
- ✅ Lower gas cost
- ❌ No atomicity guarantee

### StopOnError Mode

Process items sequentially, stopping at the first error.

**Use Cases:**
- Sequential operations with dependencies
- Ordered processing requirements
- Early failure detection
- Resource-constrained scenarios

**Example:**
```rust
let result = BatchExecutor::execute_batch(
    &env,
    ordered_items,
    BatchMode::StopOnError,
    |env, item| process_sequential(env, item),
);

if result.failure_count > 0 {
    // Processing stopped at first error
    let first_error = result.failed.get(0).unwrap();
    handle_error(&first_error);
}
```

**Characteristics:**
- ✅ Early failure detection
- ✅ Preserves order
- ✅ Resource efficient
- ❌ Incomplete processing on error

---

## Usage Guide

### Basic Batch Processing

```rust
use soroban_sdk::{Env, Vec};
use batch_processor::{BatchAggregator, BatchExecutor, BatchMode};

// 1. Create aggregator
let mut aggregator = BatchAggregator::new(&env, batch_id, 50);

// 2. Add items
for item in items.iter() {
    aggregator.add(item)?;
}

// 3. Execute when ready
if aggregator.is_ready(&env) {
    let result = BatchExecutor::execute_batch(
        &env,
        aggregator.items,
        BatchMode::BestEffort,
        |env, item| process_item(env, item),
    );
    
    // 4. Handle results
    println!("Processed: {}/{}", 
        result.success_count, 
        result.total_processed
    );
    println!("Gas saved: {}", result.gas_saved);
}
```

### Batch Size Optimization

```rust
use batch_processor::BatchSizeOptimizer;

let optimizer = BatchSizeOptimizer::new();

// Calculate optimal size based on gas
let optimal_size = optimizer.calculate_optimal_size(
    avg_gas_per_item,
    max_gas_per_batch,
);

// Get recommendation for operation type
let recommended = optimizer.recommend_size(&BatchOperation::Transfer);

// Adjust based on performance
let adjusted = optimizer.adjust_size(
    current_size,
    success_rate,
    avg_processing_time,
);
```

### Rollback Management

```rust
use batch_processor::BatchRollbackManager;

// Create savepoint before execution
let savepoint = BatchRollbackManager::create_savepoint(&env, batch_id);

// Execute batch
let result = execute_critical_batch(&env, items);

if result.failure_count > 0 {
    // Rollback on failure
    BatchRollbackManager::rollback(&env, &savepoint)?;
} else {
    // Commit on success
    BatchRollbackManager::commit(&env, batch_id);
}
```

---

## Performance Optimization

### Recommended Batch Sizes by Operation

| Operation Type | Recommended Size | Rationale |
|---------------|------------------|-----------|
| Transfer | 50 | Balanced gas/throughput |
| Stake | 30 | Higher gas per operation |
| Unstake | 30 | Higher gas per operation |
| ClaimRewards | 40 | Moderate complexity |
| RegisterSignal | 20 | Complex validation |
| UpdateSignal | 60 | Lower gas cost |

### Dynamic Size Adjustment

The optimizer automatically adjusts batch sizes based on:

1. **Success Rate**
   - >95% success + fast processing → Increase by 20%
   - <80% success or slow processing → Decrease by 20%

2. **Processing Time**
   - <1 second → Consider increasing size
   - >5 seconds → Consider decreasing size

3. **Gas Efficiency**
   - Calculate optimal size: `max_gas / avg_gas_per_item`
   - Clamp to MIN_BATCH_SIZE and MAX_BATCH_SIZE

### Performance Metrics

```rust
pub struct BatchPerformanceMetrics {
    pub batch_size: u32,
    pub processing_time_ms: u64,
    pub gas_used: u64,
    pub gas_per_item: u64,
    pub throughput: u32,        // Items per second
    pub success_rate: u32,      // Percentage
    pub efficiency_score: u32,  // 0-100
}
```

**Efficiency Score Calculation:**
- 50% weight: Success rate
- 30% weight: Gas efficiency
- 20% weight: Throughput

---

## Error Handling

### Error Types

```rust
pub enum BatchProcessingError {
    BatchFull = 1,           // Batch reached max size
    BatchEmpty = 2,          // No items to process
    InvalidBatchSize = 3,    // Size outside valid range
    ProcessingFailed = 4,    // Item processing error
    RollbackFailed = 5,      // Rollback operation failed
    TimeoutExceeded = 6,     // Batch timeout reached
}
```

### Error Information

```rust
pub struct BatchError {
    pub index: u32,           // Item index in batch
    pub error_code: u32,      // Error type code
    pub error_message: String, // Human-readable message
}
```

### Error Handling Patterns

```rust
// Pattern 1: Check for specific errors
match result.failed.get(0) {
    Some(error) if error.error_code == BatchProcessingError::TimeoutExceeded as u32 => {
        // Handle timeout
    },
    Some(error) => {
        // Handle other errors
    },
    None => {
        // No errors
    }
}

// Pattern 2: Retry failed items
if result.failure_count > 0 {
    let failed_items = extract_failed_items(&original_items, &result.failed);
    retry_batch(&env, failed_items);
}

// Pattern 3: Partial success handling
if result.success_count > 0 && result.failure_count > 0 {
    commit_successful(&result.successful);
    log_failures(&result.failed);
}
```

---

## Best Practices

### 1. Choose the Right Execution Mode

- **Use AllOrNothing** for financial transactions and critical state updates
- **Use BestEffort** for bulk operations where partial success is acceptable
- **Use StopOnError** for sequential operations with dependencies

### 2. Optimize Batch Sizes

- Start with recommended sizes for your operation type
- Monitor performance metrics and adjust accordingly
- Consider gas limits and network conditions
- Use the BatchSizeOptimizer for dynamic adjustment

### 3. Handle Timeouts Appropriately

- Set reasonable timeout values (default: 5 minutes)
- Process batches before timeout when possible
- Implement timeout monitoring and alerts

### 4. Implement Proper Error Handling

- Always check `failure_count` in results
- Log failed items for debugging and retry
- Implement exponential backoff for retries
- Monitor error patterns for system issues

### 5. Monitor Performance

- Track gas savings and efficiency scores
- Monitor success rates and processing times
- Set up alerts for degraded performance
- Use benchmarking tools for optimization

### 6. Test Thoroughly

- Test all execution modes
- Test edge cases (empty batches, single items, max size)
- Test failure scenarios and rollback
- Benchmark performance under load

---

## Integration Examples

### Example 1: Batch Token Transfers

```rust
pub fn batch_transfer(
    env: Env,
    from: Address,
    transfers: Vec<TransferItem>,
) -> BatchResult<()> {
    // Validate inputs
    from.require_auth();
    
    // Create aggregator
    let mut aggregator = BatchAggregator::new(&env, get_next_batch_id(), 50);
    
    // Add transfers
    for transfer in transfers.iter() {
        aggregator.add(transfer)?;
    }
    
    // Execute batch
    BatchExecutor::execute_batch(
        &env,
        aggregator.items,
        BatchMode::AllOrNothing,
        |env, item| {
            // Process individual transfer
            transfer_tokens(env, &from, &item.to, item.amount)
        },
    )
}
```

### Example 2: Batch Staking Operations

```rust
pub fn batch_stake(
    env: Env,
    stakers: Vec<StakeRequest>,
) -> BatchResult<StakeReceipt> {
    // Optimize batch size
    let optimizer = BatchSizeOptimizer::new();
    let batch_size = optimizer.recommend_size(&BatchOperation::Stake);
    
    // Create aggregator with optimized size
    let mut aggregator = BatchAggregator::new(&env, get_next_batch_id(), batch_size);
    
    for request in stakers.iter() {
        aggregator.add(request)?;
    }
    
    // Create savepoint for rollback
    let savepoint = BatchRollbackManager::create_savepoint(&env, aggregator.batch_id);
    
    // Execute batch
    let result = BatchExecutor::execute_batch(
        &env,
        aggregator.items,
        BatchMode::AllOrNothing,
        |env, request| {
            stake_tokens(env, &request.staker, request.amount)
        },
    );
    
    // Handle result
    if result.failure_count > 0 {
        BatchRollbackManager::rollback(&env, &savepoint)?;
    } else {
        BatchRollbackManager::commit(&env, aggregator.batch_id);
    }
    
    result
}
```

### Example 3: Batch Reward Claims

```rust
pub fn batch_claim_rewards(
    env: Env,
    claimants: Vec<Address>,
) -> BatchResult<ClaimReceipt> {
    // Use BestEffort mode for non-critical operations
    let mut aggregator = BatchAggregator::new(&env, get_next_batch_id(), 40);
    
    for claimant in claimants.iter() {
        aggregator.add(claimant)?;
    }
    
    let result = BatchExecutor::execute_batch(
        &env,
        aggregator.items,
        BatchMode::BestEffort,
        |env, claimant| {
            claim_rewards(env, claimant)
        },
    );
    
    // Log results
    log_batch_results(&env, &result);
    
    result
}
```

### Example 4: Batch Signal Updates

```rust
pub fn batch_update_signals(
    env: Env,
    updates: Vec<SignalUpdate>,
) -> BatchResult<()> {
    // Use StopOnError for sequential updates
    let mut aggregator = BatchAggregator::new(&env, get_next_batch_id(), 60);
    
    for update in updates.iter() {
        aggregator.add(update)?;
    }
    
    let result = BatchExecutor::execute_batch(
        &env,
        aggregator.items,
        BatchMode::StopOnError,
        |env, update| {
            update_signal(env, &update.signal_id, &update.data)
        },
    );
    
    // Handle early termination
    if result.failure_count > 0 {
        let error = result.failed.get(0).unwrap();
        handle_update_error(&env, &error);
    }
    
    result
}
```

### Example 5: Performance Benchmarking

```rust
pub fn benchmark_operations(env: Env) {
    let test_items = generate_test_items(&env, 100);
    
    // Benchmark batch processing
    let metrics = benchmark_batch_processing(
        &env,
        test_items,
        |env, item| process_test_item(env, item),
    );
    
    // Analyze results
    println!("Batch Size: {}", metrics.batch_size);
    println!("Processing Time: {}ms", metrics.processing_time_ms);
    println!("Gas Used: {}", metrics.gas_used);
    println!("Gas Per Item: {}", metrics.gas_per_item);
    println!("Throughput: {} items/sec", metrics.throughput);
    println!("Success Rate: {}%", metrics.success_rate);
    println!("Efficiency Score: {}/100", metrics.efficiency_score);
    
    // Calculate gas savings
    let individual_cost = metrics.batch_size as u64 * 100000;
    let savings_percent = ((individual_cost - metrics.gas_used) * 100) / individual_cost;
    println!("Gas Savings: {}%", savings_percent);
}
```

---

## Advanced Topics

### Custom Batch Processors

You can create custom processors for specific use cases:

```rust
pub struct CustomBatchProcessor {
    pub config: ProcessorConfig,
}

impl CustomBatchProcessor {
    pub fn process_with_validation<T, R>(
        &self,
        env: &Env,
        items: Vec<T>,
        validator: impl Fn(&T) -> bool,
        processor: impl Fn(&Env, &T) -> Result<R, BatchProcessingError>,
    ) -> BatchResult<R> {
        // Filter valid items
        let valid_items = items.iter()
            .filter(|item| validator(item))
            .collect();
        
        // Process valid items
        BatchExecutor::execute_batch(
            env,
            valid_items,
            self.config.mode,
            processor,
        )
    }
}
```

### Nested Batch Processing

For complex workflows, you can nest batch operations:

```rust
pub fn process_nested_batches(env: Env) -> BatchResult<()> {
    // Outer batch: Process user groups
    let groups = get_user_groups(&env);
    
    BatchExecutor::execute_batch(
        &env,
        groups,
        BatchMode::BestEffort,
        |env, group| {
            // Inner batch: Process users in group
            let users = get_group_users(env, group);
            
            let inner_result = BatchExecutor::execute_batch(
                env,
                users,
                BatchMode::BestEffort,
                |env, user| process_user(env, user),
            );
            
            if inner_result.success_count > 0 {
                Ok(())
            } else {
                Err(BatchProcessingError::ProcessingFailed)
            }
        },
    )
}
```

---

## Troubleshooting

### Common Issues

**Issue: Batch timeout exceeded**
- Solution: Reduce batch size or increase timeout value
- Check: Network conditions and processing complexity

**Issue: High failure rate**
- Solution: Validate items before adding to batch
- Check: Input data quality and business logic

**Issue: Rollback failures**
- Solution: Ensure proper savepoint creation
- Check: State management and storage operations

**Issue: Poor gas efficiency**
- Solution: Optimize batch size using BatchSizeOptimizer
- Check: Operation complexity and gas limits

### Performance Tuning

1. **Monitor key metrics**: Success rate, gas usage, throughput
2. **Adjust batch sizes**: Use optimizer recommendations
3. **Choose appropriate mode**: Match mode to use case
4. **Implement caching**: Reduce redundant operations
5. **Profile operations**: Identify bottlenecks

---

## Conclusion

The StellarSwipe batch processing system provides a robust, efficient solution for optimizing contract operations. By following the guidelines and best practices in this document, you can achieve significant gas savings and improved throughput while maintaining reliability and consistency.

For more information, see:
- [Batch Processing Benchmarks](./batch_processing_benchmarks.md)
- [Performance Optimization Guide](./protocol23_optimization.md)
- [Architecture Documentation](./ARCHITECTURE.md)
