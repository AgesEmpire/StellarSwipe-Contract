use shared::errors::{ErrorCategory, RecoveryStrategy};
use shared::initializable;
use soroban_sdk::{contracttype, Address, Env, String, Vec};
use stellar_swipe_common::storage_crud::{
    crud_get, crud_get_or, crud_has, crud_remove, crud_set, StorageTier,
};
use stellar_swipe_common::Asset;

// ── #690: Fee Distribution Waterfall ────────────────────────────────────────

/// A single tier in the fee distribution waterfall, processed in ascending
/// priority order (lower `priority` value = funded first).
#[contracttype]
#[derive(Clone, Debug)]
pub struct WaterfallTier {
    /// Human-readable label (e.g. "treasury", "insurance", "rewards").
    pub name: String,
    /// Lower value = higher priority. Tiers are processed in ascending order.
    pub priority: u32,
    /// Full allocation for this tier; lower-priority tiers receive leftovers.
    pub target_amount: i128,
    /// Minimum the tier must receive to be funded at all. If remaining funds
    /// fall below this, the tier receives nothing.
    pub minimum_amount: i128,
    /// Address that receives the allocation for this tier.
    pub recipient: Address,
}

/// Admin-configurable ordered waterfall of fee destinations.
#[contracttype]
#[derive(Clone, Debug)]
pub struct WaterfallConfig {
    pub tiers: Vec<WaterfallTier>,
}

/// Per-tier allocation record emitted in the waterfall_distribution event.
#[contracttype]
#[derive(Clone, Debug)]
pub struct WaterfallTierResult {
    pub name: String,
    pub recipient: Address,
    pub priority: u32,
    pub allocated: i128,
}

// ── #691: Provider Settlement Currency ──────────────────────────────────────

/// Stored preference for a provider's claim payout currency.
/// `preferred_token` is the SEP-41 token contract to receive fees in.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PayoutCurrency {
    pub preferred_token: Address,
}

pub const MAX_FEE_RATE_BPS: u32 = 100; // 1%
pub const MIN_FEE_RATE_BPS: u32 = 1; // 0.01%
pub const DEFAULT_FEE_RATE_BPS: u32 = 30; // 0.3%
pub const DEFAULT_BURN_RATE_BPS: u32 = 1_000; // 10%
pub const MAX_BURN_RATE_BPS: u32 = 10_000; // 100%
pub const DEFAULT_NETWORK_SCORE_BPS: u32 = 0;
pub const DEFAULT_FEE_OPTIMIZATION_MAX_RATE_BPS: u32 = 100;
pub const DEFAULT_CONGESTION_SENSITIVITY_BPS: u32 = 50;
pub const DEFAULT_MAX_RETRY_ATTEMPTS: u32 = 3;
pub const LEDGERS_PER_MONTH_APPROX: u32 = 518_400; // ~30 days at ~5 seconds per ledger
pub const SILVER_TIER_VOLUME_USD: i128 = 10_000 * 10_000_000; // $10k, 7 decimals
pub const GOLD_TIER_VOLUME_USD: i128 = 50_000 * 10_000_000; // $50k, 7 decimals
pub const SILVER_DISCOUNT_BPS: u32 = 5;
pub const GOLD_DISCOUNT_BPS: u32 = 10;
pub const DEFAULT_REFERRAL_FEE_SHARE_BPS: u32 = 0;

#[contracttype]
pub enum StorageKey {
    Admin,
    Initialized,
    OracleContract,
    TreasuryBalance(Address),              // persistent, per-token
    QueuedWithdrawal,                      // instance, single-slot
    FeeRate,                               // instance, current fee rate in bps
    BurnRate,                              // instance, burn rate in bps
    ProviderPendingFees(Address, Address), // persistent, per (provider, token)
    MonthlyTradeVolume(Address),           // persistent, per user
    /// Accumulated fee shares per provider per day (day = unix_timestamp / SECONDS_PER_DAY).
    ProviderDailyFeeShares(Address, u64),
    /// Day number of the provider's first recorded earnings (for ALL_TIME period_start).
    ProviderEarningsFirstDay(Address),
    /// Total accumulated fee shares for a provider, used to rank earnings leaders.
    ProviderTotalEarnings(Address),
    /// Providers that have recorded earnings, for leaderboard scans.
    ProviderEarningsIndex,
    /// Whether a user has completed their first trade (Issue #428).
    HasTraded(Address),
    // ── Issue #438: Protocol Token Integration ─────────────────────
    /// Optional protocol token address for token-based fee payment.
    ProtocolToken,
    /// Revenue share rate in basis points (default: 2000 = 20%).
    RevenueShareRateBps,
    /// Last snapshot ledger for revenue sharing (Issue #442).
    LastRevenueShareSnapshot,
    /// Accumulated revenue share pool waiting for next distribution.
    RevenueSharePool(Address),
    /// Latest aggregated network score for fee optimization.
    NetworkConditionScore,
    /// Configurable dynamic fee optimization parameters.
    FeeOptimizationConfig,
    /// Last recorded contract error report.
    LastErrorReport,
    /// Persisted failed fee collection operation for retry.
    FailedFeeCollection(String),
    /// #690: Admin-configured waterfall distribution tiers.
    WaterfallConfig,
    /// #691: Per-provider preferred payout token.
    ProviderPayoutCurrency(Address),
    /// Referral mapping from referee to referrer.
    Referrer(Address),
    /// Referral fee share in basis points.
    ReferralFeeShareBps,
    /// #664: Admin-configured volume-based discount tiers.
    VolumeDiscountConfig,
    /// #665: Fee forecast configuration.
    ForecastConfig,
    /// #665: Per-token daily fee total (token, day).
    DailyFeeTotal(Address, u64),
    /// #665: Last day a forecast was emitted per token.
    LastForecastDay(Address),
    /// Congestion signal parameters
    CongestionConfig,
    CongestionSignal,
    /// Issue #813: allowlisted non-admin caller permitted to invoke a narrow
    /// set of privileged, non-user-scoped entry points (e.g. a trusted
    /// trade-settlement keeper contract). See the "Authorized callers"
    /// section below.
    AuthorizedCaller(Address),
    /// Index of all tokens that have non-zero revenue share pools,
    /// used for deterministic snapshot iteration (Issue #814).
    RevenueSharePoolIndex,
    /// Deterministic fee distribution snapshot keyed by ledger sequence.
    Snapshot(u64),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeOptimizationConfig {
    pub max_dynamic_rate_bps: u32,
    pub congestion_sensitivity_bps: u32,
    pub min_effective_rate_bps: u32,
    pub max_retry_attempts: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct ErrorReport {
    pub category: ErrorCategory,
    pub strategy: RecoveryStrategy,
    pub message: String,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct FailedFeeCollection {
    pub id: String,
    pub trader: Address,
    pub token: Address,
    pub trade_amount: i128,
    pub trade_asset: Asset,
    pub retry_count: u32,
    pub last_error: String,
}

#[contracttype]
#[derive(Clone)]
pub struct QueuedWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub queued_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct MonthlyTradeVolume {
    pub month_bucket: u32,
    pub volume_usd: i128,
}

/// Describes a discrepancy between the contract's stored treasury balance
/// and the actual on-chain token balance for a given token.
#[contracttype]
#[derive(Clone, Debug)]
pub struct BalanceMismatch {
    /// The token whose balances were compared.
    pub token: Address,
    /// Balance recorded in contract storage (`TreasuryBalance`).
    pub expected: i128,
    /// Actual token balance held by this contract on-chain.
    pub actual: i128,
    /// Difference: `actual - expected`. Positive means surplus, negative means deficit.
    pub delta: i128,
}

// ── Issue #814: Deterministic fee distribution snapshots ──────────

/// A single entry in a fee distribution snapshot, recording the token
/// and its accumulated revenue share pool amount at snapshot time.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SnapshotEntry {
    pub token: Address,
    pub amount: i128,
}

/// A deterministic snapshot of all revenue share pools at a point in
/// time.  Snapshots are keyed by ledger sequence and recorded in
/// insertion order sorted by token address for determinism.
#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeSnapshot {
    pub ledger: u64,
    pub timestamp: u64,
    pub total_amount: i128,
    pub entries: Vec<SnapshotEntry>,
}

// --- Admin ---

pub fn get_admin(env: &Env) -> Address {
    crud_get::<_, Address>(env, StorageTier::Instance, &StorageKey::Admin).unwrap()
}

pub fn set_admin(env: &Env, admin: &Address) {
    crud_set(env, StorageTier::Instance, &StorageKey::Admin, admin);
}

// --- Initialized (migrated to shared::initializable, issue #584) ---

pub fn is_initialized(env: &Env) -> bool {
    initializable::is_initialized(env)
}

pub fn set_initialized(env: &Env) {
    initializable::mark_initialized(env);
}

// --- Authorized callers (Issue #813: cross-contract auth hardening) ---
//
// A small admin-curated allowlist of non-admin addresses — typically other
// contracts, such as a trade-settlement keeper — permitted to invoke a
// narrow set of privileged entry points that are not naturally scoped to a
// single end-user's own `require_auth()` (e.g. recording provider fee
// shares on their behalf, or pushing a network congestion signal). The
// admin is always implicitly authorized for these entry points and does
// not need to be added to this list.

pub fn is_authorized_caller(env: &Env, caller: &Address) -> bool {
    crud_get::<_, bool>(
        env,
        StorageTier::Instance,
        &StorageKey::AuthorizedCaller(caller.clone()),
    )
    .unwrap_or(false)
}

pub fn set_authorized_caller(env: &Env, caller: &Address) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::AuthorizedCaller(caller.clone()),
        &true,
    );
}

pub fn remove_authorized_caller(env: &Env, caller: &Address) {
    crud_remove(
        env,
        StorageTier::Instance,
        &StorageKey::AuthorizedCaller(caller.clone()),
    );
}

// --- Oracle Contract ---

pub fn get_oracle_contract(env: &Env) -> Option<Address> {
    crud_get(env, StorageTier::Instance, &StorageKey::OracleContract)
}

pub fn set_oracle_contract(env: &Env, contract: &Address) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::OracleContract,
        contract,
    );
}

// --- Treasury Balance ---

pub fn get_treasury_balance(env: &Env, token: &Address) -> i128 {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &StorageKey::TreasuryBalance(token.clone()),
        0i128,
    )
}

pub fn set_treasury_balance(env: &Env, token: &Address, balance: i128) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::TreasuryBalance(token.clone()),
        &balance,
    );
}

// --- Queued Withdrawal ---

pub fn get_queued_withdrawal(env: &Env) -> Option<QueuedWithdrawal> {
    crud_get(env, StorageTier::Instance, &StorageKey::QueuedWithdrawal)
}

pub fn set_queued_withdrawal(env: &Env, withdrawal: &QueuedWithdrawal) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::QueuedWithdrawal,
        withdrawal,
    );
}

pub fn remove_queued_withdrawal(env: &Env) {
    crud_remove(env, StorageTier::Instance, &StorageKey::QueuedWithdrawal);
}

// --- Fee Rate ---

pub fn get_fee_rate(env: &Env) -> u32 {
    crud_get_or(
        env,
        StorageTier::Instance,
        &StorageKey::FeeRate,
        DEFAULT_FEE_RATE_BPS,
    )
}

pub fn set_fee_rate(env: &Env, rate: u32) {
    crud_set(env, StorageTier::Instance, &StorageKey::FeeRate, &rate);
}

// --- Fee Optimization ---

pub fn get_network_condition_score(env: &Env) -> u32 {
    crud_get_or(
        env,
        StorageTier::Instance,
        &StorageKey::NetworkConditionScore,
        DEFAULT_NETWORK_SCORE_BPS,
    )
}

pub fn set_network_condition_score(env: &Env, score: u32) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::NetworkConditionScore,
        &score,
    );
}

pub fn get_fee_optimization_config(env: &Env) -> FeeOptimizationConfig {
    crud_get(
        env,
        StorageTier::Instance,
        &StorageKey::FeeOptimizationConfig,
    )
    .unwrap_or(FeeOptimizationConfig {
        max_dynamic_rate_bps: DEFAULT_FEE_OPTIMIZATION_MAX_RATE_BPS,
        congestion_sensitivity_bps: DEFAULT_CONGESTION_SENSITIVITY_BPS,
        min_effective_rate_bps: MIN_FEE_RATE_BPS,
        max_retry_attempts: DEFAULT_MAX_RETRY_ATTEMPTS,
    })
}

pub fn set_fee_optimization_config(env: &Env, config: &FeeOptimizationConfig) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::FeeOptimizationConfig,
        config,
    );
}

pub fn get_last_error_report(env: &Env) -> Option<ErrorReport> {
    crud_get(env, StorageTier::Instance, &StorageKey::LastErrorReport)
}

pub fn set_last_error_report(env: &Env, report: &ErrorReport) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::LastErrorReport,
        report,
    );
}

pub fn get_failed_fee_collection(env: &Env, id: &String) -> Option<FailedFeeCollection> {
    crud_get(
        env,
        StorageTier::Persistent,
        &StorageKey::FailedFeeCollection(id.clone()),
    )
}

pub fn set_failed_fee_collection(env: &Env, failed: &FailedFeeCollection) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::FailedFeeCollection(failed.id.clone()),
        failed,
    );
}

pub fn remove_failed_fee_collection(env: &Env, id: &String) {
    crud_remove(
        env,
        StorageTier::Persistent,
        &StorageKey::FailedFeeCollection(id.clone()),
    );
}

// --- Burn Rate ---

pub fn get_burn_rate(env: &Env) -> u32 {
    crud_get_or(
        env,
        StorageTier::Instance,
        &StorageKey::BurnRate,
        DEFAULT_BURN_RATE_BPS,
    )
}

pub fn set_burn_rate(env: &Env, rate: u32) {
    crud_set(env, StorageTier::Instance, &StorageKey::BurnRate, &rate);
}

// --- Provider Pending Fees ---

pub fn get_pending_fees(env: &Env, provider: &Address, token: &Address) -> i128 {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderPendingFees(provider.clone(), token.clone()),
        0i128,
    )
}

pub fn set_pending_fees(env: &Env, provider: &Address, token: &Address, amount: i128) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderPendingFees(provider.clone(), token.clone()),
        &amount,
    );
}

// --- Monthly Trade Volume ---

pub fn get_monthly_trade_volume(env: &Env, user: &Address) -> Option<MonthlyTradeVolume> {
    crud_get(
        env,
        StorageTier::Persistent,
        &StorageKey::MonthlyTradeVolume(user.clone()),
    )
}

pub fn set_monthly_trade_volume(env: &Env, user: &Address, volume: &MonthlyTradeVolume) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::MonthlyTradeVolume(user.clone()),
        volume,
    );
}

pub fn remove_monthly_trade_volume(env: &Env, user: &Address) {
    crud_remove(
        env,
        StorageTier::Persistent,
        &StorageKey::MonthlyTradeVolume(user.clone()),
    );
}

// --- Provider Daily Fee Shares (Issue #366) ---

pub fn get_provider_daily_fee_shares(env: &Env, provider: &Address, day: u64) -> i128 {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderDailyFeeShares(provider.clone(), day),
        0i128,
    )
}

pub fn get_provider_total_earnings(env: &Env, provider: &Address) -> i128 {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderTotalEarnings(provider.clone()),
        0i128,
    )
}

pub fn get_provider_earnings_index(env: &Env) -> Vec<Address> {
    crud_get(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderEarningsIndex,
    )
    .unwrap_or_else(|| Vec::new(env))
}

pub fn add_provider_to_earnings_index(env: &Env, provider: &Address) {
    let mut index = get_provider_earnings_index(env);
    for i in 0..index.len() {
        if index.get(i).unwrap() == *provider {
            return;
        }
    }
    index.push_back(provider.clone());
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderEarningsIndex,
        &index,
    );
}

pub fn add_provider_total_earnings(env: &Env, provider: &Address, amount: i128) {
    let key = StorageKey::ProviderTotalEarnings(provider.clone());
    let current: i128 = crud_get_or(env, StorageTier::Persistent, &key, 0i128);
    crud_set(
        env,
        StorageTier::Persistent,
        &key,
        &current.saturating_add(amount),
    );
    add_provider_to_earnings_index(env, provider);
}

pub fn add_provider_daily_fee_shares(env: &Env, provider: &Address, day: u64, amount: i128) {
    let key = StorageKey::ProviderDailyFeeShares(provider.clone(), day);
    let current: i128 = crud_get_or(env, StorageTier::Persistent, &key, 0i128);
    crud_set(
        env,
        StorageTier::Persistent,
        &key,
        &current.saturating_add(amount),
    );

    let first_key = StorageKey::ProviderEarningsFirstDay(provider.clone());
    if !crud_has(env, StorageTier::Persistent, &first_key) {
        crud_set(env, StorageTier::Persistent, &first_key, &day);
    }
    add_provider_total_earnings(env, provider, amount);
}

pub fn get_provider_earnings_first_day(env: &Env, provider: &Address) -> Option<u64> {
    crud_get(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderEarningsFirstDay(provider.clone()),
    )
}

// --- First-trade tracking (Issue #428) ---

pub fn has_traded(env: &Env, user: &Address) -> bool {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &StorageKey::HasTraded(user.clone()),
        false,
    )
}

pub fn set_has_traded(env: &Env, user: &Address) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::HasTraded(user.clone()),
        &true,
    );
}

// ── Issue #438: Protocol Token ──────────────────────────────────────

pub fn get_protocol_token(env: &Env) -> Option<Address> {
    crud_get(env, StorageTier::Instance, &StorageKey::ProtocolToken)
}

pub fn set_protocol_token(env: &Env, token: &Address) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::ProtocolToken,
        token,
    );
}

// ── Issue #880: Token Metadata Validation ────────────────────────────────────────

pub fn get_registered_token_metadata(env: &Env, token: &Address) -> Option<TokenMetadata> {
    crud_get(env, StorageTier::Instance, &StorageKey::RegisteredTokenMetadata(token.clone()))
}

pub fn set_registered_token_metadata(env: &Env, token: &Address, metadata: &TokenMetadata) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::RegisteredTokenMetadata(token.clone()),
        metadata,
    );
}

pub fn remove_registered_token_metadata(env: &Env, token: &Address) {
    crud_remove(env, StorageTier::Instance, &StorageKey::RegisteredTokenMetadata(token.clone()));
}

// ── Issue #442: Revenue Share ────────────────────────────────────────────────────────

pub const DEFAULT_REVENUE_SHARE_RATE_BPS: u32 = 2000; // 20%
pub const SECONDS_PER_WEEK: u64 = 604_800;

pub fn get_revenue_share_rate_bps(env: &Env) -> u32 {
    crud_get_or(
        env,
        StorageTier::Instance,
        &StorageKey::RevenueShareRateBps,
        DEFAULT_REVENUE_SHARE_RATE_BPS,
    )
}

pub fn set_revenue_share_rate_bps(env: &Env, rate_bps: u32) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::RevenueShareRateBps,
        &rate_bps,
    );
}

pub fn get_last_revenue_share_snapshot(env: &Env) -> Option<u64> {
    crud_get(
        env,
        StorageTier::Instance,
        &StorageKey::LastRevenueShareSnapshot,
    )
}

pub fn set_last_revenue_share_snapshot(env: &Env, ledger: u64) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::LastRevenueShareSnapshot,
        &ledger,
    );
}

pub fn get_revenue_share_pool(env: &Env, token: &Address) -> i128 {
    crud_get_or(
        env,
        StorageTier::Persistent,
        &StorageKey::RevenueSharePool(token.clone()),
        0i128,
    )
}

pub fn add_revenue_share_pool(env: &Env, token: &Address, amount: i128) {
    let current = get_revenue_share_pool(env, token);
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::RevenueSharePool(token.clone()),
        &current.saturating_add(amount),
    );
    if amount > 0 {
        add_to_revenue_share_pool_index(env, token);
    }
}

pub fn clear_revenue_share_pool(env: &Env, token: &Address) {
    crud_remove(
        env,
        StorageTier::Persistent,
        &StorageKey::RevenueSharePool(token.clone()),
    );
    remove_from_revenue_share_pool_index(env, token);
}

// ── Revenue Share Pool Index (Issue #814) ──────────────────────────

/// Returns the list of tokens that have non-zero revenue share pools.
/// Used for deterministic snapshot iteration.
pub fn get_revenue_share_pool_index(env: &Env) -> Vec<Address> {
    crud_get_or(
        env,
        StorageTier::Instance,
        &StorageKey::RevenueSharePoolIndex,
        Vec::new(env),
    )
}

/// Adds `token` to the pool index if not already present.
pub fn add_to_revenue_share_pool_index(env: &Env, token: &Address) {
    let mut index = get_revenue_share_pool_index(env);
    for i in 0..index.len() {
        if index.get(i).unwrap() == *token {
            return;
        }
    }
    index.push_back(token.clone());
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::RevenueSharePoolIndex,
        &index,
    );
}

/// Removes `token` from the pool index.
pub fn remove_from_revenue_share_pool_index(env: &Env, token: &Address) {
    let index = get_revenue_share_pool_index(env);
    let mut new_index = Vec::new(env);
    for i in 0..index.len() {
        let t = index.get(i).unwrap();
        if t != *token {
            new_index.push_back(t);
        }
    }
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::RevenueSharePoolIndex,
        &new_index,
    );
}

// ── Snapshot storage (Issue #814) ───────────────────────────────────

pub fn get_fee_snapshot(env: &Env, ledger: u64) -> Option<FeeSnapshot> {
    crud_get(env, StorageTier::Instance, &StorageKey::Snapshot(ledger))
}

pub fn set_fee_snapshot(env: &Env, ledger: u64, snapshot: &FeeSnapshot) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::Snapshot(ledger),
        snapshot,
    );
}

// ── #690: Waterfall Config ───────────────────────────────────────────────────

pub fn get_waterfall_config(env: &Env) -> Option<WaterfallConfig> {
    crud_get(env, StorageTier::Instance, &StorageKey::WaterfallConfig)
}

pub fn set_waterfall_config(env: &Env, config: &WaterfallConfig) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::WaterfallConfig,
        config,
    );
}

// ── #691: Provider Payout Currency ──────────────────────────────────────────

pub fn get_provider_payout_currency(env: &Env, provider: &Address) -> Option<Address> {
    crud_get::<_, PayoutCurrency>(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderPayoutCurrency(provider.clone()),
    )
    .map(|p| p.preferred_token)
}

pub fn set_provider_payout_currency(env: &Env, provider: &Address, preferred_token: &Address) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderPayoutCurrency(provider.clone()),
        &PayoutCurrency {
            preferred_token: preferred_token.clone(),
        },
    );
}

pub fn remove_provider_payout_currency(env: &Env, provider: &Address) {
    crud_remove(
        env,
        StorageTier::Persistent,
        &StorageKey::ProviderPayoutCurrency(provider.clone()),
    );
}

// ── Referral storage ────────────────────────────────────────────────────────

pub fn get_referrer(env: &Env, referee: &Address) -> Option<Address> {
    crud_get(
        env,
        StorageTier::Persistent,
        &StorageKey::Referrer(referee.clone()),
    )
}

pub fn set_referrer(env: &Env, referee: &Address, referrer: &Address) {
    crud_set(
        env,
        StorageTier::Persistent,
        &StorageKey::Referrer(referee.clone()),
        referrer,
    );
}

pub fn get_referral_fee_share_bps(env: &Env) -> u32 {
    crud_get_or(
        env,
        StorageTier::Instance,
        &StorageKey::ReferralFeeShareBps,
        DEFAULT_REFERRAL_FEE_SHARE_BPS,
    )
}

pub fn set_referral_fee_share_bps(env: &Env, share_bps: u32) {
    crud_set(
        env,
        StorageTier::Instance,
        &StorageKey::ReferralFeeShareBps,
        &share_bps,
    );
}

// ── #664: Volume Discount Tiers ──────────────────────────────────────────────

/// A single volume-based discount tier.
/// Users who meet or exceed `volume_threshold_usd` receive a `discount_bps`
/// reduction on the base fee rate.
#[contracttype]
#[derive(Clone, Debug)]
pub struct VolumeTier {
    pub volume_threshold_usd: i128,
    pub discount_bps: u32,
}

/// Admin-configurable set of volume discount tiers.
/// Must contain at least 3 tiers, sorted ascending by `volume_threshold_usd`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct VolumeDiscountConfig {
    pub tiers: Vec<VolumeTier>,
}

pub fn get_volume_discount_config(env: &Env) -> Option<VolumeDiscountConfig> {
    env.storage()
        .instance()
        .get(&StorageKey::VolumeDiscountConfig)
}

pub fn set_volume_discount_config_storage(env: &Env, config: &VolumeDiscountConfig) {
    env.storage()
        .instance()
        .set(&StorageKey::VolumeDiscountConfig, config);
}

// ── #665: Fee Forecast ───────────────────────────────────────────────────────

/// Admin-configurable forecast parameters.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ForecastConfigData {
    /// How often a forecast event is auto-emitted (in days).
    pub epoch_cadence_days: u64,
    /// Number of historical days used to compute the trailing average.
    pub window_days: u64,
}

pub const DEFAULT_EPOCH_CADENCE_DAYS: u64 = 1;
pub const DEFAULT_FORECAST_WINDOW_DAYS: u64 = 7;
pub const SECONDS_PER_DAY_FC: u64 = 86_400;

pub fn get_forecast_config(env: &Env) -> ForecastConfigData {
    env.storage()
        .instance()
        .get(&StorageKey::ForecastConfig)
        .unwrap_or(ForecastConfigData {
            epoch_cadence_days: DEFAULT_EPOCH_CADENCE_DAYS,
            window_days: DEFAULT_FORECAST_WINDOW_DAYS,
        })
}

pub fn set_forecast_config_storage(env: &Env, config: &ForecastConfigData) {
    env.storage()
        .instance()
        .set(&StorageKey::ForecastConfig, config);
}

pub fn get_daily_fee_total(env: &Env, token: &Address, day: u64) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::DailyFeeTotal(token.clone(), day))
        .unwrap_or(0)
}

pub fn add_daily_fee_total(env: &Env, token: &Address, day: u64, amount: i128) {
    let current = get_daily_fee_total(env, token, day);
    env.storage().persistent().set(
        &StorageKey::DailyFeeTotal(token.clone(), day),
        &current.saturating_add(amount),
    );
}

pub fn get_last_forecast_day(env: &Env, token: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&StorageKey::LastForecastDay(token.clone()))
        .unwrap_or(0)
}

pub fn set_last_forecast_day(env: &Env, token: &Address, day: u64) {
    env.storage()
        .persistent()
        .set(&StorageKey::LastForecastDay(token.clone()), &day);
}

// ── Congestion-Based Dynamic Fees ─────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct CongestionConfig {
    pub min_multiplier_bps: u32,
    pub max_multiplier_bps: u32,
    pub staleness_threshold_secs: u64,
    pub default_multiplier_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CongestionSignal {
    pub multiplier_bps: u32,
    pub updated_at: u64,
}

pub fn get_congestion_config(env: &Env) -> CongestionConfig {
    env.storage()
        .instance()
        .get(&StorageKey::CongestionConfig)
        .unwrap_or(CongestionConfig {
            min_multiplier_bps: 10_000,
            max_multiplier_bps: 50_000,
            staleness_threshold_secs: 300,
            default_multiplier_bps: 10_000,
        })
}

pub fn set_congestion_config(env: &Env, config: &CongestionConfig) {
    env.storage()
        .instance()
        .set(&StorageKey::CongestionConfig, config);
}

pub fn get_congestion_signal(env: &Env) -> Option<CongestionSignal> {
    env.storage().instance().get(&StorageKey::CongestionSignal)
}

pub fn set_congestion_signal(env: &Env, signal: &CongestionSignal) {
    env.storage()
        .instance()
        .set(&StorageKey::CongestionSignal, signal);
}
