//! Read-side P&L aggregation: realized from closed positions, unrealized via oracle price.

use crate::storage::DataKey;
use crate::{
    PnlSummary, Portfolio, PortfolioPosition, Position, PositionStatus, TradeHistoryEntry,
};
use soroban_sdk::{Address, Env, Vec};
use stellar_swipe_common::{
    oracle_price_to_i128, validate_freshness, IOracleClient, OnChainOracleClient,
};

const MAX_INLINE_CLOSED_POSITIONS: u32 = 20;
const MAX_TRADE_HISTORY_LIMIT: u32 = 50;

// --- Portfolio budget notes (Issue #303) ---
// Active-trader snapshots use per-user open/closed indexes. `include_closed=false`
// loads only open position structs, so a user with 20 open + 50 closed positions avoids
// loading 50 closed structs. When closed history is requested and it is large, the query
// returns closed IDs for lazy follow-up rather than full closed position structs.

/// Sum closed `realized_pnl`, optionally sum open unrealized using oracle `get_price(asset_pair) -> OraclePrice`.
/// If the oracle call fails, returns realized-only totals with `unrealized_pnl: None`.
pub fn compute_get_pnl(env: &Env, user: Address) -> PnlSummary {
    let oracle: Address = env
        .storage()
        .instance()
        .get(&DataKey::Oracle)
        .expect("oracle not configured");
    let asset_pair: u32 = env
        .storage()
        .instance()
        .get(&DataKey::OracleAssetPair)
        .unwrap_or(0);

    let ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));

    let mut realized: i128 = 0;
    let mut total_invested: i128 = 0;
    let mut has_open = false;

    for i in 0..ids.len() {
        let Some(id) = ids.get(i) else {
            continue;
        };
        let key = DataKey::Position(id);
        let Some(pos) = env.storage().persistent().get::<DataKey, Position>(&key) else {
            continue;
        };

        match pos.status {
            PositionStatus::Open => {
                has_open = true;
                if let Some(s) = total_invested.checked_add(pos.amount) {
                    total_invested = s;
                }
            }
            PositionStatus::Closed => {
                if let Some(s) = realized.checked_add(pos.realized_pnl) {
                    realized = s;
                }
                if let Some(s) = total_invested.checked_add(pos.amount) {
                    total_invested = s;
                }
            }
        }
    }

    let current_price = OnChainOracleClient { address: oracle }
        .get_price(env, asset_pair)
        .ok()
        .and_then(|price| {
            validate_freshness(env, &price)
                .ok()
                .map(|_| oracle_price_to_i128(&price))
        });

    let unrealized_pnl: Option<i128> = if !has_open {
        Some(0_i128)
    } else if let Some(price) = current_price {
        let mut unrealized: i128 = 0;
        for i in 0..ids.len() {
            let Some(id) = ids.get(i) else {
                continue;
            };
            let key = DataKey::Position(id);
            let Some(pos) = env.storage().persistent().get::<DataKey, Position>(&key) else {
                continue;
            };
            if pos.status != PositionStatus::Open || pos.entry_price == 0 {
                continue;
            }
            let diff = match price.checked_sub(pos.entry_price) {
                Some(d) => d,
                None => continue,
            };
            let num = match diff.checked_mul(pos.amount) {
                Some(n) => n,
                None => continue,
            };
            let contrib = match num.checked_div(pos.entry_price) {
                Some(c) => c,
                None => continue,
            };
            if let Some(u) = unrealized.checked_add(contrib) {
                unrealized = u;
            }
        }
        Some(unrealized)
    } else {
        None
    };

    let total_pnl = match unrealized_pnl {
        Some(u) => realized.checked_add(u).unwrap_or(realized),
        None => realized,
    };

    let roi_bps = roi_basis_points(total_pnl, total_invested);

    PnlSummary {
        realized_pnl: realized,
        unrealized_pnl,
        total_pnl,
        roi_bps,
    }
}

pub fn get_portfolio(env: &Env, user: Address, include_closed: bool) -> Portfolio {
    let open_ids = get_open_position_ids(env, user.clone());
    let mut open_positions = Vec::new(env);

    for i in 0..open_ids.len() {
        let Some(position_id) = open_ids.get(i) else {
            continue;
        };
        let Some(position) = env
            .storage()
            .persistent()
            .get::<DataKey, Position>(&DataKey::Position(position_id))
        else {
            continue;
        };
        if position.status == PositionStatus::Open {
            open_positions.push_back(PortfolioPosition {
                position_id,
                position,
            });
        }
    }

    let mut closed_positions = Vec::new(env);
    let mut closed_position_ids = Vec::new(env);

    if include_closed {
        let closed_ids = get_closed_position_ids(env, user);
        for i in 0..closed_ids.len() {
            if let Some(position_id) = closed_ids.get(i) {
                closed_position_ids.push_back(position_id);
            }
        }

        if closed_ids.len() <= MAX_INLINE_CLOSED_POSITIONS {
            for i in 0..closed_ids.len() {
                let Some(position_id) = closed_ids.get(i) else {
                    continue;
                };
                let Some(position) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, Position>(&DataKey::Position(position_id))
                else {
                    continue;
                };
                if position.status == PositionStatus::Closed {
                    closed_positions.push_back(PortfolioPosition {
                        position_id,
                        position,
                    });
                }
            }
        }
    }

    Portfolio {
        open_positions,
        closed_positions,
        closed_position_ids,
    }
}

pub fn get_trade_history(
    env: &Env,
    user: Address,
    cursor: Option<u64>,
    limit: u32,
) -> Vec<TradeHistoryEntry> {
    let page_limit = limit.min(MAX_TRADE_HISTORY_LIMIT);
    let mut page = Vec::new(env);
    if page_limit == 0 {
        return page;
    }

    let closed_ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserClosedPositions(user.clone()))
        .unwrap_or_else(|| rebuild_closed_position_index(env, user));

    let mut next_index = closed_ids.len();
    if let Some(cursor_id) = cursor {
        for i in 0..closed_ids.len() {
            if closed_ids.get(i) == Some(cursor_id) {
                next_index = i;
                break;
            }
        }
    }

    while next_index > 0 && page.len() < page_limit {
        next_index -= 1;
        let Some(trade_id) = closed_ids.get(next_index) else {
            continue;
        };
        let Some(position) = env
            .storage()
            .persistent()
            .get::<DataKey, Position>(&DataKey::Position(trade_id))
        else {
            continue;
        };
        if position.status != PositionStatus::Closed {
            continue;
        }
        page.push_back(TradeHistoryEntry { trade_id, position });
    }

    page
}

fn get_open_position_ids(env: &Env, user: Address) -> Vec<u64> {
    if let Some(ids) = env
        .storage()
        .persistent()
        .get(&DataKey::UserOpenPositions(user.clone()))
    {
        return ids;
    }
    rebuild_position_indexes(env, user).0
}

fn get_closed_position_ids(env: &Env, user: Address) -> Vec<u64> {
    if let Some(ids) = env
        .storage()
        .persistent()
        .get(&DataKey::UserClosedPositions(user.clone()))
    {
        return ids;
    }
    rebuild_position_indexes(env, user).1
}

fn rebuild_closed_position_index(env: &Env, user: Address) -> Vec<u64> {
    rebuild_position_indexes(env, user).1
}

fn rebuild_position_indexes(env: &Env, user: Address) -> (Vec<u64>, Vec<u64>) {
    let ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut open_ids = Vec::new(env);
    let mut closed_ids = Vec::new(env);

    for i in 0..ids.len() {
        let Some(id) = ids.get(i) else {
            continue;
        };
        let Some(position) = env
            .storage()
            .persistent()
            .get::<DataKey, Position>(&DataKey::Position(id))
        else {
            continue;
        };
        match position.status {
            PositionStatus::Open => open_ids.push_back(id),
            PositionStatus::Closed => closed_ids.push_back(id),
        }
    }

    env.storage()
        .persistent()
        .set(&DataKey::UserOpenPositions(user.clone()), &open_ids);
    env.storage()
        .persistent()
        .set(&DataKey::UserClosedPositions(user), &closed_ids);
    (open_ids, closed_ids)
}

fn roi_basis_points(total_pnl: i128, total_invested: i128) -> i32 {
    if total_invested == 0 {
        return 0;
    }
    let num = match total_pnl.checked_mul(10_000) {
        Some(n) => n,
        None => return 0,
    };
    let q = match num.checked_div(total_invested) {
        Some(v) => v,
        None => return 0,
    };
    if q > i32::MAX as i128 {
        i32::MAX
    } else if q < i32::MIN as i128 {
        i32::MIN
    } else {
        q as i32
    }
}
