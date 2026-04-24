//! Read-side P&L aggregation: realized from closed positions, unrealized via oracle price.

use crate::storage::DataKey;
use crate::{PnlSummary, Position, PositionStatus};
use soroban_sdk::{Address, Env, Vec};
use stellar_swipe_common::{
    oracle_price_to_i128, validate_freshness, IOracleClient, OnChainOracleClient,
};

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
