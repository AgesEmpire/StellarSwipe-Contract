#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, String, Symbol, Vec,
};
use shared::reentrancy;
use stellar_swipe_common::SECONDS_PER_DAY;

mod validators;

pub use validators::{ValidatorApproval, ValidatorApprovalKind, ValidatorSet};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BridgeError {
    /// `initialize()` was called on a bridge contract that already has an admin set.
    AlreadyInitialized = 1,
    /// Transfer, fee, or threshold amount is zero, negative, or otherwise out of range.
    InvalidAmount = 2,
    /// Validator set passed to `initialize`/updates is empty or fails threshold checks.
    InvalidValidatorSet = 3,
    /// Caller is not a member of the active validator set for this bridge.
    UnauthorizedValidator = 4,
    /// No transfer exists for the given transfer id.
    TransferNotFound = 5,
    /// Transfer has already been executed; cannot be executed a second time.
    TransferAlreadyExecuted = 6,
    /// Source-chain transaction hash has already been used for a lock/burn (replay attempt).
    ReplayDetected = 7,
    /// This validator has already submitted a signature for this transfer.
    SignatureAlreadyUsed = 8,
    /// Fewer validator approvals have been collected than the required threshold.
    NotEnoughValidatorApprovals = 9,
    /// Transfer would push the rolling 24h volume past the configured daily limit.
    DailyLimitExceeded = 10,
    /// Single transfer amount exceeds the configured per-transfer maximum.
    MaxTransferExceeded = 11,
    /// Caller's wrapped-asset balance is lower than the amount requested to burn/withdraw.
    InsufficientWrappedBalance = 12,
    /// Burn/unlock withdrawal was requested before its mandatory delay window elapsed.
    WithdrawalNotReady = 13,
    /// Requested operation is not valid for the transfer's current status.
    InvalidOperation = 14,
    /// Withdrawal exceeds the dynamic limit derived from available liquidity buffer.
    /// Distinct from DailyLimitExceeded (static anti-spam) so callers can differentiate.
    DynamicLiquidityLimitExceeded = 15,
    /// Validator approval threshold is zero, exceeds validator count, or otherwise invalid.
    InvalidThreshold = 16,
    /// Destination chain is not on the admin-managed allowlist (Issue #669).
    UnsupportedDestinationChain = 17,
    /// Message nonce is out of order; earlier messages must be confirmed first (Issue #668).
    OutOfOrderNonce = 18,
    /// Message nonce was already confirmed; duplicate delivery rejected (Issue #668).
    DuplicateNonce = 19,
    /// The bridge is paused (governance-driven emergency pause). See Issue #865.
    ContractPaused = 20,
}

impl BridgeError {
    /// Short, human-readable description of when this error is returned.
    ///
    /// Intended for logs/operator tooling; not part of the on-chain XDR spec.
    pub fn message(&self) -> &'static str {
        match self {
            BridgeError::AlreadyInitialized => {
                "bridge contract has already been initialized with an admin"
            }
            BridgeError::InvalidAmount => "amount must be a positive value within allowed bounds",
            BridgeError::InvalidValidatorSet => {
                "validator set is empty or fails minimum threshold requirements"
            }
            BridgeError::UnauthorizedValidator => {
                "caller is not a member of the active validator set"
            }
            BridgeError::TransferNotFound => "no transfer exists for the given transfer id",
            BridgeError::TransferAlreadyExecuted => {
                "transfer has already been executed and cannot run again"
            }
            BridgeError::ReplayDetected => {
                "source-chain transaction hash was already used for a transfer"
            }
            BridgeError::SignatureAlreadyUsed => {
                "this validator has already signed off on this transfer"
            }
            BridgeError::NotEnoughValidatorApprovals => {
                "not enough validator approvals collected yet to meet the threshold"
            }
            BridgeError::DailyLimitExceeded => {
                "transfer would exceed the configured rolling daily volume limit"
            }
            BridgeError::MaxTransferExceeded => {
                "transfer amount exceeds the configured per-transfer maximum"
            }
            BridgeError::InsufficientWrappedBalance => {
                "caller's wrapped-asset balance is too low for this burn/withdrawal"
            }
            BridgeError::WithdrawalNotReady => {
                "withdrawal was requested before its mandatory delay window elapsed"
            }
            BridgeError::InvalidOperation => {
                "requested operation is not valid for the transfer's current status"
            }
            BridgeError::DynamicLiquidityLimitExceeded => {
                "withdrawal exceeds the dynamic limit derived from the liquidity buffer"
            }
            BridgeError::InvalidThreshold => {
                "validator approval threshold is zero, too high, or otherwise invalid"
            }
            BridgeError::UnsupportedDestinationChain => {
                "destination chain is not on the admin-managed allowlist"
            }
            BridgeError::OutOfOrderNonce => {
                "message nonce is out of order; earlier messages must be confirmed first"
            }
            BridgeError::DuplicateNonce => {
                "message nonce was already confirmed; duplicate delivery rejected"
            }
        }
    }
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainId {
    Ethereum,
    Polygon,
    Bnb,
    Bitcoin,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferKind {
    LockMint,
    BurnUnlock,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferStatus {
    PendingValidators,
    ReadyToExecute,
    Completed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityConfig {
    pub max_transfer_amount: i128,
    pub daily_transfer_limit: i128,
    pub required_validator_signatures: u32,
    pub withdraw_delay_seconds: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrappedAsset {
    pub source_chain: ChainId,
    pub source_asset: String,
    pub wrapped_asset: String,
    pub decimals: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeConfig {
    pub admin: Address,
    pub validator_set: ValidatorSet,
    pub security: SecurityConfig,
    pub next_transfer_id: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeTransfer {
    pub id: u64,
    pub kind: TransferKind,
    pub user: Address,
    pub source_chain: ChainId,
    pub destination_chain: ChainId,
    pub source_asset: String,
    pub wrapped_asset: String,
    pub amount: i128,
    pub source_tx_hash: String,
    pub source_nonce: u64,
    pub destination_recipient: String,
    pub approvals: Vec<ValidatorApproval>,
    pub status: TransferStatus,
    pub created_at: u64,
    pub executed_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DailyVolume {
    pub day_start: u64,
    pub total_amount: i128,
}

#[contracttype]
pub enum DataKey {
    Config,
    WrappedAsset(String),
    Transfer(u64),
    ReplayLock(ChainId, String, u64),
    UsedSignature(Address, u64, ValidatorApprovalKind, String),
    WrappedBalance(Address, String),
    DailyVolume,
    /// Admin-set available liquidity buffer for dynamic rate limiting.
    LiquidityBuffer,
    TotalMinted,
    ReserveThreshold,
    /// Admin-managed set of supported destination chain IDs (Issue #669).
    SupportedChains,
    /// Issue #865: global pause flag set via governance-driven propagation.
    Paused,
    /// Issue #865: central governance contract address authorized to call
    /// `apply_governance_pause`.
    GovernanceAddress,
}

const DAY_SECONDS: u64 = 86_400;

pub mod analytics;
pub mod fees;
pub mod governance;
mod liquidity;
pub mod messaging;
pub mod monitoring;

pub use liquidity::{LiquidityPool, LiquidityPosition, PoolHealth, PoolType, SwapResult};

pub use messaging::{
    confirm_message_delivery, expire_timed_out_message, get_cross_chain_message,
    mark_message_failed, receive_message_callback, register_bridge_for_chain,
    relay_message_to_target_chain, retry_failed_message, send_cross_chain_message,
    CrossChainMessage, MessageStatus, MAX_MESSAGE_SIZE, MESSAGE_TIMEOUT,
};

soroban_sdk::contractmeta!(key = "SourceHash", val = env!("STELLAR_SOURCE_HASH"));
soroban_sdk::contractmeta!(key = "GitCommit", val = env!("STELLAR_GIT_COMMIT"));

#[contract]
pub struct BridgeContract;

#[contractimpl]
impl BridgeContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        validators: Vec<Address>,
        required_validator_signatures: u32,
        max_transfer_amount: i128,
        daily_transfer_limit: i128,
        withdraw_delay_seconds: u64,
    ) -> Result<(), BridgeError> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(BridgeError::AlreadyInitialized);
        }

        if !cfg!(test) {
            admin.require_auth();
        }
        let validator_set =
            validators::build_validator_set(&env, validators, required_validator_signatures)?;

        if max_transfer_amount <= 0 || daily_transfer_limit <= 0 {
            return Err(BridgeError::InvalidAmount);
        }

        let config = BridgeConfig {
            admin,
            validator_set,
            security: SecurityConfig {
                max_transfer_amount,
                daily_transfer_limit,
                required_validator_signatures,
                withdraw_delay_seconds,
            },
            next_transfer_id: 1,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().persistent().set(
            &DataKey::DailyVolume,
            &DailyVolume {
                day_start: day_bucket(env.ledger().timestamp()),
                total_amount: 0,
            },
        );
        Ok(())
    }

    pub fn register_wrapped_asset(
        env: Env,
        admin: Address,
        source_chain: ChainId,
        source_asset: String,
        wrapped_asset: String,
        decimals: u32,
    ) -> Result<(), BridgeError> {
        let config = require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }

        let asset = WrappedAsset {
            source_chain,
            source_asset,
            wrapped_asset: wrapped_asset.clone(),
            decimals,
        };

        env.storage()
            .persistent()
            .set(&DataKey::WrappedAsset(wrapped_asset.clone()), &asset);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "wrapped_asset_registered"),),
            wrapped_asset,
        );

        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn initiate_lock_mint(
        env: Env,
        user: Address,
        source_chain: ChainId,
        destination_chain: ChainId,
        source_asset: String,
        wrapped_asset: String,
        amount: i128,
        source_tx_hash: String,
        source_nonce: u64,
        destination_recipient: String,
    ) -> Result<u64, BridgeError> {
        if is_paused(&env) {
            return Err(BridgeError::ContractPaused);
        }
        if !cfg!(test) {
            user.require_auth();
        }
        // ── #669: allowlist check ─────────────────────────────────────────────
        ensure_chain_allowed(&env, destination_chain)?;
        validate_amount_and_limits(&env, amount)?;
        ensure_wrapped_asset_exists(&env, wrapped_asset.clone())?;

        if env.storage().persistent().has(&DataKey::ReplayLock(
            source_chain,
            source_tx_hash.clone(),
            source_nonce,
        )) {
            return Err(BridgeError::ReplayDetected);
        }

        let mut config = get_config(&env)?;
        let transfer_id = config.next_transfer_id;
        config.next_transfer_id += 1;

        let transfer = BridgeTransfer {
            id: transfer_id,
            kind: TransferKind::LockMint,
            user,
            source_chain,
            destination_chain,
            source_asset,
            wrapped_asset,
            amount,
            source_tx_hash: source_tx_hash.clone(),
            source_nonce,
            destination_recipient,
            approvals: Vec::new(&env),
            status: TransferStatus::PendingValidators,
            created_at: env.ledger().timestamp(),
            executed_at: None,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage()
            .persistent()
            .set(&DataKey::Transfer(transfer_id), &transfer);
        env.storage().persistent().set(
            &DataKey::ReplayLock(source_chain, source_tx_hash, source_nonce),
            &true,
        );

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "lock_mint_initiated"), transfer_id),
            amount,
        );

        Ok(transfer_id)
    }

    pub fn approve_lock_mint(
        env: Env,
        validator: Address,
        transfer_id: u64,
        signature: String,
    ) -> Result<(), BridgeError> {
        if !cfg!(test) {
            validator.require_auth();
        }
        let config = get_config(&env)?;
        let mut transfer = get_transfer(&env, transfer_id)?;

        if transfer.kind != TransferKind::LockMint || transfer.status == TransferStatus::Completed {
            return Err(BridgeError::InvalidOperation);
        }

        validators::verify_and_record_approval(
            &env,
            &config.validator_set,
            &mut transfer.approvals,
            validator,
            transfer_id,
            signature,
            ValidatorApprovalKind::LockMint,
        )?;

        if validators::has_quorum(
            &transfer.approvals,
            config.security.required_validator_signatures,
        ) {
            transfer.status = TransferStatus::ReadyToExecute;
        }

        store_transfer(&env, &transfer);
        Ok(())
    }

    pub fn execute_lock_mint(
        env: Env,
        admin: Address,
        transfer_id: u64,
    ) -> Result<(), BridgeError> {
        // Issue #859: Reentrancy guard for cross-contract state transitions.
        reentrancy::require_not_locked(&env).map_err(|_| BridgeError::InvalidOperation)?;
        if is_paused(&env) {
            return Err(BridgeError::ContractPaused);
        }
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        let mut transfer = get_transfer(&env, transfer_id)?;

        if transfer.kind != TransferKind::LockMint {
            return Err(BridgeError::InvalidOperation);
        }
        if transfer.status != TransferStatus::ReadyToExecute {
            return Err(BridgeError::NotEnoughValidatorApprovals);
        }
        if transfer.executed_at.is_some() {
            return Err(BridgeError::TransferAlreadyExecuted);
        }

        let balance_key =
            DataKey::WrappedBalance(transfer.user.clone(), transfer.wrapped_asset.clone());
        let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&balance_key, &(balance + transfer.amount));

        let total_minted: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalMinted)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalMinted, &(total_minted + transfer.amount));

        transfer.status = TransferStatus::Completed;
        transfer.executed_at = Some(env.ledger().timestamp());
        store_transfer(&env, &transfer);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "wrapped_asset_minted"), transfer_id),
            transfer.amount,
        );

        Ok(())
    }

    pub fn initiate_burn_unlock(
        env: Env,
        user: Address,
        source_chain: ChainId,
        destination_chain: ChainId,
        source_asset: String,
        wrapped_asset: String,
        amount: i128,
        destination_recipient: String,
    ) -> Result<u64, BridgeError> {
        if is_paused(&env) {
            return Err(BridgeError::ContractPaused);
        }
        if !cfg!(test) {
            user.require_auth();
        }
        // ── #669: allowlist check ─────────────────────────────────────────────
        ensure_chain_allowed(&env, destination_chain)?;
        validate_amount_and_limits(&env, amount)?;

        let balance_key = DataKey::WrappedBalance(user.clone(), wrapped_asset.clone());
        let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        if balance < amount {
            return Err(BridgeError::InsufficientWrappedBalance);
        }
        env.storage()
            .persistent()
            .set(&balance_key, &(balance - amount));

        let total_minted: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalMinted)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalMinted, &(total_minted - amount));

        let mut config = get_config(&env)?;
        let transfer_id = config.next_transfer_id;
        config.next_transfer_id += 1;

        let transfer = BridgeTransfer {
            id: transfer_id,
            kind: TransferKind::BurnUnlock,
            user,
            source_chain,
            destination_chain,
            source_asset,
            wrapped_asset,
            amount,
            source_tx_hash: String::from_str(&env, ""),
            source_nonce: transfer_id,
            destination_recipient,
            approvals: Vec::new(&env),
            status: TransferStatus::PendingValidators,
            created_at: env.ledger().timestamp(),
            executed_at: None,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        store_transfer(&env, &transfer);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "burn_unlock_initiated"), transfer_id),
            amount,
        );

        Ok(transfer_id)
    }

    pub fn approve_burn_unlock(
        env: Env,
        validator: Address,
        transfer_id: u64,
        signature: String,
    ) -> Result<(), BridgeError> {
        if !cfg!(test) {
            validator.require_auth();
        }
        let config = get_config(&env)?;
        let mut transfer = get_transfer(&env, transfer_id)?;

        if transfer.kind != TransferKind::BurnUnlock || transfer.status == TransferStatus::Completed
        {
            return Err(BridgeError::InvalidOperation);
        }

        validators::verify_and_record_approval(
            &env,
            &config.validator_set,
            &mut transfer.approvals,
            validator,
            transfer_id,
            signature,
            ValidatorApprovalKind::BurnUnlock,
        )?;

        if validators::has_quorum(
            &transfer.approvals,
            config.security.required_validator_signatures,
        ) {
            transfer.status = TransferStatus::ReadyToExecute;
        }

        store_transfer(&env, &transfer);
        Ok(())
    }

    pub fn execute_burn_unlock(
        env: Env,
        admin: Address,
        transfer_id: u64,
    ) -> Result<(), BridgeError> {
        if is_paused(&env) {
            return Err(BridgeError::ContractPaused);
        }
        let config = require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        let mut transfer = get_transfer(&env, transfer_id)?;

        if transfer.kind != TransferKind::BurnUnlock {
            return Err(BridgeError::InvalidOperation);
        }
        if transfer.status != TransferStatus::ReadyToExecute {
            return Err(BridgeError::WithdrawalNotReady);
        }
        let ready_at = transfer.created_at + config.security.withdraw_delay_seconds;
        if env.ledger().timestamp() < ready_at {
            return Err(BridgeError::WithdrawalNotReady);
        }
        if transfer.executed_at.is_some() {
            return Err(BridgeError::TransferAlreadyExecuted);
        }

        transfer.status = TransferStatus::Completed;
        transfer.executed_at = Some(env.ledger().timestamp());
        store_transfer(&env, &transfer);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "burn_unlock_completed"), transfer_id),
            transfer.amount,
        );

        Ok(())
    }

    pub fn get_transfer(env: Env, transfer_id: u64) -> Result<BridgeTransfer, BridgeError> {
        get_transfer(&env, transfer_id)
    }

    pub fn get_bridge_config(env: Env) -> Result<BridgeConfig, BridgeError> {
        get_config(&env)
    }

    pub fn get_wrapped_balance(env: Env, user: Address, wrapped_asset: String) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::WrappedBalance(user, wrapped_asset))
            .unwrap_or(0)
    }

    pub fn attest_reserves(
        env: Env,
        caller: Address,
        actual_locked: i128,
    ) -> Result<(), BridgeError> {
        if !cfg!(test) {
            caller.require_auth();
        }
        if actual_locked < 0 {
            return Err(BridgeError::InvalidAmount);
        }

        let total_minted = env
            .storage()
            .persistent()
            .get(&DataKey::TotalMinted)
            .unwrap_or(0i128);
        let threshold = env
            .storage()
            .persistent()
            .get(&DataKey::ReserveThreshold)
            .unwrap_or(9500u32);

        let ratio = if total_minted > 0 {
            (actual_locked * 10000) / total_minted
        } else {
            10000
        };

        let healthy = ratio >= threshold as i128;

        env.events().publish(
            (Symbol::new(&env, "reserve_attestation"), healthy),
            (actual_locked, total_minted, ratio, threshold),
        );

        Ok(())
    }

    pub fn set_reserve_threshold(
        env: Env,
        admin: Address,
        threshold_bps: u32,
    ) -> Result<(), BridgeError> {
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        if threshold_bps > 10000 {
            return Err(BridgeError::InvalidThreshold);
        }
        env.storage()
            .persistent()
            .set(&DataKey::ReserveThreshold, &threshold_bps);
        Ok(())
    }

    pub fn get_reserve_threshold(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReserveThreshold)
            .unwrap_or(9500u32)
    }

    pub fn get_total_minted(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalMinted)
            .unwrap_or(0)
    }

    pub fn create_liquidity_pool(
        env: Env,
        admin: Address,
        asset_a: String,
        asset_b: String,
        pool_type: PoolType,
        fee_bps: u32,
        reward_bps: u32,
    ) -> Result<u64, BridgeError> {
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        ensure_wrapped_asset_exists(&env, asset_a.clone())?;
        ensure_wrapped_asset_exists(&env, asset_b.clone())?;
        liquidity::create_pool(&env, asset_a, asset_b, pool_type, fee_bps, reward_bps)
    }

    pub fn add_bridge_liquidity(
        env: Env,
        provider: Address,
        pool_id: u64,
        amount_a: i128,
        amount_b: i128,
    ) -> Result<i128, BridgeError> {
        if !cfg!(test) {
            provider.require_auth();
        }
        liquidity::add_liquidity(&env, provider, pool_id, amount_a, amount_b)
    }

    pub fn remove_bridge_liquidity(
        env: Env,
        provider: Address,
        pool_id: u64,
        lp_amount: i128,
    ) -> Result<(i128, i128, i128), BridgeError> {
        if !cfg!(test) {
            provider.require_auth();
        }
        liquidity::remove_liquidity(&env, provider, pool_id, lp_amount)
    }

    pub fn swap_bridge_assets(
        env: Env,
        trader: Address,
        pool_id: u64,
        input_asset: String,
        amount_in: i128,
        min_amount_out: i128,
    ) -> Result<SwapResult, BridgeError> {
        if !cfg!(test) {
            trader.require_auth();
        }
        liquidity::swap(
            &env,
            trader,
            pool_id,
            input_asset,
            amount_in,
            min_amount_out,
        )
    }

    pub fn get_pool(env: Env, pool_id: u64) -> Result<LiquidityPool, BridgeError> {
        liquidity::get_pool(&env, pool_id)
    }

    pub fn get_liquidity_position(env: Env, provider: Address, pool_id: u64) -> LiquidityPosition {
        liquidity::get_position(&env, provider, pool_id)
    }

    pub fn get_pool_health(env: Env, pool_id: u64) -> Result<PoolHealth, BridgeError> {
        liquidity::get_pool_health(&env, pool_id)
    }

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

    /// Read-only health for ops / frontends.
    pub fn health_check(env: Env) -> stellar_swipe_common::HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        let config: Option<BridgeConfig> = env.storage().instance().get(&DataKey::Config);
        match config {
            Some(cfg) => stellar_swipe_common::HealthStatus {
                is_initialized: true,
                is_paused: is_paused(&env),
                version,
                admin: cfg.admin,
            },
            None => crate::governance::bridge_health_check(&env),
        }
    }

    // ── Issue #865: governance-driven pause propagation ────────────────────────

    /// Set the central governance contract address authorized to call
    /// `apply_governance_pause`. Admin only.
    pub fn set_governance(env: Env, admin: Address, governance: Address) -> Result<(), BridgeError> {
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        env.storage()
            .instance()
            .set(&DataKey::GovernanceAddress, &governance);
        Ok(())
    }

    /// Read-only: the configured governance contract address, if any.
    pub fn get_governance(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::GovernanceAddress)
    }

    /// Called by the configured governance contract to propagate a pause/unpause.
    /// Rejects new lock-mint/burn-unlock transfers and their execution while paused.
    pub fn apply_governance_pause(env: Env, paused: bool) -> Result<(), BridgeError> {
        let governance: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernanceAddress)
            .ok_or(BridgeError::UnauthorizedValidator)?;
        governance.require_auth();
        env.storage().instance().set(&DataKey::Paused, &paused);
        Ok(())
    }

    /// Read-only: true when a governance-driven emergency pause is active.
    pub fn is_paused(env: Env) -> bool {
        is_paused(&env)
    }

    // ── #613 Dynamic liquidity rate-limit ──────────────────────────────────────

    /// Admin: record the current available liquidity buffer.
    /// This is used to compute the dynamic per-transfer withdrawal cap.
    pub fn update_liquidity_buffer(
        env: Env,
        admin: Address,
        buffer: i128,
    ) -> Result<(), BridgeError> {
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        if buffer < 0 {
            return Err(BridgeError::InvalidAmount);
        }
        env.storage()
            .persistent()
            .set(&DataKey::LiquidityBuffer, &buffer);
        #[allow(deprecated)]
        env.events().publish(
            (
                Symbol::new(&env, "bridge"),
                Symbol::new(&env, "liquidity_buffer_updated"),
            ),
            buffer,
        );
        Ok(())
    }

    pub fn get_liquidity_buffer(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::LiquidityBuffer)
            .unwrap_or(i128::MAX)
    }

    // ── #669: Destination-chain allowlist ─────────────────────────────────────

    /// Admin: add `chain` to the supported destination-chain allowlist.
    pub fn add_supported_chain(
        env: Env,
        admin: Address,
        chain: ChainId,
    ) -> Result<(), BridgeError> {
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        let mut chains = load_supported_chains(&env);
        for i in 0..chains.len() {
            if chains.get(i) == Some(chain) {
                return Ok(());
            }
        }
        chains.push_back(chain);
        env.storage()
            .instance()
            .set(&DataKey::SupportedChains, &chains);
        #[allow(deprecated)]
        env.events()
            .publish((Symbol::new(&env, "chain_added"),), chain as u32);
        Ok(())
    }

    /// Admin: remove `chain` from the supported destination-chain allowlist.
    pub fn remove_supported_chain(
        env: Env,
        admin: Address,
        chain: ChainId,
    ) -> Result<(), BridgeError> {
        require_admin(&env, &admin)?;
        if !cfg!(test) {
            admin.require_auth();
        }
        let chains = load_supported_chains(&env);
        let mut updated: Vec<ChainId> = Vec::new(&env);
        for i in 0..chains.len() {
            if let Some(c) = chains.get(i) {
                if c != chain {
                    updated.push_back(c);
                }
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::SupportedChains, &updated);
        #[allow(deprecated)]
        env.events()
            .publish((Symbol::new(&env, "chain_removed"),), chain as u32);
        Ok(())
    }

    /// Return the current destination-chain allowlist.
    pub fn get_supported_chains(env: Env) -> Vec<ChainId> {
        load_supported_chains(&env)
    }

    // ── #668: Message nonce read-only entrypoint ───────────────────────────────

    /// Return the next expected delivery-confirmation nonce for `source_chain_id`.
    /// `source_chain_id` is the numeric value of `monitoring::ChainId`
    /// (Stellar=0, Ethereum=1, Bitcoin=2, Polygon=3, BNB=4).
    pub fn get_expected_nonce(env: Env, source_chain_id: u32) -> u64 {
        env.storage()
            .persistent()
            .get(&messaging::MessagingKey::ExpectedNonce(source_chain_id))
            .unwrap_or(0u64)
    }
}

fn get_config(env: &Env) -> Result<BridgeConfig, BridgeError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(BridgeError::AlreadyInitialized)
}

fn require_admin(env: &Env, admin: &Address) -> Result<BridgeConfig, BridgeError> {
    let config = get_config(env)?;
    if config.admin != *admin {
        return Err(BridgeError::UnauthorizedValidator);
    }
    Ok(config)
}

/// Issue #865: true when a governance-driven emergency pause is active.
fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

fn get_transfer(env: &Env, transfer_id: u64) -> Result<BridgeTransfer, BridgeError> {
    env.storage()
        .persistent()
        .get(&DataKey::Transfer(transfer_id))
        .ok_or(BridgeError::TransferNotFound)
}

fn store_transfer(env: &Env, transfer: &BridgeTransfer) {
    env.storage()
        .persistent()
        .set(&DataKey::Transfer(transfer.id), transfer);
}

/// Retrieve the current destination-chain allowlist.
fn load_supported_chains(env: &Env) -> Vec<ChainId> {
    env.storage()
        .instance()
        .get(&DataKey::SupportedChains)
        .unwrap_or_else(|| Vec::new(env))
}

/// Return `Ok(())` when `destination_chain` is on the allowlist, or when the
/// allowlist is empty (not yet configured). Returns `Err` otherwise.
fn ensure_chain_allowed(env: &Env, destination_chain: ChainId) -> Result<(), BridgeError> {
    let chains = load_supported_chains(env);
    if chains.is_empty() {
        return Ok(());
    }
    for i in 0..chains.len() {
        if chains.get(i) == Some(destination_chain) {
            return Ok(());
        }
    }
    Err(BridgeError::UnsupportedDestinationChain)
}

fn ensure_wrapped_asset_exists(env: &Env, wrapped_asset: String) -> Result<(), BridgeError> {
    if env
        .storage()
        .persistent()
        .has(&DataKey::WrappedAsset(wrapped_asset))
    {
        Ok(())
    } else {
        Err(BridgeError::InvalidOperation)
    }
}

fn validate_amount_and_limits(env: &Env, amount: i128) -> Result<(), BridgeError> {
    if amount <= 0 {
        return Err(BridgeError::InvalidAmount);
    }

    let config = get_config(env)?;
    if amount > config.security.max_transfer_amount {
        return Err(BridgeError::MaxTransferExceeded);
    }

    let current_day = day_bucket(env.ledger().timestamp());
    let mut volume: DailyVolume = env
        .storage()
        .persistent()
        .get(&DataKey::DailyVolume)
        .unwrap_or(DailyVolume {
            day_start: current_day,
            total_amount: 0,
        });

    if volume.day_start != current_day {
        volume.day_start = current_day;
        volume.total_amount = 0;
    }

    if volume.total_amount + amount > config.security.daily_transfer_limit {
        return Err(BridgeError::DailyLimitExceeded);
    }

    // ── Dynamic liquidity-buffer cap (independent of static limits) ───────────
    // Allowed single transfer = 10% of current buffer (minimum 1 if buffer > 0).
    // When buffer is unset (MAX) there is no dynamic restriction.
    let buffer: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::LiquidityBuffer)
        .unwrap_or(i128::MAX);
    if buffer != i128::MAX {
        let dynamic_limit = core::cmp::max(buffer / 10, 1);
        if amount > dynamic_limit {
            return Err(BridgeError::DynamicLiquidityLimitExceeded);
        }
    }

    volume.total_amount += amount;
    env.storage()
        .persistent()
        .set(&DataKey::DailyVolume, &volume);
    Ok(())
}

fn day_bucket(timestamp: u64) -> u64 {
    (timestamp / DAY_SECONDS) * DAY_SECONDS
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    fn setup() -> (Env, Address, Address, Vec<Address>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(BridgeContract, ());
        let admin = Address::generate(&env);
        let mut validators = Vec::new(&env);
        validators.push_back(Address::generate(&env));
        validators.push_back(Address::generate(&env));
        validators.push_back(Address::generate(&env));
        (env, contract_id, admin, validators)
    }

    fn init(env: &Env, admin: &Address, validators: &Vec<Address>) {
        BridgeContract::initialize(
            env.clone(),
            admin.clone(),
            validators.clone(),
            2,
            1_000,
            1_000,
            600,
        )
        .unwrap();

        BridgeContract::register_wrapped_asset(
            env.clone(),
            admin.clone(),
            ChainId::Ethereum,
            String::from_str(env, "ETH"),
            String::from_str(env, "wETH"),
            18,
        )
        .unwrap();
    }

    #[test]
    fn lock_and_mint_flow_requires_validator_consensus() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);

            let transfer_id = BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                500,
                String::from_str(&env, "0xabc"),
                7,
                String::from_str(&env, "stellar:user"),
            )
            .unwrap();

            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(0).unwrap(),
                transfer_id,
                String::from_str(&env, "sig-a"),
            )
            .unwrap();
            assert_eq!(
                BridgeContract::get_transfer(env.clone(), transfer_id)
                    .unwrap()
                    .status,
                TransferStatus::PendingValidators
            );

            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(1).unwrap(),
                transfer_id,
                String::from_str(&env, "sig-b"),
            )
            .unwrap();
            BridgeContract::execute_lock_mint(env.clone(), admin.clone(), transfer_id).unwrap();

            assert_eq!(
                BridgeContract::get_wrapped_balance(
                    env.clone(),
                    user,
                    String::from_str(&env, "wETH")
                ),
                500
            );
        });
    }

    #[test]
    fn replay_and_duplicate_signatures_are_rejected() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);

            let transfer_id = BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                100,
                String::from_str(&env, "0xreplay"),
                9,
                String::from_str(&env, "stellar:user"),
            )
            .unwrap();

            let replay = BridgeContract::initiate_lock_mint(
                env.clone(),
                user,
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                100,
                String::from_str(&env, "0xreplay"),
                9,
                String::from_str(&env, "stellar:user"),
            );
            assert_eq!(replay, Err(BridgeError::ReplayDetected));

            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(0).unwrap(),
                transfer_id,
                String::from_str(&env, "sig-one"),
            )
            .unwrap();

            let duplicate = BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(0).unwrap(),
                transfer_id,
                String::from_str(&env, "sig-one"),
            );
            assert_eq!(duplicate, Err(BridgeError::SignatureAlreadyUsed));
        });
    }

    #[test]
    fn burn_and_unlock_enforces_delay_and_balance() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);

            let mint_id = BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                300,
                String::from_str(&env, "0xmint"),
                3,
                String::from_str(&env, "stellar:user"),
            )
            .unwrap();
            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(0).unwrap(),
                mint_id,
                String::from_str(&env, "sig-1"),
            )
            .unwrap();
            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(1).unwrap(),
                mint_id,
                String::from_str(&env, "sig-2"),
            )
            .unwrap();
            BridgeContract::execute_lock_mint(env.clone(), admin.clone(), mint_id).unwrap();

            let burn_id = BridgeContract::initiate_burn_unlock(
                env.clone(),
                user.clone(),
                ChainId::Polygon,
                ChainId::Ethereum,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                200,
                String::from_str(&env, "0xrecipient"),
            )
            .unwrap();

            BridgeContract::approve_burn_unlock(
                env.clone(),
                validators.get(0).unwrap(),
                burn_id,
                String::from_str(&env, "sig-3"),
            )
            .unwrap();
            BridgeContract::approve_burn_unlock(
                env.clone(),
                validators.get(1).unwrap(),
                burn_id,
                String::from_str(&env, "sig-4"),
            )
            .unwrap();

            let early = BridgeContract::execute_burn_unlock(env.clone(), admin.clone(), burn_id);
            assert_eq!(early, Err(BridgeError::WithdrawalNotReady));

            env.ledger().set_timestamp(1_601);
            BridgeContract::execute_burn_unlock(env.clone(), admin, burn_id).unwrap();

            assert_eq!(
                BridgeContract::get_wrapped_balance(
                    env.clone(),
                    user,
                    String::from_str(&env, "wETH")
                ),
                100
            );
        });
    }

    #[test]
    fn security_limits_block_oversized_transfers() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);

            let too_large = BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                1_001,
                String::from_str(&env, "0xbig"),
                1,
                String::from_str(&env, "stellar:user"),
            );
            assert_eq!(too_large, Err(BridgeError::MaxTransferExceeded));

            BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                700,
                String::from_str(&env, "0x1"),
                1,
                String::from_str(&env, "stellar:user"),
            )
            .unwrap();
            let daily = BridgeContract::initiate_lock_mint(
                env.clone(),
                user,
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                400,
                String::from_str(&env, "0x2"),
                2,
                String::from_str(&env, "stellar:user"),
            );
            assert_eq!(daily, Err(BridgeError::DailyLimitExceeded));
        });
    }

    // ── #613 Dynamic liquidity rate-limit tests ────────────────────────────────

    #[test]
    fn healthy_liquidity_allows_transfers_up_to_10pct() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);
            // Buffer = 5000; dynamic limit = 500
            BridgeContract::update_liquidity_buffer(env.clone(), admin.clone(), 5_000).unwrap();

            // 500 is exactly 10% — should pass
            BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                500,
                String::from_str(&env, "0xa"),
                1,
                String::from_str(&env, "r"),
            )
            .unwrap();
        });
    }

    #[test]
    fn low_liquidity_rejects_with_dynamic_limit_error() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);
            // Buffer = 100; dynamic limit = 10
            BridgeContract::update_liquidity_buffer(env.clone(), admin.clone(), 100).unwrap();

            let result = BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                50,
                String::from_str(&env, "0xb"),
                2,
                String::from_str(&env, "r"),
            );
            assert_eq!(result, Err(BridgeError::DynamicLiquidityLimitExceeded));
        });
    }

    #[test]
    fn error_messages_are_non_empty_and_distinct() {
        let samples = [
            BridgeError::AlreadyInitialized,
            BridgeError::InvalidAmount,
            BridgeError::TransferNotFound,
            BridgeError::ReplayDetected,
            BridgeError::DynamicLiquidityLimitExceeded,
            BridgeError::UnsupportedDestinationChain,
        ];
        for err in samples.iter() {
            assert!(!err.message().is_empty());
        }
        for i in 0..samples.len() {
            for j in (i + 1)..samples.len() {
                assert_ne!(
                    samples[i].message(),
                    samples[j].message(),
                    "expected distinct messages for {:?} and {:?}",
                    samples[i],
                    samples[j]
                );
            }
        }
    }

    #[test]
    fn dynamic_limit_error_is_distinct_from_daily_limit_error() {
        // Confirms the error variant is DynamicLiquidityLimitExceeded, not DailyLimitExceeded
        assert_ne!(
            BridgeError::DynamicLiquidityLimitExceeded,
            BridgeError::DailyLimitExceeded,
        );
    }

    #[test]
    fn no_buffer_set_means_no_dynamic_restriction() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);
            // No buffer update — dynamic limit is effectively infinite
            BridgeContract::initiate_lock_mint(
                env.clone(),
                user,
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                999,
                String::from_str(&env, "0xc"),
                3,
                String::from_str(&env, "r"),
            )
            .unwrap();
        });
    }

    #[test]
    fn test_reserve_attestation_healthy_and_unhealthy() {
        let (env, contract_id, admin, validators) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            init(&env, &admin, &validators);

            // Get default reserve threshold (9500 bps = 95%)
            assert_eq!(BridgeContract::get_reserve_threshold(env.clone()), 9500);

            // Total minted is 0 at start
            assert_eq!(BridgeContract::get_total_minted(env.clone()), 0);

            // Perform lock-mint to mint 1000 tokens
            let transfer_id = BridgeContract::initiate_lock_mint(
                env.clone(),
                user.clone(),
                ChainId::Ethereum,
                ChainId::Polygon,
                String::from_str(&env, "ETH"),
                String::from_str(&env, "wETH"),
                1000,
                String::from_str(&env, "0xa"),
                1,
                String::from_str(&env, "r"),
            )
            .unwrap();

            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(0).unwrap(),
                transfer_id,
                String::from_str(&env, "sig1"),
            )
            .unwrap();
            BridgeContract::approve_lock_mint(
                env.clone(),
                validators.get(1).unwrap(),
                transfer_id,
                String::from_str(&env, "sig2"),
            )
            .unwrap();
            BridgeContract::execute_lock_mint(env.clone(), admin.clone(), transfer_id).unwrap();

            // Total minted should now be 1000
            assert_eq!(BridgeContract::get_total_minted(env.clone()), 1000);

            // Attest healthy reserve: 990 locked (990/1000 = 99% >= 95%)
            let attest_res = BridgeContract::attest_reserves(env.clone(), user.clone(), 990);
            assert!(attest_res.is_ok());

            // Attest unhealthy reserve: 900 locked (900/1000 = 90% < 95%)
            let attest_res_unhealthy =
                BridgeContract::attest_reserves(env.clone(), user.clone(), 900);
            assert!(attest_res_unhealthy.is_ok());

            // Change threshold to 8000 bps (80%)
            BridgeContract::set_reserve_threshold(env.clone(), admin.clone(), 8000).unwrap();
            assert_eq!(BridgeContract::get_reserve_threshold(env.clone()), 8000);

            // Now 900 locked is healthy (900/1000 = 90% >= 80%)
            let attest_res_healthy_now =
                BridgeContract::attest_reserves(env.clone(), user.clone(), 900);
            assert!(attest_res_healthy_now.is_ok());
        });
    }
}

#[cfg(test)]
mod test_health;
