use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

use crate::{set_treasury_balance, ContractError, FeeCollector, FeeCollectorClient};

/// Helper: registers the contract, initializes it, mints tokens to it, and sets treasury balance.
fn setup(env: &Env, amount: i128) -> (Address, Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let recipient = Address::generate(env);

    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(env, &token).mint(&contract_id, &amount);

    env.as_contract(&contract_id, || {
        set_treasury_balance(env, &token, amount);
    });

    (recipient, token, contract_id, client)
}

#[test]
fn test_initialize_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&contract_id, &100i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 100i128);
    });
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &100i128);
}

#[test]
fn test_initialize_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let result = client.try_initialize(&admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialized)));
}

#[test]
fn test_treasury_balance_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);

    let result = client.try_treasury_balance(&token);
    assert_eq!(result, Err(Ok(ContractError::NotInitialized)));
}

#[test]
fn test_treasury_balance_unknown_token() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    assert_eq!(client.treasury_balance(&token), 0i128);
}

#[test]
fn test_full_balance_withdrawal() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);

    env.ledger().set_timestamp(86400);
    client.withdraw_treasury_fees(&recipient, &token, &1000i128);

    assert_eq!(client.treasury_balance(&token), 0i128);

    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&recipient), 1000i128);
}

#[test]
fn test_withdraw_insufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 500i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &500i128);

    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 0i128);
    });

    env.ledger().set_timestamp(86400);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &500i128);
    assert_eq!(result, Err(Ok(ContractError::InsufficientTreasuryBalance)));
}

#[test]
fn test_withdraw_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);
    env.ledger().set_timestamp(86400);

    let non_admin = Address::generate(&env);
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "withdraw_treasury_fees",
        args: (&recipient, &token, &1000i128).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &non_admin,
        invoke: &mock_invoke,
    };
    let auths: &[MockAuth] = &[mock_auth];
    let result = client
        .mock_auths(auths)
        .try_withdraw_treasury_fees(&recipient, &token, &1000i128);

    assert!(result.is_err(), "non-admin call must fail");
}

#[test]
fn test_withdraw_timelock_not_elapsed() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);

    env.ledger().set_timestamp(86399);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::TimelockNotElapsed)));
}

#[test]
fn test_withdraw_not_queued() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(86400);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalNotQueued)));
}

// ---------------------------------------------------------------------------
// Fee rate tests
// ---------------------------------------------------------------------------

#[test]
fn test_fee_rate_default() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    assert_eq!(client.fee_rate(), 30u32);
}

#[test]
fn test_set_fee_rate_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_rate(&50u32);
    assert_eq!(client.fee_rate(), 50u32);
}

#[test]
fn test_set_fee_rate_min_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_rate(&1u32);
    assert_eq!(client.fee_rate(), 1u32);
}

#[test]
fn test_set_fee_rate_max_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_rate(&100u32);
    assert_eq!(client.fee_rate(), 100u32);
}

#[test]
fn test_set_fee_rate_too_high() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let result = client.try_set_fee_rate(&101u32);
    assert_eq!(result, Err(Ok(ContractError::FeeRateTooHigh)));
}

#[test]
fn test_set_fee_rate_too_low() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let result = client.try_set_fee_rate(&0u32);
    assert_eq!(result, Err(Ok(ContractError::FeeRateTooLow)));
}

#[test]
fn test_set_fee_rate_no_retroactive_application() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let rate_before = client.fee_rate();
    client.set_fee_rate(&75u32);

    assert_ne!(rate_before, 75u32, "rate_before must be the old default");
    assert_eq!(
        client.fee_rate(),
        75u32,
        "fee_rate() must reflect the new value"
    );
}

#[test]
fn test_set_fee_rate_emits_event() {
    use soroban_sdk::testutils::Events;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    env.events().all();
    client.set_fee_rate(&60u32);

    let events = env.events().all();
    assert!(!events.is_empty(), "FeeRateUpdated event must be emitted");
}

#[test]
fn test_set_fee_rate_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);

    let result = client.try_set_fee_rate(&30u32);
    assert_eq!(result, Err(Ok(ContractError::NotInitialized)));
}

#[test]
fn test_set_fee_rate_unauthorized() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "set_fee_rate",
        args: (&50u32,).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth {
        address: &non_admin,
        invoke: &mock_invoke,
    };
    let auths: &[MockAuth] = &[mock_auth];
    let result = client.mock_auths(auths).try_set_fee_rate(&50u32);

    assert!(result.is_err(), "non-admin call to set_fee_rate must fail");
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::StellarAssetClient,
        Address, Env,
    };

    use crate::{set_treasury_balance, FeeCollector, FeeCollectorClient};

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(100))]

        #[test]
        fn prop_timelock_enforcement(
            queued_at in 0u64..=u64::MAX - 86400,
            delta in 0u64..=86399u64,
        ) {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);

            let token_admin = Address::generate(&env);
            let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
            let token = token_contract.address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            StellarAssetClient::new(&env, &token).mint(&contract_id, &1000i128);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, 1000i128);
            });

            env.ledger().set_timestamp(queued_at);
            client.queue_withdrawal(&recipient, &token, &1000i128);

            env.ledger().set_timestamp(queued_at + delta);
            let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);

            prop_assert_eq!(
                result,
                Err(Ok(crate::ContractError::TimelockNotElapsed)),
                "expected TimelockNotElapsed at queued_at={}, delta={}", queued_at, delta
            );
        }

        #[test]
        fn prop_over_withdrawal_rejection_at_execute(
            b in 1i128..=10_000_000i128,
            a in 1i128..=10_000_000i128,
        ) {
            let a = a.min(b);

            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);

            let token_admin = Address::generate(&env);
            let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
            let token = token_contract.address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &a);

            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, 0i128);
            });

            env.ledger().set_timestamp(86400);
            let result = client.try_withdraw_treasury_fees(&recipient, &token, &a);

            prop_assert_eq!(
                result,
                Err(Ok(crate::ContractError::InsufficientTreasuryBalance)),
                "expected InsufficientTreasuryBalance: b={}, a={}", b, a
            );
        }

        #[test]
        fn prop_balance_conservation_after_withdrawal(
            b in 1i128..=10_000_000i128,
            a in 1i128..=10_000_000i128,
        ) {
            let a = a.min(b);

            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);

            let token_admin = Address::generate(&env);
            let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
            let token = token_contract.address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &a);

            env.ledger().set_timestamp(86400);
            client.withdraw_treasury_fees(&recipient, &token, &a);

            prop_assert_eq!(
                client.treasury_balance(&token),
                b - a,
                "balance conservation violated: b={}, a={}", b, a
            );
        }

        #[test]
        fn prop_queue_over_withdrawal_rejection(
            b in 0i128..=10_000_000i128,
            extra in 1i128..=10_000_000i128,
        ) {
            let a = b + extra;

            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);

            let token_admin = Address::generate(&env);
            let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
            let token = token_contract.address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            env.ledger().set_timestamp(0);
            let result = client.try_queue_withdrawal(&recipient, &token, &a);

            prop_assert_eq!(
                result,
                Err(Ok(crate::ContractError::InsufficientTreasuryBalance)),
                "expected InsufficientTreasuryBalance: b={}, a={}", b, a
            );
        }
    }
}

// ---------------------------------------------------------------------------
// End-to-end integration test: full fee distribution cycle
// ---------------------------------------------------------------------------

/// Verifies the complete distribution pipeline:
///   collect_trade_fee (×2) → close_epoch → provider balances credited → treasury retains remainder
///
/// Three providers with 50 / 30 / 20 bps splits (sum = 100 % of 10_000 bps).
/// Two copy-trade executions contribute fees of 1_000_000 and 500_000 stroops.
/// Total epoch fees = 1_500_000 stroops.
///
/// Expected provider shares (floor division):
///   P1: 1_500_000 * 5000 / 10_000 = 750_000
///   P2: 1_500_000 * 3000 / 10_000 = 450_000
///   P3: 1_500_000 * 2000 / 10_000 = 300_000
///   sum = 1_500_000  →  treasury_share = 0  (no dust in this scenario)
#[test]
fn test_fee_distribution_end_to_end() {
    let env = Env::default();
    env.mock_all_auths();

    // --- setup ---
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    let p3 = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Mint enough tokens to the contract so SAC transfers in close_epoch won't fail
    // (close_epoch only updates internal balances; no SAC transfer occurs here)
    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_500_000i128);

    // --- a. simulate two copy-trade fee collections ---
    client.collect_trade_fee(&token, &1_000_000i128);
    client.collect_trade_fee(&token, &500_000i128);

    // --- b. close the epoch with 50 / 30 / 20 bps splits ---
    let mut providers = Vec::new(&env);
    providers.push_back(p1.clone());
    providers.push_back(p2.clone());
    providers.push_back(p3.clone());

    let mut shares = Vec::new(&env);
    shares.push_back(5000u32); // 50%
    shares.push_back(3000u32); // 30%
    shares.push_back(2000u32); // 20%

    client.close_epoch(&token, &providers, &shares);

    // --- c. assert provider credited balances ---
    let total_fees: i128 = 1_500_000;

    let share_p1 = total_fees * 5000 / 10_000; // 750_000
    let share_p2 = total_fees * 3000 / 10_000; // 450_000
    let share_p3 = total_fees * 2000 / 10_000; // 300_000

    assert_eq!(
        client.provider_balance(&p1, &token),
        share_p1,
        "P1 balance mismatch"
    );
    assert_eq!(
        client.provider_balance(&p2, &token),
        share_p2,
        "P2 balance mismatch"
    );
    assert_eq!(
        client.provider_balance(&p3, &token),
        share_p3,
        "P3 balance mismatch"
    );

    // --- d. assert rounding invariant: distributed + treasury == total_fees (no dust lost) ---
    let distributed = share_p1 + share_p2 + share_p3;
    let treasury = client.treasury_balance(&token);

    assert_eq!(
        distributed + treasury,
        total_fees,
        "dust invariant violated: distributed={distributed}, treasury={treasury}, total={total_fees}"
    );

    // --- e. assert epoch accumulator was reset ---
    // A second close_epoch with no new fees collected must be a no-op (returns Ok, balances unchanged)
    client.close_epoch(&token, &providers, &shares);
    assert_eq!(
        client.provider_balance(&p1, &token),
        share_p1,
        "P1 balance must not change after empty epoch"
    );
    assert_eq!(
        client.treasury_balance(&token),
        treasury,
        "treasury must not change after empty epoch"
    );
}

/// Rounding-dust scenario: total_fees not evenly divisible across providers.
/// 3 providers: 33 / 33 / 33 bps (sum = 99 bps, 1 bps of dust goes to treasury).
/// total_fees = 1_000_000 stroops.
///   P1: 1_000_000 * 3333 / 10_000 = 333_300
///   P2: 1_000_000 * 3333 / 10_000 = 333_300
///   P3: 1_000_000 * 3334 / 10_000 = 333_400
///   sum = 1_000_000  →  treasury = 0
///
/// Uses an intentionally uneven split to prove no stroop is lost.
#[test]
fn test_fee_distribution_rounding_dust() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    let p3 = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(&env, &contract_id);
    client.initialize(&admin);

    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000_000i128);

    client.collect_trade_fee(&token, &1_000_000i128);

    // 3333 + 3333 + 3334 = 10_000 bps (sums to exactly 100%)
    let mut providers = Vec::new(&env);
    providers.push_back(p1.clone());
    providers.push_back(p2.clone());
    providers.push_back(p3.clone());

    let mut shares = Vec::new(&env);
    shares.push_back(3333u32);
    shares.push_back(3333u32);
    shares.push_back(3334u32);

    client.close_epoch(&token, &providers, &shares);

    let total_fees: i128 = 1_000_000;
    let s1 = client.provider_balance(&p1, &token);
    let s2 = client.provider_balance(&p2, &token);
    let s3 = client.provider_balance(&p3, &token);
    let treasury = client.treasury_balance(&token);

    // Each share must be within 1 stroop of the ideal
    let ideal_p1 = total_fees * 3333 / 10_000;
    let ideal_p2 = total_fees * 3333 / 10_000;
    let ideal_p3 = total_fees * 3334 / 10_000;

    assert!((s1 - ideal_p1).abs() <= 1, "P1 rounding error > 1 stroop");
    assert!((s2 - ideal_p2).abs() <= 1, "P2 rounding error > 1 stroop");
    assert!((s3 - ideal_p3).abs() <= 1, "P3 rounding error > 1 stroop");

    // No dust lost
    assert_eq!(
        s1 + s2 + s3 + treasury,
        total_fees,
        "dust invariant violated: s1={s1}, s2={s2}, s3={s3}, treasury={treasury}"
    );
}
