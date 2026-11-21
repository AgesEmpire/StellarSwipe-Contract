//! Deterministic contract-state fixture builders for integration tests
//! (Issue #1404).
//!
//! Integration tests used to hand-roll the same initialization sequences
//! (register → initialize → configure → fund). The builders here own those
//! sequences so every test that needs a given contract combination starts
//! from an identical, documented state.
//!
//! # Determinism
//! Every builder starts from [`deterministic_env`]: a fresh `Env` with
//! `mock_all_auths`, a fixed ledger timestamp and a fixed ledger sequence.
//! Addresses are generated and contracts registered in a fixed order, so two
//! fixtures built with the same builder configuration hold identical state.
//!
//! # Failure surfacing
//! Every initialization/configuration step goes through the generated
//! `try_*` client method. A failing step is never swallowed or turned into a
//! generic panic: `build()` returns a [`FixtureError`] naming the contract,
//! the step and the contract error, so a broken setup is reported as a setup
//! failure rather than as a confusing assertion failure later in the test.
//!
//! Include from a test crate with:
//! ```ignore
//! #[path = "fixtures/mod.rs"]
//! mod fixtures;
//! ```

#![allow(dead_code)]

extern crate std;

use core::fmt::{self, Debug};
use std::{format, string::String as StdString, vec::Vec as StdVec};

use fee_collector::{FeeCollector, FeeCollectorClient};
use signal_registry::{SignalRegistry, SignalRegistryClient};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};
use stellar_swipe_common::Asset;

/// Ledger timestamp every fixture starts at unless overridden.
pub const FIXTURE_TIMESTAMP: u64 = 1_000_000;
/// Ledger sequence every fixture starts at.
pub const FIXTURE_SEQUENCE: u32 = 1_000;

/// A fixture initialization step failed.
#[derive(Clone, PartialEq, Eq)]
pub struct FixtureError {
    /// Contract the failing step targeted (e.g. `"fee_collector"`).
    pub contract: &'static str,
    /// Entry point that failed (e.g. `"initialize"`).
    pub step: &'static str,
    /// Debug rendering of the contract / host error.
    pub detail: StdString,
}

impl Debug for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fixture setup failed: {}::{} -> {}",
            self.contract, self.step, self.detail
        )
    }
}

/// Convert a generated `try_*` client result into a fixture result, keeping
/// the failing contract and step in the error.
pub fn surface<T, CE: Debug, E: Debug, IE: Debug>(
    contract: &'static str,
    step: &'static str,
    result: Result<Result<T, CE>, Result<E, IE>>,
) -> Result<T, FixtureError> {
    let detail = match result {
        Ok(Ok(value)) => return Ok(value),
        Ok(Err(conversion)) => format!("return value conversion failed: {:?}", conversion),
        Err(Ok(contract_error)) => format!("contract error: {:?}", contract_error),
        Err(Err(invoke_error)) => format!("invoke error: {:?}", invoke_error),
    };
    Err(FixtureError {
        contract,
        step,
        detail,
    })
}

/// Fresh environment with mocked auths and a pinned ledger clock.
pub fn deterministic_env(timestamp: u64) -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| {
        l.timestamp = timestamp;
        l.sequence_number = FIXTURE_SEQUENCE;
    });
    env
}

// ── Shared stubs ──────────────────────────────────────────────────────────────

/// Oracle stub that converts every amount 1:1 into the base currency, so fee
/// math in end-to-end flows stays legible.
#[contract]
pub struct UnitOracle;

#[contractimpl]
impl UnitOracle {
    pub fn convert_to_base(_env: Env, amount: i128, _asset: Asset) -> i128 {
        amount
    }
}

/// Empty contract registered as the trade executor; the registry only checks
/// the caller address.
#[contract]
pub struct TradeExecutorStub;

#[contractimpl]
impl TradeExecutorStub {}

// ── Fee collector ─────────────────────────────────────────────────────────────

/// Fee collector + Stellar asset token + 1:1 oracle, with funded traders.
pub struct FeeCollectorFixture {
    pub env: Env,
    pub admin: Address,
    pub token: Address,
    pub contract_id: Address,
    pub oracle_id: Address,
    pub client: FeeCollectorClient<'static>,
    /// Traders in the order they were requested via `fund_trader`.
    pub traders: StdVec<Address>,
}

/// Builder for [`FeeCollectorFixture`]. Rates left unset keep the contract's
/// defaults.
#[derive(Clone, Debug, Default)]
pub struct FeeCollectorBuilder {
    timestamp: Option<u64>,
    fee_rate_bps: Option<u32>,
    burn_rate_bps: Option<u32>,
    revenue_share_rate_bps: Option<u32>,
    trader_balances: StdVec<i128>,
}

impl FeeCollectorBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn at_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn fee_rate_bps(mut self, bps: u32) -> Self {
        self.fee_rate_bps = Some(bps);
        self
    }

    pub fn burn_rate_bps(mut self, bps: u32) -> Self {
        self.burn_rate_bps = Some(bps);
        self
    }

    pub fn revenue_share_rate_bps(mut self, bps: u32) -> Self {
        self.revenue_share_rate_bps = Some(bps);
        self
    }

    /// Add a trader minted `balance` of the fixture token.
    pub fn fund_trader(mut self, balance: i128) -> Self {
        self.trader_balances.push(balance);
        self
    }

    pub fn build(self) -> Result<FeeCollectorFixture, FixtureError> {
        const C: &str = "fee_collector";
        let env = deterministic_env(self.timestamp.unwrap_or(FIXTURE_TIMESTAMP));

        let admin = Address::generate(&env);
        let token_admin = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let contract_id = env.register(FeeCollector, ());
        let client = FeeCollectorClient::new(&env, &contract_id);
        surface(C, "initialize", client.try_initialize(&admin))?;

        let oracle_id = env.register(UnitOracle, ());
        surface(C, "set_oracle_contract", client.try_set_oracle_contract(&oracle_id))?;

        if let Some(bps) = self.burn_rate_bps {
            surface(C, "set_burn_rate", client.try_set_burn_rate(&bps))?;
        }
        if let Some(bps) = self.revenue_share_rate_bps {
            surface(
                C,
                "set_revenue_share_rate_bps",
                client.try_set_revenue_share_rate_bps(&bps),
            )?;
        }
        if let Some(bps) = self.fee_rate_bps {
            surface(C, "set_fee_rate", client.try_set_fee_rate(&bps))?;
        }

        let mut traders = StdVec::new();
        for _ in 0..self.trader_balances.len() {
            traders.push(Address::generate(&env));
        }
        let token_client = StellarAssetClient::new(&env, &token);
        for (trader, balance) in traders.iter().zip(self.trader_balances.iter()) {
            surface("token", "mint", token_client.try_mint(trader, balance))?;
        }

        Ok(FeeCollectorFixture {
            env,
            admin,
            token,
            contract_id,
            oracle_id,
            client,
            traders,
        })
    }
}

// ── Signal registry ───────────────────────────────────────────────────────────

/// Signal registry with an optional trade-executor stub, staked providers and
/// unstaked accounts.
pub struct SignalRegistryFixture {
    pub env: Env,
    pub admin: Address,
    pub registry_id: Address,
    pub registry: SignalRegistryClient<'static>,
    /// Registered trade executor stub, when requested.
    pub executor_id: Option<Address>,
    /// Staked providers in the order requested via `staked_provider`.
    pub providers: StdVec<Address>,
    /// Unstaked accounts in the order requested via `account`.
    pub accounts: StdVec<Address>,
}

/// Builder for [`SignalRegistryFixture`].
#[derive(Clone, Debug, Default)]
pub struct SignalRegistryBuilder {
    timestamp: Option<u64>,
    with_trade_executor: bool,
    provider_stakes: StdVec<i128>,
    accounts: u32,
}

impl SignalRegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn at_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Register a [`TradeExecutorStub`] and wire it via `set_trade_executor`.
    pub fn with_trade_executor(mut self) -> Self {
        self.with_trade_executor = true;
        self
    }

    /// Add a provider that stakes `amount` during setup.
    pub fn staked_provider(mut self, amount: i128) -> Self {
        self.provider_stakes.push(amount);
        self
    }

    /// Add an unstaked account (e.g. a follower / copier).
    pub fn account(mut self) -> Self {
        self.accounts += 1;
        self
    }

    pub fn build(self) -> Result<SignalRegistryFixture, FixtureError> {
        const C: &str = "signal_registry";
        let env = deterministic_env(self.timestamp.unwrap_or(FIXTURE_TIMESTAMP));

        // Fixed generation order: admin, providers, accounts, then contracts.
        let admin = Address::generate(&env);
        let mut providers = StdVec::new();
        for _ in 0..self.provider_stakes.len() {
            providers.push(Address::generate(&env));
        }
        let mut accounts = StdVec::new();
        for _ in 0..self.accounts {
            accounts.push(Address::generate(&env));
        }

        let registry_id = env.register(SignalRegistry, ());
        let executor_id = if self.with_trade_executor {
            Some(env.register(TradeExecutorStub, ()))
        } else {
            None
        };

        let registry = SignalRegistryClient::new(&env, &registry_id);
        surface(C, "initialize", registry.try_initialize(&admin))?;
        if let Some(executor) = &executor_id {
            surface(
                C,
                "set_trade_executor",
                registry.try_set_trade_executor(&admin, executor),
            )?;
        }
        for (provider, amount) in providers.iter().zip(self.provider_stakes.iter()) {
            surface(C, "stake_tokens", registry.try_stake_tokens(provider, amount))?;
        }

        Ok(SignalRegistryFixture {
            env,
            admin,
            registry_id,
            registry,
            executor_id,
            providers,
            accounts,
        })
    }
}
