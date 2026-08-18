use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AutoTradeError {
    // ── Core trade errors ────────────────────────────────────────────────────
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 1,
    /// Caller is not authorized to perform this action.
    Unauthorized = 2,
    /// No trade signal exists for the given id.
    SignalNotFound = 3,
    /// The referenced trade signal has expired.
    SignalExpired = 4,
    /// Caller's balance is lower than the amount required for this trade.
    InsufficientBalance = 5,
    /// Available liquidity is insufficient to fill the requested trade.
    InsufficientLiquidity = 6,
    /// Trade would push the rolling daily volume past the configured limit.
    DailyTradeLimitExceeded = 7,
    /// Caller already holds the maximum number of allowed open positions.
    PositionLimitExceeded = 8,
    /// Trade was blocked because a stop-loss threshold was triggered.
    StopLossTriggered = 9,
    /// No auto-trade strategy exists for the given id.
    StrategyNotFound = 10,
    /// An active position already exists; cannot open a duplicate.
    PositionAlreadyExists = 11,
    /// Not enough historical price data to evaluate the strategy.
    InsufficientPriceHistory = 12,
    /// Signal ranking is currently disabled.
    RankingDisabled = 13,
    /// Caller has exceeded the allowed rate of operations.
    RateLimited = 14,
    /// Action is blocked because privacy mode is enabled for this account.
    PrivacyModeEnabled = 15,
    /// Trading is currently paused for this account/contract.
    TradingPaused = 16,
    // ── Portfolio / stat-arb ─────────────────────────────────────────────────
    /// Statistical-arbitrage basket size is outside the allowed range.
    InvalidBasketSize = 17,
    /// Supplied price data is malformed or fails validation.
    InvalidPriceData = 18,
    /// Assets in the basket fail the cointegration test required for stat-arb.
    NonCointegratedBasket = 19,
    /// An active stat-arb portfolio already exists for this caller.
    ActivePortfolioExists = 20,
    /// No active stat-arb portfolio exists for this caller.
    NoActivePortfolio = 21,
    /// No trade signal is currently available for this portfolio/strategy.
    NoTradeSignal = 22,
    /// Statistical-arbitrage configuration is invalid.
    InvalidStatArbConfig = 23,
    // ── Exit / insurance ─────────────────────────────────────────────────────
    /// No exit strategy exists for the given id.
    ExitStrategyNotFound = 24,
    /// Exit-strategy configuration is invalid.
    InvalidExitConfig = 25,
    /// Insurance has not been configured for this position/account.
    InsuranceNotConfigured = 26,
    /// Insurance configuration is invalid.
    InvalidInsuranceConfig = 27,
    // ── Referral (SelfReferral / AlreadySet / Circular / LimitExceeded) ──────
    /// Umbrella variant for referral failures; see the alias consts below for
    /// the specific referral condition (self-referral, already set, circular, limit).
    ReferralError = 28,
    // ── TWAP (InvalidDuration / NotFound / NotOwner / NotActive) ─────────────
    /// Umbrella variant for TWAP order failures; see the alias consts below
    /// for the specific condition (invalid duration, not found, not owner, not active).
    TWAPError = 29,
    // ── Correlation ──────────────────────────────────────────────────────────
    /// Portfolio correlation exceeds the configured maximum.
    CorrelationLimitExceeded = 30,
    /// Too many positions are open in highly correlated assets.
    TooManyCorrelatedPositions = 31,
    // ── Conditional orders (NotFound / NotPending / NotTriggered / Config) ───
    /// Umbrella variant for conditional-order failures; see the alias consts
    /// below for the specific condition (not found, not pending, not triggered).
    ConditionalOrderError = 32,
    /// Conditional-order configuration is invalid.
    InvalidConditionalConfig = 33,
    // ── Rate limits (all sub-types collapsed) ────────────────────────────────
    /// Umbrella variant for rate-limit failures; see the alias consts below
    /// for the specific limit that was exceeded (hourly/daily transfer/volume, etc).
    RateLimitExceeded = 34,
    // ── Pairs trading ────────────────────────────────────────────────────────
    /// No pairs-trading strategy exists for the given id.
    PairsStrategyNotFound = 35,
    /// Umbrella variant for pairs-position failures; see the alias consts
    /// below for the specific condition (active position exists / no active position).
    PairsPositionError = 36,
    /// Correlation between the pair's assets is below the required minimum.
    InsufficientCorrelation = 37,
    /// The pair fails the cointegration test required for pairs trading.
    PairNotCointegrated = 38,
    /// Pairs-trading configuration is invalid.
    InvalidPairsConfig = 39,
    // ── Oracle ───────────────────────────────────────────────────────────────
    /// No oracle price is currently available for this asset.
    OracleUnavailable = 40,
    // ── DCA (NotFound / Inactive / EndTimeReached) ────────────────────────────
    /// Umbrella variant for DCA (dollar-cost-averaging) failures; see the
    /// alias consts below for the specific condition (not found, inactive, end time reached).
    DcaError = 41,
    // ── Mean-reversion (NotFound / InsufficientHistory / LowVolatility) ──────
    /// Umbrella variant for mean-reversion strategy failures; see the alias
    /// consts below for the specific condition (not found, insufficient history, low volatility).
    MrStrategyError = 42,
    // ── Admin transfer ───────────────────────────────────────────────────────
    /// Umbrella variant for admin-transfer failures; see the alias consts
    /// below for the specific condition (no pending request, request expired).
    AdminTransferError = 43,
    // ── Routing ──────────────────────────────────────────────────────────────
    /// No routing plan exists for the given id.
    RoutingPlanNotFound = 44,
    // ── Arbitrage ────────────────────────────────────────────────────────────
    /// Umbrella variant for arbitrage failures; see the alias consts below
    /// for the specific condition (opportunity expired, unprofitable, too large).
    ArbitrageError = 45,
    /// Trade was blocked due to detected front-running risk.
    FrontRunningRisk = 46,
    // ── System / bridge / recovery ───────────────────────────────────────────
    /// Umbrella variant for system-level failures; see the alias consts below
    /// for the specific condition (atomic execution failed, bridge paused, recovery, etc).
    SystemError = 47,
    /// Executed price would deviate from the reference price by more than the allowed slippage.
    SlippageExceeded = 48,
    // ── Misc ─────────────────────────────────────────────────────────────────
    /// Cannot remove the last remaining whitelisted oracle for a pair.
    LastOracleForPair = 49,
    /// Action requires the contract to be paused, but it is not.
    NotPaused = 50,
}

impl AutoTradeError {
    /// Short, human-readable description of when this error is returned.
    ///
    /// `AutoTradeError` is at the 50-variant XDR cap (see the alias-const
    /// block below), so many logical error names below (e.g.
    /// `AtomicExecutionFailed`, `TWAPOrderNotFound`) are `const` aliases for
    /// a shared underlying variant (e.g. `SystemError`, `TWAPError`) rather
    /// than distinct discriminants. Because an alias's runtime value is
    /// *identical* to its target variant's, `message()` necessarily returns
    /// the same string for both — it cannot distinguish which name the
    /// caller used. For the precise, alias-specific meaning, see the `///`
    /// doc comment on the individual alias const instead.
    pub fn message(&self) -> &'static str {
        match self {
            AutoTradeError::InvalidAmount => "amount is zero, negative, or outside allowed bounds",
            AutoTradeError::Unauthorized => "caller is not authorized to perform this action",
            AutoTradeError::SignalNotFound => "no trade signal exists for the given id",
            AutoTradeError::SignalExpired => "the referenced trade signal has expired",
            AutoTradeError::InsufficientBalance => {
                "caller's balance is lower than the amount required for this trade"
            }
            AutoTradeError::InsufficientLiquidity => {
                "available liquidity is insufficient to fill the requested trade"
            }
            AutoTradeError::DailyTradeLimitExceeded => {
                "trade would exceed the configured rolling daily volume limit"
            }
            AutoTradeError::PositionLimitExceeded => {
                "caller already holds the maximum number of allowed open positions"
            }
            AutoTradeError::StopLossTriggered => {
                "trade blocked: a stop-loss threshold was triggered"
            }
            AutoTradeError::StrategyNotFound => "no auto-trade strategy exists for the given id",
            AutoTradeError::PositionAlreadyExists => {
                "an active position already exists; cannot open a duplicate"
            }
            AutoTradeError::InsufficientPriceHistory => {
                "not enough historical price data to evaluate the strategy"
            }
            AutoTradeError::RankingDisabled => "signal ranking is currently disabled",
            AutoTradeError::RateLimited => "caller has exceeded the allowed rate of operations",
            AutoTradeError::PrivacyModeEnabled => {
                "action is blocked because privacy mode is enabled for this account"
            }
            AutoTradeError::TradingPaused => {
                "trading is currently paused for this account/contract"
            }
            AutoTradeError::InvalidBasketSize => {
                "statistical-arbitrage basket size is outside the allowed range"
            }
            AutoTradeError::InvalidPriceData => {
                "supplied price data is malformed or fails validation"
            }
            AutoTradeError::NonCointegratedBasket => {
                "basket assets fail the cointegration test required for stat-arb"
            }
            AutoTradeError::ActivePortfolioExists => {
                "an active stat-arb portfolio already exists for this caller"
            }
            AutoTradeError::NoActivePortfolio => {
                "no active stat-arb portfolio exists for this caller"
            }
            AutoTradeError::NoTradeSignal => {
                "no trade signal is currently available for this portfolio/strategy"
            }
            AutoTradeError::InvalidStatArbConfig => {
                "statistical-arbitrage configuration is invalid"
            }
            AutoTradeError::ExitStrategyNotFound => "no exit strategy exists for the given id",
            AutoTradeError::InvalidExitConfig => "exit-strategy configuration is invalid",
            AutoTradeError::InsuranceNotConfigured => {
                "insurance has not been configured for this position/account"
            }
            AutoTradeError::InvalidInsuranceConfig => "insurance configuration is invalid",
            AutoTradeError::ReferralError => {
                "referral operation failed; see SelfReferral/ReferralAlreadySet/\
                 CircularReferral/ReferralLimitExceeded for the specific cause"
            }
            AutoTradeError::TWAPError => {
                "TWAP order operation failed; see InvalidTWAPDuration/TWAPOrderNotFound/\
                 NotTWAPOwner/TWAPNotActive for the specific cause"
            }
            AutoTradeError::CorrelationLimitExceeded => {
                "portfolio correlation exceeds the configured maximum"
            }
            AutoTradeError::TooManyCorrelatedPositions => {
                "too many positions are open in highly correlated assets"
            }
            AutoTradeError::ConditionalOrderError => {
                "conditional order operation failed; see ConditionalOrderNotFound/\
                 ConditionalOrderNotPending/ConditionalOrderNotTriggered for the specific cause"
            }
            AutoTradeError::InvalidConditionalConfig => {
                "conditional-order configuration is invalid"
            }
            AutoTradeError::RateLimitExceeded => {
                "a rate/transfer/volume limit was exceeded; see the specific alias \
                 (hourly/daily transfer, hourly/daily volume, cooldown, penalty) for the cause"
            }
            AutoTradeError::PairsStrategyNotFound => {
                "no pairs-trading strategy exists for the given id"
            }
            AutoTradeError::PairsPositionError => {
                "pairs position operation failed; see PairsActivePositionExists/\
                 PairsNoActivePosition for the specific cause"
            }
            AutoTradeError::InsufficientCorrelation => {
                "correlation between the pair's assets is below the required minimum"
            }
            AutoTradeError::PairNotCointegrated => {
                "the pair fails the cointegration test required for pairs trading"
            }
            AutoTradeError::InvalidPairsConfig => "pairs-trading configuration is invalid",
            AutoTradeError::OracleUnavailable => {
                "no oracle price is currently available for this asset"
            }
            AutoTradeError::DcaError => {
                "DCA operation failed; see DcaStrategyNotFound/DcaStrategyInactive/\
                 DcaEndTimeReached for the specific cause"
            }
            AutoTradeError::MrStrategyError => {
                "mean-reversion strategy operation failed; see MrStrategyNotFound/\
                 MrInsufficientHistory/MrLowVolatility for the specific cause"
            }
            AutoTradeError::AdminTransferError => {
                "admin-transfer operation failed; see PendingAdminNotFound/\
                 PendingAdminExpired for the specific cause"
            }
            AutoTradeError::RoutingPlanNotFound => "no routing plan exists for the given id",
            AutoTradeError::ArbitrageError => {
                "arbitrage operation failed; see ArbitrageOpportunityExpired/\
                 ArbitrageUnprofitable/ArbTooLarge for the specific cause"
            }
            AutoTradeError::FrontRunningRisk => "trade blocked due to detected front-running risk",
            AutoTradeError::SystemError => {
                "system-level operation failed; see AtomicExecutionFailed/BridgePaused/\
                 RecoveryNotFound/RecoveryIncomplete/EscrowAlreadyClosed for the specific cause"
            }
            AutoTradeError::SlippageExceeded => {
                "executed price deviates from the reference price by more than allowed slippage"
            }
            AutoTradeError::LastOracleForPair => {
                "cannot remove the last remaining whitelisted oracle for a pair"
            }
            AutoTradeError::NotPaused => "action requires the contract to be paused, but it is not",
        }
    }
}

// ── Backward-compatible aliases ───────────────────────────────────────────────
// These keep all existing call-sites compiling without modification.
#[allow(non_upper_case_globals)]
impl AutoTradeError {
    pub const SelfReferral: AutoTradeError = AutoTradeError::ReferralError;
    pub const ReferralAlreadySet: AutoTradeError = AutoTradeError::ReferralError;
    pub const CircularReferral: AutoTradeError = AutoTradeError::ReferralError;
    pub const ReferralLimitExceeded: AutoTradeError = AutoTradeError::ReferralError;

    pub const InvalidTWAPDuration: AutoTradeError = AutoTradeError::TWAPError;
    pub const TWAPOrderNotFound: AutoTradeError = AutoTradeError::TWAPError;
    pub const NotTWAPOwner: AutoTradeError = AutoTradeError::TWAPError;
    pub const TWAPNotActive: AutoTradeError = AutoTradeError::TWAPError;

    pub const ConditionalOrderNotFound: AutoTradeError = AutoTradeError::ConditionalOrderError;
    pub const ConditionalOrderNotPending: AutoTradeError = AutoTradeError::ConditionalOrderError;
    pub const ConditionalOrderNotTriggered: AutoTradeError = AutoTradeError::ConditionalOrderError;

    pub const RateLimitPenalty: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const BelowMinTransfer: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const CooldownNotElapsed: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const HourlyTransferLimitExceeded: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const HourlyVolumeLimitExceeded: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const DailyTransferLimitExceeded: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const DailyVolumeLimitExceeded: AutoTradeError = AutoTradeError::RateLimitExceeded;
    pub const GlobalCapacityExceeded: AutoTradeError = AutoTradeError::RateLimitExceeded;

    pub const PairsActivePositionExists: AutoTradeError = AutoTradeError::PairsPositionError;
    pub const PairsNoActivePosition: AutoTradeError = AutoTradeError::PairsPositionError;

    pub const DcaStrategyNotFound: AutoTradeError = AutoTradeError::DcaError;
    pub const DcaStrategyInactive: AutoTradeError = AutoTradeError::DcaError;
    pub const DcaEndTimeReached: AutoTradeError = AutoTradeError::DcaError;

    pub const MrStrategyNotFound: AutoTradeError = AutoTradeError::MrStrategyError;
    pub const MrInsufficientHistory: AutoTradeError = AutoTradeError::MrStrategyError;
    pub const MrLowVolatility: AutoTradeError = AutoTradeError::MrStrategyError;

    pub const PendingAdminNotFound: AutoTradeError = AutoTradeError::AdminTransferError;
    pub const PendingAdminExpired: AutoTradeError = AutoTradeError::AdminTransferError;

    pub const ArbitrageOpportunityExpired: AutoTradeError = AutoTradeError::ArbitrageError;
    pub const ArbitrageUnprofitable: AutoTradeError = AutoTradeError::ArbitrageError;
    pub const ArbTooLarge: AutoTradeError = AutoTradeError::ArbitrageError;

    pub const AtomicExecutionFailed: AutoTradeError = AutoTradeError::SystemError;
    pub const BridgePaused: AutoTradeError = AutoTradeError::SystemError;
    pub const RecoveryNotFound: AutoTradeError = AutoTradeError::SystemError;
    pub const RecoveryIncomplete: AutoTradeError = AutoTradeError::SystemError;

    // ── Escrow (per-trade isolated custody) ──────────────────────────────────
    /// No escrow record exists for the given trade_id.
    pub const EscrowNotFound: AutoTradeError = AutoTradeError::StrategyNotFound;
    /// An active escrow already exists for this trade_id (double-initiation guard).
    pub const EscrowAlreadyActive: AutoTradeError = AutoTradeError::PositionAlreadyExists;
    /// Escrow is already Settled or Cancelled; double-release attempt rejected.
    pub const EscrowAlreadyClosed: AutoTradeError = AutoTradeError::SystemError;

    // ── Dead man's switch ─────────────────────────────────────────────────────
    /// Inactivity window has not elapsed yet; trigger is premature.
    pub const InactivityWindowNotElapsed: AutoTradeError = AutoTradeError::NotPaused;

    // ── Loss-streak pause (Issue #698) ───────────────────────────────────────
    /// Auto-trade paused due to consecutive losses.
    pub const LossStreakPaused: AutoTradeError = AutoTradeError::TradingPaused;
    // ── Daily execution cap ──────────────────────────────────────────────────
    /// Auto-trade blocked by per-user daily execution cap.
    pub const DailyExecutionCapExceeded: AutoTradeError = AutoTradeError::DailyTradeLimitExceeded;

    // ── Issue #811: upgrade-safe contract versioning ─────────────────────────
    // `AutoTradeError` is already at the 50-variant cap enforced by Soroban's
    // contract-spec XDR format (`ScSpecUdtErrorEnumV0.cases: VecM<_, 50>`), so
    // this reuses `SystemError` under a clearer name rather than adding a
    // 51st discriminant (which fails the `#[contracterror]` macro at
    // compile time with "LengthExceedsMax").
    /// `upgrade()` was called with a version that is not strictly greater
    /// than the currently stored contract version.
    pub const IncompatibleContractVersion: AutoTradeError = AutoTradeError::SystemError;
}
