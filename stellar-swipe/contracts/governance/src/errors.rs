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
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidSupply = 4,
    InvalidAmount = 5,
    InvalidDuration = 6,
    DuplicateSchedule = 7,
    VestingScheduleNotFound = 8,
    CliffNotReached = 9,
    NothingToRelease = 10,
    InsufficientBalance = 11,
    InsufficientStakedBalance = 12,
    ActiveVoteLock = 13,
    BelowMinimumClaim = 14,
    LiquidityPoolExhausted = 15,
    DuplicateRecipient = 16,
    ArithmeticOverflow = 17,
    InvalidRewardConfig = 18,
    InvalidMetadata = 19,
    BudgetNotFound = 20,
    BudgetExceeded = 21,
    BudgetPeriodEnded = 22,
    MissingAssetPrice = 23,
    InvalidTreasuryConfig = 24,
    InvalidCommitteeConfig = 25,
    CommitteeNotFound = 26,
    CommitteeTermEnded = 27,
    CommitteeDecisionNotFound = 28,
    CommitteeDecisionNotOpen = 29,
    AlreadyVoted = 30,
    NoCommitteeAuthority = 31,
    CommitteeElectionNotFound = 32,
    CommitteeElectionNotActive = 33,
    NotCommitteeCandidate = 34,
    CrossCommitteeRequestNotFound = 35,
    CommitteeInactive = 36,
    InvalidCommitteeAction = 37,
    InvalidApprovalRating = 38,
    InvalidGovernanceConfig = 39,
    InvalidProposal = 40,
    ProposalNotFound = 41,
    ProposalNotActive = 42,
    ProposalNotApproved = 43,
    VotingNotStarted = 44,
    VotingEnded = 45,
    NoVotingPower = 46,
    TimelockNotInitialized = 47,
    ActionNotFound = 48,
    InvalidTimelockConfig = 49,
    ConvictionPoolNotFound = 50,
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
    /// A proposal action's numeric parameter (e.g. `ParameterChange`'s
    /// `current`/`proposed`) falls outside `proposals::MAX_PARAMETER_VALUE`
    /// (Issue #997).
    pub const ProposalParameterOutOfRange: GovernanceError = GovernanceError::InvalidProposal;
    /// The proposal's action type is not part of the governance action
    /// allowlist (Issue #997, see `proposals::PROPOSAL_ACTION_SCHEMA_VERSION`).
    /// In practice this can only be surfaced by off-chain tooling:
    /// `ProposalType` is a closed Rust enum, so the Soroban host rejects any
    /// XDR payload that doesn't decode into a known variant before contract
    /// code — and this error — ever runs.
    pub const UnknownProposalAction: GovernanceError = GovernanceError::InvalidProposal;
}
