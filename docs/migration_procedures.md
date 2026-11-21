# State Migration Procedures

## Overview

This document provides detailed, step-by-step procedures for executing state migrations in the StellarSwipe protocol. Follow these procedures carefully to ensure safe and successful contract upgrades.

## Table of Contents

1. [Pre-Migration Checklist](#pre-migration-checklist)
2. [Standard Migration Procedure](#standard-migration-procedure)
3. [Emergency Rollback Procedure](#emergency-rollback-procedure)
4. [Post-Migration Verification](#post-migration-verification)
5. [Migration Templates](#migration-templates)
6. [Troubleshooting Guide](#troubleshooting-guide)

---

## Pre-Migration Checklist

### Planning Phase

- [ ] Define migration objectives
- [ ] Identify affected data structures
- [ ] Document schema changes
- [ ] Create migration plan
- [ ] Review with team
- [ ] Get stakeholder approval

### Technical Preparation

- [ ] Check version compatibility
- [ ] Review breaking changes
- [ ] Estimate gas costs
- [ ] Plan rollback strategy
- [ ] Prepare monitoring tools
- [ ] Set up logging

### Testing Phase

- [ ] Test on local environment
- [ ] Deploy to testnet
- [ ] Execute test migration
- [ ] Verify test results
- [ ] Benchmark performance
- [ ] Document test findings

### Communication

- [ ] Notify users of maintenance window
- [ ] Inform team members
- [ ] Prepare status updates
- [ ] Set up communication channels

---

## Standard Migration Procedure

### Phase 1: Preparation (30 minutes)

#### Step 1.1: Verify Environment

```bash
# Check network status
soroban network status

# Verify contract deployment
soroban contract info --id CONTRACT_ID

# Check current version
soroban contract invoke \
  --id CONTRACT_ID \
  --fn get_current_version
```

#### Step 1.2: Create Backup

```rust
// Create state snapshot
let snapshot = MigrationRollbackManager::create_snapshot(
    &env,
    migration_id,
)?;

// Verify snapshot created
assert!(snapshot.snapshot_id == migration_id);
```

#### Step 1.3: Enable Maintenance Mode

```rust
// Set maintenance mode
set_maintenance_mode(&env, true);

// Notify users
emit_event(&env, Event::MaintenanceStarted);
```

### Phase 2: Pre-Migration Validation (15 minutes)

#### Step 2.1: Validate Current State

```rust
// Run pre-migration validation
let pre_report = MigrationDataValidator::validate_data_integrity(
    &env,
    migration_id,
)?;

// Check validation results
if pre_report.checks_failed > 0 {
    log_errors(&pre_report.errors);
    return Err(Error::ValidationFailed);
}
```

#### Step 2.2: Check Version Compatibility

```rust
let current_version = get_current_version(&env);
let target_version = plan.to_version;

// Verify compatibility
if !VersionCompatibilityChecker::is_compatible(current_version, target_version) {
    return Err(Error::IncompatibleVersion);
}

// Get compatibility score
let score = VersionCompatibilityChecker::get_compatibility_score(
    current_version,
    target_version,
);

log_info(&format!("Compatibility score: {}", score));
```

### Phase 3: Migration Execution (Variable)

#### Step 3.1: Initialize Migration

```rust
// Create migration metadata
let metadata = MigrationMetadata {
    migration_id,
    from_version: current_version,
    to_version: target_version,
    status: MigrationStatus::InProgress,
    started_at: env.ledger().timestamp(),
    completed_at: 0,
    initiator: initiator.clone(),
};

// Store metadata
store_migration_metadata(&env, &metadata);
```

#### Step 3.2: Execute Migration Plan

```rust
// Execute migration
let result = MigrationExecutor::execute_migration(
    &env,
    plan,
    initiator,
)?;

// Log progress
log_info(&format!(
    "Steps completed: {}/{}",
    result.steps_completed,
    result.steps_completed + result.steps_failed
));
```

#### Step 3.3: Handle Errors

```rust
if !result.success {
    log_error("Migration failed");
    
    // Log errors
    for error in result.errors.iter() {
        log_error(&format!(
            "Step {}: {}",
            error.step_id,
            error.error_message
        ));
    }
    
    // Initiate rollback if enabled
    if plan.rollback_enabled {
        log_info("Initiating rollback...");
        let rollback_result = MigrationRollbackManager::rollback(
            &env,
            migration_id,
        )?;
        
        log_info(&format!(
            "Rollback completed in {}ms",
            rollback_result.duration_ms
        ));
    }
    
    return Err(Error::MigrationFailed);
}
```

### Phase 4: Post-Migration Verification (20 minutes)

#### Step 4.1: Verify Migration

```rust
// Run verification
let verification_report = MigrationVerifier::verify_migration(
    &env,
    migration_id,
    target_version,
)?;

// Check verification results
if !verification_report.verified {
    log_error("Verification failed");
    
    for issue in verification_report.issues.iter() {
        log_error(&format!("Issue: {}", issue));
    }
    
    // Consider rollback
    if should_rollback(&verification_report) {
        MigrationRollbackManager::rollback(&env, migration_id)?;
    }
    
    return Err(Error::VerificationFailed);
}
```

#### Step 4.2: Performance Check

```rust
// Benchmark post-migration performance
let benchmark = MigrationPerformanceTester::benchmark_migration(
    &env,
    &plan,
);

log_info(&format!("Total gas used: {}", benchmark.total_gas));
log_info(&format!("Total time: {}ms", benchmark.total_time_ms));

// Verify performance acceptable
if benchmark.total_gas > MAX_GAS_THRESHOLD {
    log_warning("Gas usage exceeds threshold");
}
```

#### Step 4.3: Functional Testing

```bash
# Test critical functions
soroban contract invoke --fn test_function_1
soroban contract invoke --fn test_function_2
soroban contract invoke --fn test_function_3

# Verify outputs
```

### Phase 5: Completion (10 minutes)

#### Step 5.1: Update Status

```rust
// Update migration status
set_migration_status(&env, migration_id, MigrationStatus::Completed);

// Update metadata
update_migration_metadata(&env, migration_id, |metadata| {
    metadata.completed_at = env.ledger().timestamp();
    metadata.status = MigrationStatus::Completed;
});
```

#### Step 5.2: Disable Maintenance Mode

```rust
// Clear maintenance mode
set_maintenance_mode(&env, false);

// Notify users
emit_event(&env, Event::MaintenanceCompleted);
```

#### Step 5.3: Documentation

```rust
// Generate migration report
let report = generate_migration_report(&env, migration_id);

// Store report
store_report(&env, &report);

// Log summary
log_info(&format!(
    "Migration {} completed successfully",
    migration_id
));
```

---

## Emergency Rollback Procedure

### When to Initiate Emergency Rollback

- Critical functionality broken
- Data corruption detected
- Severe performance degradation
- Security vulnerability introduced
- User-impacting bugs discovered

### Emergency Rollback Steps

#### Step 1: Immediate Actions (5 minutes)

```rust
// 1. Enable maintenance mode immediately
set_maintenance_mode(&env, true);

// 2. Stop all ongoing operations
pause_all_operations(&env);

// 3. Notify team
send_emergency_alert("Migration rollback initiated");
```

#### Step 2: Execute Rollback (10 minutes)

```rust
// 1. Retrieve snapshot
let snapshot: MigrationSnapshot = env
    .storage()
    .instance()
    .get(&DataKey::Snapshot(migration_id))
    .expect("Snapshot must exist");

// 2. Execute rollback
let rollback_result = MigrationRollbackManager::rollback(
    &env,
    migration_id,
)?;

// 3. Verify rollback
let verified = MigrationRollbackManager::verify_rollback(
    &env,
    &snapshot,
);

if !verified {
    panic!("Rollback verification failed - manual intervention required");
}
```

#### Step 3: Verification (10 minutes)

```rust
// 1. Verify version restored
let current_version = get_current_version(&env);
assert_eq!(current_version, snapshot.version);

// 2. Verify data integrity
let integrity_report = MigrationDataValidator::validate_data_integrity(
    &env,
    migration_id,
)?;

assert_eq!(integrity_report.checks_failed, 0);

// 3. Test critical functions
test_critical_functions(&env)?;
```

#### Step 4: Recovery (15 minutes)

```rust
// 1. Update status
set_migration_status(&env, migration_id, MigrationStatus::RolledBack);

// 2. Clear maintenance mode
set_maintenance_mode(&env, false);

// 3. Notify stakeholders
send_notification("System restored to previous version");

// 4. Document incident
create_incident_report(&env, migration_id);
```

---

## Post-Migration Verification

### Verification Checklist

#### Data Integrity

- [ ] All records present
- [ ] No data corruption
- [ ] Relationships intact
- [ ] Indexes valid
- [ ] Constraints satisfied

#### Functionality

- [ ] Core functions working
- [ ] Edge cases handled
- [ ] Error handling correct
- [ ] Events emitted properly
- [ ] Access control enforced

#### Performance

- [ ] Response times acceptable
- [ ] Gas costs reasonable
- [ ] No memory leaks
- [ ] Efficient queries
- [ ] Scalability maintained

#### Security

- [ ] No new vulnerabilities
- [ ] Access controls intact
- [ ] Data privacy maintained
- [ ] Audit logs working
- [ ] Encryption functional

---

## Migration Templates

### Template 1: Simple Version Upgrade

```rust
pub fn execute_simple_upgrade(
    env: Env,
    initiator: Address,
) -> Result<MigrationResult, Error> {
    let plan = MigrationPlan {
        plan_id: 1,
        from_version: 1,
        to_version: 2,
        steps: vec![
            MigrationStep {
                step_id: 1,
                step_type: StepType::UpdateSchema,
                description: String::from_str(&env, "Update version"),
                critical: false,
            },
        ],
        validation_rules: vec![],
        rollback_enabled: true,
    };
    
    MigrationExecutor::execute_migration(&env, plan, initiator)
}
```

### Template 2: Data Transformation

```rust
pub fn execute_data_transformation(
    env: Env,
    initiator: Address,
) -> Result<MigrationResult, Error> {
    let plan = MigrationPlan {
        plan_id: 2,
        from_version: 2,
        to_version: 3,
        steps: vec![
            MigrationStep {
                step_id: 1,
                step_type: StepType::TransformData,
                description: String::from_str(&env, "Transform user data"),
                critical: true,
            },
            MigrationStep {
                step_id: 2,
                step_type: StepType::UpdateSchema,
                description: String::from_str(&env, "Update schema"),
                critical: true,
            },
        ],
        validation_rules: vec![
            ValidationRule {
                rule_id: 1,
                rule_type: ValidationType::DataIntegrity,
                description: String::from_str(&env, "Verify transformation"),
                required: true,
            },
        ],
        rollback_enabled: true,
    };
    
    MigrationExecutor::execute_migration(&env, plan, initiator)
}
```

### Template 3: Multi-Step Migration

```rust
pub fn execute_complex_migration(
    env: Env,
    initiator: Address,
) -> Result<MigrationResult, Error> {
    let plan = MigrationPlan {
        plan_id: 3,
        from_version: 3,
        to_version: 4,
        steps: vec![
            MigrationStep {
                step_id: 1,
                step_type: StepType::AddField,
                description: String::from_str(&env, "Add new fields"),
                critical: false,
            },
            MigrationStep {
                step_id: 2,
                step_type: StepType::TransformData,
                description: String::from_str(&env, "Populate new fields"),
                critical: true,
            },
            MigrationStep {
                step_id: 3,
                step_type: StepType::RemoveField,
                description: String::from_str(&env, "Remove deprecated fields"),
                critical: false,
            },
            MigrationStep {
                step_id: 4,
                step_type: StepType::UpdateSchema,
                description: String::from_str(&env, "Finalize schema"),
                critical: true,
            },
        ],
        validation_rules: vec![
            ValidationRule {
                rule_id: 1,
                rule_type: ValidationType::DataIntegrity,
                description: String::from_str(&env, "Verify data complete"),
                required: true,
            },
            ValidationRule {
                rule_id: 2,
                rule_type: ValidationType::SchemaCompatibility,
                description: String::from_str(&env, "Verify schema valid"),
                required: true,
            },
        ],
        rollback_enabled: true,
    };
    
    MigrationExecutor::execute_migration(&env, plan, initiator)
}
```

---

## Troubleshooting Guide

### Common Issues and Solutions

**Issue: Migration Timeout**
- Cause: Migration taking too long
- Solution: Break into smaller steps or increase timeout
- Prevention: Benchmark before production

**Issue: Out of Gas**
- Cause: Migration exceeds gas limit
- Solution: Optimize steps or split migration
- Prevention: Test gas usage on testnet

**Issue: Data Validation Failed**
- Cause: Data doesn't meet validation rules
- Solution: Fix data issues before retry
- Prevention: Validate data before migration

**Issue: Rollback Failed**
- Cause: Snapshot corrupted or missing
- Solution: Manual state restoration
- Prevention: Verify snapshot creation

### Debug Commands

```bash
# Check migration status
soroban contract invoke --fn get_migration_status --arg MIGRATION_ID

# Get validation report
soroban contract invoke --fn get_validation_report --arg MIGRATION_ID

# Verify current version
soroban contract invoke --fn get_current_version

# Check snapshot exists
soroban contract invoke --fn has_snapshot --arg MIGRATION_ID
```

---

## Conclusion

Following these procedures ensures safe, reliable state migrations. Always:

1. Test thoroughly on testnet
2. Create snapshots before migration
3. Monitor progress closely
4. Verify results comprehensively
5. Document everything
6. Have rollback plan ready

For additional support, consult the [State Migration Guide](./state_migration_guide.md).
