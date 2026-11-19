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
//!
//! # Canonical feed identifiers (Issue #1303)
//! A feed is identified by an unordered asset pair, not by the order in which
//! the caller supplied its assets. Asset codes are upper-cased (so `xlm` and
//! `XLM` are the same alias), and `XLM` with an issuer is normalized to native
//! XLM. The two assets are then sorted by their deterministic identifier
//! (code bytes, then issuer bytes). Identical assets are rejected. The
//! canonical pair is the single storage key used for configuration and rounds,
//! so reversed/aliased spellings can never register a second feed for the same
//! market. Distinct quote assets remain independent feeds.

use core::cmp::Ordering;

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, String, Vec};
use stellar_swipe_common::{Asset, AssetPair};

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
    /// Maximum single-update movement in basis points; 0 disables (Issue #1305).
    pub max_movement_bps: u32,
    /// Inclusive lower bound for accepted observations (Issue #1304).
    pub min_price: i128,
    /// Inclusive upper bound for accepted observations (Issue #1304).
    pub max_price: i128,
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
    /// Maximum single-update movement in basis points; 0 disables (Issue #1305).
    pub max_movement_bps: u32,
    /// Inclusive price bounds; must satisfy `0 < min_price < max_price`.
    pub min_price: i128,
    pub max_price: i128,
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

/// Upper-case an asset code (alias normalization) and canonicalize native XLM.
fn normalized_asset(env: &Env, asset: &Asset) -> Result<Asset, OracleError> {
    let raw = asset.code.to_bytes();
    if raw.is_empty() || raw.len() > 12 {
        return Err(OracleError::InvalidFeedConfig);
    }
    let mut upper = [0u8; 12];
    for i in 0..raw.len() {
        let b = raw.get_unchecked(i);
        let b = if b.is_ascii_lowercase() { b - 32 } else { b };
        if !b.is_ascii_alphanumeric() {
            return Err(OracleError::InvalidFeedConfig);
        }
        upper[i as usize] = b;
    }
    let code = Bytes::from_slice(env, &upper[..raw.len() as usize]);
    let native = code == Bytes::from_slice(env, b"XLM");
    Ok(Asset {
        code: String::from_bytes(env, &upper[..raw.len() as usize]),
        // XLM is always native; an issuer on XLM is just an alias spelling.
        issuer: if native { None } else { asset.issuer.clone() },
    })
}

/// Deterministic byte identifier for one normalized asset.
fn asset_id(env: &Env, asset: &Asset) -> Bytes {
    let mut id = asset.code.to_bytes();
    id.append(&Bytes::from_slice(env, b":"));
    match &asset.issuer {
        Some(issuer) => id.append(&issuer.to_string().to_bytes()),
        None => id.append(&Bytes::from_slice(env, b"native")),
    }
    id
}

fn bytes_cmp(a: &Bytes, b: &Bytes) -> Ordering {
    let len = if a.len() < b.len() { a.len() } else { b.len() };
    for i in 0..len {
        match a.get_unchecked(i).cmp(&b.get_unchecked(i)) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

/// Return the canonical representation of `pair`, or `InvalidFeedConfig` when
/// both sides denote the same asset. See module docs for the policy.
pub fn canonical_pair(env: &Env, pair: &AssetPair) -> Result<AssetPair, OracleError> {
    let base = normalized_asset(env, &pair.base)?;
    let quote = normalized_asset(env, &pair.quote)?;
    if bytes_cmp(&asset_id(env, &base), &asset_id(env, &quote)) == Ordering::Equal {
        return Err(OracleError::InvalidFeedConfig);
    }
    if bytes_cmp(&asset_id(env, &base), &asset_id(env, &quote)) == Ordering::Greater {
        Ok(AssetPair {
            base: quote,
            quote: base,
        })
    } else {
        Ok(AssetPair { base, quote })
    }
}

/// Canonical key for lookups; a malformed pair simply has no configured feed.
fn lookup_pair(env: &Env, pair: &AssetPair) -> AssetPair {
    canonical_pair(env, pair).unwrap_or_else(|_| pair.clone())
}

pub fn get_config(env: &Env, pair: &AssetPair) -> Option<FeedConfig> {
    env.storage()
        .persistent()
        .get(&FeedKey::Config(lookup_pair(env, pair)))
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

/// Reject an observation outside the configured inclusive price bounds before
/// any latest-price state is written. Unconfigured feeds are unrestricted.
pub fn check_bounds(env: &Env, pair: &AssetPair, price: i128) -> Result<(), OracleError> {
    if let Some(cfg) = get_config(env, pair) {
        if price < cfg.min_price || price > cfg.max_price {
            return Err(OracleError::PriceOutOfBounds);
        }
    }
    Ok(())
}

/// Register feeds that must not already exist under their canonical identifier.
pub fn register_batch(env: &Env, entries: &Vec<FeedConfigInput>) -> Result<(), OracleError> {
    let mut pairs = Vec::new(env);
    for e in entries.iter() {
        let canonical = canonical_pair(env, &e.pair)?;
        if get_config(env, &canonical).is_some() {
            return Err(OracleError::OracleAlreadyExists);
        }
        for previous in pairs.iter() {
            if previous == canonical {
                return Err(OracleError::InvalidFeedConfig);
            }
        }
        pairs.push_back(canonical);
    }
    configure_batch(env, entries)
}

/// Validate all entries, then write them. Nothing is written on error.
pub fn configure_batch(env: &Env, entries: &Vec<FeedConfigInput>) -> Result<(), OracleError> {
    if entries.is_empty() || entries.len() > MAX_FEED_BATCH {
        return Err(OracleError::InvalidFeedBatch);
    }
    let mut pairs = Vec::new(env);
    for i in 0..entries.len() {
        let e = entries.get_unchecked(i);
        if e.max_age == 0
            || e.min_price <= 0
            || e.max_price <= e.min_price
            || e.max_movement_bps > 10_000
        {
            return Err(OracleError::InvalidFeedConfig);
        }
        validate_publishers(&e.publishers)?;
        // Normalize before duplicate detection: reversed pairs and aliases
        // resolve to one canonical feed identifier.
        let canonical = canonical_pair(env, &e.pair)?;
        for previous in pairs.iter() {
            if previous == canonical {
                return Err(OracleError::InvalidFeedConfig);
            }
        }
        pairs.push_back(canonical);
    }

    for (i, e) in entries.iter().enumerate() {
        let pair = pairs.get_unchecked(i as u32);
        let version = get_config(env, &pair).map(|c| c.version).unwrap_or(0) + 1;
        // #1305: the configured threshold governs the breaker for this feed.
        price_cb::set_threshold(env, &pair, e.max_movement_bps);
        put_config(
            env,
            &pair,
            &FeedConfig {
                owner: e.owner,
                publishers: e.publishers,
                max_age: e.max_age,
                enabled: e.enabled,
                max_movement_bps: e.max_movement_bps,
                min_price: e.min_price,
                max_price: e.max_price,
                version,
            },
        );
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("feed_cfg")),
            (pair, version),
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
    put_config(env, &lookup_pair(env, pair), &cfg);
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
    put_config(env, &lookup_pair(env, pair), &cfg);
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
    let key = FeedKey::Round(lookup_pair(env, pair));
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
        .get(&FeedKey::Round(lookup_pair(env, pair)));
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
