#![allow(dead_code)]
//! Position management with open/close tracking and P&L calculation.
//!
//! Issues #191 (open_position) and #192 (close_position).

use soroban_sdk::{contracttype, Address, BytesN, Env, Map, Vec};

/// Position status
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionStatus {
    Open,
    Closed,
}

/// Full position record stored on-chain.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionData {
    pub trade_id: BytesN<32>,
    pub user: Address,
    pub signal_id: u64,
    pub asset_pair: u32,
    pub amount: i128,
    pub entry_price: i128,
    pub stop_loss: i128,
    pub take_profit: i128,
    pub status: PositionStatus,
    pub exit_price: i128,
    pub pnl: i128,
    pub opened_at: u64,
    pub closed_at: u64,
}

/// Result returned by close_position.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionResult {
    pub trade_id: BytesN<32>,
    pub entry_price: i128,
    pub exit_price: i128,
    pub pnl: i128,
    pub amount: i128,
    pub closed_at: u64,
}

/// Storage keys for positions.
#[contracttype]
pub enum PositionKey {
    /// Individual position by trade_id
    Position(BytesN<32>),
    /// List of trade_ids per user
    UserPositions(Address),
    /// Counter for generating unique trade ids
    PositionNonce,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_nonce(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&PositionKey::PositionNonce)
        .unwrap_or(0)
}

fn set_nonce(env: &Env, nonce: u64) {
    env.storage()
        .persistent()
        .set(&PositionKey::PositionNonce, &nonce);
}

fn generate_trade_id(env: &Env, user: &Address, signal_id: u64) -> BytesN<32> {
    let nonce = get_nonce(env);
    set_nonce(env, nonce + 1);

    // Build a unique preimage: user strkey bytes + signal_id + nonce + timestamp
    let mut preimage = soroban_sdk::Bytes::new(env);
    let user_bytes = user.to_string().to_bytes();
    preimage.append(&user_bytes);
    preimage.append(&soroban_sdk::Bytes::from_array(
        env,
        &signal_id.to_be_bytes(),
    ));
    preimage.append(&soroban_sdk::Bytes::from_array(env, &nonce.to_be_bytes()));
    preimage.append(&soroban_sdk::Bytes::from_array(
        env,
        &env.ledger().timestamp().to_be_bytes(),
    ));

    env.crypto().sha256(&preimage).into()
}

fn save_position(env: &Env, position: &PositionData) {
    env.storage()
        .persistent()
        .set(&PositionKey::Position(position.trade_id.clone()), position);
}

pub fn get_position(env: &Env, trade_id: &BytesN<32>) -> Option<PositionData> {
    env.storage()
        .persistent()
        .get(&PositionKey::Position(trade_id.clone()))
}

fn get_user_trade_ids(env: &Env, user: &Address) -> Vec<BytesN<32>> {
    env.storage()
        .persistent()
        .get(&PositionKey::UserPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn save_user_trade_ids(env: &Env, user: &Address, ids: &Vec<BytesN<32>>) {
    env.storage()
        .persistent()
        .set(&PositionKey::UserPositions(user.clone()), ids);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Open a new position. Returns the generated `trade_id` (BytesN<32>).
///
/// Issue #191
pub fn open_position(
    env: &Env,
    user: &Address,
    signal_id: u64,
    asset_pair: u32,
    amount: i128,
    entry_price: i128,
    stop_loss: i128,
    take_profit: i128,
) -> BytesN<32> {
    let trade_id = generate_trade_id(env, user, signal_id);

    let position = PositionData {
        trade_id: trade_id.clone(),
        user: user.clone(),
        signal_id,
        asset_pair,
        amount,
        entry_price,
        stop_loss,
        take_profit,
        status: PositionStatus::Open,
        exit_price: 0,
        pnl: 0,
        opened_at: env.ledger().timestamp(),
        closed_at: 0,
    };

    save_position(env, &position);

    // Append to user's trade id list
    let mut ids = get_user_trade_ids(env, user);
    ids.push_back(trade_id.clone());
    save_user_trade_ids(env, user, &ids);

    trade_id
}

/// Close an existing position and calculate P&L.
///
/// Issue #192
pub fn close_position(
    env: &Env,
    user: &Address,
    trade_id: &BytesN<32>,
    exit_price: i128,
) -> Option<PositionResult> {
    let mut position = get_position(env, trade_id)?;

    // Only the owner can close, and it must be open
    if position.user != *user || position.status != PositionStatus::Open {
        return None;
    }

    // P&L = (exit_price - entry_price) * amount
    let pnl = (exit_price - position.entry_price) * position.amount;

    position.status = PositionStatus::Closed;
    position.exit_price = exit_price;
    position.pnl = pnl;
    position.closed_at = env.ledger().timestamp();

    save_position(env, &position);

    Some(PositionResult {
        trade_id: trade_id.clone(),
        entry_price: position.entry_price,
        exit_price,
        pnl,
        amount: position.amount,
        closed_at: position.closed_at,
    })
}

/// Get all positions (open and closed) for a user.
///
/// Issue #193
pub fn get_all_positions(env: &Env, user: &Address) -> Vec<PositionData> {
    let ids = get_user_trade_ids(env, user);
    let mut result = Vec::new(env);

    for i in 0..ids.len() {
        if let Some(id) = ids.get(i) {
            if let Some(pos) = get_position(env, &id) {
                result.push_back(pos);
            }
        }
    }

    result
}

/// Get only open positions for a user.
pub fn get_open_positions(env: &Env, user: &Address) -> Vec<PositionData> {
    let ids = get_user_trade_ids(env, user);
    let mut result = Vec::new(env);

    for i in 0..ids.len() {
        if let Some(id) = ids.get(i) {
            if let Some(pos) = get_position(env, &id) {
                if pos.status == PositionStatus::Open {
                    result.push_back(pos);
                }
            }
        }
    }

    result
}

/// Get only closed positions for a user.
pub fn get_closed_positions(env: &Env, user: &Address) -> Vec<PositionData> {
    let ids = get_user_trade_ids(env, user);
    let mut result = Vec::new(env);

    for i in 0..ids.len() {
        if let Some(id) = ids.get(i) {
            if let Some(pos) = get_position(env, &id) {
                if pos.status == PositionStatus::Closed {
                    result.push_back(pos);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as TestAddress, Ledger};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.ledger().set_timestamp(1000);
        let contract_id = env.register(TestContract, ());
        (env, contract_id)
    }

    #[test]
    fn test_open_position() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let trade_id = open_position(&env, &user, 1, 42, 1000, 500, 400, 700);
            assert_eq!(trade_id.len(), 32);

            let pos = get_position(&env, &trade_id).unwrap();
            assert_eq!(pos.user, user);
            assert_eq!(pos.signal_id, 1);
            assert_eq!(pos.asset_pair, 42);
            assert_eq!(pos.amount, 1000);
            assert_eq!(pos.entry_price, 500);
            assert_eq!(pos.stop_loss, 400);
            assert_eq!(pos.take_profit, 700);
            assert_eq!(pos.status, PositionStatus::Open);
            assert_eq!(pos.pnl, 0);
            assert_eq!(pos.opened_at, 1000);
        });
    }

    #[test]
    fn test_close_position_profit() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let trade_id = open_position(&env, &user, 1, 42, 100, 500, 400, 700);

            env.ledger().set_timestamp(2000);
            let result = close_position(&env, &user, &trade_id, 600).unwrap();

            // P&L = (600 - 500) * 100 = 10000
            assert_eq!(result.pnl, 10000);
            assert_eq!(result.exit_price, 600);
            assert_eq!(result.closed_at, 2000);

            let pos = get_position(&env, &trade_id).unwrap();
            assert_eq!(pos.status, PositionStatus::Closed);
        });
    }

    #[test]
    fn test_close_position_loss() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let trade_id = open_position(&env, &user, 1, 42, 100, 500, 400, 700);

            env.ledger().set_timestamp(2000);
            let result = close_position(&env, &user, &trade_id, 300).unwrap();

            // P&L = (300 - 500) * 100 = -20000
            assert_eq!(result.pnl, -20000);
        });
    }

    #[test]
    fn test_cannot_close_others_position() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);
        let other = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let trade_id = open_position(&env, &user, 1, 42, 100, 500, 400, 700);
            let result = close_position(&env, &other, &trade_id, 600);
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_cannot_close_twice() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let trade_id = open_position(&env, &user, 1, 42, 100, 500, 400, 700);
            close_position(&env, &user, &trade_id, 600).unwrap();

            let result = close_position(&env, &user, &trade_id, 700);
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_get_all_positions() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let id1 = open_position(&env, &user, 1, 42, 100, 500, 400, 700);
            let _id2 = open_position(&env, &user, 2, 43, 200, 600, 500, 800);
            close_position(&env, &user, &id1, 550);

            let all = get_all_positions(&env, &user);
            assert_eq!(all.len(), 2);

            let open = get_open_positions(&env, &user);
            assert_eq!(open.len(), 1);

            let closed = get_closed_positions(&env, &user);
            assert_eq!(closed.len(), 1);
            assert_eq!(closed.get(0).unwrap().pnl, (550 - 500) * 100);
        });
    }

    #[test]
    fn test_unique_trade_ids() {
        let (env, contract_id) = setup();
        let user = Address::generate(&env);

        env.as_contract(&contract_id, || {
            let id1 = open_position(&env, &user, 1, 42, 100, 500, 400, 700);
            let id2 = open_position(&env, &user, 1, 42, 100, 500, 400, 700);
            assert_ne!(id1, id2);
        });
    }
}
