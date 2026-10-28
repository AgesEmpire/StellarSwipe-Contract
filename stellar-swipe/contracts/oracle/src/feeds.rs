//! Feed configuration, publisher authorization and health diagnostics.
//!
//! # Publisher authorization (Issue #1309)
//! A configured feed binds an `owner` and a bounded set of delegated
//! `publishers`. Only those addresses may submit observations for the feed;
//! the owner may add or revoke delegates. Feeds without a configuration keep
//! the legacy rule (any registered source may submit).
//!
//! # Atomic batch configuration (Issue #1312)
//! [`configure_batch`] validates every entry (size cap, duplicate pairs,
//! publisher bounds, `max_age > 0`) before the first persistent write, so a
//! failed entry leaves every feed configuration and version unchanged.
//!
//! # Health diagnostics (Issue #1311)
//! [`status`] reads a constant number of keys for one pair, so its cost is
//! independent of the total number of feeds. States:
//! - `Unknown`  — no configuration and no accepted round;
//! - `Disabled` — configured with `enabled == false`;
//! - `Stale`    — no accepted round, or `now - last_update >= max_age`;
//! - `Active`   — otherwise (`now - last_update < max_age`).
//!
//! Unconfigured feeds use the pair's staleness window as `max_age`.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};
use stellar_swipe_common::AssetPair;

use crate::{errors::OracleError, price_cb, staleness};

/// Maximum number of feeds in one configuration batch.
pub const MAX_FEED_BATCH: u32 = 10;
/// Maximum delegated publishers per feed.
pub const MAX_PUBLISHERS: u32 = 10;

#[contracttype]
#[derive(Clone)]
enum FeedKey {
    Config(AssetPair),
    Round(AssetPair),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedConfig {
    pub owner: Address,
    pub publishers: Vec<Address>,
    /// Seconds after the last accepted round before the feed is stale.
    pub max_age: u64,
    pub enabled: bool,
    /// Incremented on every configuration change.
    pub version: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedConfigInput {
    pub pair: AssetPair,
    pub owner: Address,
    pub publishers: Vec<Address>,
    pub max_age: u64,
    pub enabled: bool,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedState {
    Unknown,
    Active,
    Stale,
    Disabled,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedStatus {
    pub state: FeedState,
    /// Number of accepted rounds (0 = none).
    pub latest_round: u64,
    /// Ledger timestamp of the latest accepted round (0 = none).
    pub last_update: u64,
    pub max_age: u64,
    /// Configuration version (0 = unconfigured).
    pub config_version: u32,
    pub configured: bool,
    pub stale: bool,
    pub breaker_tripped: bool,
    /// Consecutive valid updates toward breaker recovery (Issue #1310).
    pub recovery_streak: u32,
}

#[contracttype]
#[derive(Clone)]
struct FeedRound {
    round: u64,
    updated_at: u64,
}

pub fn get_config(env: &Env, pair: &AssetPair) -> Option<FeedConfig> {
    env.storage()
        .persistent()
        .get(&FeedKey::Config(pair.clone()))
}

fn put_config(env: &Env, pair: &AssetPair, cfg: &FeedConfig) {
    env.storage()
        .persistent()
        .set(&FeedKey::Config(pair.clone()), cfg);
}

fn validate_publishers(publishers: &Vec<Address>) -> Result<(), OracleError> {
    if publishers.len() > MAX_PUBLISHERS {
        return Err(OracleError::InvalidFeedConfig);
    }
    for i in 0..publishers.len() {
        for j in (i + 1)..publishers.len() {
            if publishers.get_unchecked(i) == publishers.get_unchecked(j) {
                return Err(OracleError::InvalidFeedConfig);
            }
        }
    }
    Ok(())
}

/// Validate all entries, then write them. Nothing is written on error.
pub fn configure_batch(env: &Env, entries: &Vec<FeedConfigInput>) -> Result<(), OracleError> {
    if entries.is_empty() || entries.len() > MAX_FEED_BATCH {
        return Err(OracleError::InvalidFeedBatch);
    }
    for i in 0..entries.len() {
        let e = entries.get_unchecked(i);
        if e.max_age == 0 {
            return Err(OracleError::InvalidFeedConfig);
        }
        validate_publishers(&e.publishers)?;
        for j in (i + 1)..entries.len() {
            if entries.get_unchecked(j).pair == e.pair {
                return Err(OracleError::InvalidFeedConfig);
            }
        }
    }

    for e in entries.iter() {
        let version = get_config(env, &e.pair).map(|c| c.version).unwrap_or(0) + 1;
        put_config(
            env,
            &e.pair,
            &FeedConfig {
                owner: e.owner,
                publishers: e.publishers,
                max_age: e.max_age,
                enabled: e.enabled,
                version,
            },
        );
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("feed_cfg")),
            (e.pair, version),
        );
    }
    Ok(())
}

fn owner_config(env: &Env, caller: &Address, pair: &AssetPair) -> Result<FeedConfig, OracleError> {
    let cfg = get_config(env, pair).ok_or(OracleError::OracleNotFound)?;
    if cfg.owner != *caller {
        return Err(OracleError::Unauthorized);
    }
    Ok(cfg)
}

/// Owner: delegate submission rights for the feed to `publisher`.
pub fn add_publisher(
    env: &Env,
    caller: &Address,
    pair: &AssetPair,
    publisher: Address,
) -> Result<(), OracleError> {
    let mut cfg = owner_config(env, caller, pair)?;
    if cfg.publishers.contains(&publisher) {
        return Err(OracleError::OracleAlreadyExists);
    }
    cfg.publishers.push_back(publisher);
    validate_publishers(&cfg.publishers)?;
    cfg.version += 1;
    put_config(env, pair, &cfg);
    Ok(())
}

/// Owner: revoke a delegated publisher.
pub fn revoke_publisher(
    env: &Env,
    caller: &Address,
    pair: &AssetPair,
    publisher: &Address,
) -> Result<(), OracleError> {
    let mut cfg = owner_config(env, caller, pair)?;
    let idx = cfg
        .publishers
        .first_index_of(publisher)
        .ok_or(OracleError::OracleNotFound)?;
    cfg.publishers.remove(idx);
    cfg.version += 1;
    put_config(env, pair, &cfg);
    Ok(())
}

/// Reject `source` unless it is the owner or a delegated publisher of a
/// configured feed. Unconfigured feeds are not restricted here.
pub fn authorize_publisher(
    env: &Env,
    pair: &AssetPair,
    source: &Address,
) -> Result<(), OracleError> {
    let Some(cfg) = get_config(env, pair) else {
        return Ok(());
    };
    if cfg.owner != *source && !cfg.publishers.contains(source) {
        return Err(OracleError::Unauthorized);
    }
    if !cfg.enabled {
        return Err(OracleError::FeedDisabled);
    }
    Ok(())
}

/// Record an accepted round for the feed.
pub fn record_round(env: &Env, pair: &AssetPair) {
    let key = FeedKey::Round(pair.clone());
    let round = env
        .storage()
        .persistent()
        .get::<_, FeedRound>(&key)
        .map(|r| r.round)
        .unwrap_or(0)
        + 1;
    env.storage().persistent().set(
        &key,
        &FeedRound {
            round,
            updated_at: env.ledger().timestamp(),
        },
    );
}

pub fn status(env: &Env, pair: &AssetPair) -> FeedStatus {
    let cfg = get_config(env, pair);
    let round: Option<FeedRound> = env
        .storage()
        .persistent()
        .get(&FeedKey::Round(pair.clone()));
    let (latest_round, last_update) = round.map(|r| (r.round, r.updated_at)).unwrap_or((0, 0));
    let max_age = cfg
        .as_ref()
        .map(|c| c.max_age)
        .unwrap_or_else(|| staleness::get_staleness_window(env, pair));
    let stale =
        latest_round == 0 || env.ledger().timestamp().saturating_sub(last_update) >= max_age;

    let state = match &cfg {
        None if latest_round == 0 => FeedState::Unknown,
        Some(c) if !c.enabled => FeedState::Disabled,
        _ if stale => FeedState::Stale,
        _ => FeedState::Active,
    };

    FeedStatus {
        state,
        latest_round,
        last_update,
        max_age,
        config_version: cfg.as_ref().map(|c| c.version).unwrap_or(0),
        configured: cfg.is_some(),
        stale,
        breaker_tripped: price_cb::is_breaker_tripped(env, pair),
        recovery_streak: price_cb::recovery_streak(env, pair),
    }
}
