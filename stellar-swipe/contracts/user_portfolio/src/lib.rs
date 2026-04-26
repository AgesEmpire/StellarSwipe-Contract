//! User portfolio contract: positions and `get_pnl` (source of truth for portfolio performance).

#![cfg_attr(target_family = "wasm", no_std)]

mod queries;
mod storage;
mod subscriptions;
#[cfg(test)]
#[path = "tests/mod.rs"]
mod portfolio_tests;

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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeHistoryEntry {
    pub trade_id: u64,
    pub position: Position,
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

    /// Admin: set or clear the KYC-verified flag for a user.
    /// No PII is stored — only a boolean.
    /// Emits `KYCStatusUpdated { user, verified }`.
    pub fn set_kyc_status(env: Env, user: Address, verified: bool) {
        Self::require_admin(&env);
        env.storage()
            .persistent()
            .set(&DataKey::KycVerified(user.clone()), &verified);
        shared::events::emit_kyc_status_updated(
            &env,
            shared::events::EvtKycStatusUpdated {
                schema_version: shared::events::SCHEMA_VERSION,
                user,
                verified,
            },
        );
    }

    /// Returns the KYC-verified status for a user (defaults to false if never set).
    pub fn is_kyc_verified(env: Env, user: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::KycVerified(user))
            .unwrap_or(false)
    }

    /// Admin: enable or disable KYC-required mode.
    /// When true, only KYC-verified users can open positions.
    pub fn set_kyc_required_mode(env: Env, required: bool) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::KycRequiredMode, &required);
    }

    /// Returns whether KYC-required mode is active (defaults to false).
    pub fn get_kyc_required_mode(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::KycRequiredMode)
            .unwrap_or(false)
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
        // KYC gate: if KYC-required mode is active, only verified users may open positions.
        let kyc_required: bool = env
            .storage()
            .instance()
            .get(&DataKey::KycRequiredMode)
            .unwrap_or(false);
        if kyc_required {
            let verified: bool = env
                .storage()
                .persistent()
                .get(&DataKey::KycVerified(user.clone()))
                .unwrap_or(false);
            if !verified {
                panic!("KYC verification required to open a position");
            }
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
                &env,
                shared::events::EvtTradeShareable {
                    schema_version: shared::events::SCHEMA_VERSION,
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

        shared::events::emit_position_closed(
            &env,
            shared::events::EvtPositionClosed {
                schema_version: shared::events::SCHEMA_VERSION,
                user,
                trade_id: position_id,
                exit_price,
                realized_pnl,
                timestamp: env.ledger().timestamp(),
                action_required: false,
            },
        );
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
            &env,
            shared::events::EvtPositionClosedByKeeper {
                schema_version: shared::events::SCHEMA_VERSION,
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

// ── KYC unit tests ────────────────────────────────────────────────────────────
#[cfg(test)]
mod kyc_tests {
    use super::oracle_ok::OracleMock;
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let oracle = env.register_contract(None, OracleMock);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(env, &contract_id);
        client.initialize(&admin, &oracle);
        (admin, contract_id)
    }

    // KYC flag is readable and defaults to false.
    #[test]
    fn kyc_flag_defaults_to_false() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        assert!(!client.is_kyc_verified(&user));
    }

    // Admin can set and clear the KYC flag.
    #[test]
    fn admin_can_set_and_clear_kyc_flag() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_kyc_status(&user, &true);
        assert!(client.is_kyc_verified(&user));

        client.set_kyc_status(&user, &false);
        assert!(!client.is_kyc_verified(&user));
    }

    // set_kyc_status emits KYCStatusUpdated event.
    #[test]
    fn set_kyc_status_emits_event() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_kyc_status(&user, &true);

        let has_kyc_event = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "kyc_status_updated"))
                .unwrap_or(false)
        });
        assert!(has_kyc_event, "kyc_status_updated event not emitted");
    }

    // KYC-required mode defaults to false.
    #[test]
    fn kyc_required_mode_defaults_to_false() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        assert!(!client.get_kyc_required_mode());
    }

    // Non-required mode: unverified user can open a position.
    #[test]
    fn non_required_mode_allows_unverified_user() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        // KYC-required mode is off by default — unverified user should succeed.
        let id = client.open_position(&user, &100, &1_000);
        assert_eq!(id, 1);
    }

    // KYC-required mode: verified user can open a position.
    #[test]
    fn required_mode_allows_verified_user() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_kyc_required_mode(&true);
        client.set_kyc_status(&user, &true);

        let id = client.open_position(&user, &100, &1_000);
        assert_eq!(id, 1);
    }

    // KYC-required mode: unverified user cannot open a position.
    #[test]
    #[should_panic(expected = "KYC verification required to open a position")]
    fn required_mode_blocks_unverified_user() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_kyc_required_mode(&true);
        // user is not KYC-verified — should panic
        client.open_position(&user, &100, &1_000);
    }

    // Disabling KYC-required mode re-allows unverified users.
    #[test]
    fn disabling_required_mode_allows_unverified_user() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_kyc_required_mode(&true);
        client.set_kyc_required_mode(&false);

        // Mode is now off — unverified user should succeed.
        let id = client.open_position(&user, &100, &1_000);
        assert_eq!(id, 1);
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
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        // pnl = -50 (loss) → TradeShareable must NOT be emitted
        client.close_position(&user, &1, &-50, &90i128, &42u32, &provider, &7u64);
        let has_trade_shareable = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 { return false; }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "trade_shareable"))
                .unwrap_or(false)
        });
        assert!(!has_trade_shareable, "TradeShareable must not be emitted for a loss");
    }

    /// Breakeven close (pnl == 0) must NOT emit TradeShareable.
    #[test]
    fn breakeven_close_does_not_emit_trade_shareable() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        // pnl = 0 (breakeven) → TradeShareable must NOT be emitted
        client.close_position(&user, &1, &0, &100i128, &42u32, &provider, &7u64);
        let has_trade_shareable = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 { return false; }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "trade_shareable"))
                .unwrap_or(false)
        });
        assert!(!has_trade_shareable, "TradeShareable must not be emitted for breakeven");
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

    // ── Event format tests ────────────────────────────────────────────────────

    fn last_topics(env: &Env) -> (soroban_sdk::Symbol, soroban_sdk::Symbol) {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let events = env.events().all();
        let e = events.last().unwrap();
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1;
        let t0 = soroban_sdk::Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1 = soroban_sdk::Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        (t0, t1)
    }

    #[test]
    fn trade_shareable_event_has_two_topic_format() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);
        client.open_position(&user, &100, &1_000);
        client.close_position(&user, &1, &200, &120i128, &42u32, &provider, &7u64);
        // Find the trade_shareable event specifically (position_closed is emitted after it).
        let shareable_evt = env.events().all().iter().find(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 { return false; }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "trade_shareable"))
                .unwrap_or(false)
        });
        assert!(shareable_evt.is_some(), "trade_shareable event not found");
        let topics: soroban_sdk::Vec<soroban_sdk::Val> = shareable_evt.unwrap().1.clone();
        let t0 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, soroban_sdk::Symbol::new(&env, "user_portfolio"));
        assert_eq!(t1, soroban_sdk::Symbol::new(&env, "trade_shareable"));
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
