#![cfg(test)]

use crate::{
    migration::{seed_v1_stakes, MigrationKey, StakeInfoV2},
    StakeVaultContract, StakeVaultContractClient, StakeVaultError,
};
use soroban_sdk::{
    contract, contractimpl,
    testutils::Address as _,
    token::StellarAssetClient,
    Address, Env, Map, Symbol,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sac_token(env: &Env, admin: &Address) -> Address {
    env.register_stellar_asset_contract_v2(admin.clone())
        .address()
}

/// Seed a V2 stake record directly (bypasses migration).
fn seed_v2_stake(env: &Env, contract_id: &Address, staker: &Address, balance: i128, locked_until: u64) {
    env.as_contract(contract_id, || {
        let mut stakes: Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| Map::new(env));
        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance,
                locked_until,
                last_updated: env.ledger().timestamp(),
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);
    });
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token = sac_token(&env, &admin);
    let vault_id = env.register(StakeVaultContract, ());

    StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token);

    (env, vault_id, token, admin)
}

// ── Basic withdraw tests ──────────────────────────────────────────────────────

#[test]
fn withdraw_stake_transfers_balance() {
    let (env, vault_id, token, admin) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 5_000_000;

    // Fund the vault so it can transfer out.
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    let withdrawn = client.withdraw_stake(&staker);
    assert_eq!(withdrawn, amount);

    // Balance zeroed in storage.
    assert_eq!(client.get_stake(&staker), 0);
}

#[test]
fn withdraw_stake_no_stake_returns_error() {
    let (env, vault_id, _token, _admin) = setup();
    let staker = Address::generate(&env);

    let err = env.as_contract(&vault_id, || {
        StakeVaultContract::withdraw_stake(env.clone(), staker)
    });
    assert_eq!(err, Err(StakeVaultError::NoStake));
}

#[test]
fn withdraw_stake_locked_returns_error() {
    let (env, vault_id, token, _admin) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 1_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    // locked_until = far future
    seed_v2_stake(&env, &vault_id, &staker, amount, u64::MAX);

    let err = env.as_contract(&vault_id, || {
        StakeVaultContract::withdraw_stake(env.clone(), staker)
    });
    assert_eq!(err, Err(StakeVaultError::StakeLocked));
}

// ── Reentrancy guard tests ────────────────────────────────────────────────────

/// A malicious SEP-41 token that calls back into `withdraw_stake` during `transfer`.
#[contract]
pub struct ReentrantToken;

#[contractimpl]
impl ReentrantToken {
    pub fn set_vault(env: Env, vault: Address) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("vault"), &vault);
    }
    pub fn set_staker(env: Env, staker: Address) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("staker"), &staker);
    }

    /// Minimal SEP-41 `transfer` that attempts a reentrant `withdraw_stake`.
    pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
        let vault: Address = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("vault"))
            .unwrap();
        let staker: Address = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("staker"))
            .unwrap();

        let client = StakeVaultContractClient::new(&env, &vault);
        let result = client.try_withdraw_stake(&staker);

        // Record whether the reentrant call was blocked.
        let blocked = matches!(result, Err(Ok(StakeVaultError::ReentrancyDetected)));
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("blocked"), &blocked);
    }

    pub fn was_blocked(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("blocked"))
            .unwrap_or(false)
    }

    // Stub out other SEP-41 methods so the contract compiles.
    pub fn balance(_env: Env, _id: Address) -> i128 { 0 }
    pub fn transfer_from(_env: Env, _spender: Address, _from: Address, _to: Address, _amount: i128) {}
    pub fn approve(_env: Env, _from: Address, _spender: Address, _amount: i128, _expiration_ledger: u32) {}
    pub fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 { 0 }
    pub fn decimals(_env: Env) -> u32 { 7 }
    pub fn name(env: Env) -> soroban_sdk::String { soroban_sdk::String::from_str(&env, "ReentrantToken") }
    pub fn symbol(env: Env) -> soroban_sdk::String { soroban_sdk::String::from_str(&env, "RT") }
    pub fn mint(_env: Env, _to: Address, _amount: i128) {}
}

#[test]
fn reentrant_withdraw_is_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let staker = Address::generate(&env);

    // Register the malicious token and the vault.
    let token_id = env.register(ReentrantToken, ());
    let vault_id = env.register(StakeVaultContract, ());

    StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token_id);

    // Wire the reentrant token to know the vault and staker.
    ReentrantTokenClient::new(&env, &token_id).set_vault(&vault_id);
    ReentrantTokenClient::new(&env, &token_id).set_staker(&staker);

    // Seed a stake so the outer call proceeds past the NoStake check.
    seed_v2_stake(&env, &vault_id, &staker, 1_000_000, 0);

    // Outer call — will trigger the reentrant token's transfer callback.
    let _ = StakeVaultContractClient::new(&env, &vault_id).try_withdraw_stake(&staker);

    assert!(
        ReentrantTokenClient::new(&env, &token_id).was_blocked(),
        "reentrant withdraw_stake was not blocked with ReentrancyDetected"
    );
}

#[test]
fn lock_cleared_after_successful_withdrawal() {
    let (env, vault_id, token, _admin) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 2_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &(amount * 2));
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.withdraw_stake(&staker);

    // Re-seed and withdraw again — must succeed (lock was cleared).
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);
    let second = client.withdraw_stake(&staker);
    assert_eq!(second, amount);
}

#[test]
fn lock_cleared_after_failed_withdrawal() {
    let (env, vault_id, _token, _admin) = setup();
    let staker = Address::generate(&env);

    // First call fails (NoStake) — lock must still be cleared.
    let err = env.as_contract(&vault_id, || {
        StakeVaultContract::withdraw_stake(env.clone(), staker.clone())
    });
    assert_eq!(err, Err(StakeVaultError::NoStake));

    // Verify the lock key is absent (not set to true).
    let lock_still_set: bool = env.as_contract(&vault_id, || {
        env.storage()
            .temporary()
            .get::<_, bool>(&Symbol::new(&env, "WithdrawLock"))
            .unwrap_or(false)
    });
    assert!(!lock_still_set, "lock was not cleared after failed withdrawal");
}

// ── Issue #388: stake-below-minimum tests ─────────────────────────────────────

#[test]
fn signal_submission_allowed_when_stake_above_minimum() {
    let (env, vault_id, token, _admin) = setup();
    let provider = Address::generate(&env);
    let amount: i128 = 1_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &provider, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);

    // Stake (1_000_000) >= minimum (500_000) → allowed.
    let result = client.check_signal_submission_allowed(&provider);
    assert!(result.is_ok());
}

#[test]
fn notify_stake_below_minimum_emits_event() {
    use soroban_sdk::testutils::Events;
    let (env, vault_id, token, _admin) = setup();
    let provider = Address::generate(&env);

    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);

    let events_before = env.events().all().len();
    client.notify_stake_below_minimum(&provider);
    assert!(env.events().all().len() > events_before, "event not emitted");
}

#[test]
fn signal_submission_blocked_after_grace_period_expires() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin) = setup();
    let provider = Address::generate(&env);

    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);

    // Record the drop.
    client.notify_stake_below_minimum(&provider);

    // Advance time past the 24h grace period.
    env.ledger().with_mut(|l| l.timestamp += 86_401);

    let result = client.check_signal_submission_allowed(&provider);
    assert_eq!(result, Err(StakeVaultError::StakeBelowMinimum));
}

#[test]
fn signal_submission_allowed_within_grace_period() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin) = setup();
    let provider = Address::generate(&env);

    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);

    client.notify_stake_below_minimum(&provider);

    // Advance time within the grace period (12h).
    env.ledger().with_mut(|l| l.timestamp += 43_200);

    let result = client.check_signal_submission_allowed(&provider);
    assert!(result.is_ok(), "should be allowed within grace period");
}

#[test]
fn stake_restoration_clears_below_min_flag() {
    let (env, vault_id, token, _admin) = setup();
    let provider = Address::generate(&env);

    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);

    client.notify_stake_below_minimum(&provider);
    assert!(client.get_stake_below_min_since(&provider).is_some());

    // Restore stake above minimum.
    seed_v2_stake(&env, &vault_id, &provider, 1_000_000, 0);

    // check_signal_submission_allowed should clear the flag and return Ok.
    let result = client.check_signal_submission_allowed(&provider);
    assert!(result.is_ok());
    assert!(client.get_stake_below_min_since(&provider).is_none());
}
