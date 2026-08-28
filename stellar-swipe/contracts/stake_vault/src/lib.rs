#![no_std]

pub mod emergency_unstake;
pub mod events;
pub mod migration;

use emergency_unstake::{EmergencyMultiSigConfig, EmergencyRequest};
use migration::{MigrationKey, StakeInfoV2};
use shared::{initializable, multisig, pausable, reentrancy};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Bytes, BytesN, Env,
    IntoVal, String, Symbol, Val, Vec,
};

// ── Slash severity tiers ──────────────────────────────────────────────────────

/// Severity tier for a slashing event.
/// Controls what fraction of stake is burned.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SlashSeverity {
    Minor = 0,
    Major = 1,
    Critical = 2,
}

/// On-chain configuration for how much stake each tier slashes.
/// Values are in basis points (10_000 = 100 %).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashTierConfig {
    /// Basis points slashed for a Minor violation.
    pub minor_bps: u32,
    /// Basis points slashed for a Major violation.
    pub major_bps: u32,
    /// Basis points slashed for a Critical violation (typically 10_000 = full stake).
    pub critical_bps: u32,
}

impl SlashTierConfig {
    pub const fn default_config() -> Self {
        Self {
            minor_bps: 500,       // 5 %
            major_bps: 3_000,     // 30 %
            critical_bps: 10_000, // 100 %
        }
    }
}

const BPS_DENOMINATOR: i128 = 10_000;

// ── Issue #787: stake duration-weighted voting power ──────────────────────────

/// Seconds in a week, used to convert a stake's remaining lock time into the
/// whole-week buckets that `LockMultiplierTier` thresholds are expressed in.
const SECS_PER_WEEK: u64 = 604_800;

/// Upper bound on an admin-configured lock-multiplier tier (100x), well above
/// the default max tier (2x) while still keeping `balance * bps` comfortably
/// clear of `i128` overflow for realistic stake sizes.
const MAX_LOCK_MULTIPLIER_BPS: u32 = 1_000_000;

/// Admin-configurable voting-power multiplier tier (issue #787).
///
/// While a stake's remaining lock time (`locked_until - now`) is at least
/// `weeks`, its voting power is multiplied by `bps` basis points
/// (`10_000` = 1x). The highest-`bps` tier whose `weeks` threshold is met by
/// the remaining lock time applies.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockMultiplierTier {
    pub weeks: u32,
    pub bps: u32,
}

/// Default lock-multiplier tiers, used until an admin configures custom ones
/// via `set_lock_multiplier_tier`.
fn default_lock_multiplier_tiers(env: &Env) -> Vec<LockMultiplierTier> {
    let mut tiers = Vec::new(env);
    tiers.push_back(LockMultiplierTier {
        weeks: 1,
        bps: 10_000, // 1x
    });
    tiers.push_back(LockMultiplierTier {
        weeks: 4,
        bps: 12_500, // 1.25x
    });
    tiers.push_back(LockMultiplierTier {
        weeks: 12,
        bps: 20_000, // 2x
    });
    tiers
}

/// Returns the multiplier (basis points) for a stake with `remaining_secs`
/// left on its lock. `0` means the remaining duration does not meet even the
/// smallest configured tier (equivalent to the pre-#787 "still within the
/// eligibility lock" zero-voting-power behaviour).
fn lock_multiplier_bps_for_remaining(tiers: &Vec<LockMultiplierTier>, remaining_secs: u64) -> u32 {
    let remaining_weeks = (remaining_secs / SECS_PER_WEEK) as u32;
    let mut applicable_bps: u32 = 0;
    for tier in tiers.iter() {
        if remaining_weeks >= tier.weeks && tier.bps > applicable_bps {
            applicable_bps = tier.bps;
        }
    }
    applicable_bps
}

/// Applies a basis-point multiplier to `balance`, saturating instead of
/// overflowing for pathologically large inputs.
fn apply_multiplier_bps(balance: i128, bps: u32) -> i128 {
    balance
        .checked_mul(bps as i128)
        .and_then(|v| v.checked_div(BPS_DENOMINATOR))
        .unwrap_or(i128::MAX)
}

/// Temporary-storage key for the reentrancy lock on `withdraw_stake`.
const EXECUTION_LOCK: &str = "WithdrawLock";

/// 24 hours in seconds — grace period for providers to top up stake.
const GRACE_PERIOD_SECS: u64 = 86_400;

/// Default time-lock for large withdrawals (flash loan prevention), used until
/// an admin configures a custom value via `set_withdrawal_cooldown` (issue #816).
const LARGE_WITHDRAWAL_TIMELOCK_SECS: u64 = 3_600;

/// Upper bound on the configurable withdrawal cooldown (issue #816): 30 days.
const MAX_WITHDRAWAL_COOLDOWN_SECS: u64 = 2_592_000;

/// Threshold above which a withdrawal is considered "large" and requires time-lock.
/// Set to SILVER tier (500M = 5 * 10^8 stroops).
const LARGE_WITHDRAWAL_THRESHOLD: i128 = 500_000_000;

/// Default cap on the number of pending entries in the unstake queue (issue #786).
/// Bounds instance storage growth and per-call compute cost of `queue_unstake`.
const DEFAULT_MAX_UNSTAKE_QUEUE_SIZE: u32 = 200;

/// Maximum number of items permitted in a batch operation (e.g. batch_slash_stake, batch_resolve_appeal).
const MAX_BATCH_SIZE: u32 = 100;

/// Default cooldown window in ledgers between slash events for the same
/// provider.  Approx. 8 minutes on Stellar mainnet (~5 s per ledger).
const DEFAULT_SLASH_COOLDOWN_LEDGERS: u32 = 100;

pub const GOLD_TIER_STAKE: i128 = 1_000_000_000;
pub const SILVER_TIER_STAKE: i128 = GOLD_TIER_STAKE / 2;
pub const BRONZE_TIER_STAKE: i128 = GOLD_TIER_STAKE / 10;

fn stake_tier_for_amount(amount: i128) -> u32 {
    if amount >= GOLD_TIER_STAKE {
        3
    } else if amount >= SILVER_TIER_STAKE {
        2
    } else if amount >= BRONZE_TIER_STAKE {
        1
    } else {
        0
    }
}

/// Validates slash tier basis points (issue #816): each value must be within
/// `[0, 10_000]` and severity must be non-decreasing (minor <= major <= critical)
/// so a "worse" violation can never slash a smaller fraction than a lesser one.
fn validate_slash_tiers(
    minor_bps: u32,
    major_bps: u32,
    critical_bps: u32,
) -> Result<(), StakeVaultError> {
    if minor_bps > 10_000 || major_bps > 10_000 || critical_bps > 10_000 {
        return Err(StakeVaultError::InvalidSlashTier);
    }
    if minor_bps > major_bps || major_bps > critical_bps {
        return Err(StakeVaultError::InvalidSlashTierOrder);
    }
    Ok(())
}

/// Validates a configurable withdrawal cooldown (issue #816): bounded to
/// `[0, MAX_WITHDRAWAL_COOLDOWN_SECS]`. Zero disables the time-lock delay
/// entirely (a large withdrawal request becomes immediately actionable).
fn validate_cooldown(cooldown_secs: u64) -> Result<(), StakeVaultError> {
    if cooldown_secs > MAX_WITHDRAWAL_COOLDOWN_SECS {
        return Err(StakeVaultError::InvalidCooldown);
    }
    Ok(())
}

fn get_withdrawal_cooldown_secs(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&StorageKey::WithdrawalCooldownSecs)
        .unwrap_or(LARGE_WITHDRAWAL_TIMELOCK_SECS)
}

fn emit_provider_tier_change(
    env: &Env,
    provider: &Address,
    old_tier: u32,
    new_tier: u32,
    stake_balance: i128,
) {
    if old_tier == new_tier {
        return;
    }
    events::emit_provider_tier_changed(env, provider.clone(), old_tier, new_tier, stake_balance);
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    StakeToken,
    SignalRegistry,
    /// Minimum stake required for a provider to submit signals.
    MinimumStake,
    /// Timestamp when a provider's stake first dropped below minimum.
    /// `None` means stake is currently at or above minimum.
    StakeBelowMinSince(Address),
    /// Timestamp when a large-withdrawal request was initiated (per staker).
    LargeWithdrawalRequestedAt(Address),
    /// Ledger sequence at which a stake was last deposited (per staker).
    /// Used to detect same-ledger stake+unstake flash loan patterns.
    LastStakeLedger(Address),
    /// Admin-configurable slashing tier percentages.
    SlashTierConfig,
    /// Minimum duration (seconds) that newly staked funds must remain locked
    /// before they count toward voting power. Configurable by admin.
    MinimumStakeDuration,
    /// Tracks the deposit timestamp for each staker's most recent deposit,
    /// used to enforce the minimum stake duration lock on voting power.
    /// Only the newly deposited portion is subject to the fresh lock.
    StakeDepositTimestamps(Address),
    /// Per-provider map of delegator → delegated amount (issue #688).
    ProviderDelegatedStakes(Address),
    /// Cached total amount delegated to a provider across all delegators (issue #688).
    ProviderTotalDelegated(Address),
    // ── Issue #663: Unstake request queue ──────────────────────────────────────
    /// Next ticket number to assign (tail of queue).
    UnstakeQueueTail,
    /// Next ticket number to process (head of queue).
    UnstakeQueueHead,
    /// Per-ticket unstake request entry.
    UnstakeQueueEntry(u64),
    /// Ticket number assigned to a queued user (for position lookup).
    UserUnstakeTicket(Address),
    /// Admin-configurable cap on unstake queue length (issue #786, default 200).
    MaxUnstakeQueueSize,
    // ── Issue #754: Multi-sig emergency early-unstake ──────────────────────────
    /// N-of-M multi-sig configuration for emergency early unstakes.
    EmergencyMultiSigConfig,
    /// Per-staker pending emergency unstake request (approvals accumulate here).
    EmergencyRequest(Address),
    // ── Issue #689: Slash appeal ─────────────────────────────────────────────────
    SlashCounter,
    SlashRecord(u64),
    SlashAppeal(u64),
    SlashedFundsHeld(u64),
    AppealWindowSecs,
    // ── Slash cooldown (issue #816) ──────────────────────────────────────────
    /// Ledger sequence of the most recent global slash event.
    LastSlashLedger,
    /// Admin-configurable cooldown (in ledgers) between slash events.
    SlashCooldownLedgers,
    // ── Issue #816: configurable withdrawal cooldown ───────────────────────────
    /// Admin-configurable large-withdrawal time-lock duration (seconds).
    /// Falls back to `LARGE_WITHDRAWAL_TIMELOCK_SECS` when unset.
    WithdrawalCooldownSecs,
    // ── Issue #787: stake duration-weighted voting power ───────────────────────
    /// Admin-configurable table mapping a remaining-lock-duration threshold
    /// (in whole weeks) to a voting-power multiplier (basis points).
    /// Falls back to the built-in default tiers when unset.
    LockMultiplierTiers,
    /// Issue #865: central governance contract address authorized to call
    /// `apply_governance_pause`.
    GovernanceAddress,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StakeVaultError {
    /// Contract function was called before `initialize()` set up the admin/config.
    NotInitialized = 1,
    /// Caller is not authorized to perform this action (not the admin/owner/staker).
    Unauthorized = 2,
    /// Caller has no active stake recorded in the vault.
    NoStake = 3,
    /// Stake is still within its lock period and cannot be unstaked yet.
    StakeLocked = 4,
    /// Re-entrant call into a state-mutating function was detected and rejected.
    ReentrancyDetected = 5,
    /// Provider stake is below minimum and grace period has expired.
    StakeBelowMinimum = 6,
    /// Contract is paused due to suspicious activity or admin action.
    ContractPaused = 7,
    /// Large withdrawal requires a pending time-lock request first.
    TimelockRequired = 8,
    /// Time-lock period has not yet elapsed.
    TimelockNotElapsed = 9,
    /// Stake and unstake in the same ledger — flash loan pattern detected.
    FlashLoanDetected = 10,
    /// Slash tier percentage would exceed 100% of stake.
    InvalidSlashTier = 11,
    /// Voting power is locked because the minimum stake duration has not elapsed.
    StakeDurationNotElapsed = 12,
    /// No delegated stake found for the given delegator/provider pair.
    NoDelegatedStake = 13,
    /// User already has a pending unstake request in the queue.
    UnstakeAlreadyQueued = 14,
    /// No unstake request found for this user.
    NoUnstakeQueued = 15,
    /// Unstake queue is empty.
    QueueEmpty = 16,
    // ── Issue #754 ─────────────────────────────────────────────────────────────
    /// Emergency unstake is not configured (no multi-sig config set).
    EmergencyNotConfigured = 17,
    /// A pending emergency request already exists for this staker.
    EmergencyRequestAlreadyExists = 18,
    /// No emergency request found for this staker.
    EmergencyRequestNotFound = 19,
    /// The emergency request has expired without sufficient approvals.
    EmergencyRequestExpired = 20,
    /// The request has not yet expired and cannot be discarded.
    EmergencyRequestNotExpired = 21,
    /// The signer has already approved this request.
    AlreadyApproved = 22,
    /// Penalty basis points exceed 100%.
    InvalidEmergencyPenalty = 23,
    /// Multi-sig required count is 0 or exceeds the number of admins.
    InvalidMultiSigConfig = 24,
    /// The privileged action requires multi-sig approval; use propose/approve/execute.
    RequiresMultisig = 25,
    // ── Issue #689: Slash appeal ─────────────────────────────────────────────────
    /// Appeal window duration configured is zero or otherwise invalid.
    InvalidAppealWindow = 26,
    /// Appeal window for this slash has already elapsed; appeal can no longer be filed.
    AppealWindowClosed = 27,
    /// No slash record exists for the given id.
    SlashNotFound = 28,
    /// An appeal has already been filed for this slash.
    AppealAlreadyExists = 29,
    /// This appeal has already been resolved (approved or rejected).
    AppealAlreadyResolved = 30,
    // ── Rate limiting & validation ───────────────────────────────────────────────
    /// Caller has exceeded the allowed rate of stake/unstake operations.
    RateLimitExceeded = 31,
    /// Amount is zero, negative, or otherwise outside allowed bounds.
    InvalidAmount = 32,
    /// Partial unstake would leave a remaining stake below the minimum threshold.
    RemainingStakeBelowMinimum = 33,
    // ── Issue #811: upgrade-safe contract versioning ─────────────────────────
    /// `upgrade()` was called with a version that is not strictly greater
    /// than the currently stored contract version.
    IncompatibleContractVersion = 34,
    // ── Issue #787: stake duration-weighted voting power ─────────────────────
    /// `set_lock_multiplier_tier` was called with `weeks == 0` or a `bps`
    /// value outside `[BPS_DENOMINATOR, MAX_LOCK_MULTIPLIER_BPS]`.
    InvalidLockMultiplierTier = 35,
    // ── Issue #883: error taxonomy consolidation ──────────────────────────────
    /// `queue_unstake` was called while the unstake queue is already at
    /// `MaxUnstakeQueueSize`; distinct from `QueueEmpty` (processing an empty
    /// queue). Previously referenced without a backing variant — added here.
    QueueFull = 36,
    /// Slash tier severity order is invalid (minor > major or major > critical).
    InvalidSlashTierOrder = 37,
    /// Withdrawal cooldown duration exceeds maximum allowed limit.
    InvalidCooldown = 38,
    /// Batch size is 0 or exceeds MAX_BATCH_SIZE.
    BatchSizeInvalid = 39,
    /// Lengths of parallel batch parameter arrays do not match.
    BatchLengthMismatch = 40,
    // ── Slash cooldown ────────────────────────────────────────────────────────
    /// A slash was attempted within the cooldown window after a prior slash.
    SlashCooldownActive = 41,
}

impl StakeVaultError {
    /// Short, human-readable description of when this error is returned.
    pub fn message(&self) -> &'static str {
        match self {
            StakeVaultError::NotInitialized => {
                "vault has not been initialized yet; call initialize() first"
            }
            StakeVaultError::Unauthorized => "caller is not authorized to perform this action",
            StakeVaultError::NoStake => "caller has no active stake in the vault",
            StakeVaultError::StakeLocked => "stake is still within its lock period",
            StakeVaultError::ReentrancyDetected => "re-entrant call was detected and rejected",
            StakeVaultError::StakeBelowMinimum => {
                "stake is below the minimum and its grace period has expired"
            }
            StakeVaultError::ContractPaused => {
                "contract is paused due to suspicious activity or admin action"
            }
            StakeVaultError::TimelockRequired => {
                "large withdrawal requires a pending time-lock request first"
            }
            StakeVaultError::TimelockNotElapsed => "time-lock period has not yet elapsed",
            StakeVaultError::FlashLoanDetected => {
                "stake and unstake in the same ledger is not allowed (flash-loan pattern)"
            }
            StakeVaultError::InvalidSlashTier => "slash tier percentage would exceed 100% of stake",
            StakeVaultError::StakeDurationNotElapsed => {
                "voting power is locked until the minimum stake duration elapses"
            }
            StakeVaultError::NoDelegatedStake => {
                "no delegated stake found for this delegator/provider pair"
            }
            StakeVaultError::UnstakeAlreadyQueued => {
                "caller already has a pending unstake request in the queue"
            }
            StakeVaultError::NoUnstakeQueued => "no unstake request found for this user",
            StakeVaultError::QueueEmpty => "unstake queue is empty",
            StakeVaultError::EmergencyNotConfigured => {
                "emergency unstake is not configured (no multi-sig config set)"
            }
            StakeVaultError::EmergencyRequestAlreadyExists => {
                "a pending emergency request already exists for this staker"
            }
            StakeVaultError::EmergencyRequestNotFound => {
                "no emergency request found for this staker"
            }
            StakeVaultError::EmergencyRequestExpired => {
                "emergency request expired without sufficient approvals"
            }
            StakeVaultError::EmergencyRequestNotExpired => {
                "request has not yet expired and cannot be discarded"
            }
            StakeVaultError::AlreadyApproved => "this signer has already approved this request",
            StakeVaultError::InvalidEmergencyPenalty => "penalty basis points exceed 100%",
            StakeVaultError::InvalidMultiSigConfig => {
                "multi-sig required count is 0 or exceeds the number of admins"
            }
            StakeVaultError::RequiresMultisig => {
                "this privileged action requires multi-sig approval (propose/approve/execute)"
            }
            StakeVaultError::InvalidAppealWindow => {
                "appeal window duration is zero or otherwise invalid"
            }
            StakeVaultError::AppealWindowClosed => {
                "appeal window for this slash has already elapsed"
            }
            StakeVaultError::SlashNotFound => "no slash record exists for the given id",
            StakeVaultError::AppealAlreadyExists => {
                "an appeal has already been filed for this slash"
            }
            StakeVaultError::AppealAlreadyResolved => "this appeal has already been resolved",
            StakeVaultError::RateLimitExceeded => {
                "caller has exceeded the allowed rate of stake/unstake operations"
            }
            StakeVaultError::InvalidAmount => "amount is zero, negative, or otherwise out of range",
            StakeVaultError::RemainingStakeBelowMinimum => {
                "partial unstake would leave a remaining stake below the minimum threshold"
            }
            StakeVaultError::IncompatibleContractVersion => {
                "upgrade() version is not strictly greater than the currently stored version"
            }
            StakeVaultError::InvalidLockMultiplierTier => {
                "lock multiplier tier has weeks == 0 or a bps value out of allowed range"
            }
            StakeVaultError::QueueFull => {
                "unstake queue is at MaxUnstakeQueueSize; wait for entries to process"
            }
            StakeVaultError::InvalidSlashTierOrder => {
                "slash tiers must be non-decreasing: minor <= major <= critical"
            }
            StakeVaultError::InvalidCooldown => {
                "withdrawal cooldown exceeds maximum allowed duration"
            }
            StakeVaultError::BatchSizeInvalid => "batch is empty or exceeds maximum batch size",
            StakeVaultError::BatchLengthMismatch => {
                "batch parameter arrays have mismatched lengths"
            }
            StakeVaultError::SlashCooldownActive => {
                "slash cooldown is active; wait for the cooldown window to expire"
            }
        }
    }
}

/// A pending unstake request held in the FIFO queue (issue #663).
#[contracttype]
#[derive(Clone)]
pub struct UnstakeRequest {
    pub ticket: u64,
    pub user: Address,
    pub queued_at: u64,
}

// ── Issue #689: Slash appeal types ─────────────────────────────────────────────

/// Status of a slash appeal.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AppealStatus {
    /// Appeal submitted; awaiting admin resolution.  Fund disposition is blocked.
    Pending = 0,
    /// Admin upheld the slash; slashed funds remain burned.
    Upheld = 1,
    /// Admin reversed the slash; slashed funds are restored to the provider.
    Reversed = 2,
}

/// Immutable record written at slash time.
#[contracttype]
#[derive(Clone)]
pub struct SlashRecord {
    /// Unique monotonic identifier for this slash event.
    pub slash_id: u64,
    /// The provider whose stake was slashed.
    pub provider: Address,
    /// Severity tier applied.
    pub severity: u32,
    /// Actual amount of tokens slashed (own + delegated).
    pub slash_amount: i128,
    /// Ledger timestamp at the time of the slash.
    pub slashed_at: u64,
    /// Textual reason passed in to slash_stake.
    pub reason: Symbol,
    /// Ledger sequence when the cooldown expires (0 if no cooldown configured).
    pub cooldown_expires_at: u32,
}

/// Mutable appeal record created when a provider calls `appeal_slash`.
#[contracttype]
#[derive(Clone)]
pub struct SlashAppeal {
    /// Slash this appeal relates to.
    pub slash_id: u64,
    /// Provider who submitted the appeal.
    pub appellant: Address,
    /// Off-chain evidence URI (IPFS CID, HTTPS link, etc.).
    pub evidence_uri: soroban_sdk::String,
    /// Ledger timestamp when the appeal was submitted.
    pub submitted_at: u64,
    /// Current resolution status.
    pub status: AppealStatus,
}

soroban_sdk::contractmeta!(key = "SourceHash", val = env!("STELLAR_SOURCE_HASH"));
soroban_sdk::contractmeta!(key = "GitCommit", val = env!("STELLAR_GIT_COMMIT"));

// ── Multi-sig action discriminators ──────────────────────────────────────────

/// First byte of a multi-sig action payload identifying the privileged action.
mod action {
    pub const PAUSE: u8 = 0;
    pub const UNPAUSE: u8 = 1;
    pub const SET_MINIMUM_STAKE: u8 = 2;
    pub const SET_MINIMUM_STAKE_DURATION: u8 = 3;
    pub const CONFIGURE_SLASH_TIERS: u8 = 4;
    pub const SET_APPEAL_WINDOW: u8 = 5;
    pub const SET_WITHDRAWAL_COOLDOWN: u8 = 6;
    pub const RESOLVE_APPEAL: u8 = 7;
}

fn encode_action(env: &Env, tag: u8, data: &[u8]) -> Bytes {
    let mut b = Bytes::new(env);
    b.push_back(tag);
    for &byte in data {
        b.push_back(byte);
    }
    b
}

fn encode_i128_bytes(value: i128) -> [u8; 16] {
    let mut buf = [0u8; 16];
    let le = value.to_le_bytes();
    buf.copy_from_slice(&le);
    buf
}

fn decode_i128_from_bytes(env: &Env, payload: &Bytes, offset: u32) -> i128 {
    let mut buf = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        buf[i] = payload.get(offset + i as u32).unwrap_or(0);
        i += 1;
    }
    i128::from_le_bytes(buf)
}

#[contract]
pub struct StakeVaultContract;

#[contractimpl]
impl StakeVaultContract {
    pub fn get_build_info(env: Env) -> soroban_sdk::Map<soroban_sdk::String, soroban_sdk::String> {
        let mut m = soroban_sdk::Map::new(&env);
        m.set(
            soroban_sdk::String::from_str(&env, "version"),
            soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION")),
        );
        m.set(
            soroban_sdk::String::from_str(&env, "source_hash"),
            soroban_sdk::String::from_str(&env, env!("STELLAR_SOURCE_HASH")),
        );
        m.set(
            soroban_sdk::String::from_str(&env, "git_commit"),
            soroban_sdk::String::from_str(&env, env!("STELLAR_GIT_COMMIT")),
        );
        m
    }

    /// One-time initialization (uses shared::initializable guard, issue #584).
    pub fn initialize(env: Env, admin: Address, stake_token: Address, signal_registry: Address) {
        if initializable::is_initialized(&env) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&StorageKey::StakeToken, &stake_token);
        env.storage()
            .instance()
            .set(&StorageKey::SignalRegistry, &signal_registry);
        // Initialize pause state to false via shared::pausable (no event on init).
        env.storage()
            .instance()
            .set(&pausable::PausableKey::Paused, &false);
        initializable::mark_initialized(&env);
        shared::version::set_contract_version(&env, shared::version::STAKE_VAULT_VERSION);
    }

    // ── Issue #811: upgrade-safe contract versioning ─────────────────────────

    /// Returns this contract's stored version. Cross-contract callers can use
    /// this to enforce a minimum compatible version before invoking this
    /// contract (see `shared::version::validate_callee_version`).
    pub fn get_contract_version(env: Env) -> u32 {
        shared::version::get_contract_version(&env)
    }

    /// Admin-only: replace this contract's executable with `new_wasm_hash`
    /// (previously uploaded via `Deployer::upload_contract_wasm`) and record
    /// `new_version` as the contract's version.
    ///
    /// `new_version` must be strictly greater than the currently stored
    /// version, rejecting accidental or malicious downgrades.
    ///
    /// # Errors
    /// - [`StakeVaultError::NotInitialized`] — contract not initialized.
    /// - [`StakeVaultError::IncompatibleContractVersion`] — `new_version` is
    ///   not strictly greater than the currently stored version.
    pub fn upgrade(
        env: Env,
        new_wasm_hash: BytesN<32>,
        new_version: u32,
    ) -> Result<(), StakeVaultError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();

        let current_version = shared::version::get_contract_version(&env);
        shared::version::guard_upgrade(current_version, new_version)
            .map_err(|_| StakeVaultError::IncompatibleContractVersion)?;

        env.deployer().update_current_contract_wasm(new_wasm_hash);
        shared::version::set_contract_version(&env, new_version);
        shared::version::emit_contract_upgraded(&env, current_version, new_version);
        Ok(())
    }

    // ── Emergency pause (shared::pausable) ────────────────────────────────────

    /// Pause all stake/unstake operations.
    ///
    /// When multi-sig is configured this entrypoint returns
    /// [`StakeVaultError::RequiresMultisig`]; use `propose_action` /
    /// `approve_action` / `execute_action` instead.
    pub fn pause(env: Env) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        pausable::set_paused(&env, true);
        Ok(())
    }

    /// Resume operations. Behaviour mirrors [`pause`].
    pub fn unpause(env: Env) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        pausable::set_paused(&env, false);
        Ok(())
    }

    pub fn is_paused(env: Env) -> bool {
        pausable::is_paused(&env)
    }

    // ── Issue #865: governance-driven pause propagation ────────────────────────

    /// Set the central governance contract address authorized to call
    /// `apply_governance_pause`. Admin only.
    pub fn set_governance(
        env: Env,
        admin: Address,
        governance: Address,
    ) -> Result<(), StakeVaultError> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        if admin != stored_admin {
            return Err(StakeVaultError::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&StorageKey::GovernanceAddress, &governance);
        Ok(())
    }

    /// Read-only: the configured governance contract address, if any.
    pub fn get_governance(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::GovernanceAddress)
    }

    /// Called by the configured governance contract to propagate a pause/unpause
    /// (Issue #865). Bypasses the multi-sig requirement enforced on `pause`/`unpause`
    /// since this is a trusted, governance-initiated emergency action.
    pub fn apply_governance_pause(env: Env, paused: bool) -> Result<(), StakeVaultError> {
        let governance: Address = env
            .storage()
            .instance()
            .get(&StorageKey::GovernanceAddress)
            .ok_or(StakeVaultError::Unauthorized)?;
        governance.require_auth();
        pausable::set_paused(&env, paused);
        Ok(())
    }

    /// Read-only health probe for monitoring and front-ends (no auth). Issue #865.
    pub fn health_check(env: Env) -> stellar_swipe_common::HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        let admin: Option<Address> = env.storage().instance().get(&StorageKey::Admin);
        let Some(admin) = admin else {
            return stellar_swipe_common::health_uninitialized(&env, version);
        };
        let status = stellar_swipe_common::HealthStatus {
            is_initialized: true,
            is_paused: pausable::is_paused(&env),
            version,
            admin,
            initialized_at: env.ledger().timestamp(),
        };
        stellar_swipe_common::emit_health_event(&env, &status);
        status
    }

    // ── Helpers ────────────────────────────────────────────────────────

    fn require_not_paused(env: &Env) -> Result<(), StakeVaultError> {
        pausable::require_not_paused(env).map_err(|_| StakeVaultError::ContractPaused)
    }

    // ── Deposit stake (records ledger for flash-loan detection) ────────────────

    /// Deposit `amount` of stake tokens from `staker`.
    ///
    /// Records the current ledger sequence to detect same-ledger withdraw
    /// attempts (flash loan pattern).
    pub fn deposit_stake(env: Env, staker: Address, amount: i128) -> Result<(), StakeVaultError> {
        // Issue #859: Reentrancy guard for cross-contract token transfer.
        reentrancy::require_not_locked(&env).map_err(|_| StakeVaultError::ReentrancyDetected)?;
        staker.require_auth();
        Self::require_not_paused(&env)?;

        if amount <= 0 {
            return Err(StakeVaultError::NoStake);
        }

        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        let now = env.ledger().timestamp();
        let current = stakes.get(staker.clone()).unwrap_or(StakeInfoV2 {
            balance: 0,
            locked_until: 0,
            last_updated: 0,
        });

        let old_tier = stake_tier_for_amount(current.balance);
        let new_balance = current.balance.checked_add(amount).unwrap_or(i128::MAX);
        let new_tier = stake_tier_for_amount(new_balance);

        // ── Minimum stake duration lock ───────────────────────────────────────
        // Fresh stake (balance was 0) → lock the full amount for min_duration.
        // Top-up deposit → keep existing lock for the old portion; extend lock
        // on the combined balance so the new deposit is also locked.
        let min_duration: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::MinimumStakeDuration)
            .unwrap_or(0);

        let locked_until = if current.balance == 0 {
            // Fresh stake: lock entire amount
            now.saturating_add(min_duration)
        } else if min_duration > 0 {
            // Top-up: keep existing locked_until, but also ensure the new
            // deposit's lock is respected by extending to max(old, new_lock).
            let topup_lock = now.saturating_add(min_duration);
            core::cmp::max(current.locked_until, topup_lock)
        } else {
            current.locked_until
        };

        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance: new_balance,
                locked_until,
                last_updated: now,
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);

        // Record the deposit timestamp for voting-power eligibility checks.
        env.storage()
            .persistent()
            .set(&StorageKey::StakeDepositTimestamps(staker.clone()), &now);

        // Record the ledger sequence at deposit time for flash-loan detection.
        let ledger_seq = env.ledger().sequence();
        env.storage()
            .temporary()
            .set(&StorageKey::LastStakeLedger(staker.clone()), &ledger_seq);

        emit_provider_tier_change(&env, &staker, old_tier, new_tier, new_balance);

        // Transfer tokens into the vault (after state update — CEI pattern).
        token::Client::new(&env, &token).transfer(&staker, env.current_contract_address(), &amount);

        Ok(())
    }

    // ── Voting power (minimum stake duration aware) ─────────────────────────────

    /// Returns the voting power for `staker`.
    ///
    /// If the minimum stake duration lock has not yet elapsed, the staked balance
    /// does not count toward voting power (returns 0). This prevents flash-stake
    /// voting manipulation where a user stakes immediately before a vote and
    /// unstakes immediately after.
    ///
    /// ## Acceptance criteria (Issue #705)
    /// - Tracks the timestamp at which each stake deposit was made ✓
    /// - Admin-configurable minimum duration that newly staked funds must remain
    ///   locked before being eligible to count toward voting power ✓
    /// - Top-up deposits to an existing stake tracked separately so only the new
    ///   portion is subject to the fresh lock ✓
    ///
    /// ## Lock-duration-weighted multiplier (Issue #787)
    /// Once locked, a stake's remaining lock time (`locked_until - now`) is
    /// looked up against the configured `LockMultiplierTier` table
    /// (`get_lock_multiplier_tiers`) and the balance is scaled by the
    /// matching tier's basis points. A remaining duration below the smallest
    /// configured tier keeps the pre-existing zero-voting-power behaviour; a
    /// stake with no lock, or whose lock has fully elapsed, always counts at
    /// the 1x baseline.
    pub fn get_voting_power(env: Env, staker: Address) -> i128 {
        let stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        let info = match stakes.get(staker.clone()) {
            Some(info) => info,
            None => return 0,
        };

        if info.balance == 0 {
            return 0;
        }

        let now = env.ledger().timestamp();
        if info.locked_until == 0 || now >= info.locked_until {
            // No lock, or the lock has fully elapsed — baseline 1x.
            return info.balance;
        }

        // Still locked — apply the tiered multiplier for the remaining duration.
        let remaining = info.locked_until - now;
        let tiers = Self::get_lock_multiplier_tiers(env.clone());
        let bps = lock_multiplier_bps_for_remaining(&tiers, remaining);
        if bps == 0 {
            return 0;
        }
        apply_multiplier_bps(info.balance, bps)
    }

    // ── Admin: configure lock-duration voting-power multiplier tiers ───────────

    /// Returns the configured lock-multiplier tiers, or the built-in defaults
    /// (1 week = 1x, 4 weeks = 1.25x, 12 weeks = 2x) if none have been set.
    pub fn get_lock_multiplier_tiers(env: Env) -> Vec<LockMultiplierTier> {
        env.storage()
            .instance()
            .get(&StorageKey::LockMultiplierTiers)
            .unwrap_or_else(|| default_lock_multiplier_tiers(&env))
    }

    /// Admin-only: set (or update) the voting-power multiplier for stakes
    /// whose remaining lock time is at least `weeks`, in basis points
    /// (`10_000` = 1x). `bps` must be in `[10_000, MAX_LOCK_MULTIPLIER_BPS]` —
    /// a lock can never be worth less than the unlocked baseline.
    pub fn set_lock_multiplier_tier(env: Env, weeks: u32, bps: u32) -> Result<(), StakeVaultError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();

        if weeks == 0 || bps < BPS_DENOMINATOR as u32 || bps > MAX_LOCK_MULTIPLIER_BPS {
            return Err(StakeVaultError::InvalidLockMultiplierTier);
        }

        let existing = Self::get_lock_multiplier_tiers(env.clone());
        let mut updated_tiers: Vec<LockMultiplierTier> = Vec::new(&env);
        let mut replaced = false;
        for tier in existing.iter() {
            if tier.weeks == weeks {
                updated_tiers.push_back(LockMultiplierTier { weeks, bps });
                replaced = true;
            } else {
                updated_tiers.push_back(tier);
            }
        }
        if !replaced {
            updated_tiers.push_back(LockMultiplierTier { weeks, bps });
        }

        env.storage()
            .instance()
            .set(&StorageKey::LockMultiplierTiers, &updated_tiers);
        events::emit_lock_multiplier_tier_updated(&env, weeks, bps);
        Ok(())
    }

    // ── Admin: configure minimum stake duration ────────────────────────────────

    /// Set the minimum duration (in seconds) that newly staked funds
    /// must remain locked before counting toward voting power.
    ///
    /// When multi-sig is configured use `execute_action` with action
    /// [`action::SET_MINIMUM_STAKE_DURATION`] instead.
    pub fn set_minimum_stake_duration(env: Env, duration_secs: u64) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::MinimumStakeDuration, &duration_secs);
        events::emit_min_stake_duration_updated(&env, duration_secs);
        Ok(())
    }

    /// Returns the configured minimum stake duration in seconds (defaults to 0).
    pub fn get_minimum_stake_duration(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&StorageKey::MinimumStakeDuration)
            .unwrap_or(0)
    }

    /// Returns the timestamp when `staker` last deposited stake.
    pub fn get_stake_deposit_timestamp(env: Env, staker: Address) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&StorageKey::StakeDepositTimestamps(staker))
    }

    // ── Minimum stake ──────────────────────────────────────────────────────────

    pub fn set_minimum_stake(env: Env, minimum: i128) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::MinimumStake, &minimum);
        Ok(())
    }

    pub fn get_minimum_stake(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0)
    }

    // ── Multi-sig approval flow ────────────────────────────────────────────────

    /// Admin: configure the N-of-M multi-sig parameters for privileged actions.
    ///
    /// Once configured, single-admin-gated entrypoints such as
    /// [`set_minimum_stake`] will return [`StakeVaultError::RequiresMultisig`]
    /// and must instead be invoked via `propose_action` / `approve_action` /
    /// `execute_action`.
    ///
    /// Pass `None` to clear the config and restore single-admin behaviour.
    pub fn set_multisig_config(
        env: Env,
        caller: Address,
        config: Option<multisig::MultisigConfig>,
    ) -> Result<(), StakeVaultError> {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        if caller != admin {
            return Err(StakeVaultError::Unauthorized);
        }
        if let Some(cfg) = config {
            multisig::set_config(&env, &cfg).map_err(|_| StakeVaultError::InvalidMultiSigConfig)?;
        } else {
            env.storage()
                .instance()
                .remove(&multisig::MultisigStorageKey::Config);
        }
        Ok(())
    }

    /// Propose a privileged action. `caller` must be a registered signer.
    /// Returns the proposal ID.
    pub fn propose_action(
        env: Env,
        caller: Address,
        payload: Bytes,
    ) -> Result<u64, StakeVaultError> {
        caller.require_auth();
        multisig::propose(&env, &caller, payload).map_err(|_| StakeVaultError::Unauthorized)
    }

    /// Approve a pending proposal. `caller` must be a registered signer.
    /// Returns the current number of approvals.
    pub fn approve_action(
        env: Env,
        caller: Address,
        proposal_id: u64,
    ) -> Result<u32, StakeVaultError> {
        caller.require_auth();
        multisig::approve(&env, &caller, proposal_id).map_err(|e| -> StakeVaultError {
            match e {
                multisig::MultisigError::ProposalNotFound => {
                    StakeVaultError::EmergencyRequestNotFound
                }
                multisig::MultisigError::AlreadyApproved => StakeVaultError::AlreadyApproved,
                multisig::MultisigError::ProposalExpired => {
                    StakeVaultError::EmergencyRequestExpired
                }
                _ => StakeVaultError::Unauthorized,
            }
        })
    }

    /// Execute an approved proposal. Decodes the action payload and performs
    /// the privileged action, then marks the proposal as executed.
    pub fn execute_action(
        env: Env,
        caller: Address,
        proposal_id: u64,
    ) -> Result<(), StakeVaultError> {
        caller.require_auth();
        let proposal = multisig::get_proposal(&env, proposal_id)
            .map_err(|_| StakeVaultError::EmergencyRequestNotFound)?;
        multisig::require_can_execute(&env, &proposal).map_err(|e| -> StakeVaultError {
            match e {
                multisig::MultisigError::AlreadyExecuted => {
                    StakeVaultError::EmergencyRequestNotFound
                }
                multisig::MultisigError::ThresholdNotMet => StakeVaultError::Unauthorized,
                multisig::MultisigError::ProposalExpired => {
                    StakeVaultError::EmergencyRequestExpired
                }
                _ => StakeVaultError::Unauthorized,
            }
        })?;

        let payload = proposal.payload;
        let tag = payload.get(0).ok_or(StakeVaultError::Unauthorized)?;
        match tag {
            action::PAUSE => Self::do_pause(&env)?,
            action::UNPAUSE => Self::do_unpause(&env)?,
            action::SET_MINIMUM_STAKE => {
                let minimum = decode_i128_from_bytes(&env, &payload, 1);
                env.storage()
                    .instance()
                    .set(&StorageKey::MinimumStake, &minimum);
            }
            action::SET_MINIMUM_STAKE_DURATION => {
                let mut buf = [0u8; 8];
                let mut i = 0;
                while i < 8 {
                    buf[i] = payload.get(1 + i as u32).unwrap_or(0);
                    i += 1;
                }
                let duration = u64::from_le_bytes(buf);
                env.storage()
                    .instance()
                    .set(&StorageKey::MinimumStakeDuration, &duration);
            }
            action::CONFIGURE_SLASH_TIERS => {
                let minor = decode_i128_from_bytes(&env, &payload, 1) as u32;
                let major = decode_i128_from_bytes(&env, &payload, 17) as u32;
                let critical = decode_i128_from_bytes(&env, &payload, 33) as u32;
                validate_slash_tiers(minor, major, critical)?;
                let cfg = SlashTierConfig {
                    minor_bps: minor,
                    major_bps: major,
                    critical_bps: critical,
                };
                env.storage()
                    .instance()
                    .set(&StorageKey::SlashTierConfig, &cfg);
            }
            action::SET_APPEAL_WINDOW => {
                let mut buf = [0u8; 8];
                let mut i = 0;
                while i < 8 {
                    buf[i] = payload.get(1 + i as u32).unwrap_or(0);
                    i += 1;
                }
                let window = u64::from_le_bytes(buf);
                if window > 2_592_000 {
                    return Err(StakeVaultError::InvalidAppealWindow);
                }
                env.storage()
                    .instance()
                    .set(&StorageKey::AppealWindowSecs, &window);
            }
            action::SET_WITHDRAWAL_COOLDOWN => {
                let mut buf = [0u8; 8];
                let mut i = 0;
                while i < 8 {
                    buf[i] = payload.get(1 + i as u32).unwrap_or(0);
                    i += 1;
                }
                let cooldown = u64::from_le_bytes(buf);
                validate_cooldown(cooldown)?;
                env.storage()
                    .instance()
                    .set(&StorageKey::WithdrawalCooldownSecs, &cooldown);
            }
            action::RESOLVE_APPEAL => {
                // Payload: [tag][slash_id: 8-byte LE][uphold: 1 byte]
                let mut buf = [0u8; 8];
                let mut i = 0;
                while i < 8 {
                    buf[i] = payload.get(1 + i as u32).unwrap_or(0);
                    i += 1;
                }
                let slash_id = u64::from_le_bytes(buf);
                let uphold = payload.get(9).unwrap_or(0) == 1;
                Self::do_resolve_appeal(&env, slash_id, uphold)?;
            }
            _ => return Err(StakeVaultError::Unauthorized),
        }

        multisig::mark_executed(&env, proposal_id).map_err(|_| StakeVaultError::Unauthorized)?;
        Ok(())
    }

    // ── Internal execution helpers ────────────────────────────────────────────

    fn do_pause(env: &Env) -> Result<(), StakeVaultError> {
        pausable::set_paused(env, true);
        Ok(())
    }

    fn do_unpause(env: &Env) -> Result<(), StakeVaultError> {
        pausable::set_paused(env, false);
        Ok(())
    }

    /// Core slash-appeal resolution logic, shared by the single-admin
    /// `resolve_appeal` entrypoint (used when no multi-sig config is set)
    /// and the multi-sig `execute_action` dispatch (used once one is).
    fn do_resolve_appeal(env: &Env, slash_id: u64, uphold: bool) -> Result<(), StakeVaultError> {
        // Load slash record.
        let record: SlashRecord = env
            .storage()
            .persistent()
            .get(&StorageKey::SlashRecord(slash_id))
            .ok_or(StakeVaultError::SlashNotFound)?;

        // Load appeal record.
        let mut appeal: SlashAppeal = env
            .storage()
            .persistent()
            .get(&StorageKey::SlashAppeal(slash_id))
            .ok_or(StakeVaultError::SlashNotFound)?;

        if appeal.status != AppealStatus::Pending {
            return Err(StakeVaultError::AppealAlreadyResolved);
        }

        // Retrieve held funds (only present when an appeal window is configured
        // and the slash_stake did not burn immediately).
        let held: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::SlashedFundsHeld(slash_id))
            .unwrap_or(0);

        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        if uphold {
            appeal.status = AppealStatus::Upheld;
            // Slash confirmed — burn the held funds now.
            if held > 0 {
                env.storage()
                    .persistent()
                    .remove(&StorageKey::SlashedFundsHeld(slash_id));
                token::Client::new(env, &token).burn(&env.current_contract_address(), &held);
            }
        } else {
            // Reversed — tokens are still in the vault; credit provider's stake.
            appeal.status = AppealStatus::Reversed;

            let amount_to_restore = if held > 0 { held } else { record.slash_amount };

            let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
                .storage()
                .persistent()
                .get(&MigrationKey::StakesV2)
                .unwrap_or_else(|| soroban_sdk::Map::new(env));

            let current_info = stakes.get(record.provider.clone()).unwrap_or(StakeInfoV2 {
                balance: 0,
                locked_until: 0,
                last_updated: 0,
            });

            let restored_balance = current_info.balance.saturating_add(amount_to_restore);
            let now = env.ledger().timestamp();

            stakes.set(
                record.provider.clone(),
                StakeInfoV2 {
                    balance: restored_balance,
                    locked_until: current_info.locked_until,
                    last_updated: now,
                },
            );
            env.storage()
                .persistent()
                .set(&MigrationKey::StakesV2, &stakes);

            if held > 0 {
                env.storage()
                    .persistent()
                    .remove(&StorageKey::SlashedFundsHeld(slash_id));
            }
        }

        env.storage()
            .persistent()
            .set(&StorageKey::SlashAppeal(slash_id), &appeal);

        #[allow(deprecated)]
        env.events().publish(
            (
                Symbol::new(env, "stake_vault"),
                Symbol::new(env, "appeal_resolved"),
            ),
            (slash_id, uphold, record.provider),
        );

        Ok(())
    }

    pub fn notify_stake_below_minimum(env: Env, provider: Address) {
        let minimum: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0);

        let current_stake = Self::get_stake(env.clone(), provider.clone());

        if current_stake >= minimum {
            return;
        }

        let key = StorageKey::StakeBelowMinSince(provider.clone());
        if !env.storage().persistent().has(&key) {
            let now = env.ledger().timestamp();
            env.storage().persistent().set(&key, &now);

            events::emit_stake_below_min(&env, provider, current_stake, minimum);
        }
    }

    pub fn check_signal_submission_allowed(
        env: Env,
        provider: Address,
    ) -> Result<(), StakeVaultError> {
        let minimum: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0);

        let current_stake = Self::get_stake(env.clone(), provider.clone());

        if current_stake >= minimum {
            let key = StorageKey::StakeBelowMinSince(provider);
            env.storage().persistent().remove(&key);
            return Ok(());
        }

        let key = StorageKey::StakeBelowMinSince(provider.clone());
        let below_since: u64 = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.ledger().timestamp());

        let now = env.ledger().timestamp();
        if now.saturating_sub(below_since) > GRACE_PERIOD_SECS {
            Err(StakeVaultError::StakeBelowMinimum)
        } else {
            Ok(())
        }
    }

    pub fn get_stake_below_min_since(env: Env, provider: Address) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&StorageKey::StakeBelowMinSince(provider))
    }

    // ── Large-withdrawal time-lock (issue #816: configurable cooldown) ─────────

    /// Admin: configure the large-withdrawal cooldown duration (seconds).
    ///
    /// Bounded to `[0, MAX_WITHDRAWAL_COOLDOWN_SECS]`. `0` disables the delay —
    /// a large withdrawal becomes actionable immediately after `request_withdrawal`.
    /// Falls back to the built-in default (1 hour) until this is called.
    ///
    /// When multi-sig is configured use `execute_action` with action
    /// [`action::SET_WITHDRAWAL_COOLDOWN`] instead.
    pub fn set_withdrawal_cooldown(env: Env, cooldown_secs: u64) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        validate_cooldown(cooldown_secs)?;
        env.storage()
            .instance()
            .set(&StorageKey::WithdrawalCooldownSecs, &cooldown_secs);
        events::emit_withdrawal_cooldown_updated(&env, cooldown_secs);
        Ok(())
    }

    /// Returns the configured large-withdrawal cooldown in seconds. Defaults to
    /// the built-in 1-hour timelock until an admin sets a custom value.
    pub fn get_withdrawal_cooldown(env: Env) -> u64 {
        get_withdrawal_cooldown_secs(&env)
    }

    /// Initiate a time-locked withdrawal request for a large stake.
    ///
    /// After calling this, the staker must wait for the configured withdrawal
    /// cooldown (see [`Self::get_withdrawal_cooldown`]) before `withdraw_stake`
    /// will succeed for amounts >= `LARGE_WITHDRAWAL_THRESHOLD`.
    pub fn request_withdrawal(env: Env, staker: Address) -> Result<(), StakeVaultError> {
        staker.require_auth();
        Self::require_not_paused(&env)?;

        let balance = Self::get_stake(env.clone(), staker.clone());
        if balance < LARGE_WITHDRAWAL_THRESHOLD {
            // Small withdrawals don't need a time-lock request.
            return Ok(());
        }

        let now = env.ledger().timestamp();
        env.storage().persistent().set(
            &StorageKey::LargeWithdrawalRequestedAt(staker.clone()),
            &now,
        );

        let cooldown = get_withdrawal_cooldown_secs(&env);
        events::emit_withdrawal_requested(&env, staker, balance, now.saturating_add(cooldown));

        Ok(())
    }

    pub fn get_withdrawal_unlock_time(env: Env, staker: Address) -> Option<u64> {
        let cooldown = get_withdrawal_cooldown_secs(&env);
        env.storage()
            .persistent()
            .get::<_, u64>(&StorageKey::LargeWithdrawalRequestedAt(staker))
            .map(|t| t.saturating_add(cooldown))
    }

    // ── Withdraw ───────────────────────────────────────────────────────────────

    /// Withdraw all unlocked stake for `staker`.
    ///
    /// Flash loan protections applied:
    /// 1. Reentrancy guard (temporary storage lock).
    /// 2. Same-ledger deposit+withdraw detection.
    /// 3. Time-lock for large withdrawals (>= LARGE_WITHDRAWAL_THRESHOLD).
    ///
    /// Authorization is scoped to `(staker, amount)` via `require_auth_for_args`
    /// so a valid signature for one withdrawal amount cannot be replayed for a
    /// different amount (Issue #563).
    pub fn withdraw_stake(env: Env, staker: Address) -> Result<i128, StakeVaultError> {
        Self::require_not_paused(&env)?;

        if stellar_swipe_common::rate_limit::check_rate_limit(
            &env,
            &staker,
            stellar_swipe_common::rate_limit::ActionType::StakeChange,
            0,
        )
        .is_err()
        {
            return Err(StakeVaultError::RateLimitExceeded);
        }

        // Read the stake balance first so we can scope the auth signature to
        // the exact amount being withdrawn, preventing signature reuse attacks.
        let stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));
        let amount_to_withdraw = stakes.get(staker.clone()).map(|i| i.balance).unwrap_or(0);

        let mut auth_args: Vec<Val> = Vec::new(&env);
        auth_args.push_back(staker.clone().into_val(&env));
        auth_args.push_back(amount_to_withdraw.into_val(&env));
        staker.require_auth_for_args(auth_args);

        // ── Reentrancy guard ──────────────────────────────────────────────────
        let lock_key = Symbol::new(&env, EXECUTION_LOCK);
        if env
            .storage()
            .temporary()
            .get::<_, bool>(&lock_key)
            .unwrap_or(false)
        {
            return Err(StakeVaultError::ReentrancyDetected);
        }
        env.storage().temporary().set(&lock_key, &true);

        let result = Self::do_withdraw(&env, &staker);

        env.storage().temporary().remove(&lock_key);

        if result.is_ok() {
            stellar_swipe_common::rate_limit::record_action(
                &env,
                &staker,
                stellar_swipe_common::rate_limit::ActionType::StakeChange,
            );
        }

        result
    }

    fn do_withdraw(env: &Env, staker: &Address) -> Result<i128, StakeVaultError> {
        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(env));

        let info = stakes.get(staker.clone()).ok_or(StakeVaultError::NoStake)?;

        if info.balance == 0 {
            return Err(StakeVaultError::NoStake);
        }

        let now = env.ledger().timestamp();
        if now < info.locked_until {
            return Err(StakeVaultError::StakeLocked);
        }

        // ── Flash loan detection: same-ledger stake + unstake ─────────────────
        let current_ledger = env.ledger().sequence();
        let last_stake_ledger = env
            .storage()
            .temporary()
            .get::<_, u32>(&StorageKey::LastStakeLedger(staker.clone()))
            .unwrap_or(0);
        if last_stake_ledger == current_ledger && current_ledger != 0 {
            // Emit alert event for monitoring system.
            events::emit_flash_loan_attempt(env, staker.clone(), info.balance, current_ledger);
            return Err(StakeVaultError::FlashLoanDetected);
        }

        // ── Time-lock for large withdrawals ───────────────────────────────────
        if info.balance >= LARGE_WITHDRAWAL_THRESHOLD {
            let requested_at: u64 = env
                .storage()
                .persistent()
                .get(&StorageKey::LargeWithdrawalRequestedAt(staker.clone()))
                .ok_or(StakeVaultError::TimelockRequired)?;

            if now < requested_at.saturating_add(get_withdrawal_cooldown_secs(env)) {
                return Err(StakeVaultError::TimelockNotElapsed);
            }

            // Consume the request so it can't be reused.
            env.storage()
                .persistent()
                .remove(&StorageKey::LargeWithdrawalRequestedAt(staker.clone()));
        }

        let amount = info.balance;
        let old_tier = stake_tier_for_amount(info.balance);
        let new_tier = stake_tier_for_amount(0);

        // Zero balance before transfer (checks-effects-interactions).
        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance: 0,
                locked_until: info.locked_until,
                last_updated: now,
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);

        emit_provider_tier_change(env, staker, old_tier, new_tier, 0);

        // Cross-contract call: transfer tokens back to staker.
        token::Client::new(env, &token).transfer(&env.current_contract_address(), staker, &amount);

        Ok(amount)
    }

    // ── Partial unstake ────────────────────────────────────────────────────────

    /// Withdraw `amount` of staked tokens from `staker`, leaving the remainder staked.
    ///
    /// Any unvested reward forfeiture is calculated proportionally to the withdrawn
    /// fraction only — the remaining stake's lock and accrued rewards are untouched.
    ///
    /// Constraints:
    /// - `amount` must be > 0 and strictly less than the full staked balance
    ///   (use `withdraw_stake` for a full withdrawal).
    /// - After withdrawal the remaining stake must be >= `minimum_stake` if one
    ///   has been configured.
    /// - Same flash-loan, large-withdrawal time-lock, and reentrancy protections
    ///   as `withdraw_stake` apply.
    pub fn partial_unstake(
        env: Env,
        staker: Address,
        amount: i128,
    ) -> Result<i128, StakeVaultError> {
        Self::require_not_paused(&env)?;

        if amount <= 0 {
            return Err(StakeVaultError::InvalidAmount);
        }

        // Rate limit: counts as a StakeChange alongside full withdraw_stake calls.
        if stellar_swipe_common::rate_limit::check_rate_limit(
            &env,
            &staker,
            stellar_swipe_common::rate_limit::ActionType::StakeChange,
            0,
        )
        .is_err()
        {
            return Err(StakeVaultError::RateLimitExceeded);
        }

        staker.require_auth();

        // Reentrancy guard.
        let lock_key = Symbol::new(&env, EXECUTION_LOCK);
        if env
            .storage()
            .temporary()
            .get::<_, bool>(&lock_key)
            .unwrap_or(false)
        {
            return Err(StakeVaultError::ReentrancyDetected);
        }
        env.storage().temporary().set(&lock_key, &true);

        let result = Self::do_partial_unstake(&env, &staker, amount);

        env.storage().temporary().remove(&lock_key);

        if result.is_ok() {
            stellar_swipe_common::rate_limit::record_action(
                &env,
                &staker,
                stellar_swipe_common::rate_limit::ActionType::StakeChange,
            );
        }

        result
    }

    fn do_partial_unstake(
        env: &Env,
        staker: &Address,
        amount: i128,
    ) -> Result<i128, StakeVaultError> {
        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(env));

        let info = stakes.get(staker.clone()).ok_or(StakeVaultError::NoStake)?;

        if info.balance == 0 {
            return Err(StakeVaultError::NoStake);
        }

        // amount must be strictly less than the full balance — full withdrawal
        // belongs in withdraw_stake.
        if amount >= info.balance {
            return Err(StakeVaultError::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        if now < info.locked_until {
            return Err(StakeVaultError::StakeLocked);
        }

        // Flash loan detection: same-ledger stake + unstake.
        let current_ledger = env.ledger().sequence();
        let last_stake_ledger = env
            .storage()
            .temporary()
            .get::<_, u32>(&StorageKey::LastStakeLedger(staker.clone()))
            .unwrap_or(0);
        if last_stake_ledger == current_ledger && current_ledger != 0 {
            events::emit_flash_loan_attempt(env, staker.clone(), info.balance, current_ledger);
            return Err(StakeVaultError::FlashLoanDetected);
        }

        // Time-lock for large partial withdrawals (threshold applies to the withdrawn amount).
        if amount >= LARGE_WITHDRAWAL_THRESHOLD {
            let requested_at: u64 = env
                .storage()
                .persistent()
                .get(&StorageKey::LargeWithdrawalRequestedAt(staker.clone()))
                .ok_or(StakeVaultError::TimelockRequired)?;

            if now < requested_at.saturating_add(get_withdrawal_cooldown_secs(env)) {
                return Err(StakeVaultError::TimelockNotElapsed);
            }

            // Consume the request so it can't be reused.
            env.storage()
                .persistent()
                .remove(&StorageKey::LargeWithdrawalRequestedAt(staker.clone()));
        }

        let remaining = info.balance - amount;

        // Enforce minimum remaining stake.
        let minimum_stake: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::MinimumStake)
            .unwrap_or(0);
        if minimum_stake > 0 && remaining < minimum_stake {
            return Err(StakeVaultError::RemainingStakeBelowMinimum);
        }

        let old_tier = stake_tier_for_amount(info.balance);
        let new_tier = stake_tier_for_amount(remaining);

        // Reduce balance by the withdrawn amount; the remaining stake keeps its
        // lock expiry — only the withdrawn fraction's unvested rewards are forfeited.
        stakes.set(
            staker.clone(),
            StakeInfoV2 {
                balance: remaining,
                locked_until: info.locked_until,
                last_updated: now,
            },
        );
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);

        emit_provider_tier_change(env, staker, old_tier, new_tier, remaining);

        events::emit_partial_unstake(env, staker.clone(), amount, remaining);

        // Transfer tokens back to staker (CEI: state updated before cross-contract call).
        token::Client::new(env, &token).transfer(&env.current_contract_address(), staker, &amount);

        Ok(amount)
    }

    // ── Slash ──────────────────────────────────────────────────────────────────

    /// Configure the slash percentage for each severity tier (in basis points).
    /// `minor_bps`, `major_bps`, `critical_bps` must all be <= 10_000.
    ///
    /// When multi-sig is configured use `execute_action` with action
    /// [`action::CONFIGURE_SLASH_TIERS`] instead.
    pub fn configure_slash_tiers(
        env: Env,
        minor_bps: u32,
        major_bps: u32,
        critical_bps: u32,
    ) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        Self::set_slash_tiers(&env, minor_bps, major_bps, critical_bps)
    }

    /// Governance-driven slash-tier configuration (Issue #governance_slash).
    ///
    /// Allows the configured governance contract (`set_governance`) to update
    /// slash severity percentages without requiring the admin/multi-sig path.
    pub fn governance_configure_slash_tiers(
        env: Env,
        governance: Address,
        minor_bps: u32,
        major_bps: u32,
        critical_bps: u32,
    ) -> Result<(), StakeVaultError> {
        let stored_governance: Address = env
            .storage()
            .instance()
            .get(&StorageKey::GovernanceAddress)
            .ok_or(StakeVaultError::Unauthorized)?;
        if governance != stored_governance {
            return Err(StakeVaultError::Unauthorized);
        }
        governance.require_auth();
        Self::set_slash_tiers(&env, minor_bps, major_bps, critical_bps)
    }

    fn set_slash_tiers(
        env: &Env,
        minor_bps: u32,
        major_bps: u32,
        critical_bps: u32,
    ) -> Result<(), StakeVaultError> {
        validate_slash_tiers(minor_bps, major_bps, critical_bps)?;
        let cfg = SlashTierConfig {
            minor_bps,
            major_bps,
            critical_bps,
        };
        env.storage()
            .instance()
            .set(&StorageKey::SlashTierConfig, &cfg);
        events::emit_slash_tiers_updated(env, minor_bps, major_bps, critical_bps);
        Ok(())
    }

    pub fn get_slash_tier_config(env: Env) -> SlashTierConfig {
        env.storage()
            .instance()
            .get(&StorageKey::SlashTierConfig)
            .unwrap_or_else(SlashTierConfig::default_config)
    }

    /// Slash `provider`'s stake according to `severity`.
    ///
    /// The slashed amount is computed from the configured tier percentages
    /// (default: minor=5%, major=30%, critical=100%).  Only the signal registry
    /// may call this.
    pub fn slash_stake(
        env: Env,
        caller: Address,
        provider: Address,
        severity: SlashSeverity,
        reason: Symbol,
    ) -> Result<i128, StakeVaultError> {
        // Issue #859: Reentrancy guard for cross-contract token transfer.
        reentrancy::require_not_locked(&env).map_err(|_| StakeVaultError::ReentrancyDetected)?;
        caller.require_auth();
        Self::require_signal_registry(&env, &caller)?;
        Self::do_slash(&env, &provider, severity, reason)
    }

    /// Slash multiple providers in a single call (issue #815).
    ///
    /// `providers` and `severities` must have matching length, bounded by
    /// `MAX_BATCH_SIZE`. `reason` applies uniformly to every slash in the batch.
    /// Only the signal registry may call this — authorization is checked once
    /// for the whole batch rather than per item.
    ///
    /// Individual providers with no stake are skipped (amount `0` in the
    /// returned vector) rather than aborting the entire batch, so one bad
    /// entry cannot block settlement for the rest. Each successful slash still
    /// emits its own `stake_slashed` event for per-provider audit trails, plus
    /// one aggregate `batch_slash_completed` event summarizing the run.
    pub fn batch_slash_stake(
        env: Env,
        caller: Address,
        providers: Vec<Address>,
        severities: Vec<SlashSeverity>,
        reason: Symbol,
    ) -> Result<Vec<i128>, StakeVaultError> {
        caller.require_auth();
        Self::require_signal_registry(&env, &caller)?;

        let len = providers.len();
        if len == 0 || len > MAX_BATCH_SIZE {
            return Err(StakeVaultError::BatchSizeInvalid);
        }
        if len != severities.len() {
            return Err(StakeVaultError::BatchLengthMismatch);
        }

        let mut results: Vec<i128> = Vec::new(&env);
        let mut total_slashed: i128 = 0;
        let mut processed_count: u32 = 0;

        for i in 0..len {
            let provider = providers.get(i).unwrap();
            let severity = severities.get(i).unwrap();
            match Self::do_slash(&env, &provider, severity, reason.clone()) {
                Ok(amount) => {
                    total_slashed = total_slashed.saturating_add(amount);
                    processed_count = processed_count.saturating_add(1);
                    results.push_back(amount);
                }
                Err(_) => results.push_back(0),
            }
        }

        events::emit_batch_slash_completed(&env, processed_count, total_slashed);
        Ok(results)
    }

    fn require_signal_registry(env: &Env, caller: &Address) -> Result<(), StakeVaultError> {
        let signal_registry: Address = env
            .storage()
            .instance()
            .get(&StorageKey::SignalRegistry)
            .ok_or(StakeVaultError::NotInitialized)?;
        if caller != &signal_registry {
            return Err(StakeVaultError::Unauthorized);
        }
        Ok(())
    }

    /// Core slashing logic shared by [`Self::slash_stake`] and
    /// [`Self::batch_slash_stake`]. Caller authorization/registry checks are
    /// the responsibility of the entry point — this assumes the caller is
    /// already validated.
    fn do_slash(
        env: &Env,
        provider: &Address,
        severity: SlashSeverity,
        reason: Symbol,
    ) -> Result<i128, StakeVaultError> {
        let provider = provider.clone();
        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        // ── Slash cooldown check ──────────────────────────────────────────────
        let current_ledger = env.ledger().sequence();
        let last_slash_ledger: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::LastSlashLedger)
            .unwrap_or(0);
        let cooldown: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::SlashCooldownLedgers)
            .unwrap_or(DEFAULT_SLASH_COOLDOWN_LEDGERS);
        if last_slash_ledger > 0 && current_ledger.saturating_sub(last_slash_ledger) < cooldown {
            return Err(StakeVaultError::SlashCooldownActive);
        }

        let cfg: SlashTierConfig = env
            .storage()
            .instance()
            .get(&StorageKey::SlashTierConfig)
            .unwrap_or_else(SlashTierConfig::default_config);

        let tier_bps = match severity {
            SlashSeverity::Minor => cfg.minor_bps as i128,
            SlashSeverity::Major => cfg.major_bps as i128,
            SlashSeverity::Critical => cfg.critical_bps as i128,
        };

        let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        let mut info = stakes
            .get(provider.clone())
            .ok_or(StakeVaultError::NoStake)?;

        if info.balance == 0 {
            return Err(StakeVaultError::NoStake);
        }

        // Compute slash on own stake; min 1 stroop if balance > 0.
        let own_slash = if info.balance > 0 {
            let s = core::cmp::max((info.balance * tier_bps) / BPS_DENOMINATOR, 1);
            core::cmp::min(s, info.balance)
        } else {
            0
        };

        info.balance = info.balance.saturating_sub(own_slash);
        info.last_updated = env.ledger().timestamp();
        stakes.set(provider.clone(), info);
        env.storage()
            .persistent()
            .set(&MigrationKey::StakesV2, &stakes);

        // ── Slash delegated stakes pro-rata (issue #688) ──────────────────────
        let mut delegated: soroban_sdk::Map<Address, i128> = env
            .storage()
            .persistent()
            .get(&StorageKey::ProviderDelegatedStakes(provider.clone()))
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        let delegator_keys = delegated.keys();
        let mut delegated_slash_total: i128 = 0;

        for i in 0..delegator_keys.len() {
            let delegator = delegator_keys.get(i).unwrap();
            let d_amount = delegated.get(delegator.clone()).unwrap_or(0);
            if d_amount == 0 {
                continue;
            }
            let d_slash = core::cmp::max((d_amount * tier_bps) / BPS_DENOMINATOR, 1);
            let d_slash = core::cmp::min(d_slash, d_amount);
            delegated.set(delegator, d_amount.saturating_sub(d_slash));
            delegated_slash_total = delegated_slash_total.saturating_add(d_slash);
        }

        if delegated_slash_total > 0 {
            env.storage().persistent().set(
                &StorageKey::ProviderDelegatedStakes(provider.clone()),
                &delegated,
            );

            let total_delegated: i128 = env
                .storage()
                .persistent()
                .get(&StorageKey::ProviderTotalDelegated(provider.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &StorageKey::ProviderTotalDelegated(provider.clone()),
                &total_delegated.saturating_sub(delegated_slash_total),
            );
        }

        let slash_amount = own_slash.saturating_add(delegated_slash_total);
        let slash_amount = if slash_amount == 0 { 1 } else { slash_amount };

        // ── Assign a unique slash_id and persist the SlashRecord (issue #689) ──
        let slash_id: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::SlashCounter)
            .unwrap_or(0u64);
        let next_slash_id = slash_id.saturating_add(1);
        env.storage()
            .instance()
            .set(&StorageKey::SlashCounter, &next_slash_id);

        let record = SlashRecord {
            slash_id,
            provider: provider.clone(),
            severity: severity as u32,
            slash_amount,
            slashed_at: env.ledger().timestamp(),
            reason: reason.clone(),
            cooldown_expires_at: if cooldown > 0 {
                current_ledger.saturating_add(cooldown)
            } else {
                0
            },
        };
        env.storage()
            .persistent()
            .set(&StorageKey::SlashRecord(slash_id), &record);

        // ── Record last slash ledger for cooldown enforcement ──────────────────
        env.storage()
            .instance()
            .set(&StorageKey::LastSlashLedger, &current_ledger);

        // ── Hold slashed funds pending appeal resolution (issue #689) ──────────
        // Tokens remain in the vault's custody and are NOT burned until the
        // appeal window closes or an admin resolves the appeal.  This puts the
        // slash in a "disputed" state that blocks final fund disposition.
        let appeal_window_secs: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::AppealWindowSecs)
            .unwrap_or(0);

        if appeal_window_secs > 0 {
            // Park the slashed amount; resolve_appeal / finalize_slash will burn or return it.
            env.storage()
                .persistent()
                .set(&StorageKey::SlashedFundsHeld(slash_id), &slash_amount);
        } else {
            // No appeal window configured — burn immediately (legacy behaviour).
            token::Client::new(&env, &token).burn(&env.current_contract_address(), &slash_amount);
        }

        // Event records severity tier, slash amount, and slash_id for audit.
        events::emit_stake_slashed(
            &env,
            provider.clone(),
            severity as u32,
            slash_amount,
            slash_id,
            reason,
        );

        Ok(slash_amount)
    }

    // ── Slash cooldown (issue #816) ────────────────────────────────────────────

    /// Admin: configure the cooldown window (in ledgers) between slash events.
    /// Set to 0 to disable the cooldown entirely.
    pub fn set_slash_cooldown(env: Env, ledgers: u32) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::SlashCooldownLedgers, &ledgers);
        events::emit_slash_cooldown_updated(&env, ledgers);
    }

    /// Returns the configured slash cooldown window in ledgers (default: 100).
    pub fn get_slash_cooldown(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&StorageKey::SlashCooldownLedgers)
            .unwrap_or(DEFAULT_SLASH_COOLDOWN_LEDGERS)
    }

    // ── Issue #689: Slash appeal window ────────────────────────────────────────

    /// Set the window (in seconds) after a slash during which the
    /// affected provider may submit an evidence appeal.
    /// Set to 0 to disable appeals entirely.
    ///
    /// When multi-sig is configured use `execute_action` with action
    /// [`action::SET_APPEAL_WINDOW`] instead.
    pub fn set_appeal_window(env: Env, window_secs: u64) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        // Sanity cap: max 30 days.
        if window_secs > 2_592_000 {
            return Err(StakeVaultError::InvalidAppealWindow);
        }
        env.storage()
            .instance()
            .set(&StorageKey::AppealWindowSecs, &window_secs);
        events::emit_appeal_window_updated(&env, window_secs);
        Ok(())
    }

    /// Returns the configured appeal window in seconds (defaults to 0 — disabled).
    pub fn get_appeal_window(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&StorageKey::AppealWindowSecs)
            .unwrap_or(0)
    }

    /// Provider: submit an appeal for a past slash, attaching off-chain evidence.
    ///
    /// Requirements:
    /// - `slash_id` must refer to an existing [`SlashRecord`].
    /// - The caller must be the slashed provider.
    /// - The appeal must be submitted within the admin-configured window.
    /// - Only one appeal per slash is allowed.
    ///
    /// While a `Pending` appeal exists the slash is considered "disputed":
    /// an admin must resolve it before final fund disposition is determined.
    pub fn appeal_slash(
        env: Env,
        appellant: Address,
        slash_id: u64,
        evidence_uri: String,
    ) -> Result<(), StakeVaultError> {
        appellant.require_auth();

        // Load the slash record.
        let record: SlashRecord = env
            .storage()
            .persistent()
            .get(&StorageKey::SlashRecord(slash_id))
            .ok_or(StakeVaultError::SlashNotFound)?;

        // Only the slashed provider may appeal.
        if record.provider != appellant {
            return Err(StakeVaultError::Unauthorized);
        }

        // Enforce appeal window.
        let window_secs: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::AppealWindowSecs)
            .unwrap_or(0);
        let now = env.ledger().timestamp();
        if window_secs == 0 || now > record.slashed_at.saturating_add(window_secs) {
            return Err(StakeVaultError::AppealWindowClosed);
        }

        // Prevent duplicate appeals.
        if env
            .storage()
            .persistent()
            .has(&StorageKey::SlashAppeal(slash_id))
        {
            return Err(StakeVaultError::AppealAlreadyExists);
        }

        let appeal = SlashAppeal {
            slash_id,
            appellant: appellant.clone(),
            evidence_uri: evidence_uri.clone(),
            submitted_at: now,
            status: AppealStatus::Pending,
        };
        env.storage()
            .persistent()
            .set(&StorageKey::SlashAppeal(slash_id), &appeal);

        events::emit_slash_appealed(&env, appellant, slash_id, evidence_uri);

        Ok(())
    }

    /// Admin: resolve a pending slash appeal.
    ///
    /// - `uphold`: `true` → slash stands; held funds are burned.
    /// - `uphold`: `false` → slash is reversed; held funds are returned to the
    ///   provider's stake balance.
    ///
    /// Resolving an appeal that does not exist or has already been resolved
    /// returns an error.
    ///
    /// Resolution determines whether slashed funds are burned or returned to
    /// the provider, so — like every other privileged mutation in this
    /// contract — it must be routed through multi-sig approval
    /// (`propose_action` / `approve_action` / `execute_action` with
    /// [`action::RESOLVE_APPEAL`]) once a multi-sig config is set. See issue
    /// #818: this entrypoint previously stayed admin-only even after
    /// `set_multisig_config` was configured, letting a single admin key
    /// bypass the approval flow that every sibling admin entrypoint enforces.
    pub fn resolve_appeal(env: Env, slash_id: u64, uphold: bool) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        Self::do_resolve_appeal(&env, slash_id, uphold)
    }

    /// Resolve multiple pending slash appeals in a single call (issue #815).
    ///
    /// `slash_ids` and `upholds` must have matching length, bounded by
    /// `MAX_BATCH_SIZE`. Authorization is checked once for the admin rather
    /// than per item.
    ///
    /// Entries that don't exist or are already resolved are skipped rather
    /// than aborting the whole batch, so one bad entry cannot block settling
    /// the rest. Returns the number of appeals successfully resolved.
    pub fn batch_resolve_appeal(
        env: Env,
        slash_ids: Vec<u64>,
        upholds: Vec<bool>,
    ) -> Result<u32, StakeVaultError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();

        let len = slash_ids.len();
        if len == 0 || len > MAX_BATCH_SIZE {
            return Err(StakeVaultError::BatchSizeInvalid);
        }
        if len != upholds.len() {
            return Err(StakeVaultError::BatchLengthMismatch);
        }

        let mut processed_count: u32 = 0;
        for i in 0..len {
            let slash_id = slash_ids.get(i).unwrap();
            let uphold = upholds.get(i).unwrap();
            if Self::do_resolve_appeal(&env, slash_id, uphold).is_ok() {
                processed_count = processed_count.saturating_add(1);
            }
        }

        events::emit_batch_appeals_resolved(&env, processed_count);
        Ok(processed_count)
    }

    /// Returns the [`SlashRecord`] for the given `slash_id`, or `None`.
    pub fn get_slash_record(env: Env, slash_id: u64) -> Option<SlashRecord> {
        env.storage()
            .persistent()
            .get(&StorageKey::SlashRecord(slash_id))
    }

    /// Returns the [`SlashAppeal`] for the given `slash_id`, or `None`.
    pub fn get_slash_appeal(env: Env, slash_id: u64) -> Option<SlashAppeal> {
        env.storage()
            .persistent()
            .get(&StorageKey::SlashAppeal(slash_id))
    }

    // ── Read ───────────────────────────────────────────────────────────────────

    pub fn get_stake(env: Env, staker: Address) -> i128 {
        let stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));
        stakes.get(staker).map(|s| s.balance).unwrap_or(0)
    }

    // ── Issue #754: Multi-sig emergency early-unstake ─────────────────────────

    /// Admin: configure the multi-sig parameters for emergency early unstakes.
    ///
    /// - `admins`: list of addresses that may approve emergency requests.
    /// - `required`: how many approvals are needed (N-of-M).
    /// - `penalty_bps`: basis points deducted from the withdrawn stake (0–10_000).
    /// - `timeout_secs`: seconds before an unapproved request expires (0 = no timeout).
    pub fn configure_emergency_multisig(
        env: Env,
        caller: Address,
        admins: Vec<Address>,
        required: u32,
        penalty_bps: u32,
        timeout_secs: u64,
    ) -> Result<(), StakeVaultError> {
        caller.require_auth();
        emergency_unstake::configure(&env, &caller, admins, required, penalty_bps, timeout_secs)
    }

    /// Returns the current emergency multi-sig configuration, if set.
    pub fn get_emergency_multisig_config(env: Env) -> Option<EmergencyMultiSigConfig> {
        emergency_unstake::get_config_pub(&env)
    }

    /// Staker: submit an emergency early-unstake request.
    ///
    /// The request will only execute once `required` multi-sig admins have called
    /// `approve_emergency_unstake`. Until then, no funds move.
    pub fn request_emergency_unstake(env: Env, staker: Address) -> Result<(), StakeVaultError> {
        staker.require_auth();
        emergency_unstake::request(&env, &staker)
    }

    /// Admin signer: approve a pending emergency unstake request.
    ///
    /// When the N-of-M threshold is reached the unstake executes immediately,
    /// applying the configured penalty to the withdrawn amount.
    pub fn approve_emergency_unstake(
        env: Env,
        signer: Address,
        staker: Address,
    ) -> Result<(), StakeVaultError> {
        signer.require_auth();
        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;
        emergency_unstake::approve(&env, &signer, &staker, &token)
    }

    /// Anyone may call this to clean up an expired emergency request.
    pub fn expire_emergency_request(env: Env, staker: Address) -> Result<(), StakeVaultError> {
        emergency_unstake::expire_request(&env, &staker)
    }

    /// Returns the pending emergency request for `staker`, if any.
    pub fn get_emergency_request(env: Env, staker: Address) -> Option<EmergencyRequest> {
        emergency_unstake::get_emergency_request(&env, &staker)
    }

    // ── Stake delegation (issue #688) ──────────────────────────────────────────

    /// Stake `amount` tokens on behalf of `provider`. The delegator's funds are
    /// transferred into the vault but credited to `provider`'s delegated stake.
    /// Delegated stake is tracked separately from the provider's own stake for
    /// accounting purposes, but counts toward the provider's total stake for
    /// signal submission and tier checks.
    pub fn delegate_stake(
        env: Env,
        delegator: Address,
        provider: Address,
        amount: i128,
    ) -> Result<(), StakeVaultError> {
        delegator.require_auth();
        Self::require_not_paused(&env)?;

        if amount <= 0 {
            return Err(StakeVaultError::NoStake);
        }

        let token: Address = env
            .storage()
            .instance()
            .get(&StorageKey::StakeToken)
            .ok_or(StakeVaultError::NotInitialized)?;

        let mut delegated: soroban_sdk::Map<Address, i128> = env
            .storage()
            .persistent()
            .get(&StorageKey::ProviderDelegatedStakes(provider.clone()))
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));

        let current = delegated.get(delegator.clone()).unwrap_or(0);
        let new_amount = current.checked_add(amount).unwrap_or(i128::MAX);
        delegated.set(delegator.clone(), new_amount);
        env.storage().persistent().set(
            &StorageKey::ProviderDelegatedStakes(provider.clone()),
            &delegated,
        );

        let total_delegated: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::ProviderTotalDelegated(provider.clone()))
            .unwrap_or(0);
        let new_total = total_delegated.checked_add(amount).unwrap_or(i128::MAX);
        env.storage().persistent().set(
            &StorageKey::ProviderTotalDelegated(provider.clone()),
            &new_total,
        );

        token::Client::new(&env, &token).transfer(
            &delegator,
            env.current_contract_address(),
            &amount,
        );

        events::emit_stake_delegated(&env, delegator, provider, amount);

        Ok(())
    }

    /// Returns the amount delegated by `delegator` to `provider`.
    pub fn get_delegated_stake(env: Env, delegator: Address, provider: Address) -> i128 {
        let delegated: soroban_sdk::Map<Address, i128> = env
            .storage()
            .persistent()
            .get(&StorageKey::ProviderDelegatedStakes(provider))
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));
        delegated.get(delegator).unwrap_or(0)
    }

    /// Returns the total amount delegated to `provider` across all delegators.
    pub fn get_total_delegated(env: Env, provider: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&StorageKey::ProviderTotalDelegated(provider))
            .unwrap_or(0)
    }

    /// Returns `provider`'s effective total stake (own + all delegated).
    pub fn get_total_stake_for_provider(env: Env, provider: Address) -> i128 {
        let own = Self::get_stake(env.clone(), provider.clone());
        let delegated = Self::get_total_delegated(env, provider);
        own.saturating_add(delegated)
    }

    // ── Issue #663: Unstake request queue ─────────────────────────────────────

    /// Queue an unstake request for `staker`.
    ///
    /// The request is assigned a sequential ticket number and placed at the tail
    /// of the FIFO queue. The staker must have a positive stake balance and must
    /// not already have a pending request in the queue.
    ///
    /// Emits `unstake_queued` with the assigned ticket number and queue position.
    pub fn queue_unstake(env: Env, staker: Address) -> Result<u64, StakeVaultError> {
        staker.require_auth();
        Self::require_not_paused(&env)?;

        if env
            .storage()
            .persistent()
            .has(&StorageKey::UserUnstakeTicket(staker.clone()))
        {
            return Err(StakeVaultError::UnstakeAlreadyQueued);
        }

        let stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
            .storage()
            .persistent()
            .get(&MigrationKey::StakesV2)
            .unwrap_or_else(|| soroban_sdk::Map::new(&env));
        let info = stakes.get(staker.clone()).ok_or(StakeVaultError::NoStake)?;
        if info.balance == 0 {
            return Err(StakeVaultError::NoStake);
        }

        let tail: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::UnstakeQueueTail)
            .unwrap_or(0);
        let head: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::UnstakeQueueHead)
            .unwrap_or(0);

        let max_queue_size: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::MaxUnstakeQueueSize)
            .unwrap_or(DEFAULT_MAX_UNSTAKE_QUEUE_SIZE);
        if tail.saturating_sub(head) >= max_queue_size as u64 {
            return Err(StakeVaultError::QueueFull);
        }

        let ticket = tail;
        let queue_position = tail.saturating_sub(head);

        let request = UnstakeRequest {
            ticket,
            user: staker.clone(),
            queued_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&StorageKey::UnstakeQueueEntry(ticket), &request);
        env.storage()
            .persistent()
            .set(&StorageKey::UserUnstakeTicket(staker.clone()), &ticket);
        env.storage()
            .instance()
            .set(&StorageKey::UnstakeQueueTail, &tail.saturating_add(1));

        events::emit_unstake_queued(&env, staker, ticket, queue_position);

        Ok(ticket)
    }

    /// Sets the maximum number of pending entries allowed in the unstake queue.
    ///
    /// Admin-only. Lowering the cap below the current queue length does not
    /// affect already-queued entries; it only blocks new `queue_unstake` calls
    /// until the queue drains back under the new cap. Emits `queue_size_updated`.
    pub fn set_max_unstake_queue_size(env: Env, size: u32) -> Result<(), StakeVaultError> {
        if multisig::get_config(&env).is_some() {
            return Err(StakeVaultError::RequiresMultisig);
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(StakeVaultError::NotInitialized)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::MaxUnstakeQueueSize, &size);
        #[allow(deprecated)]
        env.events().publish(
            (
                Symbol::new(&env, "stake_vault"),
                Symbol::new(&env, "queue_size_updated"),
            ),
            (size,),
        );
        Ok(())
    }

    /// Returns the configured maximum unstake queue length (defaults to 200).
    pub fn get_max_unstake_queue_size(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&StorageKey::MaxUnstakeQueueSize)
            .unwrap_or(DEFAULT_MAX_UNSTAKE_QUEUE_SIZE)
    }

    /// Process up to `limit` queued unstake requests in FIFO order.
    ///
    /// Requests are processed from the queue head. A request that fails (e.g.
    /// due to a time-lock or locked stake) is left at the head; processing stops
    /// so strict FIFO ordering is preserved. The caller should retry later once
    /// the blocking condition is resolved.
    ///
    /// Recommended invocation frequency: call at least as often as the queue
    /// approaches `MaxUnstakeQueueSize` (e.g. keyed off `unstake_queued` events
    /// or periodic polling of `get_queue_position`), since `queue_unstake` starts
    /// rejecting new entries once the queue is at or above the configured cap.
    ///
    /// Returns the number of requests successfully processed.
    pub fn process_unstake_queue(env: Env, limit: u32) -> Result<u32, StakeVaultError> {
        Self::require_not_paused(&env)?;

        if limit == 0 {
            return Ok(0);
        }

        let head: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::UnstakeQueueHead)
            .unwrap_or(0);
        let tail: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::UnstakeQueueTail)
            .unwrap_or(0);

        if head >= tail {
            return Err(StakeVaultError::QueueEmpty);
        }

        let mut processed: u32 = 0;
        let mut current_head = head;

        while processed < limit && current_head < tail {
            let entry_key = StorageKey::UnstakeQueueEntry(current_head);
            let request: UnstakeRequest = match env.storage().persistent().get(&entry_key) {
                Some(r) => r,
                None => {
                    // Orphaned slot — advance head and continue.
                    current_head = current_head.saturating_add(1);
                    continue;
                }
            };

            match Self::do_withdraw(&env, &request.user) {
                Ok(amount) => {
                    env.storage().persistent().remove(&entry_key);
                    env.storage()
                        .persistent()
                        .remove(&StorageKey::UserUnstakeTicket(request.user.clone()));
                    current_head = current_head.saturating_add(1);
                    processed = processed.saturating_add(1);

                    stellar_swipe_common::rate_limit::record_action(
                        &env,
                        &request.user,
                        stellar_swipe_common::rate_limit::ActionType::StakeChange,
                    );

                    events::emit_unstake_processed(&env, request.user, request.ticket, amount);
                }
                Err(_) => {
                    // Blocked — stop processing to preserve FIFO order.
                    break;
                }
            }
        }

        env.storage()
            .instance()
            .set(&StorageKey::UnstakeQueueHead, &current_head);

        Ok(processed)
    }

    /// Returns the zero-based queue position for `user`, or `None` if the user
    /// has no pending unstake request.
    ///
    /// Position 0 means the user is next to be processed.
    pub fn get_queue_position(env: Env, user: Address) -> Option<u64> {
        let ticket: u64 = env
            .storage()
            .persistent()
            .get(&StorageKey::UserUnstakeTicket(user))?;
        let head: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::UnstakeQueueHead)
            .unwrap_or(0);
        Some(ticket.saturating_sub(head))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_emergency;
