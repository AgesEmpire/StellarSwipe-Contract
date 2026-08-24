# State Migration Framework Guide

## Overview

The StellarSwipe State Migration Framework provides a robust, production-ready system for managing contract upgrades with full backward compatibility, data validation, and rollback capabilities. This guide covers everything you need to know to safely migrate contract state across versions.

## Table of Contents

1. [Architecture](#architecture)
2. [Core Components](#core-components)
3. [Migration Planning](#migration-planning)
4. [Version Compatibility](#version-compatibility)
5. [Data Validation](#data-validation)
6. [Rollback Mechanisms](#rollback-mechanisms)
7. [Migration Verification](#migration-verification)
8. [Performance Testing](#performance-testing)
9. [Best Practices](#best-practices)
10. [Migration Procedures](#migration-procedures)
11. [Troubleshooting](#troubleshooting)

---

## Architecture

### System Design

```
┌─────────────────────────────────────────────────────────────┐
│              State Migration Framework                       │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │   Version    │  │     Data     │  │   Rollback   │      │
│  │ Compatibility│  │  Validation  │  │   Manager    │      │
│  │   Checker    │  │              │  │              │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│                                                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │  Migration   │  │  Migration   │  │ Performance  │      │
│  │   Executor   │  │   Verifier   │  │    Tester    │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐    │
│  │         Migration Abstraction Layer                 │    │
│  │  • Plans  • Steps  • Metadata  • Results          │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                               │
└─────────────────────────────────────────────────────────────┘
```

### Key Features

- **Version Compatibility Checking**: Automatic validation of migration paths
- **Data Validation**: Comprehensive integrity checks during migration
- **Rollback Support**: Snapshot-based rollback for failed migrations
- **Migration Verification**: Post-migration validation and testing
- **Performance Testing**: Benchmarking and load testing capabilities
- **Backward Compatibility**: Support for gradual upgrades

---

## Core Components

### 1. Migration Abstraction Layer

The foundation of the framework, providing structured migration management.

#### MigrationPlan

Defines the complete migration strategy:

```rust
pub struct MigrationPlan {
    pub plan_id: u64,
    pub from_version: MigrationVersion,
    pub to_version: MigrationVersion,
    pub steps: Vec<MigrationStep>,
    pub validation_rules: Vec<ValidationRule>,
    pub rollback_enabled: bool,
}
```

**Usage**:
```rust
let plan = MigrationPlan {
    plan_id: 1,
    from_version: 1,
    to_version: 2,
    steps: vec![
        MigrationStep {
            step_id: 1,
            step_type: StepType::AddField,
            description: String::from_str(&env, "Add new field"),
            critical: true,
        },
    ],
    validation_rules: vec![],
    rollback_enabled: true,
};
```

#### MigrationStep

Individual migration operations:

```rust
pub struct MigrationStep {
    pub step_id: u32,
    pub step_type: StepType,
    pub description: String,
    pub critical: bool,
}
```

**Step Types**:
- `AddField`: Add new field to schema
- `RemoveField`: Remove field from schema
- `RenameField`: Rename existing field
- `TransformData`: Transform data format
- `UpdateSchema`: Update schema definition
- `MigrateStorage`: Migrate storage format

#### MigrationMetadata

Tracks migration execution:

```rust
pub struct MigrationMetadata {
    pub migration_id: u64,
    pub from_version: MigrationVersion,
    pub to_version: MigrationVersion,
    pub status: MigrationStatus,
    pub started_at: u64,
    pub completed_at: u64,
    pub initiator: Address,
}
```

**Status Values**:
- `NotStarted`: Migration not yet begun
- `InProgress`: Currently executing
- `Completed`: Successfully finished
- `Failed`: Encountered error
- `RolledBack`: Reverted to previous state


### 2. Version Compatibility Checker

Ensures safe migration paths between versions.

#### Key Methods

**is_compatible()**
```rust
pub fn is_compatible(
    from_version: MigrationVersion,
    to_version: MigrationVersion,
) -> bool
```

Checks if migration is possible:
- Cannot downgrade (to_version < from_version)
- Cannot skip more than 5 versions
- No breaking changes in path

**has_breaking_changes()**
```rust
pub fn has_breaking_changes(
    from_version: MigrationVersion,
    to_version: MigrationVersion,
) -> bool
```

Identifies breaking changes between versions.

**get_migration_path()**
```rust
pub fn get_migration_path(
    from_version: MigrationVersion,
    to_version: MigrationVersion,
) -> Vec<MigrationVersion>
```

Returns sequential migration path.

**get_compatibility_score()**
```rust
pub fn get_compatibility_score(
    from_version: MigrationVersion,
    to_version: MigrationVersion,
) -> u32
```

Returns compatibility score (0-100):
- 100: Perfect compatibility
- 70-99: Good compatibility
- 40-69: Moderate compatibility
- 0-39: Poor compatibility

#### Example Usage

```rust
// Check if migration is possible
if VersionCompatibilityChecker::is_compatible(1, 3) {
    // Get migration path
    let path = VersionCompatibilityChecker::get_migration_path(1, 3);
    // path = [2, 3]
    
    // Check compatibility score
    let score = VersionCompatibilityChecker::get_compatibility_score(1, 3);
    // score = 80 (good compatibility)
}
```

### 3. Data Validation System

Ensures data integrity throughout migration.

#### MigrationDataValidator

**validate_data_integrity()**
```rust
pub fn validate_data_integrity(
    env: &Env,
    migration_id: u64,
) -> Result<ValidationReport, MigrationError>
```

Performs comprehensive validation:
1. Data consistency checks
2. Referential integrity verification
3. Schema compatibility validation

**validate_business_logic()**
```rust
pub fn validate_business_logic(
    env: &Env,
    data: &StateData,
) -> Result<(), MigrationError>
```

Validates business rules and constraints.

**validate_data_ranges()**
```rust
pub fn validate_data_ranges(
    env: &Env,
    data: &StateData,
) -> Result<(), MigrationError>
```

Checks data values are within acceptable ranges.

#### ValidationReport

```rust
pub struct ValidationReport {
    pub validation_id: u64,
    pub checks_passed: u32,
    pub checks_failed: u32,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}
```

#### Example Usage

```rust
// Validate data integrity
let report = MigrationDataValidator::validate_data_integrity(
    &env,
    migration_id,
)?;

if report.checks_failed > 0 {
    // Handle validation failures
    for error in report.errors.iter() {
        log_error(&error);
    }
}
```

### 4. Rollback Manager

Provides snapshot-based rollback capabilities.

#### MigrationRollbackManager

**create_snapshot()**
```rust
pub fn create_snapshot(
    env: &Env,
    migration_id: u64,
) -> Result<MigrationSnapshot, MigrationError>
```

Creates state snapshot before migration.

**rollback()**
```rust
pub fn rollback(
    env: &Env,
    migration_id: u64,
) -> Result<RollbackResult, MigrationError>
```

Restores state from snapshot.

**verify_rollback()**
```rust
pub fn verify_rollback(
    env: &Env,
    snapshot: &MigrationSnapshot,
) -> bool
```

Verifies rollback success.

#### MigrationSnapshot

```rust
pub struct MigrationSnapshot {
    pub snapshot_id: u64,
    pub version: MigrationVersion,
    pub timestamp: u64,
    pub state_data: Vec<StateData>,
    pub metadata: Vec<String>,
}
```

#### Example Usage

```rust
// Create snapshot before migration
let snapshot = MigrationRollbackManager::create_snapshot(
    &env,
    migration_id,
)?;

// Execute migration
let result = execute_migration(&env, plan);

if !result.success {
    // Rollback on failure
    let rollback_result = MigrationRollbackManager::rollback(
        &env,
        migration_id,
    )?;
    
    // Verify rollback
    if MigrationRollbackManager::verify_rollback(&env, &snapshot) {
        println!("Rollback successful");
    }
}
```


### 5. Migration Verifier

Post-migration validation and verification.

#### MigrationVerifier

**verify_migration()**
```rust
pub fn verify_migration(
    env: &Env,
    migration_id: u64,
    expected_version: MigrationVersion,
) -> Result<VerificationReport, MigrationError>
```

Comprehensive post-migration verification:
1. Version verification
2. Data integrity checks
3. Schema validation
4. Business logic verification

**verify_performance()**
```rust
pub fn verify_performance(
    env: &Env,
    expected_performance: PerformanceMetrics,
) -> VerificationCheck
```

Validates performance meets expectations.

#### VerificationReport

```rust
pub struct VerificationReport {
    pub migration_id: u64,
    pub verified: bool,
    pub checks: Vec<VerificationCheck>,
    pub issues: Vec<String>,
}
```

#### Example Usage

```rust
// Verify migration completion
let report = MigrationVerifier::verify_migration(
    &env,
    migration_id,
    expected_version,
)?;

if report.verified {
    println!("Migration verified successfully");
} else {
    println!("Verification failed:");
    for issue in report.issues.iter() {
        println!("  - {}", issue);
    }
}
```

### 6. Performance Tester

Benchmarking and load testing for migrations.

#### MigrationPerformanceTester

**benchmark_migration()**
```rust
pub fn benchmark_migration(
    env: &Env,
    migration_plan: &MigrationPlan,
) -> MigrationBenchmark
```

Measures migration performance.

**load_test_migration()**
```rust
pub fn load_test_migration(
    env: &Env,
    migration_plan: &MigrationPlan,
    data_size: u32,
) -> LoadTestResult
```

Tests migration under varying loads.

**compare_strategies()**
```rust
pub fn compare_strategies(
    env: &Env,
    strategies: Vec<MigrationPlan>,
) -> Vec<MigrationBenchmark>
```

Compares different migration approaches.

#### MigrationBenchmark

```rust
pub struct MigrationBenchmark {
    pub plan_id: u64,
    pub total_time_ms: u64,
    pub total_gas: u64,
    pub steps_executed: u32,
    pub avg_time_per_step: u64,
    pub avg_gas_per_step: u64,
}
```

#### Example Usage

```rust
// Benchmark migration
let benchmark = MigrationPerformanceTester::benchmark_migration(
    &env,
    &plan,
);

println!("Total time: {}ms", benchmark.total_time_ms);
println!("Total gas: {}", benchmark.total_gas);
println!("Avg time per step: {}ms", benchmark.avg_time_per_step);

// Load test
let load_result = MigrationPerformanceTester::load_test_migration(
    &env,
    &plan,
    100, // Test with 100 iterations
);

println!("Load test completed in {}ms", load_result.total_duration_ms);
```

---

## Migration Planning

### Creating a Migration Plan

#### Step 1: Define Version Transition

```rust
let from_version = 1;
let to_version = 2;

// Check compatibility
if !VersionCompatibilityChecker::is_compatible(from_version, to_version) {
    panic!("Incompatible versions");
}
```

#### Step 2: Define Migration Steps

```rust
let steps = vec![
    MigrationStep {
        step_id: 1,
        step_type: StepType::AddField,
        description: String::from_str(&env, "Add user_tier field"),
        critical: true,
    },
    MigrationStep {
        step_id: 2,
        step_type: StepType::TransformData,
        description: String::from_str(&env, "Calculate tier for existing users"),
        critical: true,
    },
    MigrationStep {
        step_id: 3,
        step_type: StepType::UpdateSchema,
        description: String::from_str(&env, "Update schema version"),
        critical: false,
    },
];
```

#### Step 3: Define Validation Rules

```rust
let validation_rules = vec![
    ValidationRule {
        rule_id: 1,
        rule_type: ValidationType::DataIntegrity,
        description: String::from_str(&env, "Verify all users have tier"),
        required: true,
    },
    ValidationRule {
        rule_id: 2,
        rule_type: ValidationType::BusinessLogic,
        description: String::from_str(&env, "Verify tier values are valid"),
        required: true,
    },
];
```

#### Step 4: Create Migration Plan

```rust
let plan = MigrationPlan {
    plan_id: get_next_plan_id(&env),
    from_version,
    to_version,
    steps,
    validation_rules,
    rollback_enabled: true,
};
```

### Migration Plan Templates

#### Template 1: Simple Field Addition

```rust
fn create_add_field_plan(env: &Env) -> MigrationPlan {
    MigrationPlan {
        plan_id: 1,
        from_version: 1,
        to_version: 2,
        steps: vec![
            MigrationStep {
                step_id: 1,
                step_type: StepType::AddField,
                description: String::from_str(env, "Add new field"),
                critical: false,
            },
        ],
        validation_rules: vec![],
        rollback_enabled: true,
    }
}
```

#### Template 2: Data Transformation

```rust
fn create_data_transform_plan(env: &Env) -> MigrationPlan {
    MigrationPlan {
        plan_id: 2,
        from_version: 2,
        to_version: 3,
        steps: vec![
            MigrationStep {
                step_id: 1,
                step_type: StepType::TransformData,
                description: String::from_str(env, "Transform data format"),
                critical: true,
            },
            MigrationStep {
                step_id: 2,
                step_type: StepType::UpdateSchema,
                description: String::from_str(env, "Update schema"),
                critical: true,
            },
        ],
        validation_rules: vec![
            ValidationRule {
                rule_id: 1,
                rule_type: ValidationType::DataIntegrity,
                description: String::from_str(env, "Verify transformation"),
                required: true,
            },
        ],
        rollback_enabled: true,
    }
}
```

#### Template 3: Storage Migration

```rust
fn create_storage_migration_plan(env: &Env) -> MigrationPlan {
    MigrationPlan {
        plan_id: 3,
        from_version: 3,
        to_version: 4,
        steps: vec![
            MigrationStep {
                step_id: 1,
                step_type: StepType::MigrateStorage,
                description: String::from_str(env, "Migrate to new storage"),
                critical: true,
            },
        ],
        validation_rules: vec![
            ValidationRule {
                rule_id: 1,
                rule_type: ValidationType::DataIntegrity,
                description: String::from_str(env, "Verify data migrated"),
                required: true,
            },
            ValidationRule {
                rule_id: 2,
                rule_type: ValidationType::PerformanceCheck,
                description: String::from_str(env, "Verify performance"),
                required: false,
            },
        ],
        rollback_enabled: true,
    }
}
```


---

## Version Compatibility

### Compatibility Rules

1. **No Downgrades**: Cannot migrate to lower version
2. **Version Gap Limit**: Maximum 5 versions per migration
3. **Breaking Changes**: Identified at versions 3, 7, 10
4. **Sequential Path**: Must follow version sequence

### Compatibility Matrix

| From → To | Compatible | Score | Notes |
|-----------|-----------|-------|-------|
| 1 → 2 | ✅ | 90 | Simple upgrade |
| 1 → 3 | ✅ | 80 | Includes breaking change |
| 1 → 6 | ❌ | 0 | Gap too large |
| 5 → 3 | ❌ | 0 | Downgrade not allowed |
| 2 → 4 | ✅ | 70 | Crosses breaking version |

---

## Data Validation

### Validation Types

1. **Data Integrity**: Consistency and completeness
2. **Schema Compatibility**: Structure validation
3. **Referential Integrity**: Relationship validation
4. **Business Logic**: Rule compliance
5. **Performance Check**: Efficiency validation

### Validation Workflow

```rust
// 1. Pre-migration validation
let pre_report = MigrationDataValidator::validate_data_integrity(&env, migration_id)?;

// 2. Execute migration
let result = MigrationExecutor::execute_migration(&env, plan, initiator)?;

// 3. Post-migration validation
let post_report = MigrationVerifier::verify_migration(&env, migration_id, to_version)?;
```

---

## Rollback Mechanisms

### When to Use Rollback

- Critical step failure
- Validation failure
- Data corruption detected
- Performance degradation
- User-initiated abort

### Rollback Process

```rust
// 1. Create snapshot
let snapshot = MigrationRollbackManager::create_snapshot(&env, migration_id)?;

// 2. Attempt migration
match MigrationExecutor::execute_migration(&env, plan, initiator) {
    Ok(result) if !result.success => {
        // 3. Rollback on failure
        MigrationRollbackManager::rollback(&env, migration_id)?;
    },
    Err(_) => {
        // 4. Rollback on error
        MigrationRollbackManager::rollback(&env, migration_id)?;
    },
    Ok(_) => {
        // Success - no rollback needed
    }
}
```

---

## Migration Verification

### Verification Checklist

- [ ] Version updated correctly
- [ ] Data integrity maintained
- [ ] Schema valid
- [ ] Business logic satisfied
- [ ] Performance acceptable
- [ ] No data loss
- [ ] Backward compatibility preserved

### Verification Example

```rust
let report = MigrationVerifier::verify_migration(&env, migration_id, 2)?;

for check in report.checks.iter() {
    println!("{}: {}", check.check_name, if check.passed { "✓" } else { "✗" });
}
```

---

## Performance Testing

### Benchmark Metrics

- Total execution time
- Gas consumption
- Steps executed
- Average time per step
- Average gas per step

### Load Testing

```rust
let load_result = MigrationPerformanceTester::load_test_migration(
    &env,
    &plan,
    1000, // 1000 iterations
);

println!("Avg time per iteration: {}ms", load_result.avg_time_per_iteration);
```

---

## Best Practices

### 1. Always Enable Rollback

```rust
let plan = MigrationPlan {
    // ...
    rollback_enabled: true, // Always enable for production
};
```

### 2. Mark Critical Steps

```rust
MigrationStep {
    step_id: 1,
    step_type: StepType::TransformData,
    description: String::from_str(&env, "Critical transformation"),
    critical: true, // Failure triggers rollback
}
```

### 3. Test on Testnet First

```bash
# Deploy to testnet
soroban contract deploy --network testnet

# Run migration
soroban contract invoke --network testnet --fn execute_migration

# Verify results
soroban contract invoke --network testnet --fn verify_migration
```

### 4. Monitor Performance

```rust
let benchmark = MigrationPerformanceTester::benchmark_migration(&env, &plan);

if benchmark.total_gas > MAX_GAS_LIMIT {
    panic!("Migration exceeds gas limit");
}
```

### 5. Validate Before and After

```rust
// Pre-migration validation
let pre_report = validate_data_integrity(&env, migration_id)?;
assert_eq!(pre_report.checks_failed, 0);

// Execute migration
execute_migration(&env, plan, initiator)?;

// Post-migration validation
let post_report = verify_migration(&env, migration_id, to_version)?;
assert!(post_report.verified);
```

---

## Migration Procedures

### Standard Migration Procedure

1. **Preparation**
   - Review migration plan
   - Check version compatibility
   - Backup current state
   - Test on testnet

2. **Pre-Migration**
   - Create snapshot
   - Validate current state
   - Notify stakeholders
   - Set maintenance mode

3. **Execution**
   - Execute migration plan
   - Monitor progress
   - Log all steps
   - Handle errors

4. **Verification**
   - Verify version
   - Validate data
   - Check performance
   - Test functionality

5. **Completion**
   - Clear maintenance mode
   - Notify stakeholders
   - Document results
   - Archive snapshot

### Emergency Rollback Procedure

1. **Detection**
   - Identify failure
   - Assess impact
   - Decide on rollback

2. **Execution**
   - Initiate rollback
   - Monitor progress
   - Verify restoration

3. **Recovery**
   - Analyze failure
   - Fix issues
   - Plan retry

---

## Troubleshooting

### Common Issues

**Issue: Incompatible Version**
```
Error: IncompatibleVersion
Solution: Check version gap and breaking changes
```

**Issue: Validation Failed**
```
Error: ValidationFailed
Solution: Review validation report and fix data issues
```

**Issue: Snapshot Not Found**
```
Error: SnapshotNotFound
Solution: Ensure snapshot was created before migration
```

**Issue: Rollback Failed**
```
Error: RollbackFailed
Solution: Manual state restoration may be required
```

### Debug Commands

```rust
// Check current version
let version = get_current_version(&env);

// Check migration status
let status = get_migration_status(&env, migration_id);

// Get validation report
let report = validate_data_integrity(&env, migration_id)?;
```

---

## Conclusion

The State Migration Framework provides enterprise-grade migration capabilities with:

- ✅ Robust version compatibility checking
- ✅ Comprehensive data validation
- ✅ Reliable rollback mechanisms
- ✅ Thorough verification processes
- ✅ Performance testing tools
- ✅ Production-ready procedures

Follow the best practices and procedures outlined in this guide to ensure safe, reliable contract upgrades.

For more information, see:
- [Migration Procedures Document](./migration_procedures.md)
- [Architecture Documentation](./ARCHITECTURE.md)
- [Troubleshooting Guide](./TROUBLESHOOTING.md)
