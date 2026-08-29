use soroban_sdk::{contracttype, Address, Bytes, Env, IntoVal, Map, String, Symbol, Val, Vec};

use crate::events;
use crate::types::{ProviderPerformance, Signal, SignalStatus};

/// Storage key for the banned providers map
#[contracttype]
#[derive(Clone)]
pub enum BanStorageKey {
    /// (provider) -> reason_hash; presence of key indicates banned status
    ProviderBanReason(Address),
}

pub const GOLD_TIER_STAKE: i128 = 1_000_000_000;
pub const MIN_CLOSED_SIGNALS: u32 = 20;
pub const MIN_SUCCESS_RATE_BPS: u32 = 6_000;

// ─── Provider specialization tags (Issue #704) ───────────────────────────────

/// Maximum number of specialization tags a single provider may select.
pub const MAX_SPECIALIZATION_TAGS_PER_PROVIDER: u32 = 3;

/// Maximum length of a specialization tag string (in bytes).
pub const MAX_SPECIALIZATION_TAG_LENGTH: u32 = 32;

/// Maximum number of admin-defined specialization tags.
pub const MAX_ADMIN_SPECIALIZATION_TAGS: u32 = 50;

/// Storage key for provider specialization tags.
#[contracttype]
#[derive(Clone)]
pub enum SpecializationStorageKey {
    /// Admin-defined set of valid specialization tags.
    SpecializationTags,
    /// Per-provider: the list of specialization tags they self-selected.
    /// Stored as `Vec<String>`.
    ProviderSpecializations(Address),
    /// Reverse index: tag -> Vec<Address> of providers with that tag.
    ProvidersBySpecialization(soroban_sdk::String),
}

/// Add a specialization tag to the admin-defined set.
pub fn add_specialization_tag(env: &Env, admin: &Address, tag: String) -> Result<(), ()> {
    admin.require_auth();
    if tag.len() == 0 || tag.len() > MAX_SPECIALIZATION_TAG_LENGTH {
        panic!("tag length out of bounds");
    }

    let mut tags: soroban_sdk::Vec<String> = env
        .storage()
        .instance()
        .get(&SpecializationStorageKey::SpecializationTags)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));

    for i in 0..tags.len() {
        if let Some(t) = tags.get(i) {
            if t == tag {
                return Ok(());
            }
        }
    }

    if tags.len() >= MAX_ADMIN_SPECIALIZATION_TAGS {
        panic!("max specialization tags reached");
    }

    tags.push_back(tag);
    env.storage()
        .instance()
        .set(&SpecializationStorageKey::SpecializationTags, &tags);
    Ok(())
}

/// Remove a specialization tag from the admin-defined set.
pub fn remove_specialization_tag(env: &Env, admin: &Address, tag: String) {
    admin.require_auth();
    let mut tags: soroban_sdk::Vec<String> = env
        .storage()
        .instance()
        .get(&SpecializationStorageKey::SpecializationTags)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));

    let mut updated = soroban_sdk::Vec::new(env);
    for i in 0..tags.len() {
        if let Some(t) = tags.get(i) {
            if t != tag {
                updated.push_back(t);
            }
        }
    }
    env.storage()
        .instance()
        .set(&SpecializationStorageKey::SpecializationTags, &updated);
}

/// Returns the admin-defined set of specialization tags.
pub fn get_specialization_tags(env: &Env) -> soroban_sdk::Vec<String> {
    env.storage()
        .instance()
        .get(&SpecializationStorageKey::SpecializationTags)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

// ─── Provider Profile (Task 3) ────────────────────────────────────────────────

/// On-chain provider profile. Content (display name, bio) is stored off-chain;
/// only their hashes are stored here.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderProfile {
    /// Hash of the provider's display name (off-chain content).
    pub display_name_hash: String,
    /// Hash of the provider's bio (off-chain content).
    pub bio_hash: String,
    /// Ledger timestamp when the profile was created.
    pub created_at: u64,
    /// Total signals submitted (mirrored from ProviderPerformance for quick reads).
    pub total_signals: u32,
    /// Success rate in basis points (0–10_000).
    pub success_rate: u32,
    /// Reputation score (0–100).
    pub reputation_score: u32,
    /// Stake tier: 0 = none, 1 = bronze, 2 = silver, 3 = gold.
    pub stake_tier: u32,
    /// Whether the provider has passed verification.
    pub verified: bool,
}

/// Storage key for provider profiles.
#[contracttype]
#[derive(Clone)]
pub enum ProviderStorageKey {
    Profile(Address),
    BanAppeal(Address),
}

/// Create or update a provider profile.
///
/// - On first call (no existing profile): creates a new profile with `created_at = now`.
/// - On subsequent calls: updates `display_name_hash` and `bio_hash` only.
/// - `total_signals`, `success_rate`, `reputation_score`, `stake_tier`, and `verified`
///   are derived from `stats` and `stake` on every call so the profile stays in sync.
pub fn create_or_update_provider_profile(
    env: &Env,
    provider: Address,
    display_name_hash: String,
    bio_hash: String,
    stats: &ProviderPerformance,
    stake: i128,
    verified: bool,
) -> ProviderProfile {
    let key = ProviderStorageKey::Profile(provider.clone());

    let created_at = env
        .storage()
        .persistent()
        .get::<_, ProviderProfile>(&key)
        .map(|p| p.created_at)
        .unwrap_or_else(|| env.ledger().timestamp());

    let stake_tier = if stake >= GOLD_TIER_STAKE {
        3
    } else if stake >= GOLD_TIER_STAKE / 2 {
        2
    } else if stake >= GOLD_TIER_STAKE / 10 {
        1
    } else {
        0
    };

    let profile = ProviderProfile {
        display_name_hash,
        bio_hash,
        created_at,
        total_signals: stats.total_signals,
        success_rate: stats.success_rate,
        reputation_score: (stats.success_rate / 100).min(100),
        stake_tier,
        verified,
    };

    env.storage().persistent().set(&key, &profile);

    let topics = (Symbol::new(env, "provider_profile_updated"),);
    env.events().publish(topics, provider);

    profile
}

/// Read a provider profile. Returns `None` if no profile exists.
pub fn get_provider_profile(env: &Env, provider: &Address) -> Option<ProviderProfile> {
    env.storage()
        .persistent()
        .get(&ProviderStorageKey::Profile(provider.clone()))
}

// ─── Provider Appeal Mechanism (Task 4) ──────────────────────────────────────

/// Status of a ban appeal.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppealStatus {
    Pending,
    Approved,
    Rejected,
}

/// On-chain ban appeal record.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BanAppeal {
    pub provider: Address,
    /// IPFS hash (or similar) of the evidence document.
    pub evidence_hash: Bytes,
    /// Governance proposal ID created for this appeal.
    pub governance_proposal_id: u64,
    /// Current status of the appeal.
    pub status: AppealStatus,
    /// Ledger timestamp when the appeal was submitted.
    pub submitted_at: u64,
}

/// Submit a ban appeal for `provider`.
///
/// Creates a governance proposal (via `create_governance_proposal_fn`) and stores
/// the appeal record. Emits `BanAppealSubmitted`.
///
/// `create_governance_proposal_fn` is injected so this module stays decoupled from
/// the governance contract. In production, pass a closure that calls the governance
/// contract; in tests, pass a stub.
pub fn submit_ban_appeal<F>(
    env: &Env,
    provider: Address,
    evidence_hash: Bytes,
    create_governance_proposal_fn: F,
) -> Result<BanAppeal, AppealError>
where
    F: Fn(&Env, &Address, &Bytes) -> Result<u64, AppealError>,
{
    // Prevent duplicate pending appeals.
    let key = ProviderStorageKey::BanAppeal(provider.clone());
    if let Some(existing) = env.storage().persistent().get::<_, BanAppeal>(&key) {
        if existing.status == AppealStatus::Pending {
            return Err(AppealError::AppealAlreadyPending);
        }
    }

    let proposal_id = create_governance_proposal_fn(env, &provider, &evidence_hash)?;

    let appeal = BanAppeal {
        provider: provider.clone(),
        evidence_hash,
        governance_proposal_id: proposal_id,
        status: AppealStatus::Pending,
        submitted_at: env.ledger().timestamp(),
    };

    env.storage().persistent().set(&key, &appeal);

    let topics = (Symbol::new(env, "ban_appeal_submitted"),);
    env.events().publish(topics, (provider, proposal_id));

    Ok(appeal)
}

/// Governance calls this to reverse a ban (approve the appeal).
///
/// Restores the provider's `verified` flag in their profile and emits `BanReversed`.
/// `return_stake_fn` is injected to handle stake return logic.
pub fn reverse_ban<F>(env: &Env, provider: Address, return_stake_fn: F) -> Result<(), AppealError>
where
    F: Fn(&Env, &Address) -> Result<(), AppealError>,
{
    let key = ProviderStorageKey::BanAppeal(provider.clone());
    let mut appeal: BanAppeal = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(AppealError::AppealNotFound)?;

    if appeal.status != AppealStatus::Pending {
        return Err(AppealError::AppealAlreadyResolved);
    }

    appeal.status = AppealStatus::Approved;
    env.storage().persistent().set(&key, &appeal);

    // Restore verified flag in profile if it exists.
    let profile_key = ProviderStorageKey::Profile(provider.clone());
    if let Some(mut profile) = env
        .storage()
        .persistent()
        .get::<_, ProviderProfile>(&profile_key)
    {
        profile.verified = true;
        env.storage().persistent().set(&profile_key, &profile);
    }

    return_stake_fn(env, &provider)?;

    let topics = (Symbol::new(env, "ban_reversed"),);
    env.events().publish(topics, provider);

    Ok(())
}

/// Governance calls this to reject an appeal.
pub fn reject_ban_appeal(env: &Env, provider: Address) -> Result<(), AppealError> {
    let key = ProviderStorageKey::BanAppeal(provider.clone());
    let mut appeal: BanAppeal = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(AppealError::AppealNotFound)?;

    if appeal.status != AppealStatus::Pending {
        return Err(AppealError::AppealAlreadyResolved);
    }

    appeal.status = AppealStatus::Rejected;
    env.storage().persistent().set(&key, &appeal);

    let topics = (Symbol::new(env, "ban_appeal_rejected"),);
    env.events().publish(topics, provider);

    Ok(())
}

/// Get the current appeal record for a provider.
pub fn get_ban_appeal(env: &Env, provider: &Address) -> Option<BanAppeal> {
    env.storage()
        .persistent()
        .get(&ProviderStorageKey::BanAppeal(provider.clone()))
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AppealError {
    AppealAlreadyPending,
    AppealNotFound,
    AppealAlreadyResolved,
    GovernanceError,
}

// ─── Verification Eligibility (existing) ─────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationEligibility {
    pub eligible: bool,
    pub stake_ok: bool,
    pub history_ok: bool,
    pub success_rate_ok: bool,
    pub missing_criteria: Vec<String>,
}

pub fn check_verification_eligibility(
    env: &Env,
    provider: Address,
    stake: i128,
    stats: ProviderPerformance,
) -> VerificationEligibility {
    let stake_ok = stake >= GOLD_TIER_STAKE;
    let history_ok = stats.total_signals >= MIN_CLOSED_SIGNALS;
    let success_rate_ok = stats.success_rate >= MIN_SUCCESS_RATE_BPS;
    let eligible = stake_ok && history_ok && success_rate_ok;

    let mut missing_criteria = Vec::new(env);
    if !stake_ok {
        missing_criteria.push_back(String::from_str(env, "gold_tier_stake"));
    }
    if !history_ok {
        missing_criteria.push_back(String::from_str(env, "closed_signals"));
    }
    if !success_rate_ok {
        missing_criteria.push_back(String::from_str(env, "success_rate"));
    }

    crate::events::emit_verification_eligibility_checked(env, provider, eligible);

    VerificationEligibility {
        eligible,
        stake_ok,
        history_ok,
        success_rate_ok,
        missing_criteria,
    }
}

/// Set the specialization tags for a provider (self-selected).
/// Replaces any existing tags for this provider.
/// Validates that all tags are in the admin-defined set and within count limits.
pub fn set_provider_specializations(env: &Env, provider: &Address, tags: soroban_sdk::Vec<String>) {
    provider.require_auth();

    if tags.len() > MAX_SPECIALIZATION_TAGS_PER_PROVIDER {
        panic!("provider may select at most {MAX_SPECIALIZATION_TAGS_PER_PROVIDER} specialization tags");
    }

    let valid_tags: soroban_sdk::Vec<String> = env
        .storage()
        .instance()
        .get(&SpecializationStorageKey::SpecializationTags)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));

    for i in 0..tags.len() {
        if let Some(tag) = tags.get(i) {
            if tag.len() == 0 || tag.len() > MAX_SPECIALIZATION_TAG_LENGTH {
                panic!("invalid tag length");
            }
            let mut found = false;
            for j in 0..valid_tags.len() {
                if let Some(vt) = valid_tags.get(j) {
                    if vt == tag {
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                panic!("tag is not in the admin-defined set");
            }
        }
    }

    // Remove old reverse index entries.
    let old_key = SpecializationStorageKey::ProviderSpecializations(provider.clone());
    if let Some(old_tags) = env
        .storage()
        .persistent()
        .get::<_, soroban_sdk::Vec<String>>(&old_key)
    {
        for i in 0..old_tags.len() {
            if let Some(old_tag) = old_tags.get(i) {
                let rev_key = SpecializationStorageKey::ProvidersBySpecialization(old_tag);
                let mut providers: soroban_sdk::Vec<Address> = env
                    .storage()
                    .persistent()
                    .get(&rev_key)
                    .unwrap_or_else(|| soroban_sdk::Vec::new(env));
                let mut updated = soroban_sdk::Vec::new(env);
                for j in 0..providers.len() {
                    if let Some(p) = providers.get(j) {
                        if p != *provider {
                            updated.push_back(p);
                        }
                    }
                }
                if updated.len() > 0 {
                    env.storage().persistent().set(&rev_key, &updated);
                } else {
                    env.storage().persistent().remove(&rev_key);
                }
            }
        }
    }

    // Store new tags and update reverse index.
    env.storage().persistent().set(&old_key, &tags);
    for i in 0..tags.len() {
        if let Some(tag) = tags.get(i) {
            let rev_key = SpecializationStorageKey::ProvidersBySpecialization(tag);
            let mut providers: soroban_sdk::Vec<Address> = env
                .storage()
                .persistent()
                .get(&rev_key)
                .unwrap_or_else(|| soroban_sdk::Vec::new(env));
            let mut already = false;
            for j in 0..providers.len() {
                if let Some(p) = providers.get(j) {
                    if p == *provider {
                        already = true;
                        break;
                    }
                }
            }
            if !already {
                providers.push_back(provider.clone());
                env.storage().persistent().set(&rev_key, &providers);
            }
        }
    }
}

/// Returns the specialization tags for a given provider.
pub fn get_provider_specializations(env: &Env, provider: &Address) -> soroban_sdk::Vec<String> {
    env.storage()
        .persistent()
        .get(&SpecializationStorageKey::ProviderSpecializations(
            provider.clone(),
        ))
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

/// Returns all providers that have selected the given specialization tag.
pub fn list_providers_by_specialization(
    env: &Env,
    tag: soroban_sdk::String,
) -> soroban_sdk::Vec<Address> {
    env.storage()
        .persistent()
        .get(&SpecializationStorageKey::ProvidersBySpecialization(tag))
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

// ═══════════════════════════════════════════════════════════════════

/// Check if a provider is banned (presence of ban reason indicates banned status)
pub fn is_provider_banned(env: &Env, provider: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&BanStorageKey::ProviderBanReason(provider.clone()))
}

/// Get the ban reason hash for a banned provider
pub fn get_ban_reason(env: &Env, provider: &Address) -> Option<String> {
    env.storage()
        .persistent()
        .get(&BanStorageKey::ProviderBanReason(provider.clone()))
}

/// Ban a provider: persist the ban and cancel all active signals.
///
/// This is the "effects" half of the ban flow — it performs **no external
/// calls** and is safe to call and persist in full before the caller talks
/// to `StakeVault` (see [`slash_stake`]). Splitting the pure-storage work
/// from the cross-contract interaction lets the caller follow
/// checks-effects-interactions: persist the ban reason and cancelled
/// signals first, *then* invoke the external contract, so a reentrant call
/// arriving during that external call can never observe a signal that is
/// "active" but already mid-ban (Issue #781).
///
/// # Arguments
/// * `env` - Soroban environment
/// * `signals_map` - Mutable reference to the signals map (signals will be cancelled in-place)
/// * `provider` - Address of the provider to ban
/// * `reason_hash` - On-chain evidence hash (e.g. IPFS CID of dispute documentation)
///
/// # Returns
/// Number of signals cancelled.
pub fn apply_ban(
    env: &Env,
    signals_map: &mut Map<u64, Signal>,
    provider: &Address,
    reason_hash: &String,
) -> u32 {
    // Mark provider as banned by storing the reason hash
    env.storage().persistent().set(
        &BanStorageKey::ProviderBanReason(provider.clone()),
        reason_hash,
    );

    // Cancel all active signals from this provider
    let mut signals_cancelled: u32 = 0;
    for i in 0..signals_map.keys().len() {
        if let Some(key) = signals_map.keys().get(i) {
            if let Some(mut signal) = signals_map.get(key) {
                if signal.provider == *provider && signal.status == SignalStatus::Active {
                    signal.status = SignalStatus::Failed;
                    signals_map.set(key, signal);
                    signals_cancelled += 1;
                }
            }
        }
    }

    signals_cancelled
}

/// Slash the full stake of a provider via a `StakeVault` cross-contract call.
///
/// # Reentrancy risk assessment (Issue #781)
/// This is the only cross-contract *write* call site in `signal_registry`.
/// `stake_vault` is admin-supplied and, in the general case, untrusted code:
/// nothing stops a misconfigured or malicious address from calling back into
/// `signal_registry` while its `slash_stake` entrypoint is executing. The
/// caller (`SignalRegistry::ban_provider` in `lib.rs`) addresses this with
/// two independent layers:
/// 1. **Checks-effects-interactions** — all storage effects of the ban
///    ([`apply_ban`]) are persisted *before* this function is called, so a
///    reentrant read sees the post-ban state, not an intermediate one.
/// 2. **Reentrancy guard** — the caller wraps the whole entrypoint in
///    [`crate::reentrancy::guarded`], so any reentrant *state-changing*
///    call back into a guarded `signal_registry` entrypoint (including a
///    second `ban_provider`) is rejected with `AdminError::ReentrancyDetected`
///    before it can touch storage.
pub fn slash_stake(env: &Env, provider: &Address, stake_vault: &Address) -> i128 {
    let sym = soroban_sdk::Symbol::new(env, "get_stake");
    let mut args = soroban_sdk::Vec::<soroban_sdk::Val>::new(env);
    args.push_back(provider.clone().into_val(env));
    let stake: i128 = env.invoke_contract(stake_vault, &sym, args);

    if stake <= 0 {
        return 0;
    }

    // Call slash_stake on StakeVault — pass this contract as caller (authorizes
    // the slash), the provider, the slash severity tier, and a reason tag for
    // the audit event. A ban always applies the harshest tier: StakeVault's
    // `SlashSeverity` is a `#[repr(u32)]` fieldless enum (Minor=0, Major=1,
    // Critical=2), so it crosses the contract boundary as a plain u32 — there
    // is no shared Rust type between the two independently deployed contracts.
    const SLASH_SEVERITY_CRITICAL: u32 = 2;

    let slash_sym = soroban_sdk::Symbol::new(env, "slash_stake");
    let mut slash_args = soroban_sdk::Vec::<soroban_sdk::Val>::new(env);
    let caller = env.current_contract_address();
    slash_args.push_back(caller.into_val(env));
    slash_args.push_back(provider.clone().into_val(env));
    slash_args.push_back(SLASH_SEVERITY_CRITICAL.into_val(env));
    let reason = soroban_sdk::Symbol::new(env, "ban");
    slash_args.push_back(reason.into_val(env));

    // StakeVault::slash_stake returns the amount actually slashed, which is
    // the tier percentage of `stake` (default Critical = 100%) — read it back
    // instead of assuming `stake` in full, so the emitted event always
    // reflects what was really burned.
    env.invoke_contract::<i128>(stake_vault, &slash_sym, slash_args)
}

/// Emit the ProviderBanned event
pub fn emit_provider_banned(
    env: &Env,
    provider: &Address,
    reason_hash: &String,
    signals_cancelled: u32,
    stake_slashed: i128,
) {
    let topics = (
        soroban_sdk::Symbol::new(env, "provider_banned"),
        provider.clone(),
    );
    env.events().publish(
        topics,
        (reason_hash.clone(), signals_cancelled, stake_slashed),
    );
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Bytes;

    fn stats(total_signals: u32, success_rate: u32) -> ProviderPerformance {
        ProviderPerformance {
            total_signals,
            successful_signals: 0,
            failed_signals: 0,
            total_copies: 0,
            success_rate,
            avg_return: 0,
            total_volume: 0,
            follower_count: 0,
        }
    }

    // ── Profile tests ──────────────────────────────────────────────────────

    fn with_contract<R>(f: impl FnOnce(&Env) -> R) -> R {
        let env = Env::default();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        env.as_contract(&cid, || f(&env))
    }

    fn mocked_contract() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        #[allow(deprecated)]
        let cid = env.register_contract(None, crate::SignalRegistry);
        (env, cid)
    }

    fn with_contract_mocked<R>(f: impl FnOnce(&Env) -> R) -> R {
        let (env, cid) = mocked_contract();
        env.as_contract(&cid, || f(&env))
    }

    #[test]
    fn profile_created_on_first_stake() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let s = stats(25, 7_000);

            let profile = create_or_update_provider_profile(
                &env,
                provider.clone(),
                String::from_str(&env, "abc123"),
                String::from_str(&env, "bio456"),
                &s,
                GOLD_TIER_STAKE,
                false,
            );

            assert_eq!(profile.total_signals, 25);
            assert_eq!(profile.stake_tier, 3);
            assert!(!profile.verified);

            let stored = get_provider_profile(&env, &provider).unwrap();
            assert_eq!(stored.display_name_hash, String::from_str(&env, "abc123"));
        });
    }

    #[test]
    fn profile_update_preserves_created_at() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let s = stats(10, 5_000);

            let first = create_or_update_provider_profile(
                &env,
                provider.clone(),
                String::from_str(&env, "hash1"),
                String::from_str(&env, "bio1"),
                &s,
                0,
                false,
            );

            let second = create_or_update_provider_profile(
                &env,
                provider.clone(),
                String::from_str(&env, "hash2"),
                String::from_str(&env, "bio2"),
                &s,
                0,
                true,
            );

            assert_eq!(first.created_at, second.created_at);
            assert_eq!(second.display_name_hash, String::from_str(&env, "hash2"));
            assert!(second.verified);
        });
    }

    #[test]
    fn profile_readable_by_anyone() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let s = stats(5, 4_000);

            create_or_update_provider_profile(
                &env,
                provider.clone(),
                String::from_str(&env, "h"),
                String::from_str(&env, "b"),
                &s,
                0,
                false,
            );

            // Any address can read
            let reader = Address::generate(&env);
            let _ = reader;
            assert!(get_provider_profile(&env, &provider).is_some());
        });
    }

    // ── Appeal tests ───────────────────────────────────────────────────────

    fn stub_create_proposal(
        _env: &Env,
        _provider: &Address,
        _evidence: &Bytes,
    ) -> Result<u64, AppealError> {
        Ok(42) // fake proposal id
    }

    fn stub_return_stake(_env: &Env, _provider: &Address) -> Result<(), AppealError> {
        Ok(())
    }

    #[test]
    fn appeal_submission_creates_governance_proposal() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let evidence = Bytes::from_slice(&env, b"ipfs://evidence");

            let appeal =
                submit_ban_appeal(&env, provider.clone(), evidence, stub_create_proposal).unwrap();

            assert_eq!(appeal.governance_proposal_id, 42);
            assert_eq!(appeal.status, AppealStatus::Pending);

            let stored = get_ban_appeal(&env, &provider).unwrap();
            assert_eq!(stored.governance_proposal_id, 42);
        });
    }

    #[test]
    fn governance_reversal_restores_provider_status_and_stake() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let evidence = Bytes::from_slice(&env, b"ipfs://evidence");

            // Create profile first
            let s = stats(25, 7_000);
            create_or_update_provider_profile(
                &env,
                provider.clone(),
                String::from_str(&env, "h"),
                String::from_str(&env, "b"),
                &s,
                GOLD_TIER_STAKE,
                false, // banned → verified=false
            );

            submit_ban_appeal(&env, provider.clone(), evidence, stub_create_proposal).unwrap();
            reverse_ban(&env, provider.clone(), stub_return_stake).unwrap();

            let appeal = get_ban_appeal(&env, &provider).unwrap();
            assert_eq!(appeal.status, AppealStatus::Approved);

            let profile = get_provider_profile(&env, &provider).unwrap();
            assert!(profile.verified);
        });
    }

    #[test]
    fn governance_rejection_sets_rejected_status() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let evidence = Bytes::from_slice(&env, b"ipfs://evidence");

            submit_ban_appeal(&env, provider.clone(), evidence, stub_create_proposal).unwrap();
            reject_ban_appeal(&env, provider.clone()).unwrap();

            let appeal = get_ban_appeal(&env, &provider).unwrap();
            assert_eq!(appeal.status, AppealStatus::Rejected);
        });
    }

    #[test]
    fn duplicate_pending_appeal_rejected() {
        with_contract(|env| {
            let provider = Address::generate(&env);
            let evidence = Bytes::from_slice(&env, b"ipfs://evidence");

            submit_ban_appeal(
                &env,
                provider.clone(),
                evidence.clone(),
                stub_create_proposal,
            )
            .unwrap();
            let result = submit_ban_appeal(&env, provider.clone(), evidence, stub_create_proposal);
            assert_eq!(result, Err(AppealError::AppealAlreadyPending));
        });
    }

    // ── Existing eligibility tests ─────────────────────────────────────────

    #[test]
    fn fully_eligible_provider_passes() {
        let env = Env::default();
        let provider = Address::generate(&env);

        let eligibility = check_verification_eligibility(
            &env,
            provider,
            GOLD_TIER_STAKE,
            stats(MIN_CLOSED_SIGNALS, MIN_SUCCESS_RATE_BPS),
        );

        assert!(eligibility.eligible);
        assert!(eligibility.stake_ok);
        assert!(eligibility.history_ok);
        assert!(eligibility.success_rate_ok);
        assert_eq!(eligibility.missing_criteria.len(), 0);
    }

    #[test]
    fn partially_eligible_provider_reports_missing_criteria() {
        let env = Env::default();
        let provider = Address::generate(&env);

        let eligibility = check_verification_eligibility(
            &env,
            provider,
            GOLD_TIER_STAKE,
            stats(MIN_CLOSED_SIGNALS - 1, MIN_SUCCESS_RATE_BPS),
        );

        assert!(!eligibility.eligible);
        assert!(eligibility.stake_ok);
        assert!(!eligibility.history_ok);
        assert!(eligibility.success_rate_ok);
        assert_eq!(eligibility.missing_criteria.len(), 1);
    }

    #[test]
    fn not_eligible_provider_reports_all_missing_criteria() {
        let env = Env::default();
        let provider = Address::generate(&env);

        let eligibility = check_verification_eligibility(&env, provider, 0, stats(0, 0));

        assert!(!eligibility.eligible);
        assert!(!eligibility.stake_ok);
        assert!(!eligibility.history_ok);
        assert!(!eligibility.success_rate_ok);
        assert_eq!(eligibility.missing_criteria.len(), 3);
    }

    // ── Provider specialization tag tests (Issue #704) ──────────────────────

    #[test]
    fn add_and_list_specialization_tags() {
        let (env, cid) = mocked_contract();
        let admin = Address::generate(&env);
        let provider1 = Address::generate(&env);
        let provider2 = Address::generate(&env);

        // Add admin-defined tags
        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "DeFi")).unwrap();
        });
        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "Forex")).unwrap();
        });

        let tags = env.as_contract(&cid, || get_specialization_tags(&env));
        assert_eq!(tags.len(), 2);

        // Set provider specializations
        let mut p1_tags = soroban_sdk::Vec::new(&env);
        p1_tags.push_back(String::from_str(&env, "DeFi"));
        env.as_contract(&cid, || {
            set_provider_specializations(&env, &provider1, p1_tags.clone());
        });

        let mut p2_tags = soroban_sdk::Vec::new(&env);
        p2_tags.push_back(String::from_str(&env, "Forex"));
        env.as_contract(&cid, || {
            set_provider_specializations(&env, &provider2, p2_tags.clone());
        });

        // List by tag
        let defi_providers =
            env.as_contract(&cid, || list_providers_by_specialization(&env, String::from_str(&env, "DeFi")));
        assert_eq!(defi_providers.len(), 1);
        assert_eq!(defi_providers.get(0).unwrap(), provider1);
    }

    #[test]
    fn tag_count_limit_enforced() {
        let (env, cid) = mocked_contract();
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);

        for tag in ["DeFi", "Forex", "Large-cap", "Crypto"] {
            env.as_contract(&cid, || {
                add_specialization_tag(&env, &admin, String::from_str(&env, tag)).unwrap();
            });
        }

        // Try to set 4 tags (limit is 3)
        let mut tags = soroban_sdk::Vec::new(&env);
        tags.push_back(String::from_str(&env, "DeFi"));
        tags.push_back(String::from_str(&env, "Forex"));
        tags.push_back(String::from_str(&env, "Large-cap"));
        tags.push_back(String::from_str(&env, "Crypto"));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.as_contract(&cid, || set_provider_specializations(&env, &provider, tags));
        }));
        assert!(result.is_err());
    }

    #[test]
    fn unknown_tag_rejected() {
        let (env, cid) = mocked_contract();
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);

        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "DeFi")).unwrap();
        });

        let mut tags = soroban_sdk::Vec::new(&env);
        tags.push_back(String::from_str(&env, "UnknownTag"));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.as_contract(&cid, || set_provider_specializations(&env, &provider, tags));
        }));
        assert!(result.is_err());
    }

    #[test]
    fn provider_can_retag() {
        let (env, cid) = mocked_contract();
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);

        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "DeFi")).unwrap();
        });
        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "Forex")).unwrap();
        });

        let mut tags1 = soroban_sdk::Vec::new(&env);
        tags1.push_back(String::from_str(&env, "DeFi"));
        env.as_contract(&cid, || {
            set_provider_specializations(&env, &provider, tags1.clone());
        });

        let stored1 = env.as_contract(&cid, || get_provider_specializations(&env, &provider));
        assert_eq!(stored1.len(), 1);
        assert_eq!(stored1.get(0).unwrap(), String::from_str(&env, "DeFi"));

        // Retag
        let mut tags2 = soroban_sdk::Vec::new(&env);
        tags2.push_back(String::from_str(&env, "Forex"));
        env.as_contract(&cid, || {
            set_provider_specializations(&env, &provider, tags2.clone());
        });

        let stored2 = env.as_contract(&cid, || get_provider_specializations(&env, &provider));
        assert_eq!(stored2.len(), 1);
        assert_eq!(stored2.get(0).unwrap(), String::from_str(&env, "Forex"));

        // Old tag should no longer list the provider
        let defi_providers = env
            .as_contract(&cid, || list_providers_by_specialization(&env, String::from_str(&env, "DeFi")));
        assert_eq!(defi_providers.len(), 0);
    }

    #[test]
    fn remove_specialization_tag_works() {
        let (env, cid) = mocked_contract();
        let admin = Address::generate(&env);

        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "DeFi")).unwrap();
        });
        env.as_contract(&cid, || {
            add_specialization_tag(&env, &admin, String::from_str(&env, "Forex")).unwrap();
        });

        let count = env.as_contract(&cid, || get_specialization_tags(&env).len());
        assert_eq!(count, 2);

        env.as_contract(&cid, || {
            remove_specialization_tag(&env, &admin, String::from_str(&env, "DeFi"));
        });

        let remaining = env.as_contract(&cid, || get_specialization_tags(&env));
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining.get(0).unwrap(), String::from_str(&env, "Forex"));
    }
}
