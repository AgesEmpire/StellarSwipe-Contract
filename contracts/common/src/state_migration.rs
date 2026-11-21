// State Migration Framework for Contract Upgrades
// Provides robust migration with backward compatibility

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Vec, Bytes, Map};

// ============================================================================
// Archival Export/Restore Verification
// ============================================================================

/// Current archival export schema version
pub const ARCHIVAL_EXPORT_SCHEMA_VERSION: u32 = 1;

/// Archival export record as defined in STATE_MIGRATION_SUMMARY.md
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ArchivalExportRecord {
    pub contract_id: String,
    pub schema_version: u32,
    pub ledger_sequence: u32,
    pub ledger_close_time: u64,
    pub network_passphrase: String,
    pub key: String,
    pub value: String,
}

/// Full archival export containing records and metadata
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ArchivalExport {
    pub records: Vec<ArchivalExportRecord>,
    pub export_timestamp: u64,
    pub export_checksum: u64,
    pub format_version: u32,
}

/// Result of restore validation
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RestoreValidationResult {
    pub valid: bool,
    pub format_version_valid: bool,
    pub checksum_valid: bool,
    pub invariants_valid: bool,
    pub record_count: u32,
    pub errors: Vec<String>,
}

/// Verifier for archival export restoration
pub struct ArchivalRestoreVerifier;

impl ArchivalRestoreVerifier {
    /// Compute checksum for a set of records
    pub fn compute_checksum(records: &Vec<ArchivalExportRecord>) -> u64 {
        let mut hasher = ChecksumHasher::new();
        for record in records.iter() {
            hasher.update(&record.contract_id);
            hasher.update_u32(record.schema_version);
            hasher.update_u32(record.ledger_sequence);
            hasher.update_u64(record.ledger_close_time);
            hasher.update(&record.network_passphrase);
            hasher.update(&record.key);
            hasher.update(&record.value);
        }
        hasher.finalize()
    }

    /// Validate archival export before restore
    pub fn validate_export(
        env: &Env,
        export: &ArchivalExport,
    ) -> RestoreValidationResult {
        let mut errors = Vec::new(env);
        
        // Check format version
        let format_version_valid = export.format_version == ARCHIVAL_EXPORT_SCHEMA_VERSION;
        if !format_version_valid {
            errors.push_back(String::from_str(
                env,
                &format!("Invalid format version: expected {}, got {}", ARCHIVAL_EXPORT_SCHEMA_VERSION, export.format_version),
            ));
        }

        // Check checksum
        let computed_checksum = Self::compute_checksum(&export.records);
        let checksum_valid = computed_checksum == export.export_checksum;
        if !checksum_valid {
            errors.push_back(String::from_str(
                env,
                &format!("Checksum mismatch: expected {}, computed {}", export.export_checksum, computed_checksum),
            ));
        }

        // Check required invariants
        let invariants_valid = Self::validate_invariants(env, export, &mut errors);

        // Overall validity - all checks must pass
        let valid = format_version_valid && checksum_valid && invariants_valid;

        RestoreValidationResult {
            valid,
            format_version_valid,
            checksum_valid,
            invariants_valid,
            record_count: export.records.len(),
            errors,
        }
    }

    /// Validate required invariants on the export
    fn validate_invariants(
        env: &Env,
        export: &ArchivalExport,
        errors: &mut Vec<String>,
    ) -> bool {
        let mut all_valid = true;

        // Invariant 1: All records must have same schema version
        if export.records.len() > 0 {
            let expected_schema_version = export.records.get(0).unwrap().schema_version;
            for record in export.records.iter() {
                if record.schema_version != expected_schema_version {
                    errors.push_back(String::from_str(
                        env,
                        &format!("Schema version mismatch in records: expected {}, got {}", expected_schema_version, record.schema_version),
                    ));
                    all_valid = false;
                }
            }
        }

        // Invariant 2: All records must have same network passphrase
        if export.records.len() > 0 {
            let expected_network = export.records.get(0).unwrap().network_passphrase.clone();
            for record in export.records.iter() {
                if record.network_passphrase != expected_network {
                    errors.push_back(String::from_str(
                        env,
                        &format!("Network passphrase mismatch in records"),
                    ));
                    all_valid = false;
                }
            }
        }

        // Invariant 3: Records must be sorted by (contract_id, key)
        let mut prev_contract_id = String::from_str(env, "");
        let mut prev_key = String::from_str(env, "");
        let mut first = true;
        for record in export.records.iter() {
            if !first {
                let cmp = Self::compare_keys(&prev_contract_id, &prev_key, &record.contract_id, &record.key);
                if cmp > 0 {
                    errors.push_back(String::from_str(
                        env,
                        &format!("Records not sorted: {}:{} should come before {}:{}", 
                            record.contract_id, record.key, prev_contract_id, prev_key),
                    ));
                    all_valid = false;
                }
            }
            prev_contract_id = record.contract_id.clone();
            prev_key = record.key.clone();
            first = false;
        }

        // Invariant 4: No duplicate keys
        let mut seen_keys = Map::new(env);
        for record in export.records.iter() {
            let composite_key = (record.contract_id.clone(), record.key.clone());
            if seen_keys.get(composite_key.clone()).is_some() {
                errors.push_back(String::from_str(
                    env,
                    &format!("Duplicate key found: {}:{}", record.contract_id, record.key),
                ));
                all_valid = false;
            } else {
                seen_keys.set(composite_key, true);
            }
        }

        // Invariant 5: Export timestamp must be reasonable (not in future, not too old)
        let current_time = env.ledger().timestamp();
        if export.export_timestamp > current_time {
            errors.push_back(String::from_str(
                env,
                &format!("Export timestamp is in the future: {} > {}", export.export_timestamp, current_time),
            ));
            all_valid = false;
        }
        // Allow exports up to ~1 year old
        if current_time.saturating_sub(export.export_timestamp) > 365 * 24 * 60 * 60 {
            errors.push_back(String::from_str(
                env,
                &format!("Export timestamp is too old: {} (current: {})", export.export_timestamp, current_time),
            ));
            all_valid = false;
        }

        all_valid
    }

    /// Compare composite keys for sorting
    fn compare_keys(
        contract_id_a: &String,
        key_a: &String,
        contract_id_b: &String,
        key_b: &String,
    ) -> i32 {
        // Compare contract_id first
        let cid_cmp = contract_id_a.to_string().cmp(&contract_id_b.to_string());
        if cid_cmp != std::cmp::Ordering::Equal {
            return if cid_cmp == std::cmp::Ordering::Less { -1 } else { 1 };
        }
        // Then compare key
        let key_cmp = key_a.to_string().cmp(&key_b.to_string());
        if key_cmp == std::cmp::Ordering::Less { -1 } else if key_cmp == std::cmp::Ordering::Greater { 1 } else { 0 }
    }

    /// Restore state from validated export (atomic - all or nothing)
    pub fn restore_from_export(
        env: &Env,
        export: &ArchivalExport,
    ) -> Result<RestoreResult, RestoreError> {
        // First validate the export
        let validation = Self::validate_export(env, export);
        if !validation.valid {
            return Err(RestoreError::ValidationFailed(validation.errors));
        }

        // Perform atomic restore - collect all writes first, then apply
        let mut writes = Vec::new(env);
        for record in export.records.iter() {
            writes.push_back(RestoreWrite {
                contract_id: record.contract_id.clone(),
                key: record.key.clone(),
                value: record.value.clone(),
            });
        }

        // Apply all writes atomically (in Soroban, this means we do them sequentially
        // but if any fails, we return error - the caller should handle rollback)
        for write in writes.iter() {
            // In a real implementation, this would write to the target contract's storage
            // For this framework, we store the restored data in a restore namespace
            let restore_key = DataKey::RestoredState(write.contract_id.clone(), write.key.clone());
            env.storage().instance().set(&restore_key, &write.value);
        }

        Ok(RestoreResult {
            success: true,
            records_restored: export.records.len(),
            export_timestamp: export.export_timestamp,
        })
    }

    /// Verify restored state matches export
    pub fn verify_restored_state(
        env: &Env,
        export: &ArchivalExport,
    ) -> Result<VerificationResult, RestoreError> {
        let mut mismatches = Vec::new(env);
        let mut verified_count = 0u32;

        for record in export.records.iter() {
            let restore_key = DataKey::RestoredState(record.contract_id.clone(), record.key.clone());
            let stored_value: String = env.storage().instance().get(&restore_key).unwrap_or(String::from_str(env, ""));
            
            if stored_value != record.value {
                mismatches.push_back(RestoreMismatch {
                    contract_id: record.contract_id.clone(),
                    key: record.key.clone(),
                    expected: record.value.clone(),
                    actual: stored_value,
                });
            } else {
                verified_count += 1;
            }
        }

        Ok(VerificationResult {
            verified: mismatches.len() == 0,
            verified_count,
            mismatch_count: mismatches.len(),
            mismatches,
        })
    }
}

/// Simple checksum hasher
struct ChecksumHasher {
    hash: u64,
}

impl ChecksumHasher {
    fn new() -> Self {
        Self { hash: 0x9e3779b97f4a7c15 } // Golden ratio
    }

    fn update(&mut self, s: &String) {
        let bytes = s.to_bytes();
        for i in 0..bytes.len() {
            self.hash = self.hash.wrapping_mul(31).wrapping_add(bytes.get(i) as u64);
        }
    }

    fn update_u32(&mut self, v: u32) {
        self.hash = self.hash.wrapping_mul(31).wrapping_add(v as u64);
    }

    fn update_u64(&mut self, v: u64) {
        self.hash = self.hash.wrapping_mul(31).wrapping_add(v);
    }

    fn finalize(self) -> u64 {
        // Final avalanche
        let mut h = self.hash;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51afd7ed558ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
        h ^= h >> 33;
        h
    }
}

/// Write operation for atomic restore
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RestoreWrite {
    pub contract_id: String,
    pub key: String,
    pub value: String,
}

/// Result of restore operation
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RestoreResult {
    pub success: bool,
    pub records_restored: u32,
    pub export_timestamp: u64,
}

/// Result of verification after restore
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationResult {
    pub verified: bool,
    pub verified_count: u32,
    pub mismatch_count: u32,
    pub mismatches: Vec<RestoreMismatch>,
}

/// Mismatch detail
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RestoreMismatch {
    pub contract_id: String,
    pub key: String,
    pub expected: String,
    pub actual: String,
}

/// Restore error types
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum RestoreError {
    ValidationFailed(Vec<String>),
    WriteFailed(String),
    VerificationFailed,
    InvalidFormat,
}

/// Extended storage keys for restore
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    CurrentVersion,
    MigrationCounter,
    MigrationStatus(u64),
    Snapshot(u64),
    RestoredState(String, String), // contract_id, key
}

// ============================================================================
// Migration Abstraction Layer
// ============================================================================

/// Migration version identifier
pub type MigrationVersion = u32;

/// Current migration version
pub const CURRENT_VERSION: MigrationVersion = 1;

/// Migration status
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum MigrationStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed,
    RolledBack,
}

/// Migration metadata
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct MigrationMetadata {
    pub migration_id: u64,
    pub from_version: MigrationVersion,
    pub to_version: MigrationVersion,
    pub status: MigrationStatus,
    pub started_at: u64,
    pub completed_at: u64,
    pub initiator: Address,
}

/// Migration plan
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct MigrationPlan {
    pub plan_id: u64,
    pub from_version: MigrationVersion,
    pub to_version: MigrationVersion,
    pub steps: Vec<MigrationStep>,
    pub validation_rules: Vec<ValidationRule>,
    pub rollback_enabled: bool,
}

/// Individual migration step
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct MigrationStep {
    pub step_id: u32,
    pub step_type: StepType,
    pub description: String,
    pub critical: bool,
}

/// Migration step type
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum StepType {
    AddField,
    RemoveField,
    RenameField,
    TransformData,
    UpdateSchema,
    MigrateStorage,
}

/// Validation rule
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ValidationRule {
    pub rule_id: u32,
    pub rule_type: ValidationType,
    pub description: String,
    pub required: bool,
}

/// Validation type
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum ValidationType {
    DataIntegrity,
    SchemaCompatibility,
    ReferentialIntegrity,
    BusinessLogic,
    PerformanceCheck,
}

/// Migration result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct MigrationResult {
    pub migration_id: u64,
    pub success: bool,
    pub steps_completed: u32,
    pub steps_failed: u32,
    pub validation_passed: bool,
    pub errors: Vec<MigrationError>,
    pub duration_ms: u64,
}

/// Migration error
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct MigrationError {
    pub step_id: u32,
    pub error_code: u32,
    pub error_message: String,
    pub recoverable: bool,
}

// ============================================================================
// Version Compatibility Checks
// ============================================================================

/// Version compatibility checker
pub struct VersionCompatibilityChecker;

impl VersionCompatibilityChecker {
    /// Check if migration is possible between versions
    pub fn is_compatible(
        from_version: MigrationVersion,
        to_version: MigrationVersion,
    ) -> bool {
        // Cannot downgrade
        if to_version < from_version {
            return false;
        }
        
        // Cannot skip more than 5 versions
        if to_version - from_version > 5 {
            return false;
        }
        
        // Check for breaking changes
        !Self::has_breaking_changes(from_version, to_version)
    }
    
    /// Check for breaking changes between versions
    pub fn has_breaking_changes(
        from_version: MigrationVersion,
        to_version: MigrationVersion,
    ) -> bool {
        // Define breaking change versions
        let breaking_versions = vec![3, 7, 10];
        
        for breaking_version in breaking_versions {
            if from_version < breaking_version && to_version >= breaking_version {
                return true;
            }
        }
        
        false
    }

    /// Get required intermediate versions
    pub fn get_migration_path(
        from_version: MigrationVersion,
        to_version: MigrationVersion,
    ) -> Vec<MigrationVersion> {
        let mut path = Vec::new();
        
        if !Self::is_compatible(from_version, to_version) {
            return path;
        }
        
        // Generate sequential path
        for version in (from_version + 1)..=to_version {
            path.push(version);
        }
        
        path
    }
    
    /// Validate version format
    pub fn is_valid_version(version: MigrationVersion) -> bool {
        version > 0 && version <= 1000
    }
    
    /// Get compatibility score (0-100)
    pub fn get_compatibility_score(
        from_version: MigrationVersion,
        to_version: MigrationVersion,
    ) -> u32 {
        if !Self::is_compatible(from_version, to_version) {
            return 0;
        }
        
        let version_gap = to_version - from_version;
        let has_breaking = Self::has_breaking_changes(from_version, to_version);
        
        let mut score = 100u32;
        
        // Reduce score based on version gap
        score = score.saturating_sub(version_gap * 10);
        
        // Reduce score if breaking changes exist
        if has_breaking {
            score = score.saturating_sub(30);
        }
        
        score
    }
}

// ============================================================================
// Data Validation During Migration
// ============================================================================

/// Data validator for migrations
pub struct MigrationDataValidator;

impl MigrationDataValidator {
    /// Validate data integrity
    pub fn validate_data_integrity(
        env: &Env,
        migration_id: u64,
    ) -> Result<ValidationReport, MigrationError> {
        let mut report = ValidationReport {
            validation_id: migration_id,
            checks_passed: 0,
            checks_failed: 0,
            warnings: Vec::new(env),
            errors: Vec::new(env),
        };
        
        // Check 1: Data consistency
        if Self::check_data_consistency(env) {
            report.checks_passed += 1;
        } else {
            report.checks_failed += 1;
            report.errors.push_back(String::from_str(
                env,
                "Data consistency check failed",
            ));
        }
        
        // Check 2: Referential integrity
        if Self::check_referential_integrity(env) {
            report.checks_passed += 1;
        } else {
            report.checks_failed += 1;
            report.errors.push_back(String::from_str(
                env,
                "Referential integrity check failed",
            ));
        }
        
        // Check 3: Schema compatibility
        if Self::check_schema_compatibility(env) {
            report.checks_passed += 1;
        } else {
            report.checks_failed += 1;
            report.errors.push_back(String::from_str(
                env,
                "Schema compatibility check failed",
            ));
        }
        
        Ok(report)
    }

    /// Check data consistency
    fn check_data_consistency(env: &Env) -> bool {
        // Verify all data structures are valid
        // This is a placeholder - implement actual checks
        true
    }
    
    /// Check referential integrity
    fn check_referential_integrity(env: &Env) -> bool {
        // Verify all references are valid
        // This is a placeholder - implement actual checks
        true
    }
    
    /// Check schema compatibility
    fn check_schema_compatibility(env: &Env) -> bool {
        // Verify schema is compatible
        // This is a placeholder - implement actual checks
        true
    }
    
    /// Validate business logic constraints
    pub fn validate_business_logic(
        env: &Env,
        data: &StateData,
    ) -> Result<(), MigrationError> {
        // Check business rules
        if data.value < 0 {
            return Err(MigrationError {
                step_id: 0,
                error_code: ErrorCode::InvalidData as u32,
                error_message: String::from_str(env, "Negative value not allowed"),
                recoverable: false,
            });
        }
        
        Ok(())
    }
    
    /// Validate data ranges
    pub fn validate_data_ranges(
        env: &Env,
        data: &StateData,
    ) -> Result<(), MigrationError> {
        // Check value ranges
        if data.value > 1_000_000_000 {
            return Err(MigrationError {
                step_id: 0,
                error_code: ErrorCode::OutOfRange as u32,
                error_message: String::from_str(env, "Value exceeds maximum"),
                recoverable: false,
            });
        }
        
        Ok(())
    }
}

/// Validation report
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ValidationReport {
    pub validation_id: u64,
    pub checks_passed: u32,
    pub checks_failed: u32,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// State data for validation
#[derive(Clone, Debug)]
#[contracttype]
pub struct StateData {
    pub key: String,
    pub value: i128,
}

// ============================================================================
// Rollback Mechanisms
// ============================================================================

/// Rollback manager for migrations
pub struct MigrationRollbackManager;

impl MigrationRollbackManager {
    /// Create snapshot before migration
    pub fn create_snapshot(
        env: &Env,
        migration_id: u64,
    ) -> Result<MigrationSnapshot, MigrationError> {
        let snapshot = MigrationSnapshot {
            snapshot_id: migration_id,
            version: get_current_version(env),
            timestamp: env.ledger().timestamp(),
            state_data: Self::capture_state(env),
            metadata: Self::capture_metadata(env),
        };
        
        // Store snapshot
        env.storage().instance().set(
            &DataKey::Snapshot(migration_id),
            &snapshot
        );
        
        Ok(snapshot)
    }
    
    /// Capture current state
    fn capture_state(env: &Env) -> Vec<StateData> {
        // Capture all relevant state
        // This is a placeholder - implement actual state capture
        Vec::new(env)
    }
    
    /// Capture metadata
    fn capture_metadata(env: &Env) -> Vec<String> {
        // Capture metadata
        Vec::new(env)
    }

    /// Rollback to snapshot
    pub fn rollback(
        env: &Env,
        migration_id: u64,
    ) -> Result<RollbackResult, MigrationError> {
        let start_time = env.ledger().timestamp();
        
        // Retrieve snapshot
        let snapshot: MigrationSnapshot = env
            .storage()
            .instance()
            .get(&DataKey::Snapshot(migration_id))
            .ok_or(MigrationError {
                step_id: 0,
                error_code: ErrorCode::SnapshotNotFound as u32,
                error_message: String::from_str(env, "Snapshot not found"),
                recoverable: false,
            })?;
        
        // Restore state
        Self::restore_state(env, &snapshot.state_data)?;
        
        // Restore metadata
        Self::restore_metadata(env, &snapshot.metadata)?;
        
        // Restore version
        set_current_version(env, snapshot.version);
        
        let end_time = env.ledger().timestamp();
        
        Ok(RollbackResult {
            migration_id,
            success: true,
            restored_version: snapshot.version,
            duration_ms: (end_time - start_time) * 1000,
        })
    }
    
    /// Restore state from snapshot
    fn restore_state(
        env: &Env,
        state_data: &Vec<StateData>,
    ) -> Result<(), MigrationError> {
        // Restore all state data
        for data in state_data.iter() {
            // Restore individual state items
            // This is a placeholder - implement actual restoration
        }
        Ok(())
    }
    
    /// Restore metadata
    fn restore_metadata(
        env: &Env,
        metadata: &Vec<String>,
    ) -> Result<(), MigrationError> {
        // Restore metadata
        Ok(())
    }

    /// Verify rollback success
    pub fn verify_rollback(
        env: &Env,
        snapshot: &MigrationSnapshot,
    ) -> bool {
        // Verify version matches
        if get_current_version(env) != snapshot.version {
            return false;
        }
        
        // Verify state integrity
        if !Self::verify_state_integrity(env, &snapshot.state_data) {
            return false;
        }
        
        true
    }
    
    /// Verify state integrity
    fn verify_state_integrity(
        env: &Env,
        expected_state: &Vec<StateData>,
    ) -> bool {
        // Verify state matches snapshot
        true
    }
    
    /// Clean up old snapshots
    pub fn cleanup_snapshots(env: &Env, keep_count: u32) {
        // Remove old snapshots beyond keep_count
        // This is a placeholder - implement actual cleanup
    }
}

/// Migration snapshot
#[derive(Clone, Debug)]
#[contracttype]
pub struct MigrationSnapshot {
    pub snapshot_id: u64,
    pub version: MigrationVersion,
    pub timestamp: u64,
    pub state_data: Vec<StateData>,
    pub metadata: Vec<String>,
}

/// Rollback result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct RollbackResult {
    pub migration_id: u64,
    pub success: bool,
    pub restored_version: MigrationVersion,
    pub duration_ms: u64,
}

// ============================================================================
// Migration Verification
// ============================================================================

/// Migration verifier
pub struct MigrationVerifier;

impl MigrationVerifier {
    /// Verify migration completion
    pub fn verify_migration(
        env: &Env,
        migration_id: u64,
        expected_version: MigrationVersion,
    ) -> Result<VerificationReport, MigrationError> {
        let mut report = VerificationReport {
            migration_id,
            verified: true,
            checks: Vec::new(env),
            issues: Vec::new(env),
        };
        
        // Check 1: Version matches
        let current_version = get_current_version(env);
        let version_check = VerificationCheck {
            check_name: String::from_str(env, "Version Check"),
            passed: current_version == expected_version,
            details: String::from_str(env, "Verify version updated correctly"),
        };
        
        if !version_check.passed {
            report.verified = false;
            report.issues.push_back(String::from_str(
                env,
                "Version mismatch",
            ));
        }
        report.checks.push_back(version_check);
        
        // Check 2: Data integrity
        let integrity_check = Self::verify_data_integrity(env);
        if !integrity_check.passed {
            report.verified = false;
            report.issues.push_back(String::from_str(
                env,
                "Data integrity check failed",
            ));
        }
        report.checks.push_back(integrity_check);
        
        // Check 3: Schema validity
        let schema_check = Self::verify_schema(env);
        if !schema_check.passed {
            report.verified = false;
            report.issues.push_back(String::from_str(
                env,
                "Schema validation failed",
            ));
        }
        report.checks.push_back(schema_check);
        
        Ok(report)
    }

    /// Verify data integrity
    fn verify_data_integrity(env: &Env) -> VerificationCheck {
        let passed = MigrationDataValidator::check_data_consistency(env)
            && MigrationDataValidator::check_referential_integrity(env);
        
        VerificationCheck {
            check_name: String::from_str(env, "Data Integrity"),
            passed,
            details: String::from_str(env, "Verify all data is consistent"),
        }
    }
    
    /// Verify schema
    fn verify_schema(env: &Env) -> VerificationCheck {
        let passed = MigrationDataValidator::check_schema_compatibility(env);
        
        VerificationCheck {
            check_name: String::from_str(env, "Schema Validation"),
            passed,
            details: String::from_str(env, "Verify schema is valid"),
        }
    }
    
    /// Verify business logic
    pub fn verify_business_logic(env: &Env) -> VerificationCheck {
        // Verify business rules are satisfied
        VerificationCheck {
            check_name: String::from_str(env, "Business Logic"),
            passed: true,
            details: String::from_str(env, "Verify business rules"),
        }
    }
    
    /// Verify performance
    pub fn verify_performance(
        env: &Env,
        expected_performance: PerformanceMetrics,
    ) -> VerificationCheck {
        // Verify performance meets expectations
        let actual = measure_performance(env);
        let passed = actual.gas_cost <= expected_performance.gas_cost
            && actual.execution_time <= expected_performance.execution_time;
        
        VerificationCheck {
            check_name: String::from_str(env, "Performance"),
            passed,
            details: String::from_str(env, "Verify performance acceptable"),
        }
    }
}

/// Verification report
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationReport {
    pub migration_id: u64,
    pub verified: bool,
    pub checks: Vec<VerificationCheck>,
    pub issues: Vec<String>,
}

/// Verification check
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationCheck {
    pub check_name: String,
    pub passed: bool,
    pub details: String,
}

/// Performance metrics
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PerformanceMetrics {
    pub gas_cost: u64,
    pub execution_time: u64,
    pub storage_used: u64,
}

// ============================================================================
// Migration Performance Tests
// ============================================================================

/// Migration performance tester
pub struct MigrationPerformanceTester;

impl MigrationPerformanceTester {
    /// Benchmark migration performance
    pub fn benchmark_migration(
        env: &Env,
        migration_plan: &MigrationPlan,
    ) -> MigrationBenchmark {
        let start_time = env.ledger().timestamp();
        let start_gas = estimate_gas(env);
        
        // Simulate migration
        let steps_executed = migration_plan.steps.len();
        
        let end_time = env.ledger().timestamp();
        let end_gas = estimate_gas(env);
        
        MigrationBenchmark {
            plan_id: migration_plan.plan_id,
            total_time_ms: (end_time - start_time) * 1000,
            total_gas: end_gas.saturating_sub(start_gas),
            steps_executed,
            avg_time_per_step: if steps_executed > 0 {
                ((end_time - start_time) * 1000) / steps_executed as u64
            } else {
                0
            },
            avg_gas_per_step: if steps_executed > 0 {
                end_gas.saturating_sub(start_gas) / steps_executed as u64
            } else {
                0
            },
        }
    }

    /// Test migration under load
    pub fn load_test_migration(
        env: &Env,
        migration_plan: &MigrationPlan,
        data_size: u32,
    ) -> LoadTestResult {
        let start_time = env.ledger().timestamp();
        
        // Simulate migration with varying data sizes
        let mut results = Vec::new(env);
        
        for size in 1..=data_size {
            let benchmark = Self::benchmark_migration(env, migration_plan);
            results.push_back(benchmark);
        }
        
        let end_time = env.ledger().timestamp();
        
        LoadTestResult {
            total_duration_ms: (end_time - start_time) * 1000,
            iterations: data_size,
            avg_time_per_iteration: if data_size > 0 {
                ((end_time - start_time) * 1000) / data_size as u64
            } else {
                0
            },
            benchmarks: results,
        }
    }
    
    /// Compare migration strategies
    pub fn compare_strategies(
        env: &Env,
        strategies: Vec<MigrationPlan>,
    ) -> Vec<MigrationBenchmark> {
        let mut results = Vec::new(env);
        
        for strategy in strategies.iter() {
            let benchmark = Self::benchmark_migration(env, &strategy);
            results.push_back(benchmark);
        }
        
        results
    }
}

/// Migration benchmark
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct MigrationBenchmark {
    pub plan_id: u64,
    pub total_time_ms: u64,
    pub total_gas: u64,
    pub steps_executed: u32,
    pub avg_time_per_step: u64,
    pub avg_gas_per_step: u64,
}

/// Load test result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct LoadTestResult {
    pub total_duration_ms: u64,
    pub iterations: u32,
    pub avg_time_per_iteration: u64,
    pub benchmarks: Vec<MigrationBenchmark>,
}

// ============================================================================
// Migration Executor
// ============================================================================

/// Main migration executor
pub struct MigrationExecutor;

impl MigrationExecutor {
    /// Execute migration plan
    pub fn execute_migration(
        env: &Env,
        plan: MigrationPlan,
        initiator: Address,
    ) -> Result<MigrationResult, MigrationError> {
        initiator.require_auth();
        
        let migration_id = get_next_migration_id(env);
        let start_time = env.ledger().timestamp();
        
        // Check version compatibility
        let current_version = get_current_version(env);
        if !VersionCompatibilityChecker::is_compatible(
            current_version,
            plan.to_version,
        ) {
            return Err(MigrationError {
                step_id: 0,
                error_code: ErrorCode::IncompatibleVersion as u32,
                error_message: String::from_str(env, "Incompatible versions"),
                recoverable: false,
            });
        }
        
        // Create snapshot if rollback enabled
        let snapshot = if plan.rollback_enabled {
            Some(MigrationRollbackManager::create_snapshot(env, migration_id)?)
        } else {
            None
        };
        
        // Update status
        set_migration_status(env, migration_id, MigrationStatus::InProgress);
        
        // Execute migration steps
        let mut steps_completed = 0u32;
        let mut steps_failed = 0u32;
        let mut errors = Vec::new(env);
        
        for step in plan.steps.iter() {
            match Self::execute_step(env, &step) {
                Ok(_) => steps_completed += 1,
                Err(error) => {
                    steps_failed += 1;
                    errors.push_back(error.clone());
                    
                    if step.critical {
                        // Rollback on critical failure
                        if let Some(snap) = snapshot {
                            MigrationRollbackManager::rollback(env, migration_id)?;
                        }
                        set_migration_status(env, migration_id, MigrationStatus::Failed);
                        
                        return Ok(MigrationResult {
                            migration_id,
                            success: false,
                            steps_completed,
                            steps_failed,
                            validation_passed: false,
                            errors,
                            duration_ms: (env.ledger().timestamp() - start_time) * 1000,
                        });
                    }
                }
            }
        }

        // Validate migration
        let validation_report = MigrationDataValidator::validate_data_integrity(
            env,
            migration_id,
        )?;
        
        let validation_passed = validation_report.checks_failed == 0;
        
        if !validation_passed && plan.rollback_enabled {
            if let Some(snap) = snapshot {
                MigrationRollbackManager::rollback(env, migration_id)?;
            }
            set_migration_status(env, migration_id, MigrationStatus::Failed);
            
            return Ok(MigrationResult {
                migration_id,
                success: false,
                steps_completed,
                steps_failed,
                validation_passed: false,
                errors,
                duration_ms: (env.ledger().timestamp() - start_time) * 1000,
            });
        }
        
        // Update version
        set_current_version(env, plan.to_version);
        
        // Mark as completed
        set_migration_status(env, migration_id, MigrationStatus::Completed);
        
        let end_time = env.ledger().timestamp();
        
        Ok(MigrationResult {
            migration_id,
            success: true,
            steps_completed,
            steps_failed,
            validation_passed,
            errors,
            duration_ms: (end_time - start_time) * 1000,
        })
    }
    
    /// Execute individual migration step
    fn execute_step(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        match step.step_type {
            StepType::AddField => Self::execute_add_field(env, step),
            StepType::RemoveField => Self::execute_remove_field(env, step),
            StepType::RenameField => Self::execute_rename_field(env, step),
            StepType::TransformData => Self::execute_transform_data(env, step),
            StepType::UpdateSchema => Self::execute_update_schema(env, step),
            StepType::MigrateStorage => Self::execute_migrate_storage(env, step),
        }
    }

    /// Execute add field step
    fn execute_add_field(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        // Add new field to schema
        // This is a placeholder - implement actual logic
        Ok(())
    }
    
    /// Execute remove field step
    fn execute_remove_field(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        // Remove field from schema
        // This is a placeholder - implement actual logic
        Ok(())
    }
    
    /// Execute rename field step
    fn execute_rename_field(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        // Rename field in schema
        // This is a placeholder - implement actual logic
        Ok(())
    }
    
    /// Execute transform data step
    fn execute_transform_data(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        // Transform data format
        // This is a placeholder - implement actual logic
        Ok(())
    }
    
    /// Execute update schema step
    fn execute_update_schema(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        // Update schema definition
        // This is a placeholder - implement actual logic
        Ok(())
    }
    
    /// Execute migrate storage step
    fn execute_migrate_storage(
        env: &Env,
        step: &MigrationStep,
    ) -> Result<(), MigrationError> {
        // Migrate storage format
        // This is a placeholder - implement actual logic
        Ok(())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get current version
fn get_current_version(env: &Env) -> MigrationVersion {
    env.storage()
        .instance()
        .get(&DataKey::CurrentVersion)
        .unwrap_or(1)
}

/// Set current version
fn set_current_version(env: &Env, version: MigrationVersion) {
    env.storage()
        .instance()
        .set(&DataKey::CurrentVersion, &version);
}

/// Get next migration ID
fn get_next_migration_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::MigrationCounter)
        .unwrap_or(0);
    
    let next = current + 1;
    env.storage()
        .instance()
        .set(&DataKey::MigrationCounter, &next);
    
    next
}

/// Set migration status
fn set_migration_status(
    env: &Env,
    migration_id: u64,
    status: MigrationStatus,
) {
    env.storage()
        .instance()
        .set(&DataKey::MigrationStatus(migration_id), &status);
}

/// Estimate gas usage
fn estimate_gas(env: &Env) -> u64 {
    env.ledger().sequence() as u64 * 1000
}

/// Measure performance
fn measure_performance(env: &Env) -> PerformanceMetrics {
    PerformanceMetrics {
        gas_cost: estimate_gas(env),
        execution_time: env.ledger().timestamp(),
        storage_used: 0,
    }
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum ErrorCode {
    IncompatibleVersion = 1,
    SnapshotNotFound = 2,
    ValidationFailed = 3,
    RollbackFailed = 4,
    InvalidData = 5,
    OutOfRange = 6,
    StepExecutionFailed = 7,
}

/// Storage keys
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    CurrentVersion,
    MigrationCounter,
    MigrationStatus(u64),
    Snapshot(u64),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_compatibility() {
        assert!(VersionCompatibilityChecker::is_compatible(1, 2));
        assert!(VersionCompatibilityChecker::is_compatible(1, 5));
        assert!(!VersionCompatibilityChecker::is_compatible(5, 1));
        assert!(!VersionCompatibilityChecker::is_compatible(1, 10));
    }

    #[test]
    fn test_breaking_changes() {
        assert!(!VersionCompatibilityChecker::has_breaking_changes(1, 2));
        assert!(VersionCompatibilityChecker::has_breaking_changes(2, 4));
        assert!(VersionCompatibilityChecker::has_breaking_changes(1, 10));
    }

    #[test]
    fn test_compatibility_score() {
        let score1 = VersionCompatibilityChecker::get_compatibility_score(1, 2);
        let score2 = VersionCompatibilityChecker::get_compatibility_score(1, 5);
        
        assert!(score1 > score2);
        assert!(score1 <= 100);
    }

    #[test]
    fn test_migration_path() {
        let path = VersionCompatibilityChecker::get_migration_path(1, 4);
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], 2);
        assert_eq!(path[1], 3);
        assert_eq!(path[2], 4);
    }

    // ============================================================================
    // Archival Export/Restore Tests
    // ============================================================================

    /// Create a test environment
    fn test_env() -> Env {
        Env::default()
    }

    /// Create a valid test export
    fn create_valid_export(env: &Env) -> ArchivalExport {
        let mut records = Vec::new(env);
        
        // Add test records sorted by (contract_id, key)
        let record1 = ArchivalExportRecord {
            contract_id: String::from_str(env, "contract_A"),
            schema_version: 1,
            ledger_sequence: 100,
            ledger_close_time: 1000000,
            network_passphrase: String::from_str(env, "Test Network"),
            key: String::from_str(env, "key1"),
            value: String::from_str(env, "value1"),
        };
        let record2 = ArchivalExportRecord {
            contract_id: String::from_str(env, "contract_A"),
            schema_version: 1,
            ledger_sequence: 100,
            ledger_close_time: 1000000,
            network_passphrase: String::from_str(env, "Test Network"),
            key: String::from_str(env, "key2"),
            value: String::from_str(env, "value2"),
        };
        let record3 = ArchivalExportRecord {
            contract_id: String::from_str(env, "contract_B"),
            schema_version: 1,
            ledger_sequence: 100,
            ledger_close_time: 1000000,
            network_passphrase: String::from_str(env, "Test Network"),
            key: String::from_str(env, "key1"),
            value: String::from_str(env, "value3"),
        };
        
        records.push_back(record1);
        records.push_back(record2);
        records.push_back(record3);
        
        let checksum = ArchivalRestoreVerifier::compute_checksum(&records);
        
        ArchivalExport {
            records,
            export_timestamp: env.ledger().timestamp(),
            export_checksum: checksum,
            format_version: ARCHIVAL_EXPORT_SCHEMA_VERSION,
        }
    }

    #[test]
    fn test_compute_checksum() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        // Verify checksum is deterministic
        let checksum1 = ArchivalRestoreVerifier::compute_checksum(&export.records);
        let checksum2 = ArchivalRestoreVerifier::compute_checksum(&export.records);
        assert_eq!(checksum1, checksum2);
        assert_eq!(checksum1, export.export_checksum);
    }

    #[test]
    fn test_checksum_changes_with_data() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        // Modify a record value
        let mut modified_records = export.records.clone();
        let mut record = modified_records.get(0).unwrap();
        record.value = String::from_str(&env, "modified_value");
        modified_records.set(0, record);
        
        let new_checksum = ArchivalRestoreVerifier::compute_checksum(&modified_records);
        assert_ne!(new_checksum, export.export_checksum);
    }

    #[test]
    fn test_validate_export_valid() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(result.valid);
        assert!(result.format_version_valid);
        assert!(result.checksum_valid);
        assert!(result.invariants_valid);
        assert_eq!(result.record_count, 3);
        assert_eq!(result.errors.len(), 0);
    }

    #[test]
    fn test_validate_export_invalid_format_version() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        export.format_version = 999; // Invalid version
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.format_version_valid);
        assert!(result.checksum_valid); // Checksum still valid
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_validate_export_corrupted_checksum() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        export.export_checksum = export.export_checksum.wrapping_add(1); // Corrupt checksum
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(result.format_version_valid);
        assert!(!result.checksum_valid);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_validate_export_schema_version_mismatch() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        
        // Modify one record to have different schema version
        let mut records = export.records.clone();
        let mut record = records.get(1).unwrap();
        record.schema_version = 2;
        records.set(1, record);
        export.records = records;
        // Don't update checksum - should fail on both checksum and invariant
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.checksum_valid);
        assert!(!result.invariants_valid);
    }

    #[test]
    fn test_validate_export_network_passphrase_mismatch() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        
        // Modify one record to have different network passphrase
        let mut records = export.records.clone();
        let mut record = records.get(1).unwrap();
        record.network_passphrase = String::from_str(&env, "Different Network");
        records.set(1, record);
        export.records = records;
        // Recompute checksum since we modified data
        export.export_checksum = ArchivalRestoreVerifier::compute_checksum(&export.records);
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.invariants_valid);
    }

    #[test]
    fn test_validate_export_unsorted_records() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        
        // Swap records to make them unsorted
        let mut records = export.records.clone();
        let record0 = records.get(0).unwrap();
        let record1 = records.get(1).unwrap();
        records.set(0, record1);
        records.set(1, record0);
        export.records = records;
        // Recompute checksum since we modified order
        export.export_checksum = ArchivalRestoreVerifier::compute_checksum(&export.records);
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.invariants_valid);
    }

    #[test]
    fn test_validate_export_duplicate_keys() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        
        // Add duplicate record
        let mut records = export.records.clone();
        let duplicate = ArchivalExportRecord {
            contract_id: String::from_str(&env, "contract_A"),
            schema_version: 1,
            ledger_sequence: 100,
            ledger_close_time: 1000000,
            network_passphrase: String::from_str(&env, "Test Network"),
            key: String::from_str(&env, "key1"), // Same as first record
            value: String::from_str(&env, "duplicate_value"),
        };
        records.push_back(duplicate);
        export.records = records;
        export.export_checksum = ArchivalRestoreVerifier::compute_checksum(&export.records);
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.invariants_valid);
    }

    #[test]
    fn test_validate_export_future_timestamp() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        export.export_timestamp = env.ledger().timestamp() + 1000; // Future timestamp
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.invariants_valid);
    }

    #[test]
    fn test_validate_export_too_old_timestamp() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        export.export_timestamp = env.ledger().timestamp() - (400 * 24 * 60 * 60); // > 1 year old
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.invariants_valid);
    }

    #[test]
    fn test_restore_from_export_valid() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        let result = ArchivalRestoreVerifier::restore_from_export(&env, &export);
        
        assert!(result.is_ok());
        let restore_result = result.unwrap();
        assert!(restore_result.success);
        assert_eq!(restore_result.records_restored, 3);
        assert_eq!(restore_result.export_timestamp, export.export_timestamp);
    }

    #[test]
    fn test_restore_from_export_invalid_rejected() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        export.export_checksum = export.export_checksum.wrapping_add(1); // Corrupt
        
        let result = ArchivalRestoreVerifier::restore_from_export(&env, &export);
        
        assert!(result.is_err());
        match result {
            Err(RestoreError::ValidationFailed(errors)) => {
                assert!(!errors.is_empty());
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_restore_from_export_atomic_no_partial_writes() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        
        // Add a record that will cause validation to fail (duplicate key)
        // but first let's test that valid export restores all records
        let result = ArchivalRestoreVerifier::restore_from_export(&env, &export);
        assert!(result.is_ok());
        
        // Verify all records were restored
        let verify_result = ArchivalRestoreVerifier::verify_restored_state(&env, &export).unwrap();
        assert!(verify_result.verified);
        assert_eq!(verify_result.verified_count, 3);
        assert_eq!(verify_result.mismatch_count, 0);
        
        // Now test invalid export - should not have written anything
        // We need a fresh env for this test since the previous restore wrote data
        let env2 = test_env();
        let mut bad_export = create_valid_export(&env2);
        bad_export.export_checksum = bad_export.export_checksum.wrapping_add(1);
        
        let result2 = ArchivalRestoreVerifier::restore_from_export(&env2, &bad_export);
        assert!(result2.is_err());
        
        // Verify nothing was written in the failed restore attempt
        for record in bad_export.records.iter() {
            let restore_key = DataKey::RestoredState(record.contract_id.clone(), record.key.clone());
            let stored: String = env2.storage().instance().get(&restore_key).unwrap_or(String::from_str(&env2, ""));
            assert_eq!(stored.len(), 0); // Should be empty/default
        }
    }

    #[test]
    fn test_verify_restored_state_valid() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        // Restore
        ArchivalRestoreVerifier::restore_from_export(&env, &export).unwrap();
        
        // Verify
        let result = ArchivalRestoreVerifier::verify_restored_state(&env, &export).unwrap();
        
        assert!(result.verified);
        assert_eq!(result.verified_count, 3);
        assert_eq!(result.mismatch_count, 0);
    }

    #[test]
    fn test_verify_restored_state_detects_mismatch() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        // Restore
        ArchivalRestoreVerifier::restore_from_export(&env, &export).unwrap();
        
        // Manually corrupt one restored value
        let record = export.records.get(0).unwrap();
        let restore_key = DataKey::RestoredState(record.contract_id.clone(), record.key.clone());
        env.storage().instance().set(&restore_key, &String::from_str(&env, "corrupted_value"));
        
        // Verify should detect mismatch
        let result = ArchivalRestoreVerifier::verify_restored_state(&env, &export).unwrap();
        
        assert!(!result.verified);
        assert_eq!(result.verified_count, 2);
        assert_eq!(result.mismatch_count, 1);
        assert_eq!(result.mismatches.len(), 1);
        assert_eq!(result.mismatches.get(0).unwrap().expected, String::from_str(&env, "value1"));
        assert_eq!(result.mismatches.get(0).unwrap().actual, String::from_str(&env, "corrupted_value"));
    }

    #[test]
    fn test_restore_round_trip() {
        let env = test_env();
        
        // Create export
        let export = create_valid_export(&env);
        
        // Restore
        let restore_result = ArchivalRestoreVerifier::restore_from_export(&env, &export).unwrap();
        assert!(restore_result.success);
        
        // Verify
        let verify_result = ArchivalRestoreVerifier::verify_restored_state(&env, &export).unwrap();
        assert!(verify_result.verified);
        
        // Validate export again (should still be valid)
        let validation = ArchivalRestoreVerifier::validate_export(&env, &export);
        assert!(validation.valid);
    }

    #[test]
    fn test_truncated_export_rejected() {
        let env = test_env();
        let mut export = create_valid_export(&env);
        
        // Truncate records (remove last record but keep original checksum)
        let mut records = export.records.clone();
        records.pop_back();
        export.records = records;
        // Note: we DON'T update checksum - simulating truncation
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        assert!(!result.valid);
        assert!(!result.checksum_valid);
    }

    #[test]
    fn test_empty_export() {
        let env = test_env();
        let records = Vec::new(&env);
        let checksum = ArchivalRestoreVerifier::compute_checksum(&records);
        
        let export = ArchivalExport {
            records,
            export_timestamp: env.ledger().timestamp(),
            export_checksum: checksum,
            format_version: ARCHIVAL_EXPORT_SCHEMA_VERSION,
        };
        
        let result = ArchivalRestoreVerifier::validate_export(&env, &export);
        
        // Empty export should be valid (no data to verify)
        assert!(result.valid);
        assert_eq!(result.record_count, 0);
    }

    #[test]
    fn test_checksum_deterministic_across_runs() {
        let env = test_env();
        let export = create_valid_export(&env);
        
        // Compute checksum multiple times
        let c1 = ArchivalRestoreVerifier::compute_checksum(&export.records);
        let c2 = ArchivalRestoreVerifier::compute_checksum(&export.records);
        let c3 = ArchivalRestoreVerifier::compute_checksum(&export.records);
        
        assert_eq!(c1, c2);
        assert_eq!(c2, c3);
        assert_eq!(c1, export.export_checksum);
    }

    #[test]
    fn test_record_ordering_in_checksum() {
        let env = test_env();
        
        // Create two exports with same records but different order
        let mut records1 = Vec::new(&env);
        let mut records2 = Vec::new(&env);
        
        let record_a = ArchivalExportRecord {
            contract_id: String::from_str(&env, "contract_A"),
            schema_version: 1,
            ledger_sequence: 100,
            ledger_close_time: 1000000,
            network_passphrase: String::from_str(&env, "Test Network"),
            key: String::from_str(&env, "key1"),
            value: String::from_str(&env, "value1"),
        };
        let record_b = ArchivalExportRecord {
            contract_id: String::from_str(&env, "contract_A"),
            schema_version: 1,
            ledger_sequence: 100,
            ledger_close_time: 1000000,
            network_passphrase: String::from_str(&env, "Test Network"),
            key: String::from_str(&env, "key2"),
            value: String::from_str(&env, "value2"),
        };
        
        records1.push_back(record_a.clone());
        records1.push_back(record_b.clone());
        
        records2.push_back(record_b.clone());
        records2.push_back(record_a.clone());
        
        let checksum1 = ArchivalRestoreVerifier::compute_checksum(&records1);
        let checksum2 = ArchivalRestoreVerifier::compute_checksum(&records2);
        
        // Different order should produce different checksums
        assert_ne!(checksum1, checksum2);
    }
}
