// Provider Onboarding and KYC Verification System
// Streamlined onboarding with compliance and risk assessment

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Vec};

// ============================================================================
// Provider Verification Workflow
// ============================================================================

/// Provider verification status
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum VerificationStatus {
    NotStarted,
    Pending,
    InReview,
    Approved,
    Rejected,
    Suspended,
    Revoked,
}

/// Provider tier levels
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum ProviderTier {
    Unverified,      // No verification
    Bronze,          // Basic verification
    Silver,          // Enhanced verification
    Gold,            // Full verification + track record
    Platinum,        // Gold + institutional backing
}

/// Verification workflow state
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationWorkflow {
    pub provider: Address,
    pub workflow_id: u64,
    pub status: VerificationStatus,
    pub current_step: u32,
    pub total_steps: u32,
    pub started_at: u64,
    pub updated_at: u64,
    pub completed_at: u64,
}

/// Verification step
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationStep {
    pub step_id: u32,
    pub step_type: StepType,
    pub status: StepStatus,
    pub required: bool,
    pub completed_at: u64,
}

/// Step type
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum StepType {
    IdentityVerification,
    DocumentSubmission,
    BackgroundCheck,
    RiskAssessment,
    ComplianceReview,
    TierAssignment,
}

/// Step status
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum StepStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

/// Maximum number of records a single paginated query may return.
pub const MAX_PAGE_SIZE: u32 = 50;

/// Maximum number of cleanup records a single exit-cleanup call may process.
///
/// Provider exit cleanup can touch many positions, signals, and related
/// records. To stay within Soroban execution limits, each call processes at
/// most this many records and returns a cursor so the caller can resume.
pub const MAX_EXIT_CLEANUP_PER_CALL: u32 = 25;

/// A provider relationship record (e.g. a provider linked to another provider).
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ProviderRelationship {
    pub provider: Address,
    pub counterparty: Address,
    pub relationship_id: u64,
    pub created_at: u64,
}

/// A provider membership record (e.g. a provider belonging to a group).
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ProviderMembership {
    pub provider: Address,
    pub group_id: u64,
    pub membership_id: u64,
    pub joined_at: u64,
}

/// A single page of results plus the cursor to fetch the next page.
///
/// `next_cursor` is `0` when the returned page is the final page.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: u64,
}

/// Progress of a bounded provider exit cleanup.
///
/// `cursor` is the id of the last record processed; pass it back into
/// `continue_exit_cleanup` to resume. `done` is `true` once every record has
/// been processed. `processed` counts records handled by the most recent call.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ExitCleanupProgress {
    pub provider: Address,
    pub cursor: u64,
    pub processed: u32,
    pub done: bool,
}

/// Verification workflow manager
pub struct VerificationWorkflowManager;

impl VerificationWorkflowManager {
    /// Create new verification workflow
    pub fn create_workflow(
        env: &Env,
        provider: Address,
    ) -> VerificationWorkflow {
        let workflow_id = get_next_workflow_id(env);
        
        let workflow = VerificationWorkflow {
            provider: provider.clone(),
            workflow_id,
            status: VerificationStatus::Pending,
            current_step: 0,
            total_steps: 6,
            started_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            completed_at: 0,
        };
        
        // Store workflow
        env.storage().instance().set(
            &DataKey::Workflow(workflow_id),
            &workflow
        );
        
        workflow
    }
    
    /// Advance workflow to next step
    pub fn advance_step(
        env: &Env,
        workflow_id: u64,
    ) -> Result<VerificationWorkflow, OnboardingError> {
        let mut workflow: VerificationWorkflow = env
            .storage()
            .instance()
            .get(&DataKey::Workflow(workflow_id))
            .ok_or(OnboardingError::WorkflowNotFound)?;
        
        workflow.current_step += 1;
        workflow.updated_at = env.ledger().timestamp();
        
        if workflow.current_step >= workflow.total_steps {
            workflow.status = VerificationStatus::InReview;
        }
        
        // Update storage
        env.storage().instance().set(
            &DataKey::Workflow(workflow_id),
            &workflow
        );
        
        Ok(workflow)
    }

    /// Complete workflow
    pub fn complete_workflow(
        env: &Env,
        workflow_id: u64,
        approved: bool,
    ) -> Result<VerificationWorkflow, OnboardingError> {
        let mut workflow: VerificationWorkflow = env
            .storage()
            .instance()
            .get(&DataKey::Workflow(workflow_id))
            .ok_or(OnboardingError::WorkflowNotFound)?;
        
        workflow.status = if approved {
            VerificationStatus::Approved
        } else {
            VerificationStatus::Rejected
        };
        workflow.completed_at = env.ledger().timestamp();
        workflow.updated_at = env.ledger().timestamp();
        
        // Update storage
        env.storage().instance().set(
            &DataKey::Workflow(workflow_id),
            &workflow
        );
        
        Ok(workflow)
    }
    
    /// Get workflow status
    pub fn get_workflow(
        env: &Env,
        workflow_id: u64,
    ) -> Option<VerificationWorkflow> {
        env.storage()
            .instance()
            .get(&DataKey::Workflow(workflow_id))
    }
}

// ============================================================================
// Bounded Provider Exit Cleanup
// ============================================================================

/// Bounded, resumable cleanup of a provider's relationships and memberships.
///
/// Each call processes at most `MAX_EXIT_CLEANUP_PER_CALL` records so the work
/// stays within Soroban execution limits even when a provider owns many
/// positions, signals, or related records. Progress is persisted per provider
/// so repeated or resumed calls advance toward completion without
/// double-processing already-cleaned records.
pub struct ProviderExitCleanupManager;

impl ProviderExitCleanupManager {
    /// Begin (or restart) exit cleanup for `provider`.
    ///
    /// Requires the provider's authorization. Resets any prior progress so the
    /// cleanup starts from the first record.
    pub fn begin_exit_cleanup(
        env: &Env,
        provider: Address,
    ) -> Result<ExitCleanupProgress, OnboardingError> {
        provider.require_auth();

        let progress = ExitCleanupProgress {
            provider: provider.clone(),
            cursor: 0,
            processed: 0,
            done: false,
        };
        env.storage()
            .instance()
            .set(&DataKey::ExitCleanup(provider.clone()), &progress);

        Ok(progress)
    }

    /// Continue exit cleanup for `provider`, processing a bounded batch.
    ///
    /// Requires the provider's authorization. Idempotent: once `done` is
    /// `true`, further calls are no-ops that return the completed progress
    /// without reprocessing records. Each call advances the stored cursor by
    /// at most `MAX_EXIT_CLEANUP_PER_CALL` records.
    pub fn continue_exit_cleanup(
        env: &Env,
        provider: Address,
    ) -> Result<ExitCleanupProgress, OnboardingError> {
        provider.require_auth();

        let mut progress: ExitCleanupProgress = env
            .storage()
            .instance()
            .get(&DataKey::ExitCleanup(provider.clone()))
            .ok_or(OnboardingError::CleanupNotStarted)?;

        // Idempotent: nothing left to do.
        if progress.done {
            return Ok(progress);
        }

        let ids: Vec<u64> = env
            .storage()
            .instance()
            .get(&DataKey::RelationshipIds(provider.clone()))
            .unwrap_or_else(|| Vec::new(env));

        let mut processed: u32 = 0;
        let mut cursor = progress.cursor;
        let mut done = true;

        for id in ids.iter() {
            if id <= cursor {
                continue;
            }
            if processed == MAX_EXIT_CLEANUP_PER_CALL {
                // More work remains; stop and let the caller resume.
                done = false;
                break;
            }
            // Remove the relationship record and its id from the index.
            env.storage()
                .instance()
                .remove(&DataKey::Relationship(id));
            cursor = id;
            processed += 1;
        }

        progress.cursor = cursor;
        progress.processed = processed;
        progress.done = done;
        env.storage()
            .instance()
            .set(&DataKey::ExitCleanup(provider.clone()), &progress);

        Ok(progress)
    }

    /// Read the current exit-cleanup progress for `provider`, if any.
    pub fn get_exit_cleanup(
        env: &Env,
        provider: Address,
    ) -> Option<ExitCleanupProgress> {
        env.storage()
            .instance()
            .get(&DataKey::ExitCleanup(provider))
    }
}

// ============================================================================
// Paginated Provider Relationship Queries
// ============================================================================

/// Read-only paginated queries over provider relationships and memberships.
///
/// Cursors are stable: a cursor is the id of the last record returned by the
/// previous page, and `0` denotes the first page. Records are always returned
/// in ascending id order, so repeated calls with the same cursor and limit
/// yield identical results.
pub struct ProviderQueryManager;

impl ProviderQueryManager {
    /// Validate a requested page size against the documented bound.
    fn validate_limit(limit: u32) -> Result<(), OnboardingError> {
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(OnboardingError::InvalidPageSize);
        }
        Ok(())
    }

    /// Fetch a bounded page of relationships for `provider`.
    ///
    /// `cursor` is the id of the last relationship seen; pass `0` for the
    /// first page. Returns `OnboardingError::InvalidCursor` when the cursor
    /// does not correspond to a stored relationship for this provider, and
    /// `OnboardingError::InvalidPageSize` when `limit` is `0` or exceeds
    /// `MAX_PAGE_SIZE`.
    pub fn get_relationships(
        env: &Env,
        provider: Address,
        cursor: u64,
        limit: u32,
    ) -> Result<Page<ProviderRelationship>, OnboardingError> {
        Self::validate_limit(limit)?;

        let ids: Vec<u64> = env
            .storage()
            .instance()
            .get(&DataKey::RelationshipIds(provider.clone()))
            .unwrap_or_else(|| Vec::new(env));

        if cursor != 0 && !ids.iter().any(|id| id == cursor) {
            return Err(OnboardingError::InvalidCursor);
        }

        let mut items = Vec::new(env);
        let mut next_cursor: u64 = 0;
        let mut collected: u32 = 0;

        for id in ids.iter() {
            if id <= cursor {
                continue;
            }
            if collected == limit {
                next_cursor = id;
                break;
            }
            if let Some(record) = env
                .storage()
                .instance()
                .get::<DataKey, ProviderRelationship>(&DataKey::Relationship(id))
            {
                items.push_back(record);
                collected += 1;
            }
        }

        Ok(Page { items, next_cursor })
    }
}
