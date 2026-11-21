#![allow(dead_code)]
//! Trade history storage and query.
//!
//! Stores all executed trades per user with full details.
//! Gas: ~O(limit) per get_trade_history query.

use soroban_sdk::{contracttype, Address, Env, Vec};
/// Default page size for trade history
pub const DEFAULT_HISTORY_LIMIT: u32 = 20;

/// Maximum trades per page
pub const MAX_HISTORY_LIMIT: u32 = 100;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryTradeStatus {
    Pending,
    Executed,
    Failed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct HistoryTrade {
    pub id: u64,
    pub signal_id: u64,
    pub base_asset: u32,
    pub amount: i128,
    pub price: i128,
    pub fee: i128,
    pub timestamp: u64,
    pub status: HistoryTradeStatus,
}

#[contracttype]
pub enum HistoryDataKey {
    UserTradeCount(Address),
    Trade(Address, u64),
}

/// Get number of trades for a user
pub fn get_user_trade_count(env: &Env, user: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&HistoryDataKey::UserTradeCount(user.clone()))
        .unwrap_or(0)
}

/// Record a trade to history. Called after successful execution.
#[allow(clippy::too_many_arguments)]
pub fn record_trade(
    env: &Env,
    user: &Address,
    signal_id: u64,
    base_asset: u32,
    amount: i128,
    price: i128,
    fee: i128,
    status: HistoryTradeStatus,
) -> u64 {
    let count = get_user_trade_count(env, user);
    let id = count;

    let trade = HistoryTrade {
        id,
        signal_id,
        base_asset,
        amount,
        price,
        fee,
        timestamp: env.ledger().timestamp(),
        status,
    };

    env.storage()
        .persistent()
        .set(&HistoryDataKey::Trade(user.clone(), id), &trade);

    let new_count = count + 1;
    env.storage()
        .persistent()
        .set(&HistoryDataKey::UserTradeCount(user.clone()), &new_count);

    id
}

/// Get trade by user and index
fn get_trade_by_index(env: &Env, user: &Address, index: u64) -> Option<HistoryTrade> {
    env.storage()
        .persistent()
        .get(&HistoryDataKey::Trade(user.clone(), index))
}

/// Get trade history for user, newest first, using stable cursor pagination
/// (Issue #1446).
///
/// Unlike `get_trade_history` below, this is immune to results shifting under
/// concurrent writes: the cursor is the trade `id` (a monotonic per-user
/// sequence number assigned at `record_trade` time), not a positional offset.
/// New trades recorded between page fetches always get a *larger* id, so they
/// never shift already-returned ids earlier in the sequence and are simply
/// not part of the chain starting at `after_id`.
///
/// `after_id`: `None` starts from the newest trade; `Some(id)` resumes
/// strictly before `id` (exclusive), i.e. continues from the cursor returned
/// by the previous page.
///
/// Returns `(page, next_cursor)` where `next_cursor` is `Some(last_id_returned)`
/// to pass as `after_id` on the next call, or `None` once there are no more
/// older trades.
pub fn get_trade_history_by_cursor(
    env: &Env,
    user: &Address,
    after_id: Option<u64>,
    limit: u32,
) -> (Vec<HistoryTrade>, Option<u64>) {
    let count = get_user_trade_count(env, user);
    if count == 0 {
        return (Vec::new(env), None);
    }

    let limit = if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else if limit > MAX_HISTORY_LIMIT {
        MAX_HISTORY_LIMIT
    } else {
        limit
    };

    // Start strictly before `after_id`, or from the newest id if no cursor
    // was supplied. `start` is exclusive upper bound (we iterate downward).
    let start = match after_id {
        Some(id) if id == 0 => return (Vec::new(env), None),
        Some(id) => id, // iterate ids < id
        None => count,  // iterate ids < count, i.e. from the newest
    };

    let mut result = Vec::new(env);
    let mut taken = 0u32;
    let mut last_id: Option<u64> = None;

    let mut i = start;
    while i > 0 && taken < limit {
        i -= 1;
        if let Some(trade) = get_trade_by_index(env, user, i) {
            result.push_back(trade);
            last_id = Some(i);
            taken += 1;
        }
    }

    // There is a next page only if we stopped above id 0.
    let next_cursor = match last_id {
        Some(id) if id > 0 => Some(id),
        _ => None,
    };

    (result, next_cursor)
}

/// Get trade history for user, newest first, with offset-based pagination.
///
/// NOTE (Issue #1446): offset pagination is unstable under concurrent writes
/// — a trade recorded between two page fetches shifts every subsequent id
/// "behind" it, which can cause skipped or duplicated entries across pages.
/// New integrations should prefer `get_trade_history_by_cursor`, which keys
/// off the stable trade id rather than a positional offset. This entrypoint
/// is retained for backward compatibility.
pub fn get_trade_history(env: &Env, user: &Address, offset: u32, limit: u32) -> Vec<HistoryTrade> {
    let count = get_user_trade_count(env, user);
    if count == 0 {
        return Vec::new(env);
    }

    let limit = if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else if limit > MAX_HISTORY_LIMIT {
        MAX_HISTORY_LIMIT
    } else {
        limit
    };

    let mut result = Vec::new(env);
    let total = count;
    let mut taken = 0u32;
    let mut skipped = 0u32;

    for i in (0..total).rev() {
        if skipped < offset {
            skipped += 1;
            continue;
        }
        if taken >= limit {
            break;
        }
        if let Some(trade) = get_trade_by_index(env, user, i) {
            result.push_back(trade);
            taken += 1;
        }
    }

    result
}
