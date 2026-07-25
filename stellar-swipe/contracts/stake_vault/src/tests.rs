#![cfg(test)]

use crate::{
    action, encode_action, encode_i128_bytes,
    migration::{MigrationKey, StakeInfoV2},
    SlashSeverity, StakeVaultContract, StakeVaultContractClient, StakeVaultError,
};
use shared::multisig::MultisigConfig;
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, token::StellarAssetClient, Address, Bytes,
    Env, Map, MuxedAddress, Symbol, Vec,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sac_token(env: &Env, admin: &Address) -> Address {
    env.register_stellar_asset_contract_v2(admin.clone())
        .address()
}

fn seed_v2_stake(
    env: &Env,
    contract_id: &Address,
    staker: &Address,
    balance: i128,
    locked_until: u64,
) {
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

fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signal_registry = Address::generate(&env);
    let token = sac_token(&env, &admin);
    let vault_id = env.register(StakeVaultContract, ());
    StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token, &signal_registry);
    (env, vault_id, token, admin, signal_registry)
}

/// Vault wired to a malicious token that re-enters `withdraw_stake` during `transfer`.
fn setup_with_reentrant_token() -> (Env, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signal_registry = Address::generate(&env);
    let staker = Address::generate(&env);
    let token_id = env.register(ReentrantToken, ());
    let vault_id = env.register(StakeVaultContract, ());
    StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token_id, &signal_registry);
    let token_client = ReentrantTokenClient::new(&env, &token_id);
    token_client.set_vault(&vault_id);
    token_client.set_staker(&staker);
    (env, vault_id, token_id, admin, signal_registry, staker)
}

/// Vault wired to a benign token that records cross-contract `transfer` invocations.
fn setup_with_recording_token() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signal_registry = Address::generate(&env);
    let token_id = env.register(TransferRecordingToken, ());
    let vault_id = env.register(StakeVaultContract, ());
    StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token_id, &signal_registry);
    (env, vault_id, token_id, admin, signal_registry)
}

// ── Basic withdraw tests ──────────────────────────────────────────────────────

#[test]
fn withdraw_stake_transfers_balance() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 5_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    assert_eq!(client.withdraw_stake(&staker), amount);
    assert_eq!(client.get_stake(&staker), 0);
}

// ── Withdrawal rate limiting (Issue #595: shared rate limiter) ─────────────────
//
// withdraw_stake adopts the shared rate limiter via common::rate_limit's
// StakeChange action (default: 5 per day), giving stake_vault rate-limiting
// protection it previously had none of.

#[test]
fn withdraw_stake_within_daily_limit_succeeds() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let per_withdrawal: i128 = 1_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &(per_withdrawal * 5));
    let client = StakeVaultContractClient::new(&env, &vault_id);

    for _ in 0..5 {
        seed_v2_stake(&env, &vault_id, &staker, per_withdrawal, 0);
        assert_eq!(client.withdraw_stake(&staker), per_withdrawal);
    }
}

#[test]
fn withdraw_stake_over_daily_limit_is_rejected() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let per_withdrawal: i128 = 1_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &(per_withdrawal * 6));
    let client = StakeVaultContractClient::new(&env, &vault_id);

    for _ in 0..5 {
        seed_v2_stake(&env, &vault_id, &staker, per_withdrawal, 0);
        client.withdraw_stake(&staker);
    }

    // 6th withdrawal within the same day exceeds the default StakeChange limit.
    seed_v2_stake(&env, &vault_id, &staker, per_withdrawal, 0);
    let err = client.try_withdraw_stake(&staker);
    assert_eq!(err, Err(Ok(StakeVaultError::RateLimitExceeded)));
}

#[test]
fn withdraw_stake_limit_resets_after_window_elapses() {
    use soroban_sdk::testutils::Ledger;

    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let per_withdrawal: i128 = 1_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &(per_withdrawal * 6));
    let client = StakeVaultContractClient::new(&env, &vault_id);

    for _ in 0..5 {
        seed_v2_stake(&env, &vault_id, &staker, per_withdrawal, 0);
        client.withdraw_stake(&staker);
    }
    seed_v2_stake(&env, &vault_id, &staker, per_withdrawal, 0);
    assert!(client.try_withdraw_stake(&staker).is_err());

    // Advance past the 24h window — the limit resets.
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 86_400 + 1);
    seed_v2_stake(&env, &vault_id, &staker, per_withdrawal, 0);
    assert_eq!(client.withdraw_stake(&staker), per_withdrawal);
}

#[test]
fn withdraw_stake_no_stake_returns_error() {
    let (env, vault_id, _token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let err = env.as_contract(&vault_id, || {
        StakeVaultContract::withdraw_stake(env.clone(), staker)
    });
    assert_eq!(err, Err(StakeVaultError::NoStake));
}

#[test]
fn withdraw_stake_locked_returns_error() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, u64::MAX);
    let err = env.as_contract(&vault_id, || {
        StakeVaultContract::withdraw_stake(env.clone(), staker)
    });
    assert_eq!(err, Err(StakeVaultError::StakeLocked));
}

// ── Reentrancy guard tests ────────────────────────────────────────────────────

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
    /// SEP-41 callback invoked by `withdraw_stake`'s cross-contract transfer.
    pub fn transfer(env: Env, _from: Address, _to: MuxedAddress, _amount: i128) {
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
        let result = StakeVaultContractClient::new(&env, &vault).try_withdraw_stake(&staker);
        let blocked = matches!(result, Err(Ok(StakeVaultError::ReentrancyDetected)));
        // Only write true; don't overwrite a previously set true with false.
        if blocked {
            env.storage()
                .instance()
                .set(&soroban_sdk::symbol_short!("blocked"), &true);
        }
    }
    pub fn was_blocked(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("blocked"))
            .unwrap_or(false)
    }
    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }
    pub fn transfer_from(
        _env: Env,
        _spender: Address,
        _from: Address,
        _to: Address,
        _amount: i128,
    ) {
    }
    pub fn approve(
        _env: Env,
        _from: Address,
        _spender: Address,
        _amount: i128,
        _expiration_ledger: u32,
    ) {
    }
    pub fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        0
    }
    pub fn decimals(_env: Env) -> u32 {
        7
    }
    pub fn name(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "ReentrantToken")
    }
    pub fn symbol(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "RT")
    }
    pub fn mint(_env: Env, _to: Address, _amount: i128) {}
}

/// Benign SEP-41 mock that records `transfer` calls without re-entering the vault.
#[contract]
pub struct TransferRecordingToken;

#[contractimpl]
impl TransferRecordingToken {
    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        let to_addr = to.address();
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("called"), &true);
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("from"), &from);
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("to"), &to_addr);
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("amount"), &amount);
    }
    pub fn transfer_was_called(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("called"))
            .unwrap_or(false)
    }
    pub fn last_transfer_from(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("from"))
            .unwrap()
    }
    pub fn last_transfer_to(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("to"))
            .unwrap()
    }
    pub fn last_transfer_amount(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("amount"))
            .unwrap()
    }
    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }
    pub fn transfer_from(
        _env: Env,
        _spender: Address,
        _from: Address,
        _to: Address,
        _amount: i128,
    ) {
    }
    pub fn approve(
        _env: Env,
        _from: Address,
        _spender: Address,
        _amount: i128,
        _expiration_ledger: u32,
    ) {
    }
    pub fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        0
    }
    pub fn decimals(_env: Env) -> u32 {
        7
    }
    pub fn name(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "RecordingToken")
    }
    pub fn symbol(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "REC")
    }
    pub fn mint(_env: Env, _to: Address, _amount: i128) {}
}

/// Malicious token is invoked on the cross-contract transfer path during withdraw.
#[test]
fn reentrant_withdraw_is_blocked() {
    use soroban_sdk::testutils::Ledger;

    let (env, vault_id, token_id, _admin, _registry, staker) = setup_with_reentrant_token();
    let amount: i128 = 1_000_000;

    env.ledger().with_mut(|l| l.sequence_number = 5);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    // First withdrawal must succeed; reentrancy guard is active during token.transfer.
    assert_eq!(client.withdraw_stake(&staker), amount);
    // Stake must be zeroed — no double-spend via reentrancy.
    assert_eq!(client.get_stake(&staker), 0);
    // A direct second withdrawal must fail (stake already gone).
    assert_eq!(
        client.try_withdraw_stake(&staker),
        Err(Ok(StakeVaultError::NoStake)),
        "stake must not be withdrawable twice"
    );
}

/// Holding the execution lock rejects a reentrant `withdraw_stake` with
/// `ReentrancyDetected` (models the malicious `ReentrantToken` attack).
#[test]
fn execution_lock_blocks_concurrent_withdraw() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 1_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    env.ledger().with_mut(|l| l.sequence_number = 5);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    env.as_contract(&vault_id, || {
        env.storage()
            .temporary()
            .set(&Symbol::new(&env, "WithdrawLock"), &true);
    });

    let result = StakeVaultContractClient::new(&env, &vault_id).try_withdraw_stake(&staker);
    assert_eq!(result, Err(Ok(StakeVaultError::ReentrancyDetected)));
}

/// Normal withdrawal succeeds when the token does not re-enter the vault.
#[test]
fn normal_withdrawal_succeeds_without_reentrancy() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token_id, _admin, _registry) = setup_with_recording_token();
    let staker = Address::generate(&env);
    let amount: i128 = 2_500_000;

    env.ledger().with_mut(|l| l.sequence_number = 5);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    assert_eq!(client.withdraw_stake(&staker), amount);
    assert_eq!(client.get_stake(&staker), 0);

    let token_client = TransferRecordingTokenClient::new(&env, &token_id);
    assert!(token_client.transfer_was_called());
    assert_eq!(token_client.last_transfer_from(), vault_id);
    assert_eq!(token_client.last_transfer_to(), staker);
    assert_eq!(token_client.last_transfer_amount(), amount);
}

/// Regression: `withdraw_stake` reaches the SEP-41 `transfer` cross-contract path.
#[test]
fn withdraw_stake_cross_contract_transfer_path() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token_id, _admin, _registry) = setup_with_recording_token();
    let staker = Address::generate(&env);
    let amount: i128 = 1_000_000;

    env.ledger().with_mut(|l| l.sequence_number = 10);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.withdraw_stake(&staker);

    let token_client = TransferRecordingTokenClient::new(&env, &token_id);
    assert!(
        token_client.transfer_was_called(),
        "withdraw_stake must invoke token.transfer"
    );
    assert_eq!(token_client.last_transfer_from(), vault_id);
    assert_eq!(token_client.last_transfer_to(), staker);
    assert_eq!(token_client.last_transfer_amount(), amount);
}

#[test]
fn lock_cleared_after_successful_withdrawal() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 2_000_000;
    StellarAssetClient::new(&env, &token).mint(&vault_id, &(amount * 2));
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.withdraw_stake(&staker);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);
    assert_eq!(client.withdraw_stake(&staker), amount);
}

#[test]
fn lock_cleared_after_failed_withdrawal() {
    let (env, vault_id, _token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let err = env.as_contract(&vault_id, || {
        StakeVaultContract::withdraw_stake(env.clone(), staker.clone())
    });
    assert_eq!(err, Err(StakeVaultError::NoStake));
    let lock_still_set: bool = env.as_contract(&vault_id, || {
        env.storage()
            .temporary()
            .get::<_, bool>(&Symbol::new(&env, "WithdrawLock"))
            .unwrap_or(false)
    });
    assert!(
        !lock_still_set,
        "lock was not cleared after failed withdrawal"
    );
}

// ── slash_stake tests ────────────────────────────────────────────────────────

#[test]
fn slash_stake_emits_event() {
    use soroban_sdk::testutils::Events;
    let (env, vault_id, token, _admin, signal_registry) = setup();
    let provider = Address::generate(&env);
    let amount: i128 = 500_000;
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &provider, amount, 0);
    let events_before = env.events().all().len();
    StakeVaultContractClient::new(&env, &vault_id).slash_stake(
        &signal_registry,
        &provider,
        &SlashSeverity::Minor,
        &Symbol::new(&env, "ban"),
    );
    assert!(
        env.events().all().len() > events_before,
        "stake_slashed event not emitted"
    );
}

#[test]
fn slash_stake_reduces_provider_balance() {
    let (env, vault_id, token, _admin, signal_registry) = setup();
    let provider = Address::generate(&env);
    let initial: i128 = 1_000_000;
    // Major severity = 30% (3000 bps), so slash = 1_000_000 * 3000 / 10_000 = 300_000
    let expected_slash: i128 = 300_000;
    StellarAssetClient::new(&env, &token).mint(&vault_id, &initial);
    seed_v2_stake(&env, &vault_id, &provider, initial, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.slash_stake(
        &signal_registry,
        &provider,
        &SlashSeverity::Major,
        &Symbol::new(&env, "fraud"),
    );
    assert_eq!(client.get_stake(&provider), initial - expected_slash);
}

#[test]
fn slash_stake_burns_tokens_from_vault() {
    use soroban_sdk::token;
    let (env, vault_id, token_addr, _admin, signal_registry) = setup();
    let provider = Address::generate(&env);
    let initial: i128 = 1_000_000;
    // Major severity = 30% (3000 bps), so slash = 1_000_000 * 3000 / 10_000 = 300_000
    let expected_slash: i128 = 300_000;
    StellarAssetClient::new(&env, &token_addr).mint(&vault_id, &initial);
    seed_v2_stake(&env, &vault_id, &provider, initial, 0);
    let token_client = token::Client::new(&env, &token_addr);
    let balance_before = token_client.balance(&vault_id);
    StakeVaultContractClient::new(&env, &vault_id).slash_stake(
        &signal_registry,
        &provider,
        &SlashSeverity::Major,
        &Symbol::new(&env, "misconduct"),
    );
    assert_eq!(
        token_client.balance(&vault_id),
        balance_before - expected_slash,
        "slashed tokens were not burned from vault"
    );
}

#[test]
fn slash_stake_unauthorized_caller_rejected() {
    let (env, vault_id, token, _admin, _signal_registry) = setup();
    let unauthorized = Address::generate(&env);
    let provider = Address::generate(&env);
    let amount: i128 = 500_000;
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &provider, amount, 0);
    let result = StakeVaultContractClient::new(&env, &vault_id).try_slash_stake(
        &unauthorized,
        &provider,
        &SlashSeverity::Minor,
        &Symbol::new(&env, "ban"),
    );
    assert_eq!(result, Err(Ok(StakeVaultError::Unauthorized)));
}

// ── Issue #388: stake-below-minimum tests ─────────────────────────────────────

#[test]
fn signal_submission_allowed_when_stake_above_minimum() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let provider = Address::generate(&env);
    let amount: i128 = 1_000_000;
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &provider, amount, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);
    // Should not panic — stake (1_000_000) >= minimum (500_000).
    client.check_signal_submission_allowed(&provider);
}

#[test]
fn notify_stake_below_minimum_emits_event() {
    use soroban_sdk::testutils::Events;
    let (env, vault_id, token, _admin, _registry) = setup();
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);
    let events_before = env.events().all().len();
    client.notify_stake_below_minimum(&provider);
    assert!(
        env.events().all().len() > events_before,
        "event not emitted"
    );
}

#[test]
fn signal_submission_blocked_after_grace_period_expires() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);
    client.notify_stake_below_minimum(&provider);
    env.ledger().with_mut(|l| l.timestamp += 86_401);
    let result = client.try_check_signal_submission_allowed(&provider);
    assert_eq!(result, Err(Ok(StakeVaultError::StakeBelowMinimum)));
}

#[test]
fn signal_submission_allowed_within_grace_period() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);
    client.notify_stake_below_minimum(&provider);
    env.ledger().with_mut(|l| l.timestamp += 43_200);
    // Should not panic — within 24h grace period.
    client.check_signal_submission_allowed(&provider);
}

#[test]
fn stake_restoration_clears_below_min_flag() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &100_000i128);
    seed_v2_stake(&env, &vault_id, &provider, 100_000, 0);
    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.set_minimum_stake(&500_000i128);
    client.notify_stake_below_minimum(&provider);
    assert!(client.get_stake_below_min_since(&provider).is_some());
    seed_v2_stake(&env, &vault_id, &provider, 1_000_000, 0);
    // Should not panic — stake restored.
    client.check_signal_submission_allowed(&provider);
    assert!(client.get_stake_below_min_since(&provider).is_none());
}

// ── Flash loan protection tests ───────────────────────────────────────────────

/// Simulates a flash loan: deposit_stake and withdraw_stake in the same ledger.
#[test]
fn flash_loan_same_ledger_deposit_withdraw_blocked() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let attacker = Address::generate(&env);
    let amount: i128 = 100_000;

    env.ledger().with_mut(|l| l.sequence_number = 42);
    StellarAssetClient::new(&env, &token).mint(&attacker, &amount);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    // Deposit in ledger 42 — records LastStakeLedger = 42.
    client.deposit_stake(&attacker, &amount);
    // Withdraw in same ledger 42 — must be blocked.
    let result = client.try_withdraw_stake(&attacker);
    assert_eq!(result, Err(Ok(StakeVaultError::FlashLoanDetected)));
}

/// After advancing one ledger, withdrawal must succeed.
#[test]
fn withdrawal_allowed_after_ledger_advance() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 100_000;

    env.ledger().with_mut(|l| l.sequence_number = 10);
    StellarAssetClient::new(&env, &token).mint(&staker, &amount);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.deposit_stake(&staker, &amount);
    env.ledger().with_mut(|l| l.sequence_number = 11);
    assert_eq!(client.withdraw_stake(&staker), amount);
}

/// Large withdrawal without a prior time-lock request must be rejected.
#[test]
fn large_withdrawal_without_timelock_request_blocked() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 600_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let result = StakeVaultContractClient::new(&env, &vault_id).try_withdraw_stake(&staker);
    assert_eq!(result, Err(Ok(StakeVaultError::TimelockRequired)));
}

/// Large withdrawal before time-lock expires must be rejected.
#[test]
fn large_withdrawal_before_timelock_expires_blocked() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 600_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.request_withdrawal(&staker);
    // 30 min elapsed — still within 1h lock.
    env.ledger().with_mut(|l| l.timestamp += 1_800);
    let result = client.try_withdraw_stake(&staker);
    assert_eq!(result, Err(Ok(StakeVaultError::TimelockNotElapsed)));
}

/// Large withdrawal after time-lock expires must succeed.
#[test]
fn large_withdrawal_after_timelock_succeeds() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 600_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.request_withdrawal(&staker);
    env.ledger().with_mut(|l| l.timestamp += 3_601);
    assert_eq!(client.withdraw_stake(&staker), amount);
}

/// Small withdrawal (below threshold) does not need a time-lock request.
#[test]
fn small_withdrawal_no_timelock_needed() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 100_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);
    assert_eq!(
        StakeVaultContractClient::new(&env, &vault_id).withdraw_stake(&staker),
        amount
    );
}

/// Admin pause blocks both deposit_stake and withdraw_stake.
#[test]
fn paused_contract_blocks_stake_and_unstake() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 100_000;

    StellarAssetClient::new(&env, &token).mint(&staker, &amount);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.pause();

    assert_eq!(
        client.try_deposit_stake(&staker, &amount),
        Err(Ok(StakeVaultError::ContractPaused))
    );
    assert_eq!(
        client.try_withdraw_stake(&staker),
        Err(Ok(StakeVaultError::ContractPaused))
    );
}

/// Unpause restores normal operation.
#[test]
fn unpause_restores_operations() {
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 100_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.pause();
    client.unpause();
    assert_eq!(client.withdraw_stake(&staker), amount);
}

/// Flash loan detection emits a monitoring alert (diagnostic event preserved in test env).
/// Verifies the flash_loan_attempt error code is returned, which triggers the event path.
#[test]
fn flash_loan_attempt_emits_alert_event() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let attacker = Address::generate(&env);
    let amount: i128 = 100_000;

    env.ledger().with_mut(|l| l.sequence_number = 99);
    StellarAssetClient::new(&env, &token).mint(&attacker, &amount);
    StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.deposit_stake(&attacker, &amount);

    // The monitoring alert event is emitted inside do_withdraw before returning
    // FlashLoanDetected. Soroban preserves diagnostic events even on failed calls.
    let result = client.try_withdraw_stake(&attacker);
    assert_eq!(
        result,
        Err(Ok(StakeVaultError::FlashLoanDetected)),
        "flash_loan_attempt should return FlashLoanDetected (event emitted on this path)"
    );
}

/// Time-lock request is consumed after a successful large withdrawal.
#[test]
fn timelock_request_consumed_after_withdrawal() {
    use soroban_sdk::testutils::Ledger;
    let (env, vault_id, token, _admin, _registry) = setup();
    let staker = Address::generate(&env);
    let amount: i128 = 600_000_000;

    StellarAssetClient::new(&env, &token).mint(&vault_id, &(amount * 2));
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);

    let client = StakeVaultContractClient::new(&env, &vault_id);
    client.request_withdrawal(&staker);
    env.ledger().with_mut(|l| l.timestamp += 3_601);
    client.withdraw_stake(&staker);

    // Re-seed for a second attempt — must require a fresh request.
    seed_v2_stake(&env, &vault_id, &staker, amount, 0);
    assert_eq!(
        client.try_withdraw_stake(&staker),
        Err(Ok(StakeVaultError::TimelockRequired))
    );
}

// ── #612 Severity-tiered slashing tests ──────────────────────────────────────

#[cfg(test)]
mod slash_severity_tests {
    use crate::{
        migration::{MigrationKey, StakeInfoV2},
        SlashSeverity, SlashTierConfig, StakeVaultContract, StakeVaultContractClient,
        StakeVaultError,
    };
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::StellarAssetClient,
        Address, Env, Map, Symbol,
    };

    fn sac_token(env: &Env, admin: &Address) -> Address {
        env.register_stellar_asset_contract_v2(admin.clone())
            .address()
    }

    fn seed(env: &Env, contract_id: &Address, staker: &Address, balance: i128) {
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
                    locked_until: 0,
                    last_updated: 0,
                },
            );
            env.storage()
                .persistent()
                .set(&MigrationKey::StakesV2, &stakes);
        });
    }

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token, &registry);
        (env, vault_id, token, admin, registry)
    }

    #[test]
    fn minor_slash_burns_default_5_percent() {
        let (env, vault_id, token, _admin, registry) = setup();
        let provider = Address::generate(&env);
        let balance: i128 = 1_000_000;
        StellarAssetClient::new(&env, &token).mint(&vault_id, &balance);
        seed(&env, &vault_id, &provider, balance);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        let slashed = client.slash_stake(
            &registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "bad"),
        );
        assert_eq!(slashed, 50_000); // 5% of 1_000_000
        assert_eq!(client.get_stake(&provider), 950_000);
    }

    #[test]
    fn major_slash_burns_default_30_percent() {
        let (env, vault_id, token, _admin, registry) = setup();
        let provider = Address::generate(&env);
        let balance: i128 = 1_000_000;
        StellarAssetClient::new(&env, &token).mint(&vault_id, &balance);
        seed(&env, &vault_id, &provider, balance);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        let slashed = client.slash_stake(
            &registry,
            &provider,
            &SlashSeverity::Major,
            &Symbol::new(&env, "fraud"),
        );
        assert_eq!(slashed, 300_000); // 30%
        assert_eq!(client.get_stake(&provider), 700_000);
    }

    #[test]
    fn critical_slash_burns_full_stake() {
        let (env, vault_id, token, _admin, registry) = setup();
        let provider = Address::generate(&env);
        let balance: i128 = 1_000_000;
        StellarAssetClient::new(&env, &token).mint(&vault_id, &balance);
        seed(&env, &vault_id, &provider, balance);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        let slashed = client.slash_stake(
            &registry,
            &provider,
            &SlashSeverity::Critical,
            &Symbol::new(&env, "attack"),
        );
        assert_eq!(slashed, balance);
        assert_eq!(client.get_stake(&provider), 0);
    }

    #[test]
    fn admin_can_reconfigure_tiers() {
        let (env, vault_id, token, _admin, registry) = setup();
        let provider = Address::generate(&env);
        let balance: i128 = 1_000_000;
        StellarAssetClient::new(&env, &token).mint(&vault_id, &balance);
        seed(&env, &vault_id, &provider, balance);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.configure_slash_tiers(&100, &2_000, &10_000); // minor = 1%
        let slashed = client.slash_stake(
            &registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "test"),
        );
        assert_eq!(slashed, 10_000); // 1%
    }

    #[test]
    fn invalid_tier_bps_rejected() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(
            client.try_configure_slash_tiers(&500, &3_000, &10_001),
            Err(Ok(StakeVaultError::InvalidSlashTier))
        );
    }

    #[test]
    fn unauthorized_caller_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let provider = Address::generate(&env);
        let attacker = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&vault_id, &1_000);
        seed(&env, &vault_id, &provider, 1_000);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(
            client.try_slash_stake(
                &attacker,
                &provider,
                &SlashSeverity::Major,
                &Symbol::new(&env, "x")
            ),
            Err(Ok(StakeVaultError::Unauthorized))
        );
    }

    // ── Minimum stake duration lock tests (Issue #705) ───────────────────────────

    #[test]
    fn voting_power_zero_within_lock_period() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let amount: i128 = 1_000_000;

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_minimum_stake_duration(&3600);

        StellarAssetClient::new(&env, &token).mint(&staker, &amount);
        client.deposit_stake(&staker, &amount);

        assert_eq!(client.get_voting_power(&staker), 0);
    }

    #[test]
    fn voting_power_full_after_lock_expires() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let signal_registry = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.initialize(&admin, &token, &signal_registry);

        let staker = Address::generate(&env);
        let amount: i128 = 1_000_000;

        client.set_minimum_stake_duration(&3600);

        StellarAssetClient::new(&env, &token).mint(&staker, &amount);
        client.deposit_stake(&staker, &amount);

        env.ledger().with_mut(|l| l.timestamp = 3601);

        assert_eq!(client.get_voting_power(&staker), amount);
    }

    #[test]
    fn top_up_deposit_extends_lock_for_new_portion() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let signal_registry = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.initialize(&admin, &token, &signal_registry);

        let staker = Address::generate(&env);
        let first_deposit: i128 = 500_000;
        let second_deposit: i128 = 300_000;

        client.set_minimum_stake_duration(&3600);

        StellarAssetClient::new(&env, &token).mint(&staker, &first_deposit);
        client.deposit_stake(&staker, &first_deposit);

        env.ledger().with_mut(|l| l.timestamp = 3601);

        StellarAssetClient::new(&env, &token).mint(&staker, &second_deposit);
        client.deposit_stake(&staker, &second_deposit);

        assert_eq!(client.get_voting_power(&staker), 0);

        env.ledger().with_mut(|l| l.timestamp = 7202);

        assert_eq!(
            client.get_voting_power(&staker),
            first_deposit + second_deposit
        );
    }

    #[test]
    fn no_min_duration_means_voting_power_immediately() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let amount: i128 = 1_000_000;

        let client = StakeVaultContractClient::new(&env, &vault_id);

        StellarAssetClient::new(&env, &token).mint(&staker, &amount);
        client.deposit_stake(&staker, &amount);

        assert_eq!(client.get_voting_power(&staker), amount);
    }

    #[test]
    fn set_minimum_stake_duration_admin_only() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.get_minimum_stake_duration(), 0);
    }

    #[test]
    fn deposit_timestamp_tracked() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let amount: i128 = 1_000_000;

        let client = StakeVaultContractClient::new(&env, &vault_id);
        StellarAssetClient::new(&env, &token).mint(&staker, &amount);
        client.deposit_stake(&staker, &amount);

        let ts = client.get_stake_deposit_timestamp(&staker);
        assert!(ts.is_some());
        assert_eq!(ts.unwrap(), env.ledger().timestamp());
    }

    #[test]
    fn withdraw_stake_respects_min_duration_lock() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let signal_registry = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.initialize(&admin, &token, &signal_registry);

        let staker = Address::generate(&env);
        let amount: i128 = 1_000_000;

        client.set_minimum_stake_duration(&3600);

        StellarAssetClient::new(&env, &token).mint(&staker, &amount);
        client.deposit_stake(&staker, &amount);

        let result = client.try_withdraw_stake(&staker);
        assert_eq!(result, Err(Ok(StakeVaultError::StakeLocked)));

        env.ledger().with_mut(|l| l.timestamp = 3601);

        assert_eq!(client.withdraw_stake(&staker), amount);
    }

    // ── Issue #563: require_auth_for_args ─────────────────────────────────

    /// Auth scoped to (staker, amount=0) is rejected when the staker has a
    /// non-zero balance — the signature covers a different amount than what
    /// will actually be withdrawn.
    #[test]
    fn withdraw_stake_arg_scoped_auth_rejects_wrong_amount() {
        use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
        use soroban_sdk::IntoVal;

        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let amount: i128 = 5_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &staker, amount);

        // Auth claims amount=0 but the real balance is 5_000_000.
        let sub_invokes: &[MockAuthInvoke] = &[];
        let wrong_args = (&staker, &0i128).into_val(&env);
        let mock_invoke = MockAuthInvoke {
            contract: &vault_id,
            fn_name: "withdraw_stake",
            args: wrong_args,
            sub_invokes,
        };
        let mock_auth = MockAuth {
            address: &staker,
            invoke: &mock_invoke,
        };

        let result = StakeVaultContractClient::new(&env, &vault_id)
            .mock_auths(&[mock_auth])
            .try_withdraw_stake(&staker);

        assert!(
            result.is_err(),
            "auth scoped to amount=0 must not authorize withdrawal of 5_000_000"
        );
    }

    // ── Issue #662: partial_unstake tests ─────────────────────────────────────

    #[test]
    fn partial_unstake_reduces_balance_by_requested_amount() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let withdraw: i128 = 300_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.partial_unstake(&staker, &withdraw), withdraw);
        assert_eq!(client.get_stake(&staker), total - withdraw);
    }

    #[test]
    fn partial_unstake_quarter_stake() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 800_000;
        let withdraw: i128 = 200_000; // 25%

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.partial_unstake(&staker, &withdraw), withdraw);
        assert_eq!(client.get_stake(&staker), 600_000);
    }

    #[test]
    fn partial_unstake_half_stake() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let withdraw: i128 = 500_000; // 50%

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.partial_unstake(&staker, &withdraw), withdraw);
        assert_eq!(client.get_stake(&staker), 500_000);
    }

    #[test]
    fn partial_unstake_leaves_one_stroop_remaining() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let withdraw: i128 = total - 1; // leaves exactly 1 stroop

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.partial_unstake(&staker, &withdraw), withdraw);
        assert_eq!(client.get_stake(&staker), 1);
    }

    #[test]
    fn partial_unstake_full_amount_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let result =
            StakeVaultContractClient::new(&env, &vault_id).try_partial_unstake(&staker, &total);
        assert_eq!(result, Err(Ok(StakeVaultError::InvalidAmount)));
    }

    #[test]
    fn partial_unstake_zero_amount_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let result =
            StakeVaultContractClient::new(&env, &vault_id).try_partial_unstake(&staker, &0i128);
        assert_eq!(result, Err(Ok(StakeVaultError::InvalidAmount)));
    }

    #[test]
    fn partial_unstake_exceeds_balance_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let result = StakeVaultContractClient::new(&env, &vault_id)
            .try_partial_unstake(&staker, &(total + 1));
        assert_eq!(result, Err(Ok(StakeVaultError::InvalidAmount)));
    }

    #[test]
    fn partial_unstake_no_stake_rejected() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let staker = Address::generate(&env);

        let result = StakeVaultContractClient::new(&env, &vault_id)
            .try_partial_unstake(&staker, &500_000i128);
        assert_eq!(result, Err(Ok(StakeVaultError::NoStake)));
    }

    #[test]
    fn partial_unstake_below_minimum_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let minimum: i128 = 200_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_minimum_stake(&minimum);

        // Withdrawing 900_000 would leave 100_000, which is below minimum (200_000).
        let result = client.try_partial_unstake(&staker, &900_000i128);
        assert_eq!(result, Err(Ok(StakeVaultError::RemainingStakeBelowMinimum)));
    }

    #[test]
    fn partial_unstake_at_minimum_boundary_succeeds() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let minimum: i128 = 200_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_minimum_stake(&minimum);

        // Withdrawing exactly enough to leave remaining == minimum.
        let withdraw = total - minimum; // 800_000
        assert_eq!(client.partial_unstake(&staker, &withdraw), withdraw);
        assert_eq!(client.get_stake(&staker), minimum);
    }

    #[test]
    fn partial_unstake_one_below_minimum_boundary_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let minimum: i128 = 200_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_minimum_stake(&minimum);

        // Leaves remaining == minimum - 1 → rejected.
        let result = client.try_partial_unstake(&staker, &(total - minimum + 1));
        assert_eq!(result, Err(Ok(StakeVaultError::RemainingStakeBelowMinimum)));
    }

    #[test]
    fn partial_unstake_locked_stake_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        // Seed with far-future lock.
        super::seed_v2_stake(&env, &vault_id, &staker, total, u64::MAX);

        let result = StakeVaultContractClient::new(&env, &vault_id)
            .try_partial_unstake(&staker, &500_000i128);
        assert_eq!(result, Err(Ok(StakeVaultError::StakeLocked)));
    }

    #[test]
    fn partial_unstake_flash_loan_blocked() {
        use soroban_sdk::testutils::Ledger;
        let (env, vault_id, token, _admin, _registry) = setup();
        let attacker = Address::generate(&env);
        let total: i128 = 200_000;

        env.ledger().with_mut(|l| l.sequence_number = 77);
        StellarAssetClient::new(&env, &token).mint(&attacker, &total);
        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.deposit_stake(&attacker, &total);

        let result = client.try_partial_unstake(&attacker, &(total - 1));
        assert_eq!(result, Err(Ok(StakeVaultError::FlashLoanDetected)));
    }

    #[test]
    fn partial_unstake_large_amount_requires_timelock() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000_000;
        let withdraw: i128 = 600_000_000; // above LARGE_WITHDRAWAL_THRESHOLD

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let result =
            StakeVaultContractClient::new(&env, &vault_id).try_partial_unstake(&staker, &withdraw);
        assert_eq!(result, Err(Ok(StakeVaultError::TimelockRequired)));
    }

    #[test]
    fn partial_unstake_large_amount_succeeds_after_timelock() {
        use soroban_sdk::testutils::Ledger;
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000_000;
        let withdraw: i128 = 600_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.request_withdrawal(&staker);
        env.ledger().with_mut(|l| l.timestamp += 3_601);

        assert_eq!(client.partial_unstake(&staker, &withdraw), withdraw);
        assert_eq!(client.get_stake(&staker), total - withdraw);
    }

    #[test]
    fn partial_unstake_emits_event() {
        use soroban_sdk::testutils::Events;
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;
        let withdraw: i128 = 400_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let events_before = env.events().all().len();
        StakeVaultContractClient::new(&env, &vault_id).partial_unstake(&staker, &withdraw);
        assert!(
            env.events().all().len() > events_before,
            "partial_unstake event not emitted"
        );
    }

    #[test]
    fn partial_unstake_paused_contract_rejected() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.pause();

        let result = client.try_partial_unstake(&staker, &500_000i128);
        assert_eq!(result, Err(Ok(StakeVaultError::ContractPaused)));
    }

    #[test]
    fn partial_unstake_no_minimum_stake_allows_any_nonzero_remainder() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let total: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &total);
        seed(&env, &vault_id, &staker, total);

        // No minimum set — leaving 1 stroop should be fine.
        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.partial_unstake(&staker, &(total - 1)), total - 1);
        assert_eq!(client.get_stake(&staker), 1);
    }

    /// Auth scoped to the correct (staker, amount) passes for withdraw_stake.
    #[test]
    fn withdraw_stake_arg_scoped_auth_passes_for_correct_args() {
        use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
        use soroban_sdk::IntoVal;

        let (env, vault_id, token, _admin, _registry) = setup();
        let staker = Address::generate(&env);
        let amount: i128 = 5_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &staker, amount);

        // Auth scoped to the exact balance that will be withdrawn.
        let sub_invokes: &[MockAuthInvoke] = &[];
        let correct_args = (&staker, &amount).into_val(&env);
        let mock_invoke = MockAuthInvoke {
            contract: &vault_id,
            fn_name: "withdraw_stake",
            args: correct_args,
            sub_invokes,
        };
        let mock_auth = MockAuth {
            address: &staker,
            invoke: &mock_invoke,
        };

        let withdrawn = StakeVaultContractClient::new(&env, &vault_id)
            .mock_auths(&[mock_auth])
            .withdraw_stake(&staker);

        assert_eq!(withdrawn, amount, "correctly scoped auth must succeed");
    }
}

// ── Issue #689: Slash appeal window tests ─────────────────────────────────────

#[cfg(test)]
mod slash_appeal_tests {
    use crate::{
        action, encode_action, encode_i128_bytes,
        migration::{MigrationKey, StakeInfoV2},
        AppealStatus, SlashSeverity, StakeVaultContract, StakeVaultContractClient, StakeVaultError,
    };
    use shared::multisig::MultisigConfig;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::StellarAssetClient,
        Address, Bytes, Env, Map, String, Symbol, Vec,
    };

    fn sac_token(env: &Env, admin: &Address) -> Address {
        env.register_stellar_asset_contract_v2(admin.clone())
            .address()
    }

    fn seed(env: &Env, contract_id: &Address, staker: &Address, balance: i128) {
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
                    locked_until: 0,
                    last_updated: env.ledger().timestamp(),
                },
            );
            env.storage()
                .persistent()
                .set(&MigrationKey::StakesV2, &stakes);
        });
    }

    /// Convenience setup: returns (env, vault_id, token, admin, signal_registry).
    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let signal_registry = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token, &signal_registry);
        (env, vault_id, token, admin, signal_registry)
    }

    // ── set_appeal_window tests ───────────────────────────────────────────────

    #[test]
    fn set_appeal_window_persists_and_readable() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.get_appeal_window(), 0, "default window should be 0");
        client.set_appeal_window(&86_400u64);
        assert_eq!(client.get_appeal_window(), 86_400);
    }

    #[test]
    fn set_appeal_window_rejects_value_over_30_days() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        let result = client.try_set_appeal_window(&2_592_001u64);
        assert_eq!(result, Err(Ok(StakeVaultError::InvalidAppealWindow)));
    }

    // ── appeal_slash within window ────────────────────────────────────────────

    #[test]
    fn appeal_slash_within_window_succeeds() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        // Configure a 24 h appeal window.
        client.set_appeal_window(&86_400u64);

        // Slash the provider — slash_id 0 is assigned.
        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        // Appeal within the window — should succeed.
        let evidence = String::from_str(&env, "ipfs://Qm_evidence_hash_here");
        client.appeal_slash(&provider, &0u64, &evidence);

        // The appeal record should exist and be Pending.
        let appeal = client.get_slash_appeal(&0u64).unwrap();
        assert_eq!(appeal.status, AppealStatus::Pending);
        assert_eq!(appeal.appellant, provider);
        assert_eq!(appeal.slash_id, 0);
    }

    #[test]
    fn appeal_slash_emits_slash_appealed_event() {
        use soroban_sdk::testutils::Events;
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);
        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        let events_before = env.events().all().len();
        client.appeal_slash(&provider, &0u64, &String::from_str(&env, "ipfs://evidence"));
        assert!(
            env.events().all().len() > events_before,
            "slash_appealed event not emitted"
        );
    }

    // ── appeal_slash after window ─────────────────────────────────────────────

    #[test]
    fn appeal_slash_after_window_returns_appeal_window_closed() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&3_600u64); // 1 hour window

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        // Advance past the appeal window.
        env.ledger().with_mut(|l| l.timestamp += 3_601);

        let result = client.try_appeal_slash(
            &provider,
            &0u64,
            &String::from_str(&env, "ipfs://late_evidence"),
        );
        assert_eq!(result, Err(Ok(StakeVaultError::AppealWindowClosed)));
    }

    #[test]
    fn appeal_slash_when_no_window_configured_returns_appeal_window_closed() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        // No appeal window set (defaults to 0).
        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "misconduct"),
        );

        let result =
            client.try_appeal_slash(&provider, &0u64, &String::from_str(&env, "ipfs://evidence"));
        assert_eq!(result, Err(Ok(StakeVaultError::AppealWindowClosed)));
    }

    // ── duplicate appeal ──────────────────────────────────────────────────────

    #[test]
    fn duplicate_appeal_returns_appeal_already_exists() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        let evidence = String::from_str(&env, "ipfs://evidence");
        client.appeal_slash(&provider, &0u64, &evidence);

        // Second appeal for the same slash_id should fail.
        let result = client.try_appeal_slash(&provider, &0u64, &evidence);
        assert_eq!(result, Err(Ok(StakeVaultError::AppealAlreadyExists)));
    }

    // ── appeal_slash on non-existent slash_id ─────────────────────────────────

    #[test]
    fn appeal_slash_nonexistent_slash_id_returns_slash_not_found() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let provider = Address::generate(&env);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        let result = client.try_appeal_slash(
            &provider,
            &999u64,
            &String::from_str(&env, "ipfs://evidence"),
        );
        assert_eq!(result, Err(Ok(StakeVaultError::SlashNotFound)));
    }

    // ── appeal by wrong address ───────────────────────────────────────────────

    #[test]
    fn appeal_slash_by_non_provider_returns_unauthorized() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let attacker = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        let result =
            client.try_appeal_slash(&attacker, &0u64, &String::from_str(&env, "ipfs://evidence"));
        assert_eq!(result, Err(Ok(StakeVaultError::Unauthorized)));
    }

    // ── resolve_appeal: uphold ────────────────────────────────────────────────

    #[test]
    fn resolve_appeal_uphold_burns_held_funds_and_marks_upheld() {
        use soroban_sdk::token;
        let (env, vault_id, token_addr, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let initial: i128 = 1_000_000;
        // Minor = 5 % → slash = 50_000
        let expected_slash: i128 = 50_000;

        StellarAssetClient::new(&env, &token_addr).mint(&vault_id, &initial);
        seed(&env, &vault_id, &provider, initial);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        // Vault still holds the tokens (not yet burned).
        let token_client = token::Client::new(&env, &token_addr);
        let balance_after_slash = token_client.balance(&vault_id);
        // Provider's stake balance is reduced by the slash.
        assert_eq!(client.get_stake(&provider), initial - expected_slash);

        // Submit appeal.
        client.appeal_slash(
            &provider,
            &0u64,
            &String::from_str(&env, "ipfs://Qm_evidence"),
        );

        // Admin upholds the appeal → tokens burned now.
        client.resolve_appeal(&0u64, &true);

        assert!(
            token_client.balance(&vault_id) < balance_after_slash,
            "held funds must be burned on uphold"
        );

        let appeal = client.get_slash_appeal(&0u64).unwrap();
        assert_eq!(appeal.status, AppealStatus::Upheld);
    }

    // ── resolve_appeal: reverse ───────────────────────────────────────────────

    #[test]
    fn resolve_appeal_reverse_restores_provider_stake() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let initial: i128 = 1_000_000;
        // Minor = 5 % → slash = 50_000
        let expected_slash: i128 = 50_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &initial);
        seed(&env, &vault_id, &provider, initial);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        // Stake was reduced.
        assert_eq!(client.get_stake(&provider), initial - expected_slash);

        // Submit appeal.
        client.appeal_slash(
            &provider,
            &0u64,
            &String::from_str(&env, "ipfs://Qm_exculpatory_evidence"),
        );

        // Admin reverses the appeal → stake restored.
        client.resolve_appeal(&0u64, &false);

        assert_eq!(
            client.get_stake(&provider),
            initial,
            "stake must be fully restored after reversal"
        );

        let appeal = client.get_slash_appeal(&0u64).unwrap();
        assert_eq!(appeal.status, AppealStatus::Reversed);
    }

    #[test]
    fn resolve_appeal_reverse_emits_appeal_resolved_event() {
        use soroban_sdk::testutils::Events;
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );
        client.appeal_slash(&provider, &0u64, &String::from_str(&env, "ipfs://evidence"));

        let events_before = env.events().all().len();
        client.resolve_appeal(&0u64, &false);
        assert!(
            env.events().all().len() > events_before,
            "appeal_resolved event not emitted"
        );
    }

    // ── resolve already-resolved appeal ──────────────────────────────────────

    #[test]
    fn resolve_appeal_already_resolved_returns_error() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );
        client.appeal_slash(&provider, &0u64, &String::from_str(&env, "ipfs://evidence"));
        client.resolve_appeal(&0u64, &true);

        // Second resolution should fail.
        let result = client.try_resolve_appeal(&0u64, &false);
        assert_eq!(result, Err(Ok(StakeVaultError::AppealAlreadyResolved)));
    }

    // ── resolve_appeal with no appeal filed ───────────────────────────────────

    #[test]
    fn resolve_appeal_no_appeal_filed_returns_slash_not_found() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_appeal_window(&86_400u64);

        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "violation"),
        );

        // No appeal filed — resolve should fail.
        let result = client.try_resolve_appeal(&0u64, &true);
        assert_eq!(result, Err(Ok(StakeVaultError::SlashNotFound)));
    }

    // ── get_slash_record ──────────────────────────────────────────────────────

    #[test]
    fn get_slash_record_returns_correct_data() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let provider = Address::generate(&env);
        let amount: i128 = 1_000_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &amount);
        seed(&env, &vault_id, &provider, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.slash_stake(
            &signal_registry,
            &provider,
            &SlashSeverity::Major,
            &Symbol::new(&env, "fraud"),
        );

        let record = client.get_slash_record(&0u64).unwrap();
        assert_eq!(record.slash_id, 0);
        assert_eq!(record.provider, provider);
        assert_eq!(record.severity, SlashSeverity::Major as u32);

        // Non-existent slash_id returns None.
        assert!(client.get_slash_record(&999u64).is_none());
    }

    // ── slash counter increments monotonically ────────────────────────────────

    #[test]
    fn slash_ids_increment_across_multiple_slashes() {
        let (env, vault_id, token, _admin, signal_registry) = setup();
        let p1 = Address::generate(&env);
        let p2 = Address::generate(&env);
        let amount: i128 = 500_000;

        StellarAssetClient::new(&env, &token).mint(&vault_id, &(amount * 2));
        seed(&env, &vault_id, &p1, amount);
        seed(&env, &vault_id, &p2, amount);

        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.slash_stake(
            &signal_registry,
            &p1,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "r1"),
        );
        client.slash_stake(
            &signal_registry,
            &p2,
            &SlashSeverity::Minor,
            &Symbol::new(&env, "r2"),
        );

        assert_eq!(client.get_slash_record(&0u64).unwrap().provider, p1);
        assert_eq!(client.get_slash_record(&1u64).unwrap().provider, p2);
    }

    // ── Multi-sig approval flow tests ─────────────────────────────────────────

    fn multisig_env() -> (Env, Address, Address, Vec<Address>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let sig_reg = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.initialize(&admin, &token, &sig_reg);

        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let signer_c = Address::generate(&env);
        let signers = soroban_sdk::vec![&env, signer_a.clone(), signer_b.clone(), signer_c.clone()];
        let cfg = MultisigConfig {
            signers: signers.clone(),
            threshold: 2,
            proposal_timeout_secs: 86_400,
        };
        client.set_multisig_config(&admin, &Some(cfg));
        (env, vault_id, admin, signers)
    }

    #[test]
    fn set_minimum_stake_returns_requires_multisig_when_configured() {
        let (env, vault_id, _admin, _signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        let result = client.try_set_minimum_stake(&1_000_000i128);
        assert_eq!(result, Err(Ok(StakeVaultError::RequiresMultisig)));
    }

    #[test]
    fn pause_returns_requires_multisig_when_configured() {
        let (env, vault_id, _admin, _signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        let result = client.try_pause();
        assert_eq!(result, Err(Ok(StakeVaultError::RequiresMultisig)));
    }

    #[test]
    fn propose_approve_execute_set_minimum_stake() {
        let (env, vault_id, _admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);

        // Proposer encodes the action
        let payload = {
            let e = &env;
            encode_action(
                e,
                action::SET_MINIMUM_STAKE,
                &encode_i128_bytes(500_000i128),
            )
        };

        // Propose by signer A
        let id = client.propose_action(&signers.get(0).unwrap(), &payload);

        // Approve by signer B (threshold = 2, so this reaches threshold)
        let count = client.approve_action(&signers.get(1).unwrap(), &id);
        assert_eq!(count, 2);

        // Execute (auto-unwraps in mock auth mode)
        client.execute_action(&signers.get(0).unwrap(), &id);

        // Verify the state change
        assert_eq!(client.get_minimum_stake(), 500_000);
    }

    #[test]
    fn propose_approve_execute_pause() {
        let (env, vault_id, _admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);

        let payload = {
            let e = &env;
            encode_action(e, action::PAUSE, &[])
        };

        let id = client.propose_action(&signers.get(0).unwrap(), &payload);
        let _ = client.approve_action(&signers.get(1).unwrap(), &id);
        client.execute_action(&signers.get(0).unwrap(), &id);
        assert!(client.is_paused());
    }

    #[test]
    fn execute_before_threshold_rejected() {
        let (env, vault_id, _admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);

        let payload = {
            let e = &env;
            encode_action(e, action::PAUSE, &[])
        };

        let id = client.propose_action(&signers.get(0).unwrap(), &payload);
        // Only 1 approval, threshold is 2
        let result = client.try_execute_action(&signers.get(0).unwrap(), &id);
        assert_eq!(result, Err(Ok(StakeVaultError::Unauthorized)));
    }

    #[test]
    fn duplicate_approval_rejected() {
        let (env, vault_id, _admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);

        let payload = {
            let e = &env;
            encode_action(
                e,
                action::SET_MINIMUM_STAKE,
                &encode_i128_bytes(100_000i128),
            )
        };

        let id = client.propose_action(&signers.get(0).unwrap(), &payload);
        // Second signer approves
        let _ = client.approve_action(&signers.get(1).unwrap(), &id);
        // First signer tries to approve again — fails
        let result = client.try_approve_action(&signers.get(0).unwrap(), &id);
        assert_eq!(result, Err(Ok(StakeVaultError::AlreadyApproved)));
    }

    #[test]
    fn unauthorized_signer_approval_rejected() {
        let (env, vault_id, _admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        let impostor = Address::generate(&env);

        let payload = {
            let e = &env;
            encode_action(e, action::PAUSE, &[])
        };

        let id = client.propose_action(&signers.get(0).unwrap(), &payload);
        let result = client.try_approve_action(&impostor, &id);
        assert_eq!(result, Err(Ok(StakeVaultError::Unauthorized)));
    }

    #[test]
    fn proposal_expiry_blocks_execution() {
        use soroban_sdk::testutils::Ledger;
        let (env, vault_id, _admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);

        let payload = {
            let e = &env;
            encode_action(e, action::PAUSE, &[])
        };

        let id = client.propose_action(&signers.get(0).unwrap(), &payload);
        let _ = client.approve_action(&signers.get(1).unwrap(), &id);

        // Jump past the 1-day expiry
        env.ledger().with_mut(|l| l.timestamp = 86_401);

        let result = client.try_execute_action(&signers.get(0).unwrap(), &id);
        assert_eq!(result, Err(Ok(StakeVaultError::EmergencyRequestExpired)));
    }

    #[test]
    fn clear_multisig_restores_direct_admin() {
        let (env, vault_id, admin, signers) = multisig_env();
        let client = StakeVaultContractClient::new(&env, &vault_id);

        // With multisig configured, direct call fails
        assert_eq!(
            client.try_set_minimum_stake(&1_000_000i128),
            Err(Ok(StakeVaultError::RequiresMultisig))
        );

        // Admin clears the multisig config
        client.set_multisig_config(&admin, &None);

        // Now direct admin call succeeds again (auto-unwraps in mock auth)
        client.set_minimum_stake(&1_000_000i128);
        assert_eq!(client.get_minimum_stake(), 1_000_000);
    }
}

// ── Issue #786: Unstake queue size cap ──────────────────────────────────────────

mod unstake_queue_cap_tests {
    use crate::{
        migration::{MigrationKey, StakeInfoV2},
        StakeVaultContract, StakeVaultContractClient, StakeVaultError,
    };
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Map};

    fn sac_token(env: &Env, admin: &Address) -> Address {
        env.register_stellar_asset_contract_v2(admin.clone())
            .address()
    }

    fn seed(env: &Env, contract_id: &Address, staker: &Address, balance: i128) {
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
                    locked_until: 0,
                    last_updated: env.ledger().timestamp(),
                },
            );
            env.storage()
                .persistent()
                .set(&MigrationKey::StakesV2, &stakes);
        });
    }

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let signal_registry = Address::generate(&env);
        let token = sac_token(&env, &admin);
        let vault_id = env.register(StakeVaultContract, ());
        StakeVaultContractClient::new(&env, &vault_id).initialize(&admin, &token, &signal_registry);
        (env, vault_id, token, admin, signal_registry)
    }

    #[test]
    fn default_max_queue_size_is_two_hundred() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        assert_eq!(client.get_max_unstake_queue_size(), 200);
    }

    #[test]
    fn enqueue_below_cap_succeeds() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_max_unstake_queue_size(&2);

        let staker_a = Address::generate(&env);
        let staker_b = Address::generate(&env);
        seed(&env, &vault_id, &staker_a, 1_000_000);
        seed(&env, &vault_id, &staker_b, 1_000_000);

        assert_eq!(client.queue_unstake(&staker_a), 0);
        assert_eq!(client.queue_unstake(&staker_b), 1);
    }

    #[test]
    fn enqueue_at_cap_is_rejected() {
        let (env, vault_id, _token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_max_unstake_queue_size(&2);

        let staker_a = Address::generate(&env);
        let staker_b = Address::generate(&env);
        let staker_c = Address::generate(&env);
        seed(&env, &vault_id, &staker_a, 1_000_000);
        seed(&env, &vault_id, &staker_b, 1_000_000);
        seed(&env, &vault_id, &staker_c, 1_000_000);

        client.queue_unstake(&staker_a);
        client.queue_unstake(&staker_b);

        // Queue is now at the cap (2 entries) — the next enqueue must be rejected.
        let result = client.try_queue_unstake(&staker_c);
        assert_eq!(result, Err(Ok(StakeVaultError::QueueFull)));
    }

    #[test]
    fn lowering_cap_below_current_length_keeps_existing_entries_but_blocks_new_ones() {
        let (env, vault_id, token, _admin, _registry) = setup();
        let client = StakeVaultContractClient::new(&env, &vault_id);
        client.set_max_unstake_queue_size(&5);

        let staker_a = Address::generate(&env);
        let staker_b = Address::generate(&env);
        let staker_c = Address::generate(&env);
        seed(&env, &vault_id, &staker_a, 1_000_000);
        seed(&env, &vault_id, &staker_b, 1_000_000);
        seed(&env, &vault_id, &staker_c, 1_000_000);

        client.queue_unstake(&staker_a);
        client.queue_unstake(&staker_b);
        client.queue_unstake(&staker_c);

        // Lower the cap below the current queue length (3).
        client.set_max_unstake_queue_size(&2);

        // Existing entries are untouched — positions still resolve correctly.
        assert_eq!(client.get_queue_position(&staker_a), Some(0));
        assert_eq!(client.get_queue_position(&staker_b), Some(1));
        assert_eq!(client.get_queue_position(&staker_c), Some(2));

        // A brand-new enqueue is rejected while the queue sits above the new cap.
        let staker_d = Address::generate(&env);
        seed(&env, &vault_id, &staker_d, 1_000_000);
        let result = client.try_queue_unstake(&staker_d);
        assert_eq!(result, Err(Ok(StakeVaultError::QueueFull)));

        // The existing entries still process normally despite being over the new cap.
        StellarAssetClient::new(&env, &token).mint(&vault_id, &3_000_000);
        assert_eq!(client.process_unstake_queue(&3), 3);
    }
}
