//! Contract-state fixture builders (Issue #1404).
//!
//! Proves the shared builders in `fixtures/mod.rs` produce identical state on
//! repeated construction and report initialization failures with the failing
//! contract and step instead of swallowing them.

extern crate std;

#[path = "fixtures/mod.rs"]
mod fixtures;

use fixtures::{FeeCollectorBuilder, SignalRegistryBuilder, FIXTURE_SEQUENCE, FIXTURE_TIMESTAMP};
use soroban_sdk::{testutils::Ledger, token::Client as TokenClient};

fn fee_builder() -> FeeCollectorBuilder {
    FeeCollectorBuilder::new()
        .burn_rate_bps(0)
        .revenue_share_rate_bps(0)
        .fee_rate_bps(100)
        .fund_trader(10_000_000)
        .fund_trader(5_000_000)
}

#[test]
fn fee_collector_fixture_is_deterministic() {
    let a = fee_builder().build().expect("first build");
    let b = fee_builder().build().expect("second build");

    for f in [&a, &b] {
        assert_eq!(f.env.ledger().timestamp(), FIXTURE_TIMESTAMP);
        assert_eq!(f.env.ledger().sequence(), FIXTURE_SEQUENCE);
        assert_eq!(f.client.burn_rate(), 0);
        assert_eq!(f.client.treasury_balance(&f.token), 0);
        assert_eq!(f.traders.len(), 2);
    }

    // Same generation order → same addresses (compared by strkey so the two
    // independent environments are never mixed in one host call).
    assert_eq!(
        a.admin.to_string().to_string(),
        b.admin.to_string().to_string()
    );
    assert_eq!(
        a.contract_id.to_string().to_string(),
        b.contract_id.to_string().to_string()
    );
    for i in 0..2 {
        let ta = TokenClient::new(&a.env, &a.token);
        let tb = TokenClient::new(&b.env, &b.token);
        assert_eq!(ta.balance(&a.traders[i]), tb.balance(&b.traders[i]));
        assert_eq!(
            a.traders[i].to_string().to_string(),
            b.traders[i].to_string().to_string()
        );
    }
}

#[test]
fn fee_collector_fixture_surfaces_configuration_failure() {
    // Far above the contract's maximum fee rate.
    let err = FeeCollectorBuilder::new()
        .fee_rate_bps(10_000)
        .build()
        .err()
        .expect("invalid fee rate must fail the fixture build");
    assert_eq!(err.contract, "fee_collector");
    assert_eq!(err.step, "set_fee_rate");
    assert!(err.detail.contains("contract error"), "{:?}", err);
}

#[test]
fn signal_registry_fixture_is_deterministic() {
    let build = || {
        SignalRegistryBuilder::new()
            .with_trade_executor()
            .staked_provider(200_000_000)
            .account()
            .build()
            .expect("registry fixture")
    };
    let a = build();
    let b = build();

    for f in [&a, &b] {
        assert!(f.executor_id.is_some());
        assert_eq!(f.registry.get_stake(&f.providers[0]), 200_000_000);
        assert_eq!(f.registry.get_stake(&f.accounts[0]), 0);
        assert_eq!(f.registry.get_admin(), f.admin);
    }
    assert_eq!(
        a.registry_id.to_string().to_string(),
        b.registry_id.to_string().to_string()
    );
    assert_eq!(
        a.providers[0].to_string().to_string(),
        b.providers[0].to_string().to_string()
    );
}

#[test]
fn signal_registry_fixture_surfaces_stake_failure() {
    // A non-positive stake is rejected by the registry; the fixture must
    // report which step failed rather than building partial state.
    let err = SignalRegistryBuilder::new()
        .staked_provider(0)
        .build()
        .err()
        .expect("zero stake must fail the fixture build");
    assert_eq!(err.contract, "signal_registry");
    assert_eq!(err.step, "stake_tokens");
}
