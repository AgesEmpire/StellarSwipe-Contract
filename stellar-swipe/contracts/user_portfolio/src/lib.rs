//! User portfolio contract: positions, `get_pnl`, and on-chain badges.

#![cfg_attr(target_family = "wasm", no_std)]

mod badges;
mod queries;
mod storage;

pub use badges::{Badge, BadgeType};

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Vec};
use storage::DataKey;

/// Aggregated P&L for display. When the oracle cannot supply a price and there are open
/// positions, `unrealized_pnl` is `None` and `total_pnl` equals `realized_pnl` only.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PnlSummary {
    pub realized_pnl: i128,
    pub unrealized_pnl: Option<i128>,
    pub total_pnl: i128,
    pub roi_bps: i32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionStatus {
    Open = 0,
    Closed = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    pub entry_price: i128,
    pub amount: i128,
    pub status: PositionStatus,
    /// Set when `status == Closed`; ignored while open.
    pub realized_pnl: i128,
    /// Ledger close time when `status == Closed`; `0` while open.
    pub closed_at: u64,
}

/// One closed leg in `get_trade_history` (newest-first pages).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeHistoryEntry {
    pub trade_id: u64,
    pub entry_price: i128,
    pub amount: i128,
    pub realized_pnl: i128,
    pub closed_at: u64,
}

#[contract]
pub struct UserPortfolio;

#[contractimpl]
impl UserPortfolio {
    /// One-time setup: admin, oracle (`get_price() -> i128`), and max users who receive `EarlyAdopter`.
    pub fn initialize(
        env: Env,
        admin: Address,
        oracle: Address,
        early_adopter_user_cap: u32,
    ) {
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage().instance().set(&DataKey::NextPositionId, &1u64);
        env.storage()
            .instance()
            .set(&DataKey::EarlyAdopterCap, &early_adopter_user_cap);
        env.storage()
            .instance()
            .set(&DataKey::TotalUsersFirstOpen, &0u32);
    }

    pub fn set_oracle(env: Env, oracle: Address) {
        Self::require_admin(&env);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
    }

    /// Backend / admin sets current leaderboard rank (1 = best). Checked on open/close for `Top10Leaderboard`.
    pub fn set_leaderboard_rank(env: Env, user: Address, rank: u32) {
        Self::require_admin(&env);
        env.storage()
            .persistent()
            .set(&DataKey::LeaderboardRank(user), &rank);
    }

    /// Opens a position for `user` (caller must be `user`). `amount` is invested notional at entry.
    pub fn open_position(env: Env, user: Address, entry_price: i128, amount: i128) -> u64 {
        user.require_auth();
        if entry_price <= 0 || amount <= 0 {
            panic!("invalid entry_price or amount");
        }
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextPositionId)
            .expect("next id");
        let next = id.checked_add(1).expect("position id overflow");
        env.storage().instance().set(&DataKey::NextPositionId, &next);

        let pos = Position {
            entry_price,
            amount,
            status: PositionStatus::Open,
            realized_pnl: 0,
            closed_at: 0,
        };
        env.storage().persistent().set(&DataKey::Position(id), &pos);

        let key = DataKey::UserPositions(user.clone());
        let mut list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        let is_first_ever_open = list.is_empty();
        list.push_back(id);
        env.storage().persistent().set(&key, &list);

        badges::after_open_position(&env, &user, is_first_ever_open);

        id
    }

    /// Closes an open position and records realized P&L for that leg.
    pub fn close_position(env: Env, user: Address, position_id: u64, realized_pnl: i128) {
        user.require_auth();
        let key = DataKey::UserPositions(user.clone());
        let list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        let mut found = false;
        for i in 0..list.len() {
            if let Some(pid) = list.get(i) {
                if pid == position_id {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            panic!("position not found for user");
        }

        let pkey = DataKey::Position(position_id);
        let mut pos: Position = env
            .storage()
            .persistent()
            .get(&pkey)
            .expect("position missing");
        if pos.status != PositionStatus::Open {
            panic!("position not open");
        }
        pos.status = PositionStatus::Closed;
        pos.realized_pnl = realized_pnl;
        pos.closed_at = env.ledger().timestamp();
        env.storage().persistent().set(&pkey, &pos);

        let chrono_key = DataKey::UserClosedChronological(user.clone());
        let mut closed_order: Vec<u64> = env
            .storage()
            .persistent()
            .get(&chrono_key)
            .unwrap_or_else(|| Vec::new(&env));
        closed_order.push_back(position_id);
        env.storage().persistent().set(&chrono_key, &closed_order);

        badges::after_close_position(&env, &user, realized_pnl);
    }

    /// All badges earned by `user`, in award order.
    pub fn get_badges(env: Env, user: Address) -> Vec<Badge> {
        badges::get_badges(&env, user)
    }

    /// Portfolio P&L including open positions when oracle price is available.
    pub fn get_pnl(env: Env, user: Address) -> PnlSummary {
        queries::compute_get_pnl(&env, user)
    }

    /// Paginated closed trades, newest first. `cursor` is `trade_id` of the last item from the prior page; `limit` at most 50.
    pub fn get_trade_history(
        env: Env,
        user: Address,
        cursor: Option<u64>,
        limit: u32,
    ) -> Vec<TradeHistoryEntry> {
        queries::get_trade_history(&env, user, cursor, limit)
    }

    fn require_admin(env: &Env) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).expect("admin");
        admin.require_auth();
    }
}

#[cfg(test)]
mod oracle_ok {
    use soroban_sdk::{contract, contractimpl, Env, Symbol};

    #[contract]
    pub struct OracleMock;

    #[contractimpl]
    impl OracleMock {
        pub fn set_price(env: Env, price: i128) {
            let key = Symbol::new(&env, "PRICE");
            env.storage().instance().set(&key, &price);
        }

        pub fn get_price(env: Env) -> i128 {
            let key = Symbol::new(&env, "PRICE");
            env.storage().instance().get(&key).unwrap()
        }
    }
}

#[cfg(test)]
mod oracle_fail {
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct OraclePanic;

    #[contractimpl]
    impl OraclePanic {
        pub fn get_price(_env: Env) -> i128 {
            panic!("oracle unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::oracle_fail::OraclePanic;
    use super::oracle_ok::OracleMock;
    use super::oracle_ok::OracleMockClient;
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[allow(deprecated)]
    fn setup_portfolio(
        env: &Env,
        use_working_oracle: bool,
        initial_price: i128,
        early_adopter_cap: u32,
    ) -> (Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let user = Address::generate(env);
        let oracle_id = if use_working_oracle {
            let id = env.register_contract(None, OracleMock);
            OracleMockClient::new(env, &id).set_price(&initial_price);
            id
        } else {
            env.register_contract(None, OraclePanic)
        };
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(env, &contract_id);
        env.mock_all_auths();
        client.initialize(&admin, &oracle_id, &early_adopter_cap);
        (admin, user, contract_id, oracle_id)
    }

    /// All positions closed: unrealized is 0, total = realized, ROI uses invested sums.
    #[test]
    fn get_pnl_all_closed() {
        let env = Env::default();
        let (_, user, portfolio_id, _) = setup_portfolio(&env, true, 100, 1000);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        client.open_position(&user, &100, &1_000);
        client.open_position(&user, &100, &500);
        client.close_position(&user, &1, &200);
        client.close_position(&user, &2, &-50);

        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 150);
        assert_eq!(pnl.unrealized_pnl, Some(0));
        assert_eq!(pnl.total_pnl, 150);
        // invested 1500, roi = 150 * 10000 / 1500 = 1000 bps = 10%
        assert_eq!(pnl.roi_bps, 1000);
    }

    /// Only open positions: realized 0, unrealized from oracle.
    #[test]
    fn get_pnl_all_open() {
        let env = Env::default();
        let (_, user, portfolio_id, oracle_id) = setup_portfolio(&env, true, 100, 1000);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        // entry 100, amount 1000, current 120 -> (120-100)*1000/100 = 200
        client.open_position(&user, &100, &1_000);
        OracleMockClient::new(&env, &oracle_id).set_price(&120);

        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 0);
        assert_eq!(pnl.unrealized_pnl, Some(200));
        assert_eq!(pnl.total_pnl, 200);
        assert_eq!(pnl.roi_bps, 2000); // 200/1000 * 10000
    }

    /// Mixed open + closed.
    #[test]
    fn get_pnl_mixed() {
        let env = Env::default();
        let (_, user, portfolio_id, oracle_id) = setup_portfolio(&env, true, 50, 1000);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        client.open_position(&user, &50, &2_000);
        client.open_position(&user, &50, &1_000);
        client.close_position(&user, &1, &300);

        OracleMockClient::new(&env, &oracle_id).set_price(&60);
        // open pos 2: (60-50)*1000/50 = 200
        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 300);
        assert_eq!(pnl.unrealized_pnl, Some(200));
        assert_eq!(pnl.total_pnl, 500);
        // invested: closed 2000 + open 1000 = 3000
        assert_eq!(pnl.roi_bps, 1666);
    }

    /// Oracle fails: partial result, unrealized None, total = realized only.
    #[test]
    fn get_pnl_oracle_unavailable() {
        let env = Env::default();
        let (_, user, portfolio_id, _) = setup_portfolio(&env, false, 0, 1000);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        client.open_position(&user, &100, &1_000);
        client.close_position(&user, &1, &50);

        client.open_position(&user, &100, &500);
        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 50);
        assert_eq!(pnl.unrealized_pnl, None);
        assert_eq!(pnl.total_pnl, 50);
        // invested: 1000 closed + 500 open = 1500
        assert_eq!(pnl.roi_bps, 333);
    }

    /// 100 closes, pages of 20: full coverage in reverse chronological order.
    #[test]
    fn get_trade_history_paginate_100_by_20() {
        let env = Env::default();
        let (_, user, portfolio_id, _) = setup_portfolio(&env, true, 100, 1000);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        for i in 0..100 {
            let id = client.open_position(&user, &100, &(i as i128 + 1));
            client.close_position(&user, &id, &(i as i128));
        }

        let mut cursor = Option::<u64>::None;
        let mut flat: Vec<u64> = Vec::new(&env);
        loop {
            let page = client.get_trade_history(&user, &cursor, &20);
            if page.is_empty() {
                break;
            }
            for j in 0..page.len() {
                if let Some(e) = page.get(j) {
                    flat.push_back(e.trade_id);
                }
            }
            if page.len() < 20 {
                break;
            }
            let last_idx = page.len() - 1;
            let last = page.get(last_idx).expect("last entry");
            cursor = Some(last.trade_id);
        }

        assert_eq!(flat.len(), 100);
        for i in 0u32..100 {
            let expected = 100_u64 - i as u64;
            assert_eq!(flat.get(i), Some(expected));
        }
    }

    #[test]
    fn get_trade_history_limit_capped_at_50() {
        let env = Env::default();
        let (_, user, portfolio_id, _) = setup_portfolio(&env, true, 100, 1000);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        for i in 0..60 {
            let id = client.open_position(&user, &100, &(100 + i as i128));
            client.close_position(&user, &id, &0);
        }
        let page = client.get_trade_history(&user, &Option::None, &200);
        assert_eq!(page.len(), 50);
    }
}

#[cfg(test)]
mod badge_tests;
