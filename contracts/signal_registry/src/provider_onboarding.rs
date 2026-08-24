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
// KYC Integration
// ============================================================================

/// KYC verification data
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct KYCData {
    pub provider: Address,
    pub kyc_id: String,
    pub verification_level: KYCLevel,
    pub verified_at: u64,
    pub expires_at: u64,
    pub provider_name: String,
    pub document_hash: String,
}

/// KYC verification level
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum KYCLevel {
    None,
    Basic,       // Name, email, phone
    Enhanced,    // + Address, DOB, ID
    Full,        // + Biometric, video verification
}

/// KYC verification result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct KYCVerificationResult {
    pub success: bool,
    pub kyc_id: String,
    pub level: KYCLevel,
    pub verified_at: u64,
    pub expires_at: u64,
    pub checks_passed: Vec<String>,
    pub checks_failed: Vec<String>,
}

/// KYC integration manager
pub struct KYCIntegrationManager;

impl KYCIntegrationManager {
    /// Submit KYC verification request
    pub fn submit_kyc_verification(
        env: &Env,
        provider: Address,
        level: KYCLevel,
        document_hash: String,
    ) -> Result<String, OnboardingError> {
        provider.require_auth();
        
        // Generate KYC ID
        let kyc_id = generate_kyc_id(env, &provider);
        
        // Create KYC data
        let kyc_data = KYCData {
            provider: provider.clone(),
            kyc_id: kyc_id.clone(),
            verification_level: level,
            verified_at: 0,
            expires_at: 0,
            provider_name: String::from_str(env, ""),
            document_hash,
        };
        
        // Store KYC data
        env.storage().instance().set(
            &DataKey::KYCData(provider.clone()),
            &kyc_data
        );
        
        Ok(kyc_id)
    }
    
    /// Verify KYC submission
    pub fn verify_kyc(
        env: &Env,
        provider: Address,
        kyc_id: String,
    ) -> Result<KYCVerificationResult, OnboardingError> {
        // Retrieve KYC data
        let mut kyc_data: KYCData = env
            .storage()
            .instance()
            .get(&DataKey::KYCData(provider.clone()))
            .ok_or(OnboardingError::KYCNotFound)?;
        
        // Perform verification checks
        let mut checks_passed = Vec::new(env);
        let mut checks_failed = Vec::new(env);
        
        // Check 1: Document validity
        if Self::verify_document(env, &kyc_data.document_hash) {
            checks_passed.push_back(String::from_str(env, "Document valid"));
        } else {
            checks_failed.push_back(String::from_str(env, "Document invalid"));
        }
        
        // Check 2: Identity verification
        if Self::verify_identity(env, &provider) {
            checks_passed.push_back(String::from_str(env, "Identity verified"));
        } else {
            checks_failed.push_back(String::from_str(env, "Identity failed"));
        }
        
        let success = checks_failed.is_empty();
        
        if success {
            // Update KYC data
            kyc_data.verified_at = env.ledger().timestamp();
            kyc_data.expires_at = env.ledger().timestamp() + (365 * 24 * 60 * 60); // 1 year
            
            env.storage().instance().set(
                &DataKey::KYCData(provider),
                &kyc_data
            );
        }
        
        Ok(KYCVerificationResult {
            success,
            kyc_id,
            level: kyc_data.verification_level,
            verified_at: kyc_data.verified_at,
            expires_at: kyc_data.expires_at,
            checks_passed,
            checks_failed,
        })
    }

    /// Check if KYC is valid
    pub fn is_kyc_valid(env: &Env, provider: &Address) -> bool {
        if let Some(kyc_data) = env
            .storage()
            .instance()
            .get::<DataKey, KYCData>(&DataKey::KYCData(provider.clone()))
        {
            let current_time = env.ledger().timestamp();
            kyc_data.verified_at > 0 && kyc_data.expires_at > current_time
        } else {
            false
        }
    }
    
    /// Verify document (placeholder)
    fn verify_document(env: &Env, document_hash: &String) -> bool {
        // In production, integrate with document verification service
        true
    }
    
    /// Verify identity (placeholder)
    fn verify_identity(env: &Env, provider: &Address) -> bool {
        // In production, integrate with identity verification service
        true
    }
    
    /// Get KYC data
    pub fn get_kyc_data(env: &Env, provider: &Address) -> Option<KYCData> {
        env.storage()
            .instance()
            .get(&DataKey::KYCData(provider.clone()))
    }
}

// ============================================================================
// Background Check Interface
// ============================================================================

/// Background check result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct BackgroundCheckResult {
    pub provider: Address,
    pub check_id: String,
    pub passed: bool,
    pub risk_score: u32,
    pub checks_performed: Vec<BackgroundCheckType>,
    pub flags: Vec<String>,
    pub completed_at: u64,
}

/// Background check type
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum BackgroundCheckType {
    CriminalRecord,
    CreditHistory,
    EmploymentHistory,
    EducationVerification,
    RegulatoryCheck,
    SanctionsList,
}

/// Background check manager
pub struct BackgroundCheckManager;

impl BackgroundCheckManager {
    /// Initiate background check
    pub fn initiate_check(
        env: &Env,
        provider: Address,
        check_types: Vec<BackgroundCheckType>,
    ) -> Result<String, OnboardingError> {
        // Generate check ID
        let check_id = generate_check_id(env, &provider);
        
        // Store check request
        env.storage().instance().set(
            &DataKey::BackgroundCheck(provider.clone()),
            &check_id
        );
        
        Ok(check_id)
    }
    
    /// Complete background check
    pub fn complete_check(
        env: &Env,
        provider: Address,
        check_id: String,
    ) -> Result<BackgroundCheckResult, OnboardingError> {
        // Perform checks
        let mut checks_performed = Vec::new(env);
        let mut flags = Vec::new(env);
        
        // Check 1: Criminal record
        checks_performed.push_back(BackgroundCheckType::CriminalRecord);
        if !Self::check_criminal_record(env, &provider) {
            flags.push_back(String::from_str(env, "Criminal record found"));
        }
        
        // Check 2: Sanctions list
        checks_performed.push_back(BackgroundCheckType::SanctionsList);
        if !Self::check_sanctions_list(env, &provider) {
            flags.push_back(String::from_str(env, "On sanctions list"));
        }
        
        // Check 3: Regulatory check
        checks_performed.push_back(BackgroundCheckType::RegulatoryCheck);
        if !Self::check_regulatory(env, &provider) {
            flags.push_back(String::from_str(env, "Regulatory issues"));
        }
        
        // Calculate risk score (0-100, lower is better)
        let risk_score = Self::calculate_risk_score(&flags);
        let passed = risk_score < 50;
        
        let result = BackgroundCheckResult {
            provider: provider.clone(),
            check_id,
            passed,
            risk_score,
            checks_performed,
            flags,
            completed_at: env.ledger().timestamp(),
        };
        
        // Store result
        env.storage().instance().set(
            &DataKey::BackgroundCheckResult(provider),
            &result
        );
        
        Ok(result)
    }
    
    /// Check criminal record (placeholder)
    fn check_criminal_record(env: &Env, provider: &Address) -> bool {
        // In production, integrate with background check service
        true
    }
    
    /// Check sanctions list (placeholder)
    fn check_sanctions_list(env: &Env, provider: &Address) -> bool {
        // In production, integrate with sanctions screening service
        true
    }
    
    /// Check regulatory status (placeholder)
    fn check_regulatory(env: &Env, provider: &Address) -> bool {
        // In production, integrate with regulatory database
        true
    }
    
    /// Calculate risk score
    fn calculate_risk_score(flags: &Vec<String>) -> u32 {
        // Each flag adds 20 points to risk score
        (flags.len() * 20).min(100) as u32
    }
    
    /// Get background check result
    pub fn get_check_result(
        env: &Env,
        provider: &Address,
    ) -> Option<BackgroundCheckResult> {
        env.storage()
            .instance()
            .get(&DataKey::BackgroundCheckResult(provider.clone()))
    }
}

// ============================================================================
// Provider Tier Assignment
// ============================================================================

/// Tier assignment criteria
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct TierCriteria {
    pub min_kyc_level: KYCLevel,
    pub max_risk_score: u32,
    pub min_track_record_days: u32,
    pub min_success_rate: u32,
    pub requires_institutional_backing: bool,
}

/// Tier assignment result
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct TierAssignmentResult {
    pub provider: Address,
    pub assigned_tier: ProviderTier,
    pub previous_tier: ProviderTier,
    pub criteria_met: Vec<String>,
    pub criteria_not_met: Vec<String>,
    pub assigned_at: u64,
}

/// Provider tier manager
pub struct ProviderTierManager;

impl ProviderTierManager {
    /// Assign tier to provider
    pub fn assign_tier(
        env: &Env,
        provider: Address,
    ) -> Result<TierAssignmentResult, OnboardingError> {
        // Get KYC data
        let kyc_data = KYCIntegrationManager::get_kyc_data(env, &provider)
            .ok_or(OnboardingError::KYCNotFound)?;
        
        // Get background check result
        let bg_check = BackgroundCheckManager::get_check_result(env, &provider)
            .ok_or(OnboardingError::BackgroundCheckNotFound)?;
        
        // Get current tier
        let previous_tier = Self::get_provider_tier(env, &provider);
        
        // Determine tier based on criteria
        let mut criteria_met = Vec::new(env);
        let mut criteria_not_met = Vec::new(env);
        
        let assigned_tier = Self::determine_tier(
            env,
            &kyc_data,
            &bg_check,
            &mut criteria_met,
            &mut criteria_not_met,
        );
        
        // Store tier assignment
        env.storage().instance().set(
            &DataKey::ProviderTier(provider.clone()),
            &assigned_tier
        );
        
        Ok(TierAssignmentResult {
            provider,
            assigned_tier,
            previous_tier,
            criteria_met,
            criteria_not_met,
            assigned_at: env.ledger().timestamp(),
        })
    }

    /// Determine appropriate tier
    fn determine_tier(
        env: &Env,
        kyc_data: &KYCData,
        bg_check: &BackgroundCheckResult,
        criteria_met: &mut Vec<String>,
        criteria_not_met: &mut Vec<String>,
    ) -> ProviderTier {
        // Platinum tier criteria
        if Self::meets_platinum_criteria(env, kyc_data, bg_check, criteria_met, criteria_not_met) {
            return ProviderTier::Platinum;
        }
        
        // Gold tier criteria
        if Self::meets_gold_criteria(env, kyc_data, bg_check, criteria_met, criteria_not_met) {
            return ProviderTier::Gold;
        }
        
        // Silver tier criteria
        if Self::meets_silver_criteria(env, kyc_data, bg_check, criteria_met, criteria_not_met) {
            return ProviderTier::Silver;
        }
        
        // Bronze tier criteria
        if Self::meets_bronze_criteria(env, kyc_data, bg_check, criteria_met, criteria_not_met) {
            return ProviderTier::Bronze;
        }
        
        ProviderTier::Unverified
    }
    
    /// Check Platinum tier criteria
    fn meets_platinum_criteria(
        env: &Env,
        kyc_data: &KYCData,
        bg_check: &BackgroundCheckResult,
        criteria_met: &mut Vec<String>,
        criteria_not_met: &mut Vec<String>,
    ) -> bool {
        let mut all_met = true;
        
        // Full KYC required
        if kyc_data.verification_level == KYCLevel::Full {
            criteria_met.push_back(String::from_str(env, "Full KYC verified"));
        } else {
            criteria_not_met.push_back(String::from_str(env, "Full KYC required"));
            all_met = false;
        }
        
        // Perfect background check
        if bg_check.risk_score == 0 {
            criteria_met.push_back(String::from_str(env, "Perfect background check"));
        } else {
            criteria_not_met.push_back(String::from_str(env, "Perfect background required"));
            all_met = false;
        }
        
        all_met
    }
    
    /// Check Gold tier criteria
    fn meets_gold_criteria(
        env: &Env,
        kyc_data: &KYCData,
        bg_check: &BackgroundCheckResult,
        criteria_met: &mut Vec<String>,
        criteria_not_met: &mut Vec<String>,
    ) -> bool {
        let mut all_met = true;
        
        // Enhanced or Full KYC
        if kyc_data.verification_level == KYCLevel::Enhanced 
            || kyc_data.verification_level == KYCLevel::Full {
            criteria_met.push_back(String::from_str(env, "Enhanced KYC verified"));
        } else {
            criteria_not_met.push_back(String::from_str(env, "Enhanced KYC required"));
            all_met = false;
        }
        
        // Low risk score
        if bg_check.risk_score < 20 {
            criteria_met.push_back(String::from_str(env, "Low risk score"));
        } else {
            criteria_not_met.push_back(String::from_str(env, "Lower risk score required"));
            all_met = false;
        }
        
        all_met
    }
    
    /// Check Silver tier criteria
    fn meets_silver_criteria(
        env: &Env,
        kyc_data: &KYCData,
        bg_check: &BackgroundCheckResult,
        criteria_met: &mut Vec<String>,
        criteria_not_met: &mut Vec<String>,
    ) -> bool {
        let mut all_met = true;
        
        // Basic or higher KYC
        if kyc_data.verification_level != KYCLevel::None {
            criteria_met.push_back(String::from_str(env, "KYC verified"));
        } else {
            criteria_not_met.push_back(String::from_str(env, "KYC required"));
            all_met = false;
        }
        
        // Moderate risk score
        if bg_check.risk_score < 50 {
            criteria_met.push_back(String::from_str(env, "Acceptable risk score"));
        } else {
            criteria_not_met.push_back(String::from_str(env, "Risk score too high"));
            all_met = false;
        }
        
        all_met
    }
    
    /// Check Bronze tier criteria
    fn meets_bronze_criteria(
        env: &Env,
        kyc_data: &KYCData,
        bg_check: &BackgroundCheckResult,
        criteria_met: &mut Vec<String>,
        criteria_not_met: &mut Vec<String>,
    ) -> bool {
        // Basic KYC and passed background check
        if kyc_data.verification_level != KYCLevel::None && bg_check.passed {
            criteria_met.push_back(String::from_str(env, "Basic requirements met"));
            true
        } else {
            criteria_not_met.push_back(String::from_str(env, "Basic requirements not met"));
            false
        }
    }
    
    /// Get provider tier
    pub fn get_provider_tier(env: &Env, provider: &Address) -> ProviderTier {
        env.storage()
            .instance()
            .get(&DataKey::ProviderTier(provider.clone()))
            .unwrap_or(ProviderTier::Unverified)
    }
    
    /// Get tier benefits
    pub fn get_tier_benefits(tier: &ProviderTier) -> TierBenefits {
        match tier {
            ProviderTier::Platinum => TierBenefits {
                max_signals: 100,
                reduced_fees: 50,
                priority_support: true,
                featured_listing: true,
            },
            ProviderTier::Gold => TierBenefits {
                max_signals: 50,
                reduced_fees: 30,
                priority_support: true,
                featured_listing: false,
            },
            ProviderTier::Silver => TierBenefits {
                max_signals: 20,
                reduced_fees: 15,
                priority_support: false,
                featured_listing: false,
            },
            ProviderTier::Bronze => TierBenefits {
                max_signals: 10,
                reduced_fees: 5,
                priority_support: false,
                featured_listing: false,
            },
            ProviderTier::Unverified => TierBenefits {
                max_signals: 3,
                reduced_fees: 0,
                priority_support: false,
                featured_listing: false,
            },
        }
    }
}

/// Tier benefits
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct TierBenefits {
    pub max_signals: u32,
    pub reduced_fees: u32,        // Percentage
    pub priority_support: bool,
    pub featured_listing: bool,
}

// ============================================================================
// Verification Tracking
// ============================================================================

/// Verification tracking record
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationTracking {
    pub provider: Address,
    pub events: Vec<VerificationEvent>,
    pub current_status: VerificationStatus,
    pub last_updated: u64,
}

/// Verification event
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct VerificationEvent {
    pub event_id: u64,
    pub event_type: EventType,
    pub timestamp: u64,
    pub details: String,
    pub actor: Address,
}

/// Event type
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub enum EventType {
    WorkflowStarted,
    StepCompleted,
    KYCSubmitted,
    KYCVerified,
    BackgroundCheckInitiated,
    BackgroundCheckCompleted,
    TierAssigned,
    StatusChanged,
    DocumentUploaded,
    ReviewCompleted,
}

/// Verification tracking manager
pub struct VerificationTrackingManager;

impl VerificationTrackingManager {
    /// Initialize tracking for provider
    pub fn initialize_tracking(
        env: &Env,
        provider: Address,
    ) -> VerificationTracking {
        let tracking = VerificationTracking {
            provider: provider.clone(),
            events: Vec::new(env),
            current_status: VerificationStatus::NotStarted,
            last_updated: env.ledger().timestamp(),
        };
        
        // Store tracking
        env.storage().instance().set(
            &DataKey::Tracking(provider),
            &tracking
        );
        
        tracking
    }
    
    /// Add verification event
    pub fn add_event(
        env: &Env,
        provider: Address,
        event_type: EventType,
        details: String,
        actor: Address,
    ) -> Result<(), OnboardingError> {
        let mut tracking: VerificationTracking = env
            .storage()
            .instance()
            .get(&DataKey::Tracking(provider.clone()))
            .ok_or(OnboardingError::TrackingNotFound)?;
        
        let event = VerificationEvent {
            event_id: tracking.events.len() as u64,
            event_type,
            timestamp: env.ledger().timestamp(),
            details,
            actor,
        };
        
        tracking.events.push_back(event);
        tracking.last_updated = env.ledger().timestamp();
        
        // Update storage
        env.storage().instance().set(
            &DataKey::Tracking(provider),
            &tracking
        );
        
        Ok(())
    }

    /// Update verification status
    pub fn update_status(
        env: &Env,
        provider: Address,
        new_status: VerificationStatus,
        actor: Address,
    ) -> Result<(), OnboardingError> {
        let mut tracking: VerificationTracking = env
            .storage()
            .instance()
            .get(&DataKey::Tracking(provider.clone()))
            .ok_or(OnboardingError::TrackingNotFound)?;
        
        tracking.current_status = new_status.clone();
        tracking.last_updated = env.ledger().timestamp();
        
        // Add status change event
        let event = VerificationEvent {
            event_id: tracking.events.len() as u64,
            event_type: EventType::StatusChanged,
            timestamp: env.ledger().timestamp(),
            details: String::from_str(env, "Status updated"),
            actor,
        };
        
        tracking.events.push_back(event);
        
        // Update storage
        env.storage().instance().set(
            &DataKey::Tracking(provider),
            &tracking
        );
        
        Ok(())
    }
    
    /// Get verification tracking
    pub fn get_tracking(
        env: &Env,
        provider: &Address,
    ) -> Option<VerificationTracking> {
        env.storage()
            .instance()
            .get(&DataKey::Tracking(provider.clone()))
    }
    
    /// Get event history
    pub fn get_event_history(
        env: &Env,
        provider: &Address,
    ) -> Vec<VerificationEvent> {
        if let Some(tracking) = Self::get_tracking(env, provider) {
            tracking.events
        } else {
            Vec::new(env)
        }
    }
}

// ============================================================================
// Provider Dashboard
// ============================================================================

/// Provider dashboard data
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct ProviderDashboard {
    pub provider: Address,
    pub tier: ProviderTier,
    pub verification_status: VerificationStatus,
    pub kyc_status: KYCStatus,
    pub background_check_status: BackgroundCheckStatus,
    pub active_signals: u32,
    pub total_followers: u32,
    pub success_rate: u32,
    pub tier_benefits: TierBenefits,
    pub next_tier: Option<ProviderTier>,
    pub requirements_for_next_tier: Vec<String>,
}

/// KYC status summary
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct KYCStatus {
    pub verified: bool,
    pub level: KYCLevel,
    pub expires_at: u64,
}

/// Background check status summary
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct BackgroundCheckStatus {
    pub completed: bool,
    pub passed: bool,
    pub risk_score: u32,
}

/// Provider dashboard manager
pub struct ProviderDashboardManager;

impl ProviderDashboardManager {
    /// Get provider dashboard
    pub fn get_dashboard(
        env: &Env,
        provider: Address,
    ) -> Result<ProviderDashboard, OnboardingError> {
        // Get tier
        let tier = ProviderTierManager::get_provider_tier(env, &provider);
        
        // Get verification status
        let tracking = VerificationTrackingManager::get_tracking(env, &provider)
            .ok_or(OnboardingError::TrackingNotFound)?;
        
        // Get KYC status
        let kyc_status = Self::get_kyc_status(env, &provider);
        
        // Get background check status
        let bg_check_status = Self::get_background_check_status(env, &provider);
        
        // Get tier benefits
        let tier_benefits = ProviderTierManager::get_tier_benefits(&tier);
        
        // Determine next tier and requirements
        let (next_tier, requirements) = Self::get_next_tier_info(env, &tier, &kyc_status, &bg_check_status);
        
        Ok(ProviderDashboard {
            provider: provider.clone(),
            tier,
            verification_status: tracking.current_status,
            kyc_status,
            background_check_status: bg_check_status,
            active_signals: Self::get_active_signals(env, &provider),
            total_followers: Self::get_total_followers(env, &provider),
            success_rate: Self::get_success_rate(env, &provider),
            tier_benefits,
            next_tier,
            requirements_for_next_tier: requirements,
        })
    }
    
    /// Get KYC status summary
    fn get_kyc_status(env: &Env, provider: &Address) -> KYCStatus {
        if let Some(kyc_data) = KYCIntegrationManager::get_kyc_data(env, provider) {
            KYCStatus {
                verified: kyc_data.verified_at > 0,
                level: kyc_data.verification_level,
                expires_at: kyc_data.expires_at,
            }
        } else {
            KYCStatus {
                verified: false,
                level: KYCLevel::None,
                expires_at: 0,
            }
        }
    }
    
    /// Get background check status summary
    fn get_background_check_status(env: &Env, provider: &Address) -> BackgroundCheckStatus {
        if let Some(bg_check) = BackgroundCheckManager::get_check_result(env, provider) {
            BackgroundCheckStatus {
                completed: true,
                passed: bg_check.passed,
                risk_score: bg_check.risk_score,
            }
        } else {
            BackgroundCheckStatus {
                completed: false,
                passed: false,
                risk_score: 100,
            }
        }
    }
    
    /// Get next tier information
    fn get_next_tier_info(
        env: &Env,
        current_tier: &ProviderTier,
        kyc_status: &KYCStatus,
        bg_check_status: &BackgroundCheckStatus,
    ) -> (Option<ProviderTier>, Vec<String>) {
        let mut requirements = Vec::new(env);
        
        let next_tier = match current_tier {
            ProviderTier::Unverified => {
                requirements.push_back(String::from_str(env, "Complete KYC verification"));
                requirements.push_back(String::from_str(env, "Pass background check"));
                Some(ProviderTier::Bronze)
            },
            ProviderTier::Bronze => {
                requirements.push_back(String::from_str(env, "Upgrade to Enhanced KYC"));
                requirements.push_back(String::from_str(env, "Achieve risk score < 50"));
                Some(ProviderTier::Silver)
            },
            ProviderTier::Silver => {
                requirements.push_back(String::from_str(env, "Upgrade to Full KYC"));
                requirements.push_back(String::from_str(env, "Achieve risk score < 20"));
                Some(ProviderTier::Gold)
            },
            ProviderTier::Gold => {
                requirements.push_back(String::from_str(env, "Achieve perfect risk score"));
                requirements.push_back(String::from_str(env, "Obtain institutional backing"));
                Some(ProviderTier::Platinum)
            },
            ProviderTier::Platinum => None,
        };
        
        (next_tier, requirements)
    }
    
    /// Get active signals count (placeholder)
    fn get_active_signals(env: &Env, provider: &Address) -> u32 {
        // In production, query actual signal count
        0
    }
    
    /// Get total followers count (placeholder)
    fn get_total_followers(env: &Env, provider: &Address) -> u32 {
        // In production, query actual follower count
        0
    }
    
    /// Get success rate (placeholder)
    fn get_success_rate(env: &Env, provider: &Address) -> u32 {
        // In production, calculate actual success rate
        0
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get next workflow ID
fn get_next_workflow_id(env: &Env) -> u64 {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::WorkflowCounter)
        .unwrap_or(0);
    
    let next = current + 1;
    env.storage()
        .instance()
        .set(&DataKey::WorkflowCounter, &next);
    
    next
}

/// Generate KYC ID
fn generate_kyc_id(env: &Env, provider: &Address) -> String {
    let timestamp = env.ledger().timestamp();
    String::from_str(env, &format!("KYC-{}-{}", provider, timestamp))
}

/// Generate check ID
fn generate_check_id(env: &Env, provider: &Address) -> String {
    let timestamp = env.ledger().timestamp();
    String::from_str(env, &format!("BGC-{}-{}", provider, timestamp))
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum OnboardingError {
    WorkflowNotFound = 1,
    KYCNotFound = 2,
    BackgroundCheckNotFound = 3,
    TrackingNotFound = 4,
    InvalidStatus = 5,
    VerificationFailed = 6,
    InsufficientTier = 7,
}

/// Storage keys
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    WorkflowCounter,
    Workflow(u64),
    KYCData(Address),
    BackgroundCheck(Address),
    BackgroundCheckResult(Address),
    ProviderTier(Address),
    Tracking(Address),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_creation() {
        let env = Env::default();
        let provider = Address::generate(&env);
        
        let workflow = VerificationWorkflowManager::create_workflow(&env, provider);
        
        assert_eq!(workflow.status, VerificationStatus::Pending);
        assert_eq!(workflow.current_step, 0);
        assert_eq!(workflow.total_steps, 6);
    }

    #[test]
    fn test_tier_benefits() {
        let platinum_benefits = ProviderTierManager::get_tier_benefits(&ProviderTier::Platinum);
        assert_eq!(platinum_benefits.max_signals, 100);
        assert_eq!(platinum_benefits.reduced_fees, 50);
        
        let bronze_benefits = ProviderTierManager::get_tier_benefits(&ProviderTier::Bronze);
        assert_eq!(bronze_benefits.max_signals, 10);
        assert_eq!(bronze_benefits.reduced_fees, 5);
    }
}
