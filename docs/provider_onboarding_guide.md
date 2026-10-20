# Provider Onboarding and KYC Verification Guide

## Overview

The StellarSwipe Provider Onboarding system provides a comprehensive, compliant framework for verifying and onboarding signal providers. This guide covers the complete onboarding process, KYC verification, background checks, tier assignment, and provider dashboard features.

## Table of Contents

1. [Onboarding Workflow](#onboarding-workflow)
2. [KYC Verification](#kyc-verification)
3. [Background Checks](#background-checks)
4. [Provider Tiers](#provider-tiers)
5. [Verification Tracking](#verification-tracking)
6. [Provider Dashboard](#provider-dashboard)
7. [Compliance Requirements](#compliance-requirements)
8. [Integration Guide](#integration-guide)
9. [Best Practices](#best-practices)
10. [Troubleshooting](#troubleshooting)

---

## Onboarding Workflow

### Workflow Overview

```
┌─────────────────────────────────────────────────────────┐
│          Provider Onboarding Workflow                    │
├─────────────────────────────────────────────────────────┤
│                                                           │
│  1. Registration                                         │
│     └─> Create account                                   │
│                                                           │
│  2. Identity Verification (KYC)                          │
│     ├─> Submit documents                                 │
│     ├─> Verify identity                                  │
│     └─> Assign KYC level                                 │
│                                                           │
│  3. Background Check                                     │
│     ├─> Criminal record check                            │
│     ├─> Sanctions list screening                         │
│     ├─> Regulatory check                                 │
│     └─> Risk score calculation                           │
│                                                           │
│  4. Risk Assessment                                      │
│     └─> Calculate overall risk                           │
│                                                           │
│  5. Tier Assignment                                      │
│     └─> Assign provider tier                             │
│                                                           │
│  6. Approval                                             │
│     └─> Final review and approval                        │
│                                                           │
└─────────────────────────────────────────────────────────┘
```

### Verification Status

| Status | Description | Next Steps |
|--------|-------------|------------|
| NotStarted | Provider registered but not started verification | Begin KYC process |
| Pending | Verification in progress | Complete required steps |
| InReview | All steps completed, awaiting review | Wait for admin review |
| Approved | Verification approved | Start providing signals |
| Rejected | Verification rejected | Review rejection reasons |
| Suspended | Temporarily suspended | Contact support |
| Revoked | Permanently revoked | Appeal or reapply |

### Creating a Verification Workflow

```rust
// Create new workflow
let workflow = VerificationWorkflowManager::create_workflow(
    &env,
    provider_address,
);

// Workflow starts in Pending status
assert_eq!(workflow.status, VerificationStatus::Pending);
assert_eq!(workflow.total_steps, 6);
```

### Advancing Through Steps

```rust
// Complete a step and advance
let updated_workflow = VerificationWorkflowManager::advance_step(
    &env,
    workflow_id,
)?;

// Check progress
println!("Step {}/{}", 
    updated_workflow.current_step,
    updated_workflow.total_steps
);
```

---

## KYC Verification

### KYC Levels

| Level | Requirements | Verification Time | Use Case |
|-------|-------------|-------------------|----------|
| **None** | No verification | N/A | Not allowed to provide signals |
| **Basic** | Name, email, phone | 1-2 hours | Small-scale providers |
| **Enhanced** | + Address, DOB, ID document | 1-2 days | Medium-scale providers |
| **Full** | + Biometric, video verification | 3-5 days | Large-scale/institutional providers |

### KYC Submission Process

#### Step 1: Submit KYC Request

```rust
// Submit KYC verification
let kyc_id = KYCIntegrationManager::submit_kyc_verification(
    &env,
    provider,
    KYCLevel::Enhanced,
    document_hash,
)?;

println!("KYC ID: {}", kyc_id);
```

#### Step 2: Document Requirements

**Basic KYC**:
- Government-issued ID (front)
- Proof of address (utility bill, bank statement)
- Selfie photo

**Enhanced KYC**:
- Government-issued ID (front and back)
- Proof of address (dated within 3 months)
- Selfie with ID
- Additional identity verification

**Full KYC**:
- All Enhanced KYC documents
- Biometric verification
- Live video verification
- Additional background documentation

#### Step 3: Verification Process

```rust
// Verify KYC submission
let result = KYCIntegrationManager::verify_kyc(
    &env,
    provider,
    kyc_id,
)?;

if result.success {
    println!("KYC verified successfully");
    println!("Level: {:?}", result.level);
    println!("Expires: {}", result.expires_at);
} else {
    println!("KYC verification failed:");
    for failure in result.checks_failed.iter() {
        println!("  - {}", failure);
    }
}
```

### KYC Validity

KYC verification is valid for **1 year** from verification date. Providers must renew before expiration.

```rust
// Check if KYC is still valid
let is_valid = KYCIntegrationManager::is_kyc_valid(&env, &provider);

if !is_valid {
    println!("KYC expired or not verified - renewal required");
}
```

---

## Background Checks

### Check Types

1. **Criminal Record Check**: Searches criminal databases
2. **Credit History**: Reviews financial responsibility
3. **Employment History**: Verifies work experience
4. **Education Verification**: Confirms credentials
5. **Regulatory Check**: Checks regulatory actions
6. **Sanctions List**: Screens against sanctions lists

### Initiating Background Check

```rust
// Define check types
let check_types = vec![
    BackgroundCheckType::CriminalRecord,
    BackgroundCheckType::SanctionsList,
    BackgroundCheckType::RegulatoryCheck,
];

// Initiate check
let check_id = BackgroundCheckManager::initiate_check(
    &env,
    provider,
    check_types,
)?;
```

### Background Check Results

```rust
// Complete and retrieve results
let result = BackgroundCheckManager::complete_check(
    &env,
    provider,
    check_id,
)?;

println!("Background Check Results:");
println!("  Passed: {}", result.passed);
println!("  Risk Score: {}/100", result.risk_score);
println!("  Checks Performed: {}", result.checks_performed.len());

if !result.flags.is_empty() {
    println!("  Flags:");
    for flag in result.flags.iter() {
        println!("    - {}", flag);
    }
}
```

### Risk Score Interpretation

| Score Range | Risk Level | Action |
|-------------|-----------|--------|
| 0-20 | Very Low | Approve for all tiers |
| 21-40 | Low | Approve for Bronze-Gold |
| 41-60 | Moderate | Approve for Bronze-Silver only |
| 61-80 | High | Manual review required |
| 81-100 | Very High | Likely rejection |

---

## Provider Tiers

### Tier System

| Tier | KYC Level | Max Risk Score | Max Signals | Fee Discount | Priority Support |
|------|-----------|----------------|-------------|--------------|------------------|
| **Platinum** | Full | 0 | 100 | 50% | ✅ + Featured |
| **Gold** | Enhanced/Full | <20 | 50 | 30% | ✅ |
| **Silver** | Basic+ | <50 | 20 | 15% | ❌ |
| **Bronze** | Basic | <100 | 10 | 5% | ❌ |
| **Unverified** | None | N/A | 3 | 0% | ❌ |

### Tier Assignment

```rust
// Assign tier based on verification results
let assignment = ProviderTierManager::assign_tier(
    &env,
    provider,
)?;

println!("Tier Assignment:");
println!("  Previous: {:?}", assignment.previous_tier);
println!("  Assigned: {:?}", assignment.assigned_tier);
println!("  Criteria Met:");
for criterion in assignment.criteria_met.iter() {
    println!("    ✓ {}", criterion);
}

if !assignment.criteria_not_met.is_empty() {
    println!("  Criteria Not Met:");
    for criterion in assignment.criteria_not_met.iter() {
        println!("    ✗ {}", criterion);
    }
}
```

### Tier Benefits

```rust
// Get benefits for a tier
let benefits = ProviderTierManager::get_tier_benefits(&ProviderTier::Gold);

println!("Gold Tier Benefits:");
println!("  Max Signals: {}", benefits.max_signals);
println!("  Fee Discount: {}%", benefits.reduced_fees);
println!("  Priority Support: {}", benefits.priority_support);
println!("  Featured Listing: {}", benefits.featured_listing);
```

### Tier Upgrade Path

**Unverified → Bronze**:
- Complete Basic KYC
- Pass background check

**Bronze → Silver**:
- Upgrade to Enhanced KYC
- Achieve risk score < 50

**Silver → Gold**:
- Upgrade to Full KYC
- Achieve risk score < 20
- Build track record

**Gold → Platinum**:
- Achieve perfect risk score (0)
- Obtain institutional backing
- Demonstrate exceptional performance

---

## Verification Tracking

### Event Types

- `WorkflowStarted`: Verification process initiated
- `StepCompleted`: Individual step finished
- `KYCSubmitted`: KYC documents submitted
- `KYCVerified`: KYC verification completed
- `BackgroundCheckInitiated`: Background check started
- `BackgroundCheckCompleted`: Background check finished
- `TierAssigned`: Provider tier assigned
- `StatusChanged`: Verification status updated
- `DocumentUploaded`: Document uploaded
- `ReviewCompleted`: Manual review completed

### Tracking Events

```rust
// Add verification event
VerificationTrackingManager::add_event(
    &env,
    provider,
    EventType::KYCSubmitted,
    String::from_str(&env, "Enhanced KYC documents submitted"),
    admin_address,
)?;
```

### Viewing Event History

```rust
// Get complete event history
let events = VerificationTrackingManager::get_event_history(
    &env,
    &provider,
);

println!("Verification History:");
for event in events.iter() {
    println!("  [{}] {:?}: {}",
        event.timestamp,
        event.event_type,
        event.details
    );
}
```

---

## Provider Dashboard

### Dashboard Overview

The provider dashboard provides a comprehensive view of verification status, tier information, and performance metrics.

```rust
// Get provider dashboard
let dashboard = ProviderDashboardManager::get_dashboard(
    &env,
    provider,
)?;

println!("Provider Dashboard");
println!("==================");
println!("Tier: {:?}", dashboard.tier);
println!("Verification Status: {:?}", dashboard.verification_status);
println!();
println!("KYC Status:");
println!("  Verified: {}", dashboard.kyc_status.verified);
println!("  Level: {:?}", dashboard.kyc_status.level);
println!("  Expires: {}", dashboard.kyc_status.expires_at);
println!();
println!("Background Check:");
println!("  Completed: {}", dashboard.background_check_status.completed);
println!("  Passed: {}", dashboard.background_check_status.passed);
println!("  Risk Score: {}", dashboard.background_check_status.risk_score);
println!();
println!("Performance:");
println!("  Active Signals: {}", dashboard.active_signals);
println!("  Total Followers: {}", dashboard.total_followers);
println!("  Success Rate: {}%", dashboard.success_rate);
println!();
println!("Tier Benefits:");
println!("  Max Signals: {}", dashboard.tier_benefits.max_signals);
println!("  Fee Discount: {}%", dashboard.tier_benefits.reduced_fees);

if let Some(next_tier) = dashboard.next_tier {
    println!();
    println!("Next Tier: {:?}", next_tier);
    println!("Requirements:");
    for req in dashboard.requirements_for_next_tier.iter() {
        println!("  - {}", req);
    }
}
```

---

## Compliance Requirements

### Regulatory Compliance

The onboarding system ensures compliance with:

1. **KYC/AML Regulations**: Know Your Customer and Anti-Money Laundering
2. **Data Protection**: GDPR, CCPA compliance
3. **Financial Regulations**: Securities and trading regulations
4. **Sanctions Screening**: OFAC and international sanctions lists

### Data Retention

- **KYC Documents**: Retained for 7 years after account closure
- **Background Check Results**: Retained for 5 years
- **Verification Events**: Retained indefinitely for audit trail
- **Personal Data**: Deleted upon request (right to be forgotten)

### Audit Trail

All verification activities are logged and tracked:

```rust
// Get complete audit trail
let tracking = VerificationTrackingManager::get_tracking(&env, &provider)?;

for event in tracking.events.iter() {
    audit_log(&format!(
        "[{}] {} by {} - {}",
        event.timestamp,
        event.event_type,
        event.actor,
        event.details
    ));
}
```

---

## Integration Guide

### Complete Onboarding Flow

```rust
pub fn complete_provider_onboarding(
    env: Env,
    provider: Address,
) -> Result<ProviderDashboard, OnboardingError> {
    // Step 1: Initialize tracking
    VerificationTrackingManager::initialize_tracking(&env, provider.clone());
    
    // Step 2: Create workflow
    let workflow = VerificationWorkflowManager::create_workflow(&env, provider.clone());
    
    // Step 3: Submit KYC
    let kyc_id = KYCIntegrationManager::submit_kyc_verification(
        &env,
        provider.clone(),
        KYCLevel::Enhanced,
        document_hash,
    )?;
    
    VerificationTrackingManager::add_event(
        &env,
        provider.clone(),
        EventType::KYCSubmitted,
        String::from_str(&env, "KYC submitted"),
        provider.clone(),
    )?;
    
    // Step 4: Verify KYC
    let kyc_result = KYCIntegrationManager::verify_kyc(
        &env,
        provider.clone(),
        kyc_id,
    )?;
    
    if !kyc_result.success {
        return Err(OnboardingError::VerificationFailed);
    }
    
    // Step 5: Background check
    let check_id = BackgroundCheckManager::initiate_check(
        &env,
        provider.clone(),
        vec![
            BackgroundCheckType::CriminalRecord,
            BackgroundCheckType::SanctionsList,
            BackgroundCheckType::RegulatoryCheck,
        ],
    )?;
    
    let bg_result = BackgroundCheckManager::complete_check(
        &env,
        provider.clone(),
        check_id,
    )?;
    
    if !bg_result.passed {
        return Err(OnboardingError::VerificationFailed);
    }
    
    // Step 6: Assign tier
    let tier_assignment = ProviderTierManager::assign_tier(&env, provider.clone())?;
    
    VerificationTrackingManager::add_event(
        &env,
        provider.clone(),
        EventType::TierAssigned,
        String::from_str(&env, &format!("Tier: {:?}", tier_assignment.assigned_tier)),
        provider.clone(),
    )?;
    
    // Step 7: Complete workflow
    VerificationWorkflowManager::complete_workflow(&env, workflow.workflow_id, true)?;
    
    // Step 8: Get dashboard
    ProviderDashboardManager::get_dashboard(&env, provider)
}
```

---

## Best Practices

### For Providers

1. **Prepare Documents in Advance**: Have all required documents ready
2. **Use High-Quality Scans**: Clear, legible document images
3. **Provide Accurate Information**: Errors delay verification
4. **Respond Promptly**: Quick responses speed up process
5. **Maintain KYC Validity**: Renew before expiration

### For Platform Operators

1. **Automate Where Possible**: Use automated verification services
2. **Manual Review for Edge Cases**: Human review for complex cases
3. **Regular Compliance Audits**: Ensure ongoing compliance
4. **Clear Communication**: Keep providers informed of status
5. **Secure Data Storage**: Protect sensitive information

---

## Troubleshooting

### Common Issues

**Issue: KYC Verification Failed**
- Check document quality and legibility
- Ensure all required documents submitted
- Verify information matches across documents
- Contact support for specific failure reasons

**Issue: Background Check Flags**
- Review specific flags raised
- Provide additional documentation if needed
- Request manual review for false positives
- Appeal decision if appropriate

**Issue: Tier Assignment Lower Than Expected**
- Review tier criteria
- Check which requirements not met
- Plan upgrade path to next tier
- Improve areas needing attention

**Issue: Verification Stuck in Review**
- Check for pending actions
- Verify all steps completed
- Contact support for status update
- Review event history for issues

---

## Conclusion

The Provider Onboarding and KYC Verification system provides:

✅ Streamlined onboarding workflow
✅ Comprehensive KYC verification
✅ Thorough background checks
✅ Fair tier assignment system
✅ Complete verification tracking
✅ Informative provider dashboard
✅ Full regulatory compliance

For additional support, see:
- [Compliance Documentation](./provider_compliance.md)
- [API Reference](./provider_onboarding_api.md)
- [FAQ](./faq.md)
