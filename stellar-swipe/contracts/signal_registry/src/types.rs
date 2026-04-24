use crate::categories::{RiskLevel, SignalCategory};
use soroban_sdk::{contracttype, Address, String, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortOption {
    PerformanceDesc,
    RecencyDesc,
    VolumeDesc,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalSummary {
    pub id: u64,
    pub provider: Address,
    pub asset_pair: String,
    pub action: SignalAction,
    pub price: i128,
    pub success_rate: u32,
    pub total_copies: u32,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignalStatus {
    Pending,
    Active,
    Executed,
    Expired,
    Successful,
    Failed,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignalAction {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug)]
/// TradeSignal struct for storing trading signals with category tags for filtering.
/// Category: SCALP (ultra-short), SWING (short-term), LONG_TERM (position), ARBITRAGE (inefficiencies).
pub struct Signal {
    pub id: u64,
    pub provider: Address,
    pub asset_pair: String,
    pub action: SignalAction,
    pub price: i128,
    pub rationale: String,
    pub timestamp: u64,
    pub expiry: u64,
    pub status: SignalStatus,
    pub executions: u32,
    pub successful_executions: u32,
    pub total_volume: i128,
    pub total_roi: i128,
    /// Required SignalCategory tag: SCALP, SWING, LONG_TERM, ARBITRAGE for feed filtering.
    pub category: SignalCategory,
    pub tags: Vec<String>,
    pub risk_level: RiskLevel,
    pub is_collaborative: bool,
    /// Ledger time when the signal was submitted (edit window anchor; Issue #168).
    pub submitted_at: u64,
    /// Editable fingerprint of rationale (Issue #168).
    pub rationale_hash: String,
    /// Provider confidence 0-100.
    pub confidence: u32,
    /// Number of unique adoptions/trades copying this signal
    pub adoption_count: u32,
    /// Optional xAI (or other) off-chain validation score, 0–100; set only by the configured AI oracle.
    pub ai_validation_score: Option<u32>,
}

/// Legacy on-chain format (v1) before v2 added `submitted_at`, `rationale_hash`,
/// `confidence`, and `adoption_count`. Used only for admin migration to [`Signal`].
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalV1 {
    pub id: u64,
    pub provider: Address,
    pub asset_pair: String,
    pub action: SignalAction,
    pub price: i128,
    pub rationale: String,
    pub timestamp: u64,
    pub expiry: u64,
    pub status: SignalStatus,
    pub executions: u32,
    pub successful_executions: u32,
    pub total_volume: i128,
    pub total_roi: i128,
    pub category: SignalCategory,
    pub tags: Vec<String>,
    pub risk_level: RiskLevel,
    pub is_collaborative: bool,
}

/// Emitted each time `migrate_signals_v1_to_v2` processes a batch.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationProgress {
    /// How many v1 records were written to v2 in this batch.
    pub migrated_count: u32,
    /// Total v1 records that existed at the start of migration (constant across batches).
    pub total_count: u32,
}

/// Outcome reported by TradeExecutor when a signal is closed (Issue #170).
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalOutcome {
    Profit,
    Loss,
    Neutral,
}

/// Partial update payload for `update_signal` (Issue #168). Only flags that are true are applied.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalEditInput {
    pub set_price: bool,
    pub price: i128,
    pub set_rationale_hash: bool,
    pub rationale_hash: String,
    pub set_confidence: bool,
    pub confidence: u32,
}


#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct ProviderPerformance {
    pub total_signals: u32,
    pub successful_signals: u32,
    pub failed_signals: u32,
    pub total_copies: u64,
    pub success_rate: u32,
    pub avg_return: i128,
    pub total_volume: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum FeeStorageKey {
    PlatformTreasury,
    ProviderTreasury,
    TreasuryBalances,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeBreakdown {
    pub total_fee: i128,
    pub platform_fee: i128,
    pub provider_fee: i128,
    pub trade_amount_after_fee: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub symbol: Symbol,
    pub contract: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradeExecution {
    pub signal_id: u64,
    pub executor: Address,
    pub entry_price: i128,
    pub exit_price: i128,
    pub volume: i128,
    pub roi: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalPerformanceView {
    pub signal_id: u64,
    pub executions: u32,
    pub total_volume: i128,
    pub average_roi: i128,
    pub status: SignalStatus,
}

#[allow(dead_code)]
pub type SignalStats = ProviderPerformance;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportFormat {
    CSV,
    JSON,
    TradingView,
    TwitterParse,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub format: ImportFormat,
    pub data: soroban_sdk::Bytes,
    pub provider: Address,
    pub validate_only: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ImportResultView {
    pub success_count: u32,
    pub error_count: u32,
    pub signal_ids: soroban_sdk::Vec<u64>,
}

// ==========================================
// NEW SCHEDULING TYPES (Issue #42)
// ==========================================

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalData {
    pub asset_pair: String,
    pub action: SignalAction,
    pub price: i128,
    pub rationale: String,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleStatus {
    Pending = 0,
    Published = 1,
    Cancelled = 2,
    Failed = 3,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrencePattern {
    pub is_recurring: bool,
    pub interval_seconds: u64,
    pub repeat_count: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ScheduledSignal {
    pub id: u64,
    pub provider: Address,
    pub signal_data: SignalData,
    pub publish_at: u64,
    pub recurrence: RecurrencePattern,
    pub status: ScheduleStatus,
}

// ==========================================
// CROSS-CHAIN SYNC TYPES (Issue #95)
// ==========================================

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    Pending,
    Verified,
    Imported,
    UpdatePending,
    Failed,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CrossChainSignal {
    pub source_chain: String,
    pub source_signal_id: String,
    pub stellar_signal_id: u64,
    pub provider_source_address: String,
    pub stellar_address: Address,
    pub verification_proof: soroban_sdk::Bytes,
    pub sync_status: SyncStatus,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AddressMapping {
    pub source_chain: String,
    pub source_address: String,
    pub stellar_address: Address,
    pub is_verified: bool,
}
