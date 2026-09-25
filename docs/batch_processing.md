# Batch Processing System

## Overview

The StellarSwipe batch processing system optimizes contract operations by grouping multiple transactions together, significantly reducing gas costs and improving throughput. This comprehensive guide covers the architecture, usage patterns, and best practices for implementing batch operations.

## Table of Contents

1. [Architecture](#architecture)
2. [Core Components](#core-components)
3. [Batch Execution Modes](#batch-execution-modes)
4. [Bounded Batch Authorization Verification](#bounded-batch-authorization-verification)
5. [Usage Guide](#usage-guide)
6. [Performance Optimization](#performance-optimization)
7. [Error Handling](#error-handling)
8. [Best Practices](#best-practices)
9. [Integration Examples](#integration-examples)

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
- **Bounded Authorization Verification**: Capped batch verification of repeated authorization checks with per-item failure reporting

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

## Bounded Batch Authorization Verification

Repeated authorization checks (e.g. verifying many callers against the same operation) can be batched to reduce per-check overhead. The bounded batch verifier guarantees that batching never weakens the security properties of an individual check.

### Guarantees

1. **Explicit batch cap** — the batch size is capped at `MAX_AUTH_BATCH_SIZE`. Oversized batches are rejected deterministically with `BatchAuthError::BatchTooLarge`; they are never silently truncated.
2. **Item binding** — every item carries the `operation` and `caller` it was authorized for. Verification re-checks the binding, so an item cannot be replayed against a different operation or caller.
3. **No silent skips** — each item produces an explicit per-item result. Invalid items appear in `failed` with a reason; they are never dropped from the report.
4. **Atomicity preserved** — under `AllOrNothing`, any per-item failure rolls back the whole batch; under `BestEffort`, per-item failures are reported individually.

### Types

```rust
pub const MAX_AUTH_BATCH_SIZE: u32 = 64;

pub struct AuthCheckItem {
    pub operation: Symbol,
    pub caller: Address,
    pub payload_hash: BytesN<32>,
}

pub enum BatchAuthError {
    BatchTooLarge,
    EmptyBatch,
    OperationMismatch,
    CallerMismatch,
    Unauthorized,
}

pub struct AuthCheckOutcome {
    pub index: u32,
    pub result: Result<(), BatchAuthError>,
}

pub struct BatchAuthResult {
    pub outcomes: Vec<AuthCheckOutcome>,
    pub success_count: u32,
    pub failure_count: u32,
}
```

### Verifier

```rust
pub fn verify_batch_auth(
    env: &Env,
    expected_operation: Symbol,
    expected_caller: Address,
    items: Vec<AuthCheckItem>,
    mode: BatchMode,
) -> Result<BatchAuthResult, BatchAuthError>
```

**Behavior:**
- Rejects `items.len() > MAX_AUTH_BATCH_SIZE` with `BatchAuthError::BatchTooLarge`.
- Rejects an empty batch with `BatchAuthError::EmptyBatch`.
- For each item, checks `item.operation == expected_operation` and `item.caller == expected_caller`; mismatches yield `OperationMismatch` / `CallerMismatch` for that index.
- Runs the underlying authorization check; failures yield `Unauthorized` for that index.
- Under `AllOrNothing`, the first failure aborts the batch and the caller rolls back; under `BestEffort`, all outcomes are returned.

### Example

```rust
let items = build_auth_items(&env, &callers, operation.clone());

let result = verify_batch_auth(
    &env,
    operation.clone(),
    caller.clone(),
    items,
    BatchMode::BestEffort,
)?;

for outcome in result.outcomes.iter() {
    match outcome.result {
        Ok(()) => log_success(outcome.index),
        Err(reason) => log_failure(outcome.index, reason),
    }
}
```

### Benchmarks

Benchmarks compare batch verification against single-item verification to quantify the resource savings:

- `bench_single_auth_check`: verifies one item at a time.
- `bench_batch_auth_check`: verifies the same items through `verify_batch_auth`.
- Reported metrics: CPU instructions and memory bytes per item, plus the batch/single ratio.

Run with:

```
cargo bench -p batch_processor --bench auth_batch
```

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
