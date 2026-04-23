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

    /// Admin: register the TradeExecutor contract that is allowed to call
    /// `close_position_keeper` on behalf of keepers.
    pub fn set_trade_executor(env: Env, trade_executor: Address) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::TradeExecutor, &trade_executor);
    }

    pub fn get_trade_executor(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::TradeExecutor)
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
            shared::events::emit_trade_shareable(
                env,
                shared::events::EvtTradeShareable {
                    user: user.clone(),
                    position_id,
                    asset_pair,
                    entry_price: pos.entry_price,
                    exit_price,
                    pnl_bps,
                    signal_provider,
                    signal_id,
                },
            );
        }
    }

    /// Keeper-callable position close: used by TradeExecutor for stop-loss / take-profit
    /// triggers. Does NOT require user signature; instead verifies that the caller is
    /// the registered TradeExecutor contract.
    ///
    /// ## Auth model
    /// - `caller` must be the registered TradeExecutor address (set by admin via
    ///   `set_trade_executor`). The caller must sign the transaction (i.e. the
    ///   TradeExecutor contract itself is the transaction source or sub-invocation
    ///   authoriser).
    /// - No user signature required (keeper pattern).
    ///
    /// ## Parameters
    /// - `caller`: must equal the registered TradeExecutor
    /// - `user`: position owner
    /// - `position_id`: position to close
    /// - `asset_pair`: asset pair for event emission (informational)
    ///
    /// Realized P&L is set to 0 (keeper closes do not calculate P&L; that is done
    /// off-chain or in a separate settlement step).
    pub fn close_position_keeper(
        env: Env,
        caller: Address,
        user: Address,
        position_id: u64,
        asset_pair: u32,
    ) {
        // Require the caller to authorise this call.
        caller.require_auth();

        // Verify caller is the registered TradeExecutor.
        let trade_executor: Address = env
            .storage()
            .instance()
            .get(&DataKey::TradeExecutor)
            .expect("trade executor not set");
        if caller != trade_executor {
            panic!("unauthorized: only trade executor can call close_position_keeper");
        }

        // Verify position exists and belongs to user.
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

        // Close position with zero P&L (keeper closes don't calculate P&L).
        pos.status = PositionStatus::Closed;
        pos.realized_pnl = 0;
        env.storage().persistent().set(&pkey, &pos);

        // Emit event for keeper close (no TradeShareable since pnl=0).
        shared::events::emit_position_closed_by_keeper(
            env,
            shared::events::EvtPositionClosedByKeeper {
                user: user.clone(),
                position_id,
                asset_pair,
            },
        );
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
    use soroban_sdk::testutils::{Address as _, Ledger};
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

        OracleMockClient::new(&env, &oracle_id).set_price(&60);
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
        // pnl = 200 > 0 → event must be emitted
        client.close_position(&user, &1, &200, &120i128, &42u32, &provider, &7u64);

        let found = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            topics
                .get(0)
                .and_then(|v| soroban_sdk::Symbol::try_from(v).ok())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "TradeShareable"))
                .unwrap_or(false)
        });
        assert!(found, "TradeShareable event not emitted for profitable close");
    }

    /// Loss close must NOT emit TradeShareable.
    #[test]
    fn loss_close_does_not_emit_trade_shareable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        // pnl = -50 (loss) → no event
        client.close_position(&user, &1, &-50, &90i128, &42u32, &provider, &7u64);

        let found = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            topics
                .get(0)
                .and_then(|v| soroban_sdk::Symbol::try_from(v).ok())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "TradeShareable"))
                .unwrap_or(false)
        });
        assert!(!found, "TradeShareable must not be emitted for a loss");
    }

    /// Breakeven close (pnl == 0) must NOT emit TradeShareable.
    #[test]
    fn breakeven_close_does_not_emit_trade_shareable() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        // pnl = 0 (breakeven) → no event
        client.close_position(&user, &1, &0, &100i128, &42u32, &provider, &7u64);

        let found = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            topics
                .get(0)
                .and_then(|v| soroban_sdk::Symbol::try_from(v).ok())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "TradeShareable"))
                .unwrap_or(false)
        });
        assert!(!found, "TradeShareable must not be emitted for breakeven");
    }

    // ── Event format tests ────────────────────────────────────────────────────

    fn last_topics(env: &Env) -> (soroban_sdk::Symbol, soroban_sdk::Symbol) {
        use soroban_sdk::testutils::Events;
        let events = env.events().all();
        let e = events.last().unwrap();
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1;
        let t0 = soroban_sdk::Symbol::try_from(topics.get(0).unwrap()).unwrap();
        let t1 = soroban_sdk::Symbol::try_from(topics.get(1).unwrap()).unwrap();
        (t0, t1)
    }

    #[test]
    fn trade_shareable_event_has_two_topic_format() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);
        client.open_position(&user, &100, &1_000);
        client.close_position(&user, &1, &200, &120i128, &42u32, &provider, &7u64);
        let (contract, event) = last_topics(&env);
        assert_eq!(contract, soroban_sdk::Symbol::new(&env, "user_portfolio"));
        assert_eq!(event, soroban_sdk::Symbol::new(&env, "trade_shareable"));
    }

    #[test]
    fn subscription_created_event_has_two_topic_format() {
        use soroban_sdk::testutils::Ledger;
        use soroban_sdk::token::StellarAssetClient;
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = 1_000);
        let admin = Address::generate(&env);
        let provider = Address::generate(&env);
        let subscriber = Address::generate(&env);
        let oracle = Address::generate(&env);
        let portfolio_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        client.initialize(&admin, &oracle);
        let token = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        StellarAssetClient::new(&env, &token).mint(&subscriber, &1_000_000i128);
        client.set_provider_subscription_terms(&provider, &token, &10_000i128);
        client.subscribe_to_provider(&subscriber, &provider, &7u32);
        let (contract, event) = last_topics(&env);
        assert_eq!(contract, soroban_sdk::Symbol::new(&env, "user_portfolio"));
        assert_eq!(event, soroban_sdk::Symbol::new(&env, "subscription_created"));
    }
}
