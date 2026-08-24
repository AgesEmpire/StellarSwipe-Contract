//! Cross-contract reentrancy integration test for `SignalRegistry::ban_provider`
//! (Issue #781).
//!
//! The issue's threat model: `signal_registry` calls out to an admin-supplied
//! `stake_vault` address from `ban_provider`'s slashing step. A misconfigured
//! or outright malicious address at that slot could, in principle, call back
//! into `signal_registry` mid-transaction and observe or corrupt state.
//!
//! ## What this test found
//!
//! Soroban's host enforces `ContractReentryMode::Prohibited` on *every*
//! standard cross-contract call (`env.invoke_contract` / `try_invoke_contract`
//! — see `soroban-env-host`'s `host/frame.rs`), unconditionally, for every
//! contract that doesn't opt in to a looser mode. There is currently no
//! `#[contractimpl]`-level opt-out available to ordinary contract code, so a
//! callee attempting to call back into a contract already on the stack — even
//! for a *read-only* query — is rejected by the host itself before it ever
//! reaches `signal_registry`'s own code, and the entire transaction reverts.
//!
//! `test_malicious_reentry_reverts_whole_call` below confirms exactly that:
//! the attack in the issue is already closed at the protocol layer, and
//! reverting is atomic — a rejected reentrant attempt leaves *zero* trace,
//! not a half-applied ban.
//!
//! That platform guarantee doesn't make `signal_registry`'s own hardening
//! redundant, though:
//! - It's the only protection this contract controls; the protocol behavior
//!   above is Soroban's, not `signal_registry`'s, and the host code itself
//!   notes a `reentry` opt-in flag is planned for `try_call` (currently
//!   hardcoded to `Prohibited`) — if a future protocol version relaxes this
//!   default, `signal_registry`'s own guard is what keeps it safe.
//! - It fails with a typed `AdminError::ReentrancyDetected` rather than an
//!   opaque host trap, which is what `contracts/signal_registry`'s own unit
//!   tests (`tests/test_reentrancy.rs`) exercise directly.
//! - `apply_ban`'s checks-effects-interactions ordering (persist before
//!   calling out) is correct practice independent of *why* reentrancy is
//!   blocked.
//!
//! `test_ban_provider_slashes_via_real_stake_vault` below covers the *non*
//! -adversarial path end-to-end against the real `StakeVaultContract`,
//! which also exercises a correctness fix made alongside this audit:
//! `providers::slash_stake` was previously calling `StakeVault::slash_stake`
//! with a raw stake amount where the real contract expects a `SlashSeverity`
//! tier — a mismatch that would have caused every `ban_provider` slash to
//! trap against a real vault. `ban_provider` had no test coverage at all
//! before this issue.

extern crate std;

use signal_registry::{
    RiskLevel, SignalAction, SignalCategory, SignalRegistry, SignalRegistryClient, SignalStatus,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, token::StellarAssetClient,
    Address, Env, String, Symbol,
};
use stake_vault::{StakeVaultContract, StakeVaultContractClient};
use std::panic::{catch_unwind, AssertUnwindSafe};

// ── Malicious StakeVault stand-in ───────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Registry,
    Admin,
    Provider,
}

/// A `StakeVault` stand-in exposing the exact `get_stake` / `slash_stake`
/// wire ABI `providers::slash_stake` calls — but whose `slash_stake`
/// attempts to call straight back into `SignalRegistry` instead of doing any
/// real slashing.
#[contract]
struct MaliciousStakeVault;

#[contractimpl]
impl MaliciousStakeVault {
    /// Test-only setup; a real `StakeVault` has no equivalent.
    pub fn configure(env: Env, registry: Address, admin: Address, provider: Address) {
        env.storage().instance().set(&DataKey::Registry, &registry);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Provider, &provider);
    }

    pub fn get_stake(_env: Env, _staker: Address) -> i128 {
        100_000_000
    }

    pub fn slash_stake(
        env: Env,
        _caller: Address,
        provider: Address,
        _severity: u32,
        _reason: Symbol,
    ) -> i128 {
        let registry: Address = env.storage().instance().get(&DataKey::Registry).unwrap();
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        let client = SignalRegistryClient::new(&env, &registry);

        // Attempt to reenter `signal_registry` — even a read-only call.
        // Soroban's host rejects this unconditionally (see module docs
        // above); this call never returns normally.
        let _ = client.is_provider_banned(&provider);

        // Unreachable in practice, but keep the mock's control flow honest.
        let reason = String::from_str(&env, "reentrant-attempt");
        client.ban_provider(&admin, &provider, &reason, &env.current_contract_address());
        0
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn sac_token(env: &Env, admin: &Address) -> Address {
    env.register_stellar_asset_contract_v2(admin.clone())
        .address()
}

/// Deploy `SignalRegistry`, initialize it, stake and submit one signal for
/// `provider`. Returns (client, signal_id).
fn setup_registry_with_signal(
    env: &Env,
    admin: &Address,
    provider: &Address,
) -> (SignalRegistryClient<'static>, u64) {
    let registry_id = env.register(SignalRegistry, ());
    let registry = SignalRegistryClient::new(env, &registry_id);
    registry.initialize(admin);
    registry.stake_tokens(provider, &200_000_000i128);

    let expiry = env.ledger().timestamp() + 7_200;
    let signal_id = registry.create_signal(
        provider,
        &String::from_str(env, "XLM/USDC"),
        &SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(env, "reentrancy fixture signal"),
        &expiry,
        &SignalCategory::SWING,
        &soroban_sdk::Vec::new(env),
        &RiskLevel::Medium,
    );

    (registry, signal_id)
}

// ── Tests ────────────────────────────────────────────────────────────────

/// A malicious `stake_vault` attempting to reenter `signal_registry` from
/// `slash_stake` must not be able to leave the provider half-banned. Soroban
/// rejects the reentrant call at the host level (see module docs), which
/// aborts the *entire* `ban_provider` transaction — so this test asserts the
/// stronger, end-to-end property the issue actually cares about: the attempt
/// leaves no trace at all, not even the effects `apply_ban` persisted before
/// the external call.
#[test]
fn test_malicious_reentry_reverts_whole_call() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let (registry, signal_id) = setup_registry_with_signal(&env, &admin, &provider);

    let vault_id = env.register(MaliciousStakeVault, ());
    MaliciousStakeVaultClient::new(&env, &vault_id).configure(&registry.address, &admin, &provider);

    let reason = String::from_str(&env, "ipfs://ban-evidence");
    let registry_for_call = SignalRegistryClient::new(&env, &registry.address);
    let admin_for_call = admin.clone();
    let provider_for_call = provider.clone();
    let reason_for_call = reason.clone();
    let vault_for_call = vault_id.clone();

    let result = catch_unwind(AssertUnwindSafe(|| {
        registry_for_call.ban_provider(
            &admin_for_call,
            &provider_for_call,
            &reason_for_call,
            &vault_for_call,
        );
    }));

    assert!(
        result.is_err(),
        "ban_provider with a reentrant stake_vault should have reverted the whole call"
    );

    // No trace of the attempted ban: the transaction-level revert must undo
    // even the effects `apply_ban` persisted before calling out to the
    // (malicious) vault.
    assert!(
        !registry.is_provider_banned(&provider),
        "a reverted ban_provider call must not leave the provider banned"
    );
    assert_eq!(
        registry.get_signal(&signal_id).unwrap().status,
        SignalStatus::Active,
        "a reverted ban_provider call must not leave signals cancelled"
    );

    // The registry must still be fully usable afterwards — a failed
    // adversarial call must not brick the contract or leak a held lock.
    let provider2 = Address::generate(&env);
    registry.stake_tokens(&provider2, &200_000_000i128);
    registry.unstake_tokens(&provider2);
}

/// Non-adversarial happy path against the real `StakeVaultContract`: proves
/// `ban_provider` actually interoperates with the deployed vault ABI
/// end-to-end (exercising the `SlashSeverity` encoding fix made alongside
/// this audit), cancels the provider's active signals, and slashes their
/// full stake (Critical tier = 100% by default).
#[test]
fn test_ban_provider_slashes_via_real_stake_vault() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let (registry, signal_id) = setup_registry_with_signal(&env, &admin, &provider);

    let token = sac_token(&env, &admin);
    let vault_id = env.register(StakeVaultContract, ());
    let vault = StakeVaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &token, &registry.address);

    StellarAssetClient::new(&env, &token).mint(&provider, &1_000_000_000i128);
    vault.deposit_stake(&provider, &500_000_000i128);
    assert_eq!(vault.get_stake(&provider), 500_000_000i128);

    let reason = String::from_str(&env, "ipfs://ban-evidence");
    registry.ban_provider(&admin, &provider, &reason, &vault_id);

    assert!(registry.is_provider_banned(&provider));
    assert_eq!(
        registry.get_signal(&signal_id).unwrap().status,
        SignalStatus::Failed
    );
    // Critical tier defaults to 100% — the vault's full stake is gone.
    assert_eq!(vault.get_stake(&provider), 0i128);
}
