use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};
use stellar_swipe_common::emergency::{
    CircuitBreakerConfig, CircuitBreakerStats, PauseState, CAT_ALL, CAT_SIGNALS, CAT_STAKES,
    CAT_TRADING,
};
use stellar_swipe_common::validate_signer_config;

use crate::errors::AdminError;
use crate::events::*;

// Constants
pub const MAX_FEE_BPS: u32 = 100; // 1% max fee
pub const MAX_RISK_PERCENTAGE: u32 = 100; // 100% max
/// Wall-clock admin transfer validity (matches admin transfer tests).
const ADMIN_TRANSFER_EXPIRY_SECS: u64 = 48 * 60 * 60;

// Default values
pub const DEFAULT_MIN_STAKE: i128 = 100_000_000; // 100 XLM (7 decimals)
pub const DEFAULT_TRADE_FEE_BPS: u32 = 10; // 0.1%
pub const DEFAULT_STOP_LOSS: u32 = 15; // 15%
pub const DEFAULT_POSITION_LIMIT: u32 = 20; // 20%
pub const DEFAULT_BRONZE_SIGNAL_LIMIT: u32 = 5;
pub const DEFAULT_SILVER_SIGNAL_LIMIT: u32 = 10;
pub const DEFAULT_GOLD_SIGNAL_LIMIT: u32 = 20;

#[contracttype]
#[derive(Clone)]
pub enum AdminStorageKey {
    Admin,
    PendingAdminTransfer,
    Guardian,
    ConfigAdmin,
    EmergencyAdmin,
    TreasuryAdmin,
    MinStake,
    TradeFee,
    StopLoss,
    PositionLimit,
    PauseStates,
    CircuitBreakerStats,
    CircuitBreakerConfig,
    MultiSigEnabled,
    MultiSigSigners,
    MultiSigThreshold,
    FeeCollectionPaused,
    PendingAdmin,
    PendingAdminExpiry,
    BronzeSignalLimit,
    SilverSignalLimit,
    GoldSignalLimit,
    PreventSelfDestruct,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminRole {
    Config,
    Emergency,
    Treasury,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAdminTransfer {
    pub pending_admin: Address,
    pub expires_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AdminConfig {
    pub min_stake: i128,
    pub trade_fee_bps: u32,
    pub default_stop_loss: u32,
    pub default_position_limit: u32,
    pub bronze_signal_limit: u32,
    pub silver_signal_limit: u32,
    pub gold_signal_limit: u32,
}

/// Initialize admin with default parameters
pub fn init_admin(env: &Env, admin: Address) -> Result<(), AdminError> {
    if has_admin(env) {
        return Err(AdminError::AlreadyInitialized);
    }

    env.storage()
        .instance()
        .set(&AdminStorageKey::Admin, &admin);
    env.storage()
        .instance()
        .set(&AdminStorageKey::MinStake, &DEFAULT_MIN_STAKE);
    env.storage()
        .instance()
        .set(&AdminStorageKey::TradeFee, &DEFAULT_TRADE_FEE_BPS);
    env.storage()
        .instance()
        .set(&AdminStorageKey::StopLoss, &DEFAULT_STOP_LOSS);
    env.storage()
        .instance()
        .set(&AdminStorageKey::PositionLimit, &DEFAULT_POSITION_LIMIT);
    env.storage().instance().set(
        &AdminStorageKey::BronzeSignalLimit,
        &DEFAULT_BRONZE_SIGNAL_LIMIT,
    );
    env.storage().instance().set(
        &AdminStorageKey::SilverSignalLimit,
        &DEFAULT_SILVER_SIGNAL_LIMIT,
    );
    env.storage().instance().set(
        &AdminStorageKey::GoldSignalLimit,
        &DEFAULT_GOLD_SIGNAL_LIMIT,
    );
    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigEnabled, &false);

    // Self-destruct protection enabled by default.
    env.storage()
        .instance()
        .set(&AdminStorageKey::PreventSelfDestruct, &true);

    let states: Map<String, PauseState> = Map::new(env);
    env.storage()
        .instance()
        .set(&AdminStorageKey::PauseStates, &states);

    let cb_stats = CircuitBreakerStats {
        attempts_window: 0,
        failures_window: 0,
        window_start: env.ledger().timestamp(),
        volume_1h: 0,
        volume_24h_avg: 0,
        last_price: 0,
        last_price_time: 0,
    };
    env.storage()
        .instance()
        .set(&AdminStorageKey::CircuitBreakerStats, &cb_stats);

    Ok(())
}

/// Check if admin is initialized
pub fn has_admin(env: &Env) -> bool {
    env.storage().instance().has(&AdminStorageKey::Admin)
}

/// Get current admin address
pub fn get_admin(env: &Env) -> Result<Address, AdminError> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::Admin)
        .ok_or(AdminError::NotInitialized)
}

/// Set guardian address (admin only)
pub fn set_guardian(env: &Env, caller: &Address, guardian: Address) -> Result<(), AdminError> {
    require_emergency_admin(env, caller)?;
    caller.require_auth();
    set_guardian_direct(env, caller, guardian)
}

pub fn set_guardian_direct(
    env: &Env,
    _caller: &Address,
    guardian: Address,
) -> Result<(), AdminError> {
    env.storage()
        .instance()
        .set(&AdminStorageKey::Guardian, &guardian);
    emit_guardian_set(env, guardian);
    Ok(())
}

/// Revoke guardian (admin only)
pub fn revoke_guardian(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_emergency_admin(env, caller)?;
    caller.require_auth();
    let guardian: Address = env
        .storage()
        .instance()
        .get(&AdminStorageKey::Guardian)
        .ok_or(AdminError::NotInitialized)?;
    env.storage().instance().remove(&AdminStorageKey::Guardian);
    emit_guardian_revoked(env, guardian);
    Ok(())
}

/// Get current guardian, if any
pub fn get_guardian(env: &Env) -> Option<Address> {
    env.storage().instance().get(&AdminStorageKey::Guardian)
}

/// Check if caller is the guardian
fn is_guardian(env: &Env, caller: &Address) -> bool {
    get_guardian(env).map(|g| &g == caller).unwrap_or(false)
}

fn require_direct_admin_or_not_multisig(env: &Env, caller: &Address) -> Result<(), AdminError> {
    if is_multisig_enabled(env) {
        if !is_multisig_signer(env, caller) {
            return Err(AdminError::Unauthorized);
        }
        return Err(AdminError::RequiresMultisigApproval);
    }
    require_admin(env, caller)
}

fn role_key(role: &AdminRole) -> AdminStorageKey {
    match role {
        AdminRole::Config => AdminStorageKey::ConfigAdmin,
        AdminRole::Emergency => AdminStorageKey::EmergencyAdmin,
        AdminRole::Treasury => AdminStorageKey::TreasuryAdmin,
    }
}

pub fn get_admin_role(env: &Env, role: AdminRole) -> Option<Address> {
    env.storage().instance().get(&role_key(&role))
}

pub fn set_admin_role(
    env: &Env,
    caller: &Address,
    role: AdminRole,
    account: Address,
) -> Result<(), AdminError> {
    require_direct_admin_or_not_multisig(env, caller)?;
    caller.require_auth();
    env.storage().instance().set(&role_key(&role), &account);
    Ok(())
}

fn has_scoped_role(env: &Env, caller: &Address, role: &AdminRole) -> bool {
    env.storage()
        .instance()
        .get::<_, Address>(&role_key(role))
        .map(|role_admin| &role_admin == caller)
        .unwrap_or(false)
}

pub fn require_role_or_admin(
    env: &Env,
    caller: &Address,
    role: AdminRole,
) -> Result<(), AdminError> {
    if has_scoped_role(env, caller, &role) {
        return Ok(());
    }
    require_admin(env, caller)
}

pub fn require_config_admin(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_role_or_admin(env, caller, AdminRole::Config)
}

pub fn require_emergency_admin(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_role_or_admin(env, caller, AdminRole::Emergency)
}

pub fn require_treasury_admin(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_role_or_admin(env, caller, AdminRole::Treasury)
}

/// Verify caller is admin
pub fn require_admin(env: &Env, caller: &Address) -> Result<(), AdminError> {
    let admin = get_admin(env)?;

    if is_multisig_enabled(env) {
        // For multi-sig, caller must be one of the signers
        if !is_multisig_signer(env, caller) {
            return Err(AdminError::Unauthorized);
        }
        Ok(())
    } else {
        // For single admin, caller must be the admin
        if caller != &admin {
            return Err(AdminError::Unauthorized);
        }
        Ok(())
    }
}

fn get_pending_admin_transfer(env: &Env) -> Option<PendingAdminTransfer> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::PendingAdminTransfer)
}

fn require_active_pending_admin_transfer(env: &Env) -> Result<PendingAdminTransfer, AdminError> {
    let pending = get_pending_admin_transfer(env).ok_or(AdminError::PendingAdminNotFound)?;
    if env.ledger().sequence() > pending.expires_at_ledger {
        env.storage()
            .instance()
            .remove(&AdminStorageKey::PendingAdminTransfer);
        return Err(AdminError::PendingAdminExpired);
    }
    Ok(pending)
}

pub fn propose_admin_transfer(
    env: &Env,
    caller: &Address,
    new_admin: Address,
) -> Result<(), AdminError> {
    require_direct_admin_or_not_multisig(env, caller)?;
    caller.require_auth();
    propose_admin_transfer_direct(env, caller, new_admin)
}

pub fn propose_admin_transfer_direct(
    env: &Env,
    caller: &Address,
    new_admin: Address,
) -> Result<(), AdminError> {
    let expires_at_ledger = env
        .ledger()
        .sequence()
        .saturating_add((ADMIN_TRANSFER_EXPIRY_SECS / 5) as u32);
    let pending = PendingAdminTransfer {
        pending_admin: new_admin.clone(),
        expires_at_ledger,
    };

    env.storage()
        .instance()
        .set(&AdminStorageKey::PendingAdminTransfer, &pending);

    emit_admin_transfer_proposed(env, caller.clone(), new_admin, expires_at_ledger as u64);
    Ok(())
}

pub fn accept_admin_transfer(env: &Env, caller: &Address) -> Result<(), AdminError> {
    caller.require_auth();

    let pending = require_active_pending_admin_transfer(env)?;
    if caller != &pending.pending_admin {
        return Err(AdminError::Unauthorized);
    }

    let old_admin = get_admin(env)?;
    env.storage()
        .instance()
        .set(&AdminStorageKey::Admin, caller);
    env.storage()
        .instance()
        .remove(&AdminStorageKey::PendingAdminTransfer);

    emit_admin_transfer_completed(env, old_admin.clone(), caller.clone());
    emit_admin_transferred(env, old_admin, caller.clone());
    Ok(())
}

pub fn cancel_admin_transfer(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();
    require_active_pending_admin_transfer(env)?;
    env.storage()
        .instance()
        .remove(&AdminStorageKey::PendingAdminTransfer);
    Ok(())
}

/// Set minimum stake requirement
pub fn set_min_stake(env: &Env, caller: &Address, new_amount: i128) -> Result<(), AdminError> {
    require_config_admin(env, caller)?;
    caller.require_auth();
    set_min_stake_direct(env, caller, new_amount)
}

pub fn set_min_stake_direct(
    env: &Env,
    _caller: &Address,
    new_amount: i128,
) -> Result<(), AdminError> {
    if new_amount <= 0 {
        return Err(AdminError::InvalidParameter);
    }

    let old_value: i128 = env
        .storage()
        .instance()
        .get(&AdminStorageKey::MinStake)
        .unwrap_or(DEFAULT_MIN_STAKE);

    env.storage()
        .instance()
        .set(&AdminStorageKey::MinStake, &new_amount);

    emit_parameter_updated(
        env,
        soroban_sdk::Symbol::new(env, "min_stake"),
        old_value,
        new_amount,
    );
    Ok(())
}

/// Get minimum stake requirement
pub fn get_min_stake(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::MinStake)
        .unwrap_or(DEFAULT_MIN_STAKE)
}

/// Set trade fee in basis points
pub fn set_trade_fee(env: &Env, caller: &Address, new_fee_bps: u32) -> Result<(), AdminError> {
    require_config_admin(env, caller)?;
    caller.require_auth();
    set_trade_fee_direct(env, caller, new_fee_bps)
}

pub fn set_trade_fee_direct(
    env: &Env,
    _caller: &Address,
    new_fee_bps: u32,
) -> Result<(), AdminError> {
    if new_fee_bps > MAX_FEE_BPS {
        return Err(AdminError::InvalidFeeRate);
    }

    let old_value: u32 = env
        .storage()
        .instance()
        .get(&AdminStorageKey::TradeFee)
        .unwrap_or(DEFAULT_TRADE_FEE_BPS);

    env.storage()
        .instance()
        .set(&AdminStorageKey::TradeFee, &new_fee_bps);

    emit_parameter_updated(
        env,
        soroban_sdk::Symbol::new(env, "trade_fee"),
        old_value as i128,
        new_fee_bps as i128,
    );
    Ok(())
}

/// Get trade fee in basis points
pub fn get_trade_fee(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::TradeFee)
        .unwrap_or(DEFAULT_TRADE_FEE_BPS)
}

/// Set risk defaults (stop loss and position limit)
pub fn set_risk_defaults(
    env: &Env,
    caller: &Address,
    stop_loss: u32,
    position_limit: u32,
) -> Result<(), AdminError> {
    require_config_admin(env, caller)?;
    caller.require_auth();
    set_risk_defaults_direct(env, caller, stop_loss, position_limit)
}

pub fn set_risk_defaults_direct(
    env: &Env,
    _caller: &Address,
    stop_loss: u32,
    position_limit: u32,
) -> Result<(), AdminError> {
    if stop_loss > MAX_RISK_PERCENTAGE || position_limit > MAX_RISK_PERCENTAGE {
        return Err(AdminError::InvalidRiskParameter);
    }

    let old_stop_loss: u32 = env
        .storage()
        .instance()
        .get(&AdminStorageKey::StopLoss)
        .unwrap_or(DEFAULT_STOP_LOSS);

    let old_position_limit: u32 = env
        .storage()
        .instance()
        .get(&AdminStorageKey::PositionLimit)
        .unwrap_or(DEFAULT_POSITION_LIMIT);

    env.storage()
        .instance()
        .set(&AdminStorageKey::StopLoss, &stop_loss);
    env.storage()
        .instance()
        .set(&AdminStorageKey::PositionLimit, &position_limit);

    emit_parameter_updated(
        env,
        soroban_sdk::Symbol::new(env, "stop_loss"),
        old_stop_loss as i128,
        stop_loss as i128,
    );
    emit_parameter_updated(
        env,
        soroban_sdk::Symbol::new(env, "position_limit"),
        old_position_limit as i128,
        position_limit as i128,
    );

    Ok(())
}

/// Get default stop loss percentage
pub fn get_default_stop_loss(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::StopLoss)
        .unwrap_or(DEFAULT_STOP_LOSS)
}

/// Get default position limit percentage
pub fn get_default_position_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::PositionLimit)
        .unwrap_or(DEFAULT_POSITION_LIMIT)
}

/// Pause a category (admin or guardian; admin path requires multisig when enabled)
pub fn pause_category(
    env: &Env,
    caller: &Address,
    category: String,
    duration: Option<u64>,
    reason: String,
) -> Result<(), AdminError> {
    if is_guardian(env, caller) {
        caller.require_auth();
    } else {
        require_emergency_admin(env, caller)?;
        caller.require_auth();
    }
    pause_category_direct(env, caller, category, duration, reason)
}

pub fn pause_category_direct(
    env: &Env,
    caller: &Address,
    category: String,
    duration: Option<u64>,
    reason: String,
) -> Result<(), AdminError> {
    let now = env.ledger().timestamp();
    let auto_unpause_at = duration.map(|d| now + d);

    let pause_state = PauseState {
        paused: true,
        paused_at: now,
        auto_unpause_at,
        reason: reason.clone(),
    };

    let mut states = get_pause_states(env);
    states.set(category.clone(), pause_state);
    env.storage()
        .instance()
        .set(&AdminStorageKey::PauseStates, &states);

    emit_emergency_paused(env, category, caller.clone(), reason, auto_unpause_at);
    Ok(())
}

/// Pause trading (legacy wrapper)
pub fn pause_trading(env: &Env, caller: &Address) -> Result<(), AdminError> {
    pause_category(
        env,
        caller,
        String::from_str(env, CAT_TRADING),
        Some(48 * 60 * 60),
        String::from_str(env, "Manual pause"),
    )
}

/// Unpause a category
pub fn unpause_category(env: &Env, caller: &Address, category: String) -> Result<(), AdminError> {
    require_emergency_admin(env, caller)?;
    caller.require_auth();
    unpause_category_direct(env, caller, category)
}

pub fn unpause_category_direct(
    env: &Env,
    caller: &Address,
    category: String,
) -> Result<(), AdminError> {
    let mut states = get_pause_states(env);
    if states.contains_key(category.clone()) {
        states.remove(category.clone());
        env.storage()
            .instance()
            .set(&AdminStorageKey::PauseStates, &states);
        emit_emergency_unpaused(env, category, caller.clone());
    }

    Ok(())
}

/// Unpause trading (legacy wrapper)
pub fn unpause_trading(env: &Env, caller: &Address) -> Result<(), AdminError> {
    unpause_category(env, caller, String::from_str(env, CAT_TRADING))
}

/// Get all pause states
pub fn get_pause_states(env: &Env) -> Map<String, PauseState> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::PauseStates)
        .unwrap_or(Map::new(env))
}

/// Check if a category is paused
pub fn is_category_paused(env: &Env, category: String) -> bool {
    let states = get_pause_states(env);

    // Check "all" category first
    if let Some(all_pause) = states.get(String::from_str(env, CAT_ALL)) {
        if is_state_active(env, &all_pause) {
            return true;
        }
    }

    // Check specific category
    if let Some(pause) = states.get(category) {
        return is_state_active(env, &pause);
    }

    false
}

fn is_state_active(env: &Env, state: &PauseState) -> bool {
    if !state.paused {
        return false;
    }

    if let Some(auto_unpause_at) = state.auto_unpause_at {
        if env.ledger().timestamp() >= auto_unpause_at {
            return false;
        }
    }

    true
}

/// Check if trading is paused (legacy wrapper)
pub fn is_trading_paused(env: &Env) -> bool {
    is_category_paused(env, String::from_str(env, CAT_TRADING))
}

/// Require category not paused
pub fn require_not_paused(env: &Env, category: String) -> Result<(), AdminError> {
    if is_category_paused(env, category) {
        return Err(AdminError::TradingPaused);
    }
    Ok(())
}

/// Require trading not paused (legacy wrapper)
pub fn require_not_paused_legacy(env: &Env) -> Result<(), AdminError> {
    require_not_paused(env, String::from_str(env, CAT_TRADING))
}

/// Get pause info (legacy wrapper - returns CAT_TRADING info)
pub fn get_pause_info(env: &Env) -> PauseState {
    let states = get_pause_states(env);
    states
        .get(String::from_str(env, CAT_TRADING))
        .unwrap_or(PauseState {
            paused: false,
            paused_at: 0,
            auto_unpause_at: None,
            reason: String::from_str(env, ""),
        })
}

/// Get all admin configuration
pub fn get_admin_config(env: &Env) -> AdminConfig {
    AdminConfig {
        min_stake: get_min_stake(env),
        trade_fee_bps: get_trade_fee(env),
        default_stop_loss: get_default_stop_loss(env),
        default_position_limit: get_default_position_limit(env),
        bronze_signal_limit: get_bronze_signal_limit(env),
        silver_signal_limit: get_silver_signal_limit(env),
        gold_signal_limit: get_gold_signal_limit(env),
    }
}

pub fn set_tier_signal_limits(
    env: &Env,
    caller: &Address,
    bronze: u32,
    silver: u32,
    gold: u32,
) -> Result<(), AdminError> {
    require_config_admin(env, caller)?;
    caller.require_auth();
    set_tier_signal_limits_direct(env, caller, bronze, silver, gold)
}

pub fn set_tier_signal_limits_direct(
    env: &Env,
    _caller: &Address,
    bronze: u32,
    silver: u32,
    gold: u32,
) -> Result<(), AdminError> {
    env.storage()
        .instance()
        .set(&AdminStorageKey::BronzeSignalLimit, &bronze);
    env.storage()
        .instance()
        .set(&AdminStorageKey::SilverSignalLimit, &silver);
    env.storage()
        .instance()
        .set(&AdminStorageKey::GoldSignalLimit, &gold);
    Ok(())
}

pub fn get_bronze_signal_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::BronzeSignalLimit)
        .unwrap_or(DEFAULT_BRONZE_SIGNAL_LIMIT)
}

pub fn get_silver_signal_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::SilverSignalLimit)
        .unwrap_or(DEFAULT_SILVER_SIGNAL_LIMIT)
}

pub fn get_gold_signal_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::GoldSignalLimit)
        .unwrap_or(DEFAULT_GOLD_SIGNAL_LIMIT)
}

// ==================== Multi-Sig Functions ====================

/// Enable multi-sig admin with specified signers and threshold
pub fn enable_multisig(
    env: &Env,
    caller: &Address,
    signers: Vec<Address>,
    threshold: u32,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();

    validate_signer_config(&signers, threshold).map_err(|e| match e {
        stellar_swipe_common::MultisigError::DuplicateSigner => AdminError::DuplicateSigner,
        stellar_swipe_common::MultisigError::InvalidThreshold => AdminError::InvalidParameter,
        _ => AdminError::InvalidParameter,
    })?;

    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigEnabled, &true);
    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigSigners, &signers);
    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigThreshold, &threshold);

    Ok(())
}

/// Disable multi-sig admin
pub fn disable_multisig(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();

    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigEnabled, &false);

    Ok(())
}

/// Check if multi-sig is enabled
pub fn is_multisig_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&AdminStorageKey::MultiSigEnabled)
        .unwrap_or(false)
}

/// Check if address is a multi-sig signer
pub fn is_multisig_signer(env: &Env, address: &Address) -> bool {
    if !is_multisig_enabled(env) {
        return false;
    }

    let signers: Vec<Address> = env
        .storage()
        .instance()
        .get(&AdminStorageKey::MultiSigSigners)
        .unwrap_or(Vec::new(env));

    for i in 0..signers.len() {
        if &signers.get(i).unwrap() == address {
            return true;
        }
    }

    false
}

/// Get multi-sig signers
pub fn get_multisig_signers(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::MultiSigSigners)
        .unwrap_or(Vec::new(env))
}

/// Get multi-sig threshold
pub fn get_multisig_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&AdminStorageKey::MultiSigThreshold)
        .unwrap_or(0)
}

/// Add a multi-sig signer
pub fn add_multisig_signer(
    env: &Env,
    caller: &Address,
    new_signer: Address,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();

    if !is_multisig_enabled(env) {
        return Err(AdminError::NotInitialized);
    }

    let mut signers: Vec<Address> = get_multisig_signers(env);

    // Check if already a signer
    for i in 0..signers.len() {
        if signers.get(i).unwrap() == new_signer {
            return Err(AdminError::DuplicateSigner);
        }
    }

    signers.push_back(new_signer.clone());
    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigSigners, &signers);

    emit_multisig_signer_added(env, new_signer, caller.clone());
    Ok(())
}

/// Remove a multi-sig signer
pub fn remove_multisig_signer(
    env: &Env,
    caller: &Address,
    signer_to_remove: Address,
) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();

    if !is_multisig_enabled(env) {
        return Err(AdminError::NotInitialized);
    }

    let signers: Vec<Address> = get_multisig_signers(env);
    let threshold = get_multisig_threshold(env);

    // Ensure we don't go below threshold
    if signers.len() - 1 < threshold {
        return Err(AdminError::InsufficientSignatures);
    }

    let mut new_signers = Vec::new(env);
    let mut found = false;

    for i in 0..signers.len() {
        let signer = signers.get(i).unwrap();
        if signer == signer_to_remove {
            found = true;
        } else {
            new_signers.push_back(signer);
        }
    }

    if !found {
        return Err(AdminError::Unauthorized);
    }

    env.storage()
        .instance()
        .set(&AdminStorageKey::MultiSigSigners, &new_signers);

    emit_multisig_signer_removed(env, signer_to_remove, caller.clone());
    Ok(())
}

// ==================== Fee Collection Pause (Issue #189) ====================

/// Pause fee collection. Read operations and position closures continue.
pub fn pause_fee_collection(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_emergency_admin(env, caller)?;
    caller.require_auth();
    pause_fee_collection_direct(env, caller)
}

pub fn pause_fee_collection_direct(env: &Env, _caller: &Address) -> Result<(), AdminError> {
    env.storage()
        .instance()
        .set(&AdminStorageKey::FeeCollectionPaused, &true);

    emit_parameter_updated(env, soroban_sdk::Symbol::new(env, "fee_paused"), 0, 1);
    Ok(())
}

/// Resume fee collection.
pub fn resume_fee_collection(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_emergency_admin(env, caller)?;
    caller.require_auth();
    resume_fee_collection_direct(env, caller)
}

pub fn resume_fee_collection_direct(env: &Env, _caller: &Address) -> Result<(), AdminError> {
    env.storage()
        .instance()
        .set(&AdminStorageKey::FeeCollectionPaused, &false);

    emit_parameter_updated(env, soroban_sdk::Symbol::new(env, "fee_paused"), 1, 0);
    Ok(())
}

/// Check if fee collection is paused.
pub fn is_fee_collection_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&AdminStorageKey::FeeCollectionPaused)
        .unwrap_or(false)
}

/// Set circuit breaker configuration
pub fn set_circuit_breaker_config(
    env: &Env,
    caller: &Address,
    config: CircuitBreakerConfig,
) -> Result<(), AdminError> {
    require_emergency_admin(env, caller)?;
    caller.require_auth();

    env.storage()
        .instance()
        .set(&AdminStorageKey::CircuitBreakerConfig, &config);

    Ok(())
}

/// Get circuit breaker configuration
pub fn get_circuit_breaker_config(env: &Env) -> Option<CircuitBreakerConfig> {
    env.storage()
        .instance()
        .get(&AdminStorageKey::CircuitBreakerConfig)
}

/// Get circuit breaker stats
pub fn get_circuit_breaker_stats(env: &Env) -> CircuitBreakerStats {
    env.storage()
        .instance()
        .get(&AdminStorageKey::CircuitBreakerStats)
        .unwrap_or(CircuitBreakerStats {
            attempts_window: 0,
            failures_window: 0,
            window_start: env.ledger().timestamp(),
            volume_1h: 0,
            volume_24h_avg: 0,
            last_price: 0,
            last_price_time: 0,
        })
}

/// Update circuit breaker stats and check for triggers
pub fn update_circuit_breaker_stats(env: &Env, failed: bool, volume: i128, price: i128) {
    let mut stats = get_circuit_breaker_stats(env);
    let now = env.ledger().timestamp();

    // Reset 10m window if needed
    if now >= stats.window_start + 600 {
        stats.attempts_window = 0;
        stats.failures_window = 0;
        stats.window_start = now;
    }

    stats.attempts_window += 1;
    if failed {
        stats.failures_window += 1;
    }

    // Simplified: update volume (real implementation would use a sliding window for 1h/24h)
    stats.volume_1h += volume;

    // Update price
    if price > 0 {
        stats.last_price = price;
        stats.last_price_time = now;
    }

    env.storage()
        .instance()
        .set(&AdminStorageKey::CircuitBreakerStats, &stats);

    // Check circuit breaker triggers
    if let Some(config) = get_circuit_breaker_config(env) {
        if let Some(reason) =
            stellar_swipe_common::emergency::check_thresholds(env, &stats, &config, price)
        {
            // Auto-pause "all" category
            let pause_state = PauseState {
                paused: true,
                paused_at: now,
                auto_unpause_at: None, // Circuit breakers require manual unpause
                reason: reason.clone(),
            };
            let mut states = get_pause_states(env);
            states.set(String::from_str(env, CAT_ALL), pause_state);
            env.storage()
                .instance()
                .set(&AdminStorageKey::PauseStates, &states);

            emit_circuit_breaker_triggered(env, String::from_str(env, CAT_ALL), reason);
        }
    }
}

// ==================== Self-Destruct Protection ====================

/// Returns `true` when the contract deletion guard is active.
pub fn is_self_destruct_protected(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&AdminStorageKey::PreventSelfDestruct)
        .unwrap_or(true) // safe default: protected
}

/// Block any operation that would delete the contract while protection is on.
/// Returns `Err(AdminError::Unauthorized)` if protection is enabled.
pub fn require_self_destruct_allowed(env: &Env) -> Result<(), AdminError> {
    if is_self_destruct_protected(env) {
        return Err(AdminError::Unauthorized);
    }
    Ok(())
}

/// Governance-only: disable self-destruct protection so the contract can be
/// deleted after a successful governance proposal.
pub fn disable_self_destruct_protection(env: &Env, caller: &Address) -> Result<(), AdminError> {
    require_admin(env, caller)?;
    caller.require_auth();
    env.storage()
        .instance()
        .set(&AdminStorageKey::PreventSelfDestruct, &false);
    Ok(())
}
