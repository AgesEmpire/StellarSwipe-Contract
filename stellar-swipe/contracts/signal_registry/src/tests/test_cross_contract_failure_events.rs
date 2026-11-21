#![cfg(test)]
//! Failed cross-contract calls must emit no success events (Issue #1340).
//!
//! `ban_provider` persists its ban effects and *then* performs a
//! cross-contract call to `StakeVault::slash_stake` before finally emitting
//! `provider_banned` (see `providers::slash_stake` / `SignalRegistry::ban_provider`
//! in `lib.rs`). If that downstream call panics, the whole host invocation —
//! including any storage writes already made and any events the function
//! would otherwise publish — must be rolled back atomically by the Soroban
//! host. These tests lock that guarantee down for the `ban_provider` /
//! `StakeVault::slash_stake` boundary specifically (the adversarial
//! *reentrancy* scenario is covered separately by `test_reentrancy.rs` and
//! `test_signal_registry_reentrancy` in `contracts/integration_tests`; this
//! file is about plain downstream failure, not malicious reentry).

use crate::{SignalRegistry, SignalRegistryClient, SignalStatus};
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, Address, Env, String, Symbol, Vec,
};

fn setup() -> (Env, Address, SignalRegistryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    #[allow(deprecated)]
    let id = env.register_contract(None, SignalRegistry);
    let client = SignalRegistryClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, admin, client)
}

/// `StakeVault` stand-in with a positive stake (so `slash_stake`'s
/// zero-stake short-circuit in `providers::slash_stake` is NOT taken) whose
/// `slash_stake` entrypoint unconditionally panics — simulating a downstream
/// failure (e.g. the vault's own invariant check rejecting the slash).
#[contract]
struct PanickingStakeVaultStub;

#[contractimpl]
impl PanickingStakeVaultStub {
    pub fn get_stake(_env: Env, _staker: Address) -> i128 {
        1_000_000i128
    }

    pub fn slash_stake(
        _env: Env,
        _caller: Address,
        _provider: Address,
        _severity: u32,
        _reason: Symbol,
    ) -> i128 {
        panic!("stake vault rejected the slash");
    }
}

/// Well-behaved counterpart: positive stake, slash succeeds and returns the
/// full amount. Used as the control case to confirm a *successful* cross-call
/// does emit exactly the expected success event.
#[contract]
struct SlashingStakeVaultStub;

#[contractimpl]
impl SlashingStakeVaultStub {
    pub fn get_stake(_env: Env, _staker: Address) -> i128 {
        1_000_000i128
    }

    pub fn slash_stake(
        _env: Env,
        _caller: Address,
        _provider: Address,
        _severity: u32,
        _reason: Symbol,
    ) -> i128 {
        1_000_000i128
    }
}

fn panicking_stake_vault(env: &Env) -> Address {
    #[allow(deprecated)]
    env.register_contract(None, PanickingStakeVaultStub)
}

fn slashing_stake_vault(env: &Env) -> Address {
    #[allow(deprecated)]
    env.register_contract(None, SlashingStakeVaultStub)
}

/// A failed downstream `slash_stake` call must abort the entire
/// `ban_provider` invocation: no `provider_banned` event (or any other
/// event) may be emitted, the provider must not end up marked banned, and
/// an active signal from that provider must remain `Active`.
#[test]
#[should_panic]
fn ban_provider_panics_when_downstream_slash_panics() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let stake_vault = panicking_stake_vault(&env);
    let reason = String::from_str(&env, "ipfs://evidence");

    // Give the provider an active signal so a real `apply_ban` effect would
    // be observable if it leaked past the panic.
    client.stake_tokens(&provider, &200_000_000i128);
    let expiry = 3_600u64;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "rationale"),
        &expiry,
        &crate::SignalCategory::SWING,
        &Vec::new(&env),
        &crate::RiskLevel::Medium,
    );

    // `ban_provider` panics because the stub's `slash_stake` panics; the
    // whole invocation (including `apply_ban`'s storage writes) must be
    // rolled back by the host.
    let _ = client.ban_provider(&admin, &provider, &reason, &stake_vault);

    // Unreachable in practice (the line above panics), but documents intent.
    let _ = (signal_id, SignalStatus::Active);
}

/// Companion assertion to the `#[should_panic]` test above: after the
/// panicking `ban_provider` call is caught (via `try_ban_provider`, which
/// Soroban reports as an error rather than unwinding the test harness), the
/// provider must NOT be banned, the signal must still be `Active`, and no
/// event must have been recorded — proving the rollback is complete, not
/// just that the outer call returned an error.
#[test]
fn failed_ban_provider_leaves_no_success_event_or_state_change() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let stake_vault = panicking_stake_vault(&env);
    let reason = String::from_str(&env, "ipfs://evidence");

    client.stake_tokens(&provider, &200_000_000i128);
    let expiry = 3_600u64;
    let signal_id = client.create_signal(
        &provider,
        &String::from_str(&env, "XLM/USDC"),
        &crate::SignalAction::Buy,
        &1_000_000i128,
        &String::from_str(&env, "rationale"),
        &expiry,
        &crate::SignalCategory::SWING,
        &Vec::new(&env),
        &crate::RiskLevel::Medium,
    );

    let events_before = env.events().all().len();

    // `try_ban_provider` surfaces the downstream panic as a client-side
    // invocation error instead of unwinding this test.
    let result = client.try_ban_provider(&admin, &provider, &reason, &stake_vault);
    assert!(
        result.is_err(),
        "ban_provider must fail when the downstream slash panics"
    );

    // No event — success or otherwise — survives the rollback.
    assert_eq!(
        env.events().all().len(),
        events_before,
        "a failed cross-contract call must not leave behind any events, \
         including a spurious provider_banned success event"
    );

    // State effects from `apply_ban` (which ran before the failing
    // cross-call) must also have been rolled back.
    assert!(
        !client.is_provider_banned(&provider),
        "provider must not be banned when the downstream slash failed"
    );
    let signal = client.get_signal(&signal_id).unwrap();
    assert_eq!(
        signal.status,
        SignalStatus::Active,
        "signal must remain Active — apply_ban's cancellation must have been rolled back"
    );
}

/// Control case: when the downstream `slash_stake` call *succeeds*, exactly
/// one `provider_banned` event is emitted (not zero, not duplicated), and
/// the provider ends up banned.
#[test]
fn successful_ban_provider_emits_exactly_one_success_event() {
    let (env, admin, client) = setup();
    let provider = Address::generate(&env);
    let stake_vault = slashing_stake_vault(&env);
    let reason = String::from_str(&env, "ipfs://evidence");

    client.stake_tokens(&provider, &200_000_000i128);

    let events_before = env.events().all().len();
    client.ban_provider(&admin, &provider, &reason, &stake_vault);
    let events_after = env.events().all().len();

    assert_eq!(
        events_after - events_before,
        1,
        "a successful ban_provider call must emit exactly one event"
    );
    assert!(client.is_provider_banned(&provider));
}
