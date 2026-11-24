//! Integration tests for the token vault (issues #1524, #1525, #1526).

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env};
use token_vault::{DataKey, TokenVault, TokenVaultClient, VaultError};

const RESERVE: i128 = 1_000_000;
const DEPOSIT: i128 = 10_000;

struct Setup<'a> {
    env: Env,
    vault_id: Address,
    vault: TokenVaultClient<'a>,
    user: Address,
    token_a: Address,
    token_b: Address,
}

fn new_token(env: &Env, admin: &Address) -> Address {
    env.register_stellar_asset_contract_v2(admin.clone())
        .address()
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token_a = new_token(&env, &admin);
    let token_b = new_token(&env, &admin);

    let vault_id = env.register(TokenVault, ());
    let vault = TokenVaultClient::new(&env, &vault_id);
    vault.initialize(&admin);
    vault.add_allowed_token(&token_a);
    vault.add_allowed_token(&token_b);

    for token in [&token_a, &token_b] {
        let sac = StellarAssetClient::new(&env, token);
        sac.mint(&admin, &RESERVE);
        sac.mint(&user, &DEPOSIT);
        vault.fund_reserve(token, &RESERVE);
    }

    Setup {
        env,
        vault_id,
        vault,
        user,
        token_a,
        token_b,
    }
}

#[test]
fn deposit_swap_withdraw_flow_updates_token_balances() {
    let s = setup();
    let a = TokenClient::new(&s.env, &s.token_a);
    let b = TokenClient::new(&s.env, &s.token_b);

    s.vault.deposit(&s.user, &s.token_a, &DEPOSIT);
    assert_eq!(a.balance(&s.user), 0);
    assert_eq!(a.balance(&s.vault_id), RESERVE + DEPOSIT);
    assert_eq!(s.vault.balance(&s.user, &s.token_a), DEPOSIT);

    // Constant product: out = reserve_out * in / (reserve_in + in).
    let expected_out = RESERVE * DEPOSIT / (RESERVE + DEPOSIT);
    let out = s
        .vault
        .swap(&s.user, &s.token_a, &s.token_b, &DEPOSIT, &expected_out);
    assert_eq!(out, expected_out);
    assert_eq!(s.vault.balance(&s.user, &s.token_a), 0);
    assert_eq!(s.vault.balance(&s.user, &s.token_b), expected_out);
    assert_eq!(s.vault.reserve(&s.token_a), RESERVE + DEPOSIT);
    assert_eq!(s.vault.reserve(&s.token_b), RESERVE - expected_out);

    s.vault.withdraw(&s.user, &s.token_b, &expected_out);
    assert_eq!(s.vault.balance(&s.user, &s.token_b), 0);
    assert_eq!(a.balance(&s.user), 0);
    assert_eq!(b.balance(&s.user), DEPOSIT + expected_out);
    assert_eq!(a.balance(&s.vault_id), RESERVE + DEPOSIT);
    assert_eq!(b.balance(&s.vault_id), RESERVE - expected_out);
    assert!(!s.vault.is_locked());
}

#[test]
fn swap_rejects_slippage_and_overdraw() {
    let s = setup();
    s.vault.deposit(&s.user, &s.token_a, &DEPOSIT);
    assert_eq!(
        s.vault
            .try_swap(&s.user, &s.token_a, &s.token_b, &DEPOSIT, &DEPOSIT),
        Err(Ok(VaultError::SlippageExceeded))
    );
    assert_eq!(
        s.vault
            .try_swap(&s.user, &s.token_a, &s.token_b, &(DEPOSIT + 1), &0),
        Err(Ok(VaultError::InsufficientBalance))
    );
    assert_eq!(
        s.vault.try_withdraw(&s.user, &s.token_a, &(DEPOSIT + 1)),
        Err(Ok(VaultError::InsufficientBalance))
    );
}

#[test]
fn allowlist_is_admin_managed() {
    let s = setup();
    let other = Address::generate(&s.env);
    assert!(!s.vault.is_token_allowed(&other));
    s.vault.add_allowed_token(&other);
    assert!(s.vault.is_token_allowed(&other));
    s.vault.remove_allowed_token(&other);
    assert!(!s.vault.is_token_allowed(&other));
}

#[test]
fn deposit_and_swap_reject_tokens_not_on_allowlist() {
    let s = setup();
    let admin = Address::generate(&s.env);
    let rogue = new_token(&s.env, &admin);
    StellarAssetClient::new(&s.env, &rogue).mint(&s.user, &DEPOSIT);

    assert_eq!(
        s.vault.try_deposit(&s.user, &rogue, &DEPOSIT),
        Err(Ok(VaultError::TokenNotAllowed))
    );

    s.vault.deposit(&s.user, &s.token_a, &DEPOSIT);
    assert_eq!(
        s.vault.try_swap(&s.user, &s.token_a, &rogue, &DEPOSIT, &0),
        Err(Ok(VaultError::TokenNotAllowed))
    );

    // Delisting blocks new deposits/swaps but never traps funds.
    s.vault.remove_allowed_token(&s.token_a);
    assert_eq!(
        s.vault.try_deposit(&s.user, &s.token_a, &1),
        Err(Ok(VaultError::TokenNotAllowed))
    );
    assert_eq!(
        s.vault
            .try_swap(&s.user, &s.token_a, &s.token_b, &DEPOSIT, &0),
        Err(Ok(VaultError::TokenNotAllowed))
    );
    s.vault.withdraw(&s.user, &s.token_a, &DEPOSIT);
    assert_eq!(
        TokenClient::new(&s.env, &s.token_a).balance(&s.user),
        DEPOSIT
    );
}

#[test]
fn reentrant_calls_revert_with_reentrancy_guard() {
    let s = setup();
    s.vault.deposit(&s.user, &s.token_a, &DEPOSIT);

    // Simulate being mid-call: the lock is held as it would be while a
    // guarded function is running when a callback re-enters the vault.
    s.env.as_contract(&s.vault_id, || {
        s.env.storage().instance().set(&DataKey::Locked, &true);
    });
    assert!(s.vault.is_locked());

    assert_eq!(
        s.vault.try_deposit(&s.user, &s.token_a, &1),
        Err(Ok(VaultError::ReentrancyGuard))
    );
    assert_eq!(
        s.vault.try_swap(&s.user, &s.token_a, &s.token_b, &1, &0),
        Err(Ok(VaultError::ReentrancyGuard))
    );
    assert_eq!(
        s.vault.try_withdraw(&s.user, &s.token_a, &1),
        Err(Ok(VaultError::ReentrancyGuard))
    );

    // Once the outer call releases the lock, calls succeed again.
    s.env.as_contract(&s.vault_id, || {
        s.env.storage().instance().remove(&DataKey::Locked);
    });
    s.vault.withdraw(&s.user, &s.token_a, &DEPOSIT);
    assert!(!s.vault.is_locked());
}
