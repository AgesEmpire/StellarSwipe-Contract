use crate::community_voting::DisputeError;
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AdminError {
    Unauthorized = 1,
    AlreadyInitialized = 2,
    NotInitialized = 3,
    InvalidParameter = 4,
    TradingPaused = 5,
    PauseExpired = 6,
    InvalidFeeRate = 7,
    InvalidRiskParameter = 8,
    InsufficientSignatures = 9,
    DuplicateSigner = 10,
    InvalidAssetPair = 11,
    CannotFollowSelf = 12,
    RateLimitExceeded = 13,
    SignalLimitExceeded = 14,
    InvalidTimestamp = 16,
    ScheduleTooFarFuture = 17,
    ScheduleLimitReached = 18,
    ScheduleNotFound = 19,
    NotScheduleOwner = 20,
    CircuitBreakerTriggered = 21,
    StakeBelowMinimum = 22,
    PendingAdminNotFound = 23,
    PendingAdminExpired = 24,
    ReentrancyDetected = 25,
    RequiresMultisigApproval = 26,
    ProposalNotFound = 27,
    AlreadyApproved = 28,
    ProposalNotApproved = 29,
    TimelockNotElapsed = 30,
    ProposalAlreadyExecuted = 31,
    ProposalCancelled = 32,
    TooManyProposals = 33,
    /// Provider has submitted too recently; must wait for the cooldown period to elapse.
    CooldownNotElapsed = 34,
    /// Issue #811: `upgrade()` was called with a version that is not
    /// strictly greater than the currently stored contract version.
    IncompatibleContractVersion = 35,
    /// Issue #812: the on-chain storage schema version does not match what
    /// `migrate_signals_v1_to_v2` expects as a precondition. Migration logic
    /// does not run when this is returned — no state is touched.
    IncompatibleStorageLayout = 36,
}

/// Issue #782: renumbered from 600-603, which collided with `ComboError`
/// (600-613) — two different failure conditions decoded to the same
/// contract-wide `u32` error code.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AiScoreError {
    Unauthorized = 1500,
    OracleNotConfigured = 1501,
    InvalidScore = 1502,
    SignalNotFound = 1503,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FeeError {
    TradeTooSmall = 100,
    FeeRoundedToZero = 101,
    ArithmeticOverflow = 102,
    InvalidAmount = 103,
    InvalidProviderAddress = 104,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SocialError {
    CannotFollowSelf = 50,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PerformanceError {
    SignalNotFound = 200,
    InvalidPrice = 201,
    DivisionByZero = 202,
    InvalidVolume = 203,
    SignalExpired = 204,
    NoExecutions = 205,
    TradingPaused = 206,
    /// Entry or exit price is older than the acceptable staleness window.
    /// Matches `OracleError::PriceStale` from the oracle contract.
    OraclePriceStale = 207,
    /// No oracle price has been published for this asset pair
    /// (timestamp == 0). Matches `OracleError::PriceNotFound`.
    OraclePriceMissing = 208,
    /// Price is outside `[MIN_ORACLE_PRICE, MAX_ORACLE_PRICE]`.
    /// A zero/negative price signals a corrupt feed; an excessively large
    /// price would overflow basis-point ROI calculations.
    OraclePriceOutOfBounds = 209,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TemplateError {
    TemplateNotFound = 300,
    Unauthorized = 301,
    PrivateTemplate = 302,
    MissingVariable = 303,
    InvalidTemplate = 304,
    InvalidAction = 305,
    InvalidExpiry = 306,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ImportError {
    InvalidFormat = 400,
    InvalidAssetPair = 401,
    InvalidPrice = 402,
    InvalidAction = 403,
    InvalidRationale = 404,
    InvalidExpiry = 405,
    BatchSizeExceeded = 406,
    EmptyData = 407,
    ParseError = 408,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollaborationError {
    NotCoAuthor = 500,
    AlreadyApproved = 501,
    InvalidContributions = 502,
    NotCollaborative = 503,
    PendingApproval = 504,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ExportError {
    UnsupportedFormat = 700,
    NoDataInRange = 701,
    ExportTooLarge = 702,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ComboError {
    ComboNotFound = 600,
    SignalNotFound = 601,
    NotSignalOwner = 602,
    InvalidWeights = 603,
    WeightOverflow = 604,
    NoComponents = 605,
    TooManyComponents = 606,
    SignalNotActive = 607,
    ComponentSignalExpired = 608,
    InvalidConditionReference = 609,
    ComboNotActive = 610,
    InvalidAmount = 611,
    TradingPaused = 612,
    CircuitBreakerTriggered = 613,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContestError {
    ContestNotFound = 800,
    InvalidTimeRange = 801,
    InvalidPrizePool = 802,
    ContestNotEnded = 803,
    AlreadyFinalized = 804,
    NotQualified = 805,
    TradingPaused = 806,
    CircuitBreakerTriggered = 807,
    /// Finalization was attempted before the committed randomness ledger is reached.
    RandomnessNotAvailable = 808,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VersioningError {
    NotSignalOwner = 900,
    CannotUpdateInactive = 901,
    MaxUpdatesReached = 902,
    UpdateCooldown = 903,
    SignalExpired = 904,
    InvalidPrice = 905,
    InvalidExpiry = 906,
    VersionNotFound = 907,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CrossChainError {
    SignalAlreadyExists = 1000,
    SignalNotFound = 1001,
    VerificationFailed = 1002,
    InvalidProof = 1003,
    AddressNotRegistered = 1004,
    InvalidSyncStatus = 1005,
    NotSignalOwner = 1006,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignalEditError {
    EditWindowClosed = 1100,
    FieldNotEditable = 1101,
    SignalAlreadyCopied = 1102,
    SignalNotFound = 1103,
    NotSignalOwner = 1104,
    InvalidConfidence = 1105,
    TradingPaused = 1106,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignalOutcomeError {
    Unauthorized = 1150,
    SignalNotFound = 1151,
    SignalNotClosed = 1152,
    OutcomeAlreadyRecorded = 1153,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SubmissionError {
    NoStake = 1200,
    BelowMinimumStake = 1201,
    InvalidAssetPair = 1202,
    InvalidPrice = 1203,
    EmptyRationale = 1204,
    DuplicateSignal = 1205,
    MissingRationale = 1206,
    PriceUnreasonable = 1207,
}

/// Errors returned by signal input validation (issue #634).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignalValidationError {
    /// Asset pair is empty or malformed.
    InvalidAssetPair = 1400,
    /// Price must be greater than zero.
    InvalidPrice = 1401,
    /// Rationale is empty.
    EmptyRationale = 1402,
    /// Rationale exceeds the maximum allowed length.
    RationaleTooLong = 1403,
    /// Expiry is in the past.
    InvalidExpiry = 1404,
    /// Too many tags supplied (max 10).
    TooManyTags = 1405,
    /// Provider has exceeded the daily signal creation limit (issue #778).
    DailyLimitExceeded = 1406,
}

/// Errors returned by `cancel_signal` (issue #687).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignalCancelError {
    /// Signal does not exist.
    NotFound = 1300,
    /// Caller is not the signal provider.
    NotOwner = 1301,
    /// Signal is not in Active state and cannot be cancelled.
    NotActive = 1302,
    /// The configured minimum signal lifetime has not yet elapsed; cancellation rejected.
    LifetimeNotElapsed = 1303,
}

// ── Issue #782: compile-time discriminant uniqueness ───────────────────────────
//
// Soroban encodes every `#[contracterror]` value as a single contract-wide
// `u32` on-chain — there is no enum-level tag recoverable from a raw error
// code by an off-chain client. Two variants sharing a numeric value (even
// across different enums) are therefore indistinguishable once decoded.
// This asserts every discriminant across every `#[contracterror]` enum in
// the crate — including `DisputeError`, defined in `community_voting.rs` —
// is unique, at compile time.
const ALL_ERROR_CODES: &[u32] = &[
    // AdminError
    AdminError::Unauthorized as u32,
    AdminError::AlreadyInitialized as u32,
    AdminError::NotInitialized as u32,
    AdminError::InvalidParameter as u32,
    AdminError::TradingPaused as u32,
    AdminError::PauseExpired as u32,
    AdminError::InvalidFeeRate as u32,
    AdminError::InvalidRiskParameter as u32,
    AdminError::InsufficientSignatures as u32,
    AdminError::DuplicateSigner as u32,
    AdminError::InvalidAssetPair as u32,
    AdminError::CannotFollowSelf as u32,
    AdminError::RateLimitExceeded as u32,
    AdminError::SignalLimitExceeded as u32,
    AdminError::InvalidTimestamp as u32,
    AdminError::ScheduleTooFarFuture as u32,
    AdminError::ScheduleLimitReached as u32,
    AdminError::ScheduleNotFound as u32,
    AdminError::NotScheduleOwner as u32,
    AdminError::CircuitBreakerTriggered as u32,
    AdminError::StakeBelowMinimum as u32,
    AdminError::PendingAdminNotFound as u32,
    AdminError::PendingAdminExpired as u32,
    AdminError::ReentrancyDetected as u32,
    AdminError::RequiresMultisigApproval as u32,
    AdminError::ProposalNotFound as u32,
    AdminError::AlreadyApproved as u32,
    AdminError::ProposalNotApproved as u32,
    AdminError::TimelockNotElapsed as u32,
    AdminError::ProposalAlreadyExecuted as u32,
    AdminError::ProposalCancelled as u32,
    AdminError::TooManyProposals as u32,
    AdminError::CooldownNotElapsed as u32,
    AdminError::IncompatibleContractVersion as u32,
    AdminError::IncompatibleStorageLayout as u32,
    // AiScoreError
    AiScoreError::Unauthorized as u32,
    AiScoreError::OracleNotConfigured as u32,
    AiScoreError::InvalidScore as u32,
    AiScoreError::SignalNotFound as u32,
    // FeeError
    FeeError::TradeTooSmall as u32,
    FeeError::FeeRoundedToZero as u32,
    FeeError::ArithmeticOverflow as u32,
    FeeError::InvalidAmount as u32,
    FeeError::InvalidProviderAddress as u32,
    // SocialError
    SocialError::CannotFollowSelf as u32,
    // PerformanceError
    PerformanceError::SignalNotFound as u32,
    PerformanceError::InvalidPrice as u32,
    PerformanceError::DivisionByZero as u32,
    PerformanceError::InvalidVolume as u32,
    PerformanceError::SignalExpired as u32,
    PerformanceError::NoExecutions as u32,
    PerformanceError::TradingPaused as u32,
    PerformanceError::OraclePriceStale as u32,
    PerformanceError::OraclePriceMissing as u32,
    PerformanceError::OraclePriceOutOfBounds as u32,
    // TemplateError
    TemplateError::TemplateNotFound as u32,
    TemplateError::Unauthorized as u32,
    TemplateError::PrivateTemplate as u32,
    TemplateError::MissingVariable as u32,
    TemplateError::InvalidTemplate as u32,
    TemplateError::InvalidAction as u32,
    TemplateError::InvalidExpiry as u32,
    // ImportError
    ImportError::InvalidFormat as u32,
    ImportError::InvalidAssetPair as u32,
    ImportError::InvalidPrice as u32,
    ImportError::InvalidAction as u32,
    ImportError::InvalidRationale as u32,
    ImportError::InvalidExpiry as u32,
    ImportError::BatchSizeExceeded as u32,
    ImportError::EmptyData as u32,
    ImportError::ParseError as u32,
    // CollaborationError
    CollaborationError::NotCoAuthor as u32,
    CollaborationError::AlreadyApproved as u32,
    CollaborationError::InvalidContributions as u32,
    CollaborationError::NotCollaborative as u32,
    CollaborationError::PendingApproval as u32,
    // ExportError
    ExportError::UnsupportedFormat as u32,
    ExportError::NoDataInRange as u32,
    ExportError::ExportTooLarge as u32,
    // ComboError
    ComboError::ComboNotFound as u32,
    ComboError::SignalNotFound as u32,
    ComboError::NotSignalOwner as u32,
    ComboError::InvalidWeights as u32,
    ComboError::WeightOverflow as u32,
    ComboError::NoComponents as u32,
    ComboError::TooManyComponents as u32,
    ComboError::SignalNotActive as u32,
    ComboError::ComponentSignalExpired as u32,
    ComboError::InvalidConditionReference as u32,
    ComboError::ComboNotActive as u32,
    ComboError::InvalidAmount as u32,
    ComboError::TradingPaused as u32,
    ComboError::CircuitBreakerTriggered as u32,
    // ContestError
    ContestError::ContestNotFound as u32,
    ContestError::InvalidTimeRange as u32,
    ContestError::InvalidPrizePool as u32,
    ContestError::ContestNotEnded as u32,
    ContestError::AlreadyFinalized as u32,
    ContestError::NotQualified as u32,
    ContestError::TradingPaused as u32,
    ContestError::CircuitBreakerTriggered as u32,
    ContestError::RandomnessNotAvailable as u32,
    // VersioningError
    VersioningError::NotSignalOwner as u32,
    VersioningError::CannotUpdateInactive as u32,
    VersioningError::MaxUpdatesReached as u32,
    VersioningError::UpdateCooldown as u32,
    VersioningError::SignalExpired as u32,
    VersioningError::InvalidPrice as u32,
    VersioningError::InvalidExpiry as u32,
    VersioningError::VersionNotFound as u32,
    // CrossChainError
    CrossChainError::SignalAlreadyExists as u32,
    CrossChainError::SignalNotFound as u32,
    CrossChainError::VerificationFailed as u32,
    CrossChainError::InvalidProof as u32,
    CrossChainError::AddressNotRegistered as u32,
    CrossChainError::InvalidSyncStatus as u32,
    CrossChainError::NotSignalOwner as u32,
    // SignalEditError
    SignalEditError::EditWindowClosed as u32,
    SignalEditError::FieldNotEditable as u32,
    SignalEditError::SignalAlreadyCopied as u32,
    SignalEditError::SignalNotFound as u32,
    SignalEditError::NotSignalOwner as u32,
    SignalEditError::InvalidConfidence as u32,
    SignalEditError::TradingPaused as u32,
    // SignalOutcomeError
    SignalOutcomeError::Unauthorized as u32,
    SignalOutcomeError::SignalNotFound as u32,
    SignalOutcomeError::SignalNotClosed as u32,
    SignalOutcomeError::OutcomeAlreadyRecorded as u32,
    // SubmissionError
    SubmissionError::NoStake as u32,
    SubmissionError::BelowMinimumStake as u32,
    SubmissionError::InvalidAssetPair as u32,
    SubmissionError::InvalidPrice as u32,
    SubmissionError::EmptyRationale as u32,
    SubmissionError::DuplicateSignal as u32,
    SubmissionError::MissingRationale as u32,
    SubmissionError::PriceUnreasonable as u32,
    // SignalValidationError
    SignalValidationError::InvalidAssetPair as u32,
    SignalValidationError::InvalidPrice as u32,
    SignalValidationError::EmptyRationale as u32,
    SignalValidationError::RationaleTooLong as u32,
    SignalValidationError::InvalidExpiry as u32,
    SignalValidationError::TooManyTags as u32,
    SignalValidationError::DailyLimitExceeded as u32,
    // SignalCancelError
    SignalCancelError::NotFound as u32,
    SignalCancelError::NotOwner as u32,
    SignalCancelError::NotActive as u32,
    SignalCancelError::LifetimeNotElapsed as u32,
    // DisputeError (defined in community_voting.rs)
    DisputeError::Unauthorized as u32,
    DisputeError::DisputeNotFound as u32,
    DisputeError::DisputeNotOpen as u32,
    DisputeError::AppealAlreadySubmitted as u32,
    DisputeError::AppealNotPending as u32,
    DisputeError::AppealWindowNotElapsed as u32,
];

const fn has_duplicate(codes: &[u32]) -> bool {
    let mut i = 0;
    while i < codes.len() {
        let mut j = i + 1;
        while j < codes.len() {
            if codes[i] == codes[j] {
                return true;
            }
            j += 1;
        }
        i += 1;
    }
    false
}

const _: () = assert!(
    !has_duplicate(ALL_ERROR_CODES),
    "duplicate #[contracterror] discriminant detected across signal_registry error enums"
);
