use soroban_sdk::contracterror;

/// Governance contract errors (≤ 50 variants — Soroban XDR limit).
///
/// Variants added after `ConvictionPoolNotFound` are exposed as associated
/// constants below so existing call sites keep working without exceeding the
/// XDR variant cap.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GovernanceError {
    /// `initialize()` was called on a contract that already has an admin set.
    AlreadyInitialized = 1,
    /// Contract function was called before `initialize()` set up the admin/config.
    NotInitialized = 2,
    /// Caller is not authorized to perform this action.
    Unauthorized = 3,
    /// Token supply parameter is zero or otherwise invalid.
    InvalidSupply = 4,
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 5,
    /// Duration parameter is zero or otherwise outside allowed bounds.
    InvalidDuration = 6,
    /// A vesting schedule already exists for this recipient.
    DuplicateSchedule = 7,
    /// No vesting schedule exists for the given recipient.
    VestingScheduleNotFound = 8,
    /// Vesting cliff period has not yet been reached.
    CliffNotReached = 9,
    /// No vested tokens are currently available to release.
    NothingToRelease = 10,
    /// Caller's balance is lower than the amount required for this action.
    InsufficientBalance = 11,
    /// Caller's staked balance is lower than the amount required for this action.
    InsufficientStakedBalance = 12,
    /// Tokens are locked by an active vote and cannot be transferred/unstaked yet.
    ActiveVoteLock = 13,
    /// Claimable amount is below the configured minimum claim threshold.
    BelowMinimumClaim = 14,
    /// Liquidity pool reserved for this operation has been exhausted.
    LiquidityPoolExhausted = 15,
    /// This recipient already appears in the batch/list; duplicates are not allowed.
    DuplicateRecipient = 16,
    /// Arithmetic operation overflowed.
    ArithmeticOverflow = 17,
    /// Reward configuration is invalid (e.g. rates out of range).
    InvalidRewardConfig = 18,
    /// Metadata payload is malformed or fails validation.
    InvalidMetadata = 19,
    /// No budget record exists for the given id.
    BudgetNotFound = 20,
    /// Requested spend would exceed the remaining budget.
    BudgetExceeded = 21,
    /// This budget's period has already ended; no further spend is allowed.
    BudgetPeriodEnded = 22,
    /// Required asset price is missing (oracle has no quote for this asset).
    MissingAssetPrice = 23,
    /// Treasury configuration is invalid.
    InvalidTreasuryConfig = 24,
    /// Committee configuration is invalid.
    InvalidCommitteeConfig = 25,
    /// No committee exists for the given id.
    CommitteeNotFound = 26,
    /// This committee's term has already ended.
    CommitteeTermEnded = 27,
    /// No committee decision exists for the given id.
    CommitteeDecisionNotFound = 28,
    /// This committee decision is not currently open for voting.
    CommitteeDecisionNotOpen = 29,
    /// Caller has already voted on this proposal/decision.
    AlreadyVoted = 30,
    /// Caller does not hold authority within the relevant committee.
    NoCommitteeAuthority = 31,
    /// No committee election exists for the given id.
    CommitteeElectionNotFound = 32,
    /// This committee election is not currently active.
    CommitteeElectionNotActive = 33,
    /// Caller is not a registered candidate in this committee election.
    NotCommitteeCandidate = 34,
    /// No cross-committee request exists for the given id.
    CrossCommitteeRequestNotFound = 35,
    /// This committee is inactive and cannot process the requested action.
    CommitteeInactive = 36,
    /// Requested committee action is invalid for the current state.
    InvalidCommitteeAction = 37,
    /// Approval rating value is outside the allowed range.
    InvalidApprovalRating = 38,
    /// Governance configuration parameter is invalid.
    InvalidGovernanceConfig = 39,
    /// Proposal payload is malformed or fails validation.
    InvalidProposal = 40,
    /// No proposal exists for the given id.
    ProposalNotFound = 41,
    /// Proposal is not in the active state required for this action.
    ProposalNotActive = 42,
    /// Proposal has not been approved and cannot be executed.
    ProposalNotApproved = 43,
    /// Voting period for this proposal has not started yet.
    VotingNotStarted = 44,
    /// Voting period for this proposal has already ended.
    VotingEnded = 45,
    /// Caller has zero voting power and cannot cast a vote.
    NoVotingPower = 46,
    /// Timelock has not been initialized for this contract.
    TimelockNotInitialized = 47,
    /// No queued timelock action exists for the given id.
    ActionNotFound = 48,
    /// Timelock configuration is invalid.
    InvalidTimelockConfig = 49,
    /// No conviction-voting pool exists for the given id.
    ConvictionPoolNotFound = 50,
}

impl GovernanceError {
    /// Short, human-readable description of when this error is returned.
    ///
    /// `GovernanceError` is at the 50-variant XDR cap (see the alias-const
    /// block below), so several logical error names are `const` aliases for
    /// a shared underlying variant (the closest semantic match) rather than
    /// distinct discriminants. An alias's runtime value is *identical* to
    /// its target variant's, so `message()` returns the same string for
    /// both. For the alias-specific meaning, see the `///` doc comment on
    /// the individual alias const instead.
    pub fn message(&self) -> &'static str {
        match self {
            GovernanceError::AlreadyInitialized => {
                "contract has already been initialized with an admin"
            }
            GovernanceError::NotInitialized => {
                "contract has not been initialized yet; call initialize() first"
            }
            GovernanceError::Unauthorized => "caller is not authorized to perform this action",
            GovernanceError::InvalidSupply => "token supply parameter is zero or invalid",
            GovernanceError::InvalidAmount => "amount is zero, negative, or outside allowed bounds",
            GovernanceError::InvalidDuration => "duration is zero or outside allowed bounds",
            GovernanceError::DuplicateSchedule => {
                "a vesting schedule already exists for this recipient"
            }
            GovernanceError::VestingScheduleNotFound => {
                "no vesting schedule exists for the given recipient"
            }
            GovernanceError::CliffNotReached => "vesting cliff period has not yet been reached",
            GovernanceError::NothingToRelease => {
                "no vested tokens are currently available to release"
            }
            GovernanceError::InsufficientBalance => {
                "caller's balance is lower than the amount required for this action"
            }
            GovernanceError::InsufficientStakedBalance => {
                "caller's staked balance is lower than the amount required for this action"
            }
            GovernanceError::ActiveVoteLock => {
                "tokens are locked by an active vote and cannot be moved yet"
            }
            GovernanceError::BelowMinimumClaim => {
                "claimable amount is below the configured minimum claim threshold"
            }
            GovernanceError::LiquidityPoolExhausted => {
                "liquidity pool reserved for this operation has been exhausted"
            }
            GovernanceError::DuplicateRecipient => {
                "this recipient already appears in the batch/list"
            }
            GovernanceError::ArithmeticOverflow => "arithmetic operation overflowed",
            GovernanceError::InvalidRewardConfig => "reward configuration is invalid",
            GovernanceError::InvalidMetadata => "metadata payload is malformed or fails validation",
            GovernanceError::BudgetNotFound => "no budget record exists for the given id",
            GovernanceError::BudgetExceeded => "requested spend would exceed the remaining budget",
            GovernanceError::BudgetPeriodEnded => {
                "this budget's period has already ended; no further spend is allowed"
            }
            GovernanceError::MissingAssetPrice => {
                "required asset price is missing (no oracle quote available)"
            }
            GovernanceError::InvalidTreasuryConfig => "treasury configuration is invalid",
            GovernanceError::InvalidCommitteeConfig => "committee configuration is invalid",
            GovernanceError::CommitteeNotFound => "no committee exists for the given id",
            GovernanceError::CommitteeTermEnded => "this committee's term has already ended",
            GovernanceError::CommitteeDecisionNotFound => {
                "no committee decision exists for the given id"
            }
            GovernanceError::CommitteeDecisionNotOpen => {
                "this committee decision is not currently open for voting"
            }
            GovernanceError::AlreadyVoted => "caller has already voted on this proposal/decision",
            GovernanceError::NoCommitteeAuthority => {
                "caller does not hold authority within the relevant committee"
            }
            GovernanceError::CommitteeElectionNotFound => {
                "no committee election exists for the given id"
            }
            GovernanceError::CommitteeElectionNotActive => {
                "this committee election is not currently active"
            }
            GovernanceError::NotCommitteeCandidate => {
                "caller is not a registered candidate in this committee election"
            }
            GovernanceError::CrossCommitteeRequestNotFound => {
                "no cross-committee request exists for the given id"
            }
            GovernanceError::CommitteeInactive => {
                "this committee is inactive and cannot process the requested action"
            }
            GovernanceError::InvalidCommitteeAction => {
                "requested committee action is invalid for the current state"
            }
            GovernanceError::InvalidApprovalRating => {
                "approval rating value is outside the allowed range"
            }
            GovernanceError::InvalidGovernanceConfig => "governance configuration is invalid",
            GovernanceError::InvalidProposal => "proposal payload is malformed or fails validation",
            GovernanceError::ProposalNotFound => "no proposal exists for the given id",
            GovernanceError::ProposalNotActive => {
                "proposal is not in the active state required for this action"
            }
            GovernanceError::ProposalNotApproved => {
                "proposal has not been approved and cannot be executed"
            }
            GovernanceError::VotingNotStarted => {
                "voting period for this proposal has not started yet"
            }
            GovernanceError::VotingEnded => "voting period for this proposal has already ended",
            GovernanceError::NoVotingPower => "caller has zero voting power and cannot cast a vote",
            GovernanceError::TimelockNotInitialized => {
                "timelock has not been initialized for this contract"
            }
            GovernanceError::ActionNotFound => "no queued timelock action exists for the given id",
            GovernanceError::InvalidTimelockConfig => "timelock configuration is invalid",
            GovernanceError::ConvictionPoolNotFound => {
                "no conviction-voting pool exists for the given id"
            }
        }
    }
}

#[allow(non_upper_case_globals)]
impl GovernanceError {
    pub const ElectionQuorumNotMet: GovernanceError = GovernanceError::InvalidCommitteeAction;
    pub const InvalidElectionVote: GovernanceError = GovernanceError::InvalidCommitteeAction;
    pub const BudgetApprovalRequired: GovernanceError = GovernanceError::BudgetNotFound;
    pub const ApprovedCapExceeded: GovernanceError = GovernanceError::BudgetExceeded;
    pub const ContractPaused: GovernanceError = GovernanceError::Unauthorized;
    pub const InvalidCalibrationConfig: GovernanceError = GovernanceError::InvalidGovernanceConfig;
    pub const IterationLimitExceeded: GovernanceError = GovernanceError::InvalidCommitteeAction;
    /// Vote rejected because the mandatory discussion window has not yet elapsed (Issue #667).
    pub const DiscussionPeriodActive: GovernanceError = GovernanceError::VotingNotStarted;
    /// Proposal has already been withdrawn — cannot be withdrawn again.
    pub const ProposalAlreadyWithdrawn: GovernanceError = GovernanceError::ProposalNotActive;
    /// Decay rate is outside the valid range (MIN_DECAY_RATE..=MAX_DECAY_RATE).
    pub const InvalidDecayRate: GovernanceError = GovernanceError::InvalidGovernanceConfig;
    /// A succeeded proposal's execution window (`execution_deadline`) has
    /// passed — it can no longer be executed and must instead be reclaimed
    /// via `reclaim_expired_proposal` (Issue #796).
    pub const ProposalExpired: GovernanceError = GovernanceError::BudgetPeriodEnded;
    /// No pending admin rotation has been proposed.
    pub const PendingAdminNotFound: GovernanceError = GovernanceError::Unauthorized;
    /// Guardian address has not been configured.
    pub const GuardianNotSet: GovernanceError = GovernanceError::Unauthorized;
    /// A proposal's embedded parameter value (`current` or `proposed` in a
    /// `ParameterChange`) is outside the allowlisted `±MAX_PARAMETER_VALUE`
    /// range enforced by `validate_proposal`.
    pub const ProposalParameterOutOfRange: GovernanceError = GovernanceError::InvalidAmount;
    /// A direct admin entry point was called while admin-timelock enforcement
    /// is active (Issue #942). The action must instead be routed through its
    /// `queue_*` + `*_timelocked` pair so the mandatory delay applies.
    pub const TimelockBypassBlocked: GovernanceError = GovernanceError::Unauthorized;
}
