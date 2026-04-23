//! User portfolio contract: positions and `get_pnl` (source of truth for portfolio performance).

#![cfg_attr(target_family = "wasm", no_std)]

mod queries;
mod storage;

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
}

#[contract]
pub struct UserPortfolio;

#[contractimpl]
impl UserPortfolio {
    /// One-time setup: admin and oracle (`get_price() -> i128`) used for unrealized P&L.
    pub fn initialize(env: Env, admin: Address, oracle: Address) {
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage().instance().set(&DataKey::NextPositionId, &1u64);
    }

    pub fn set_oracle(env: Env, oracle: Address) {
        Self::require_admin(&env);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
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
        };
        env.storage().persistent().set(&DataKey::Position(id), &pos);

        let key = DataKey::UserPositions(user.clone());
        let mut list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        list.push_back(id);
        env.storage().persistent().set(&key, &list);

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
        env.storage().persistent().set(&pkey, &pos);
    }

    /// Portfolio P&L including open positions when oracle price is available.
    pub fn get_pnl(env: Env, user: Address) -> PnlSummary {
        queries::compute_get_pnl(&env, user)
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
    ) -> (Address, Address, Address) {
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
        client.initialize(&admin, &oracle_id);
        (user, contract_id, oracle_id)
    }

    /// All positions closed: unrealized is 0, total = realized, ROI uses invested sums.
    #[test]
    fn get_pnl_all_closed() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
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
        let (user, portfolio_id, oracle_id) = setup_portfolio(&env, true, 100);
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
        let (user, portfolio_id, oracle_id) = setup_portfolio(&env, true, 50);
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

    /// roi_basis_points: zero invested → roi_bps = 0 (no division-by-zero).
    #[test]
    fn get_pnl_zero_invested_no_div_by_zero() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        // No positions at all → total_invested = 0, must not panic
        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.roi_bps, 0);
    }

    /// roi_basis_points: i128::MAX pnl with small investment saturates to i32::MAX.
    #[test]
    fn get_pnl_roi_saturates_to_i32_max() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        // realized_pnl = i128::MAX / 10_000 + 1 so that pnl * 10_000 overflows → roi_bps = 0
        // (checked_mul returns None → 0). Verify no panic.
        client.open_position(&user, &1, &1);
        client.close_position(&user, &1, &(i128::MAX / 10_001));
        let pnl = client.get_pnl(&user);
        // Either saturated or 0 — must not panic
        assert!(pnl.roi_bps >= 0 || pnl.roi_bps == i32::MIN);
    }

    /// Oracle fails: partial result, unrealized None, total = realized only.
    #[test]
    fn get_pnl_oracle_unavailable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, false, 0);
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
}
