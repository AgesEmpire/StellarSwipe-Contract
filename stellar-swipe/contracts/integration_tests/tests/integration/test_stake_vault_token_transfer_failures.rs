//! Downstream token-transfer failure handling for `stake_vault`'s reward
//! claim path (Issue #1337).
//!
//! `StakeVaultContract::batch_claim_rewards` (via
//! `reward_vault::batch_claim_rewards`) commits claim/balance accounting
//! writes *before* transferring tokens out (a CEI pattern — see the
//! "Phase 2: commit entitlements before any transfer" comment in
//! `contracts/stake_vault/src/reward_vault.rs`), relying on the Soroban
//! host to roll back the whole invocation, accounting writes included, if
//! the transfer later fails. That guarantee had no test: nothing in this
//! workspace exercised `batch_claim_rewards` against a reward asset whose
//! token contract actually rejects the transfer.
//!
//! These tests use two minimal SEP-41-shaped stub contracts (not the real
//! Stellar Asset Contract) standing in for a downstream token that always
//! rejects a transfer, and one that fails exactly once before succeeding
//! (modeling a transient failure a caller retries past).

use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};
use stake_vault::{StakeVaultContract, StakeVaultContractClient};

/// Deploys a real Stellar Asset Contract to stand in for the vault's stake
/// token (not under test here — only the reward-asset token transfer is).
/// Mirrors the identical helper in `test_signal_to_reward_lifecycle.rs`.
fn sac_token(env: &Env, admin: &Address) -> Address {
    env.register_stellar_asset_contract_v2(admin.clone())
        .address()
}

/// Token stub whose `transfer` entrypoint unconditionally rejects, standing
/// in for a downstream token contract that always fails the call (e.g. a
/// frozen/deauthorized trustline, or a custom SEP-41 token panicking on an
/// invariant it enforces internally).
#[contract]
struct AlwaysRejectingToken;

#[contractimpl]
impl AlwaysRejectingToken {
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {
        panic!("token rejected the transfer");
    }
}

/// Token stub that rejects its first `transfer` call and succeeds on every
/// call after that, modeling a transient downstream failure (e.g. a
/// temporary trustline/authorization hiccup) that a caller retries past.
#[contract]
struct FailOnceThenSucceedToken;

#[contractimpl]
impl FailOnceThenSucceedToken {
    pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
        let key = soroban_sdk::symbol_short!("calls");
        let calls: u32 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(calls + 1));
        if calls == 0 {
            panic!("transient token failure on first call");
        }
    }
}

fn setup() -> (
    Env,
    Address,
    Address,
    StakeVaultContractClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let stake_token = sac_token(&env, &admin);
    let signal_registry_stub = Address::generate(&env);

    let vault_id = env.register(StakeVaultContract, ());
    let vault = StakeVaultContractClient::new(&env, &vault_id);
    vault.initialize(&admin, &stake_token, &signal_registry_stub);
    (env, admin, provider, vault)
}

/// A failed downstream transfer must roll back the whole claim: the
/// bucket must remain unclaimed (claimable again if the asset is ever
/// fixed/replaced) and the provider's recorded balance must be unchanged
/// — no partial accounting despite the entitlement having been written in
/// "Phase 2" before the failing transfer in "Phase 3".
#[test]
fn failed_token_transfer_leaves_no_partial_accounting() {
    let (env, admin, provider, vault) = setup();
    let reward_asset = env.register(AlwaysRejectingToken, ());

    vault.add_reward_asset(&reward_asset);
    let bucket_id = vault.deposit_reward(&admin, &provider, &reward_asset, &1_000_000i128, &1u64);

    let balance_before = vault.get_provider_reward_balance(&provider, &reward_asset);
    assert_eq!(balance_before, 1_000_000i128);

    let result = vault.try_batch_claim_rewards(&provider, &soroban_sdk::vec![&env, bucket_id]);
    assert!(
        result.is_err(),
        "claim must fail when the downstream token transfer is rejected"
    );

    // No partial accounting: balance must be exactly what it was before
    // the failed claim attempt, not partially decremented.
    let balance_after = vault.get_provider_reward_balance(&provider, &reward_asset);
    assert_eq!(
        balance_after, balance_before,
        "a failed token transfer must leave the reward balance untouched \
         (Phase 2's entitlement write must have been rolled back with Phase 3's failure)"
    );
}

/// Companion to the rollback test above: after a transient downstream
/// failure, retrying the exact same claim must succeed, and must produce
/// exactly one transfer and one `rwdbatch` event — not zero (stuck forever)
/// and not more than one (double-spend of the same bucket).
#[test]
fn retry_after_transient_token_failure_produces_exactly_one_transfer_and_event() {
    let (env, admin, provider, vault) = setup();
    let reward_asset = env.register(FailOnceThenSucceedToken, ());

    vault.add_reward_asset(&reward_asset);
    let bucket_id = vault.deposit_reward(&admin, &provider, &reward_asset, &1_000_000i128, &1u64);

    // First attempt: the stub's token transfer fails transiently.
    let first = vault.try_batch_claim_rewards(&provider, &soroban_sdk::vec![&env, bucket_id]);
    assert!(first.is_err(), "first claim attempt must fail (transient token failure)");
    assert_eq!(
        vault.get_provider_reward_balance(&provider, &reward_asset),
        1_000_000i128,
        "balance must still be unclaimed after the failed first attempt"
    );

    // Retry: the stub's second call succeeds.
    let events_before = env.events().all().len();
    let totals = vault.batch_claim_rewards(&provider, &soroban_sdk::vec![&env, bucket_id]);
    let events_after = env.events().all().len();

    assert_eq!(totals.len(), 1, "exactly one asset transfer must be recorded");
    assert_eq!(totals.get(0).unwrap().1, 1_000_000i128);
    assert_eq!(
        events_after - events_before,
        1,
        "the successful retry must emit exactly one reward-claimed event"
    );
    assert_eq!(
        vault.get_provider_reward_balance(&provider, &reward_asset),
        0,
        "balance must be fully consumed after the successful retry"
    );

    // A second retry of the same (already-claimed) bucket must fail --
    // proving the successful claim was recorded, not silently re-claimable.
    let second_retry = vault.try_batch_claim_rewards(&provider, &soroban_sdk::vec![&env, bucket_id]);
    assert!(
        second_retry.is_err(),
        "an already-claimed bucket must not be claimable again"
    );
}
