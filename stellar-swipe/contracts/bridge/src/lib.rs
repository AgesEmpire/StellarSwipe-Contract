#![no_std]

feat/cross-chain-bridge-91
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, String, Symbol, Vec,
};

mod validators;

pub use validators::{ValidatorApproval, ValidatorApprovalKind, ValidatorSet};

const DAY_SECONDS: u64 = 86_400;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BridgeError {
    AlreadyInitialized = 1,
    InvalidAmount = 2,
    InvalidValidatorSet = 3,
    UnauthorizedValidator = 4,
    TransferNotFound = 5,
    TransferAlreadyExecuted = 6,
    ReplayDetected = 7,
    SignatureAlreadyUsed = 8,
    NotEnoughValidatorApprovals = 9,
    DailyLimitExceeded = 10,
    MaxTransferExceeded = 11,
    InsufficientWrappedBalance = 12,
    WithdrawalNotReady = 13,
    InvalidOperation = 14,
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
}

feat/bridge-liquidity-pools-96
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, String, Symbol, Vec,

use soroban_sdk::{contract, contractimpl, Env};
use stellar_swipe_common::HealthStatus;

pub mod monitoring;
pub mod governance;
pub mod analytics;
pub mod fees;
pub mod messaging;

pub use monitoring::{
    ChainFinalityConfig, ChainId, MonitoredTransaction, MonitoringStatus, VerificationMethod,
    BridgeTransfer, TransferStatus,
    monitor_source_transaction, get_monitored_tx, check_for_reorg, handle_reorg,
    update_transaction_confirmation_count, mark_transaction_failed, create_bridge_transfer,
    add_validator_signature, approve_transfer_for_minting, complete_transfer,
    get_chain_finality_config, set_chain_finality_config,
main
};

mod liquidity;
mod validators;

pub use liquidity::{LiquidityPool, LiquidityPosition, PoolHealth, PoolType, SwapResult};
pub use validators::{ValidatorApproval, ValidatorApprovalKind, ValidatorSet};

const DAY_SECONDS: u64 = 86_400;

feat/bridge-liquidity-pools-96
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BridgeError {
    AlreadyInitialized = 1,
    InvalidAmount = 2,
    InvalidValidatorSet = 3,
    UnauthorizedValidator = 4,
    TransferNotFound = 5,
    TransferAlreadyExecuted = 6,
    ReplayDetected = 7,
    SignatureAlreadyUsed = 8,
    NotEnoughValidatorApprovals = 9,
    DailyLimitExceeded = 10,
    MaxTransferExceeded = 11,
    InsufficientWrappedBalance = 12,
    WithdrawalNotReady = 13,
    InvalidOperation = 14,
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
}

pub use messaging::{
    CrossChainMessage, MessageStatus,
    MAX_MESSAGE_SIZE, MESSAGE_TIMEOUT,
    register_bridge_for_chain,
    send_cross_chain_message,
    relay_message_to_target_chain,
    confirm_message_delivery,
    receive_message_callback,
    mark_message_failed,
    retry_failed_message,
    expire_timed_out_message,
    get_cross_chain_message,
};
 main
 main

#[contract]
pub struct BridgeContract;

#[contractimpl]
impl BridgeContract {
feat/cross-chain-bridge-91

feat/bridge-liquidity-pools-96
main
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
        if !cfg!(test) {
            user.require_auth();
        }
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
        if !cfg!(test) {
            user.require_auth();
        }
        validate_amount_and_limits(&env, amount)?;

        let balance_key = DataKey::WrappedBalance(user.clone(), wrapped_asset.clone());
        let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        if balance < amount {
            return Err(BridgeError::InsufficientWrappedBalance);
        }
        env.storage()
            .persistent()
            .set(&balance_key, &(balance - amount));

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
 feat/cross-chain-bridge-91


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
main
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
}
 feat/cross-chain-bridge-91


    /// Read-only health for ops / frontends; no auth, no storage writes.
    pub fn health_check(env: Env) -> HealthStatus {
        crate::governance::bridge_health_check(&env)
    }
}

#[cfg(test)]
mod test_health;
 main
 main
