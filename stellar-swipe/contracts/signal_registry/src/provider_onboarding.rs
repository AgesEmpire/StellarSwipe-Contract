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

use soroban_sdk::{contracttype, symbol_short, Address, Env, String, Symbol};

use crate::errors::ProviderOnboardingError as ObErr;

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
    /// Validated onboarding application with reserved collateral / fees
    /// (issues #1017, #1043).
    Application(Address),
    /// Global isolated fund buckets for the onboarding pipeline.
    Reserves,
    /// Governance-configured onboarding parameters.
    Params,
}

// ── Rich onboarding pipeline (issues #1017, #1043) ────────────────────────────
//
// The simple flow above tracks *status only*.  The pipeline below adds strict
// input validation, reserved collateral / fee accounting in **isolated fund
// buckets**, and a refund path for aborted or rejected onboarding — none of
// which mutate registry state unless every validation check passes first.

/// Fallback identifier length limit when governance has not configured one.
pub const DEFAULT_MAX_ID_LEN: u32 = 64;
/// Fallback metadata-URI length limit when governance has not configured one.
pub const DEFAULT_MAX_URI_LEN: u32 = 200;

/// Governance-configured onboarding thresholds.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnboardingParams {
    /// Minimum collateral a provider must post to be considered.
    pub min_collateral: i128,
    /// Minimum non-refundable-on-success onboarding fee.
    pub min_fee: i128,
    /// Maximum provider identifier length (bytes).
    pub max_id_len: u32,
    /// Maximum metadata URI length (bytes).
    pub max_uri_len: u32,
}

/// Lifecycle of a [`ProviderApplication`].
///
/// ```text
/// (none | Failed) ──submit──► PendingReview ──approve──► Active
///                                   │
///                                   ├──reject (governance)──► Failed (+refund)
///                                   └──abort  (provider)────► Failed (+refund)
/// ```
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationStatus {
    /// Submitted and validated; collateral + fee reserved; awaiting review.
    PendingReview,
    /// Approved: collateral converted to an active bond, fee collected.
    Active,
    /// Aborted or rejected: reserved funds refunded. Terminal, but the
    /// provider may submit a fresh application.
    Failed,
}

/// A fully validated onboarding application.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderApplication {
    pub provider: Address,
    pub provider_id: String,
    pub metadata_uri: String,
    pub status: ApplicationStatus,
    /// Collateral reserved while `PendingReview`; the active bond once `Active`;
    /// `0` once `Failed`.
    pub reserved_collateral: i128,
    /// Fee reserved while `PendingReview`; `0` after approval or refund.
    pub reserved_fee: i128,
    /// Total refunded to the provider across the lifetime of this record.
    pub refunded: i128,
    /// How many times this provider has submitted (1 on first submit).
    pub attempts: u32,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Isolated fund buckets for the onboarding pipeline.
///
/// Treasury / operational funds never appear here; every unit of value in a
/// bucket is traceable to a specific provider deposit.  The invariant
///
/// ```text
/// total_deposited
///   == reserved_collateral + reserved_fees + active_bonds
///    + collected_fees + refunded_total
/// ```
///
/// holds after every operation (see [`assert_reserves_invariant`]).
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnboardingReserves {
    /// Collateral held for applications still in `PendingReview`.
    pub reserved_collateral: i128,
    /// Fees held for applications still in `PendingReview`.
    pub reserved_fees: i128,
    /// Collateral locked as an active bond for `Active` providers.
    pub active_bonds: i128,
    /// Fees earned from successfully onboarded providers.
    pub collected_fees: i128,
    /// Lifetime total refunded to providers with failed onboarding.
    pub refunded_total: i128,
    /// Lifetime total collateral + fees taken into the pipeline.
    pub total_deposited: i128,
}

impl OnboardingReserves {
    const fn zero() -> Self {
        OnboardingReserves {
            reserved_collateral: 0,
            reserved_fees: 0,
            active_bonds: 0,
            collected_fees: 0,
            refunded_total: 0,
            total_deposited: 0,
        }
    }
}

// ── Arithmetic ────────────────────────────────────────────────────────────────

fn add(a: i128, b: i128) -> Result<i128, ObErr> {
    a.checked_add(b).ok_or(ObErr::ArithmeticError)
}

fn sub(a: i128, b: i128) -> Result<i128, ObErr> {
    a.checked_sub(b).ok_or(ObErr::ArithmeticError)
}

// ── Pipeline storage helpers ──────────────────────────────────────────────────

fn get_params(env: &Env) -> Option<OnboardingParams> {
    env.storage().persistent().get(&OnboardingKey::Params)
}

fn get_application(env: &Env, provider: &Address) -> Option<ProviderApplication> {
    env.storage()
        .persistent()
        .get(&OnboardingKey::Application(provider.clone()))
}

fn set_application(env: &Env, app: &ProviderApplication) {
    env.storage()
        .persistent()
        .set(&OnboardingKey::Application(app.provider.clone()), app);
}

fn get_reserves(env: &Env) -> OnboardingReserves {
    env.storage()
        .persistent()
        .get(&OnboardingKey::Reserves)
        .unwrap_or(OnboardingReserves::zero())
}

fn set_reserves(env: &Env, reserves: &OnboardingReserves) {
    env.storage()
        .persistent()
        .set(&OnboardingKey::Reserves, reserves);
}

fn emit_pipeline(env: &Env, topic: Symbol, provider: &Address, collateral: i128, fee: i128) {
    env.events()
        .publish((topic, provider.clone()), (collateral, fee));
}

// ── Validation (pure — never touches storage) ─────────────────────────────────

/// Validate an onboarding payload against `params`.
///
/// Returns the first failing check as a [`ProviderOnboardingError`]; performs
/// no state mutation, so a rejected payload leaves the registry untouched.
pub fn validate_application(
    params: &OnboardingParams,
    provider_id: &String,
    metadata_uri: &String,
    collateral: i128,
    fee: i128,
) -> Result<(), ObErr> {
    if provider_id.len() == 0 {
        return Err(ObErr::EmptyProviderId);
    }
    if provider_id.len() > params.max_id_len {
        return Err(ObErr::ProviderIdTooLong);
    }
    if metadata_uri.len() > params.max_uri_len {
        return Err(ObErr::MetadataUriTooLong);
    }
    if collateral <= 0 || fee <= 0 {
        return Err(ObErr::InvalidAmount);
    }
    if collateral < params.min_collateral {
        return Err(ObErr::CollateralBelowMinimum);
    }
    if fee < params.min_fee {
        return Err(ObErr::FeeBelowMinimum);
    }
    Ok(())
}

/// Check the isolation invariant.  Returns `true` when the buckets balance
/// against `total_deposited`.
pub fn reserves_balanced(r: &OnboardingReserves) -> bool {
    let sum = r
        .reserved_collateral
        .checked_add(r.reserved_fees)
        .and_then(|v| v.checked_add(r.active_bonds))
        .and_then(|v| v.checked_add(r.collected_fees))
        .and_then(|v| v.checked_add(r.refunded_total));
    matches!(sum, Some(s) if s == r.total_deposited)
}

// ── Pipeline API ─────────────────────────────────────────────────────────────

/// Governance: configure onboarding thresholds.  Must be called before the
/// first `submit_application`.
pub fn configure_onboarding(
    env: &Env,
    governance: &Address,
    min_collateral: i128,
    min_fee: i128,
    max_id_len: u32,
    max_uri_len: u32,
) -> Result<OnboardingParams, ObErr> {
    governance.require_auth();
    if min_collateral <= 0 || min_fee <= 0 || max_id_len == 0 || max_uri_len == 0 {
        return Err(ObErr::InvalidConfig);
    }
    let params = OnboardingParams {
        min_collateral,
        min_fee,
        max_id_len,
        max_uri_len,
    };
    env.storage()
        .persistent()
        .set(&OnboardingKey::Params, &params);
    Ok(params)
}

/// Read the configured onboarding parameters.
pub fn get_onboarding_params(env: &Env) -> Option<OnboardingParams> {
    get_params(env)
}

/// Provider: submit a validated onboarding application, reserving `collateral`
/// and `fee` into the isolated onboarding buckets.
///
/// Rejects (with no state change) when:
/// * the pipeline is not configured ([`ObErr::NotConfigured`]);
/// * validation fails (see [`validate_application`]);
/// * an application for this provider is already `PendingReview` or `Active`
///   ([`ObErr::DuplicateApplication`]).
///
/// A previously `Failed` application is replaced by the new one (the
/// `attempts` counter carries forward).
pub fn submit_application(
    env: &Env,
    provider: &Address,
    provider_id: String,
    metadata_uri: String,
    collateral: i128,
    fee: i128,
) -> Result<ProviderApplication, ObErr> {
    provider.require_auth();

    let params = get_params(env).ok_or(ObErr::NotConfigured)?;
    validate_application(&params, &provider_id, &metadata_uri, collateral, fee)?;

    let prior_attempts = match get_application(env, provider) {
        Some(existing) => match existing.status {
            ApplicationStatus::PendingReview | ApplicationStatus::Active => {
                return Err(ObErr::DuplicateApplication);
            }
            ApplicationStatus::Failed => existing.attempts,
        },
        None => 0,
    };

    // ── Reserve funds into the isolated buckets ──────────────────────────────
    let mut reserves = get_reserves(env);
    reserves.reserved_collateral = add(reserves.reserved_collateral, collateral)?;
    reserves.reserved_fees = add(reserves.reserved_fees, fee)?;
    reserves.total_deposited = add(reserves.total_deposited, add(collateral, fee)?)?;
    set_reserves(env, &reserves);

    let now = env.ledger().timestamp();
    let app = ProviderApplication {
        provider: provider.clone(),
        provider_id,
        metadata_uri,
        status: ApplicationStatus::PendingReview,
        reserved_collateral: collateral,
        reserved_fee: fee,
        refunded: 0,
        attempts: prior_attempts.saturating_add(1),
        created_at: now,
        updated_at: now,
    };
    set_application(env, &app);

    emit_pipeline(env, symbol_short!("ob_sub"), provider, collateral, fee);
    Ok(app)
}

/// Governance: approve a `PendingReview` application.
///
/// Converts the reserved collateral into an active bond and moves the reserved
/// fee into `collected_fees`.  Emits a registration event.
pub fn approve_application(
    env: &Env,
    governance: &Address,
    provider: &Address,
) -> Result<ProviderApplication, ObErr> {
    governance.require_auth();

    let mut app = get_application(env, provider).ok_or(ObErr::ApplicationNotFound)?;
    match app.status {
        ApplicationStatus::PendingReview => {}
        _ => return Err(ObErr::AlreadyFinalized),
    }

    let mut reserves = get_reserves(env);
    reserves.reserved_collateral = sub(reserves.reserved_collateral, app.reserved_collateral)?;
    reserves.active_bonds = add(reserves.active_bonds, app.reserved_collateral)?;
    reserves.reserved_fees = sub(reserves.reserved_fees, app.reserved_fee)?;
    reserves.collected_fees = add(reserves.collected_fees, app.reserved_fee)?;
    set_reserves(env, &reserves);

    let collected_fee = app.reserved_fee;
    app.status = ApplicationStatus::Active;
    app.reserved_fee = 0;
    app.updated_at = env.ledger().timestamp();
    set_application(env, &app);

    emit_pipeline(
        env,
        symbol_short!("ob_appl"),
        provider,
        app.reserved_collateral,
        collected_fee,
    );
    Ok(app)
}

/// Internal: move a `PendingReview` application to `Failed`, releasing every
/// reserved unit back to the provider.  Shared by `abort_application`
/// (provider-initiated) and `reject_application` (governance-initiated).
fn fail_and_refund(env: &Env, provider: &Address) -> Result<i128, ObErr> {
    let mut app = get_application(env, provider).ok_or(ObErr::ApplicationNotFound)?;
    match app.status {
        ApplicationStatus::PendingReview => {}
        _ => return Err(ObErr::AlreadyFinalized),
    }

    let refund = add(app.reserved_collateral, app.reserved_fee)?;
    if refund <= 0 {
        return Err(ObErr::NothingToRefund);
    }

    let mut reserves = get_reserves(env);
    reserves.reserved_collateral = sub(reserves.reserved_collateral, app.reserved_collateral)?;
    reserves.reserved_fees = sub(reserves.reserved_fees, app.reserved_fee)?;
    reserves.refunded_total = add(reserves.refunded_total, refund)?;
    set_reserves(env, &reserves);

    let released_collateral = app.reserved_collateral;
    let released_fee = app.reserved_fee;
    app.status = ApplicationStatus::Failed;
    app.reserved_collateral = 0;
    app.reserved_fee = 0;
    app.refunded = add(app.refunded, refund)?;
    app.updated_at = env.ledger().timestamp();
    set_application(env, &app);

    emit_pipeline(
        env,
        symbol_short!("ob_rfnd"),
        provider,
        released_collateral,
        released_fee,
    );
    Ok(refund)
}

/// Provider: abort your own `PendingReview` application and reclaim the
/// reserved collateral + fee.  Returns the refunded amount.
pub fn abort_application(env: &Env, provider: &Address) -> Result<i128, ObErr> {
    provider.require_auth();
    fail_and_refund(env, provider)
}

/// Governance: reject a `PendingReview` application; the provider's reserved
/// collateral + fee are refunded.  Returns the refunded amount.
pub fn reject_application(
    env: &Env,
    governance: &Address,
    provider: &Address,
) -> Result<i128, ObErr> {
    governance.require_auth();
    fail_and_refund(env, provider)
}

/// Read the current onboarding application for a provider.
pub fn get_onboarding_application(env: &Env, provider: &Address) -> Option<ProviderApplication> {
    get_application(env, provider)
}

/// Read the isolated onboarding fund buckets.
pub fn get_onboarding_reserves(env: &Env) -> OnboardingReserves {
    get_reserves(env)
}

/// Panic unless the fund-isolation invariant holds.  Intended for tests and
/// invariant assertions at call sites.
pub fn assert_reserves_invariant(env: &Env) {
    assert!(
        reserves_balanced(&get_reserves(env)),
        "onboarding reserves isolation invariant violated"
    );
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
/// The provider must authenticate the call.  Panics if a record already exists
/// (use `request_reverification` to restart from `Rejected`).
pub fn register_provider(env: &Env, provider: &Address) {
    provider.require_auth();
    if get_record(env, provider).is_some() {
        panic!("provider already registered");
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
    use soroban_sdk::{Address, Env};

    /// Per-operation contract-frame harness: soroban-sdk 23 requires a running
    /// contract for host storage access, and each privileged call needs its own
    /// frame so repeated `require_auth` for the same address does not clash.
    struct H {
        env: Env,
        cid: Address,
    }

    impl H {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            #[allow(deprecated)]
            let cid = env.register_contract(None, crate::SignalRegistry);
            H { env, cid }
        }

        fn at<R>(&self, f: impl FnOnce(&Env) -> R) -> R {
            let env = self.env.clone();
            self.env.as_contract(&self.cid, move || f(&env))
        }
    }

    // ── register ──────────────────────────────────────────────────────────

    #[test]
    fn register_sets_pending_status() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        let rec = h.at(|e| get_onboarding_record(e, &provider)).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Pending);
    }

    #[test]
    #[should_panic]
    fn register_twice_panics() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| register_provider(e, &provider));
    }

    // ── approve ───────────────────────────────────────────────────────────

    #[test]
    fn approve_pending_provider_succeeds() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| approve_provider(e, &gov, &provider));
        let rec = h.at(|e| get_onboarding_record(e, &provider)).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Approved);
    }

    #[test]
    #[should_panic]
    fn approve_already_approved_panics() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| approve_provider(e, &gov, &provider));
        h.at(|e| approve_provider(e, &gov, &provider));
    }

    // ── reject ────────────────────────────────────────────────────────────

    #[test]
    fn reject_pending_provider_succeeds() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| reject_provider(e, &gov, &provider));
        let rec = h.at(|e| get_onboarding_record(e, &provider)).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Rejected);
    }

    #[test]
    #[should_panic]
    fn reject_approved_provider_panics() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| approve_provider(e, &gov, &provider));
        h.at(|e| reject_provider(e, &gov, &provider));
    }

    // ── re-verification ───────────────────────────────────────────────────

    #[test]
    fn rejected_provider_can_request_reverification() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| reject_provider(e, &gov, &provider));
        h.at(|e| request_reverification(e, &provider));
        let rec = h.at(|e| get_onboarding_record(e, &provider)).unwrap();
        assert_eq!(rec.status, OnboardingStatus::Pending);
    }

    #[test]
    #[should_panic]
    fn reverification_on_pending_panics() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| request_reverification(e, &provider));
    }

    #[test]
    #[should_panic]
    fn reverification_on_approved_panics() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| register_provider(e, &provider));
        h.at(|e| approve_provider(e, &gov, &provider));
        h.at(|e| request_reverification(e, &provider));
    }

    // ── full lifecycle ────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_register_reject_reverify_approve() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);

        h.at(|e| register_provider(e, &provider));
        assert_eq!(
            h.at(|e| get_onboarding_record(e, &provider))
                .unwrap()
                .status,
            OnboardingStatus::Pending
        );

        h.at(|e| reject_provider(e, &gov, &provider));
        assert_eq!(
            h.at(|e| get_onboarding_record(e, &provider))
                .unwrap()
                .status,
            OnboardingStatus::Rejected
        );

        h.at(|e| request_reverification(e, &provider));
        assert_eq!(
            h.at(|e| get_onboarding_record(e, &provider))
                .unwrap()
                .status,
            OnboardingStatus::Pending
        );

        h.at(|e| approve_provider(e, &gov, &provider));
        assert_eq!(
            h.at(|e| get_onboarding_record(e, &provider))
                .unwrap()
                .status,
            OnboardingStatus::Approved
        );
    }

    // ── unknown provider ──────────────────────────────────────────────────

    #[test]
    #[should_panic]
    fn approve_unknown_provider_panics() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        let gov = Address::generate(&h.env);
        h.at(|e| approve_provider(e, &gov, &provider));
    }

    #[test]
    fn get_record_returns_none_for_unknown_provider() {
        let h = H::new();
        let provider = Address::generate(&h.env);
        assert!(h.at(|e| get_onboarding_record(e, &provider)).is_none());
    }
}

// ── Pipeline tests (issues #1017, #1043) ─────────────────────────────────────

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, String};

    const MIN_COLLATERAL: i128 = 1_000;
    const MIN_FEE: i128 = 100;

    struct H {
        env: Env,
        cid: Address,
        gov: Address,
        provider: Address,
    }

    impl H {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            #[allow(deprecated)]
            let cid = env.register_contract(None, crate::SignalRegistry);
            let gov = Address::generate(&env);
            let provider = Address::generate(&env);
            let h = H {
                env,
                cid,
                gov,
                provider,
            };
            let gov2 = h.gov.clone();
            h.at(move |e| {
                configure_onboarding(
                    e,
                    &gov2,
                    MIN_COLLATERAL,
                    MIN_FEE,
                    DEFAULT_MAX_ID_LEN,
                    DEFAULT_MAX_URI_LEN,
                )
                .unwrap();
            });
            h
        }

        /// Like [`H::new`] but without configuring the pipeline.
        fn unconfigured() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            #[allow(deprecated)]
            let cid = env.register_contract(None, crate::SignalRegistry);
            let gov = Address::generate(&env);
            let provider = Address::generate(&env);
            H {
                env,
                cid,
                gov,
                provider,
            }
        }

        fn at<R>(&self, f: impl FnOnce(&Env) -> R) -> R {
            let env = self.env.clone();
            self.env.as_contract(&self.cid, move || f(&env))
        }

        fn id(&self) -> String {
            String::from_str(&self.env, "provider-alpha")
        }

        fn uri(&self) -> String {
            String::from_str(&self.env, "ipfs://QmProviderMetadata")
        }

        fn submit(
            &self,
            provider: &Address,
            collateral: i128,
            fee: i128,
        ) -> Result<ProviderApplication, ObErr> {
            let (p, id, uri) = (provider.clone(), self.id(), self.uri());
            self.at(move |e| submit_application(e, &p, id, uri, collateral, fee))
        }

        fn reserves(&self) -> OnboardingReserves {
            self.at(|e| get_onboarding_reserves(e))
        }

        fn application(&self, provider: &Address) -> Option<ProviderApplication> {
            let p = provider.clone();
            self.at(move |e| get_onboarding_application(e, &p))
        }

        fn invariant(&self) {
            self.at(|e| assert_reserves_invariant(e));
        }
    }

    // ── configuration ────────────────────────────────────────────────────────

    #[test]
    fn submit_before_configure_is_rejected() {
        let h = H::unconfigured();
        let res = h.submit(&h.provider.clone(), 1_000, 100);
        assert_eq!(res, Err(ObErr::NotConfigured));
        assert!(h.application(&h.provider).is_none());
    }

    #[test]
    fn configure_rejects_bad_params() {
        let h = H::unconfigured();
        let g1 = h.gov.clone();
        assert_eq!(
            h.at(move |e| configure_onboarding(e, &g1, 0, 100, 64, 200)),
            Err(ObErr::InvalidConfig)
        );
        let g2 = h.gov.clone();
        assert_eq!(
            h.at(move |e| configure_onboarding(e, &g2, 100, 100, 0, 200)),
            Err(ObErr::InvalidConfig)
        );
    }

    // ── validation (#1017) ───────────────────────────────────────────────────

    #[test]
    fn rejects_empty_provider_id_without_state_change() {
        let h = H::new();
        let (p, uri) = (h.provider.clone(), h.uri());
        let empty = String::from_str(&h.env, "");
        let res = h.at(move |e| submit_application(e, &p, empty, uri, 1_000, 100));
        assert_eq!(res, Err(ObErr::EmptyProviderId));
        assert!(h.application(&h.provider).is_none());
        assert_eq!(h.reserves(), OnboardingReserves::zero());
    }

    #[test]
    fn rejects_oversized_provider_id() {
        let h = H::new();
        let (p, uri) = (h.provider.clone(), h.uri());
        let long = String::from_str(
            &h.env,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let res = h.at(move |e| submit_application(e, &p, long, uri, 1_000, 100));
        assert_eq!(res, Err(ObErr::ProviderIdTooLong));
    }

    #[test]
    fn rejects_collateral_below_minimum() {
        let h = H::new();
        assert_eq!(
            h.submit(&h.provider.clone(), 999, 100),
            Err(ObErr::CollateralBelowMinimum)
        );
    }

    #[test]
    fn rejects_fee_below_minimum() {
        let h = H::new();
        assert_eq!(
            h.submit(&h.provider.clone(), 1_000, 99),
            Err(ObErr::FeeBelowMinimum)
        );
    }

    #[test]
    fn rejects_non_positive_amounts() {
        let h = H::new();
        assert_eq!(
            h.submit(&h.provider.clone(), 0, 100),
            Err(ObErr::InvalidAmount)
        );
        assert_eq!(
            h.submit(&h.provider.clone(), 1_000, -1),
            Err(ObErr::InvalidAmount)
        );
    }

    // ── happy path + reservation ─────────────────────────────────────────────

    #[test]
    fn submit_reserves_into_isolated_buckets() {
        let h = H::new();
        let app = h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        assert_eq!(app.status, ApplicationStatus::PendingReview);
        assert_eq!(app.attempts, 1);

        let r = h.reserves();
        assert_eq!(r.reserved_collateral, 5_000);
        assert_eq!(r.reserved_fees, 250);
        assert_eq!(r.active_bonds, 0);
        assert_eq!(r.collected_fees, 0);
        assert_eq!(r.total_deposited, 5_250);
        h.invariant();
    }

    #[test]
    fn approve_moves_funds_to_bond_and_collected_fee() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let (g, p) = (h.gov.clone(), h.provider.clone());
        let app = h.at(move |e| approve_application(e, &g, &p)).unwrap();
        assert_eq!(app.status, ApplicationStatus::Active);

        let r = h.reserves();
        assert_eq!(r.reserved_collateral, 0);
        assert_eq!(r.reserved_fees, 0);
        assert_eq!(r.active_bonds, 5_000);
        assert_eq!(r.collected_fees, 250);
        assert_eq!(r.refunded_total, 0);
        h.invariant();
    }

    // ── duplicate rejection (#1017) ──────────────────────────────────────────

    #[test]
    fn duplicate_pending_application_is_rejected_without_mutation() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let before = h.reserves();
        assert_eq!(
            h.submit(&h.provider.clone(), 9_999, 999),
            Err(ObErr::DuplicateApplication)
        );
        assert_eq!(h.reserves(), before);
    }

    #[test]
    fn duplicate_active_application_is_rejected() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let (g, p) = (h.gov.clone(), h.provider.clone());
        h.at(move |e| approve_application(e, &g, &p)).unwrap();
        assert_eq!(
            h.submit(&h.provider.clone(), 5_000, 250),
            Err(ObErr::DuplicateApplication)
        );
    }

    // ── refund path (#1043) ──────────────────────────────────────────────────

    #[test]
    fn provider_can_abort_and_be_refunded() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let p = h.provider.clone();
        let refund = h.at(move |e| abort_application(e, &p)).unwrap();
        assert_eq!(refund, 5_250);

        let app = h.application(&h.provider).unwrap();
        assert_eq!(app.status, ApplicationStatus::Failed);
        assert_eq!(app.reserved_collateral, 0);
        assert_eq!(app.reserved_fee, 0);
        assert_eq!(app.refunded, 5_250);

        let r = h.reserves();
        assert_eq!(r.reserved_collateral, 0);
        assert_eq!(r.reserved_fees, 0);
        assert_eq!(r.refunded_total, 5_250);
        h.invariant();
    }

    #[test]
    fn governance_can_reject_and_provider_is_refunded() {
        let h = H::new();
        h.submit(&h.provider.clone(), 3_000, 150).unwrap();
        let (g, p) = (h.gov.clone(), h.provider.clone());
        let refund = h.at(move |e| reject_application(e, &g, &p)).unwrap();
        assert_eq!(refund, 3_150);
        assert_eq!(
            h.application(&h.provider).unwrap().status,
            ApplicationStatus::Failed
        );
        h.invariant();
    }

    #[test]
    fn abort_after_approval_is_rejected() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let (g, p) = (h.gov.clone(), h.provider.clone());
        h.at(move |e| approve_application(e, &g, &p)).unwrap();
        let p2 = h.provider.clone();
        assert_eq!(
            h.at(move |e| abort_application(e, &p2)),
            Err(ObErr::AlreadyFinalized)
        );
    }

    #[test]
    fn double_abort_is_rejected() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let p = h.provider.clone();
        h.at(move |e| abort_application(e, &p)).unwrap();
        let p2 = h.provider.clone();
        assert_eq!(
            h.at(move |e| abort_application(e, &p2)),
            Err(ObErr::AlreadyFinalized)
        );
    }

    #[test]
    fn abort_unknown_provider_is_rejected() {
        let h = H::new();
        let p = h.provider.clone();
        assert_eq!(
            h.at(move |e| abort_application(e, &p)),
            Err(ObErr::ApplicationNotFound)
        );
    }

    // ── repeated onboarding attempts (#1043) ─────────────────────────────────

    #[test]
    fn provider_can_resubmit_after_failed_onboarding() {
        let h = H::new();
        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        let p = h.provider.clone();
        h.at(move |e| abort_application(e, &p)).unwrap();

        let app2 = h.submit(&h.provider.clone(), 2_000, 120).unwrap();
        assert_eq!(app2.status, ApplicationStatus::PendingReview);
        assert_eq!(app2.attempts, 2);
        assert_eq!(app2.refunded, 0);

        let r = h.reserves();
        assert_eq!(r.reserved_collateral, 2_000);
        assert_eq!(r.reserved_fees, 120);
        assert_eq!(r.refunded_total, 5_250);
        assert_eq!(r.total_deposited, 5_250 + 2_120);
        h.invariant();

        let (g, p) = (h.gov.clone(), h.provider.clone());
        h.at(move |e| approve_application(e, &g, &p)).unwrap();
        h.invariant();
    }

    #[test]
    fn multiple_providers_keep_independent_reserves() {
        let h = H::new();
        let p2 = Address::generate(&h.env);
        let p3 = Address::generate(&h.env);

        h.submit(&h.provider.clone(), 5_000, 250).unwrap();
        {
            let (p, uri) = (p2.clone(), h.uri());
            let idp = String::from_str(&h.env, "prov-2");
            h.at(move |e| submit_application(e, &p, idp, uri, 4_000, 200))
                .unwrap();
        }
        {
            let (p, uri) = (p3.clone(), h.uri());
            let idp = String::from_str(&h.env, "prov-3");
            h.at(move |e| submit_application(e, &p, idp, uri, 3_000, 150))
                .unwrap();
        }

        let (g, p) = (h.gov.clone(), h.provider.clone());
        h.at(move |e| approve_application(e, &g, &p)).unwrap();
        let (g, p) = (h.gov.clone(), p2.clone());
        h.at(move |e| reject_application(e, &g, &p)).unwrap();

        let r = h.reserves();
        assert_eq!(r.active_bonds, 5_000);
        assert_eq!(r.collected_fees, 250);
        assert_eq!(r.refunded_total, 4_200);
        assert_eq!(r.reserved_collateral, 3_000);
        assert_eq!(r.reserved_fees, 150);
        assert_eq!(r.total_deposited, 5_250 + 4_200 + 3_150);
        h.invariant();
    }

    #[test]
    fn approve_unknown_application_is_rejected() {
        let h = H::new();
        let (g, p) = (h.gov.clone(), h.provider.clone());
        assert_eq!(
            h.at(move |e| approve_application(e, &g, &p)),
            Err(ObErr::ApplicationNotFound)
        );
    }

    #[test]
    fn validate_application_is_pure() {
        let env = Env::default();
        let params = OnboardingParams {
            min_collateral: 1_000,
            min_fee: 100,
            max_id_len: 64,
            max_uri_len: 200,
        };
        assert_eq!(
            validate_application(
                &params,
                &String::from_str(&env, "ok"),
                &String::from_str(&env, ""),
                1_000,
                100
            ),
            Ok(())
        );
        assert_eq!(
            validate_application(
                &params,
                &String::from_str(&env, ""),
                &String::from_str(&env, ""),
                1_000,
                100
            ),
            Err(ObErr::EmptyProviderId)
        );
    }
}
