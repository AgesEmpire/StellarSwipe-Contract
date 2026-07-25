#![cfg(test)]

//! Deterministic accounting for admin-configured volume discount tiers
//! (#664, hardened for #819).
//!
//! `FeeCollector::set_volume_discount_config` is the only way an admin can
//! change which traders qualify for a fee rebate and how large that rebate
//! is. These tests lock down that:
//!   1. a valid, explicit tier configuration is applied deterministically at
//!      the exact volume threshold boundary (standard case), and
//!   2. tiers with an implicit/ambiguous eligibility condition (a negative
//!      USD threshold that would match every trader regardless of volume)
//!      or a non-auditable payout (a zero or out-of-range discount) are
//!      rejected outright instead of being silently accepted (edge case).

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

use crate::storage::{self, VolumeDiscountConfig, VolumeTier};
use crate::{ContractError, FeeCollector, FeeCollectorClient};

fn setup(env: &Env) -> (Address, Address, FeeCollectorClient<'_>) {
    let admin = Address::generate(env);
    let contract_id = env.register(FeeCollector, ());
    let client = FeeCollectorClient::new(env, &contract_id);
    client.initialize(&admin);
    (admin, contract_id, client)
}

fn tiers(env: &Env, entries: &[(i128, u32)]) -> VolumeDiscountConfig {
    let mut tiers = Vec::new(env);
    for (threshold, discount) in entries {
        tiers.push_back(VolumeTier {
            volume_threshold_usd: *threshold,
            discount_bps: *discount,
        });
    }
    VolumeDiscountConfig { tiers }
}

fn set_volume(env: &Env, contract_id: &Address, user: &Address, volume_usd: i128) {
    env.as_contract(contract_id, || {
        let month_bucket = env.ledger().sequence() / storage::LEDGERS_PER_MONTH_APPROX;
        storage::set_monthly_trade_volume(
            env,
            user,
            &storage::MonthlyTradeVolume {
                month_bucket,
                volume_usd,
            },
        );
    });
}

/// Standard case: a valid three-tier config applies the highest discount for
/// which the trader's recorded volume qualifies, and does so exactly at each
/// tier's threshold boundary (`volume_usd == threshold` counts as eligible).
#[test]
fn test_volume_discount_applies_at_exact_threshold_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, contract_id, client) = setup(&env);

    client.set_fee_rate(&30u32); // 0.30% base rate, no congestion adjustment.

    let config = tiers(
        &env,
        &[
            (1_000 * 10_000_000, 2),  // Bronze: $1k -> 0.02% off
            (10_000 * 10_000_000, 5), // Silver: $10k -> 0.05% off
            (50_000 * 10_000_000, 10), // Gold:  $50k -> 0.10% off
        ],
    );
    client.set_volume_discount_config(&admin, &config);

    let user = Address::generate(&env);

    // Below every threshold: no discount.
    set_volume(&env, &contract_id, &user, 999 * 10_000_000);
    assert_eq!(client.fee_rate_for_user(&user), 30);

    // Exactly at the Silver threshold: Silver discount applies (boundary is
    // inclusive, not "strictly greater than").
    set_volume(&env, &contract_id, &user, 10_000 * 10_000_000);
    assert_eq!(client.fee_rate_for_user(&user), 30 - 5);

    // Exactly at the Gold threshold: Gold discount applies.
    set_volume(&env, &contract_id, &user, 50_000 * 10_000_000);
    assert_eq!(client.fee_rate_for_user(&user), 30 - 10);
}

/// Edge case (would have failed before #819): a tier with a negative
/// `volume_threshold_usd` is an implicit "always eligible" condition, since
/// `volume_usd >= negative_threshold` holds for every non-negative recorded
/// volume, including a trader who has never traded (volume_usd == 0). Prior
/// to this fix `set_volume_discount_config` accepted such a config, letting
/// an admin (accidentally or otherwise) grant every trader a discount
/// regardless of actual volume. It must now be rejected explicitly.
#[test]
fn test_negative_threshold_tier_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, client) = setup(&env);

    let config = tiers(
        &env,
        &[
            (-1, 10),                 // implicit "always eligible" tier
            (10_000 * 10_000_000, 5),
            (50_000 * 10_000_000, 10),
        ],
    );

    let result = client.try_set_volume_discount_config(&admin, &config);
    assert_eq!(result, Err(Ok(ContractError::InvalidFeeConfiguration)));

    // Confirm the rejected config was never persisted: a zero-volume trader
    // gets no rebate.
    assert!(client.get_volume_discount_config_fn().is_none());
}

/// Edge case: a zero-bps "discount" is not an auditable rebate (it changes
/// nothing) and a discount above the maximum possible fee rate can never be
/// honestly reflected in the fee actually charged. Both must be rejected so
/// every stored tier corresponds to a real, explicit rebate.
#[test]
fn test_out_of_range_discount_tier_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, client) = setup(&env);

    let zero_discount = tiers(
        &env,
        &[
            (1_000 * 10_000_000, 0),
            (10_000 * 10_000_000, 5),
            (50_000 * 10_000_000, 10),
        ],
    );
    let result = client.try_set_volume_discount_config(&admin, &zero_discount);
    assert_eq!(result, Err(Ok(ContractError::InvalidFeeConfiguration)));

    let excessive_discount = tiers(
        &env,
        &[
            (1_000 * 10_000_000, 2),
            (10_000 * 10_000_000, 5),
            (50_000 * 10_000_000, crate::storage::MAX_FEE_RATE_BPS + 1),
        ],
    );
    let result = client.try_set_volume_discount_config(&admin, &excessive_discount);
    assert_eq!(result, Err(Ok(ContractError::InvalidFeeConfiguration)));
}
