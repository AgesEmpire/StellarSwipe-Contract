#![no_std]
// The `#[cfg_attr(test, derive(soroban_sdk::testutils::arbitrary::Arbitrary))]`
// on `ContractError` expands to `std::thread_local!` / `std::cell::Cell` paths,
// so the test build of this no_std crate must link `std`.
#[cfg(test)]
extern crate std;

pub mod batch_settlement;
pub mod compat;
pub mod dca;
mod errors;
pub mod feature_flags;
pub mod keeper;
pub mod leverage;
mod oracle;
pub mod priority;
pub mod risk_gates;
pub mod sdex;
pub mod triggers;
mod wire;

use errors::{ContractError, InsufficientBalanceDetail, NetworkErrorDetail};
use risk_gates::{
    check_user_balance, resolve_trade_amount, validate_and_record_position,
    validate_min_trade_size, DEFAULT_ESTIMATED_COPY_TRADE_FEE, DEFAULT_MIN_TRADE_SIZE,
    MAX_BATCH_SIZE,
};
use sdex::{execute_sdex_swap, min_received_from_slippage};
use shared::math::normalize_amount;
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Bytes, BytesN, Env, IntoVal, String, Symbol,
    Val, Vec,
};

use stellar_swipe_common::pair_validation::{self, PairValidationError};
use stellar_swipe_common::replay_protection::{
    purge_expired_nonces as replay_purge_expired_nonces, verify_and_commit, ReplayError,
};
use triggers::{ORACLE_KEY, PORTFOLIO_KEY};
use wire::TRADE_TIMEOUT_LEDGERS;

/// Instance storage keys.
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    /// Contract implementing `validate_and_record(user, max_positions) -> u32` (UserPortfolio).
    UserPortfolio,
    /// When set to `true`, this user bypasses the per-user position cap.
    PositionLimitExempt(Address),
    /// Oracle contract used by stop-loss/take-profit triggers (`get_price(asset_pair) -> i128`).
    Oracle,
    /// Portfolio contract used by stop-loss/take-profit close calls (`close_position(user, trade_id, pnl)`).
    StopLossPortfolio,
    /// Overrides default estimated fee used in balance checks (`None` = use default constant).
    CopyTradeEstimatedFee,
    /// Last balance shortfall for a user (cleared after a successful `execute_copy_trade`).
    LastInsufficientBalance(Address),
    SdexRouter,
    /// Global daily trade volume limit in USD-equivalent units (0 = no limit).
    DailyVolumeLimit,
    /// Accumulated trade volume for `user` on the current day.
    DailyVolume(Address),
    /// The ledger-day (timestamp / 86400) when `DailyVolume(user)` was last reset.
    DailyVolumeDay(Address),
    /// Oracle contracts allowed to feed stop-loss / take-profit triggers.
    OracleWhitelisted(Address),
    OracleWhitelistCount,
    NextLimitOrderId,
    PendingLimitOrder(u64),
    PendingLimitOrderIds,
    SdexPrice(Address),
    /// DCA plan for (user, signal_id). Stores a `DCAPlan`.
    DCAPlan(Address, u64),
    /// Set when fee fallback was used for a trade: stores the fee amount deducted from received.
    FeeDeductedFromReceived(Address, u64),
    CircuitBreakerActive,
    CircuitBreakerLedger,
    MaxOpenInterestPerPair,
    OpenInterestPerPair(Address),
    /// Feature flag: keyed by flag name. `true` = enabled, absent/`false` = disabled.
    FeatureFlag(String),
    /// Per-asset minimum trade size override (absent = use [`DEFAULT_MIN_TRADE_SIZE`]).
    MinTradeSize(Address),
    /// Grace period (in ledger sequences) for cancelling queued trades before execution.
    TradeGracePeriod,
    /// A queued copy trade waiting for the grace period to elapse.
    QueuedTrade(u64),
    /// Next available queued trade ID.
    NextQueuedTradeId,
    /// List of all pending queued trade IDs.
    QueuedTradeIds,
    /// Maximum number of execution attempts before a queued trade is dead-lettered.
    MaxRetryCount,
    /// A dead-lettered trade record stored by its original queued trade ID.
    DeadLetterTrade(u64),
    /// Per-user index of dead-lettered trade IDs.
    DeadLetterIds(Address),
    /// Global index of all dead-lettered trade IDs across every user, used by
    /// `prune_dead_letter_queue` when sweeping without a `user` filter.
    DeadLetterAllIds,
    /// Retention window (in ledgers) after which a dead-lettered trade becomes
    /// eligible for removal via `prune_dead_letter_queue`.
    DeadLetterRetentionLedgers,
    /// Priority-lane configuration (PriorityConfig).
    PriorityConfig,
    /// The most recent trade order awaiting confirmation-depth finalization.
    PendingTradeConfirmation,
    /// Maximum concurrent open positions per user (absent = [`DEFAULT_MAX_OPEN_POSITIONS`]).
    MaxOpenPositions,
    /// Number of currently open copy-trade positions for `user` (persistent storage).
    UserPositionCount(Address),
    /// Consecutive priority-only batch counter for fairness fallback.
    PriorityBatchCounter,
    /// SHA-256 receipt hash for a completed trade, keyed by monotonic receipt ID.
    TradeReceiptHash(u64),
    /// Next trade receipt ID counter.
    NextTradeReceiptId,
    ConfirmationDepth,
    /// Issue #865: global pause flag set via governance-driven propagation.
    Paused,
    /// Issue #865: central governance contract address authorized to call
    /// `apply_governance_pause`.
    GovernanceAddress,
    /// Issue #959: per-user partial fill record keyed by (user, trade_id).
    /// Stores a `PartialFillRecord` when the SDEX only fills part of the requested amount.
    PartialFillRecord(Address, u64),
    /// Callback context stored before the SDEX routing call, consumed on
    /// settlement (`accept_settlement_callback`) to atomically claim the trade.
    CallbackContext(u64),
    /// Issue #992: asset registry contract used to validate asset pairs before
    /// any swap/offer is attempted. When set, pairs must be registered and
    /// distinct, and the configured route must support them.
    AssetRegistry,
}

/// Temporary-storage key for the reentrancy lock on `execute_copy_trade`.
const EXECUTION_LOCK: &str = "ExecLock";
pub const CIRCUIT_BREAKER_DURATION_LEDGERS: u32 = 720;

/// Denominator used to convert `entry_price * amount` into `to_token` units.
/// Entry prices are expected to be in 7‑decimal format (e.g. 10_000_000 = 1.0).
const ENTRY_PRICE_DENOMINATOR: i128 = 10_000_000;

/// Recorded when the SDEX only partially matches a copy-trade offer (Issue #959).
/// Persisted in instance storage under `StorageKey::PartialFillRecord(user, trade_id)`
/// so the frontend / keeper can inspect the shortfall without re-parsing events.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialFillRecord {
    /// Amount originally submitted to the SDEX.
    pub requested_amount: i128,
    /// Amount actually received from the SDEX.
    pub filled_amount: i128,
    /// Unfilled remainder (`requested_amount - filled_amount`).
    pub remaining_amount: i128,
    /// Ledger sequence at which the partial fill was detected.
    pub detected_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallbackContext {
    pub executor: Address,
    pub route: BytesN<32>,
    pub from_asset: Address,
    pub to_asset: Address,
}

/// A trade queued for execution, subject to a configurable grace period.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedTrade {
    pub queued_trade_id: u64,
    pub user: Address,
    pub token: Address,
    pub amount: i128,
    pub queued_at_ledger: u32,
    pub portfolio_pct_bps: Option<u32>,
    /// Number of failed execution attempts so far (0 = never tried).
    pub retry_count: u32,
}

/// Default grace period: 10 ledgers (~50 seconds at 5s/ledger).
pub const DEFAULT_GRACE_PERIOD_LEDGERS: u32 = 10;

/// Default maximum execution attempts before a trade is dead-lettered (3 attempts total).
pub const DEFAULT_MAX_RETRY_COUNT: u32 = 3;

/// Default dead-letter retention window: ~30 days at 5s/ledger
/// (30 * 24 * 60 * 60 / 5 = 518_400 ledgers).
pub const DEFAULT_DEAD_LETTER_RETENTION_LEDGERS: u32 = 518_400;

/// A trade that has exhausted all retry attempts and been moved to the dead-letter queue.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedTrade {
    /// Original queued trade ID.
    pub trade_id: u64,
    pub user: Address,
    pub token: Address,
    pub amount: i128,
    pub portfolio_pct_bps: Option<u32>,
    /// `ContractError` discriminant of the last failure.
    pub failure_code: u32,
    /// Total execution attempts made before dead-lettering.
    pub retry_count: u32,
    /// Ledger sequence at which the trade was dead-lettered.
    pub dead_lettered_at_ledger: u32,
}

/// Event emitted when a trade is moved to the dead-letter queue.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeDeadLettered {
    pub trade_id: u64,
    pub user: Address,
    pub failure_code: u32,
    pub retry_count: u32,
}

/// Event emitted when a dead-lettered trade is removed by `prune_dead_letter_queue`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadLetterPruned {
    pub user: Address,
    pub trade_id: u64,
    pub reason: String,
}

/// A single trade input for [`TradeExecutorContract::batch_execute`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchTradeInput {
    pub user: Address,
    pub token: Address,
    pub amount: i128,
}

/// Per-trade outcome returned by [`TradeExecutorContract::batch_execute`].
/// `ok = true` means the trade succeeded; `ok = false` means it failed with `error_code`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchTradeResult {
    pub ok: bool,
    /// `ContractError` discriminant when `ok == false`; 0 when `ok == true`.
    pub error_code: u32,
    /// `true` when this entry belongs to an atomic batch (Issue #793) that
    /// failed and was rolled back via panic. Set on every entry of the
    /// locally-computed result vec right before the panic that discards it —
    /// the panic itself (not this field) is what makes the rollback real;
    /// this exists so the marking logic is unit-testable on its own (see
    /// `mark_atomic_rollback`) and so the intent is self-documenting in code
    /// that inspects a `BatchTradeResult` outside the panicking call path
    /// (e.g. a future off-chain dry-run/simulation layer). Always `false` in
    /// non-atomic mode and in a successful atomic batch, since nothing was
    /// rolled back in either case.
    pub atomic_rollback: bool,
}

/// Instance config hoisted once per `batch_execute` call to amortize storage reads.
#[derive(Clone)]
struct BatchExecutionContext {
    portfolio: Address,
    estimated_fee: i128,
    daily_limit: i128,
    circuit_breaker_active: bool,
}

fn prepare_batch_context(env: &Env) -> Result<BatchExecutionContext, ContractError> {
    let portfolio = env
        .storage()
        .instance()
        .get(&StorageKey::UserPortfolio)
        .ok_or(ContractError::NotInitialized)?;
    Ok(BatchExecutionContext {
        portfolio,
        estimated_fee: effective_estimated_fee(env),
        daily_limit: env
            .storage()
            .instance()
            .get(&StorageKey::DailyVolumeLimit)
            .unwrap_or(0i128),
        circuit_breaker_active: market_circuit_breaker_active(env),
    })
}

/// Returns `results` unchanged if every entry succeeded, otherwise a copy
/// with `atomic_rollback: true` set on every entry. Pulled out of
/// `batch_execute_impl` so the marking logic (Issue #793) is unit-testable
/// on its own, independent of the panic-based rollback it precedes.
fn mark_atomic_rollback(env: &Env, results: &Vec<BatchTradeResult>) -> Vec<BatchTradeResult> {
    if results.iter().all(|r| r.ok) {
        return results.clone();
    }
    let mut marked: Vec<BatchTradeResult> = Vec::new(env);
    for r in results.iter() {
        marked.push_back(BatchTradeResult {
            ok: r.ok,
            error_code: r.error_code,
            atomic_rollback: true,
        });
    }
    marked
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationTarget {
    pub asset_pair: u32,
    pub target_pct_bps: u32,
}

pub fn rebalance_portfolio(_user: u32, targets: Vec<AllocationTarget>) {
    for t in targets.iter() {
        if t.target_pct_bps > 10000 {
            panic!("invalid allocation");
        }
    }
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

/// Replay-protection trio, bundled into one struct so contract entrypoints with
/// several other arguments stay under Soroban's 10-parameter function limit.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayParams {
    pub nonce: u64,
    pub tx_hash: Bytes,
    pub expiry_ts: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLimitOrder {
    pub order_id: u64,
    pub user: Address,
    pub token: Address,
    pub amount: i128,
    pub portfolio_pct_bps: Option<u32>,
    pub limit_price: i128,
    pub expires_at_ledger: u32,
}

soroban_sdk::contractmeta!(key = "SourceHash", val = env!("STELLAR_SOURCE_HASH"));
soroban_sdk::contractmeta!(key = "GitCommit", val = env!("STELLAR_GIT_COMMIT"));

#[contract]
pub struct TradeExecutorContract;

fn effective_estimated_fee(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::CopyTradeEstimatedFee)
        .unwrap_or(DEFAULT_ESTIMATED_COPY_TRADE_FEE)
}

fn require_admin(env: &Env) -> Result<Address, ContractError> {
    oracle::require_admin(env)
}

/// Issue #865: true when a governance-driven emergency pause is active.
fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&StorageKey::Paused)
        .unwrap_or(false)
}

/// Map a low-level [`ReplayError`] onto the contract's public error surface so
/// callers can distinguish "already used" from "expired" (Issue: nonce replay
/// attack prevention audit).
fn map_replay_error(err: ReplayError) -> ContractError {
    match err {
        ReplayError::Expired => ContractError::TradeExpired,
        ReplayError::InvalidNonce | ReplayError::DuplicateTx => ContractError::NonceAlreadyUsed,
    }
}

/// Map a [`PairValidationError`] onto the contract's public error surface (Issue #992).
fn map_pair_error(err: PairValidationError) -> ContractError {
    match err {
        PairValidationError::RegistryNotConfigured => ContractError::AssetRegistryNotConfigured,
        PairValidationError::BaseAssetNotRegistered
        | PairValidationError::QuoteAssetNotRegistered => ContractError::AssetNotRegistered,
        PairValidationError::IdenticalAssets => ContractError::IdenticalAssets,
        PairValidationError::RouteNotConfigured => ContractError::NotInitialized,
        PairValidationError::RouteUnsupported => ContractError::UnsupportedPair,
    }
}

/// Issue #992: reject unsupported asset pairs before any external call or
/// state mutation.
///
/// Enforced only when an asset registry is configured (see
/// [`Self::set_asset_registry`]). Contracts that never had a registry
/// configured keep their previous behavior; admins enable enforcement simply
/// by configuring one. When enforced, the pair must be registered and
/// distinct, and the configured route (`router`) must support it.
fn validate_pair_before_swap(
    env: &Env,
    registry: Option<&Address>,
    router: &Address,
    from_token: &Address,
    to_token: &Address,
) -> Result<(), ContractError> {
    if let Some(registry) = registry {
        pair_validation::validate_pair_for_route(
            env,
            Some(registry),
            Some(router),
            from_token,
            to_token,
        )
        .map_err(map_pair_error)?;
    }
    Ok(())
}

fn get_confirmation_depth(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::ConfirmationDepth)
        .unwrap_or(wire::DEFAULT_CONFIRMATION_DEPTH)
}
fn set_confirmation_depth(env: &Env, depth: u32) {
    env.storage()
        .instance()
        .set(&StorageKey::ConfirmationDepth, &depth);
}

/// Effective per-asset minimum trade size: the admin-configured override for `token`,
/// or [`DEFAULT_MIN_TRADE_SIZE`] when no override has been set.
fn effective_min_trade_size(env: &Env, token: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::MinTradeSize(token.clone()))
        .unwrap_or(DEFAULT_MIN_TRADE_SIZE)
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CircuitBreakerActivated {
    pub activated_by: Address,
    pub activated_ledger: u32,
    pub expires_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CircuitBreakerReset {
    pub reset_ledger: u32,
}

fn emit_circuit_breaker_activated(env: &Env, activated_by: Address, activated_ledger: u32) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "circuit_breaker_activated"),
        ),
        CircuitBreakerActivated {
            activated_by,
            activated_ledger,
            expires_ledger: activated_ledger.saturating_add(CIRCUIT_BREAKER_DURATION_LEDGERS),
        },
    );
}

fn emit_circuit_breaker_reset(env: &Env) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "circuit_breaker_reset"),
        ),
        CircuitBreakerReset {
            reset_ledger: env.ledger().sequence(),
        },
    );
}

fn reset_circuit_breaker(env: &Env) {
    env.storage()
        .instance()
        .set(&StorageKey::CircuitBreakerActive, &false);
    env.storage()
        .instance()
        .remove(&StorageKey::CircuitBreakerLedger);
    emit_circuit_breaker_reset(env);
}

fn market_circuit_breaker_active(env: &Env) -> bool {
    let active = env
        .storage()
        .instance()
        .get(&StorageKey::CircuitBreakerActive)
        .unwrap_or(false);
    if !active {
        return false;
    }

    let activated_ledger = env
        .storage()
        .instance()
        .get(&StorageKey::CircuitBreakerLedger)
        .unwrap_or(env.ledger().sequence());
    if env.ledger().sequence().saturating_sub(activated_ledger) >= CIRCUIT_BREAKER_DURATION_LEDGERS
    {
        reset_circuit_breaker(env);
        return false;
    }

    true
}

/// Read-only counterpart of [`market_circuit_breaker_active`]: reports whether the
/// breaker is currently tripped without performing the auto-reset write when its
/// duration has elapsed. Used by the simulation entrypoint (Issue #863) so a
/// dry-run never mutates storage.
fn market_circuit_breaker_active_readonly(env: &Env) -> bool {
    let active = env
        .storage()
        .instance()
        .get(&StorageKey::CircuitBreakerActive)
        .unwrap_or(false);
    if !active {
        return false;
    }

    let activated_ledger = env
        .storage()
        .instance()
        .get(&StorageKey::CircuitBreakerLedger)
        .unwrap_or(env.ledger().sequence());
    env.ledger().sequence().saturating_sub(activated_ledger) < CIRCUIT_BREAKER_DURATION_LEDGERS
}

fn open_interest_for_pair(env: &Env, pair: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::OpenInterestPerPair(pair.clone()))
        .unwrap_or(0)
}

fn check_open_interest_limit(env: &Env, pair: &Address, amount: i128) -> Result<(), ContractError> {
    let max_open_interest = env
        .storage()
        .instance()
        .get(&StorageKey::MaxOpenInterestPerPair)
        .unwrap_or(0);
    if max_open_interest <= 0 {
        return Ok(());
    }

    let current = open_interest_for_pair(env, pair);
    let next = current.checked_add(amount).unwrap_or(i128::MAX);
    if next > max_open_interest {
        return Err(ContractError::OpenInterestLimitReached);
    }
    Ok(())
}

fn increase_open_interest(env: &Env, pair: &Address, amount: i128) {
    let key = StorageKey::OpenInterestPerPair(pair.clone());
    let current = open_interest_for_pair(env, pair);
    let next = current.checked_add(amount).unwrap_or(i128::MAX);
    env.storage().instance().set(&key, &next);
}

fn decrease_open_interest(env: &Env, pair: &Address, amount: i128) {
    let key = StorageKey::OpenInterestPerPair(pair.clone());
    let current = open_interest_for_pair(env, pair);
    let next = current.saturating_sub(amount).max(0);
    env.storage().instance().set(&key, &next);
}

// ── Per-user open-position cap (Issue #791) ──────────────────────────────────

/// Default maximum concurrent open positions per user.
pub const DEFAULT_MAX_OPEN_POSITIONS: u32 = 50;

/// Effective per-user position cap (admin override, or [`DEFAULT_MAX_OPEN_POSITIONS`]).
fn effective_max_open_positions(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::MaxOpenPositions)
        .unwrap_or(DEFAULT_MAX_OPEN_POSITIONS)
}

fn user_position_count(env: &Env, user: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&StorageKey::UserPositionCount(user.clone()))
        .unwrap_or(0)
}

fn increment_user_position_count(env: &Env, user: &Address) {
    let key = StorageKey::UserPositionCount(user.clone());
    let next = user_position_count(env, user).saturating_add(1);
    env.storage().persistent().set(&key, &next);
}

/// Saturating decrement so the count can never go negative, even if a close
/// path runs for a position opened before this counter existed.
pub(crate) fn decrement_user_position_count(env: &Env, user: &Address) {
    let key = StorageKey::UserPositionCount(user.clone());
    let next = user_position_count(env, user).saturating_sub(1);
    env.storage().persistent().set(&key, &next);
}

/// Reject a new position when a non-exempt user is already at the cap.
fn check_position_cap(env: &Env, user: &Address, exempt: bool) -> Result<(), ContractError> {
    if exempt {
        return Ok(());
    }
    if user_position_count(env, user) >= effective_max_open_positions(env) {
        return Err(ContractError::TooManyOpenPositions);
    }
    Ok(())
}

/// Compute the SHA-256 trade receipt hash over `(user, asset, amount, price, timestamp)`.
pub fn compute_trade_hash(
    env: &Env,
    user: &Address,
    asset: &Address,
    amount: i128,
    price: i128,
    timestamp: u64,
) -> BytesN<32> {
    use soroban_sdk::xdr::ToXdr;
    let mut payload = soroban_sdk::Bytes::new(env);
    payload.append(&user.to_xdr(env));
    payload.append(&asset.to_xdr(env));
    payload.append(&soroban_sdk::Bytes::from_slice(env, &amount.to_be_bytes()));
    payload.append(&soroban_sdk::Bytes::from_slice(env, &price.to_be_bytes()));
    payload.append(&soroban_sdk::Bytes::from_slice(
        env,
        &timestamp.to_be_bytes(),
    ));
    env.crypto().sha256(&payload).into()
}

/// Store a SHA-256 receipt hash for a completed trade and emit a `trade_receipt` event.
/// Returns the monotonic receipt ID assigned to this trade.
fn record_trade_receipt(env: &Env, user: &Address, token: &Address, amount: i128) -> u64 {
    use soroban_sdk::xdr::ToXdr;
    let price: i128 = env
        .storage()
        .instance()
        .get(&StorageKey::SdexPrice(token.clone()))
        .unwrap_or(0);
    let timestamp = env.ledger().timestamp();

    let receipt_id: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::NextTradeReceiptId)
        .unwrap_or(1);

    // Include receipt_id so each receipt hash is unique even within the same ledger.
    let mut payload = soroban_sdk::Bytes::new(env);
    payload.append(&user.to_xdr(env));
    payload.append(&token.to_xdr(env));
    payload.append(&soroban_sdk::Bytes::from_slice(env, &amount.to_be_bytes()));
    payload.append(&soroban_sdk::Bytes::from_slice(env, &price.to_be_bytes()));
    payload.append(&soroban_sdk::Bytes::from_slice(
        env,
        &timestamp.to_be_bytes(),
    ));
    payload.append(&soroban_sdk::Bytes::from_slice(
        env,
        &receipt_id.to_be_bytes(),
    ));
    let hash: BytesN<32> = env.crypto().sha256(&payload).into();

    env.storage()
        .instance()
        .set(&StorageKey::TradeReceiptHash(receipt_id), &hash);
    env.storage().instance().set(
        &StorageKey::NextTradeReceiptId,
        &receipt_id.saturating_add(1),
    );

    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "trade_receipt"),
        ),
        (receipt_id, hash),
    );

    receipt_id
}

// ── Issue #863: read-only trade simulation ───────────────────────────────────

/// Per-check validation outcomes surfaced by [`simulate_market_copy_trade`].
/// Evaluated in the same order the corresponding gates run in
/// `execute_market_copy_trade`, so a caller can tell exactly which check(s)
/// would reject the trade.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationValidations {
    pub min_trade_size_ok: bool,
    pub circuit_breaker_ok: bool,
    pub open_interest_ok: bool,
    pub daily_volume_ok: bool,
    pub position_cap_ok: bool,
    pub position_pct_ok: bool,
    pub balance_ok: bool,
}

/// Result of a dry-run trade simulation. Read-only: computing it never writes
/// storage, requires no auth, and performs no cross-contract mutation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationResult {
    /// True only if every check in `validations` passed.
    pub would_succeed: bool,
    /// Effective trade amount after resolving `portfolio_pct_bps` (or the explicit amount
    /// unchanged when no percentage was requested).
    pub effective_amount: i128,
    /// Fee that would be charged in `token` for this trade.
    pub estimated_fee: i128,
    /// True when the user has enough balance for `effective_amount` alone but not for
    /// `effective_amount + estimated_fee`, mirroring the fee-fallback path taken by
    /// `execute_market_copy_trade` (the fee would be deducted from trade proceeds instead).
    pub fee_paid_from_received: bool,
    /// The user's current balance of `token`.
    pub available_balance: i128,
    /// The balance that would be required for the trade to succeed without the fallback.
    pub required_balance: i128,
    pub validations: SimulationValidations,
    /// The first failing check, in the same evaluation order as `execute_market_copy_trade`.
    /// `None` when `would_succeed` is true. Stores the `ContractError` discriminant as `u32`
    /// (cast from `ContractError as u32`) because `contracterror` enums cannot be embedded
    /// directly in a `contracttype` struct.
    pub failure_reason: Option<u32>,
}

/// Dry-run a market copy trade. Mirrors the validation gate order in
/// `execute_market_copy_trade` using only reads, so simulation and execution stay
/// in lock-step.
///
/// Not replayed here: the final per-user position-limit check against the
/// configured `UserPortfolio` contract (`validate_and_record`) is inherently
/// mutating on that contract (it atomically records the position alongside
/// validating it), so it cannot be dry-run from this contract. It is still
/// enforced for real at execution time.
fn simulate_market_copy_trade(
    env: &Env,
    user: Address,
    token: Address,
    amount: i128,
    portfolio_pct_bps: Option<u32>,
) -> SimulationResult {
    let mut failure_reason: Option<u32> = None;
    let fail = |reason: ContractError, failure_reason: &mut Option<u32>| {
        if failure_reason.is_none() {
            *failure_reason = Some(reason as u32);
        }
    };

    if amount <= 0 {
        fail(ContractError::InvalidAmount, &mut failure_reason);
    }

    let min_trade_size_ok = amount >= effective_min_trade_size(env, &token);
    if !min_trade_size_ok {
        fail(ContractError::BelowMinimumTradeSize, &mut failure_reason);
    }

    let circuit_breaker_ok = !market_circuit_breaker_active_readonly(env);
    if !circuit_breaker_ok {
        fail(ContractError::CircuitBreakerActive, &mut failure_reason);
    }

    let open_interest_ok = check_open_interest_limit(env, &token, amount).is_ok();
    if !open_interest_ok {
        fail(ContractError::OpenInterestLimitReached, &mut failure_reason);
    }

    let daily_limit: i128 = env
        .storage()
        .instance()
        .get(&StorageKey::DailyVolumeLimit)
        .unwrap_or(0i128);
    let daily_volume_ok = if daily_limit > 0 {
        let today: u64 = env.ledger().timestamp() / 86_400;
        let day_key = StorageKey::DailyVolumeDay(user.clone());
        let vol_key = StorageKey::DailyVolume(user.clone());
        let stored_day: u64 = env.storage().persistent().get(&day_key).unwrap_or(0u64);
        let current_vol: i128 = if stored_day == today {
            env.storage().persistent().get(&vol_key).unwrap_or(0i128)
        } else {
            0i128
        };
        current_vol.checked_add(amount).unwrap_or(i128::MAX) <= daily_limit
    } else {
        true
    };
    if !daily_volume_ok {
        fail(ContractError::DailyVolumeLimitExceeded, &mut failure_reason);
    }

    let exempt = {
        let key = StorageKey::PositionLimitExempt(user.clone());
        env.storage().instance().get(&key).unwrap_or(false)
    };
    let position_cap_ok = check_position_cap(env, &user, exempt).is_ok();
    if !position_cap_ok {
        fail(ContractError::TooManyOpenPositions, &mut failure_reason);
    }

    let position_pct_ok = portfolio_pct_bps
        .map(|pct| pct <= risk_gates::MAX_POSITION_PCT_BPS)
        .unwrap_or(true);
    if !position_pct_ok {
        fail(ContractError::PositionPctTooHigh, &mut failure_reason);
    }

    let oracle: Option<Address> = env.storage().instance().get(&Symbol::new(env, ORACLE_KEY));
    let effective_amount = if position_pct_ok {
        resolve_trade_amount(env, &user, &token, amount, portfolio_pct_bps, oracle)
            .unwrap_or(amount)
    } else {
        amount
    };

    let available_balance = soroban_sdk::token::Client::new(env, &token).balance(&user);
    let fee = effective_estimated_fee(env);
    let (balance_ok, fee_paid_from_received, required_balance) =
        match check_user_balance(env, &user, &token, effective_amount, fee) {
            Ok(()) => (true, false, effective_amount.saturating_add(fee)),
            Err(_) => match check_user_balance(env, &user, &token, effective_amount, 0) {
                Ok(()) => (true, true, effective_amount),
                Err(detail) => (false, false, detail.required),
            },
        };
    if !balance_ok {
        fail(ContractError::InsufficientBalance, &mut failure_reason);
    }

    SimulationResult {
        would_succeed: failure_reason.is_none(),
        effective_amount,
        estimated_fee: fee,
        fee_paid_from_received,
        available_balance,
        required_balance,
        validations: SimulationValidations {
            min_trade_size_ok,
            circuit_breaker_ok,
            open_interest_ok,
            daily_volume_ok,
            position_cap_ok,
            position_pct_ok,
            balance_ok,
        },
        failure_reason,
    }
}

fn execute_market_copy_trade(
    env: &Env,
    user: Address,
    token: Address,
    amount: i128,
    portfolio_pct_bps: Option<u32>,
    require_user_auth: bool,
    batch_ctx: Option<&BatchExecutionContext>,
) -> Result<(), ContractError> {
    if require_user_auth {
        user.require_auth();
    }

    if amount <= 0 {
        return Err(ContractError::InvalidAmount);
    }
    validate_min_trade_size(amount, effective_min_trade_size(env, &token))?;

    let cb_active = batch_ctx
        .map(|c| c.circuit_breaker_active)
        .unwrap_or_else(|| market_circuit_breaker_active(env));
    if cb_active {
        return Err(ContractError::CircuitBreakerActive);
    }
    check_open_interest_limit(env, &token, amount)?;

    // ── Reentrancy guard ──────────────────────────────────────────────────
    let lock_key = Symbol::new(env, EXECUTION_LOCK);
    if env
        .storage()
        .temporary()
        .get::<_, bool>(&lock_key)
        .unwrap_or(false)
    {
        return Err(ContractError::ReentrancyDetected);
    }
    env.storage().temporary().set(&lock_key, &true);

    // ── Daily volume limit check ───────────────────────────────────────────
    let limit = batch_ctx.map(|c| c.daily_limit).unwrap_or_else(|| {
        env.storage()
            .instance()
            .get(&StorageKey::DailyVolumeLimit)
            .unwrap_or(0i128)
    });
    if limit > 0 {
        let today: u64 = env.ledger().timestamp() / 86_400;
        let day_key = StorageKey::DailyVolumeDay(user.clone());
        let vol_key = StorageKey::DailyVolume(user.clone());
        let stored_day: u64 = env.storage().persistent().get(&day_key).unwrap_or(0u64);
        let current_vol: i128 = if stored_day == today {
            env.storage().persistent().get(&vol_key).unwrap_or(0i128)
        } else {
            0i128
        };
        let new_vol = current_vol.checked_add(amount).unwrap_or(i128::MAX);
        if new_vol > limit {
            env.storage().temporary().remove(&lock_key);
            return Err(ContractError::DailyVolumeLimitExceeded);
        }
        env.storage().persistent().set(&vol_key, &new_vol);
        env.storage().persistent().set(&day_key, &today);
    }

    // ── Read cached config from instance storage (no cross-contract call) ─
    let portfolio: Address = match batch_ctx.map(|c| c.portfolio.clone()) {
        Some(p) => p,
        None => match env.storage().instance().get(&StorageKey::UserPortfolio) {
            Some(portfolio) => portfolio,
            None => {
                env.storage().temporary().remove(&lock_key);
                return Err(ContractError::NotInitialized);
            }
        },
    };

    let exempt = {
        let key = StorageKey::PositionLimitExempt(user.clone());
        env.storage().instance().get(&key).unwrap_or(false)
    };

    // ── Per-user open-position cap (Issue #791) ────────────────────────────
    if let Err(e) = check_position_cap(env, &user, exempt) {
        env.storage().temporary().remove(&lock_key);
        return Err(e);
    }

    // ── Resolve effective amount (portfolio % or explicit) ─────────────────
    let oracle: Option<Address> = env.storage().instance().get(&Symbol::new(env, ORACLE_KEY));
    let effective_amount =
        match resolve_trade_amount(env, &user, &token, amount, portfolio_pct_bps, oracle) {
            Ok(a) => a,
            Err(e) => {
                env.storage().temporary().remove(&lock_key);
                return Err(e);
            }
        };

    // ── Cross-contract call #1: SEP-41 balance check ──────────────────────
    let fee = batch_ctx
        .map(|c| c.estimated_fee)
        .unwrap_or_else(|| effective_estimated_fee(env));
    let bal_key = StorageKey::LastInsufficientBalance(user.clone());
    let use_fee_fallback = match check_user_balance(env, &user, &token, effective_amount, fee) {
        Ok(()) => {
            env.storage().instance().remove(&bal_key);
            false
        }
        Err(detail) => {
            // Primary failed. Check if user has enough for just the amount (no fee).
            match check_user_balance(env, &user, &token, effective_amount, 0) {
                Ok(()) => {
                    // User has enough for the trade but not the fee — use fallback.
                    env.storage().instance().remove(&bal_key);
                    true
                }
                Err(_) => {
                    // User doesn't even have enough for the trade amount.
                    env.storage().instance().set(&bal_key, &detail);
                    env.storage().temporary().remove(&lock_key);
                    return Err(ContractError::InsufficientBalance);
                }
            }
        }
    };

    // ── Cross-contract call #2: batched position-limit check + record ─────
    if let Err(e) = validate_and_record_position(env, &portfolio, &user, exempt) {
        env.storage().temporary().remove(&lock_key);
        return Err(e);
    }

    increase_open_interest(env, &token, amount);
    increment_user_position_count(env, &user);

    // If fallback was used, emit the FeeDeductedFromReceived event.
    // The trade_id is the current position count (used as a proxy identifier).
    if use_fee_fallback && fee > 0 {
        // Use a monotonic counter stored per user as a trade_id proxy.
        let trade_id_key = StorageKey::FeeDeductedFromReceived(user.clone(), 0);
        let trade_id: u64 = env
            .storage()
            .instance()
            .get(&trade_id_key)
            .unwrap_or(0u64)
            .saturating_add(1);
        env.storage().instance().set(&trade_id_key, &trade_id);

        shared::events::emit_fee_deducted_from_received(
            env,
            shared::events::EvtFeeDeductedFromReceived {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                fee_amount: fee,
                trade_id,
            },
        );
    }

    let receipt_id = record_trade_receipt(env, &user, &token, effective_amount);

    let pending_order = wire::TradeOrder {
        execution_ledger: env.ledger().sequence(),
        trade_id: receipt_id,
        user: user.clone(),
        amount: effective_amount,
        expires_at_ledger: env
            .ledger()
            .sequence()
            .saturating_add(TRADE_TIMEOUT_LEDGERS),
        status: wire::TradeStatus::ExecutedAwaitingConfirmation,
    };
    env.storage()
        .instance()
        .set(&StorageKey::PendingTradeConfirmation, &pending_order);

    env.storage().temporary().remove(&lock_key);
    Ok(())
}

fn next_limit_order_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::NextLimitOrderId)
        .unwrap_or(1);
    let next = id.checked_add(1).expect("limit order id overflow");
    env.storage()
        .instance()
        .set(&StorageKey::NextLimitOrderId, &next);
    id
}

fn pending_order_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .instance()
        .get(&StorageKey::PendingLimitOrderIds)
        .unwrap_or_else(|| Vec::new(env))
}

fn store_pending_order(env: &Env, order: PendingLimitOrder) {
    let mut ids = pending_order_ids(env);
    ids.push_back(order.order_id);
    env.storage()
        .instance()
        .set(&StorageKey::PendingLimitOrderIds, &ids);
    env.storage()
        .instance()
        .set(&StorageKey::PendingLimitOrder(order.order_id), &order);
}

fn set_pending_order_ids(env: &Env, ids: &Vec<u64>) {
    env.storage()
        .instance()
        .set(&StorageKey::PendingLimitOrderIds, ids);
}

// ── Grace period helpers (Issue #702) ────────────────────────────────────────

/// Return the configured grace period in ledgers (defaults to [`DEFAULT_GRACE_PERIOD_LEDGERS`]).
fn effective_grace_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::TradeGracePeriod)
        .unwrap_or(DEFAULT_GRACE_PERIOD_LEDGERS)
}

/// Return the configured max retry count (defaults to [`DEFAULT_MAX_RETRY_COUNT`]).
fn effective_max_retry_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::MaxRetryCount)
        .unwrap_or(DEFAULT_MAX_RETRY_COUNT)
}

/// Move a failed trade to the dead-letter queue and emit the `trade_dead_lettered` event.
fn dead_letter_trade_internal(env: &Env, trade: &QueuedTrade, failure_code: u32) {
    let failed = FailedTrade {
        trade_id: trade.queued_trade_id,
        user: trade.user.clone(),
        token: trade.token.clone(),
        amount: trade.amount,
        portfolio_pct_bps: trade.portfolio_pct_bps,
        failure_code,
        retry_count: trade.retry_count.saturating_add(1),
        dead_lettered_at_ledger: env.ledger().sequence(),
    };
    env.storage()
        .instance()
        .set(&StorageKey::DeadLetterTrade(trade.queued_trade_id), &failed);

    let mut ids: Vec<u64> = env
        .storage()
        .instance()
        .get(&StorageKey::DeadLetterIds(trade.user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    ids.push_back(trade.queued_trade_id);
    env.storage()
        .instance()
        .set(&StorageKey::DeadLetterIds(trade.user.clone()), &ids);
    add_dead_letter_index(env, trade.queued_trade_id);

    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "trade_dead_lettered"),
        ),
        TradeDeadLettered {
            trade_id: trade.queued_trade_id,
            user: trade.user.clone(),
            failure_code,
            retry_count: trade.retry_count.saturating_add(1),
        },
    );
}

/// Return the configured dead-letter retention window in ledgers
/// (defaults to [`DEFAULT_DEAD_LETTER_RETENTION_LEDGERS`]).
fn effective_dead_letter_retention(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::DeadLetterRetentionLedgers)
        .unwrap_or(DEFAULT_DEAD_LETTER_RETENTION_LEDGERS)
}

/// Add `id` to the global index of all dead-lettered trade IDs.
fn add_dead_letter_index(env: &Env, id: u64) {
    let mut all: Vec<u64> = env
        .storage()
        .instance()
        .get(&StorageKey::DeadLetterAllIds)
        .unwrap_or_else(|| Vec::new(env));
    all.push_back(id);
    env.storage()
        .instance()
        .set(&StorageKey::DeadLetterAllIds, &all);
}

/// Remove `id` from the global index of all dead-lettered trade IDs.
fn remove_dead_letter_index(env: &Env, id: u64) {
    let mut all: Vec<u64> = env
        .storage()
        .instance()
        .get(&StorageKey::DeadLetterAllIds)
        .unwrap_or_else(|| Vec::new(env));
    let mut i = 0;
    while i < all.len() {
        if all.get(i).unwrap() == id {
            all.remove(i);
            break;
        }
        i += 1;
    }
    env.storage()
        .instance()
        .set(&StorageKey::DeadLetterAllIds, &all);
}

/// Remove `id` from `user`'s per-user dead-letter index.
fn remove_dead_letter_id(env: &Env, user: &Address, id: u64) {
    let mut ids: Vec<u64> = env
        .storage()
        .instance()
        .get(&StorageKey::DeadLetterIds(user.clone()))
        .unwrap_or_else(|| Vec::new(env));
    let mut i = 0;
    while i < ids.len() {
        if ids.get(i).unwrap() == id {
            ids.remove(i);
            break;
        }
        i += 1;
    }
    env.storage()
        .instance()
        .set(&StorageKey::DeadLetterIds(user.clone()), &ids);
}

/// Emit a `dead_letter_pruned` event for a single removed entry.
fn emit_dead_letter_pruned(env: &Env, user: Address, trade_id: u64) {
    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "dead_letter_pruned"),
        ),
        DeadLetterPruned {
            user,
            trade_id,
            reason: String::from_str(env, "retention_expired"),
        },
    );
}

/// Generate the next queued trade ID.
fn next_queued_trade_id(env: &Env) -> u64 {
    let next: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::NextQueuedTradeId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&StorageKey::NextQueuedTradeId, &next.saturating_add(1));
    next
}

/// Store a queued trade and add its ID to the index.
fn store_queued_trade(env: &Env, trade: &QueuedTrade) {
    let id = trade.queued_trade_id;
    env.storage()
        .instance()
        .set(&StorageKey::QueuedTrade(id), trade);
    let mut ids: Vec<u64> = env
        .storage()
        .instance()
        .get(&StorageKey::QueuedTradeIds)
        .unwrap_or_else(|| Vec::new(env));
    ids.push_back(id);
    env.storage()
        .instance()
        .set(&StorageKey::QueuedTradeIds, &ids);
}

/// Check whether the grace period has elapsed for a queued trade.
/// Returns `true` if the trade is eligible for execution.
fn grace_period_elapsed(env: &Env, queued_at_ledger: u32) -> bool {
    let grace = effective_grace_period(env);
    let current = env.ledger().sequence();
    current.saturating_sub(queued_at_ledger) >= grace
}

#[contractimpl]
impl TradeExecutorContract {
    pub fn expect_settlement_callback(
        env: Env,
        trade_id: u64,
        executor: Address,
        route: BytesN<32>,
        from_asset: Address,
        to_asset: Address,
    ) -> Result<(), ContractError> {
        require_admin(&env)?;
        let key = StorageKey::CallbackContext(trade_id);
        if env.storage().instance().has(&key) {
            return Err(ContractError::ReplayDetected);
        }
        env.storage().instance().set(
            &key,
            &CallbackContext {
                executor,
                route,
                from_asset,
                to_asset,
            },
        );
        Ok(())
    }

    pub fn accept_settlement_callback(
        env: Env,
        caller: Address,
        trade_id: u64,
        route: BytesN<32>,
        from_asset: Address,
        to_asset: Address,
    ) -> Result<(), ContractError> {
        caller.require_auth();
        let key = StorageKey::CallbackContext(trade_id);
        let expected: CallbackContext = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(ContractError::TradeNotFound)?;
        if expected.executor != caller
            || expected.route != route
            || expected.from_asset != from_asset
            || expected.to_asset != to_asset
        {
            return Err(ContractError::Unauthorized);
        }
        env.storage().instance().remove(&key);
        Ok(())
    }

    /// # Summary
    /// One-time contract initialization. Stores the admin address.
    ///
    /// # Parameters
    pub fn set_depth(env: Env, depth: u32) -> Result<(), ContractError> {
        require_admin(&env)?;
        set_confirmation_depth(&env, depth);
        Ok(())
    }

    /// Mark the pending trade order as `Filled` once the configured
    /// confirmation depth has elapsed since its execution ledger.
    ///
    /// Note: finalization confirms the fill — the position remains open (and
    /// counted against the per-user cap) until it is closed via
    /// [`Self::cancel_copy_trade`] or a stop-loss/take-profit trigger.
    pub fn finalize_trade(env: Env) -> Result<(), ContractError> {
        let mut order: wire::TradeOrder = env
            .storage()
            .instance()
            .get(&StorageKey::PendingTradeConfirmation)
            .ok_or(ContractError::TradeNotFound)?;

        if order.status != wire::TradeStatus::ExecutedAwaitingConfirmation {
            return Err(ContractError::Unauthorized);
        }

        let depth = get_confirmation_depth(&env);
        let current_ledger = env.ledger().sequence();

        if current_ledger.saturating_sub(order.execution_ledger) < depth {
            return Err(ContractError::ConfirmationDepthNotReached);
        }

        order.status = wire::TradeStatus::Filled;
        env.storage()
            .instance()
            .set(&StorageKey::PendingTradeConfirmation, &order);

        Ok(())
    }
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

    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    ///
    /// # Returns
    /// Nothing. Panics if already initialized.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    // ── Issue #865: governance-driven pause propagation ────────────────────────

    /// Set the central governance contract address authorized to call
    /// `apply_governance_pause`. Admin only.
    pub fn set_governance(env: Env, governance: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&StorageKey::GovernanceAddress, &governance);
        Ok(())
    }

    /// Read-only: the configured governance contract address, if any.
    pub fn get_governance(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::GovernanceAddress)
    }

    /// Called by the configured governance contract to propagate a pause/unpause.
    pub fn apply_governance_pause(env: Env, paused: bool) -> Result<(), ContractError> {
        let governance: Address = env
            .storage()
            .instance()
            .get(&StorageKey::GovernanceAddress)
            .ok_or(ContractError::Unauthorized)?;
        governance.require_auth();
        env.storage().instance().set(&StorageKey::Paused, &paused);
        Ok(())
    }

    /// Read-only: true when a governance-driven emergency pause is active.
    pub fn is_paused(env: Env) -> bool {
        is_paused(&env)
    }

    /// Read-only health probe for monitoring and front-ends (no auth).
    pub fn health_check(env: Env) -> stellar_swipe_common::HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        let admin: Option<Address> = env.storage().instance().get(&StorageKey::Admin);
        let Some(admin) = admin else {
            return stellar_swipe_common::health_uninitialized(&env, version);
        };
        let status = stellar_swipe_common::HealthStatus {
            is_initialized: true,
            is_paused: is_paused(&env),
            version,
            admin,
            initialized_at: env.ledger().timestamp(),
        };
        stellar_swipe_common::emit_health_event(&env, &status);
        status
    }

    /// # Summary
    /// Configure the portfolio contract used for position validation and
    /// copy-trade recording. Admin auth required.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `portfolio`: Address of the UserPortfolio contract.
    ///
    /// # Returns
    /// Nothing. Panics if not initialized.
    pub fn set_user_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::UserPortfolio, &portfolio);
    }

    pub fn get_user_portfolio(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::UserPortfolio)
    }

    /// Set the fee term used in `amount + estimated_fee` balance checks (admin).
    pub fn set_copy_trade_estimated_fee(env: Env, fee: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if fee < 0 {
            panic!("fee must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::CopyTradeEstimatedFee, &fee);
    }

    pub fn get_copy_trade_estimated_fee(env: Env) -> i128 {
        effective_estimated_fee(&env)
    }

    /// Admin: set the minimum trade size for `token` (dust-amount griefing guard).
    /// Trades/copy-trades below this amount are rejected before any state changes.
    pub fn set_min_trade_size(env: Env, token: Address, minimum: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if minimum < 0 {
            panic!("minimum must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::MinTradeSize(token), &minimum);
    }

    /// Effective minimum trade size for `token` (override, or [`DEFAULT_MIN_TRADE_SIZE`]).
    pub fn get_min_trade_size(env: Env, token: Address) -> i128 {
        effective_min_trade_size(&env, &token)
    }

    /// Admin override: exempt `user` from the per-user position cap (or clear exemption).
    pub fn set_position_limit_exempt(env: Env, user: Address, exempt: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        let key = StorageKey::PositionLimitExempt(user);
        if exempt {
            env.storage().instance().set(&key, &true);
        } else {
            env.storage().instance().remove(&key);
        }
    }

    pub fn is_position_limit_exempt(env: Env, user: Address) -> bool {
        let key = StorageKey::PositionLimitExempt(user);
        env.storage().instance().get(&key).unwrap_or(false)
    }

    /// Admin: set the maximum concurrent open positions per user (Issue #791).
    /// Exempt users (see [`Self::set_position_limit_exempt`]) bypass this cap.
    pub fn set_max_open_positions(env: Env, max: u32) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if max == 0 {
            panic!("max must be positive");
        }
        env.storage()
            .instance()
            .set(&StorageKey::MaxOpenPositions, &max);
    }

    /// Effective per-user position cap ([`DEFAULT_MAX_OPEN_POSITIONS`] unless overridden).
    pub fn get_max_open_positions(env: Env) -> u32 {
        effective_max_open_positions(&env)
    }

    /// Number of open copy-trade positions currently counted for `user`.
    pub fn get_user_position_count(env: Env, user: Address) -> u32 {
        user_position_count(&env, &user)
    }

    // ── Stop-loss / take-profit configuration ─────────────────────────────────

    pub fn add_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        oracle::add(&env, oracle)
    }

    pub fn remove_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        oracle::remove(&env, oracle)
    }

    pub fn is_oracle_whitelisted(env: Env, oracle: Address) -> bool {
        oracle::is_whitelisted(&env, &oracle)
    }

    pub fn get_oracle_whitelist_count(env: Env) -> u32 {
        oracle::count(&env)
    }

    /// Set the oracle contract used by stop-loss/take-profit checks (admin only).
    pub fn set_oracle(env: Env, oracle: Address) -> Result<(), ContractError> {
        require_admin(&env)?;
        oracle::require_whitelisted(&env, &oracle)?;
        env.storage()
            .instance()
            .set(&Symbol::new(&env, ORACLE_KEY), &oracle);
        Ok(())
    }

    pub fn get_oracle(env: Env) -> Option<Address> {
        env.storage().instance().get(&Symbol::new(&env, ORACLE_KEY))
    }

    /// Set the portfolio contract used by stop-loss/take-profit close calls (admin only).
    pub fn set_stop_loss_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PORTFOLIO_KEY), &portfolio);
    }

    /// Register a stop-loss price for `(user, trade_id)`.
    pub fn set_stop_loss_price(env: Env, user: Address, trade_id: u64, stop_loss_price: i128) {
        user.require_auth();
        triggers::set_stop_loss(&env, &user, trade_id, stop_loss_price);
    }

    /// Check oracle price and trigger stop-loss if breached. Returns `true` when triggered.
    pub fn check_and_trigger_stop_loss(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_stop_loss(&env, user, trade_id, asset_pair)
    }

    /// Register a trailing stop for `(user, trade_id)`.
    /// `trail_bps`: distance from peak in basis points (e.g. 500 = 5%).
    /// `initial_price`: entry price used to seed the peak tracker.
    pub fn set_trailing_stop(
        env: Env,
        user: Address,
        trade_id: u64,
        trail_bps: u32,
        initial_price: i128,
    ) {
        user.require_auth();
        triggers::set_trailing_stop(&env, &user, trade_id, trail_bps, initial_price);
    }

    /// Keeper: update trailing peak and trigger if price has dropped `trail_bps` below peak.
    pub fn check_and_trigger_trailing_stop(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_trailing_stop(&env, user, trade_id, asset_pair)
    }

    /// Register a take-profit price for `(user, trade_id)`.
    pub fn set_take_profit_price(env: Env, user: Address, trade_id: u64, take_profit_price: i128) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
    }

    pub fn set_take_profit_price_with_pair(
        env: Env,
        user: Address,
        trade_id: u64,
        take_profit_price: i128,
        asset_pair: u32,
    ) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
        keeper::register_watch(&env, &user, trade_id, asset_pair);
    }

    pub fn check_and_trigger_take_profit(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_take_profit(&env, user, trade_id, asset_pair)
    }

    /// Structured shortfall after the last `InsufficientBalance` from [`Self::execute_copy_trade`].
    pub fn get_insufficient_balance_detail(
        env: Env,
        user: Address,
    ) -> Option<InsufficientBalanceDetail> {
        let key = StorageKey::LastInsufficientBalance(user);
        env.storage().instance().get(&key)
    }

    /// Activate the protocol-wide market circuit breaker. The admin or a whitelisted
    /// oracle may activate it during extreme volatility.
    pub fn activate_market_circuit_breaker(env: Env, caller: Address) -> Result<(), ContractError> {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(ContractError::NotInitialized)?;
        if caller != admin && !oracle::is_whitelisted(&env, &caller) {
            return Err(ContractError::Unauthorized);
        }

        let ledger = env.ledger().sequence();
        env.storage()
            .instance()
            .set(&StorageKey::CircuitBreakerActive, &true);
        env.storage()
            .instance()
            .set(&StorageKey::CircuitBreakerLedger, &ledger);
        emit_circuit_breaker_activated(&env, caller, ledger);
        Ok(())
    }

    /// Admin reset hook; normal trade flow also auto-resets after the configured duration.
    pub fn reset_market_circuit_breaker(env: Env) -> Result<(), ContractError> {
        require_admin(&env)?;
        if market_circuit_breaker_active(&env) {
            reset_circuit_breaker(&env);
        }
        Ok(())
    }

    pub fn is_market_circuit_breaker_active(env: Env) -> bool {
        market_circuit_breaker_active(&env)
    }

    /// Execute a copy trade.
    ///
    /// Accepts replay-protection parameters (`nonce`, `tx_hash`, `expiry_ts`) so that
    /// the caller can provide a strictly increasing nonce and a unique transaction hash
    /// to prevent replay attacks.
    ///
    /// ## Cross-contract call budget (Issue #306 optimization)
    /// | # | Callee            | Purpose                                      |
    /// |---|-------------------|----------------------------------------------|
    /// | 1 | SEP-41 token SAC  | Balance check (`token.balance(user)`)        |
    /// | 2 | UserPortfolio     | `validate_and_record(user, max_positions)`   |
    ///
    /// Previously 3 calls (balance + get_open_position_count + record_copy_position).
    /// Now 2 calls — calls #2 and #3 are batched into a single portfolio entrypoint.
    pub fn execute_copy_trade(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
        portfolio_pct_bps: Option<u32>,
        order_type: OrderType,
        limit_price: Option<i128>,
        nonce: u64,
        tx_hash: Bytes,
        expiry_ts: u64,
    ) -> Result<(), ContractError> {
        if is_paused(&env) {
            return Err(ContractError::ContractPaused);
        }
        verify_and_commit(&env, &user, nonce, tx_hash, expiry_ts).map_err(map_replay_error)?;
        feature_flags::require_feature_enabled(&env, feature_flags::FEAT_COPY_TRADE)?;
        match order_type {
            OrderType::Market => {
                execute_market_copy_trade(&env, user, token, amount, portfolio_pct_bps, true, None)
            }
            OrderType::Limit => {
                user.require_auth();
                if amount <= 0 {
                    return Err(ContractError::InvalidAmount);
                }
                let price = limit_price.ok_or(ContractError::InvalidAmount)?;
                if price <= 0 {
                    return Err(ContractError::InvalidAmount);
                }
                validate_min_trade_size(amount, effective_min_trade_size(&env, &token))?;

                let fee = effective_estimated_fee(&env);
                let bal_key = StorageKey::LastInsufficientBalance(user.clone());
                match check_user_balance(&env, &user, &token, amount, fee) {
                    Ok(()) => env.storage().instance().remove(&bal_key),
                    Err(detail) => {
                        env.storage().instance().set(&bal_key, &detail);
                        return Err(ContractError::InsufficientBalance);
                    }
                }

                let order_id = next_limit_order_id(&env);
                let expires_at_ledger = env
                    .ledger()
                    .sequence()
                    .saturating_add(TRADE_TIMEOUT_LEDGERS);
                store_pending_order(
                    &env,
                    PendingLimitOrder {
                        order_id,
                        user,
                        token,
                        amount,
                        portfolio_pct_bps,
                        limit_price: price,
                        expires_at_ledger,
                    },
                );
                Ok(())
            }
        }
    }

    /// Dry-run a market copy trade without submitting it. Returns the expected
    /// effective amount, fee, and a breakdown of which validation gates would
    /// pass or fail — without requiring auth, writing any storage, or mutating
    /// any cross-contract state. Issue #863.
    ///
    /// Runs the same checks, in the same order, as the `OrderType::Market` path
    /// of [`Self::execute_copy_trade`] (excluding the final mutating position-limit
    /// call to the `UserPortfolio` contract, which cannot be dry-run — see
    /// [`SimulationResult`]).
    pub fn simulate_copy_trade(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
        portfolio_pct_bps: Option<u32>,
    ) -> SimulationResult {
        simulate_market_copy_trade(&env, user, token, amount, portfolio_pct_bps)
    }

    /// Admin/keeper maintenance call: reclaim persistent storage held by
    /// replay-protection tx-hash entries that are past their `expiry_ts`.
    ///
    /// Scans at most `max` entries from the front of the internal purge queue and
    /// removes the ones that are objectively expired, so it can never break the
    /// replay guarantee for a still-valid nonce/tx_hash. Returns the number of
    /// entries removed. See the nonce replay attack prevention audit.
    pub fn purge_expired_nonces(env: Env, max: u32) -> Result<u32, ContractError> {
        require_admin(&env)?;
        Ok(replay_purge_expired_nonces(&env, max))
    }

    // ── SDEX router configuration ─────────────────────────────────────────────

    /// Set the router contract invoked by [`sdex::execute_sdex_swap`].
    pub fn set_sdex_router(env: Env, router: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::SdexRouter, &router);
    }

    pub fn get_sdex_router(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::SdexRouter)
    }

    // ── Asset registry configuration (Issue #992) ────────────────────────────

    /// Set the asset registry contract used to validate asset pairs before any
    /// swap or token operation. Admin only.
    ///
    /// Once a registry is configured, [`Self::swap`], [`Self::swap_with_slippage`]
    /// and [`Self::cancel_copy_trade`] reject pairs whose assets are not both
    /// registered in the registry, pairs whose assets are identical, and pairs
    /// the configured SDEX route does not support — before any external call or
    /// state mutation. See `docs/asset_pair_validation.md` for the supported-pair
    /// source and update authority.
    pub fn set_asset_registry(env: Env, registry: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::AssetRegistry, &registry);
    }

    /// The configured asset registry contract, if any.
    pub fn get_asset_registry(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::AssetRegistry)
    }

    /// Admin/keeper-facing price cache used to decide when pending limit orders
    /// are executable against the configured SDEX route.
    pub fn set_sdex_price(env: Env, token: Address, price: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if price <= 0 {
            panic!("price must be positive");
        }
        env.storage()
            .instance()
            .set(&StorageKey::SdexPrice(token), &price);
    }

    pub fn get_sdex_price(env: Env, token: Address) -> Option<i128> {
        env.storage().instance().get(&StorageKey::SdexPrice(token))
    }

    pub fn get_pending_limit_order(env: Env, order_id: u64) -> Option<PendingLimitOrder> {
        env.storage()
            .instance()
            .get(&StorageKey::PendingLimitOrder(order_id))
    }

    pub fn get_pending_limit_order_ids(env: Env) -> Vec<u64> {
        pending_order_ids(&env)
    }

    /// Keeper-facing sweep for pending limit orders on `token`.
    ///
    /// Orders expire after `TRADE_TIMEOUT_LEDGERS`. Executable orders run through
    /// the same market-trade path as immediate copy trades without requiring a
    /// fresh user signature, because the user authorized the limit order placement.
    pub fn check_pending_limit_orders(env: Env, token: Address) -> Result<u32, ContractError> {
        let current_price: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::SdexPrice(token.clone()))
            .ok_or(ContractError::OracleUnavailable)?;
        let ids = pending_order_ids(&env);
        let mut next_ids = Vec::new(&env);
        let mut processed = 0u32;

        for i in 0..ids.len() {
            let Some(order_id) = ids.get(i) else {
                continue;
            };
            let Some(order) = env
                .storage()
                .instance()
                .get::<StorageKey, PendingLimitOrder>(&StorageKey::PendingLimitOrder(order_id))
            else {
                continue;
            };

            if order.token != token {
                next_ids.push_back(order_id);
                continue;
            }

            if env.ledger().sequence() >= order.expires_at_ledger {
                env.storage()
                    .instance()
                    .remove(&StorageKey::PendingLimitOrder(order_id));
                processed = processed.saturating_add(1);
                continue;
            }

            if current_price <= order.limit_price {
                execute_market_copy_trade(
                    &env,
                    order.user,
                    order.token,
                    order.amount,
                    order.portfolio_pct_bps,
                    false,
                    None,
                )?;
                env.storage()
                    .instance()
                    .remove(&StorageKey::PendingLimitOrder(order_id));
                processed = processed.saturating_add(1);
            } else {
                next_ids.push_back(order_id);
            }
        }

        set_pending_order_ids(&env, &next_ids);
        Ok(processed)
    }

    /// Admin: set the global daily trade volume limit (USD-equivalent units).
    /// `0` means no limit.
    pub fn set_daily_volume_limit(env: Env, limit: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if limit < 0 {
            panic!("limit must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::DailyVolumeLimit, &limit);
    }

    pub fn get_daily_volume_limit(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::DailyVolumeLimit)
            .unwrap_or(0i128)
    }

    /// Admin: set the per-pair open interest limit. `0` means no limit.
    pub fn set_max_open_interest_per_pair(env: Env, limit: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if limit < 0 {
            panic!("limit must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::MaxOpenInterestPerPair, &limit);
    }

    pub fn get_max_open_interest_per_pair(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::MaxOpenInterestPerPair)
            .unwrap_or(0i128)
    }

    pub fn get_open_interest(env: Env, pair: Address) -> i128 {
        open_interest_for_pair(&env, &pair)
    }

    // ── Trade grace period (Issue #702) ─────────────────────────────────────────

    /// Admin: set the grace period (in ledger sequences) that a queued trade must
    /// wait before it becomes eligible for execution. Default is 10 ledgers.
    /// Pass `0` to disable the grace period (trades execute immediately).
    pub fn set_trade_grace_period(env: Env, ledgers: u32) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::TradeGracePeriod, &ledgers);
    }

    /// Return the configured grace period in ledgers.
    pub fn get_trade_grace_period(env: Env) -> u32 {
        effective_grace_period(&env)
    }

    /// Queue a copy trade for execution. Instead of executing immediately, the
    /// trade is recorded in storage and will only be picked up by `batch_execute`
    /// or `execute_queued_trades` after the grace period has elapsed.
    ///
    /// Returns the assigned queued trade ID which can be used with
    /// `cancel_queued_trade` to cancel within the grace period.
    pub fn queue_copy_trade(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
        portfolio_pct_bps: Option<u32>,
    ) -> Result<u64, ContractError> {
        user.require_auth();

        // Fail fast if the user is already at the position cap (Issue #791).
        // The cap is re-checked at execution time, when queued positions may
        // have been freed by intervening closes.
        let exempt = env
            .storage()
            .instance()
            .get(&StorageKey::PositionLimitExempt(user.clone()))
            .unwrap_or(false);
        check_position_cap(&env, &user, exempt)?;

        let queued_trade_id = next_queued_trade_id(&env);
        let trade = QueuedTrade {
            queued_trade_id,
            user: user.clone(),
            token,
            amount,
            queued_at_ledger: env.ledger().sequence(),
            portfolio_pct_bps,
            retry_count: 0,
        };
        store_queued_trade(&env, &trade);
        Ok(queued_trade_id)
    }

    /// Cancel a queued trade by its ID. Only the original user may cancel,
    /// and only within the grace period (before the configured number of ledgers
    /// has elapsed since queuing).
    pub fn cancel_queued_trade(
        env: Env,
        user: Address,
        queued_trade_id: u64,
    ) -> Result<(), ContractError> {
        user.require_auth();
        let trade: QueuedTrade = env
            .storage()
            .instance()
            .get(&StorageKey::QueuedTrade(queued_trade_id))
            .ok_or(ContractError::QueuedTradeNotFound)?;

        if trade.user != user {
            return Err(ContractError::NotTradeOwner);
        }

        if grace_period_elapsed(&env, trade.queued_at_ledger) {
            return Err(ContractError::GracePeriodExpired);
        }

        // Remove the queued trade from storage.
        env.storage()
            .instance()
            .remove(&StorageKey::QueuedTrade(queued_trade_id));

        // Remove from the index list.
        let mut ids: Vec<u64> = env
            .storage()
            .instance()
            .get(&StorageKey::QueuedTradeIds)
            .unwrap_or_else(|| Vec::new(&env));
        let mut i = 0;
        while i < ids.len() {
            if ids.get(i).unwrap() == queued_trade_id {
                ids.remove(i);
                break;
            }
            i += 1;
        }
        env.storage()
            .instance()
            .set(&StorageKey::QueuedTradeIds, &ids);

        Ok(())
    }

    /// Execute all queued trades whose grace period has elapsed.
    /// Returns the number of trades successfully executed.
    pub fn execute_queued_trades(env: Env) -> Result<u32, ContractError> {
        let ids: Vec<u64> = env
            .storage()
            .instance()
            .get(&StorageKey::QueuedTradeIds)
            .unwrap_or_else(|| Vec::new(&env));

        let mut remaining: Vec<u64> = Vec::new(&env);
        let mut executed: u32 = 0;

        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            let trade: QueuedTrade =
                match env.storage().instance().get(&StorageKey::QueuedTrade(id)) {
                    Some(t) => t,
                    None => continue,
                };

            if grace_period_elapsed(&env, trade.queued_at_ledger) {
                let result = execute_market_copy_trade(
                    &env,
                    trade.user.clone(),
                    trade.token.clone(),
                    trade.amount,
                    trade.portfolio_pct_bps,
                    false,
                    None,
                );
                match result {
                    Ok(()) => {
                        executed = executed.saturating_add(1);
                        env.storage()
                            .instance()
                            .remove(&StorageKey::QueuedTrade(id));
                    }
                    Err(e) => {
                        let max_retries = effective_max_retry_count(&env);
                        let new_retry_count = trade.retry_count.saturating_add(1);
                        if new_retry_count >= max_retries {
                            dead_letter_trade_internal(&env, &trade, e as u32);
                            env.storage()
                                .instance()
                                .remove(&StorageKey::QueuedTrade(id));
                        } else {
                            let updated = QueuedTrade {
                                retry_count: new_retry_count,
                                queued_trade_id: trade.queued_trade_id,
                                user: trade.user,
                                token: trade.token,
                                amount: trade.amount,
                                queued_at_ledger: trade.queued_at_ledger,
                                portfolio_pct_bps: trade.portfolio_pct_bps,
                            };
                            env.storage()
                                .instance()
                                .set(&StorageKey::QueuedTrade(id), &updated);
                            remaining.push_back(id);
                        }
                    }
                }
            } else {
                remaining.push_back(id);
            }
        }

        env.storage()
            .instance()
            .set(&StorageKey::QueuedTradeIds, &remaining);
        Ok(executed)
    }

    /// # Summary
    /// Execute a swap via the configured SDEX router with an explicit minimum
    /// received amount. Enforces slippage at the balance-delta level.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `from_token`: SEP-41 token to sell.
    /// - `to_token`: SEP-41 token to buy.
    /// - `amount`: Amount of `from_token` to sell (must be > 0).
    /// - `min_received`: Minimum acceptable amount of `to_token` (must be >= 0).
    ///
    /// # Returns
    /// Actual amount of `to_token` received.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — SDEX router not configured.
    /// - [`ContractError::InvalidAmount`] — amount <= 0 or min_received < 0.
    /// - [`ContractError::SlippageExceeded`] — actual received < min_received.
    /// - [`ContractError::AssetNotRegistered`] — an asset is not in the configured registry.
    /// - [`ContractError::IdenticalAssets`] — both tokens are the same asset.
    /// - [`ContractError::UnsupportedPair`] — the configured route does not support the pair.
    ///
    /// # Example
    /// ```rust,ignore
    /// client.swap(&xlm_token, &usdc_token, &1_000_0000000i128, &990_0000000i128);
    /// ```
    pub fn swap(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
    ) -> Result<i128, ContractError> {
        let router = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;
        let registry: Option<Address> = env.storage().instance().get(&StorageKey::AssetRegistry);
        validate_pair_before_swap(&env, registry.as_ref(), &router, &from_token, &to_token)?;
        execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)
    }

    /// # Summary
    /// Execute a swap with automatic slippage protection. Computes
    /// `min_received = amount * (10_000 - max_slippage_bps) / 10_000`
    /// and delegates to [`Self::swap`].
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `from_token`: SEP-41 token to sell.
    /// - `to_token`: SEP-41 token to buy.
    /// - `amount`: Amount of `from_token` to sell.
    /// - `max_slippage_bps`: Maximum acceptable slippage in basis points (e.g. `100` = 1%).
    ///
    /// # Returns
    /// Actual amount of `to_token` received.
    ///
    /// # Errors
    /// - [`ContractError::InvalidAmount`] — amount <= 0 or slippage calculation overflows.
    /// - [`ContractError::NotInitialized`] — SDEX router not configured.
    /// - [`ContractError::SlippageExceeded`] — actual received < computed min_received.
    pub fn swap_with_slippage(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        max_slippage_bps: u32,
    ) -> Result<i128, ContractError> {
        let min_received = min_received_from_slippage(amount, max_slippage_bps)
            .ok_or(ContractError::InvalidAmount)?;
        Self::swap(env, from_token, to_token, amount, min_received)
    }

    // ── Manual position exit ──────────────────────────────────────────────────

    /// Cancel a copy trade manually: executes a SDEX swap to close the position,
    /// records exit in UserPortfolio, and emits `TradeCancelled`.
    ///
    /// `entry_price` is the per-unit price of `from_token` in `to_token` terms at
    /// entry (scaled by [`ENTRY_PRICE_DENOMINATOR`]).  
    /// Realized P&L = `exit_price - (amount × entry_price / ENTRY_PRICE_DENOMINATOR)`,
    /// which expresses both terms in `to_token` units.
    ///
    /// Replay-protection parameters (`nonce`, `tx_hash`, `expiry_ts`) are verified
    /// via [`verify_and_commit`] before the swap executes.
    pub fn cancel_copy_trade(
        env: Env,
        caller: Address,
        user: Address,
        trade_id: u64,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
        entry_price: i128,
        replay: ReplayParams,
    ) -> Result<(), ContractError> {
        verify_and_commit(&env, &user, replay.nonce, replay.tx_hash, replay.expiry_ts)
            .map_err(map_replay_error)?;
        caller.require_auth();
        if caller != user {
            return Err(ContractError::Unauthorized);
        }

        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)
            .ok_or(ContractError::NotInitialized)?;

        let exists: bool = {
            let sym = Symbol::new(&env, "has_position");
            let mut args = Vec::<Val>::new(&env);
            args.push_back(user.clone().into_val(&env));
            args.push_back(trade_id.into_val(&env));
            env.invoke_contract(&portfolio, &sym, args)
        };
        if !exists {
            return Err(ContractError::TradeNotFound);
        }

        let router: Address = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;

        let registry: Option<Address> = env.storage().instance().get(&StorageKey::AssetRegistry);
        validate_pair_before_swap(&env, registry.as_ref(), &router, &from_token, &to_token)?;

        let exit_price =
            execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)?;

        // Convert the entry-position value to `to_token` units so that both
        // `exit_price` and the entry value are expressed in the same asset unit.
        // entry_price is in 7-decimal fixed-point, so amount × entry_price has
        // 14 implicit decimals; normalize back to 7 via the shared utility.
        let entry_value = {
            let product = amount
                .checked_mul(entry_price)
                .ok_or(ContractError::InvalidAmount)?;
            normalize_amount(product, 14, 7).ok_or(ContractError::InvalidAmount)?
        };
        let realized_pnl = exit_price - entry_value;
        let close_sym = Symbol::new(&env, "close_position");
        let mut close_args = Vec::<Val>::new(&env);
        close_args.push_back(user.clone().into_val(&env));
        close_args.push_back(trade_id.into_val(&env));
        close_args.push_back(realized_pnl.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &close_sym, close_args);
        decrease_open_interest(&env, &from_token, amount);
        decrement_user_position_count(&env, &user);

        shared::events::emit_trade_cancelled(
            &env,
            shared::events::EvtTradeCancelled {
                schema_version: shared::events::SCHEMA_VERSION,
                user: user.clone(),
                trade_id,
                exit_price,
                realized_pnl,
            },
        );

        Ok(())
    }

    /// Execute a batch of copy trades in best-effort mode. Each trade is
    /// attempted independently; a failure in one trade does NOT roll back
    /// successful trades.
    ///
    /// Trades are processed in priority-tier order (high-stake -> high-tenure -> standard)
    /// when a portfolio contract is configured (Issue #682). A fairness fallback
    /// prevents starvation of standard followers after N consecutive priority-only batches.
    ///
    /// Returns a Vec<BatchTradeResult> with one entry per input trade, in the
    /// priority-sorted order (not the original input order).
    ///
    /// Kept as a frozen, unchanged entry point (equivalent to
    /// `batch_execute_atomic(env, trades, false)`) so existing callers who
    /// already integrated against this exact signature are unaffected by
    /// atomic mode (Issue #793) — see `batch_execute_atomic` for that.
    ///
    /// # Errors
    /// - [ContractError::InvalidAmount] - batch is empty or exceeds MAX_BATCH_SIZE.
    pub fn batch_execute(
        env: Env,
        trades: Vec<BatchTradeInput>,
    ) -> Result<Vec<BatchTradeResult>, ContractError> {
        Self::batch_execute_impl(&env, trades, false)
    }

    /// Execute a batch of copy trades, optionally with all-or-nothing atomic
    /// semantics (Issue #793).
    ///
    /// - `atomic = false`: identical behavior to [`Self::batch_execute`] —
    ///   each trade is attempted independently and a failure does not affect
    ///   the others.
    /// - `atomic = true`: if any trade in the batch fails, the entire batch
    ///   is rolled back — including trades that individually succeeded
    ///   earlier in the same call — by panicking. Soroban reverts every
    ///   storage effect from a panicking top-level invocation, so this
    ///   relies on the host's own transaction-level atomicity rather than
    ///   any manual undo bookkeeping. Because a panic aborts the call
    ///   entirely, this function never actually *returns* a
    ///   `Vec<BatchTradeResult>` to the caller on the failing path — the
    ///   caller instead observes the whole invocation failing (e.g. a
    ///   `try_*` client call returning an error, or the transaction itself
    ///   failing on submission). It still computes and marks
    ///   `atomic_rollback: true` on every entry before panicking, purely so
    ///   that marking logic has a testable, correct implementation (see
    ///   `mark_atomic_rollback` and its unit test) independent of testing
    ///   the panic/rollback behavior itself.
    ///
    /// # Errors
    /// - [ContractError::InvalidAmount] - batch is empty or exceeds MAX_BATCH_SIZE.
    ///
    /// # Panics
    /// - If `atomic == true` and any trade in `trades` fails.
    pub fn batch_execute_atomic(
        env: Env,
        trades: Vec<BatchTradeInput>,
        atomic: bool,
    ) -> Result<Vec<BatchTradeResult>, ContractError> {
        Self::batch_execute_impl(&env, trades, atomic)
    }

    fn batch_execute_impl(
        env: &Env,
        trades: Vec<BatchTradeInput>,
        atomic: bool,
    ) -> Result<Vec<BatchTradeResult>, ContractError> {
        let len = trades.len();
        if len == 0 || len > MAX_BATCH_SIZE {
            return Err(ContractError::InvalidAmount);
        }

        // ── #993: Reject duplicate user+token entries deterministically ────
        // This prevents the same trade from appearing twice in one batch,
        // which would double-count volume and position changes.
        {
            let mut seen: soroban_sdk::Map<soroban_sdk::BytesN<32>, bool> =
                soroban_sdk::Map::new(env);
            for i in 0..len {
                let trade = trades.get(i).unwrap();
                use soroban_sdk::xdr::ToXdr;
                let mut payload = soroban_sdk::Bytes::new(env);
                payload.append(&trade.user.to_xdr(env));
                payload.append(&trade.token.to_xdr(env));
                let key_hash: soroban_sdk::BytesN<32> = env.crypto().sha256(&payload).into();
                if let Some(true) = seen.get(key_hash.clone()) {
                    return Err(ContractError::InvalidAmount);
                }
                seen.set(key_hash, true);
            }
        }

        let batch_ctx = prepare_batch_context(env)?;
        let mut results: Vec<BatchTradeResult> = Vec::new(env);

        for i in 0..len {
            let trade = trades.get(i).unwrap();
            let outcome = execute_market_copy_trade(
                env,
                trade.user.clone(),
                trade.token.clone(),
                trade.amount,
                None,
                true,
                Some(&batch_ctx),
            );
            let result = match outcome {
                Ok(()) => BatchTradeResult {
                    ok: true,
                    error_code: 0,
                    atomic_rollback: false,
                },
                Err(e) => BatchTradeResult {
                    ok: false,
                    error_code: e as u32,
                    atomic_rollback: false,
                },
            };
            results.push_back(result);
        }

        if atomic && results.iter().any(|r| !r.ok) {
            let failed = results.iter().filter(|r| !r.ok).count();
            let _marked = mark_atomic_rollback(env, &results);
            panic!(
                "atomic batch rolled back: {} of {} trades failed",
                failed, len
            );
        }

        Ok(results)
    }

    // ── Trade receipt (Issue #683) ────────────────────────────────────────────

    /// Returns the stored SHA-256 receipt hash for `trade_receipt_id`, or `None` if not found.
    ///
    /// The hash covers `(user, asset, amount, price, timestamp)` and can be recomputed
    /// off-chain from the `trade_receipt` event data to verify the trade's authenticity.
    pub fn get_trade_receipt(env: Env, trade_receipt_id: u64) -> Option<BytesN<32>> {
        env.storage()
            .instance()
            .get(&StorageKey::TradeReceiptHash(trade_receipt_id))
    }

    // ── DCA copy trading (Issue #360) ─────────────────────────────────────────

    /// Create a DCA plan: split `total_amount` into `num_intervals` equal trades
    /// spaced `interval_ledgers` apart.  `signal_expiry_ledger = 0` means no expiry.
    pub fn execute_dca_copy_trade(
        env: Env,
        user: Address,
        signal_id: u64,
        total_amount: i128,
        num_intervals: u32,
        interval_ledgers: u32,
        signal_expiry_ledger: u32,
    ) -> Result<(), ContractError> {
        user.require_auth();
        dca::execute_dca_copy_trade(
            &env,
            &user,
            signal_id,
            total_amount,
            num_intervals,
            interval_ledgers,
            signal_expiry_ledger,
        )
    }

    /// Execute the next DCA interval for `(user, signal_id)`.
    /// Called by the keeper network.  Returns `true` when the plan is complete.
    pub fn execute_dca_interval(
        env: Env,
        user: Address,
        signal_id: u64,
    ) -> Result<bool, ContractError> {
        feature_flags::require_feature_enabled(&env, feature_flags::FEAT_DCA)?;
        // Capture config needed inside the closure before moving env.
        let portfolio: Option<Address> = env.storage().instance().get(&StorageKey::UserPortfolio);
        let exempt = {
            let key = StorageKey::PositionLimitExempt(user.clone());
            env.storage().instance().get(&key).unwrap_or(false)
        };

        dca::execute_dca_interval(&env, &user, signal_id, |amount| {
            // Reuse the existing copy-trade balance + position-limit logic.
            let fee = effective_estimated_fee(&env);
            // We don't have a token address in the DCA plan (it's signal-level),
            // so balance check is skipped here — the caller is responsible for
            // ensuring funds are available (same pattern as batch_execute).
            let _ = (amount, fee); // suppress unused warnings

            if let Some(ref p) = portfolio {
                risk_gates::validate_and_record_position(&env, p, &user, exempt)?;
            }
            Ok(())
        })
    }

    /// Manually cancel a DCA plan. Only the plan owner may cancel.
    pub fn cancel_dca_plan(env: Env, user: Address, signal_id: u64) -> Result<(), ContractError> {
        user.require_auth();
        dca::cancel_dca_plan(&env, &user, signal_id)
    }

    // ── Dead-letter queue (Issue #657) ────────────────────────────────────────

    /// Set the maximum number of execution attempts before a queued trade is moved
    /// to the dead-letter queue. Default is [`DEFAULT_MAX_RETRY_COUNT`] (3).
    pub fn set_max_retry_count(env: Env, count: u32) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&StorageKey::MaxRetryCount, &count);
        Ok(())
    }

    /// Return the configured maximum retry count.
    pub fn get_max_retry_count(env: Env) -> u32 {
        effective_max_retry_count(&env)
    }

    /// Return all dead-lettered trades for `user`.
    pub fn get_dead_letter_trades(env: Env, user: Address) -> Vec<FailedTrade> {
        let ids: Vec<u64> = env
            .storage()
            .instance()
            .get(&StorageKey::DeadLetterIds(user))
            .unwrap_or_else(|| Vec::new(&env));
        let mut result: Vec<FailedTrade> = Vec::new(&env);
        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            if let Some(ft) = env
                .storage()
                .instance()
                .get::<_, FailedTrade>(&StorageKey::DeadLetterTrade(id))
            {
                result.push_back(ft);
            }
        }
        result
    }

    /// Re-queue a dead-lettered trade, resetting its retry count.
    /// The trade re-enters the grace-period queue as a fresh attempt.
    pub fn requeue_dead_lettered_trade(env: Env, trade_id: u64) -> Result<(), ContractError> {
        require_admin(&env)?;
        let failed: FailedTrade = env
            .storage()
            .instance()
            .get(&StorageKey::DeadLetterTrade(trade_id))
            .ok_or(ContractError::QueuedTradeNotFound)?;

        // Remove from dead-letter storage.
        env.storage()
            .instance()
            .remove(&StorageKey::DeadLetterTrade(trade_id));
        remove_dead_letter_id(&env, &failed.user, trade_id);
        remove_dead_letter_index(&env, trade_id);

        // Re-queue with a fresh retry_count.
        let trade = QueuedTrade {
            queued_trade_id: trade_id,
            user: failed.user,
            token: failed.token,
            amount: failed.amount,
            portfolio_pct_bps: failed.portfolio_pct_bps,
            queued_at_ledger: env.ledger().sequence(),
            retry_count: 0,
        };
        store_queued_trade(&env, &trade);
        Ok(())
    }

    /// Permanently discard a dead-lettered trade (no requeue).
    pub fn discard_dead_lettered_trade(env: Env, trade_id: u64) -> Result<(), ContractError> {
        require_admin(&env)?;
        let failed: FailedTrade = env
            .storage()
            .instance()
            .get(&StorageKey::DeadLetterTrade(trade_id))
            .ok_or(ContractError::QueuedTradeNotFound)?;

        env.storage()
            .instance()
            .remove(&StorageKey::DeadLetterTrade(trade_id));
        remove_dead_letter_id(&env, &failed.user, trade_id);
        remove_dead_letter_index(&env, trade_id);
        Ok(())
    }

    /// Admin: configure the dead-letter retention window (in ledgers) used by
    /// `prune_dead_letter_queue`. Default is
    /// [`DEFAULT_DEAD_LETTER_RETENTION_LEDGERS`] (~30 days at 5s/ledger).
    pub fn set_dead_letter_retention(env: Env, ledgers: u32) -> Result<(), ContractError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&StorageKey::DeadLetterRetentionLedgers, &ledgers);
        Ok(())
    }

    /// Return the configured dead-letter retention window in ledgers.
    pub fn get_dead_letter_retention(env: Env) -> u32 {
        effective_dead_letter_retention(&env)
    }

    /// Admin: prune dead-lettered trades older than the configured retention
    /// window (see [`Self::set_dead_letter_retention`]).
    ///
    /// When `user` is `Some`, only that user's dead-letter queue is scanned;
    /// when `None`, every dead-lettered trade across all users is scanned.
    /// At most `max_entries` stale entries are removed per call, bounding the
    /// resource cost of a single invocation — callers sweeping a larger
    /// backlog should call this repeatedly.
    ///
    /// Emits `dead_letter_pruned` for each entry removed. Returns the count removed.
    pub fn prune_dead_letter_queue(
        env: Env,
        user: Option<Address>,
        max_entries: u32,
    ) -> Result<u32, ContractError> {
        require_admin(&env)?;

        let retention = effective_dead_letter_retention(&env);
        let current_ledger = env.ledger().sequence();

        let ids: Vec<u64> = match &user {
            Some(u) => env
                .storage()
                .instance()
                .get(&StorageKey::DeadLetterIds(u.clone()))
                .unwrap_or_else(|| Vec::new(&env)),
            None => env
                .storage()
                .instance()
                .get(&StorageKey::DeadLetterAllIds)
                .unwrap_or_else(|| Vec::new(&env)),
        };

        let mut removed: u32 = 0;
        let mut i = 0;
        while i < ids.len() && removed < max_entries {
            let id = ids.get(i).unwrap();
            if let Some(failed) = env
                .storage()
                .instance()
                .get::<_, FailedTrade>(&StorageKey::DeadLetterTrade(id))
            {
                if current_ledger.saturating_sub(failed.dead_lettered_at_ledger) >= retention {
                    env.storage()
                        .instance()
                        .remove(&StorageKey::DeadLetterTrade(id));
                    remove_dead_letter_id(&env, &failed.user, id);
                    remove_dead_letter_index(&env, id);
                    emit_dead_letter_pruned(&env, failed.user.clone(), id);
                    removed = removed.saturating_add(1);
                }
            }
            i += 1;
        }

        Ok(removed)
    }

    // ── Feature flag registry ─────────────────────────────────────────────────

    /// Enable or disable a named feature flag.  Admin only.
    ///
    /// Emits a `feat_flag / changed` event for transparency.
    /// Toggling a flag only affects entrypoints that explicitly check it;
    /// all other entrypoints remain unaffected.
    pub fn set_feature_flag(env: Env, name: String, enabled: bool) -> Result<(), ContractError> {
        require_admin(&env)?;
        feature_flags::set_flag(&env, name, enabled);
        Ok(())
    }

    /// Return `true` when the named flag is enabled (or not set — flags default to enabled).
    pub fn is_feature_enabled(env: Env, name: String) -> bool {
        feature_flags::is_flag_enabled(&env, &name)
    }

    // ── Partial fill handling (Issue #959) ────────────────────────────────────

    /// Report a partial fill for a pending copy-trade.
    ///
    /// Called by a keeper / off-chain executor once the SDEX path-payment or
    /// offer response is known.  When `filled_amount < requested_amount`, the
    /// contract:
    ///
    /// 1. Validates inputs (amounts non-negative, filled ≤ requested, trade exists).
    /// 2. Records a [`PartialFillRecord`] in instance storage so the frontend
    ///    can surface the shortfall without re-scanning events.
    /// 3. Updates the pending-trade status to [`wire::TradeStatus::PartiallyFilled`].
    /// 4. Emits a [`shared::events::EvtPartialFill`] event containing
    ///    `requested_amount`, `filled_amount`, and `remaining_amount`.
    ///
    /// A 100 % fill (`filled_amount == requested_amount`) is accepted as a no-op
    /// (no partial-fill event is emitted) so callers can always report the SDEX
    /// result without needing to pre-check.
    ///
    /// # Errors
    /// - [`ContractError::Unauthorized`] — `caller` is not the contract admin.
    /// - [`ContractError::InvalidAmount`] — amounts are negative or filled > requested.
    /// - [`ContractError::TradeNotFound`] — no pending trade matches the given IDs.
    pub fn record_partial_fill(
        env: Env,
        caller: Address,
        user: Address,
        trade_id: u64,
        requested_amount: i128,
        filled_amount: i128,
    ) -> Result<(), ContractError> {
        // Validate amounts before auth so InvalidAmount is always the first
        // error surface for malformed inputs, regardless of caller identity.
        if requested_amount < 0 || filled_amount < 0 || filled_amount > requested_amount {
            return Err(ContractError::InvalidAmount);
        }

        caller.require_auth();
        require_admin(&env)?;

        let mut order: wire::TradeOrder = env
            .storage()
            .instance()
            .get(&StorageKey::PendingTradeConfirmation)
            .ok_or(ContractError::TradeNotFound)?;

        if order.trade_id != trade_id || order.user != user {
            return Err(ContractError::TradeNotFound);
        }

        // 100 % fill: no partial-fill record or event needed.
        if filled_amount == requested_amount {
            return Ok(());
        }

        let remaining_amount = requested_amount
            .checked_sub(filled_amount)
            .unwrap_or(requested_amount);

        let record = PartialFillRecord {
            requested_amount,
            filled_amount,
            remaining_amount,
            detected_at_ledger: env.ledger().sequence(),
        };
        env.storage().instance().set(
            &StorageKey::PartialFillRecord(user.clone(), trade_id),
            &record,
        );

        order.status = wire::TradeStatus::PartiallyFilled;
        env.storage()
            .instance()
            .set(&StorageKey::PendingTradeConfirmation, &order);

        shared::events::emit_partial_fill(
            &env,
            shared::events::EvtPartialFill {
                schema_version: shared::events::SCHEMA_VERSION,
                user,
                trade_id,
                requested_amount,
                filled_amount,
                remaining_amount,
            },
        );

        Ok(())
    }

    /// Return the [`PartialFillRecord`] for `(user, trade_id)`, if any.
    ///
    /// Returns `None` when the trade was fully filled or no partial-fill was reported.
    pub fn get_partial_fill(env: Env, user: Address, trade_id: u64) -> Option<PartialFillRecord> {
        env.storage()
            .instance()
            .get(&StorageKey::PartialFillRecord(user, trade_id))
    }
}

#[cfg(test)]
mod test;
#[cfg(test)]
mod tests;
