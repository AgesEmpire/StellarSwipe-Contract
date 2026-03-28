#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

use crate::{set_treasury_balance, ContractError, FeeCollector, FeeCollectorClient};

/// Helper: registers the contract, initializes it, mints tokens to it, and sets treasury balance.
fn setup(env: &Env, amount: i128) -> (Address, Address, Address, FeeCollectorClient) {
    let admin = Address::generate(env);
    let recipient = Address::generate(env);

    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token_contract.address();

    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);

    // Mint tokens to the fee_collector so the SAC balance matches
    StellarAssetClient::new(env, &token).mint(&contract_id, &amount);

    // Set the internal treasury balance to match
    env.as_contract(&contract_id, || {
        set_treasury_balance(env, &token, amount);
    });

    (recipient, token, contract_id, client)
}

// 3.3 — initialize: happy path (admin stored, subsequent admin-only call succeeds)
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

    // Verify admin was stored: an admin-only call (queue_withdrawal) succeeds
    StellarAssetClient::new(&env, &token).mint(&contract_id, &100i128);
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 100i128);
    });
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &100i128); // would panic if admin not set
}

// 3.3 — initialize: AlreadyInitialized on double-init
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

// 3.3 — treasury_balance: NotInitialized before init
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

// 3.3 — treasury_balance: returns 0 for unknown token
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

// 8.1 — Happy path: queue at t=0, execute at t=86400, balance reaches 0
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

    // Verify the token actually moved: recipient's SAC balance should be 1000
    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&recipient), 1000i128);
}

// 8.2 — InsufficientTreasuryBalance when amount exceeds balance
#[test]
fn test_withdraw_insufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 500i128);

    // Queue a valid amount first, then drain the treasury balance so execute fails
    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &500i128);

    // Reduce treasury balance to 0 while keeping the queued record intact
    env.as_contract(&contract_id, || {
        set_treasury_balance(&env, &token, 0i128);
    });

    env.ledger().set_timestamp(86400);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &500i128);
    assert_eq!(result, Err(Ok(ContractError::InsufficientTreasuryBalance)));
}

// 8.3 — Unauthorized when non-admin calls withdraw_treasury_fees
//
// In Soroban, admin.require_auth() panics (Abort) when the admin's auth is not
// present in the invocation context. This is the correct enforcement mechanism —
// a non-admin caller cannot satisfy the admin's require_auth() check.
// The test verifies the call fails (is_err) when called without admin auth.
#[test]
fn test_withdraw_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, contract_id, client) = setup(&env, 1000i128);

    // Queue legitimately as admin
    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);
    env.ledger().set_timestamp(86400);

    // Now call withdraw_treasury_fees providing only a non-admin's auth.
    // The contract calls admin.require_auth() — since the admin's auth is absent,
    // Soroban panics, which surfaces as Err(Err(Abort)) from try_ methods.
    let non_admin = Address::generate(&env);
    use soroban_sdk::IntoVal;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    let sub_invokes: &[MockAuthInvoke] = &[];
    let mock_invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "withdraw_treasury_fees",
        args: (&recipient, &token, &1000i128).into_val(&env),
        sub_invokes,
    };
    let mock_auth = MockAuth { address: &non_admin, invoke: &mock_invoke };
    let auths: &[MockAuth] = &[mock_auth];
    let result = client
        .mock_auths(auths)
        .try_withdraw_treasury_fees(&recipient, &token, &1000i128);

    // Non-admin auth causes require_auth() to panic → Abort error
    assert!(result.is_err(), "non-admin call must fail");
}

// 8.4 — TimelockNotElapsed when called before 86400s
#[test]
fn test_withdraw_timelock_not_elapsed() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    env.ledger().set_timestamp(0);
    client.queue_withdrawal(&recipient, &token, &1000i128);

    // Try at t=86399 — one second short
    env.ledger().set_timestamp(86399);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::TimelockNotElapsed)));
}

// 8.5 — WithdrawalNotQueued when no queued record exists
#[test]
fn test_withdraw_not_queued() {
    let env = Env::default();
    env.mock_all_auths();

    let (recipient, token, _contract_id, client) = setup(&env, 1000i128);

    // Skip queue_withdrawal entirely
    env.ledger().set_timestamp(86400);
    let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalNotQueued)));
}

// ---------------------------------------------------------------------------
// Property 1: Balance conservation after withdrawal
// Validates: Requirements 4.6
// ---------------------------------------------------------------------------
//
// For any valid treasury balance `b` and withdrawal amount `a` in [1, b],
// after a successful withdraw_treasury_fees the stored balance must equal b - a.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;
    use soroban_sdk::{
        testutils::{Address as _, Events, Ledger},
        token::StellarAssetClient,
        Address, Env,
    };

    use crate::{set_treasury_balance, FeeCollector, FeeCollectorClient};

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(100))]

        /// Property 2: Timelock enforcement
        /// Validates: Requirements 4.4
        ///
        /// For any queued_at timestamp and delta in [0, 86399], executing
        /// withdraw_treasury_fees at queued_at + delta must return TimelockNotElapsed.
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

            // Mint and set treasury balance to 1000
            StellarAssetClient::new(&env, &token).mint(&contract_id, &1000i128);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, 1000i128);
            });

            // Queue at queued_at
            env.ledger().set_timestamp(queued_at);
            client.queue_withdrawal(&recipient, &token, &1000i128);

            // Attempt to execute before timelock elapses (queued_at + delta, delta < 86400)
            env.ledger().set_timestamp(queued_at + delta);
            let result = client.try_withdraw_treasury_fees(&recipient, &token, &1000i128);

            prop_assert_eq!(
                result,
                Err(Ok(crate::ContractError::TimelockNotElapsed)),
                "expected TimelockNotElapsed at queued_at={}, delta={}, execute_at={}",
                queued_at, delta, queued_at + delta
            );
        }

        /// Property 3 (execute phase): Over-withdrawal rejection
        /// Validates: Requirements 4.5
        ///
        /// Queue a withdrawal for amount `a` from balance `b` (a <= b), then drain
        /// the treasury balance to 0 externally before executing. The execute call
        /// must return InsufficientTreasuryBalance.
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

            // Mint `b` tokens and set treasury balance to `b`
            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            // Queue a valid withdrawal at t=0
            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &a);

            // Drain the treasury balance to 0 externally (simulates race / external drain)
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, 0i128);
            });

            // Attempt to execute after timelock — must fail with InsufficientTreasuryBalance
            env.ledger().set_timestamp(86400);
            let result = client.try_withdraw_treasury_fees(&recipient, &token, &a);

            prop_assert_eq!(
                result,
                Err(Ok(crate::ContractError::InsufficientTreasuryBalance)),
                "expected InsufficientTreasuryBalance after draining balance: b={}, a={}",
                b, a
            );
        }

        /// Property 4: Unauthorized rejection on withdraw_treasury_fees
        /// Validates: Requirements 4.2
        ///
        /// For any randomly generated non-admin address, calling withdraw_treasury_fees
        /// with only that address's auth (not the admin's) must fail.
        #[test]
        fn prop_withdraw_unauthorized(
            // seed drives Address::generate determinism via proptest
            _seed in 0u32..1000u32,
        ) {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let recipient = Address::generate(&env);
            let non_admin = Address::generate(&env);

            let token_admin = Address::generate(&env);
            let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
            let token = token_contract.address();

            let contract_id = env.register(FeeCollector, ());
            let client = FeeCollectorClient::new(&env, &contract_id);
            client.initialize(&admin);

            soroban_sdk::token::StellarAssetClient::new(&env, &token)
                .mint(&contract_id, &1000i128);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, 1000i128);
            });

            // Queue legitimately (admin auth mocked)
            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &1000i128);
            env.ledger().set_timestamp(86400);

            // Now attempt withdraw with only non_admin's auth
            use soroban_sdk::IntoVal;
            use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
            let sub_invokes: &[MockAuthInvoke] = &[];
            let mock_invoke = MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_treasury_fees",
                args: (&recipient, &token, &1000i128).into_val(&env),
                sub_invokes,
            };
            let mock_auth = MockAuth { address: &non_admin, invoke: &mock_invoke };
            let auths: &[MockAuth] = &[mock_auth];
            let result = client
                .mock_auths(auths)
                .try_withdraw_treasury_fees(&recipient, &token, &1000i128);

            prop_assert!(
                result.is_err(),
                "non-admin call to withdraw_treasury_fees must fail"
            );
        }

        /// Property 5: Event emission on successful withdrawal
        /// Validates: Requirements 4.9
        ///
        /// For any valid withdrawal scenario, after a successful withdraw_treasury_fees:
        /// 1. At least one event must be emitted (the TreasuryWithdrawal event).
        /// 2. The remaining_balance in the emitted event must equal original_balance - amount,
        ///    verified by asserting the stored treasury balance equals b - a (since the contract
        ///    sets remaining_balance = new_balance = b - a before emitting the event).
        #[test]
        fn prop_event_emission_remaining_balance(
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

            // Queue at t=0, execute at t=86400
            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &a);

            // Clear events accumulated during setup so we only see the withdrawal event
            env.events().all();

            env.ledger().set_timestamp(86400);
            client.withdraw_treasury_fees(&recipient, &token, &a);

            // Property 5a: at least one event was emitted by withdraw_treasury_fees
            let events = env.events().all();
            prop_assert!(
                !events.is_empty(),
                "expected TreasuryWithdrawal event to be emitted: b={}, a={}",
                b, a
            );

            // Property 5b: remaining_balance in the event equals b - a.
            // The contract sets remaining_balance = new_balance and then stores new_balance
            // as TreasuryBalance. So treasury_balance == remaining_balance in the event.
            let expected_remaining = b - a;
            prop_assert_eq!(
                client.treasury_balance(&token),
                expected_remaining,
                "remaining_balance in event must equal b - a: b={}, a={}, expected={}",
                b, a, expected_remaining
            );
        }

        /// Property 1: Balance conservation after withdrawal
        /// Validates: Requirements 4.6
        #[test]
        fn prop_balance_conservation_after_withdrawal(
            // b in [1, 10_000_000] to keep SAC minting sane
            b in 1i128..=10_000_000i128,
            // a in [1, b]
            a in 1i128..=10_000_000i128,
        ) {
            // Clamp `a` so it never exceeds `b`
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

            // Mint `b` tokens to the fee_collector contract
            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);

            // Set internal treasury balance to `b`
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            // Queue at t=0, execute at t=86400
            env.ledger().set_timestamp(0);
            client.queue_withdrawal(&recipient, &token, &a);

            env.ledger().set_timestamp(86400);
            client.withdraw_treasury_fees(&recipient, &token, &a);

            // Property: remaining balance must equal b - a
            prop_assert_eq!(
                client.treasury_balance(&token),
                b - a,
                "balance conservation violated: b={}, a={}, expected={}, got={}",
                b, a, b - a, client.treasury_balance(&token)
            );
        }
        /// Property 3 (queue phase): Over-withdrawal rejection
        /// Validates: Requirements 3.3
        ///
        /// For any balance `b` and amount `a > b`, queue_withdrawal must return
        /// InsufficientTreasuryBalance.
        #[test]
        fn prop_queue_over_withdrawal_rejection(
            b in 0i128..=10_000_000i128,
            extra in 1i128..=10_000_000i128,
        ) {
            let a = b + extra; // always > b

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
                "expected InsufficientTreasuryBalance: b={}, a={}",
                b, a
            );
        }

        /// Property 4: Unauthorized rejection on queue_withdrawal
        /// Validates: Requirements 3.2
        ///
        /// Calling queue_withdrawal with only a non-admin address's auth must fail.
        #[test]
        fn prop_queue_unauthorized(
            _seed in 0u32..1000u32,
        ) {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let non_admin = Address::generate(&env);
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

            env.ledger().set_timestamp(0);

            use soroban_sdk::IntoVal;
            use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
            let sub_invokes: &[MockAuthInvoke] = &[];
            let mock_invoke = MockAuthInvoke {
                contract: &contract_id,
                fn_name: "queue_withdrawal",
                args: (&recipient, &token, &1000i128).into_val(&env),
                sub_invokes,
            };
            let mock_auth = MockAuth { address: &non_admin, invoke: &mock_invoke };
            let auths: &[MockAuth] = &[mock_auth];
            let result = client
                .mock_auths(auths)
                .try_queue_withdrawal(&recipient, &token, &1000i128);

            prop_assert!(
                result.is_err(),
                "non-admin call to queue_withdrawal must fail"
            );
        }

        /// Property 6: Zero and negative amount rejection
        /// Validates: Requirements 3.4
        ///
        /// For any amount <= 0, queue_withdrawal must return InvalidAmount and
        /// the treasury balance must remain unchanged.
        #[test]
        fn prop_queue_zero_negative_amount(
            b in 1i128..=10_000_000i128,
            a in i128::MIN..=0i128,
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

            StellarAssetClient::new(&env, &token).mint(&contract_id, &b);
            env.as_contract(&contract_id, || {
                set_treasury_balance(&env, &token, b);
            });

            env.ledger().set_timestamp(0);
            let result = client.try_queue_withdrawal(&recipient, &token, &a);

            prop_assert_eq!(
                result,
                Err(Ok(crate::ContractError::InvalidAmount)),
                "expected InvalidAmount for amount={}", a
            );

            // Balance must be unchanged
            prop_assert_eq!(
                client.treasury_balance(&token),
                b,
                "balance must be unchanged after rejected queue: b={}, a={}", b, a
            );
        }
    }
}
