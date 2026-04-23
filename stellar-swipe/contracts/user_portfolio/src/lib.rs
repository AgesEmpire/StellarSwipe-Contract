//! User portfolio contract: positions and `get_pnl` (source of truth for portfolio performance).

#![cfg_attr(target_family = "wasm", no_std)]

mod queries;
mod storage;
mod subscriptions;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};
use storage::DataKey;

pub use subscriptions::SubscriptionError;

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
    /// One-time setup: admin and oracle (`get_price(asset_pair) -> OraclePrice`) used for unrealized P&L.
    pub fn initialize(env: Env, admin: Address, oracle: Address) {
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage()
            .instance()
            .set(&DataKey::OracleAssetPair, &0u32);
        env.storage()
            .instance()
            .set(&DataKey::NextPositionId, &1u64);
    }

    pub fn set_oracle(env: Env, oracle: Address) {
        Self::require_admin(&env);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
    }

    pub fn set_oracle_asset_pair(env: Env, asset_pair: u32) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::OracleAssetPair, &asset_pair);
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
        env.storage()
            .instance()
            .set(&DataKey::NextPositionId, &next);

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

    /// Closes an open position, records realized P&L, and emits `TradeShareable` for
    /// profitable closes (pnl > 0) so the frontend can generate an X share card.
    #[allow(clippy::too_many_arguments)]
    pub fn close_position(
        env: Env,
        user: Address,
        position_id: u64,
        realized_pnl: i128,
        exit_price: i128,
        asset_pair: u32,
        signal_provider: Address,
        signal_id: u64,
    ) {
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

        // Emit TradeShareable only for profitable closes (pnl > 0).
        if realized_pnl > 0 {
            // pnl_bps = realized_pnl * 10_000 / entry_price (saturate on overflow).
            let pnl_bps: i64 = if pos.entry_price > 0 {
                realized_pnl
                    .checked_mul(10_000)
                    .and_then(|n| n.checked_div(pos.entry_price))
                    .and_then(|v| i64::try_from(v).ok())
                    .unwrap_or(i64::MAX)
            } else {
                0
            };
            env.events().publish(
                (Symbol::new(&env, "TradeShareable"), user.clone()),
                (position_id, asset_pair, pos.entry_price, exit_price, pnl_bps, signal_provider, signal_id),
            );
        }
    }

    /// Portfolio P&L including open positions when oracle price is available.
    pub fn get_pnl(env: Env, user: Address) -> PnlSummary {
        queries::compute_get_pnl(&env, user)
    }

    /// Provider sets per-day fee token + amount for their premium feed (XLM or USDC, etc.).
    pub fn set_provider_subscription_terms(
        env: Env,
        provider: Address,
        fee_token: Address,
        fee_per_day: i128,
    ) -> Result<(), SubscriptionError> {
        subscriptions::set_provider_subscription_terms(&env, &provider, fee_token, fee_per_day)
    }

    /// Pay the provider-configured fee and extend on-chain subscription through `duration_days`.
    pub fn subscribe_to_provider(
        env: Env,
        user: Address,
        provider: Address,
        duration_days: u32,
    ) -> Result<(), SubscriptionError> {
        subscriptions::subscribe_to_provider(&env, &user, &provider, duration_days)
    }

    /// Used by SignalRegistry (cross-contract) to gate PREMIUM signal visibility.
    pub fn check_subscription(env: Env, user: Address, provider: Address) -> bool {
        subscriptions::check_subscription(&env, &user, &provider)
    }

    fn require_admin(env: &Env) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("admin");
        admin.require_auth();
    }
}

#[cfg(test)]
mod oracle_ok {
    use soroban_sdk::{contract, contractimpl, symbol_short, Env};
    use stellar_swipe_common::OraclePrice;

    #[contract]
    pub struct OracleMock;

    #[contractimpl]
    impl OracleMock {
        pub fn set_price(env: Env, asset_pair: u32, price: OraclePrice) {
            env.storage()
                .instance()
                .set(&(symbol_short!("price"), asset_pair), &price);
        }

        pub fn get_price(env: Env, asset_pair: u32) -> OraclePrice {
            env.storage()
                .instance()
                .get(&(symbol_short!("price"), asset_pair))
                .unwrap()
        }
    }
}

#[cfg(test)]
mod oracle_fail {
    use soroban_sdk::{contract, contractimpl, Env};
    use stellar_swipe_common::OraclePrice;

    #[contract]
    pub struct OraclePanic;

    #[contractimpl]
    impl OraclePanic {
        pub fn get_price(_env: Env, _asset_pair: u32) -> OraclePrice {
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
    use soroban_sdk::testutils::{Address as _, Events, Ledger};
    use stellar_swipe_common::OraclePrice;

    #[allow(deprecated)]
    fn setup_portfolio(
        env: &Env,
        use_working_oracle: bool,
        initial_price: i128,
    ) -> (Address, Address, Address) {
        env.ledger().with_mut(|ledger| ledger.timestamp = 1_000);
        let admin = Address::generate(env);
        let user = Address::generate(env);
        let oracle_id = if use_working_oracle {
            let id = env.register_contract(None, OracleMock);
            OracleMockClient::new(env, &id).set_price(
                &7u32,
                &OraclePrice {
                    price: initial_price * 100,
                    decimals: 2,
                    timestamp: env.ledger().timestamp(),
                    source: soroban_sdk::Symbol::new(env, "mock"),
                },
            );
            id
        } else {
            env.register_contract(None, OraclePanic)
        };
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(env, &contract_id);
        env.mock_all_auths();
        client.initialize(&admin, &oracle_id);
        client.set_oracle_asset_pair(&7u32);
        (user, contract_id, oracle_id)
    }

    fn dummy_provider(env: &Env) -> Address {
        Address::generate(env)
    }

    /// All positions closed: unrealized is 0, total = realized, ROI uses invested sums.
    #[test]
    fn get_pnl_all_closed() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        client.open_position(&user, &100, &500);
        client.close_position(&user, &1, &200, &110i128, &1u32, &provider, &0u64);
        client.close_position(&user, &2, &-50, &90i128, &1u32, &provider, &0u64);

        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 150);
        assert_eq!(pnl.unrealized_pnl, Some(0));
        assert_eq!(pnl.total_pnl, 150);
        assert_eq!(pnl.roi_bps, 1000);
    }

    /// Only open positions: realized 0, unrealized from oracle.
    #[test]
    fn get_pnl_all_open() {
        let env = Env::default();
        let (user, portfolio_id, oracle_id) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        client.open_position(&user, &100, &1_000);
        OracleMockClient::new(&env, &oracle_id).set_price(
            &7u32,
            &OraclePrice {
                price: 12000,
                decimals: 2,
                timestamp: env.ledger().timestamp(),
                source: soroban_sdk::Symbol::new(&env, "mock"),
            },
        );

        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 0);
        assert_eq!(pnl.unrealized_pnl, Some(200));
        assert_eq!(pnl.total_pnl, 200);
        assert_eq!(pnl.roi_bps, 2000);
    }

    /// Mixed open + closed.
    #[test]
    fn get_pnl_mixed() {
        let env = Env::default();
        let (user, portfolio_id, oracle_id) = setup_portfolio(&env, true, 50);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &50, &2_000);
        client.open_position(&user, &50, &1_000);
        client.close_position(&user, &1, &300, &60i128, &1u32, &provider, &0u64);

        OracleMockClient::new(&env, &oracle_id).set_price(
            &7u32,
            &OraclePrice {
                price: 6000,
                decimals: 2,
                timestamp: env.ledger().timestamp(),
                source: soroban_sdk::Symbol::new(&env, "mock"),
            },
        );
        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 300);
        assert_eq!(pnl.unrealized_pnl, Some(200));
        assert_eq!(pnl.total_pnl, 500);
        assert_eq!(pnl.roi_bps, 1666);
    }

    /// Oracle fails: partial result, unrealized None, total = realized only.
    #[test]
    fn get_pnl_oracle_unavailable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, false, 0);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        client.close_position(&user, &1, &50, &110i128, &1u32, &provider, &0u64);

        client.open_position(&user, &100, &500);
        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.realized_pnl, 50);
        assert_eq!(pnl.unrealized_pnl, None);
        assert_eq!(pnl.total_pnl, 50);
        assert_eq!(pnl.roi_bps, 333);
    }

    // ── TradeShareable event tests ─────────────────────────────────────────────

    /// Profitable close emits TradeShareable with all required fields.
    #[test]
    fn profitable_close_emits_trade_shareable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        let events_before = env.events().all().len();
        // pnl = 200 > 0 → event must be emitted
        client.close_position(&user, &1, &200, &120i128, &42u32, &provider, &7u64);
        let events_after = env.events().all().len();
        assert!(
            events_after > events_before,
            "TradeShareable event not emitted for profitable close"
        );
    }

    /// Loss close must NOT emit TradeShareable.
    #[test]
    fn loss_close_does_not_emit_trade_shareable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        let events_before = env.events().all().len();
        // pnl = -50 (loss) → no new event
        client.close_position(&user, &1, &-50, &90i128, &42u32, &provider, &7u64);
        let events_after = env.events().all().len();
        assert_eq!(
            events_after, events_before,
            "TradeShareable must not be emitted for a loss"
        );
    }

    /// Breakeven close (pnl == 0) must NOT emit TradeShareable.
    #[test]
    fn breakeven_close_does_not_emit_trade_shareable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        let events_before = env.events().all().len();
        // pnl = 0 (breakeven) → no new event
        client.close_position(&user, &1, &0, &100i128, &42u32, &provider, &7u64);
        let events_after = env.events().all().len();
        assert_eq!(
            events_after, events_before,
            "TradeShareable must not be emitted for breakeven"
        );
    }

    // ── Overflow / division-by-zero tests ─────────────────────────────────────

    /// roi_basis_points: total_invested == 0 → returns 0 (no division-by-zero).
    #[test]
    fn get_pnl_zero_invested_returns_zero_roi() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);

        // No positions opened → total_invested == 0
        let pnl = client.get_pnl(&user);
        assert_eq!(pnl.roi_bps, 0);
        assert_eq!(pnl.total_pnl, 0);
    }

    /// roi_basis_points: total_pnl * 10_000 overflows i128 → saturates to 0 (checked_mul returns None).
    #[test]
    fn get_pnl_roi_overflow_saturates_to_zero() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        // Open and close with realized_pnl = i128::MAX to trigger overflow in checked_mul(10_000)
        client.open_position(&user, &1, &1);
        client.close_position(&user, &1, &i128::MAX, &2i128, &1u32, &provider, &0u64);

        let pnl = client.get_pnl(&user);
        // checked_mul overflows → roi_basis_points returns 0
        assert_eq!(pnl.roi_bps, 0);
    }
}
