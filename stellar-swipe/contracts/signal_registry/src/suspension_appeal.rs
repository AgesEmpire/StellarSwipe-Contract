//! Appeal window for provider suspension decisions (Issue #1420).
//!
//! When a provider is suspended (status transitions to `Suspended`) governance
//! may configure a bounded appeal window during which the provider may submit
//! exactly one appeal.  An authorized reviewer then approves or denies the
//! appeal.  Each appeal resolves exactly once.  After the appeal window closes
//! without resolution the appeal is auto-rejected on the next check.
//!
//! Storage uses `persistent` entries keyed by provider address.  The design
//! is intentionally independent of the community-voting dispute mechanism so
//! the two systems do not share state.

use crate::admin;
use crate::errors::AdminError;
use crate::provider_status::{self, ProviderStatus};
use soroban_sdk::{contracttype, symbol_short, Address, Env};

// ── Policy defaults ───────────────────────────────────────────────────────────

/// Default appeal window: 72 hours.
pub const DEFAULT_APPEAL_WINDOW_SECS: u64 = 72 * 60 * 60;
/// Maximum configurable appeal window: 30 days.
pub const MAX_APPEAL_WINDOW_SECS: u64 = 30 * 24 * 60 * 60;
/// Minimum configurable appeal window: 1 hour.
pub const MIN_APPEAL_WINDOW_SECS: u64 = 60 * 60;

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppealOutcome {
    Approved,
    Denied,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppealState {
    /// No appeal has been submitted by this provider.
    None,
    /// Appeal submitted and awaiting review.
    Pending,
    /// Appeal resolved (see `outcome` field).
    Resolved,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SuspensionAppeal {
    /// The suspended provider who submitted this appeal.
    pub provider: Address,
    /// Ledger timestamp when the appeal was submitted.
    pub submitted_at: u64,
    /// Deadline (timestamp) by which the appeal must be resolved.
    pub deadline: u64,
    /// Current state of the appeal.
    pub state: AppealState,
    /// Populated once the appeal is resolved.
    pub outcome: Option<AppealOutcome>,
    /// Address of the reviewer who resolved the appeal, if resolved.
    pub resolved_by: Option<Address>,
    /// Ledger timestamp of resolution.
    pub resolved_at: Option<u64>,
}

/// Governance-configurable policy for the appeal window.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppealPolicy {
    /// Seconds from suspension during which a provider may submit an appeal.
    pub window_secs: u64,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum AppealKey {
    /// Per-provider appeal record.
    Appeal(Address),
    /// Global appeal policy config.
    Policy,
}

// ── Policy helpers ────────────────────────────────────────────────────────────

/// Store the appeal policy.  `window_secs` must be in
/// `[MIN_APPEAL_WINDOW_SECS, MAX_APPEAL_WINDOW_SECS]`.
pub fn set_policy(env: &Env, policy: &AppealPolicy) -> Result<(), AdminError> {
    if policy.window_secs < MIN_APPEAL_WINDOW_SECS
        || policy.window_secs > MAX_APPEAL_WINDOW_SECS
    {
        return Err(AdminError::InvalidParameter);
    }
    env.storage().instance().set(&AppealKey::Policy, policy);
    Ok(())
}

/// Retrieve the configured appeal policy, falling back to defaults.
pub fn get_policy(env: &Env) -> AppealPolicy {
    env.storage()
        .instance()
        .get(&AppealKey::Policy)
        .unwrap_or(AppealPolicy {
            window_secs: DEFAULT_APPEAL_WINDOW_SECS,
        })
}

// ── Core logic ────────────────────────────────────────────────────────────────

/// Check whether `provider` is currently eligible to submit an appeal.
///
/// Eligibility requires:
/// - Provider's canonical status is `Suspended`.
/// - No prior appeal record exists for this provider (appeals are one-shot).
pub fn is_eligible(env: &Env, provider: &Address) -> bool {
    if provider_status::get_status(env, provider) != ProviderStatus::Suspended {
        return false;
    }
    // Disallow if any prior appeal record exists (regardless of its state).
    env.storage()
        .persistent()
        .get::<_, SuspensionAppeal>(&AppealKey::Appeal(provider.clone()))
        .is_none()
}

/// Submit an appeal for a suspended provider.
///
/// Fails if:
/// - Provider is not `Suspended` (`AdminError::Unauthorized`).
/// - An appeal already exists for this provider (`AdminError::AlreadyApproved` — reused
///   to signal "duplicate").
///
/// The deadline is computed as `now + policy.window_secs`.
pub fn submit_appeal(env: &Env, provider: &Address) -> Result<SuspensionAppeal, AdminError> {
    provider.require_auth();

    if provider_status::get_status(env, provider) != ProviderStatus::Suspended {
        return Err(AdminError::Unauthorized);
    }

    // One appeal per suspension.
    if env
        .storage()
        .persistent()
        .get::<_, SuspensionAppeal>(&AppealKey::Appeal(provider.clone()))
        .is_some()
    {
        return Err(AdminError::AlreadyApproved);
    }

    let now = env.ledger().timestamp();
    let policy = get_policy(env);
    let appeal = SuspensionAppeal {
        provider: provider.clone(),
        submitted_at: now,
        deadline: now.saturating_add(policy.window_secs),
        state: AppealState::Pending,
        outcome: None,
        resolved_by: None,
        resolved_at: None,
    };

    env.storage()
        .persistent()
        .set(&AppealKey::Appeal(provider.clone()), &appeal);

    env.events().publish(
        (symbol_short!("susp_app"), provider.clone()),
        (now, appeal.deadline),
    );

    Ok(appeal)
}

/// Retrieve the appeal record for `provider`, if one exists.
pub fn get_appeal(env: &Env, provider: &Address) -> Option<SuspensionAppeal> {
    env.storage()
        .persistent()
        .get(&AppealKey::Appeal(provider.clone()))
}

/// Resolve an appeal.  Only an admin may call this.
///
/// Fails if:
/// - Caller is not admin (`AdminError::Unauthorized`).
/// - No appeal record exists (`AdminError::ProposalNotFound`).
/// - Appeal is not in `Pending` state (`AdminError::ProposalAlreadyExecuted`).
/// - Appeal deadline has already passed (`AdminError::PauseExpired`).
///
/// On approval the provider's status transitions to `Active`.
/// On denial the provider's status transitions to `Banned`.
/// Either way the appeal is marked `Resolved` and cannot be changed again.
pub fn resolve_appeal(
    env: &Env,
    reviewer: &Address,
    provider: &Address,
    outcome: AppealOutcome,
) -> Result<(), AdminError> {
    admin::require_admin(env, reviewer)?;
    reviewer.require_auth();

    let key = AppealKey::Appeal(provider.clone());
    let mut appeal: SuspensionAppeal = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(AdminError::ProposalNotFound)?;

    if appeal.state != AppealState::Pending {
        return Err(AdminError::ProposalAlreadyExecuted);
    }

    let now = env.ledger().timestamp();
    if now > appeal.deadline {
        // Deadline passed — auto-deny and lock.
        appeal.state = AppealState::Resolved;
        appeal.outcome = Some(AppealOutcome::Denied);
        appeal.resolved_by = Some(reviewer.clone());
        appeal.resolved_at = Some(now);
        env.storage().persistent().set(&key, &appeal);
        // Keep provider Suspended (do not auto-ban; governance can decide).
        env.events().publish(
            (symbol_short!("susp_res"), provider.clone()),
            (false, now),
        );
        return Err(AdminError::PauseExpired);
    }

    // Apply outcome.
    let approved = outcome == AppealOutcome::Approved;
    let new_status = if approved {
        ProviderStatus::Active
    } else {
        ProviderStatus::Banned
    };

    // Perform the status transition through the canonical table.
    provider_status::transition(env, reviewer, provider, new_status)
        .map_err(|_| AdminError::InvalidParameter)?;

    appeal.state = AppealState::Resolved;
    appeal.outcome = Some(outcome);
    appeal.resolved_by = Some(reviewer.clone());
    appeal.resolved_at = Some(now);
    env.storage().persistent().set(&key, &appeal);

    env.events().publish(
        (symbol_short!("susp_res"), provider.clone()),
        (approved, now),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_status::ProviderStatusKey;
    use soroban_sdk::{testutils::Address as _, Env};

    fn make_provider(env: &Env) -> Address {
        Address::generate(env)
    }

    fn set_status(env: &Env, provider: &Address, status: ProviderStatus) {
        env.storage()
            .persistent()
            .set(&ProviderStatusKey::Status(provider.clone()), &status);
    }

    fn setup_admin(env: &Env) -> Address {
        let admin = make_provider(env);
        // Directly write admin so we don't need contract setup.
        env.storage()
            .instance()
            .set(&crate::admin::AdminStorageKey::Admin, &admin);
        admin
    }

    // ── eligibility ──────────────────────────────────────────────────────────

    #[test]
    fn test_active_provider_not_eligible() {
        let env = Env::default();
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Active);
        assert!(!is_eligible(&env, &p));
    }

    #[test]
    fn test_suspended_provider_eligible() {
        let env = Env::default();
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);
        assert!(is_eligible(&env, &p));
    }

    #[test]
    fn test_no_duplicate_appeal() {
        let env = Env::default();
        env.mock_all_auths();
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);

        submit_appeal(&env, &p).unwrap();
        // Second attempt must fail.
        assert_eq!(
            submit_appeal(&env, &p).unwrap_err(),
            AdminError::AlreadyApproved
        );
        // is_eligible must now return false.
        assert!(!is_eligible(&env, &p));
    }

    // ── resolution ───────────────────────────────────────────────────────────

    #[test]
    fn test_approve_appeal_restores_active_status() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = setup_admin(&env);
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);

        submit_appeal(&env, &p).unwrap();
        resolve_appeal(&env, &admin, &p, AppealOutcome::Approved).unwrap();

        let appeal = get_appeal(&env, &p).unwrap();
        assert_eq!(appeal.state, AppealState::Resolved);
        assert_eq!(appeal.outcome, Some(AppealOutcome::Approved));
        assert_eq!(
            provider_status::get_status(&env, &p),
            ProviderStatus::Active
        );
    }

    #[test]
    fn test_deny_appeal_bans_provider() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = setup_admin(&env);
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);

        submit_appeal(&env, &p).unwrap();
        resolve_appeal(&env, &admin, &p, AppealOutcome::Denied).unwrap();

        let appeal = get_appeal(&env, &p).unwrap();
        assert_eq!(appeal.outcome, Some(AppealOutcome::Denied));
        assert_eq!(
            provider_status::get_status(&env, &p),
            ProviderStatus::Banned
        );
    }

    #[test]
    fn test_resolve_twice_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = setup_admin(&env);
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);

        submit_appeal(&env, &p).unwrap();
        resolve_appeal(&env, &admin, &p, AppealOutcome::Approved).unwrap();

        assert_eq!(
            resolve_appeal(&env, &admin, &p, AppealOutcome::Denied).unwrap_err(),
            AdminError::ProposalAlreadyExecuted
        );
    }

    #[test]
    fn test_resolve_no_record_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = setup_admin(&env);
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);

        assert_eq!(
            resolve_appeal(&env, &admin, &p, AppealOutcome::Approved).unwrap_err(),
            AdminError::ProposalNotFound
        );
    }

    #[test]
    fn test_late_resolution_returns_pause_expired() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = setup_admin(&env);
        let p = make_provider(&env);
        set_status(&env, &p, ProviderStatus::Suspended);

        set_policy(
            &env,
            &AppealPolicy {
                window_secs: 3600,
            },
        )
        .unwrap();

        env.ledger().with_mut(|li| li.timestamp = 1_000);
        submit_appeal(&env, &p).unwrap();

        // Advance past the deadline.
        env.ledger().with_mut(|li| li.timestamp = 1_000 + 3601);

        let err = resolve_appeal(&env, &admin, &p, AppealOutcome::Approved).unwrap_err();
        assert_eq!(err, AdminError::PauseExpired);

        // Appeal should be recorded as resolved-denied.
        let appeal = get_appeal(&env, &p).unwrap();
        assert_eq!(appeal.state, AppealState::Resolved);
        assert_eq!(appeal.outcome, Some(AppealOutcome::Denied));
    }

    #[test]
    fn test_policy_validation() {
        let env = Env::default();
        // Too short.
        assert!(set_policy(
            &env,
            &AppealPolicy {
                window_secs: MIN_APPEAL_WINDOW_SECS - 1,
            }
        )
        .is_err());
        // Too long.
        assert!(set_policy(
            &env,
            &AppealPolicy {
                window_secs: MAX_APPEAL_WINDOW_SECS + 1,
            }
        )
        .is_err());
        // Valid.
        assert!(set_policy(
            &env,
            &AppealPolicy {
                window_secs: DEFAULT_APPEAL_WINDOW_SECS,
            }
        )
        .is_ok());
    }
}
