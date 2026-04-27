use soroban_sdk::contracterror;

/// AutoTrade contract errors (≤ 50 variants — Soroban XDR limit).
///
/// Related sub-errors are collapsed into a single variant; the emitted event
/// carries the fine-grained reason.  Aliases in the `impl` block keep all
/// existing call-sites compiling without changes.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AutoTradeError {
    // ── Core trade errors ────────────────────────────────────────────────────
    InvalidAmount = 1,
    Unauthorized = 2,
    SignalNotFound = 3,
    SignalExpired = 4,
    InsufficientBalance = 5,
    InsufficientLiquidity = 6,
    DailyTradeLimitExceeded = 7,
    PositionLimitExceeded = 8,
    StopLossTriggered = 9,
    TradingPaused = 10,
    StrategyNotFound = 11,
    PositionAlreadyExists = 12,
    // ── Portfolio / stat-arb ─────────────────────────────────────────────────
    InvalidBasketSize = 13,
    InsufficientPriceHistory = 14,
    InvalidPriceData = 15,
    NonCointegratedBasket = 16,
    ActivePortfolioExists = 17,
    NoActivePortfolio = 18,
    NoTradeSignal = 19,
    InvalidStatArbConfig = 20,
    // ── Exit / insurance ─────────────────────────────────────────────────────
    ExitStrategyNotFound = 21,
    InvalidExitConfig = 22,
    InsuranceNotConfigured = 23,
    InvalidInsuranceConfig = 24,
    // ── Referral (SelfReferral / AlreadySet / Circular / LimitExceeded) ──────
    ReferralError = 25,
    // ── TWAP (InvalidDuration / NotFound / NotOwner / NotActive) ─────────────
    TWAPError = 26,
    // ── Correlation ──────────────────────────────────────────────────────────
    CorrelationLimitExceeded = 27,
    TooManyCorrelatedPositions = 28,
    // ── Conditional orders (NotFound / NotPending / NotTriggered / Config) ───
    ConditionalOrderError = 29,
    InvalidConditionalConfig = 30,
    // ── Rate limits (all sub-types collapsed) ────────────────────────────────
    RateLimitExceeded = 31,
    // ── Pairs trading ────────────────────────────────────────────────────────
    PairsStrategyNotFound = 32,
    PairsPositionError = 33,
    InsufficientCorrelation = 34,
    PairNotCointegrated = 35,
    InvalidPairsConfig = 36,
    // ── Oracle ───────────────────────────────────────────────────────────────
    OracleUnavailable = 37,
    // ── DCA (NotFound / Inactive / EndTimeReached) ────────────────────────────
    DcaError = 38,
    // ── Mean-reversion (NotFound / InsufficientHistory / LowVolatility) ──────
    MrStrategyError = 39,
    // ── Admin transfer ───────────────────────────────────────────────────────
    AdminTransferError = 40,
    // ── Routing ──────────────────────────────────────────────────────────────
    RoutingPlanNotFound = 41,
    // ── Arbitrage ────────────────────────────────────────────────────────────
    ArbitrageError = 42,
    FrontRunningRisk = 43,
    // ── System / bridge / recovery ───────────────────────────────────────────
    SystemError = 44,
    SlippageExceeded = 45,
    // ── Misc ─────────────────────────────────────────────────────────────────
    RankingDisabled = 46,
    LastOracleForPair = 47,
    NotPaused = 48,
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
}
