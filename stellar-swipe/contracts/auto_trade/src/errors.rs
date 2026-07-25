use soroban_sdk::contracterror;

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
    StrategyNotFound = 10,
    PositionAlreadyExists = 11,
    InsufficientPriceHistory = 12,
    RankingDisabled = 13,
    RateLimited = 14,
    PrivacyModeEnabled = 15,
    TradingPaused = 16,
    // ── Portfolio / stat-arb ─────────────────────────────────────────────────
    InvalidBasketSize = 17,
    InvalidPriceData = 18,
    NonCointegratedBasket = 19,
    ActivePortfolioExists = 20,
    NoActivePortfolio = 21,
    NoTradeSignal = 22,
    InvalidStatArbConfig = 23,
    // ── Exit / insurance ─────────────────────────────────────────────────────
    ExitStrategyNotFound = 24,
    InvalidExitConfig = 25,
    InsuranceNotConfigured = 26,
    InvalidInsuranceConfig = 27,
    // ── Referral (SelfReferral / AlreadySet / Circular / LimitExceeded) ──────
    ReferralError = 28,
    // ── TWAP (InvalidDuration / NotFound / NotOwner / NotActive) ─────────────
    TWAPError = 29,
    // ── Correlation ──────────────────────────────────────────────────────────
    CorrelationLimitExceeded = 30,
    TooManyCorrelatedPositions = 31,
    // ── Conditional orders (NotFound / NotPending / NotTriggered / Config) ───
    ConditionalOrderError = 32,
    InvalidConditionalConfig = 33,
    // ── Rate limits (all sub-types collapsed) ────────────────────────────────
    RateLimitExceeded = 34,
    // ── Pairs trading ────────────────────────────────────────────────────────
    PairsStrategyNotFound = 35,
    PairsPositionError = 36,
    InsufficientCorrelation = 37,
    PairNotCointegrated = 38,
    InvalidPairsConfig = 39,
    // ── Oracle ───────────────────────────────────────────────────────────────
    OracleUnavailable = 40,
    // ── DCA (NotFound / Inactive / EndTimeReached) ────────────────────────────
    DcaError = 41,
    // ── Mean-reversion (NotFound / InsufficientHistory / LowVolatility) ──────
    MrStrategyError = 42,
    // ── Admin transfer ───────────────────────────────────────────────────────
    AdminTransferError = 43,
    // ── Routing ──────────────────────────────────────────────────────────────
    RoutingPlanNotFound = 44,
    // ── Arbitrage ────────────────────────────────────────────────────────────
    ArbitrageError = 45,
    FrontRunningRisk = 46,
    // ── System / bridge / recovery ───────────────────────────────────────────
    SystemError = 47,
    SlippageExceeded = 48,
    // ── Misc ─────────────────────────────────────────────────────────────────
    LastOracleForPair = 49,
    NotPaused = 50,
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
