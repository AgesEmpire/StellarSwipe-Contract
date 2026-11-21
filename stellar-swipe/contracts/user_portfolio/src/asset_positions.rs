//! Canonical asset-keyed positions (issue #1368) with idempotent balance
//! updates by operation id (issue #1369).
//!
//! Canonical form: `XLM` for the native asset (`native`/`xlm`, any case), or
//! `CODE:ISSUER` with a 1-12 char alphanumeric code and a 56-char `G...` issuer,
//! both uppercased. `-` is accepted as a separator. Anything else is rejected.

use soroban_sdk::{contracttype, Address, BytesN, Env, String};

use crate::PortfolioError;

const ISSUER_LEN: usize = 56;
const MAX_CODE_LEN: usize = 12;
const MAX_INPUT_LEN: usize = MAX_CODE_LEN + 1 + ISSUER_LEN;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationRecord {
    pub user: Address,
    pub asset: String,
    pub delta: i128,
    pub result: i128,
}

#[contracttype]
enum AssetKey {
    Position(Address, String),
    Operation(BytesN<32>),
}

pub fn canonicalize(env: &Env, raw: &String) -> Result<String, PortfolioError> {
    let len = raw.len() as usize;
    if len == 0 || len > MAX_INPUT_LEN {
        return Err(PortfolioError::InvalidAssetId);
    }
    let mut buf = [0u8; MAX_INPUT_LEN];
    raw.copy_into_slice(&mut buf[..len]);
    let s = &mut buf[..len];
    s.make_ascii_uppercase();
    if s == b"NATIVE" || s == b"XLM" {
        return Ok(String::from_str(env, "XLM"));
    }
    let sep = s
        .iter()
        .position(|&c| c == b':' || c == b'-')
        .ok_or(PortfolioError::InvalidAssetId)?;
    let (code, issuer) = (&s[..sep], &s[sep + 1..]);
    if code.is_empty()
        || code.len() > MAX_CODE_LEN
        || !code.iter().all(|c| c.is_ascii_alphanumeric())
        || issuer.len() != ISSUER_LEN
        || issuer[0] != b'G'
        || !issuer.iter().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(PortfolioError::InvalidAssetId);
    }
    s[sep] = b':';
    let out = core::str::from_utf8(s).map_err(|_| PortfolioError::InvalidAssetId)?;
    Ok(String::from_str(env, out))
}

fn key(env: &Env, user: &Address, asset: &String) -> Result<AssetKey, PortfolioError> {
    Ok(AssetKey::Position(user.clone(), canonicalize(env, asset)?))
}

pub fn create(env: &Env, user: &Address, asset: &String) -> Result<String, PortfolioError> {
    let k = key(env, user, asset)?;
    if env.storage().persistent().has(&k) {
        return Err(PortfolioError::AssetPositionExists);
    }
    env.storage().persistent().set(&k, &0i128);
    let AssetKey::Position(_, canonical) = k else { unreachable!() };
    Ok(canonical)
}

pub fn get(env: &Env, user: &Address, asset: &String) -> Result<Option<i128>, PortfolioError> {
    Ok(env.storage().persistent().get(&key(env, user, asset)?))
}

pub fn remove(env: &Env, user: &Address, asset: &String) -> Result<(), PortfolioError> {
    let k = key(env, user, asset)?;
    if !env.storage().persistent().has(&k) {
        return Err(PortfolioError::AssetPositionNotFound);
    }
    env.storage().persistent().remove(&k);
    Ok(())
}

/// Apply `delta` once per `op_id`. A retry with identical parameters returns
/// the original result; reuse with different parameters is rejected.
pub fn apply_delta(
    env: &Env,
    user: &Address,
    asset: &String,
    delta: i128,
    op_id: BytesN<32>,
) -> Result<i128, PortfolioError> {
    let canonical = canonicalize(env, asset)?;
    let op_key = AssetKey::Operation(op_id);
    let store = env.storage().persistent();
    if let Some(rec) = store.get::<_, OperationRecord>(&op_key) {
        if rec.user == *user && rec.asset == canonical && rec.delta == delta {
            return Ok(rec.result);
        }
        return Err(PortfolioError::OperationIdConflict);
    }
    let pos_key = AssetKey::Position(user.clone(), canonical.clone());
    let current: i128 = store
        .get(&pos_key)
        .ok_or(PortfolioError::AssetPositionNotFound)?;
    let result = current
        .checked_add(delta)
        .filter(|v| *v >= 0)
        .ok_or(PortfolioError::InvalidBalanceDelta)?;
    store.set(&pos_key, &result);
    store.set(
        &op_key,
        &OperationRecord { user: user.clone(), asset: canonical, delta, result },
    );
    Ok(result)
}
