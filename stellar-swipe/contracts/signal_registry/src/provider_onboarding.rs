//! On-chain provider onboarding and compliance verification flow.
//!
//! This module records the full lifecycle of a provider's onboarding status so
//! that governance can approve or reject providers in an auditable way.
//!
//! ## States
//!
//! ```text
//! (none) ──register──► Pending ──approve──► Approved
//!                                │
//!                                └──reject──► Rejected ──re_verify──► Pending
//! ```
//!
//! ## Storage
//!
//! Each provider's onboarding record is stored under a persistent key keyed by
//! `Address`.  All state transitions emit events for off-chain indexing.

#![allow(dead_code)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

// ── Types ─────────────────────────────────────────────────────────────────────

/// On-chain compliance / onboarding state for a provider.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnboardingStatus {
    /// Registration submitted; awaiting governance review.
    Pending,
    /// Governance approved the provider.
    Approved,
    /// Governance rejected the provider.  The provider may request
    /// re-verification by calling `request_reverification`.
    Rejected,
}

/// Full onboarding record stored on-chain.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnboardingRecord {
    pub provider: Address,
    pub status: OnboardingStatus,
    /// Ledger timestamp of the most recent state change.
    pub updated_at: u64,
}

/// Persistent storage key for onboarding records.
#[contracttype]
#[derive(Clone)]
pub enum OnboardingKey {
    Record(Address),
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn get_record(env: &Env, provider: &Address) -> Option<OnboardingRecord> {
    env.storage()
        .persistent()
        .get(&OnboardingKey::Record(provider.clone()))
}

fn set_record(env: &Env, record: &OnboardingRecord) {
    env.storage()
        .persistent()
        .set(&OnboardingKey::Record(record.provider.clone()), record);
}

// ── Events ────────────────────────────────────────────────────────────────────

fn emit(env: &Env, topic: Symbol, provider: &Address, status: &OnboardingStatus) {
    env.events()
        .publish((topic, provider.clone()), status.clone());
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Register a provider for onboarding review.
///
/// The provider must authenticate the call.
///
/// Registration is **idempotent** (issue #1025): a retry after a partial
/// submission or network hiccup does not create a second record or re-emit the
/// registration event.  If a record already exists — in any state — the current
/// [`OnboardingRecord`] is returned unchanged and no storage mutation occurs.
/// A rejected provider that wants a fresh review must call
/// [`request_reverification`] rather than re-registering.
pub fn register_provider(env: &Env, provider: &Address) -> OnboardingRecord {
    provider.require_auth();

    // Idempotent retry: the registration already succeeded — return the
    // existing state instead of mutating it or emitting a duplicate event.
    if let Some(existing) = get_record(env, provider) {
        return existing;
    }

    let record = OnboardingRecord {
        provider: provider.clone(),
        status: OnboardingStatus::Pending,
        updated_at: env.ledger().timestamp(),
    };
    set_record(env, &record);
    emit(
        env,
        symbol_short!("ob_reg"),
        provider,
        &OnboardingStatus::Pending,
    );
    record
}

/// Governance: approve a `Pending` provider.
///
/// `governance` must authenticate the call.
pub fn approve_provider(env: &Env, governance: &Address, provider: &Address) {
    governance.require_auth();
    let mut record = get_record(env, provider).expect("provider not found");
    if record.status != OnboardingStatus::Pending {
        panic!("provider is not pending");
    }
    record.status = OnboardingStatus::Approved;
    record.updated_at = env.ledger().timestamp();
    set_record(env, &record);
    emit(
        env,
        symbol_short!("ob_appr"),
        provider,
        &OnboardingStatus::Approved,
    );
}

/// Governance: reject a `Pending` provider.
///
/// `governance` must authenticate the call.
pub fn reject_provider(env: &Env, governance: &Address, provider: &Address) {
    governance.require_auth();
    let mut record = get_record(env, provider).expect("provider not found");
    if record.status != OnboardingStatus::Pending {
        panic!("provider is not pending");
    }
    record.status = OnboardingStatus::Rejected;
    record.updated_at = env.ledger().timestamp();
    set_record(env, &record);
    emit(
        env,
        symbol_short!("ob_rej"),
        provider,
        &OnboardingStatus::Rejected,
    );
}

/// Provider: request re-verification after being rejected.
///
/// The provider must authenticate the call.  Moves status back to `Pending`.
pub fn request_reverification(env: &Env, provider: &Address) {
    provider.require_auth();
    let mut record = get_record(env, provider).expect("provider not found");
    if record.status != OnboardingStatus::Rejected {
        panic!("provider is not rejected");
    }
    record.status = OnboardingStatus::Pending;
    record.updated_at = env.ledger().timestamp();
    set_record(env, &record);
    emit(
        env,
        symbol_short!("ob_rverf"),
        provider,
        &OnboardingStatus::Pending,
    );
}

/// Read the current onboarding record for a provider.
pub fn get_onboarding_record(env: &Env, provider: &Address) -> Option<OnboardingRecord> {
    get_record(env, provider)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    fn env() -> Env {
        Env::default()
    }

    /// Fresh env + registered contract id + a provider / governance address.
    ///
    /// Each onboarding call is then run in its own `as_contract` frame (see
    /// [`in_frame`]) to mirror a real, separately-authorized invocation — the
    /// storage helpers require a contract frame on the stack and `require_auth`
    /// cannot be replayed twice inside one frame.
    fn setup() -> (Env, Address, Address, Address) {
        let e = Env::default();
        e.mock_all_auths_allowing_non_root_auth();
        let contract_id = e.register_contract(None, crate::SignalRegistry);
        let provider = Address::generate(&e);
        let governance = Address::generate(&e);
        (e, contract_id, provider, governance)
    }

    /// Run one onboarding operation in its own contract frame.
    fn in_frame<T>(e: &Env, contract_id: &Address, f: impl FnOnce() -> T) -> T {
        e.as_contract(contract_id, f)
    }

    // ── register ──────────────────────────────────────────────────────────

    #[test]
    fn register_sets_pending_status() {
        let e = env();
        let provider = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        let rec = get_onboarding_record(&e, &provider).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Pending);
    }

    #[test]
    fn register_returns_the_created_record() {
        let (e, cid, provider, _gov) = setup();
        let rec = in_frame(&e, &cid, || register_provider(&e, &provider));
        assert_eq!(rec.provider, provider);
        assert_eq!(rec.status, OnboardingStatus::Pending);
    }

    // ── idempotent retry (issue #1025) ────────────────────────────────────

    #[test]
    fn register_twice_is_idempotent_and_returns_existing_state() {
        let (e, cid, provider, _gov) = setup();

        let first = in_frame(&e, &cid, || register_provider(&e, &provider));
        let second = in_frame(&e, &cid, || register_provider(&e, &provider));

        assert_eq!(first, second);
        assert_eq!(second.updated_at, first.updated_at);
        assert_eq!(second.status, OnboardingStatus::Pending);
        assert_eq!(
            in_frame(&e, &cid, || get_onboarding_record(&e, &provider)).unwrap(),
            first
        );
    }

    #[test]
    fn retry_does_not_mutate_stored_state() {
        let (e, cid, provider, _gov) = setup();

        let first = in_frame(&e, &cid, || register_provider(&e, &provider));
        for _ in 0..3 {
            in_frame(&e, &cid, || register_provider(&e, &provider));
        }
        let stored = in_frame(&e, &cid, || get_onboarding_record(&e, &provider)).unwrap();

        // Storage is byte-for-byte the record created by the first call: no
        // duplicate record, no refreshed timestamp, no status change.
        assert_eq!(stored, first);
    }

    #[test]
    fn retry_after_approval_returns_approved_without_reverting_to_pending() {
        let (e, cid, provider, gov) = setup();

        in_frame(&e, &cid, || register_provider(&e, &provider));
        in_frame(&e, &cid, || approve_provider(&e, &gov, &provider));

        let retried = in_frame(&e, &cid, || register_provider(&e, &provider));
        assert_eq!(retried.status, OnboardingStatus::Approved);
        assert_eq!(
            in_frame(&e, &cid, || get_onboarding_record(&e, &provider))
                .unwrap()
                .status,
            OnboardingStatus::Approved
        );
    }

    #[test]
    fn retry_after_rejection_returns_rejected_state() {
        let (e, cid, provider, gov) = setup();

        in_frame(&e, &cid, || register_provider(&e, &provider));
        in_frame(&e, &cid, || reject_provider(&e, &gov, &provider));

        let retried = in_frame(&e, &cid, || register_provider(&e, &provider));
        assert_eq!(retried.status, OnboardingStatus::Rejected);
    }

    // ── approve ───────────────────────────────────────────────────────────

    #[test]
    fn approve_pending_provider_succeeds() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        approve_provider(&e, &gov, &provider);
        let rec = get_onboarding_record(&e, &provider).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Approved);
    }

    #[test]
    #[should_panic(expected = "provider is not pending")]
    fn approve_already_approved_panics() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        approve_provider(&e, &gov, &provider);
        approve_provider(&e, &gov, &provider);
    }

    // ── reject ────────────────────────────────────────────────────────────

    #[test]
    fn reject_pending_provider_succeeds() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        reject_provider(&e, &gov, &provider);
        let rec = get_onboarding_record(&e, &provider).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Rejected);
    }

    #[test]
    #[should_panic(expected = "provider is not pending")]
    fn reject_approved_provider_panics() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        approve_provider(&e, &gov, &provider);
        reject_provider(&e, &gov, &provider);
    }

    // ── re-verification ───────────────────────────────────────────────────

    #[test]
    fn rejected_provider_can_request_reverification() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        reject_provider(&e, &gov, &provider);
        request_reverification(&e, &provider);
        let rec = get_onboarding_record(&e, &provider).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Pending);
    }

    #[test]
    #[should_panic(expected = "provider is not rejected")]
    fn reverification_on_pending_panics() {
        let e = env();
        let provider = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        request_reverification(&e, &provider);
    }

    #[test]
    #[should_panic(expected = "provider is not rejected")]
    fn reverification_on_approved_panics() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        register_provider(&e, &provider);
        approve_provider(&e, &gov, &provider);
        request_reverification(&e, &provider);
    }

    // ── full lifecycle ────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_register_reject_reverify_approve() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();

        register_provider(&e, &provider);
        assert_eq!(
            get_onboarding_record(&e, &provider).unwrap().status,
            OnboardingStatus::Pending
        );

        reject_provider(&e, &gov, &provider);
        assert_eq!(
            get_onboarding_record(&e, &provider).unwrap().status,
            OnboardingStatus::Rejected
        );

        request_reverification(&e, &provider);
        assert_eq!(
            get_onboarding_record(&e, &provider).unwrap().status,
            OnboardingStatus::Pending
        );

        approve_provider(&e, &gov, &provider);
        assert_eq!(
            get_onboarding_record(&e, &provider).unwrap().status,
            OnboardingStatus::Approved
        );
    }

    // ── unknown provider ──────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "provider not found")]
    fn approve_unknown_provider_panics() {
        let e = env();
        let provider = Address::generate(&e);
        let gov = Address::generate(&e);
        e.mock_all_auths();
        approve_provider(&e, &gov, &provider);
    }

    #[test]
    fn get_record_returns_none_for_unknown_provider() {
        let e = env();
        let provider = Address::generate(&e);
        assert!(get_onboarding_record(&e, &provider).is_none());
    }
}
