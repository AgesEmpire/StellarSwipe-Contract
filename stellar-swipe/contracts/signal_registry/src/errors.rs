use crate::community_voting::DisputeError;
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AdminError {
    /// Caller is not authorized to perform this action.
    Unauthorized = 1,
    /// `initialize()` was called on a contract that already has an admin set.
    AlreadyInitialized = 2,
    /// Contract function was called before `initialize()` set up the admin/config.
    NotInitialized = 3,
    /// A configuration parameter is missing or outside its allowed range.
    InvalidParameter = 4,
    /// Trading is currently paused; this action cannot proceed.
    TradingPaused = 5,
    /// The pause window has expired and needs to be renewed or lifted.
    PauseExpired = 6,
    /// Fee rate is zero, negative, or exceeds the configured maximum.
    InvalidFeeRate = 7,
    /// Risk parameter is outside the allowed range.
    InvalidRiskParameter = 8,
    /// Fewer multisig signatures were provided than the required threshold.
    InsufficientSignatures = 9,
    /// This signer has already signed; duplicate signatures are not allowed.
    DuplicateSigner = 10,
    /// Asset pair is empty, malformed, or not recognized.
    InvalidAssetPair = 11,
    /// A user cannot follow themselves.
    CannotFollowSelf = 12,
    /// Caller has exceeded the allowed rate of operations.
    RateLimitExceeded = 13,
    /// Caller has exceeded the maximum number of active signals allowed.
    SignalLimitExceeded = 14,
    /// Timestamp is zero, in the past, or otherwise invalid.
    InvalidTimestamp = 16,
    /// Requested schedule time is further in the future than allowed.
    ScheduleTooFarFuture = 17,
    /// Caller has reached the maximum number of scheduled signals allowed.
    ScheduleLimitReached = 18,
    /// No scheduled signal exists for the given id.
    ScheduleNotFound = 19,
    /// Caller is not the owner of this scheduled signal.
    NotScheduleOwner = 20,
    /// Circuit breaker has tripped and is blocking this operation.
    CircuitBreakerTriggered = 21,
    /// Provider's stake is below the required minimum for this action.
    StakeBelowMinimum = 22,
    /// No pending admin-transfer request exists to accept.
    PendingAdminNotFound = 23,
    /// Pending admin-transfer request has expired and must be re-initiated.
    PendingAdminExpired = 24,
    /// Re-entrant call into a state-mutating function was detected and rejected.
    ReentrancyDetected = 25,
    /// This privileged action requires multisig approval; use propose/approve/execute.
    RequiresMultisigApproval = 26,
    /// No multisig proposal exists for the given id.
    ProposalNotFound = 27,
    /// This signer has already approved the proposal.
    AlreadyApproved = 28,
    /// Proposal has not collected enough approvals to execute yet.
    ProposalNotApproved = 29,
    /// Timelock period for this proposal has not yet elapsed.
    TimelockNotElapsed = 30,
    /// Proposal has already been executed and cannot run again.
    ProposalAlreadyExecuted = 31,
    /// Proposal has been cancelled and can no longer be executed.
    ProposalCancelled = 32,
    /// Caller has too many pending proposals open at once.
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

impl AdminError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            AdminError::Unauthorized => "caller is not authorized to perform this action",
            AdminError::AlreadyInitialized => "contract has already been initialized",
            AdminError::NotInitialized => "contract has not been initialized yet",
            AdminError::InvalidParameter => "configuration parameter is missing or out of range",
            AdminError::TradingPaused => "trading is currently paused",
            AdminError::PauseExpired => "pause window has expired and needs renewal",
            AdminError::InvalidFeeRate => "fee rate is out of the configured allowed range",
            AdminError::InvalidRiskParameter => "risk parameter is outside the allowed range",
            AdminError::InsufficientSignatures => {
                "fewer multisig signatures provided than the required threshold"
            }
            AdminError::DuplicateSigner => "this signer has already signed",
            AdminError::InvalidAssetPair => "asset pair is empty, malformed, or not recognized",
            AdminError::CannotFollowSelf => "a user cannot follow themselves",
            AdminError::RateLimitExceeded => "caller has exceeded the allowed rate of operations",
            AdminError::SignalLimitExceeded => {
                "caller has exceeded the maximum number of active signals allowed"
            }
            AdminError::InvalidTimestamp => "timestamp is zero, in the past, or otherwise invalid",
            AdminError::ScheduleTooFarFuture => {
                "requested schedule time is further in the future than allowed"
            }
            AdminError::ScheduleLimitReached => {
                "caller has reached the maximum number of scheduled signals allowed"
            }
            AdminError::ScheduleNotFound => "no scheduled signal exists for the given id",
            AdminError::NotScheduleOwner => "caller is not the owner of this scheduled signal",
            AdminError::CircuitBreakerTriggered => {
                "circuit breaker has tripped and is blocking this operation"
            }
            AdminError::StakeBelowMinimum => {
                "provider's stake is below the required minimum for this action"
            }
            AdminError::PendingAdminNotFound => "no pending admin-transfer request exists",
            AdminError::PendingAdminExpired => "pending admin-transfer request has expired",
            AdminError::ReentrancyDetected => "re-entrant call was detected and rejected",
            AdminError::RequiresMultisigApproval => {
                "this action requires multisig approval (propose/approve/execute)"
            }
            AdminError::ProposalNotFound => "no multisig proposal exists for the given id",
            AdminError::AlreadyApproved => "this signer has already approved the proposal",
            AdminError::ProposalNotApproved => {
                "proposal has not collected enough approvals to execute yet"
            }
            AdminError::TimelockNotElapsed => "timelock period for this proposal has not elapsed",
            AdminError::ProposalAlreadyExecuted => "proposal has already been executed",
            AdminError::ProposalCancelled => "proposal has been cancelled",
            AdminError::TooManyProposals => "caller has too many pending proposals open at once",
            AdminError::CooldownNotElapsed => {
                "provider submitted too recently; must wait for the cooldown period"
            }
            AdminError::IncompatibleContractVersion => {
                "upgrade() version is not strictly greater than the currently stored version"
            }
            AdminError::IncompatibleStorageLayout => {
                "on-chain storage schema does not match the migration's expected precondition"
            }
        }
    }
}

/// Issue #782: renumbered from 600-603, which collided with `ComboError`
/// (600-613) — two different failure conditions decoded to the same
/// contract-wide `u32` error code.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AiScoreError {
    /// Caller is not authorized to perform this action.
    Unauthorized = 1500,
    /// No AI-scoring oracle has been configured.
    OracleNotConfigured = 1501,
    /// Submitted score is outside the valid range.
    InvalidScore = 1502,
    /// No signal exists for the given id.
    SignalNotFound = 1503,
}

impl AiScoreError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            AiScoreError::Unauthorized => "caller is not authorized to perform this action",
            AiScoreError::OracleNotConfigured => "no AI-scoring oracle has been configured",
            AiScoreError::InvalidScore => "submitted score is outside the valid range",
            AiScoreError::SignalNotFound => "no signal exists for the given id",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FeeError {
    /// Trade amount is too small for a meaningful fee to be charged.
    TradeTooSmall = 100,
    /// Computed fee rounded down to zero and was skipped.
    FeeRoundedToZero = 101,
    /// Arithmetic operation overflowed.
    ArithmeticOverflow = 102,
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 103,
    /// Provider address is missing or invalid for fee payout.
    InvalidProviderAddress = 104,
}

impl FeeError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            FeeError::TradeTooSmall => "trade amount is too small for a meaningful fee",
            FeeError::FeeRoundedToZero => "computed fee rounded down to zero and was skipped",
            FeeError::ArithmeticOverflow => "arithmetic operation overflowed",
            FeeError::InvalidAmount => "amount is zero, negative, or outside allowed bounds",
            FeeError::InvalidProviderAddress => {
                "provider address is missing or invalid for fee payout"
            }
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SocialError {
    /// A user cannot follow themselves.
    CannotFollowSelf = 50,
}

impl SocialError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            SocialError::CannotFollowSelf => "a user cannot follow themselves",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PerformanceError {
    /// No signal exists for the given id.
    SignalNotFound = 200,
    /// Price must be greater than zero.
    InvalidPrice = 201,
    /// Attempted division by zero in a performance calculation.
    DivisionByZero = 202,
    /// Volume must be greater than zero.
    InvalidVolume = 203,
    /// The signal has expired and can no longer be scored/executed against.
    SignalExpired = 204,
    /// No executions have been recorded for this signal yet.
    NoExecutions = 205,
    /// Trading is currently paused; this action cannot proceed.
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

impl PerformanceError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            PerformanceError::SignalNotFound => "no signal exists for the given id",
            PerformanceError::InvalidPrice => "price must be greater than zero",
            PerformanceError::DivisionByZero => {
                "attempted division by zero in a performance calculation"
            }
            PerformanceError::InvalidVolume => "volume must be greater than zero",
            PerformanceError::SignalExpired => {
                "signal has expired and can no longer be scored/executed against"
            }
            PerformanceError::NoExecutions => "no executions have been recorded for this signal",
            PerformanceError::TradingPaused => "trading is currently paused",
            PerformanceError::OraclePriceStale => {
                "entry or exit price is older than the acceptable staleness window"
            }
            PerformanceError::OraclePriceMissing => {
                "no oracle price has been published for this asset pair"
            }
            PerformanceError::OraclePriceOutOfBounds => {
                "price is outside the allowed [MIN_ORACLE_PRICE, MAX_ORACLE_PRICE] range"
            }
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TemplateError {
    /// No signal template exists for the given id.
    TemplateNotFound = 300,
    /// Caller is not authorized to perform this action.
    Unauthorized = 301,
    /// Template is private and not accessible to this caller.
    PrivateTemplate = 302,
    /// A required template variable was not supplied.
    MissingVariable = 303,
    /// Template payload is malformed or fails validation.
    InvalidTemplate = 304,
    /// Requested action is not valid for a template.
    InvalidAction = 305,
    /// Expiry is in the past or otherwise invalid.
    InvalidExpiry = 306,
}

impl TemplateError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            TemplateError::TemplateNotFound => "no signal template exists for the given id",
            TemplateError::Unauthorized => "caller is not authorized to perform this action",
            TemplateError::PrivateTemplate => {
                "template is private and not accessible to this caller"
            }
            TemplateError::MissingVariable => "a required template variable was not supplied",
            TemplateError::InvalidTemplate => "template payload is malformed or fails validation",
            TemplateError::InvalidAction => "requested action is not valid for a template",
            TemplateError::InvalidExpiry => "expiry is in the past or otherwise invalid",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ImportError {
    /// Import payload does not match the expected format.
    InvalidFormat = 400,
    /// Asset pair is empty, malformed, or not recognized.
    InvalidAssetPair = 401,
    /// Price must be greater than zero.
    InvalidPrice = 402,
    /// Requested action is not valid for an imported signal.
    InvalidAction = 403,
    /// Rationale text is empty or exceeds the allowed length.
    InvalidRationale = 404,
    /// Expiry is in the past or otherwise invalid.
    InvalidExpiry = 405,
    /// Batch contains more entries than the allowed maximum.
    BatchSizeExceeded = 406,
    /// Import payload contains no data.
    EmptyData = 407,
    /// Import payload could not be parsed.
    ParseError = 408,
}

impl ImportError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ImportError::InvalidFormat => "import payload does not match the expected format",
            ImportError::InvalidAssetPair => "asset pair is empty, malformed, or not recognized",
            ImportError::InvalidPrice => "price must be greater than zero",
            ImportError::InvalidAction => "requested action is not valid for an imported signal",
            ImportError::InvalidRationale => "rationale is empty or exceeds the allowed length",
            ImportError::InvalidExpiry => "expiry is in the past or otherwise invalid",
            ImportError::BatchSizeExceeded => {
                "batch contains more entries than the allowed maximum"
            }
            ImportError::EmptyData => "import payload contains no data",
            ImportError::ParseError => "import payload could not be parsed",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollaborationError {
    /// Caller is not a registered co-author of this signal.
    NotCoAuthor = 500,
    /// This co-author has already approved the signal.
    AlreadyApproved = 501,
    /// Contribution shares/weights supplied are invalid.
    InvalidContributions = 502,
    /// This signal is not configured for collaborative editing.
    NotCollaborative = 503,
    /// Signal is awaiting approval from other co-authors before it can proceed.
    PendingApproval = 504,
}

impl CollaborationError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            CollaborationError::NotCoAuthor => {
                "caller is not a registered co-author of this signal"
            }
            CollaborationError::AlreadyApproved => "this co-author has already approved the signal",
            CollaborationError::InvalidContributions => {
                "contribution shares/weights supplied are invalid"
            }
            CollaborationError::NotCollaborative => {
                "this signal is not configured for collaborative editing"
            }
            CollaborationError::PendingApproval => {
                "signal is awaiting approval from other co-authors"
            }
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ExportError {
    /// Requested export format is not supported.
    UnsupportedFormat = 700,
    /// No data exists in the requested time range.
    NoDataInRange = 701,
    /// Requested export would exceed the maximum allowed size.
    ExportTooLarge = 702,
}

impl ExportError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ExportError::UnsupportedFormat => "requested export format is not supported",
            ExportError::NoDataInRange => "no data exists in the requested time range",
            ExportError::ExportTooLarge => "requested export would exceed the maximum allowed size",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ComboError {
    /// No combo signal exists for the given id.
    ComboNotFound = 600,
    /// No signal exists for the given id.
    SignalNotFound = 601,
    /// Caller is not the owner of this signal.
    NotSignalOwner = 602,
    /// Component weights are invalid (e.g. don't sum to the expected total).
    InvalidWeights = 603,
    /// Combining component weights overflowed.
    WeightOverflow = 604,
    /// Combo has no component signals attached.
    NoComponents = 605,
    /// Combo has more component signals than the allowed maximum.
    TooManyComponents = 606,
    /// Component signal is not in the Active state.
    SignalNotActive = 607,
    /// A component signal has expired.
    ComponentSignalExpired = 608,
    /// Referenced condition does not exist on this combo.
    InvalidConditionReference = 609,
    /// Combo is not in the Active state required for this action.
    ComboNotActive = 610,
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 611,
    /// Trading is currently paused; this action cannot proceed.
    TradingPaused = 612,
    /// Circuit breaker has tripped and is blocking this operation.
    CircuitBreakerTriggered = 613,
}

impl ComboError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ComboError::ComboNotFound => "no combo signal exists for the given id",
            ComboError::SignalNotFound => "no signal exists for the given id",
            ComboError::NotSignalOwner => "caller is not the owner of this signal",
            ComboError::InvalidWeights => {
                "component weights are invalid (don't sum to the expected total)"
            }
            ComboError::WeightOverflow => "combining component weights overflowed",
            ComboError::NoComponents => "combo has no component signals attached",
            ComboError::TooManyComponents => {
                "combo has more component signals than the allowed maximum"
            }
            ComboError::SignalNotActive => "component signal is not in the Active state",
            ComboError::ComponentSignalExpired => "a component signal has expired",
            ComboError::InvalidConditionReference => {
                "referenced condition does not exist on this combo"
            }
            ComboError::ComboNotActive => {
                "combo is not in the Active state required for this action"
            }
            ComboError::InvalidAmount => "amount is zero, negative, or outside allowed bounds",
            ComboError::TradingPaused => "trading is currently paused",
            ComboError::CircuitBreakerTriggered => {
                "circuit breaker has tripped and is blocking this operation"
            }
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContestError {
    /// No contest exists for the given id.
    ContestNotFound = 800,
    /// Contest start/end time range is invalid.
    InvalidTimeRange = 801,
    /// Prize pool amount is zero, negative, or otherwise invalid.
    InvalidPrizePool = 802,
    /// Contest has not yet reached its end time.
    ContestNotEnded = 803,
    /// Contest has already been finalized.
    AlreadyFinalized = 804,
    /// Caller does not meet the qualification criteria for this contest.
    NotQualified = 805,
    /// Trading is currently paused; this action cannot proceed.
    TradingPaused = 806,
    /// Circuit breaker has tripped and is blocking this operation.
    CircuitBreakerTriggered = 807,
    /// Finalization was attempted before the committed randomness ledger is reached.
    RandomnessNotAvailable = 808,
}

impl ContestError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ContestError::ContestNotFound => "no contest exists for the given id",
            ContestError::InvalidTimeRange => "contest start/end time range is invalid",
            ContestError::InvalidPrizePool => "prize pool amount is zero, negative, or invalid",
            ContestError::ContestNotEnded => "contest has not yet reached its end time",
            ContestError::AlreadyFinalized => "contest has already been finalized",
            ContestError::NotQualified => {
                "caller does not meet the qualification criteria for this contest"
            }
            ContestError::TradingPaused => "trading is currently paused",
            ContestError::CircuitBreakerTriggered => {
                "circuit breaker has tripped and is blocking this operation"
            }
            ContestError::RandomnessNotAvailable => {
                "finalization attempted before the committed randomness ledger is reached"
            }
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VersioningError {
    /// Caller is not the owner of this signal.
    NotSignalOwner = 900,
    /// Only Active signals can be updated; this signal is inactive.
    CannotUpdateInactive = 901,
    /// Signal has reached the maximum number of allowed version updates.
    MaxUpdatesReached = 902,
    /// Signal was updated too recently; must wait for the cooldown period to elapse.
    UpdateCooldown = 903,
    /// The signal has expired.
    SignalExpired = 904,
    /// Price must be greater than zero.
    InvalidPrice = 905,
    /// Expiry is in the past or otherwise invalid.
    InvalidExpiry = 906,
    /// No version record exists for the given id.
    VersionNotFound = 907,
}

impl VersioningError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            VersioningError::NotSignalOwner => "caller is not the owner of this signal",
            VersioningError::CannotUpdateInactive => {
                "only Active signals can be updated; this signal is inactive"
            }
            VersioningError::MaxUpdatesReached => {
                "signal has reached the maximum number of allowed version updates"
            }
            VersioningError::UpdateCooldown => {
                "signal was updated too recently; must wait for the cooldown period"
            }
            VersioningError::SignalExpired => "the signal has expired",
            VersioningError::InvalidPrice => "price must be greater than zero",
            VersioningError::InvalidExpiry => "expiry is in the past or otherwise invalid",
            VersioningError::VersionNotFound => "no version record exists for the given id",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CrossChainError {
    /// A signal with this cross-chain id has already been synced.
    SignalAlreadyExists = 1000,
    /// No signal exists for the given id.
    SignalNotFound = 1001,
    /// Cross-chain proof verification failed.
    VerificationFailed = 1002,
    /// Supplied proof is malformed or does not match the expected format.
    InvalidProof = 1003,
    /// Source-chain address is not registered for cross-chain sync.
    AddressNotRegistered = 1004,
    /// Sync status transition requested is not valid from the current state.
    InvalidSyncStatus = 1005,
    /// Caller is not the owner of this signal.
    NotSignalOwner = 1006,
}

impl CrossChainError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            CrossChainError::SignalAlreadyExists => {
                "a signal with this cross-chain id has already been synced"
            }
            CrossChainError::SignalNotFound => "no signal exists for the given id",
            CrossChainError::VerificationFailed => "cross-chain proof verification failed",
            CrossChainError::InvalidProof => {
                "supplied proof is malformed or does not match the expected format"
            }
            CrossChainError::AddressNotRegistered => {
                "source-chain address is not registered for cross-chain sync"
            }
            CrossChainError::InvalidSyncStatus => {
                "sync status transition requested is not valid from the current state"
            }
            CrossChainError::NotSignalOwner => "caller is not the owner of this signal",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignalEditError {
    /// The window during which this signal may be edited has closed.
    EditWindowClosed = 1100,
    /// This field cannot be edited after signal creation.
    FieldNotEditable = 1101,
    /// Signal has already been copied by followers; edits are locked.
    SignalAlreadyCopied = 1102,
    /// No signal exists for the given id.
    SignalNotFound = 1103,
    /// Caller is not the owner of this signal.
    NotSignalOwner = 1104,
    /// Confidence value is outside the allowed range.
    InvalidConfidence = 1105,
    /// Trading is currently paused; this action cannot proceed.
    TradingPaused = 1106,
}

impl SignalEditError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            SignalEditError::EditWindowClosed => {
                "the window during which this signal may be edited has closed"
            }
            SignalEditError::FieldNotEditable => {
                "this field cannot be edited after signal creation"
            }
            SignalEditError::SignalAlreadyCopied => {
                "signal has already been copied by followers; edits are locked"
            }
            SignalEditError::SignalNotFound => "no signal exists for the given id",
            SignalEditError::NotSignalOwner => "caller is not the owner of this signal",
            SignalEditError::InvalidConfidence => "confidence value is outside the allowed range",
            SignalEditError::TradingPaused => "trading is currently paused",
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignalOutcomeError {
    /// Caller is not authorized to perform this action.
    Unauthorized = 1150,
    /// No signal exists for the given id.
    SignalNotFound = 1151,
    /// Signal must be closed before its outcome can be recorded.
    SignalNotClosed = 1152,
    /// An outcome has already been recorded for this signal.
    OutcomeAlreadyRecorded = 1153,
}

impl SignalOutcomeError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            SignalOutcomeError::Unauthorized => "caller is not authorized to perform this action",
            SignalOutcomeError::SignalNotFound => "no signal exists for the given id",
            SignalOutcomeError::SignalNotClosed => {
                "signal must be closed before its outcome can be recorded"
            }
            SignalOutcomeError::OutcomeAlreadyRecorded => {
                "an outcome has already been recorded for this signal"
            }
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SubmissionError {
    /// Caller has no stake and cannot submit signals.
    NoStake = 1200,
    /// Caller's stake is below the minimum required to submit signals.
    BelowMinimumStake = 1201,
    /// Asset pair is empty, malformed, or not recognized.
    InvalidAssetPair = 1202,
    /// Price must be greater than zero.
    InvalidPrice = 1203,
    /// Rationale text is empty.
    EmptyRationale = 1204,
    /// An identical signal has already been submitted.
    DuplicateSignal = 1205,
    /// Rationale field is required but was not supplied.
    MissingRationale = 1206,
    /// Submitted price deviates unreasonably from the reference/oracle price.
    PriceUnreasonable = 1207,
}

impl SubmissionError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            SubmissionError::NoStake => "caller has no stake and cannot submit signals",
            SubmissionError::BelowMinimumStake => {
                "caller's stake is below the minimum required to submit signals"
            }
            SubmissionError::InvalidAssetPair => {
                "asset pair is empty, malformed, or not recognized"
            }
            SubmissionError::InvalidPrice => "price must be greater than zero",
            SubmissionError::EmptyRationale => "rationale text is empty",
            SubmissionError::DuplicateSignal => "an identical signal has already been submitted",
            SubmissionError::MissingRationale => "rationale field is required but was not supplied",
            SubmissionError::PriceUnreasonable => {
                "submitted price deviates unreasonably from the reference/oracle price"
            }
        }
    }
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

impl SignalValidationError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            SignalValidationError::InvalidAssetPair => "asset pair is empty or malformed",
            SignalValidationError::InvalidPrice => "price must be greater than zero",
            SignalValidationError::EmptyRationale => "rationale is empty",
            SignalValidationError::RationaleTooLong => {
                "rationale exceeds the maximum allowed length"
            }
            SignalValidationError::InvalidExpiry => "expiry is in the past",
            SignalValidationError::TooManyTags => "too many tags supplied (max 10)",
            SignalValidationError::DailyLimitExceeded => {
                "provider has exceeded the daily signal creation limit"
            }
        }
    }
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

impl SignalCancelError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            SignalCancelError::NotFound => "signal does not exist",
            SignalCancelError::NotOwner => "caller is not the signal provider",
            SignalCancelError::NotActive => "signal is not in Active state and cannot be cancelled",
            SignalCancelError::LifetimeNotElapsed => {
                "the configured minimum signal lifetime has not yet elapsed"
            }
        }
    }
}

/// Errors returned by the provider onboarding pipeline (issues #1017, #1043).
///
/// Codes occupy the `1700` block, disjoint from every other `#[contracterror]`
/// enum in this crate (see the discriminant-uniqueness assertion below).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProviderOnboardingError {
    /// The onboarding pipeline has not been configured with parameters yet.
    NotConfigured = 1700,
    /// Provider identifier string is empty.
    EmptyProviderId = 1701,
    /// Provider identifier exceeds the configured maximum length.
    ProviderIdTooLong = 1702,
    /// Metadata URI exceeds the configured maximum length.
    MetadataUriTooLong = 1703,
    /// Posted collateral is below the configured minimum threshold.
    CollateralBelowMinimum = 1704,
    /// Posted onboarding fee is below the configured minimum.
    FeeBelowMinimum = 1705,
    /// Collateral or fee amount is zero or negative.
    InvalidAmount = 1706,
    /// An application for this provider already exists in a non-terminal state.
    DuplicateApplication = 1707,
    /// No application record exists for the given provider.
    ApplicationNotFound = 1708,
    /// The application is not in a state that allows this transition.
    InvalidStateTransition = 1709,
    /// The application has already reached a terminal state (Active / Failed).
    AlreadyFinalized = 1710,
    /// There are no reserved funds to refund for this application.
    NothingToRefund = 1711,
    /// An internal accounting operation overflowed.
    ArithmeticError = 1712,
    /// Caller is not authorised to perform this onboarding action.
    Unauthorized = 1713,
    /// Configuration parameter is zero or otherwise outside allowed bounds.
    InvalidConfig = 1714,
}

impl ProviderOnboardingError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            ProviderOnboardingError::NotConfigured => {
                "onboarding pipeline has not been configured with parameters"
            }
            ProviderOnboardingError::EmptyProviderId => "provider identifier is empty",
            ProviderOnboardingError::ProviderIdTooLong => {
                "provider identifier exceeds the configured maximum length"
            }
            ProviderOnboardingError::MetadataUriTooLong => {
                "metadata URI exceeds the configured maximum length"
            }
            ProviderOnboardingError::CollateralBelowMinimum => {
                "posted collateral is below the configured minimum threshold"
            }
            ProviderOnboardingError::FeeBelowMinimum => {
                "posted onboarding fee is below the configured minimum"
            }
            ProviderOnboardingError::InvalidAmount => {
                "collateral or fee amount is zero or negative"
            }
            ProviderOnboardingError::DuplicateApplication => {
                "an application for this provider already exists in a non-terminal state"
            }
            ProviderOnboardingError::ApplicationNotFound => {
                "no application record exists for the given provider"
            }
            ProviderOnboardingError::InvalidStateTransition => {
                "the application is not in a state that allows this transition"
            }
            ProviderOnboardingError::AlreadyFinalized => {
                "the application has already reached a terminal state"
            }
            ProviderOnboardingError::NothingToRefund => {
                "there are no reserved funds to refund for this application"
            }
            ProviderOnboardingError::ArithmeticError => {
                "an internal accounting operation overflowed"
            }
            ProviderOnboardingError::Unauthorized => {
                "caller is not authorised to perform this onboarding action"
            }
            ProviderOnboardingError::InvalidConfig => {
                "configuration parameter is zero or outside allowed bounds"
            }
        }
    }
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
    // ProviderOnboardingError (issues #1017, #1043)
    ProviderOnboardingError::NotConfigured as u32,
    ProviderOnboardingError::EmptyProviderId as u32,
    ProviderOnboardingError::ProviderIdTooLong as u32,
    ProviderOnboardingError::MetadataUriTooLong as u32,
    ProviderOnboardingError::CollateralBelowMinimum as u32,
    ProviderOnboardingError::FeeBelowMinimum as u32,
    ProviderOnboardingError::InvalidAmount as u32,
    ProviderOnboardingError::DuplicateApplication as u32,
    ProviderOnboardingError::ApplicationNotFound as u32,
    ProviderOnboardingError::InvalidStateTransition as u32,
    ProviderOnboardingError::AlreadyFinalized as u32,
    ProviderOnboardingError::NothingToRefund as u32,
    ProviderOnboardingError::ArithmeticError as u32,
    ProviderOnboardingError::Unauthorized as u32,
    ProviderOnboardingError::InvalidConfig as u32,
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
