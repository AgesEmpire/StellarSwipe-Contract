//! User portfolio contract: positions and `get_pnl` (source of truth for portfolio performance).

#![cfg_attr(target_family = "wasm", no_std)]

mod migration;
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

    /// Register users for migration. Must be called before `migrate_portfolio_v1_to_v2`.
    pub fn register_migration_users(env: Env, users: Vec<Address>) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::MigrationQueue, &users);
    }

    /// Migrate up to `batch_size` users from V1 → V2 layout. Admin-only.
    /// Open positions are migrated first (highest priority), then closed.
    /// Emits `PortfolioMigrationComplete` per user.
    pub fn migrate_portfolio_v1_to_v2(env: Env, batch_size: u32) -> u32 {
        Self::require_admin(&env);
        migration::migrate_batch(&env, batch_size)
    }

    fn require_admin(env: &Env) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).expect("admin");
        admin.require_auth();
    }
}

#[cfg(test)]
mod migration_tests {
    use super::oracle_ok::OracleMock;
    use super::oracle_ok::OracleMockClient;
    use super::*;
    use crate::storage::DataKey;
    use soroban_sdk::testutils::Address as _;

    /// 20 users × 5 open + 10 closed positions each.
    /// Verifies all positions are preserved after V1 → V2 migration.
    #[test]
    fn migrate_20_users_5_open_10_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle_id = env.register_contract(None, OracleMock);
        OracleMockClient::new(&env, &oracle_id).set_price(&100);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle_id);

        const USERS: usize = 20;
        const OPEN: usize = 5;
        const CLOSED: usize = 10;

        let mut users: Vec<Address> = Vec::new(&env);
        for _ in 0..USERS {
            let user = Address::generate(&env);

            // Open OPEN + CLOSED positions (all start open).
            let mut all_ids = soroban_sdk::vec![&env];
            for _ in 0..(OPEN + CLOSED) {
                let id = client.open_position(&user, &100, &1_000);
                all_ids.push_back(id);
            }
            // Close the last CLOSED of them.
            for i in OPEN..(OPEN + CLOSED) {
                let id = all_ids.get(i as u32).unwrap();
                client.close_position(&user, &id, &50);
            }

            users.push_back(user);
        }

        // Register all users for migration and run in one batch.
        client.register_migration_users(&users);
        let processed = client.migrate_portfolio_v1_to_v2(&(USERS as u32));
        assert_eq!(processed, USERS as u32);

        // Verify V2 storage for every user.
        for i in 0..USERS {
            let user = users.get(i as u32).unwrap();

            let open_ids: soroban_sdk::Vec<u64> = env
                .as_contract(&contract_id, || {
                    env.storage()
                        .persistent()
                        .get(&DataKey::UserOpenPositions(user.clone()))
                        .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
                });
            let closed_ids: soroban_sdk::Vec<u64> = env
                .as_contract(&contract_id, || {
                    env.storage()
                        .persistent()
                        .get(&DataKey::UserClosedPositions(user.clone()))
                        .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
                });

            assert_eq!(open_ids.len(), OPEN as u32, "user {i}: open count mismatch");
            assert_eq!(
                closed_ids.len(),
                CLOSED as u32,
                "user {i}: closed count mismatch"
            );

            // Verify every open position is actually Open.
            for j in 0..open_ids.len() {
                let id = open_ids.get(j).unwrap();
                let pos: Position = env.as_contract(&contract_id, || {
                    env.storage()
                        .persistent()
                        .get(&DataKey::Position(id))
                        .expect("open position missing")
                });
                assert_eq!(pos.status, PositionStatus::Open);
            }

            // Verify every closed position is actually Closed.
            for j in 0..closed_ids.len() {
                let id = closed_ids.get(j).unwrap();
                let pos: Position = env.as_contract(&contract_id, || {
                    env.storage()
                        .persistent()
                        .get(&DataKey::Position(id))
                        .expect("closed position missing")
                });
                assert_eq!(pos.status, PositionStatus::Closed);
            }
        }
    }

    /// Idempotency: running migration twice on the same users is safe.
    #[test]
    fn migrate_idempotent() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle_id = env.register_contract(None, OracleMock);
        OracleMockClient::new(&env, &oracle_id).set_price(&100);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle_id);

        let user = Address::generate(&env);
        client.open_position(&user, &100, &1_000);
        client.close_position(&user, &1, &50);

        let mut users: Vec<Address> = Vec::new(&env);
        users.push_back(user.clone());

        client.register_migration_users(&users);
        client.migrate_portfolio_v1_to_v2(&1);

        // Second run: queue is empty, nothing to process.
        client.register_migration_users(&users);
        // Re-registering same user; migrate_user skips already-migrated.
        let processed = client.migrate_portfolio_v1_to_v2(&1);
        assert_eq!(processed, 1); // processed from queue but skipped internally

        let open_ids: soroban_sdk::Vec<u64> = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::UserOpenPositions(user.clone()))
                .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
        });
        assert_eq!(open_ids.len(), 0); // position 1 was closed
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
