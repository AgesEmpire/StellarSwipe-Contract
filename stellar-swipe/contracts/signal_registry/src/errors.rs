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
    InvalidTimestamp = 16,
    ScheduleTooFarFuture = 17,
    ScheduleLimitReached = 18,
    ScheduleNotFound = 19,
    NotScheduleOwner = 20,
    CircuitBreakerTriggered = 21,
    PendingAdminNotFound = 22,
    PendingAdminExpired = 23,
    ReentrancyDetected = 24,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AiScoreError {
    Unauthorized = 600,
    OracleNotConfigured = 601,
    InvalidScore = 602,
    SignalNotFound = 603,
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
