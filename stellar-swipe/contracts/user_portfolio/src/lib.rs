//! User portfolio contract: positions and `get_pnl` (source of truth for portfolio performance).

#![no_std]

mod achievements;
mod badges;
mod exposure_cap;
mod migration;
mod onboarding;
#[cfg(test)]
#[path = "tests/mod.rs"]
mod portfolio_tests;
mod position_tags;
mod preferences;
mod queries;
mod risk_metrics;
mod storage;
mod subscriptions;
mod watchlist;

pub use achievements::{Achievement, AchievementType};
pub use badges::{Badge, BadgeType};
pub use onboarding::OnboardingStatus;
pub use preferences::{
    HoldDuration, NotificationPrefs, RiskRating, SignalAction, SignalCategory, SignalSummary,
    TradingStyle,
};

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, String, Symbol, Vec,
};
use stellar_swipe_common::health::{health_uninitialized, HealthStatus};
use storage::DataKey;

/// Compute the Herfindahl-Hirschman Index (HHI) concentration score for a user's open
/// positions.  Returned value is in basis points (0 = perfectly diversified, 10 000 = 100 %
/// concentration in a single position).
///
/// Formula:  HHI = Σ ( s_i² )  where  s_i = amount_i / total_amount
/// Scaled to basis points:  HHI_bps = Σ ( weight_i_bps² / 10 000 )
fn compute_concentration_score(env: &Env, user: &Address) -> u32 {
    let open_ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::UserOpenPositions(user.clone()))
        .unwrap_or_else(|| Vec::new(env));

    if open_ids.is_empty() {
        return 0;
    }

    let mut total: i128 = 0;
    let mut weights: Vec<i128> = Vec::new(env);

    for i in 0..open_ids.len() {
        let Some(id) = open_ids.get(i) else { continue };
        let Some(pos) = env
            .storage()
            .persistent()
            .get::<DataKey, Position>(&DataKey::Position(id))
        else {
            continue;
        };
        if pos.status != PositionStatus::Open || pos.amount <= 0 {
            continue;
        }
        total = total.saturating_add(pos.amount);
        weights.push_back(pos.amount);
    }

    if total == 0 {
        return 0;
    }

    let mut hhi: u64 = 0;
    for i in 0..weights.len() {
        let amount = weights.get(i).unwrap_or(0);
        if amount <= 0 {
            continue;
        }
        let weight_bps: i128 = amount
            .checked_mul(10_000)
            .map(|n| n / total)
            .unwrap_or(10_000)
            .min(10_000);
        let wb = weight_bps as u64;
        hhi = hhi.saturating_add(wb.saturating_mul(wb) / 10_000);
    }

    (hhi as u32).min(10_000)
}

pub use subscriptions::SubscriptionError;

/// A point-in-time record of a user's total portfolio value (issue #685).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioSnapshot {
    /// Ledger timestamp when the snapshot was taken.
    pub timestamp: u64,
    /// Total portfolio value at snapshot time (realized_pnl + unrealized_pnl).
    /// When the oracle is unavailable unrealized P&L is excluded; `total_pnl`
    /// from `get_pnl()` is used directly as the snapshot value.
    pub total_value: i128,
}

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnchorDepositInfo {
    pub deposit_address: Address,
    pub token: Address,
    pub amount_fiat: i128,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionStatus {
    Open = 0,
    Closed = 1,
    /// Transitional state: a close operation is in progress.
    /// Used as a lock to prevent concurrent double-close (race condition between
    /// user-initiated close and keeper stop-loss trigger).
    Closing = 2,
}

/// Error returned when a close is attempted on a position that is already
/// `Closing` or `Closed`.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionError {
    PositionAlreadyClosed = 1,
}

/// Errors for portfolio-level operations (Issue #752 and future extensions).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PortfolioError {
    /// The trade would push the user's exposure to a capped asset beyond their limit.
    ExposureCapExceeded = 1,
    /// Cap amount must be > 0.
    InvalidCapAmount = 2,
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

/// A realized tax event recorded whenever a position is closed (Issue #658).
/// Suitable for CSV export by downstream tax reporting tools.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaxEvent {
    pub position_id: u64,
    /// Unix timestamp (seconds) when the position was opened.
    pub open_ts: u64,
    /// Unix timestamp (seconds) when the position was closed.
    pub close_ts: u64,
    pub entry_price: i128,
    /// Exit price at close time; 0 for keeper-triggered closes where the price is unknown.
    pub exit_price: i128,
    pub amount: i128,
    pub realized_pnl: i128,
    pub asset_pair: u32,
}

/// A single cost lot used for FIFO P&L tracking.
/// Lots are enqueued via `add_cost_lot` and consumed front-first by `close_fifo`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostLot {
    pub entry_price: i128,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeHistoryEntry {
    pub trade_id: u64,
    pub position: Position,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioPosition {
    pub position_id: u64,
    pub position: Position,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Portfolio {
    pub open_positions: Vec<PortfolioPosition>,
    pub closed_positions: Vec<PortfolioPosition>,
    pub closed_position_ids: Vec<u64>,
}

soroban_sdk::contractmeta!(key = "SourceHash", val = env!("STELLAR_SOURCE_HASH"));
soroban_sdk::contractmeta!(key = "GitCommit", val = env!("STELLAR_GIT_COMMIT"));

#[contract]
pub struct UserPortfolio;

#[contractimpl]
impl UserPortfolio {
    pub fn get_build_info(env: Env) -> soroban_sdk::Map<soroban_sdk::String, soroban_sdk::String> {
        let mut m = soroban_sdk::Map::new(&env);
        m.set(
            soroban_sdk::String::from_str(&env, "version"),
            soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION")),
        );
        m.set(
            soroban_sdk::String::from_str(&env, "source_hash"),
            soroban_sdk::String::from_str(&env, env!("STELLAR_SOURCE_HASH")),
        );
        m.set(
            soroban_sdk::String::from_str(&env, "git_commit"),
            soroban_sdk::String::from_str(&env, env!("STELLAR_GIT_COMMIT")),
        );
        m
    }

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

    /// Admin: configure an anchor deposit address for a specific token.
    pub fn set_anchor_deposit_address(env: Env, token: Address, deposit_address: Address) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::AnchorDepositAddress(token), &deposit_address);
    }

    /// Query an anchor deposit address for a token and requested fiat amount.
    pub fn get_anchor_deposit_address(
        env: Env,
        _user: Address,
        token: Address,
        amount_fiat: i128,
    ) -> AnchorDepositInfo {
        let deposit_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::AnchorDepositAddress(token.clone()))
            .expect("anchor deposit address not configured for token");

        AnchorDepositInfo {
            deposit_address,
            token,
            amount_fiat,
        }
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

    /// Admin: set or clear the geographic restriction flag for a user.
    /// `reason_hash` is an IPFS CID of the reason document — no reason text stored on-chain.
    /// Emits `UserRestricted { user, reason_hash, restricted }`.
    pub fn set_user_restriction(
        env: Env,
        user: Address,
        restricted: bool,
        reason_hash: soroban_sdk::String,
    ) {
        Self::require_admin(&env);
        env.storage()
            .persistent()
            .set(&DataKey::Restricted(user.clone()), &restricted);
        shared::events::emit_user_restricted(
            &env,
            shared::events::EvtUserRestricted {
                schema_version: shared::events::SCHEMA_VERSION,
                user,
                reason_hash,
                restricted,
            },
        );
    }

    /// Returns whether a user is geographically restricted (defaults to false).
    pub fn is_restricted(env: Env, user: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Restricted(user))
            .unwrap_or(false)
    }

    pub fn set_oracle_asset_pair(env: Env, asset_pair: u32) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::OracleAssetPair, &asset_pair);
    }

    pub fn get_onboarding_status(env: Env, user: Address) -> OnboardingStatus {
        env.storage()
            .persistent()
            .get(&DataKey::UserOnboardingStatus(user))
            .unwrap_or(OnboardingStatus::NotStarted)
    }

    pub fn get_onboarding_milestone(env: Env, user: Address) -> Option<String> {
        env.storage()
            .persistent()
            .get(&DataKey::UserOnboardingMilestone(user))
    }

    pub fn update_onboarding_status(
        env: Env,
        user: Address,
        status: OnboardingStatus,
        milestone: Option<String>,
    ) {
        Self::require_admin(&env);
        env.storage()
            .persistent()
            .set(&DataKey::UserOnboardingStatus(user.clone()), &status);
        if let Some(m) = milestone.clone() {
            env.storage()
                .persistent()
                .set(&DataKey::UserOnboardingMilestone(user.clone()), &m);
        }
        onboarding::emit_onboarding_status_updated(&env, user, status, milestone);
    }

    /// Admin: enqueue users for V1 → V2 portfolio layout migration.
    pub fn register_migration_users(env: Env, users: Vec<Address>) {
        Self::require_admin(&env);
        migration::register_migration_users(&env, &users);
    }

    /// Admin: migrate up to `batch_size` queued users from V1 to V2 layout.
    pub fn migrate_portfolio_v1_to_v2(env: Env, batch_size: u32) -> u32 {
        Self::require_admin(&env);
        migration::migrate_batch(&env, batch_size)
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
        // Restriction gate: restricted users cannot open positions.
        let restricted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Restricted(user.clone()))
            .unwrap_or(false);
        if restricted {
            panic!("user is geographically restricted");
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

        // Record open timestamp for tax event log (Issue #658).
        env.storage()
            .persistent()
            .set(&DataKey::PositionOpenedAt(id), &env.ledger().timestamp());

        let key = DataKey::UserPositions(user.clone());
        let mut list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        list.push_back(id);
        env.storage().persistent().set(&key, &list);

        let open_key = DataKey::UserOpenPositions(user.clone());
        let mut open_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&open_key)
            .unwrap_or_else(|| Vec::new(&env));
        open_ids.push_back(id);
        env.storage().persistent().set(&open_key, &open_ids);

        // Check concentration threshold and emit warning if exceeded (Issue #684).
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ConcentrationThreshold)
            .unwrap_or(5_000u32);
        let score = compute_concentration_score(&env, &user);
        if score >= threshold {
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(&env, "user_portfolio"),
                    Symbol::new(&env, "high_concentration_warning"),
                ),
                (user, score, threshold),
            );
        }

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
    ) -> Result<(), PositionError> {
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

        // State machine: only Open positions can be closed.
        // Closing or Closed → return error (prevents double-close race condition).
        if pos.status != PositionStatus::Open {
            return Err(PositionError::PositionAlreadyClosed);
        }

        // Acquire the CLOSING lock before any further work.
        pos.status = PositionStatus::Closing;
        env.storage().persistent().set(&pkey, &pos);

        // Finalize: mark as Closed with realized P&L.
        pos.status = PositionStatus::Closed;
        pos.realized_pnl = realized_pnl;
        env.storage().persistent().set(&pkey, &pos);
        Self::mark_position_closed(&env, &user, position_id);

        // Append tax event for reporting (Issue #658).
        {
            let open_ts: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::PositionOpenedAt(position_id))
                .unwrap_or(0);
            let tax_event = TaxEvent {
                position_id,
                open_ts,
                close_ts: env.ledger().timestamp(),
                entry_price: pos.entry_price,
                exit_price,
                amount: pos.amount,
                realized_pnl,
                asset_pair,
            };
            let tax_key = DataKey::UserTaxEvents(user.clone());
            let mut tax_events: Vec<TaxEvent> = env
                .storage()
                .persistent()
                .get(&tax_key)
                .unwrap_or_else(|| Vec::new(&env));
            tax_events.push_back(tax_event);
            env.storage().persistent().set(&tax_key, &tax_events);
        }

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
                user: user.clone(),
                trade_id: position_id,
                exit_price,
                realized_pnl,
                timestamp: env.ledger().timestamp(),
                action_required: false,
            },
        );

        // Update streaks: increment on profitable close, reset on loss/zero.
        let current_key = DataKey::CurrentStreak(user.clone());
        let best_key = DataKey::BestStreak(user.clone());

        let mut current: u32 = env.storage().persistent().get(&current_key).unwrap_or(0u32);
        let mut best: u32 = env.storage().persistent().get(&best_key).unwrap_or(0u32);

        if realized_pnl > 0 {
            // profitable close: increment streak
            current = current.saturating_add(1);
            if current > best {
                best = current;
                env.storage().persistent().set(&best_key, &best);
            }
            env.storage().persistent().set(&current_key, &current);

            shared::events::emit_streak_updated(
                &env,
                shared::events::EvtStreakUpdated {
                    schema_version: shared::events::SCHEMA_VERSION,
                    user: user.clone(),
                    current_streak: current,
                    best_streak: best,
                },
            );
        } else {
            // loss or zero: if there was a streak, emit StreakBroken
            if current > 0 {
                shared::events::emit_streak_broken(
                    &env,
                    shared::events::EvtStreakBroken {
                        schema_version: shared::events::SCHEMA_VERSION,
                        user: user.clone(),
                        streak_length: current,
                    },
                );
            }
            current = 0;
            env.storage().persistent().set(&current_key, &current);
            shared::events::emit_streak_updated(
                &env,
                shared::events::EvtStreakUpdated {
                    schema_version: shared::events::SCHEMA_VERSION,
                    user: user.clone(),
                    current_streak: current,
                    best_streak: best,
                },
            );
        }

        Ok(())
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
    ) -> Result<(), PositionError> {
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

        // State machine: only Open positions can be closed.
        if pos.status != PositionStatus::Open {
            return Err(PositionError::PositionAlreadyClosed);
        }

        // Acquire the CLOSING lock.
        pos.status = PositionStatus::Closing;
        env.storage().persistent().set(&pkey, &pos);

        // Close position with zero P&L (keeper closes don't calculate P&L).
        pos.status = PositionStatus::Closed;
        pos.realized_pnl = 0;
        env.storage().persistent().set(&pkey, &pos);
        Self::mark_position_closed(&env, &user, position_id);

        // Append tax event for reporting (Issue #658). exit_price is 0 — keeper
        // closes don't supply a price; downstream tools should treat 0 as unknown.
        {
            let open_ts: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::PositionOpenedAt(position_id))
                .unwrap_or(0);
            let tax_event = TaxEvent {
                position_id,
                open_ts,
                close_ts: env.ledger().timestamp(),
                entry_price: pos.entry_price,
                exit_price: 0,
                amount: pos.amount,
                realized_pnl: 0,
                asset_pair,
            };
            let tax_key = DataKey::UserTaxEvents(user.clone());
            let mut tax_events: Vec<TaxEvent> = env
                .storage()
                .persistent()
                .get(&tax_key)
                .unwrap_or_else(|| Vec::new(&env));
            tax_events.push_back(tax_event);
            env.storage().persistent().set(&tax_key, &tax_events);
        }

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

        Ok(())
    }

    /// Portfolio P&L including open positions when oracle price is available.
    pub fn get_pnl(env: Env, user: Address) -> PnlSummary {
        queries::compute_get_pnl(&env, user)
    }

    /// Portfolio snapshot. Closed positions are optional because active traders can
    /// have enough history to make full closed-position loading expensive.
    pub fn get_portfolio(env: Env, user: Address, include_closed: bool) -> Portfolio {
        queries::get_portfolio(&env, user, include_closed)
    }

    pub fn get_trade_history(
        env: Env,
        user: Address,
        cursor: Option<u64>,
        limit: u32,
    ) -> Vec<TradeHistoryEntry> {
        queries::get_trade_history(&env, user, cursor, limit)
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

    // ── Issue #430: Notification Preferences ─────────────────────────────────

    /// Store notification preferences for `user`. Caller must be `user`.
    pub fn set_notification_preferences(env: Env, user: Address, prefs: NotificationPrefs) {
        preferences::set_notification_preferences(&env, &user, prefs);
    }

    /// Retrieve notification preferences for `user`.
    /// Returns default (all enabled) if never set.
    pub fn get_notification_preferences(env: Env, user: Address) -> NotificationPrefs {
        preferences::get_notification_preferences(&env, &user)
    }

    /// Configure the SignalRegistry contract address used by recommendation queries.
    pub fn set_signal_registry(env: Env, caller: Address, registry: Address) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::SignalRegistry, &registry);
    }

    /// Retrieve the configured SignalRegistry address.
    pub fn get_signal_registry(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::SignalRegistry)
    }

    /// Store trading style profile for `user`. Caller must be `user`.
    pub fn set_trading_style(env: Env, user: Address, style: TradingStyle) {
        preferences::set_trading_style(&env, &user, style);
    }

    /// Retrieve trading style profile for `user`.
    pub fn get_trading_style(env: Env, user: Address) -> Option<TradingStyle> {
        preferences::get_trading_style(&env, &user)
    }

    /// Returns recommended active signals for `user` based on their trading style.
    /// When no style is configured, returns all active signals.
    pub fn get_recommended_signals(env: Env, user: Address) -> Vec<SignalSummary> {
        let registry: Address = env
            .storage()
            .instance()
            .get(&DataKey::SignalRegistry)
            .expect("signal registry not configured");
        preferences::get_recommended_signals(&env, &user, &registry)
    }

    // ── Issue #432: Achievement System ───────────────────────────────────────

    /// Returns all achievements for `user` with current progress.
    pub fn get_achievements(env: Env, user: Address) -> Vec<Achievement> {
        achievements::get_achievements(&env, &user)
    }

    /// Returns whether the user has completed the quest identified by `quest_id`.
    pub fn verify_quest_completion(env: Env, user: Address, quest_id: u32) -> bool {
        achievements::verify_quest_completion(&env, &user, quest_id)
    }

    // ── Issue #703: Custom position tagging ─────────────────────────────────────

    /// Tag a position owned by `user` with a custom string label.
    ///
    /// - Only the position owner may call this.
    /// - `tag` must be 1–32 characters.
    /// - Retagging overwrites the previous tag.
    /// - Emits `PositionTagged` event.
    pub fn tag_position(env: Env, user: Address, position_id: u64, tag: String) {
        position_tags::tag_position(&env, user, position_id, tag).unwrap();
    }

    /// Remove a tag from a position owned by `user`.
    pub fn untag_position(env: Env, user: Address, position_id: u64) {
        position_tags::untag_position(&env, user, position_id);
    }

    /// Returns all position IDs for `user` that have been tagged with `tag`.
    pub fn get_positions_by_tag(env: Env, user: Address, tag: String) -> Vec<u64> {
        position_tags::get_positions_by_tag(&env, user, tag)
    }

    /// Returns the tag (if any) for a specific position owned by `user`.
    pub fn get_position_tag(env: Env, user: Address, position_id: u64) -> Option<String> {
        position_tags::get_position_tag(&env, user, position_id)
    }

    // ── Issue #752: Per-asset maximum exposure cap ────────────────────────────

    /// User: set an optional maximum exposure cap for `asset_id`.
    ///
    /// - Opt-in; assets without a cap are unrestricted (default behavior).
    /// - `cap_amount` must be > 0.
    /// - Caller must be `user`.
    pub fn set_asset_exposure_cap(
        env: Env,
        user: Address,
        asset_id: u32,
        cap_amount: i128,
    ) -> Result<(), PortfolioError> {
        user.require_auth();
        if cap_amount <= 0 {
            return Err(PortfolioError::InvalidCapAmount);
        }
        exposure_cap::set_cap(&env, &user, asset_id, cap_amount);
        Ok(())
    }

    /// User: remove the exposure cap for `asset_id`.
    pub fn remove_asset_exposure_cap(env: Env, user: Address, asset_id: u32) {
        user.require_auth();
        exposure_cap::remove_cap(&env, &user, asset_id);
    }

    /// Returns the configured exposure cap for `(user, asset_id)`, or `None` if not set.
    pub fn get_asset_exposure_cap(env: Env, user: Address, asset_id: u32) -> Option<i128> {
        exposure_cap::get_cap(&env, &user, asset_id)
    }

    /// Returns the user's current tracked open exposure for `asset_id`.
    pub fn get_asset_exposure(env: Env, user: Address, asset_id: u32) -> i128 {
        exposure_cap::get_exposure(&env, &user, asset_id)
    }

    /// Open a position for copy-trade execution with enforced per-asset exposure cap.
    ///
    /// If the user has configured a cap for `asset_id` and adding `amount` to their
    /// current exposure would exceed it, the call is rejected with `ExposureCapExceeded`.
    /// The cap check uses current live portfolio state — not a stale snapshot.
    ///
    /// When no cap is configured for `asset_id`, this function behaves identically
    /// to `open_position`.
    pub fn open_position_with_cap_check(
        env: Env,
        user: Address,
        asset_id: u32,
        entry_price: i128,
        amount: i128,
    ) -> Result<u64, PortfolioError> {
        user.require_auth();
        if entry_price <= 0 || amount <= 0 {
            panic!("invalid entry_price or amount");
        }
        // KYC gate
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
        // Restriction gate
        let restricted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Restricted(user.clone()))
            .unwrap_or(false);
        if restricted {
            panic!("user is geographically restricted");
        }

        // Cap check — uses current live state.
        exposure_cap::check_cap(&env, &user, asset_id, amount)?;

        // Open the position.
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

        // Record which asset this position belongs to.
        env.storage()
            .persistent()
            .set(&DataKey::PositionAsset(id), &asset_id);

        let key = DataKey::UserPositions(user.clone());
        let mut list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        list.push_back(id);
        env.storage().persistent().set(&key, &list);

        let open_key = DataKey::UserOpenPositions(user.clone());
        let mut open_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&open_key)
            .unwrap_or_else(|| Vec::new(&env));
        open_ids.push_back(id);
        env.storage().persistent().set(&open_key, &open_ids);

        // Track exposure for cap enforcement.
        exposure_cap::add_exposure(&env, &user, asset_id, amount);

        Ok(id)
    }

    // ── Portfolio snapshots (issue #685) ─────────────────────────────────────────

    /// Record a snapshot of `user`'s current total portfolio value with a timestamp.
    ///
    /// Permissionless — keepers, admins, and users themselves may call this.
    /// Snapshot value is derived from `get_pnl().total_pnl`.
    pub fn take_snapshot(env: Env, user: Address) -> PortfolioSnapshot {
        let pnl = queries::compute_get_pnl(&env, user.clone());
        let total_value = pnl.total_pnl;
        let now = env.ledger().timestamp();

        env.storage().persistent().set(
            &DataKey::PortfolioSnapshotEntry(user.clone(), now),
            &total_value,
        );

        let ts_key = DataKey::UserSnapshotTimestamps(user.clone());
        let mut timestamps: Vec<u64> = env
            .storage()
            .persistent()
            .get(&ts_key)
            .unwrap_or_else(|| Vec::new(&env));
        timestamps.push_back(now);
        env.storage().persistent().set(&ts_key, &timestamps);

        #[allow(deprecated)]
        env.events().publish(
            (
                Symbol::new(&env, "user_portfolio"),
                Symbol::new(&env, "snapshot_taken"),
            ),
            (user, now, total_value),
        );

        PortfolioSnapshot {
            timestamp: now,
            total_value,
        }
    }

    /// Keeper- or admin-triggerable batch snapshot: records the current portfolio
    /// value for every address in `users` in a single call.
    /// Returns the number of snapshots recorded.
    pub fn batch_snapshot(env: Env, caller: Address, users: Vec<Address>) -> u32 {
        caller.require_auth();
        let mut count = 0u32;
        for i in 0..users.len() {
            let user = users.get(i).unwrap();
            let pnl = queries::compute_get_pnl(&env, user.clone());
            let total_value = pnl.total_pnl;
            let now = env.ledger().timestamp();

            env.storage().persistent().set(
                &DataKey::PortfolioSnapshotEntry(user.clone(), now),
                &total_value,
            );

            let ts_key = DataKey::UserSnapshotTimestamps(user.clone());
            let mut timestamps: Vec<u64> = env
                .storage()
                .persistent()
                .get(&ts_key)
                .unwrap_or_else(|| Vec::new(&env));
            timestamps.push_back(now);
            env.storage().persistent().set(&ts_key, &timestamps);

            count += 1;
        }
        count
    }

    /// Returns all portfolio snapshots for `user` whose timestamp falls in
    /// `[from, to]` (inclusive), ordered by ascending timestamp.
    pub fn get_snapshot_history(
        env: Env,
        user: Address,
        from: u64,
        to: u64,
    ) -> Vec<PortfolioSnapshot> {
        let ts_key = DataKey::UserSnapshotTimestamps(user.clone());
        let timestamps: Vec<u64> = env
            .storage()
            .persistent()
            .get(&ts_key)
            .unwrap_or_else(|| Vec::new(&env));

        let mut result = Vec::new(&env);
        for i in 0..timestamps.len() {
            let ts = timestamps.get(i).unwrap();
            if ts >= from && ts <= to {
                let total_value: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::PortfolioSnapshotEntry(user.clone(), ts))
                    .unwrap_or(0);
                result.push_back(PortfolioSnapshot {
                    timestamp: ts,
                    total_value,
                });
            }
        }
        result
    }

    // ── Portfolio concentration risk (Issue #684) ──────────────────────────────

    /// Admin: set the Herfindahl concentration score threshold (0–10 000 basis points).
    /// A `high_concentration_warning` event is emitted whenever a user's score meets or
    /// exceeds this value after a new position is opened.  Default is 5 000 (HHI ≥ 0.5).
    pub fn set_concentration_threshold(env: Env, threshold: u32) {
        Self::require_admin(&env);
        let capped = threshold.min(10_000);
        env.storage()
            .instance()
            .set(&DataKey::ConcentrationThreshold, &capped);
    }

    /// Returns the configured concentration warning threshold (default: 5 000 bps).
    pub fn get_concentration_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ConcentrationThreshold)
            .unwrap_or(5_000u32)
    }

    /// Returns the Herfindahl-Hirschman Index concentration score for `user`'s open
    /// positions, in basis points (0 = fully diversified, 10 000 = single position).
    pub fn get_concentration_score(env: Env, user: Address) -> u32 {
        compute_concentration_score(&env, &user)
    }

    // ── FIFO cost-basis tracking ──────────────────────────────────────────────

    /// Enqueue a cost lot for `user` at the back of their FIFO queue.
    /// Lots are consumed front-first by `close_fifo`.
    pub fn add_cost_lot(env: Env, user: Address, entry_price: i128, amount: i128) {
        user.require_auth();
        if entry_price <= 0 || amount <= 0 {
            panic!("invalid entry_price or amount");
        }
        let key = DataKey::UserFifoLots(user.clone());
        let mut lots: Vec<CostLot> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env));
        lots.push_back(CostLot {
            entry_price,
            amount,
        });
        env.storage().persistent().set(&key, &lots);
    }

    /// Close `close_amount` units at `exit_price` using FIFO lot consumption.
    /// Panics if there are insufficient lots to satisfy `close_amount`.
    /// Returns the realized P&L for this operation and accumulates it in storage.
    pub fn close_fifo(env: Env, user: Address, close_amount: i128, exit_price: i128) -> i128 {
        user.require_auth();
        if close_amount <= 0 || exit_price <= 0 {
            panic!("invalid close_amount or exit_price");
        }

        let lot_key = DataKey::UserFifoLots(user.clone());
        let lots: Vec<CostLot> = env
            .storage()
            .persistent()
            .get(&lot_key)
            .unwrap_or_else(|| Vec::new(&env));

        let mut remaining = close_amount;
        let mut total_pnl: i128 = 0;
        let mut new_lots: Vec<CostLot> = Vec::new(&env);

        for i in 0..lots.len() {
            let Some(lot) = lots.get(i) else { continue };
            if remaining <= 0 {
                new_lots.push_back(lot);
                continue;
            }
            let consumed = if lot.amount <= remaining {
                lot.amount
            } else {
                remaining
            };
            if lot.entry_price > 0 {
                // realized = (exit_price - entry_price) * consumed / entry_price
                let pnl = exit_price
                    .checked_sub(lot.entry_price)
                    .and_then(|d| d.checked_mul(consumed))
                    .and_then(|p| p.checked_div(lot.entry_price))
                    .unwrap_or(0);
                total_pnl = total_pnl.saturating_add(pnl);
            }
            remaining = remaining.saturating_sub(consumed);
            let leftover = lot.amount.saturating_sub(consumed);
            if leftover > 0 {
                new_lots.push_back(CostLot {
                    entry_price: lot.entry_price,
                    amount: leftover,
                });
            }
        }

        if remaining > 0 {
            panic!("insufficient lots to close requested amount");
        }

        env.storage().persistent().set(&lot_key, &new_lots);

        let pnl_key = DataKey::UserFifoRealizedPnl(user.clone());
        let prev: i128 = env.storage().persistent().get(&pnl_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&pnl_key, &prev.saturating_add(total_pnl));

        total_pnl
    }

    // ── Tax event log (Issue #658) ────────────────────────────────────────────

    /// Returns tax events for `user` whose `close_ts` falls in `[from_ts, to_ts]`
    /// (inclusive), with pagination via `offset` and `limit`.
    ///
    /// Events are returned in chronological close order (the order they were appended).
    /// Pass `from_ts = 0` and `to_ts = u64::MAX` to retrieve all events.
    pub fn get_tax_events(
        env: Env,
        user: Address,
        from_ts: u64,
        to_ts: u64,
        offset: u32,
        limit: u32,
    ) -> Vec<TaxEvent> {
        let tax_key = DataKey::UserTaxEvents(user);
        let all: Vec<TaxEvent> = env
            .storage()
            .persistent()
            .get(&tax_key)
            .unwrap_or_else(|| Vec::new(&env));

        let mut result = Vec::new(&env);
        let mut skipped = 0u32;
        let mut count = 0u32;

        for i in 0..all.len() {
            let event = all.get(i).unwrap();
            if event.close_ts < from_ts || event.close_ts > to_ts {
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            if count >= limit {
                break;
            }
            result.push_back(event);
            count += 1;
        }
        result
    }

    // ── Issue #862: Health / Readiness ───────────────────────────────────────

    /// Read-only health probe for monitoring and front-ends (no auth).
    pub fn health_check(env: Env) -> HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        if !env.storage().instance().has(&DataKey::Initialized) {
            return health_uninitialized(&env, version);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| stellar_swipe_common::health::placeholder_admin(&env));
        HealthStatus {
            is_initialized: true,
            is_paused: false,
            version,
            admin,
        }
    }

    fn require_admin(env: &Env) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("admin");
        admin.require_auth();
    }

    fn mark_position_closed(env: &Env, user: &Address, position_id: u64) {
        let open_key = DataKey::UserOpenPositions(user.clone());
        let open_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&open_key)
            .unwrap_or_else(|| Vec::new(env));
        let mut next_open = Vec::new(env);
        for i in 0..open_ids.len() {
            if let Some(id) = open_ids.get(i) {
                if id != position_id {
                    next_open.push_back(id);
                }
            }
        }
        env.storage().persistent().set(&open_key, &next_open);

        let closed_key = DataKey::UserClosedPositions(user.clone());
        let mut closed_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&closed_key)
            .unwrap_or_else(|| Vec::new(env));
        let mut already_closed = false;
        for i in 0..closed_ids.len() {
            if closed_ids.get(i) == Some(position_id) {
                already_closed = true;
                break;
            }
        }
        if !already_closed {
            closed_ids.push_back(position_id);
            env.storage().persistent().set(&closed_key, &closed_ids);
        }
    }
}

#[cfg(test)]
mod streak_tests {
    use super::oracle_ok::OracleMock;
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Env, Symbol};

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let oracle = env.register_contract(None, OracleMock);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(env, &contract_id);
        client.initialize(&admin, &oracle);
        (admin, contract_id)
    }

    #[test]
    fn streak_building_and_best_preserved() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        // Open and close three profitable positions
        let provider = Address::generate(&env);
        for _ in 0..3 {
            let id = client.open_position(&user, &100, &1_000);
            client.close_position(&user, &id, &50, &120, &1u32, &provider, &1u64);
        }

        // Check stored streaks
        let current: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::CurrentStreak(user.clone()))
                .unwrap_or(0u32)
        });
        let best: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::BestStreak(user.clone()))
                .unwrap_or(0u32)
        });

        assert_eq!(current, 3);
        assert_eq!(best, 3);
    }

    #[test]
    fn streak_breaking_emits_event_and_resets() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;

        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        let provider = Address::generate(&env);

        // Build streak of 2
        for _ in 0..2 {
            let id = client.open_position(&user, &100, &1_000);
            client.close_position(&user, &id, &50, &120, &1u32, &provider, &1u64);
        }

        // Now close with a loss
        let id = client.open_position(&user, &100, &1_000);
        client.close_position(&user, &id, &-50, &80, &1u32, &provider, &1u64);

        // Check events before any other contract frame (env.events().all() is per-frame).
        let has_broken = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "streak_broken"))
                .unwrap_or(false)
        });
        assert!(has_broken, "streak_broken event not emitted");

        // current should be 0, best should be 2
        let current: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::CurrentStreak(user.clone()))
                .unwrap_or(0u32)
        });
        let best: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::BestStreak(user.clone()))
                .unwrap_or(0u32)
        });

        assert_eq!(current, 0);
        assert_eq!(best, 2);
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
mod migration_tests {
    use super::oracle_ok::OracleMock;
    use super::oracle_ok::OracleMockClient;
    use super::*;
    use crate::storage::DataKey;
    use soroban_sdk::testutils::Address as _;
    use stellar_swipe_common::OraclePrice;

    fn close_test_position(env: &Env, client: &UserPortfolioClient, user: &Address, id: u64) {
        let provider = Address::generate(env);
        client.close_position(user, &id, &50, &110i128, &1u32, &provider, &0u64);
    }

    fn seed_oracle_price(env: &Env, oracle_id: &Address) {
        OracleMockClient::new(env, oracle_id).set_price(
            &1u32,
            &OraclePrice {
                price: 100i128,
                decimals: 7,
                timestamp: 0,
                source: soroban_sdk::symbol_short!("mock"),
            },
        );
    }

    /// 20 users × 5 open + 10 closed positions each.
    /// Verifies all positions are preserved after V1 → V2 migration.
    #[test]
    fn migrate_20_users_5_open_10_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle_id = env.register_contract(None, OracleMock);
        seed_oracle_price(&env, &oracle_id);
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
                close_test_position(&env, &client, &user, id);
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

            let open_ids: soroban_sdk::Vec<u64> = env.as_contract(&contract_id, || {
                env.storage()
                    .persistent()
                    .get(&DataKey::UserOpenPositions(user.clone()))
                    .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
            });
            let closed_ids: soroban_sdk::Vec<u64> = env.as_contract(&contract_id, || {
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
        seed_oracle_price(&env, &oracle_id);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle_id);

        let user = Address::generate(&env);
        client.open_position(&user, &100, &1_000);
        close_test_position(&env, &client, &user, 1);

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
mod anchor_deposit_tests {
    use super::oracle_ok::OracleMock;
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn get_anchor_deposit_address_returns_configured_address() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let oracle = env.register_contract(None, OracleMock);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle);

        let token = Address::generate(&env);
        let deposit_address = Address::generate(&env);
        client.set_anchor_deposit_address(&token, &deposit_address);

        let user = Address::generate(&env);
        let info = client.get_anchor_deposit_address(&user, &token, &1_000);

        assert_eq!(info.deposit_address, deposit_address);
        assert_eq!(info.token, token);
        assert_eq!(info.amount_fiat, 1_000);
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

    #[test]
    fn get_portfolio_excludes_closed_positions_when_requested() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        client.open_position(&user, &100, &500);
        client.close_position(&user, &1, &50, &110i128, &1u32, &provider, &0u64);

        let portfolio = client.get_portfolio(&user, &false);
        assert_eq!(portfolio.open_positions.len(), 1);
        assert_eq!(portfolio.open_positions.get(0).unwrap().position_id, 2);
        assert_eq!(portfolio.closed_positions.len(), 0);
        assert_eq!(portfolio.closed_position_ids.len(), 0);
    }

    #[test]
    fn get_portfolio_includes_small_closed_positions() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);
        client.open_position(&user, &100, &500);
        client.close_position(&user, &1, &50, &110i128, &1u32, &provider, &0u64);

        let portfolio = client.get_portfolio(&user, &true);
        assert_eq!(portfolio.open_positions.len(), 1);
        assert_eq!(portfolio.closed_position_ids.len(), 1);
        assert_eq!(portfolio.closed_position_ids.get(0).unwrap(), 1);
        assert_eq!(portfolio.closed_positions.len(), 1);
        assert_eq!(portfolio.closed_positions.get(0).unwrap().position_id, 1);
    }

    #[test]
    fn get_portfolio_lazy_loads_many_closed_positions_under_half_budget() {
        const HALF_DEFAULT_CPU_BUDGET: u64 = 50_000_000;

        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        for i in 0..50 {
            let position_id = client.open_position(&user, &100, &1_000);
            client.close_position(
                &user,
                &position_id,
                &(i as i128),
                &100i128,
                &1u32,
                &provider,
                &0u64,
            );
        }
        for _ in 0..20 {
            client.open_position(&user, &100, &1_000);
        }

        env.cost_estimate().budget().reset_tracker();
        let open_only = client.get_portfolio(&user, &false);
        let instructions = env.cost_estimate().budget().cpu_instruction_cost();

        assert_eq!(open_only.open_positions.len(), 20);
        assert_eq!(open_only.closed_positions.len(), 0);
        assert_eq!(open_only.closed_position_ids.len(), 0);
        assert!(
            instructions < HALF_DEFAULT_CPU_BUDGET,
            "get_portfolio(include_closed=false) used {instructions} instructions"
        );

        let with_closed = client.get_portfolio(&user, &true);
        assert_eq!(with_closed.open_positions.len(), 20);
        assert_eq!(with_closed.closed_position_ids.len(), 50);
        assert_eq!(with_closed.closed_positions.len(), 0);
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
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "trade_shareable"))
                .unwrap_or(false)
        });
        assert!(
            !has_trade_shareable,
            "TradeShareable must not be emitted for a loss"
        );
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
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "trade_shareable"))
                .unwrap_or(false)
        });
        assert!(
            !has_trade_shareable,
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
            if topics.len() < 2 {
                return false;
            }
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
        assert_eq!(
            event,
            soroban_sdk::Symbol::new(&env, "subscription_created")
        );
    }

    // ── Issue #389: position state machine tests ──────────────────────────────

    /// First close succeeds; second close returns PositionAlreadyClosed.
    #[test]
    fn concurrent_close_second_returns_already_closed() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        client.open_position(&user, &100, &1_000);

        // First close — must succeed.
        let first = client.try_close_position(&user, &1, &50, &110i128, &1u32, &provider, &0u64);
        assert!(first.is_ok(), "first close should succeed");

        // Second close — must return PositionAlreadyClosed.
        let second = client.try_close_position(&user, &1, &50, &110i128, &1u32, &provider, &0u64);
        assert_eq!(second, Err(Ok(PositionError::PositionAlreadyClosed)));
    }

    /// Closing state transitions: Open → Closing → Closed.
    #[test]
    fn position_transitions_open_to_closed() {
        let env = Env::default();
        let (user, portfolio_id, _) = setup_portfolio(&env, true, 100);
        let client = UserPortfolioClient::new(&env, &portfolio_id);
        let provider = dummy_provider(&env);

        let id = client.open_position(&user, &100, &1_000);

        // Verify Open state.
        let pos: Position = env.as_contract(&portfolio_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::Position(id))
                .unwrap()
        });
        assert_eq!(pos.status, PositionStatus::Open);

        client.close_position(&user, &id, &100, &110i128, &1u32, &provider, &0u64);

        // Verify Closed state.
        let pos: Position = env.as_contract(&portfolio_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::Position(id))
                .unwrap()
        });
        assert_eq!(pos.status, PositionStatus::Closed);
    }
}

// ── Portfolio snapshot tests (issue #685) ─────────────────────────────────────
#[cfg(test)]
mod snapshot_tests {
    use super::oracle_ok::OracleMock;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let oracle = env.register_contract(None, OracleMock);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(env, &contract_id);
        client.initialize(&admin, &oracle);
        (admin, contract_id)
    }

    #[test]
    fn take_snapshot_records_value_and_timestamp() {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = 1_000);
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        let snap = client.take_snapshot(&user);
        assert_eq!(snap.timestamp, 1_000);
        assert_eq!(snap.total_value, 0); // no positions → P&L = 0
    }

    #[test]
    fn snapshots_ordered_by_ascending_timestamp() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        env.ledger().with_mut(|l| l.timestamp = 100);
        client.take_snapshot(&user);
        env.ledger().with_mut(|l| l.timestamp = 200);
        client.take_snapshot(&user);
        env.ledger().with_mut(|l| l.timestamp = 300);
        client.take_snapshot(&user);

        let history = client.get_snapshot_history(&user, &0u64, &u64::MAX);
        assert_eq!(history.len(), 3);
        assert_eq!(history.get(0).unwrap().timestamp, 100);
        assert_eq!(history.get(1).unwrap().timestamp, 200);
        assert_eq!(history.get(2).unwrap().timestamp, 300);
    }

    #[test]
    fn get_snapshot_history_filters_by_range() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        for ts in [100u64, 200, 300, 400, 500] {
            env.ledger().with_mut(|l| l.timestamp = ts);
            client.take_snapshot(&user);
        }

        let history = client.get_snapshot_history(&user, &200u64, &400u64);
        assert_eq!(history.len(), 3);
        assert_eq!(history.get(0).unwrap().timestamp, 200);
        assert_eq!(history.get(2).unwrap().timestamp, 400);
    }

    #[test]
    fn batch_snapshot_records_all_users() {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = 500);
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let keeper = Address::generate(&env);
        let user_a = Address::generate(&env);
        let user_b = Address::generate(&env);
        let user_c = Address::generate(&env);

        let mut users = soroban_sdk::vec![&env];
        users.push_back(user_a.clone());
        users.push_back(user_b.clone());
        users.push_back(user_c.clone());

        let count = client.batch_snapshot(&keeper, &users);
        assert_eq!(count, 3);

        for user in [user_a, user_b, user_c] {
            let history = client.get_snapshot_history(&user, &0u64, &u64::MAX);
            assert_eq!(history.len(), 1);
            assert_eq!(history.get(0).unwrap().timestamp, 500);
        }
    }

    #[test]
    fn get_snapshot_history_empty_when_no_snapshots() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        let history = client.get_snapshot_history(&user, &0u64, &u64::MAX);
        assert_eq!(history.len(), 0);
    }
}

// ── Concentration risk score tests (Issue #684) ───────────────────────────────
#[cfg(test)]
mod concentration_tests {
    use super::oracle_ok::OracleMock;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events};
    use soroban_sdk::TryFromVal;

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let oracle = env.register_contract(None, OracleMock);
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(env, &contract_id);
        client.initialize(&admin, &oracle);
        (admin, contract_id)
    }

    fn open(env: &Env, client: &UserPortfolioClient, user: &Address, amount: i128) -> u64 {
        client.open_position(user, &100, &amount)
    }

    fn close(env: &Env, client: &UserPortfolioClient, user: &Address, id: u64) {
        let provider = soroban_sdk::Address::generate(env);
        client.close_position(user, &id, &0, &100i128, &1u32, &provider, &0u64);
    }

    #[test]
    fn score_is_zero_with_no_positions() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        assert_eq!(client.get_concentration_score(&user), 0);
    }

    #[test]
    fn single_position_scores_10000() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        open(&env, &client, &user, 1_000);
        assert_eq!(client.get_concentration_score(&user), 10_000);
    }

    #[test]
    fn two_equal_positions_score_5000() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        open(&env, &client, &user, 1_000);
        // Disable threshold warning for this test (set very high)
        client.set_concentration_threshold(&10_001u32);
        open(&env, &client, &user, 1_000);
        let score = client.get_concentration_score(&user);
        // HHI for 2 equal positions = 2*(50%)^2 = 0.5 → 5000 bps
        assert_eq!(score, 5_000);
    }

    #[test]
    fn highly_concentrated_portfolio_scores_higher() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        client.set_concentration_threshold(&10_001u32);
        // 90% / 10% split
        open(&env, &client, &user, 9_000);
        open(&env, &client, &user, 1_000);
        let score = client.get_concentration_score(&user);
        // HHI = (0.9)^2 + (0.1)^2 = 0.81 + 0.01 = 0.82 → 8200 bps
        assert_eq!(score, 8_200);
    }

    #[test]
    fn equally_distributed_portfolio_has_lower_score() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        client.set_concentration_threshold(&10_001u32);
        open(&env, &client, &user, 1_000);
        open(&env, &client, &user, 1_000);
        open(&env, &client, &user, 1_000);
        open(&env, &client, &user, 1_000);
        let score = client.get_concentration_score(&user);
        // HHI = 4*(25%)^2 = 4*0.0625 = 0.25 → 2500 bps
        assert_eq!(score, 2_500);
    }

    #[test]
    fn default_threshold_is_5000() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        assert_eq!(client.get_concentration_threshold(), 5_000u32);
    }

    #[test]
    fn admin_can_configure_threshold() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.set_concentration_threshold(&7_500u32);
        assert_eq!(client.get_concentration_threshold(), 7_500u32);
    }

    #[test]
    fn threshold_is_capped_at_10000() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.set_concentration_threshold(&99_999u32);
        assert_eq!(client.get_concentration_threshold(), 10_000u32);
    }

    #[test]
    fn high_concentration_warning_emitted_when_threshold_crossed() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        // Set threshold to 6000; single position = HHI 10000 ≥ 6000 → warning
        client.set_concentration_threshold(&6_000u32);
        open(&env, &client, &user, 1_000);

        let has_warning = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "high_concentration_warning"))
                .unwrap_or(false)
        });
        assert!(has_warning, "high_concentration_warning must be emitted");
    }

    #[test]
    fn no_warning_emitted_below_threshold() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        // Set threshold very high so no warning fires
        client.set_concentration_threshold(&10_001u32);
        open(&env, &client, &user, 1_000);
        open(&env, &client, &user, 1_000);

        // score = 5000, threshold = 10001 (capped at 10000 so 5000 < 10000)
        let has_warning = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "high_concentration_warning"))
                .unwrap_or(false)
        });
        assert!(!has_warning, "no warning expected below threshold");
    }

    #[test]
    fn score_drops_after_closing_concentrated_position() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        client.set_concentration_threshold(&10_001u32);

        let id_big = open(&env, &client, &user, 9_000);
        open(&env, &client, &user, 1_000);
        let score_before = client.get_concentration_score(&user);
        assert_eq!(score_before, 8_200);

        close(&env, &client, &user, id_big);
        let score_after = client.get_concentration_score(&user);
        // Only the 1_000-amount position remains → HHI = 10_000
        assert_eq!(score_after, 10_000);
    }
}

// ── Geographic restriction unit tests ─────────────────────────────────────────
#[cfg(test)]
mod restriction_tests {
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

    fn reason(env: &Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(env, "QmFakeIpfsHash")
    }

    // Restriction flag defaults to false.
    #[test]
    fn restriction_defaults_to_false() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        assert!(!client.is_restricted(&user));
    }

    // Unrestricted user can open a position.
    #[test]
    fn unrestricted_user_can_open_position() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        let id = client.open_position(&user, &100, &1_000);
        assert_eq!(id, 1);
    }

    // Restricted user cannot open a position.
    #[test]
    #[should_panic(expected = "user is geographically restricted")]
    fn restricted_user_cannot_open_position() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        client.set_user_restriction(&user, &true, &reason(&env));
        client.open_position(&user, &100, &1_000);
    }

    // Admin can remove restriction and user can trade again.
    #[test]
    fn admin_can_remove_restriction() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        client.set_user_restriction(&user, &true, &reason(&env));
        assert!(client.is_restricted(&user));
        client.set_user_restriction(&user, &false, &reason(&env));
        assert!(!client.is_restricted(&user));
        // Should succeed now.
        let id = client.open_position(&user, &100, &1_000);
        assert_eq!(id, 1);
    }

    // set_user_restriction emits user_restricted event.
    #[test]
    fn set_user_restriction_emits_event() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::TryFromVal;
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);
        client.set_user_restriction(&user, &true, &reason(&env));
        let has_event = env.events().all().iter().any(|e| {
            let topics: soroban_sdk::Vec<soroban_sdk::Val> = e.1.clone();
            if topics.len() < 2 {
                return false;
            }
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == soroban_sdk::Symbol::new(&env, "user_restricted"))
                .unwrap_or(false)
        });
        assert!(has_event, "user_restricted event not emitted");
    }
}

// ── Issue #752: Exposure cap unit tests ──────────────────────────────────────
#[cfg(test)]
mod exposure_cap_tests {
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

    /// A trade within the cap is allowed and exposure is updated.
    #[test]
    fn test_trade_within_cap_allowed() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        // Set cap of 5_000 for asset 1.
        client.set_asset_exposure_cap(&user, &1u32, &5_000i128);

        // Trade 3_000 — within cap.
        let result = client.try_open_position_with_cap_check(&user, &1u32, &100i128, &3_000i128);
        assert!(result.is_ok(), "trade within cap should be allowed");

        assert_eq!(client.get_asset_exposure(&user, &1u32), 3_000);
    }

    /// A trade that would exceed the cap is rejected with ExposureCapExceeded.
    #[test]
    fn test_trade_exceeding_cap_rejected() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_asset_exposure_cap(&user, &1u32, &2_000i128);

        // 3_000 > 2_000 cap → must be rejected.
        let result = client.try_open_position_with_cap_check(&user, &1u32, &100i128, &3_000i128);
        assert_eq!(result, Err(Ok(PortfolioError::ExposureCapExceeded)));
    }

    /// An asset with no cap configured is unaffected — any amount is allowed.
    #[test]
    fn test_no_cap_asset_unaffected() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        // No cap set for asset 99.
        let result =
            client.try_open_position_with_cap_check(&user, &99u32, &100i128, &999_999_999i128);
        assert!(result.is_ok(), "no cap → any amount should pass");
    }

    /// Users can update or remove their caps.
    #[test]
    fn test_user_can_update_and_remove_cap() {
        let env = Env::default();
        let (_admin, contract_id) = setup(&env);
        let client = UserPortfolioClient::new(&env, &contract_id);
        let user = Address::generate(&env);

        client.set_asset_exposure_cap(&user, &1u32, &1_000i128);
        assert_eq!(client.get_asset_exposure_cap(&user, &1u32), Some(1_000));

        // Raise the cap.
        client.set_asset_exposure_cap(&user, &1u32, &5_000i128);
        assert_eq!(client.get_asset_exposure_cap(&user, &1u32), Some(5_000));

        // Remove the cap entirely.
        client.remove_asset_exposure_cap(&user, &1u32);
        assert_eq!(client.get_asset_exposure_cap(&user, &1u32), None);

        // Trade that previously would have been capped now succeeds.
        let result = client.try_open_position_with_cap_check(&user, &1u32, &100i128, &999_000i128);
        assert!(result.is_ok(), "cap removed → trade should be allowed");
    }
}
