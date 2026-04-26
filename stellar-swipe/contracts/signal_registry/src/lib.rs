#![no_std]

mod admin;
mod analytics;
mod categories;
mod collaboration;
mod combos;
mod contests;
mod cross_chain;
mod errors;
mod events;
mod expiry;
mod fees;
mod import;
mod leaderboard;
mod ml_scoring;
mod performance;
mod query;
mod reputation;
mod scheduling;
mod social;
mod stake;
mod submission;
mod templates;
mod test_reputation;
mod types;
mod migration;
mod versioning;

pub use categories::{RiskLevel, SignalCategory};
pub use types::SignalAction;

use admin::{
    get_admin, get_admin_config, init_admin, is_trading_paused,
    require_not_paused_legacy as require_not_paused, AdminConfig,
};
use stellar_swipe_common::emergency::PauseState;
use stellar_swipe_common::rate_limit::{self as rl, ActionType as RLAction, RateLimitConfig};
use stellar_swipe_common::SECONDS_PER_30_DAY_MONTH;

use combos::{
    cancel_combo, create_combo_signal, execute_combo_signal, get_combo, get_combo_executions_pub,
    get_combo_performance, ComboExecution, ComboPerformanceSummary, ComboSignal, ComboType,
    ComponentExecution, ComponentSignal,
};
use contests::{Contest, ContestEntry, ContestMetric, ContestStatus};
use errors::{
    AdminError, AiScoreError, ComboError, ContestError, CrossChainError, SignalEditError,
    SignalOutcomeError, TemplateError, VersioningError,
};
pub use leaderboard::{
    get_leaderboard as get_leaderboard_internal, update_leaderboard_index, LeaderboardMetric,
    ProviderLeaderboard, ProviderLeaderboardEntry, ProviderMetric,
};
pub use ml_scoring::{MLModel, SignalFeatures, SignalScore};
use reputation::{
    calculate_trust_score, get_trust_score, update_median_values, update_trust_score,
    TrustScoreDetails, TrustScoreTier,
};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, Map, String, Vec};
use stellar_swipe_common::{health_uninitialized, placeholder_admin, HealthStatus};
use stellar_swipe_common::{validate_asset_pair as validate_asset_pair_common, AssetPairError};
use templates::{SignalTemplate, DEFAULT_TEMPLATE_EXPIRY_HOURS};
use types::{
    AddressMapping, Asset, CrossChainSignal, FeeBreakdown, ImportResultView, ProviderPerformance,
    RecurrencePattern, Signal, SignalData, SignalEditInput, SignalOutcome, SignalPerformanceView,
    SignalStatus, SignalSummary, SortOption, SyncStatus, TradeExecution,
};
use versioning::{CopyRecord, SignalVersion};

const MAX_EXPIRY_SECONDS: u64 = SECONDS_PER_30_DAY_MONTH;

#[contract]
pub struct SignalRegistry;

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    SignalCounter,
    Signals,
    /// Legacy v1 signal map (pre-upgrade). Cleared as rows migrate to [`StorageKey::Signals`].
    SignalsV1,
    /// Next signal id to scan for v1→v2 migration (1-based, advances per batch).
    MigrationCursor,
    /// Snapshot count of v1 keys at migration start (for `MigrationProgress.total_count`).
    MigrationV1TargetTotal,
    ProviderStats,
    /// Per-provider stake balances for trust and submission gates.
    ProviderStakes,
    TradeExecutions,
    TradeCounter,
    TemplateCounter,
    Templates,
    ExternalIdMappings,
    ComboCounter,
    Combos,
    ComboExecutions(u64),
    CrossChainSignals(String, String), // (source_chain, source_signal_id)
    AddressMappings(String, String),   // (source_chain, source_address)
    /// Per-category index of active signal IDs for efficient filtering (Issue #171)
    ActiveSignalsByCategory,
    /// Nonce check for adoption increments to prevent double-counting (Issue #169)
    AdoptionNonces,
    /// Authorized TradeExecutor contract address (set by admin).
    TradeExecutor,
    /// Canonical UserPortfolio used for PREMIUM subscription checks (`check_subscription`).
    UserPortfolio,
    /// Recorded post-close outcomes per signal (Issue #170).
    RecordedSignalOutcomes,
    /// Rolling reputation score per provider (Issue #170).
    ProviderReputationScore(Address),
}
#[contractimpl]
impl SignalRegistry {
    /* =========================
       INITIALIZATION
    ========================== */

    /// # Summary
    /// One-time contract initialization. Sets the admin address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    ///
    /// # Returns
    /// `Ok(())` on success.
    ///
    /// # Errors
    /// - [`AdminError::AlreadyInitialized`] if the contract has already been initialized.
    pub fn initialize(env: Env, admin: Address) -> Result<(), AdminError> {
        init_admin(&env, admin)
    }

    /// Register the TradeExecutor contract address (admin only). Required before `increment_adoption`.
    pub fn set_trade_executor(
        env: Env,
        caller: Address,
        executor: Address,
    ) -> Result<(), AdminError> {
        admin::require_admin(&env, &caller)?;
        caller.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::TradeExecutor, &executor);
        Ok(())
    }

    /// Admin: migrate batched v1 signal records from [`StorageKey::SignalsV1`] into v2
    /// [`StorageKey::Signals`]. Idempotent; safe to call until all v1 rows are gone.
    pub fn migrate_signals_v1_to_v2(
        env: Env,
        caller: Address,
        batch_size: u32,
    ) -> Result<(), AdminError> {
        admin::require_admin(&env, &caller)?;
        caller.require_auth();
        migration::migrate_signals_v1_to_v2(&env, &caller, batch_size)
    }

    /* =========================
       ADMIN FUNCTIONS
    ========================== */

    pub fn set_min_stake(env: Env, caller: Address, new_amount: i128) -> Result<(), AdminError> {
        admin::set_min_stake(&env, &caller, new_amount)
    }

    /// User stakes tokens. Rate-limited to 5 changes per day.
    pub fn stake_tokens(env: Env, provider: Address, amount: i128) -> Result<(), AdminError> {
        provider.require_auth();
        let trust = reputation::get_trust_score(&env, &provider)
            .map(|d| d.score)
            .unwrap_or(0);
        rl::check_rate_limit(&env, &provider, RLAction::StakeChange, trust)
            .map_err(|_| AdminError::RateLimitExceeded)?;

        let mut stakes = Self::get_provider_stakes_map(&env);
        stake::stake(&env, &mut stakes, &provider, amount).map_err(|e| match e {
            stake::ContractError::InvalidStakeAmount
            | stake::ContractError::NoStakeFound
            | stake::ContractError::StakeLocked
            | stake::ContractError::InsufficientStake
            | stake::ContractError::BelowMinimumStake => AdminError::InvalidParameter,
        })?;
        Self::save_provider_stakes_map(&env, &stakes);
        rl::record_action(&env, &provider, RLAction::StakeChange);
        Ok(())
    }

    /// User unstakes tokens. Rate-limited to 5 changes per day.
    pub fn unstake_tokens(env: Env, provider: Address) -> Result<(), AdminError> {
        provider.require_auth();
        let trust = reputation::get_trust_score(&env, &provider)
            .map(|d| d.score)
            .unwrap_or(0);
        rl::check_rate_limit(&env, &provider, RLAction::StakeChange, trust)
            .map_err(|_| AdminError::RateLimitExceeded)?;

        let mut stakes = Self::get_provider_stakes_map(&env);
        let _ = stake::unstake(&env, &mut stakes, &provider).map_err(|e| match e {
            stake::ContractError::InvalidStakeAmount
            | stake::ContractError::NoStakeFound
            | stake::ContractError::StakeLocked
            | stake::ContractError::InsufficientStake
            | stake::ContractError::BelowMinimumStake => AdminError::InvalidParameter,
        })?;
        Self::save_provider_stakes_map(&env, &stakes);
        rl::record_action(&env, &provider, RLAction::StakeChange);
        Ok(())
    }

    pub fn set_trade_fee(env: Env, caller: Address, new_fee_bps: u32) -> Result<(), AdminError> {
        admin::set_trade_fee(&env, &caller, new_fee_bps)
    }

    pub fn set_risk_defaults(
        env: Env,
        caller: Address,
        stop_loss: u32,
        position_limit: u32,
    ) -> Result<(), AdminError> {
        admin::set_risk_defaults(&env, &caller, stop_loss, position_limit)
    }

    /// Admin: update rate limit config for an action type.
    pub fn set_rate_limit_config(
        env: Env,
        caller: Address,
        action: RLAction,
        window_secs: u64,
        max_actions: u32,
    ) -> Result<(), AdminError> {
        admin::require_admin(&env, &caller)?;
        caller.require_auth();
        rl::set_config(
            &env,
            action,
            RateLimitConfig {
                window_secs,
                max_actions,
            },
        );
        Ok(())
    }

    pub fn pause_trading(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::pause_trading(&env, &caller)
    }

    pub fn unpause_trading(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::unpause_trading(&env, &caller)
    }

    // ── Fee Collection Pause (Issue #189) ────────────────────────────────────

    /// Pause fee collection while allowing reads and position closures.
    pub fn pause_fee_collection(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::pause_fee_collection(&env, &caller)
    }

    /// Resume fee collection.
    pub fn resume_fee_collection(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::resume_fee_collection(&env, &caller)
    }

    /// Check if fee collection is currently paused.
    pub fn is_fee_collection_paused(env: Env) -> bool {
        admin::is_fee_collection_paused(&env)
    }

    pub fn pause_category(
        env: Env,
        caller: Address,
        category: String,
        duration: Option<u64>,
        reason: String,
    ) -> Result<(), AdminError> {
        admin::pause_category(&env, &caller, category, duration, reason)
    }

    pub fn unpause_category(env: Env, caller: Address, category: String) -> Result<(), AdminError> {
        admin::unpause_category(&env, &caller, category)
    }

    pub fn get_pause_states(env: Env) -> Map<String, PauseState> {
        admin::get_pause_states(&env)
    }

    pub fn propose_admin_transfer(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), AdminError> {
        admin::propose_admin_transfer(&env, &caller, new_admin)
    }

    pub fn accept_admin_transfer(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::accept_admin_transfer(&env, &caller)
    }

    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::cancel_admin_transfer(&env, &caller)
    }

    pub fn set_guardian(env: Env, caller: Address, guardian: Address) -> Result<(), AdminError> {
        admin::set_guardian(&env, &caller, guardian)
    }

    pub fn revoke_guardian(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::revoke_guardian(&env, &caller)
    }

    pub fn get_guardian(env: Env) -> Option<Address> {
        admin::get_guardian(&env)
    }

    pub fn get_admin(env: Env) -> Result<Address, AdminError> {
        get_admin(&env)
    }

    pub fn schedule(
        env: Env,
        provider: Address,
        signal_data: SignalData,
        publish_at: u64,
        recurrence: RecurrencePattern,
    ) -> Result<u64, AdminError> {
        scheduling::schedule_signal(env, provider, signal_data, publish_at, recurrence)
    }

    pub fn trigger_scheduled_publications(env: Env) -> Vec<u64> {
        scheduling::publish_scheduled_signals(env)
    }

    pub fn cancel_schedule(
        env: Env,
        provider: Address,
        schedule_id: u64,
    ) -> Result<(), AdminError> {
        scheduling::cancel_scheduled_signal(env, provider, schedule_id)
    }

    pub fn get_config(env: Env) -> AdminConfig {
        get_admin_config(&env)
    }

    /// Read-only health probe for monitoring and front-ends (no auth).
    pub fn health_check(env: Env) -> HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        if !admin::has_admin(&env) {
            return health_uninitialized(&env, version);
        }
        let admin_addr = match get_admin(&env) {
            Ok(a) => a,
            Err(_) => placeholder_admin(&env),
        };
        HealthStatus {
            is_initialized: true,
            is_paused: is_trading_paused(&env),
            version,
            admin: admin_addr,
        }
    }

    pub fn set_circuit_breaker_config(
        env: Env,
        caller: Address,
        config: stellar_swipe_common::emergency::CircuitBreakerConfig,
    ) -> Result<(), AdminError> {
        admin::set_circuit_breaker_config(&env, &caller, config)
    }

    pub fn get_circuit_breaker_config(
        env: Env,
    ) -> Option<stellar_swipe_common::emergency::CircuitBreakerConfig> {
        admin::get_circuit_breaker_config(&env)
    }

    pub fn get_circuit_breaker_stats(
        env: Env,
    ) -> stellar_swipe_common::emergency::CircuitBreakerStats {
        admin::get_circuit_breaker_stats(&env)
    }

    pub fn is_paused(env: Env) -> bool {
        is_trading_paused(&env)
    }

    pub fn get_pause_info(env: Env) -> PauseState {
        admin::get_pause_info(&env)
    }

    // Multi-sig functions
    pub fn enable_multisig(
        env: Env,
        caller: Address,
        signers: Vec<Address>,
        threshold: u32,
    ) -> Result<(), AdminError> {
        admin::enable_multisig(&env, &caller, signers, threshold)
    }

    pub fn disable_multisig(env: Env, caller: Address) -> Result<(), AdminError> {
        admin::disable_multisig(&env, &caller)
    }

    pub fn is_multisig_enabled(env: Env) -> bool {
        admin::is_multisig_enabled(&env)
    }

    pub fn get_multisig_signers(env: Env) -> Vec<Address> {
        admin::get_multisig_signers(&env)
    }

    pub fn get_multisig_threshold(env: Env) -> u32 {
        admin::get_multisig_threshold(&env)
    }

    pub fn add_multisig_signer(
        env: Env,
        caller: Address,
        new_signer: Address,
    ) -> Result<(), AdminError> {
        admin::add_multisig_signer(&env, &caller, new_signer)
    }

    pub fn remove_multisig_signer(
        env: Env,
        caller: Address,
        signer_to_remove: Address,
    ) -> Result<(), AdminError> {
        admin::remove_multisig_signer(&env, &caller, signer_to_remove)
    }

    /* =========================
       INTERNAL HELPERS
    ========================== */

    fn next_signal_id(env: &Env) -> u64 {
        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::SignalCounter)
            .unwrap_or(0);

        counter = counter.checked_add(1).expect("signal id overflow");

        env.storage()
            .instance()
            .set(&StorageKey::SignalCounter, &counter);

        counter
    }

    fn next_trade_id(env: &Env) -> u64 {
        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::TradeCounter)
            .unwrap_or(0);
        counter = counter.checked_add(1).expect("trade id overflow");
        env.storage()
            .instance()
            .set(&StorageKey::TradeCounter, &counter);
        counter
    }

    fn get_trade_executions_map(env: &Env) -> Map<u64, TradeExecution> {
        env.storage()
            .instance()
            .get(&StorageKey::TradeExecutions)
            .unwrap_or(Map::new(env))
    }

    fn save_trade_executions_map(env: &Env, map: &Map<u64, TradeExecution>) {
        env.storage()
            .instance()
            .set(&StorageKey::TradeExecutions, map);
    }

    fn get_signals_map(env: &Env) -> Map<u64, Signal> {
        env.storage()
            .instance()
            .get(&StorageKey::Signals)
            .unwrap_or(Map::new(env))
    }

    fn save_signals_map(env: &Env, map: &Map<u64, Signal>) {
        env.storage().instance().set(&StorageKey::Signals, map);
    }

    fn get_category_index_map(env: &Env) -> Map<SignalCategory, Vec<u64>> {
        env.storage()
            .instance()
            .get(&StorageKey::ActiveSignalsByCategory)
            .unwrap_or(Map::new(env))
    }

    fn save_category_index_map(env: &Env, map: &Map<SignalCategory, Vec<u64>>) {
        env.storage()
            .instance()
            .set(&StorageKey::ActiveSignalsByCategory, map);
    }

    fn get_provider_stakes_map(env: &Env) -> Map<Address, stake::StakeInfo> {
        env.storage()
            .instance()
            .get(&StorageKey::ProviderStakes)
            .unwrap_or(Map::new(env))
    }

    fn save_provider_stakes_map(env: &Env, map: &Map<Address, stake::StakeInfo>) {
        env.storage()
            .instance()
            .set(&StorageKey::ProviderStakes, map);
    }

    fn get_provider_stats_map(env: &Env) -> Map<Address, ProviderPerformance> {
        env.storage()
            .instance()
            .get(&StorageKey::ProviderStats)
            .unwrap_or(Map::new(env))
    }

    fn save_provider_stats_map(env: &Env, map: &Map<Address, ProviderPerformance>) {
        env.storage()
            .instance()
            .set(&StorageKey::ProviderStats, map);
    }

    fn validate_asset_pair(env: &Env, asset_pair: &String) -> Result<(), AdminError> {
        validate_asset_pair_common(env, asset_pair).map_err(|e| match e {
            AssetPairError::InvalidFormat
            | AssetPairError::InvalidAssetCode
            | AssetPairError::InvalidIssuer
            | AssetPairError::SameAssets => AdminError::InvalidAssetPair,
        })
    }

    /* =========================
       PUBLIC API
    ========================== */

    /// # Summary
    /// Create a new trading signal. The provider must authorize the call.
    /// Signals are rate-limited and subject to pause state checks.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `provider`: Address of the signal provider (must authorize).
    /// - `asset_pair`: Asset pair string (e.g. `"XLM/USDC"`).
    /// - `action`: [`SignalAction::Buy`] or [`SignalAction::Sell`].
    /// - `price`: Target price for the signal (must be > 0).
    /// - `rationale`: Human-readable rationale for the signal.
    /// - `expiry`: Unix timestamp when the signal expires (must be in the future, max 30 days).
    /// - `category`: Signal category (e.g. SWING, SCALP, PREMIUM).
    /// - `tags`: Up to 10 tags for discoverability.
    /// - `risk_level`: Risk classification (Low, Medium, High).
    ///
    /// # Returns
    /// The new signal ID.
    ///
    /// # Errors
    /// - [`AdminError::TradingPaused`] — signals category is paused.
    /// - [`AdminError::RateLimitExceeded`] — provider has exceeded submission rate limit.
    /// - [`AdminError::InvalidAssetPair`] — asset_pair format is invalid.
    /// - Panics if expiry is in the past or exceeds 30 days.
    pub fn create_signal(
        env: Env,
        provider: Address,
        asset_pair: String,
        action: SignalAction,
        price: i128,
        rationale: String,
        expiry: u64,
        category: SignalCategory,
        tags: Vec<String>,
        risk_level: RiskLevel,
    ) -> Result<u64, AdminError> {
        provider.require_auth();
        // Analytics: session start on first call by this provider
        shared::events::emit_session_started_once(&env, &provider);
        Self::create_signal_internal(
            &env, provider, asset_pair, action, price, rationale, expiry, category, tags,
            risk_level,
        )
    }

    fn create_signal_internal(
        env: &Env,
        provider: Address,
        asset_pair: String,
        action: SignalAction,
        price: i128,
        rationale: String,
        expiry: u64,
        category: SignalCategory,
        tags: Vec<String>,
        risk_level: RiskLevel,
    ) -> Result<u64, AdminError> {
        // Check if signals are paused
        admin::require_not_paused(env, String::from_str(env, CAT_SIGNALS))?;

        // Rate limit: signal submission
        let trust = reputation::get_trust_score(env, &provider)
            .map(|d| d.score)
            .unwrap_or(0);
        rl::check_rate_limit(env, &provider, RLAction::SignalSubmission, trust)
            .map_err(|_| AdminError::RateLimitExceeded)?;
        rl::record_action(env, &provider, RLAction::SignalSubmission);

        Self::validate_asset_pair(env, &asset_pair)?;

        // Validate and deduplicate tags
        categories::validate_tags(&tags)?;
        let unique_tags = categories::deduplicate_tags(env, tags);

        let now = env.ledger().timestamp();

        if expiry <= now {
            panic!("expiry must be in the future");
        }

        if expiry > now + MAX_EXPIRY_SECONDS {
            panic!("expiry exceeds max 30 days");
        }

        let id = Self::next_signal_id(env);
        let rationale_hash = rationale.clone();

        let signal = Signal {
            id,
            provider: provider.clone(),
            asset_pair,
            action,
            price,
            rationale,
            timestamp: now,
            submitted_at: now,
            expiry,
            status: SignalStatus::Active,
            // Initialize performance tracking fields
            executions: 0,
            successful_executions: 0,
            total_volume: 0,
            total_roi: 0,
            // Categorization fields
            category: category.clone(),
            tags: unique_tags.clone(),
            risk_level,
            // Collaboration field
            is_collaborative: false,
            rationale_hash,
            confidence: 50,
            adoption_count: 0,
            ai_validation_score: None,
            avg_copier_roi_bps: 0,
            copier_closed_count: 0,
        };

        // Auto-enter signal into active contests (before moving signal)
        let _ = contests::auto_enter_signal(env, &signal);

        // Store signal
        let mut signals = Self::get_signals_map(env);
        signals.set(id, signal);
        Self::save_signals_map(env, &signals);

        // Update tag popularity
        categories::increment_tag_popularity(env, &unique_tags);

        // Add to per-category index for efficient filtering (Issue #171)
        let mut cat_map = Self::get_category_index_map(env);
        let mut cat_list = cat_map.get(category.clone()).unwrap_or(Vec::new(env));
        cat_list.push_back(id);
        cat_map.set(category, cat_list);
        Self::save_category_index_map(env, &cat_map);

        // Initialize provider stats on first submission
        let mut stats = Self::get_provider_stats_map(env);
        if !stats.contains_key(provider.clone()) {
            stats.set(provider.clone(), ProviderPerformance::default());
            Self::save_provider_stats_map(env, &stats);

            // Record first signal time for trust score calculation
            reputation::record_first_signal(env, &provider);
        }

        Ok(id)
    }

    pub fn get_signal(env: Env, signal_id: u64) -> Option<Signal> {
        let signals = Self::get_signals_map(&env);
        signals.get(signal_id)
    }

    /// Return the signal if `viewer` is allowed to see it. Non-[`SignalCategory::PREMIUM`]
    /// signals are visible to any viewer. PREMIUM signals require an active on-chain
    /// subscription (via UserPortfolio [`check_subscription`]) unless the viewer is the
    /// signal provider.
    pub fn get_signal_for_viewer(
        env: Env,
        signal_id: u64,
        viewer: Address,
    ) -> Option<Signal> {
        let signals = Self::get_signals_map(&env);
        let signal = signals.get(signal_id)?;

        // Analytics: emit session + signal-viewed events (no state changes)
        shared::events::emit_session_started_once(&env, &viewer);
        shared::events::emit_signal_viewed(
            &env,
            shared::events::EvtSignalViewed {
                schema_version: shared::events::SCHEMA_VERSION,
                user: viewer.clone(),
                signal_id,
                timestamp: env.ledger().timestamp(),
            },
        );
        if signal.category != SignalCategory::PREMIUM {
            return Some(signal);
        }
        if viewer == signal.provider {
            return Some(signal);
        }
        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)?;
        let allowed = Self::invoke_check_subscription(&env, &portfolio, &viewer, &signal.provider);
        if allowed {
            Some(signal)
        } else {
            None
        }
    }

    fn invoke_check_subscription(
        env: &Env,
        portfolio: &Address,
        user: &Address,
        provider: &Address,
    ) -> bool {
        let sym = Symbol::new(env, "check_subscription");
        let mut args = Vec::<Val>::new(env);
        args.push_back(user.clone().into_val(env));
        args.push_back(provider.clone().into_val(env));
        env.invoke_contract::<bool>(portfolio, &sym, args)
    }

    /// Edit price, rationale hash, or confidence within 60s of `submitted_at` (Issue #168).
    pub fn update_signal(
        env: Env,
        provider: Address,
        signal_id: u64,
        edit: SignalEditInput,
    ) -> Result<(), SignalEditError> {
        provider.require_auth();
        admin::require_not_paused(&env, String::from_str(&env, CAT_SIGNALS))
            .map_err(|_| SignalEditError::TradingPaused)?;

        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals
            .get(signal_id)
            .ok_or(SignalEditError::SignalNotFound)?;
        if signal.provider != provider {
            return Err(SignalEditError::NotSignalOwner);
        }
        if signal.adoption_count > 0 {
            return Err(SignalEditError::SignalAlreadyCopied);
        }
        let now = env.ledger().timestamp();
        if now.saturating_sub(signal.submitted_at) > 60 {
            return Err(SignalEditError::EditWindowClosed);
        }
        if edit.set_price {
            if edit.price <= 0 {
                return Err(SignalEditError::FieldNotEditable);
            }
            signal.price = edit.price;
        }
        if edit.set_rationale_hash {
            let blen = edit.rationale_hash.len();
            if blen == 0 || blen > 128 {
                return Err(SignalEditError::FieldNotEditable);
            }
            signal.rationale_hash = edit.rationale_hash;
        }
        if edit.set_confidence {
            if edit.confidence > 100 {
                return Err(SignalEditError::InvalidConfidence);
            }
            signal.confidence = edit.confidence;
        }
        signals.set(signal_id, signal.clone());
        Self::save_signals_map(&env, &signals);
        events::emit_signal_edited(
            &env,
            signal_id,
            provider.clone(),
            signal.price,
            signal.rationale_hash.clone(),
            signal.confidence,
        );
        Ok(())
    }

    /// Record closed-signal outcome and update provider reputation (Issue #170).
    pub fn record_signal_outcome(
        env: Env,
        caller: Address,
        signal_id: u64,
        outcome: SignalOutcome,
    ) -> Result<(), SignalOutcomeError> {
        caller.require_auth();
        let executor: Address = env
            .storage()
            .instance()
            .get(&StorageKey::TradeExecutor)
            .ok_or(SignalOutcomeError::Unauthorized)?;
        if caller != executor {
            return Err(SignalOutcomeError::Unauthorized);
        }

        let mut recorded: Map<u64, bool> = env
            .storage()
            .instance()
            .get(&StorageKey::RecordedSignalOutcomes)
            .unwrap_or_else(|| Map::new(&env));
        if recorded.get(signal_id).unwrap_or(false) {
            return Err(SignalOutcomeError::OutcomeAlreadyRecorded);
        }

        let signals = Self::get_signals_map(&env);
        let signal = signals
            .get(signal_id)
            .ok_or(SignalOutcomeError::SignalNotFound)?;
        if signal.status == SignalStatus::Active {
            return Err(SignalOutcomeError::SignalNotClosed);
        }

        let provider = signal.provider.clone();
        let rep_key = StorageKey::ProviderReputationScore(provider.clone());
        let old_score: u32 = env.storage().instance().get(&rep_key).unwrap_or(50);
        let new_score = reputation::next_reputation_score(old_score, &outcome);
        env.storage().instance().set(&rep_key, &new_score);

        recorded.set(signal_id, true);
        env.storage()
            .instance()
            .set(&StorageKey::RecordedSignalOutcomes, &recorded);

        events::emit_reputation_updated(&env, provider.clone(), old_score, new_score);
        Ok(())
    }

    pub fn get_provider_reputation_score(env: Env, provider: Address) -> u32 {
        let rep_key = StorageKey::ProviderReputationScore(provider);
        env.storage().instance().get(&rep_key).unwrap_or(50)
    }

    pub fn get_provider_stats(env: Env, provider: Address) -> Option<ProviderPerformance> {
        let stats = Self::get_provider_stats_map(&env);
        stats.get(provider)
    }

    pub fn create_template(
        env: Env,
        provider: Address,
        name: String,
        asset_pair: Option<String>,
        rationale_template: String,
    ) -> Result<u64, TemplateError> {
        provider.require_auth();

        if name.len() == 0 || rationale_template.len() == 0 {
            return Err(TemplateError::InvalidTemplate);
        }

        if let Some(ref pair) = asset_pair {
            Self::validate_asset_pair(&env, pair).map_err(|_| TemplateError::InvalidTemplate)?;
        }

        let template_id = templates::get_next_template_id(&env);

        let template = SignalTemplate {
            id: template_id,
            provider: provider.clone(),
            name,
            asset_pair,
            action: None,
            rationale_template,
            default_expiry_hours: DEFAULT_TEMPLATE_EXPIRY_HOURS,
            is_public: false,
            use_count: 0,
        };

        templates::store_template(&env, template_id, &template);
        Ok(template_id)
    }

    pub fn set_template_public(
        env: Env,
        provider: Address,
        template_id: u64,
        is_public: bool,
    ) -> Result<(), TemplateError> {
        provider.require_auth();
        templates::set_template_visibility(&env, &provider, template_id, is_public)
    }

    pub fn get_template(env: Env, template_id: u64) -> Option<SignalTemplate> {
        templates::get_template(&env, template_id)
    }

    pub fn submit_from_template(
        env: Env,
        submitter: Address,
        template_id: u64,
        variables: Map<String, String>,
    ) -> Result<u64, TemplateError> {
        submitter.require_auth();

        let template =
            templates::get_template(&env, template_id).ok_or(TemplateError::TemplateNotFound)?;
        if !template.is_public && template.provider != submitter {
            return Err(TemplateError::PrivateTemplate);
        }

        let asset_pair = match template.asset_pair {
            Some(pair) => pair,
            None => templates::get_variable(&variables, "asset_pair")?
                .ok_or(TemplateError::MissingVariable)?,
        };
        Self::validate_asset_pair(&env, &asset_pair).map_err(|_| TemplateError::InvalidTemplate)?;

        let action = match template.action {
            Some(template_action) => templates::parse_action(&template_action)?,
            None => {
                let action_text = templates::get_variable(&variables, "action")?
                    .ok_or(TemplateError::MissingVariable)?;
                templates::parse_action(&action_text)?
            }
        };

        let price_text =
            templates::get_variable(&variables, "price")?.ok_or(TemplateError::MissingVariable)?;
        let price = templates::parse_price(&price_text)?;

        let rationale =
            templates::replace_variables(&env, &template.rationale_template, &variables)?;

        let expiry = env
            .ledger()
            .timestamp()
            .checked_add((template.default_expiry_hours as u64) * 60 * 60)
            .ok_or(TemplateError::InvalidExpiry)?;
        if expiry > env.ledger().timestamp() + MAX_EXPIRY_SECONDS {
            return Err(TemplateError::InvalidExpiry);
        }

        // Default category, tags, and risk_level for templates
        let category = SignalCategory::SWING;
        let tags = Vec::new(&env);
        let risk_level = RiskLevel::Medium;

        let signal_id = Self::create_signal_internal(
            &env, submitter, asset_pair, action, price, rationale, expiry, category, tags,
            risk_level,
        )
        .map_err(|_| TemplateError::InvalidTemplate)?;

        templates::increment_template_use_count(&env, template_id)?;
        Ok(signal_id)
    }

    /* =========================
       PERFORMANCE TRACKING FUNCTIONS
    ========================== */

    /// Record a trade execution for a signal and update performance stats
    pub fn record_trade_execution(
        env: Env,
        executor: Address,
        signal_id: u64,
        entry_price: i128,
        exit_price: i128,
        volume: i128,
    ) -> Result<(), errors::PerformanceError> {
        // Check if trading is paused
        if admin::is_category_paused(&env, String::from_str(&env, CAT_TRADING)) {
            return Err(errors::PerformanceError::TradingPaused);
        }

        // Require executor authorization
        executor.require_auth();

        // Rate limit: trade execution
        let trust = reputation::get_trust_score(&env, &executor)
            .map(|d| d.score)
            .unwrap_or(0);
        rl::check_rate_limit(&env, &executor, RLAction::TradeExecution, trust)
            .map_err(|_| errors::PerformanceError::TradingPaused)?; // reuse closest error variant
        rl::record_action(&env, &executor, RLAction::TradeExecution);

        // Validate inputs
        if entry_price <= 0 || exit_price <= 0 {
            return Err(errors::PerformanceError::InvalidPrice);
        }
        if volume <= 0 {
            return Err(errors::PerformanceError::InvalidVolume);
        }

        // Load signal
        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals
            .get(signal_id)
            .ok_or(errors::PerformanceError::SignalNotFound)?;

        // Calculate ROI
        let roi = performance::calculate_roi(entry_price, exit_price, &signal.action);

        // Create trade execution record
        let trade = TradeExecution {
            signal_id,
            executor: executor.clone(),
            entry_price,
            exit_price,
            volume,
            roi,
            timestamp: env.ledger().timestamp(),
        };

        // Store old status for comparison
        let old_status = signal.status.clone();

        // Update signal stats (general perf) and copier ROI (Issue #367)
        performance::update_signal_stats(&mut signal, &trade);
        performance::update_copier_roi_stats(&mut signal, roi.clamp(i32::MIN as i128, i32::MAX as i128) as i32);

        // Evaluate new status
        let now = env.ledger().timestamp();
        let new_status = performance::evaluate_signal_status(&signal, now);
        signal.status = new_status.clone();

        // Save updated signal
        signals.set(signal_id, signal.clone());
        Self::save_signals_map(&env, &signals);

        let provider_for_contest = signal.provider.clone();

        // Emit trade executed event
        events::emit_trade_executed(&env, signal_id, executor.clone(), roi, volume);

        // Analytics: session + trade executed
        shared::events::emit_session_started_once(&env, &executor);
        shared::events::emit_analytics_trade_executed(
            &env,
            shared::events::EvtTradeExecuted {
                schema_version: shared::events::SCHEMA_VERSION,
                user: executor.clone(),
                signal_id,
                timestamp: env.ledger().timestamp(),
            },
        );

        // Check if status changed and update provider stats
        if performance::should_update_provider_stats(&old_status, &new_status) {
            let mut provider_stats_map = Self::get_provider_stats_map(&env);
            let mut provider_stats = provider_stats_map
                .get(signal.provider.clone())
                .unwrap_or_default();

            let signal_avg_roi = performance::get_signal_average_roi(&signal);

            performance::update_provider_performance(
                &mut provider_stats,
                &old_status,
                &new_status,
                signal_avg_roi,
                signal.total_volume,
            );

            provider_stats_map.set(signal.provider.clone(), provider_stats.clone());
            Self::save_provider_stats_map(&env, &provider_stats_map);

            // Update leaderboard index (O(INDEX_CAPACITY) in-memory, O(1) query after)
            update_leaderboard_index(&env, signal.provider.clone(), &provider_stats);

            // Update trust score when performance changes
            Self::update_provider_trust_score(env.clone(), signal.provider.clone());

            // Emit status change event
            events::emit_signal_status_changed(
                &env,
                signal_id,
                signal.provider.clone(),
                old_status as u32,
                new_status as u32,
            );

            // Emit provider stats updated event
            events::emit_provider_stats_updated(
                &env,
                signal.provider,
                provider_stats.success_rate,
                provider_stats.avg_return,
                provider_stats.total_volume,
            );
        }

        contests::apply_trade_to_contest_entries(
            &env,
            signal_id,
            &provider_for_contest,
            roi,
            volume,
        );

        Ok(())
    }

    /// Get signal performance metrics
    pub fn get_signal_performance(env: Env, signal_id: u64) -> Option<SignalPerformanceView> {
        let signals = Self::get_signals_map(&env);
        let signal = signals.get(signal_id)?;

        let average_roi = performance::get_signal_average_roi(&signal);

        Some(SignalPerformanceView {
            signal_id: signal.id,
            executions: signal.executions,
            total_volume: signal.total_volume,
            average_roi,
            status: signal.status,
        })
    }

    /// Get provider performance stats (alias for get_provider_stats)
    pub fn get_provider_performance(env: Env, provider: Address) -> Option<ProviderPerformance> {
        Self::get_provider_stats(env, provider)
    }

    /// Get leaderboard of top providers by metric
    ///
    /// # Arguments
    /// * `metric` - SuccessRate, Volume, or Followers (empty for MVP)
    /// * `limit` - Max providers to return (0 = default 10, max 50)
    ///
    /// # Minimum qualification
    /// - >= 5 signals with terminal status
    /// - success_rate > 0 (exclude all-failed)
    pub fn get_leaderboard(
        env: Env,
        metric: LeaderboardMetric,
        limit: u32,
    ) -> Vec<ProviderLeaderboard> {
        let stats_map = Self::get_provider_stats_map(&env);
        get_leaderboard_internal(&env, &stats_map, metric, limit)
    }

    /// Get top N providers ranked by the requested metric.
    ///
    /// Providers with fewer than 10 closed signals are excluded.
    /// Verified providers (stake >= minimum) are flagged in results.
    pub fn get_provider_leaderboard(
        env: Env,
        metric: ProviderMetric,
        limit: u32,
    ) -> Vec<ProviderLeaderboardEntry> {
        leaderboard::get_provider_leaderboard(&env, metric, limit)
    }

    /// Get top providers sorted by success rate
    pub fn get_top_providers(env: Env, limit: u32) -> Vec<(Address, ProviderPerformance)> {
        let stats_map = Self::get_provider_stats_map(&env);
        let mut providers = Vec::new(&env);

        // Collect all providers
        for key in stats_map.keys() {
            if let Some(stats) = stats_map.get(key.clone()) {
                providers.push_back((key, stats));
            }
        }

        // Sort by success rate (descending)
        // Note: Soroban Vec doesn't have built-in sort, so we implement a simple bubble sort
        let len = providers.len();
        for i in 0..len {
            for j in 0..(len - i - 1) {
                let curr = providers.get(j).unwrap();
                let next = providers.get(j + 1).unwrap();

                if curr.1.success_rate < next.1.success_rate {
                    // Swap
                    let temp = curr.clone();
                    providers.set(j, next);
                    providers.set(j + 1, temp);
                }
            }
        }

        // Return top N
        let result_len = if limit < len { limit } else { len };
        let mut result = Vec::new(&env);
        for i in 0..result_len {
            result.push_back(providers.get(i).unwrap());
        }

        result
    }

    /* =========================
       SIGNAL ADOPTION (Issue #169)
    ========================== */

    pub fn increment_adoption(
        env: Env,
        caller: Address,
        signal_id: u64,
        nonce: u64,
    ) -> Result<u32, AdminError> {
        caller.require_auth();
        let executor_address: Address = env
            .storage()
            .instance()
            .get(&StorageKey::TradeExecutor)
            .ok_or(AdminError::Unauthorized)?;
        if caller != executor_address {
            return Err(AdminError::Unauthorized);
        }

        // Check nonce to prevent double-increment
        let nonce_key = (signal_id, nonce);
        let nonces: Map<(u64, u64), bool> = env
            .storage()
            .instance()
            .get(&StorageKey::AdoptionNonces)
            .unwrap_or(Map::new(&env));
        if nonces.contains_key(nonce_key.clone()) {
            return Err(AdminError::InvalidParameter); // Already incremented
        }

        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals.get(signal_id).ok_or(AdminError::InvalidParameter)?;

        if signal.status != SignalStatus::Active {
            return Err(AdminError::InvalidParameter);
        }

        signal.adoption_count = signal
            .adoption_count
            .checked_add(1)
            .ok_or(AdminError::InvalidParameter)?;
        signals.set(signal_id, signal.clone());
        Self::save_signals_map(&env, &signals);

        // Save nonce
        let mut nonces = nonces;
        nonces.set(nonce_key, true);
        env.storage()
            .instance()
            .set(&StorageKey::AdoptionNonces, &nonces);

        // Analytics: signal swiped (copy-trade initiation, before execution)
        shared::events::emit_signal_swiped(
            &env,
            shared::events::EvtSignalSwiped {
                schema_version: shared::events::SCHEMA_VERSION,
                user: caller.clone(),
                signal_id,
                timestamp: env.ledger().timestamp(),
            },
        );

        // Emit event
        events::emit_signal_adopted(&env, signal_id, caller.clone(), signal.adoption_count);

        Ok(signal.adoption_count)
    }

    /* =========================
       FEE MANAGEMENT FUNCTIONS
    ========================== */

    pub fn set_platform_treasury(
        env: Env,
        caller: Address,
        treasury: Address,
    ) -> Result<(), AdminError> {
        admin::require_admin(&env, &caller)?;
        caller.require_auth();
        fees::set_platform_treasury(&env, treasury);
        Ok(())
    }

    pub fn get_platform_treasury(env: Env) -> Option<Address> {
        fees::get_platform_treasury(&env)
    }

    pub fn get_treasury_balance(env: Env, asset: Asset) -> i128 {
        fees::get_treasury_balance(&env, asset)
    }

    pub fn get_all_treasury_balances(env: Env) -> Map<Asset, i128> {
        fees::get_all_treasury_balances(&env)
    }

    pub fn calculate_fee_preview(
        _env: Env,
        trade_amount: i128,
    ) -> Result<FeeBreakdown, errors::FeeError> {
        fees::calculate_fee_breakdown(trade_amount)
    }

    /* =========================
       API: QUERY SIGNALS
    ========================== */

    /// Get all active (non-expired) signals for feed, paginated and sorted.
    /// Supports optional category_filter for efficient per-category queries via index.
    pub fn get_active_signals(
        env: Env,
        offset: u32,
        limit: u32,
        sort_by: SortOption,
        provider: Option<Address>,
        category_filter: Option<SignalCategory>,
    ) -> Vec<SignalSummary> {
        let signals_map = Self::get_signals_map(&env);
        query::get_active_signals(
            &env,
            &signals_map,
            provider,
            offset,
            limit,
            sort_by,
            category_filter,
        )
    }

    /// Legacy fallback if front-ends rely on Old behavior
    /// (Wait, let's keep it as another name if needed, or just let users migrate to the new `get_active_signals`)
    pub fn get_active_signals_archived(
        env: Env,
        user: Address,
        followed_only: bool,
    ) -> Vec<Signal> {
        let signals = Self::get_signals_map(&env);
        if followed_only {
            let followed = social::get_followed_providers(&env, &user);
            expiry::get_active_signals_filtered(&env, &signals, &followed)
        } else {
            expiry::get_active_signals(&env, &signals)
        }
    }

    /* =========================
       SOCIAL / FOLLOW FUNCTIONS
    ========================== */

    /// Follow a provider. Idempotent if already following.
    pub fn follow_provider(env: Env, user: Address, provider: Address) -> Result<(), AdminError> {
        // Rate limit: follow actions
        let trust = reputation::get_trust_score(&env, &user)
            .map(|d| d.score)
            .unwrap_or(0);
        rl::check_rate_limit(&env, &user, RLAction::FollowAction, trust)
            .map_err(|_| AdminError::RateLimitExceeded)?;
        rl::record_action(&env, &user, RLAction::FollowAction);

        social::follow_provider(&env, user.clone(), provider.clone())
            .map_err(|_| AdminError::CannotFollowSelf)?;

        // Update trust score when follower count changes
        Self::update_provider_trust_score(env, provider);

        Ok(())
    }

    /// Unfollow a provider. No error if not following.
    pub fn unfollow_provider(env: Env, user: Address, provider: Address) -> Result<(), AdminError> {
        social::unfollow_provider(&env, user, provider.clone())
            .map_err(|_| AdminError::Unauthorized)?;

        // Update trust score when follower count changes
        Self::update_provider_trust_score(env, provider);

        Ok(())
    }

    /// Get list of providers user follows
    pub fn get_followed_providers(env: Env, user: Address) -> Vec<Address> {
        social::get_followed_providers(&env, &user)
    }

    /// Get follower count for a provider
    pub fn get_follower_count(env: Env, provider: Address) -> u32 {
        social::get_follower_count(&env, &provider)
    }

    /// Cleanup expired signals in batches
    /// Returns (signals_processed, signals_expired)
    pub fn cleanup_expired_signals(env: Env, limit: u32) -> (u32, u32) {
        let signals = Self::get_signals_map(&env);
        let result = expiry::cleanup_expired_signals(&env, &signals, limit);
        (result.signals_processed, result.signals_expired)
    }

    /// Archive old expired signals (30+ days old)
    /// Returns number of signals archived
    pub fn archive_old_signals(env: Env, limit: u32) -> u32 {
        let signals = Self::get_signals_map(&env);
        expiry::archive_old_signals(&env, &signals, limit)
    }

    /// Get count of expired signals
    pub fn get_expired_count(env: Env) -> u32 {
        let signals = Self::get_signals_map(&env);
        expiry::count_expired_signals(&signals)
    }

    /// Get count of signals pending expiry (past expiry time but not marked yet)
    pub fn get_pending_expiry_count(env: Env) -> u32 {
        let signals = Self::get_signals_map(&env);
        expiry::count_signals_pending_expiry(&env, &signals)
    }

    //  ANALYTICS FUNCTIONS

    /// Get provider analytics (requires min 10 signals)
    pub fn get_provider_analytics(
        env: Env,
        provider: Address,
    ) -> Option<analytics::ProviderAnalytics> {
        let signals = Self::get_signals_map(&env);
        analytics::calculate_provider_analytics(&env, &signals, &provider)
    }

    /// Get trending asset pairs in last N hours
    pub fn get_trending_assets(env: Env, window_hours: u64) -> Vec<(String, u32)> {
        let signals = Self::get_signals_map(&env);
        analytics::get_trending_assets(&env, &signals, window_hours)
    }

    /// Get global analytics (24h metrics)
    pub fn get_global_analytics(env: Env) -> analytics::GlobalAnalytics {
        let signals = Self::get_signals_map(&env);
        analytics::calculate_global_analytics(&env, &signals)
    }

    /* =========================
       CATEGORIZATION & TAGGING FUNCTIONS
    ========================== */

    /// Add tags to an existing signal
    pub fn add_tags_to_signal(
        env: Env,
        provider: Address,
        signal_id: u64,
        tags: Vec<String>,
    ) -> Result<(), AdminError> {
        provider.require_auth();

        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals.get(signal_id).ok_or(AdminError::InvalidParameter)?;

        // Verify provider owns the signal
        if signal.provider != provider {
            return Err(AdminError::Unauthorized);
        }

        // Validate new tags
        categories::validate_tags(&tags)?;

        // Check total tag count
        if signal.tags.len() + tags.len() > 10 {
            return Err(AdminError::InvalidParameter);
        }

        // Add tags (deduplicate)
        let mut combined = Vec::new(&env);
        for i in 0..signal.tags.len() {
            combined.push_back(signal.tags.get(i).unwrap());
        }
        for i in 0..tags.len() {
            combined.push_back(tags.get(i).unwrap());
        }

        signal.tags = categories::deduplicate_tags(&env, combined);
        let tag_count = signal.tags.len();
        signals.set(signal_id, signal);
        Self::save_signals_map(&env, &signals);

        // Update tag popularity
        categories::increment_tag_popularity(&env, &tags);

        // Emit event
        events::emit_tags_added(&env, signal_id, provider, tag_count);

        Ok(())
    }

    /// Get signals filtered by categories, tags, and risk levels
    pub fn get_signals_filtered(
        env: Env,
        categories: Option<Vec<SignalCategory>>,
        tags: Option<Vec<String>>,
        risk_levels: Option<Vec<RiskLevel>>,
        offset: u32,
        limit: u32,
    ) -> Vec<Signal> {
        let signals_map = Self::get_signals_map(&env);
        let mut filtered = Vec::new(&env);
        let now = env.ledger().timestamp();

        // Collect active signals
        for key in signals_map.keys() {
            if let Some(signal) = signals_map.get(key) {
                if signal.status == SignalStatus::Active && signal.expiry > now {
                    filtered.push_back(signal);
                }
            }
        }

        // Filter by categories
        if let Some(cats) = categories {
            let mut temp = Vec::new(&env);
            for i in 0..filtered.len() {
                let signal = filtered.get(i).unwrap();
                for j in 0..cats.len() {
                    if signal.category == cats.get(j).unwrap() {
                        temp.push_back(signal);
                        break;
                    }
                }
            }
            filtered = temp;
        }

        // Filter by tags (any match)
        if let Some(tags_filter) = tags {
            let mut temp = Vec::new(&env);
            for i in 0..filtered.len() {
                let signal = filtered.get(i).unwrap();
                let mut has_tag = false;
                for j in 0..tags_filter.len() {
                    let filter_tag = tags_filter.get(j).unwrap();
                    for k in 0..signal.tags.len() {
                        if signal.tags.get(k).unwrap().to_bytes() == filter_tag.to_bytes() {
                            has_tag = true;
                            break;
                        }
                    }
                    if has_tag {
                        break;
                    }
                }
                if has_tag {
                    temp.push_back(signal);
                }
            }
            filtered = temp;
        }

        // Filter by risk levels
        if let Some(risks) = risk_levels {
            let mut temp = Vec::new(&env);
            for i in 0..filtered.len() {
                let signal = filtered.get(i).unwrap();
                for j in 0..risks.len() {
                    if signal.risk_level == risks.get(j).unwrap() {
                        temp.push_back(signal);
                        break;
                    }
                }
            }
            filtered = temp;
        }

        // Paginate
        let total = filtered.len();
        let start = offset.min(total);
        let end = (offset + limit).min(total);

        let mut result = Vec::new(&env);
        for i in start..end {
            result.push_back(filtered.get(i).unwrap());
        }

        result
    }

    /// Get popular tags
    pub fn get_popular_tags(env: Env, limit: u32) -> Vec<(String, u32)> {
        categories::get_popular_tags(&env, limit)
    }

    /// Auto-suggest tags based on signal rationale
    pub fn suggest_tags(env: Env, rationale: String) -> Vec<String> {
        categories::auto_suggest_tags(&env, &rationale)
    }

    /* =======
       SIGNAL IMPORT FUNCTIONS
    ========================== */

    /// Import signals from CSV format
    pub fn import_signals_csv(
        env: Env,
        provider: Address,
        data: Bytes,
        validate_only: bool,
    ) -> ImportResultView {
        provider.require_auth();

        let result = import::import_signals_csv(&env, &provider, data, validate_only);

        ImportResultView {
            success_count: result.success_count,
            error_count: result.error_count,
            signal_ids: Vec::new(&env),
        }
    }

    /// Import signals from JSON format
    pub fn import_signals_json(
        env: Env,
        provider: Address,
        data: Bytes,
        validate_only: bool,
    ) -> ImportResultView {
        provider.require_auth();

        let result = import::import_signals_json(&env, &provider, data, validate_only);

        ImportResultView {
            success_count: result.success_count,
            error_count: result.error_count,
            signal_ids: Vec::new(&env),
        }
    }

    /// Get signal ID by external ID
    pub fn get_signal_by_external_id(
        env: Env,
        provider: Address,
        external_id: String,
    ) -> Option<u64> {
        import::get_signal_by_external_id(&env, &provider, &external_id)
    }

    /* =========================
       COLLABORATION FUNCTIONS
    ========================== */

    pub fn create_collaborative_signal(
        env: Env,
        primary_author: Address,
        co_authors: Vec<Address>,
        contribution_pcts: Vec<u32>,
        asset_pair: String,
        action: SignalAction,
        price: i128,
        rationale: String,
        expiry: u64,
    ) -> Result<u64, AdminError> {
        primary_author.require_auth();

        let category = SignalCategory::SWING;
        let tags = Vec::new(&env);
        let risk_level = RiskLevel::Medium;

        let signal_id = Self::create_signal_internal(
            &env,
            primary_author.clone(),
            asset_pair,
            action,
            price,
            rationale,
            expiry,
            category,
            tags,
            risk_level,
        )?;

        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals.get(signal_id).unwrap();
        signal.is_collaborative = true;
        signal.status = SignalStatus::Pending;
        signals.set(signal_id, signal);
        Self::save_signals_map(&env, &signals);

        collaboration::create_collaborative_signal(
            &env,
            signal_id,
            primary_author,
            co_authors.clone(),
            contribution_pcts,
        )?;

        events::emit_collaborative_signal_created(&env, signal_id, co_authors);
        Ok(signal_id)
    }

    pub fn approve_collaborative_signal(
        env: Env,
        signal_id: u64,
        approver: Address,
    ) -> Result<(), AdminError> {
        approver.require_auth();

        let all_approved = collaboration::approve_collaborative_signal(&env, signal_id, &approver)?;
        events::emit_collaborative_signal_approved(&env, signal_id, approver);

        if all_approved {
            let mut signals = Self::get_signals_map(&env);
            let mut signal = signals.get(signal_id).ok_or(AdminError::InvalidParameter)?;
            signal.status = SignalStatus::Active;
            signals.set(signal_id, signal);
            Self::save_signals_map(&env, &signals);
            events::emit_collaborative_signal_published(&env, signal_id);
        }

        Ok(())
    }

    pub fn get_collaboration_details(
        env: Env,
        signal_id: u64,
    ) -> Option<Vec<collaboration::Author>> {
        collaboration::get_collaborative_signal(&env, signal_id)
    }

    pub fn is_collaborative_signal(env: Env, signal_id: u64) -> bool {
        collaboration::is_collaborative_signal(&env, signal_id)
    }

    /* =========================
       COMBO SIGNAL FUNCTIONS
    ========================== */

    /// Create a combo signal linking multiple component signals.
    ///
    /// All component signals must belong to `provider` and be Active.
    /// Component weights must sum to exactly 10 000 (100% in basis points).
    pub fn create_combo_signal(
        env: Env,
        provider: Address,
        name: String,
        components: Vec<ComponentSignal>,
        combo_type: ComboType,
    ) -> Result<u64, ComboError> {
        provider.require_auth();

        let count = components.len();
        let combo_id = create_combo_signal(&env, &provider, name, components, combo_type)?;

        events::emit_combo_created(&env, combo_id, provider, count);

        Ok(combo_id)
    }

    /// Execute a combo signal, distributing `total_amount` across components
    /// according to their weights and the combo type.
    pub fn execute_combo_signal(
        env: Env,
        combo_id: u64,
        user: Address,
        total_amount: i128,
    ) -> Result<Vec<ComponentExecution>, ComboError> {
        user.require_auth();

        let executions = execute_combo_signal(&env, combo_id, &user, total_amount)?;

        // Calculate combined ROI for the event (already stored, re-derive for event)
        let execs_stored = get_combo_executions_pub(&env, combo_id);
        let combined_roi =
            if let Some(last) = execs_stored.get(execs_stored.len().saturating_sub(1)) {
                last.combined_roi
            } else {
                0
            };

        events::emit_combo_executed(&env, combo_id, user, combined_roi);

        Ok(executions)
    }

    /// Cancel an active combo. Only the provider who created it may cancel.
    pub fn cancel_combo_signal(
        env: Env,
        combo_id: u64,
        provider: Address,
    ) -> Result<(), ComboError> {
        provider.require_auth();
        cancel_combo(&env, combo_id, &provider)?;
        events::emit_combo_cancelled(&env, combo_id, provider);
        Ok(())
    }

    /// Retrieve a combo signal by ID.
    pub fn get_combo_signal(env: Env, combo_id: u64) -> Option<ComboSignal> {
        get_combo(&env, combo_id)
    }

    /// Get aggregated performance metrics for a combo.
    pub fn get_combo_performance(env: Env, combo_id: u64) -> Option<ComboPerformanceSummary> {
        get_combo_performance(&env, combo_id)
    }

    /// Get the full execution history for a combo.
    pub fn get_combo_executions(env: Env, combo_id: u64) -> Vec<ComboExecution> {
        get_combo_executions_pub(&env, combo_id)
    }

    /* =========================
       CONTEST FUNCTIONS
    ========================== */

    /// Create a new contest
    pub fn create_contest(
        env: Env,
        admin: Address,
        name: String,
        start_time: u64,
        end_time: u64,
        metric: ContestMetric,
        min_signals: u32,
        prize_pool: i128,
    ) -> Result<u64, ContestError> {
        admin.require_auth();

        require_not_paused(&env).map_err(|e| match e {
            AdminError::TradingPaused => ContestError::TradingPaused,
            AdminError::CircuitBreakerTriggered => ContestError::CircuitBreakerTriggered,
            _ => ContestError::ContestNotFound,
        })?;
        contests::create_contest(
            &env,
            name,
            start_time,
            end_time,
            metric,
            min_signals,
            prize_pool,
        )
    }

    /// Finalize a contest and distribute prizes
    pub fn finalize_contest(env: Env, contest_id: u64) -> Result<Vec<Address>, ContestError> {
        contests::finalize_contest(&env, contest_id)
    }

    /// Get contest details
    pub fn get_contest(env: Env, contest_id: u64) -> Result<Contest, ContestError> {
        contests::get_contest(&env, contest_id)
    }

    /// Get all active contests
    pub fn get_active_contests(env: Env) -> Vec<u64> {
        contests::get_active_contests(&env)
    }

    /// Get contest leaderboard
    pub fn get_contest_leaderboard(
        env: Env,
        contest_id: u64,
    ) -> Result<Vec<ContestEntry>, ContestError> {
        contests::get_contest_leaderboard(&env, contest_id)
    }

    /// Get provider's prize for a contest
    pub fn get_provider_prize(env: Env, contest_id: u64, provider: Address) -> i128 {
        contests::get_provider_prize(&env, contest_id, provider)
    }

    /* =========================
       VERSIONING FUNCTIONS
    ========================== */

    /// Versioned update (price / rationale / expiry) with history (legacy versioning API).
    pub fn update_signal_versioned(
        env: Env,
        signal_id: u64,
        updater: Address,
        new_price: Option<i128>,
        new_rationale: Option<String>,
        new_expiry: Option<u64>,
    ) -> Result<u32, VersioningError> {
        updater.require_auth();
        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals
            .get(signal_id)
            .ok_or(VersioningError::VersionNotFound)?;

        let new_version = versioning::update_signal(
            &env,
            signal_id,
            &updater,
            new_price,
            new_rationale,
            new_expiry,
            &mut signal,
        )?;

        signals.set(signal_id, signal);
        Self::save_signals_map(&env, &signals);

        Ok(new_version)
    }

    /// Get version history for a signal
    pub fn get_signal_history(env: Env, signal_id: u64) -> Vec<SignalVersion> {
        versioning::get_signal_history(&env, signal_id)
    }

    /// Record when a user copies a signal
    pub fn record_signal_copy(env: Env, user: Address, signal_id: u64) {
        user.require_auth();
        let version = versioning::get_latest_version(&env, signal_id);
        versioning::record_copy(&env, &user, signal_id, version);
    }

    /// Get pending updates for a user's copied signal
    pub fn get_pending_updates(env: Env, user: Address, signal_id: u64) -> Vec<u32> {
        versioning::get_pending_updates(&env, &user, signal_id)
    }

    /// Get copy record for a user
    pub fn get_copy_record(env: Env, user: Address, signal_id: u64) -> Option<CopyRecord> {
        versioning::get_copy_record(&env, &user, signal_id)
    }

    /// Mark user as notified of an update
    pub fn mark_update_notified(env: Env, user: Address, signal_id: u64, version: u32) {
        versioning::mark_notified(&env, &user, signal_id, version);
    }

    /* =========================
       CROSS-CHAIN SYNC FUNCTIONS
    ========================== */

    pub fn register_cross_chain_address(
        env: Env,
        stellar_address: Address,
        source_chain: String,
        source_address: String,
        proof: Bytes,
    ) -> Result<(), AdminError> {
        stellar_address.require_auth();
        cross_chain::register_address(
            &env,
            stellar_address.clone(),
            source_chain.clone(),
            source_address.clone(),
            proof,
        );
        events::emit_cross_chain_address_registered(
            &env,
            source_chain,
            source_address,
            stellar_address,
        );
        Ok(())
    }

    pub fn request_signal_import(
        env: Env,
        provider: Address,
        source_chain: String,
        source_id: String,
        source_address: String,
        proof: Bytes,
    ) -> Result<(), CrossChainError> {
        provider.require_auth();

        // Ensure address mapping exists
        let mapping = cross_chain::get_address_mapping(&env, &source_chain, &source_address)
            .ok_or(CrossChainError::AddressNotRegistered)?;

        if mapping.stellar_address != provider {
            return Err(CrossChainError::NotSignalOwner);
        }

        if cross_chain::get_cross_chain_signal(&env, &source_chain, &source_id).is_some() {
            return Err(CrossChainError::SignalAlreadyExists);
        }

        let signal = CrossChainSignal {
            source_chain: source_chain.clone(),
            source_signal_id: source_id.clone(),
            stellar_signal_id: 0,
            provider_source_address: source_address,
            stellar_address: provider.clone(),
            verification_proof: proof,
            sync_status: SyncStatus::Pending,
        };

        cross_chain::store_cross_chain_signal(
            &env,
            source_chain.clone(),
            source_id.clone(),
            signal,
        );
        events::emit_cross_chain_signal_requested(&env, source_chain, source_id, provider);

        Ok(())
    }

    pub fn import_verified_signal(
        env: Env,
        source_chain: String,
        source_id: String,
        asset_pair: String,
        action: SignalAction,
        price: i128,
        rationale: String,
        expiry: u64,
    ) -> Result<u64, CrossChainError> {
        let mut cc_signal = cross_chain::get_cross_chain_signal(&env, &source_chain, &source_id)
            .ok_or(CrossChainError::SignalNotFound)?;

        if cc_signal.sync_status != SyncStatus::Pending {
            return Err(CrossChainError::InvalidSyncStatus);
        }

        // Verify proof (placeholder)
        if !cross_chain::verify_proof(&env, &cc_signal.verification_proof) {
            cc_signal.sync_status = SyncStatus::Failed;
            cross_chain::store_cross_chain_signal(
                &env,
                source_chain.clone(),
                source_id.clone(),
                cc_signal,
            );
            return Err(CrossChainError::VerificationFailed);
        }

        // Create the signal on Stellar
        let category = SignalCategory::SWING;
        let tags = Vec::new(&env);
        let risk_level = RiskLevel::Medium;

        let stellar_id = Self::create_signal_internal(
            &env,
            cc_signal.stellar_address.clone(),
            asset_pair,
            action,
            price,
            rationale,
            expiry,
            category,
            tags,
            risk_level,
        )
        .map_err(|_| CrossChainError::InvalidProof)?;

        cc_signal.stellar_signal_id = stellar_id;
        cc_signal.sync_status = SyncStatus::Imported;
        cross_chain::store_cross_chain_signal(
            &env,
            source_chain.clone(),
            source_id.clone(),
            cc_signal,
        );

        events::emit_cross_chain_signal_imported(&env, source_chain, source_id, stellar_id);

        Ok(stellar_id)
    }

    pub fn sync_signal_update(
        env: Env,
        source_chain: String,
        source_id: String,
        new_price: Option<i128>,
        new_rationale: Option<String>,
    ) -> Result<(), CrossChainError> {
        let cc_signal = cross_chain::get_cross_chain_signal(&env, &source_chain, &source_id)
            .ok_or(CrossChainError::SignalNotFound)?;

        if cc_signal.sync_status != SyncStatus::Imported {
            return Err(CrossChainError::InvalidSyncStatus);
        }

        let mut signals = Self::get_signals_map(&env);
        let mut signal = signals
            .get(cc_signal.stellar_signal_id)
            .ok_or(CrossChainError::SignalNotFound)?;

        if let Some(price) = new_price {
            signal.price = price;
        }
        if let Some(rat) = new_rationale {
            signal.rationale = rat;
        }

        signals.set(cc_signal.stellar_signal_id, signal.clone());
        Self::save_signals_map(&env, &signals);

        events::emit_cross_chain_signal_synced(&env, source_chain, source_id, signal.status as u32);

        Ok(())
    }

    /* =========================
       TRUST SCORE FUNCTIONS
    ========================== */

    /// Get trust score for a provider
    ///
    /// Returns None if provider has insufficient history (< 5 signals)
    /// Trust score ranges from 0-100 with tier classifications
    pub fn get_provider_trust_score(env: Env, provider: Address) -> Option<TrustScoreDetails> {
        let performance = Self::get_provider_stats(env.clone(), provider.clone())?;
        let stake_info = stake::get_stake_info(&env, &provider);

        Some(calculate_trust_score(
            &env,
            &provider,
            &performance,
            &stake_info,
        ))
    }

    /// Update trust score for a provider (called after performance changes)
    ///
    /// This should be called when:
    /// - Signal status changes (success/failure)
    /// - Follower count changes
    /// - Stake amount changes
    pub fn update_provider_trust_score(env: Env, provider: Address) -> Option<TrustScoreDetails> {
        let performance = Self::get_provider_stats(env.clone(), provider.clone())?;
        let stake_info = stake::get_stake_info(&env, &provider);

        let score_details = calculate_trust_score(&env, &provider, &performance, &stake_info);
        reputation::store_trust_score(&env, &provider, &score_details);

        Some(score_details)
    }

    /// Get leaderboard sorted by trust score
    ///
    /// Returns providers with trust scores, sorted by score descending
    /// Only includes providers with sufficient history (>= 5 signals)
    pub fn get_trust_score_leaderboard(env: Env, limit: u32) -> Vec<(Address, TrustScoreDetails)> {
        // This is a simplified implementation
        // In production, you'd want to cache this or use a more efficient data structure
        let stats_map = Self::get_provider_stats_map(&env);
        let mut providers_with_scores = Vec::new(&env);

        // Collect providers with sufficient history
        for key in stats_map.keys() {
            if let Some(performance) = stats_map.get(key.clone()) {
                if performance.total_signals >= 5 {
                    // MIN_SIGNALS_FOR_TRUST_SCORE
                    let stake_info = stake::get_stake_info(&env, &key);
                    let score_details =
                        calculate_trust_score(&env, &key, &performance, &stake_info);
                    providers_with_scores.push_back((key, score_details));
                }
            }
        }

        // Sort by trust score descending (simple bubble sort)
        let len = providers_with_scores.len();
        for i in 0..len {
            for j in 0..(len - i - 1) {
                let curr = providers_with_scores.get(j).unwrap();
                let next = providers_with_scores.get(j + 1).unwrap();

                if curr.1.score < next.1.score {
                    // Swap
                    let temp = curr.clone();
                    providers_with_scores.set(j, next);
                    providers_with_scores.set(j + 1, temp);
                }
            }
        }

        // Return top N
        let result_len = if limit > 0 && limit < len { limit } else { len };
        let mut result = Vec::new(&env);
        for i in 0..result_len {
            result.push_back(providers_with_scores.get(i).unwrap());
        }

        result
    }

    /// Update global median values for trust score normalization
    ///
    /// Should be called periodically by admin to recalculate medians
    /// This affects stake and follower normalization across all providers
    pub fn update_trust_score_medians(
        env: Env,
        caller: Address,
        median_stake: i128,
        median_followers: u64,
    ) -> Result<(), AdminError> {
        admin::require_admin(&env, &caller)?;
        caller.require_auth();

        update_median_values(&env, median_stake, median_followers);
        Ok(())
    }

    /// Get trust score tier distribution
    ///
    /// Returns count of providers in each trust score tier
    pub fn get_trust_score_distribution(env: Env) -> (u32, u32, u32, u32) {
        let stats_map = Self::get_provider_stats_map(&env);
        let mut highly_trusted = 0u32;
        let mut trusted = 0u32;
        let mut emerging = 0u32;
        let mut new_unproven = 0u32;

        for key in stats_map.keys() {
            if let Some(performance) = stats_map.get(key.clone()) {
                if performance.total_signals >= 5 {
                    let stake_info = stake::get_stake_info(&env, &key);
                    let score_details =
                        calculate_trust_score(&env, &key, &performance, &stake_info);

                    match score_details.tier {
                        TrustScoreTier::HighlyTrusted => highly_trusted += 1,
                        TrustScoreTier::Trusted => trusted += 1,
                        TrustScoreTier::Emerging => emerging += 1,
                        TrustScoreTier::NewUnproven => new_unproven += 1,
                    }
                }
            }
        }

        (highly_trusted, trusted, emerging, new_unproven)
    }

    /* =========================
       STORAGE STATS (Issue #3)
    ========================== */

    /// Returns estimated storage usage metrics.
    ///
    /// # Estimation methodology
    /// - `total_signals`: exact count from Signals map.
    /// - `total_providers`: exact count from ProviderStats map.
    /// - `total_positions`: approximated as total_signals × avg_executions_per_signal (2).
    /// - `estimated_rent_xlm`: entry_count × avg_entry_size_bytes × RENT_RATE_XLM_PER_BYTE.
    ///   avg_entry_size ≈ 256 bytes; rent_rate ≈ 0.00001 XLM/byte (Soroban Protocol 23).
    ///   Result is in stroops (1 XLM = 10_000_000 stroops).
    ///
    /// # Rent cost projection for 10,000 users
    /// Assuming 5 signals/user → 50,000 signal entries + 10,000 provider entries = 60,000 entries.
    /// 60,000 × 256 bytes × 0.00001 XLM/byte ≈ 153.6 XLM total rent.
    pub fn get_storage_stats(env: Env) -> StorageStats {
        let signals = Self::get_signals_map(&env);
        let providers = Self::get_provider_stats_map(&env);

        let total_signals = signals.len();
        let total_providers = providers.len();
        // Approximate: each signal averages 2 trade executions stored
        let total_positions = total_signals.saturating_mul(2);

        // Rent estimate: entries × 256 bytes × 100 stroops/byte
        let entry_count = (total_signals + total_providers) as i128;
        let estimated_rent_xlm = entry_count * 256 * 100;

        StorageStats {
            total_signals,
            total_positions,
            total_providers,
            estimated_rent_xlm,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageStats {
    pub total_signals: u32,
    pub total_positions: u32,
    pub total_providers: u32,
    /// Estimated rent in stroops (1 XLM = 10_000_000 stroops).
    pub estimated_rent_xlm: i128,
}

#[cfg(test)]
mod test;
#[cfg(test)]
mod test_adoption;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod test_admin_transfer;
#[cfg(test)]
mod test_adoption;
#[cfg(test)]
mod test_emergency;
#[cfg(test)]
mod test_health;
#[cfg(test)]
mod test_scheduling;
#[cfg(test)]
mod test_signal_issues;
#[cfg(test)]
mod test_admin_transfer;
