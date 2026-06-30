//! Multi-sig-approved emergency early-unstake path (Issue #754).
//!
//! A staker facing a genuine emergency can submit an early-unstake request.
//! The request only executes once N-of-M configured admins have approved it.
//! Approved executions apply a configurable penalty (basis points of stake).
//! Requests that time out without enough approvals expire and must be resubmitted.

use soroban_sdk::{contracttype, symbol_short, token, Address, Env, Vec};

use crate::{
    migration::{MigrationKey, StakeInfoV2},
    StorageKey, StakeVaultError,
};

// ── Types ──────────────────────────────────────────────────────────────────────

/// On-chain multi-sig configuration for emergency unstakes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyMultiSigConfig {
    /// Ordered list of eligible admin signers.
    pub admins: Vec<Address>,
    /// Number of approvals required (N-of-M).
    pub required: u32,
    /// Penalty applied on approved early unstake, in basis points (10_000 = 100%).
    pub penalty_bps: u32,
    /// Seconds after which an unapproved request expires and must be resubmitted.
    pub timeout_secs: u64,
}

/// Per-staker pending emergency unstake request.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyRequest {
    pub staker: Address,
    pub requested_at: u64,
    /// Addresses that have approved so far.
    pub approvals: Vec<Address>,
}

// ── Storage helpers ────────────────────────────────────────────────────────────

fn get_config(env: &Env) -> Option<EmergencyMultiSigConfig> {
    env.storage()
        .instance()
        .get(&StorageKey::EmergencyMultiSigConfig)
}

fn get_request(env: &Env, staker: &Address) -> Option<EmergencyRequest> {
    env.storage()
        .persistent()
        .get(&StorageKey::EmergencyRequest(staker.clone()))
}

fn save_request(env: &Env, req: &EmergencyRequest) {
    env.storage()
        .persistent()
        .set(&StorageKey::EmergencyRequest(req.staker.clone()), req);
}

fn remove_request(env: &Env, staker: &Address) {
    env.storage()
        .persistent()
        .remove(&StorageKey::EmergencyRequest(staker.clone()));
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Admin: configure the multi-sig parameters for emergency unstakes.
pub fn configure(
    env: &Env,
    caller: &Address,
    admins: Vec<Address>,
    required: u32,
    penalty_bps: u32,
    timeout_secs: u64,
) -> Result<(), StakeVaultError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(StakeVaultError::NotInitialized)?;
    if caller != &admin {
        return Err(StakeVaultError::Unauthorized);
    }
    if required == 0 || required as usize > admins.len() as usize {
        return Err(StakeVaultError::InvalidMultiSigConfig);
    }
    if penalty_bps > 10_000 {
        return Err(StakeVaultError::InvalidEmergencyPenalty);
    }

    let cfg = EmergencyMultiSigConfig {
        admins,
        required,
        penalty_bps,
        timeout_secs,
    };
    env.storage()
        .instance()
        .set(&StorageKey::EmergencyMultiSigConfig, &cfg);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("stake_v"), symbol_short!("emg_cfg")),
        (required, penalty_bps, timeout_secs),
    );
    Ok(())
}

/// Staker: submit an emergency early-unstake request.
/// Fails if no multi-sig is configured or a request already exists.
pub fn request(env: &Env, staker: &Address) -> Result<(), StakeVaultError> {
    get_config(env).ok_or(StakeVaultError::EmergencyNotConfigured)?;

    if get_request(env, staker).is_some() {
        return Err(StakeVaultError::EmergencyRequestAlreadyExists);
    }

    let req = EmergencyRequest {
        staker: staker.clone(),
        requested_at: env.ledger().timestamp(),
        approvals: Vec::new(env),
    };
    save_request(env, &req);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("stake_v"), symbol_short!("emg_req")),
        staker.clone(),
    );
    Ok(())
}

/// Admin signer: approve a pending emergency unstake request.
///
/// When the approval count reaches `required`, the unstake executes immediately
/// with the configured penalty applied to the withdrawn amount.
pub fn approve(
    env: &Env,
    signer: &Address,
    staker: &Address,
    token_addr: &Address,
) -> Result<(), StakeVaultError> {
    let cfg = get_config(env).ok_or(StakeVaultError::EmergencyNotConfigured)?;

    // Verify signer is one of the configured admins.
    let mut is_admin = false;
    for i in 0..cfg.admins.len() {
        if cfg.admins.get(i).unwrap() == *signer {
            is_admin = true;
            break;
        }
    }
    if !is_admin {
        return Err(StakeVaultError::Unauthorized);
    }

    let mut req = get_request(env, staker).ok_or(StakeVaultError::EmergencyRequestNotFound)?;

    // Expire stale requests.
    let now = env.ledger().timestamp();
    if cfg.timeout_secs > 0 && now > req.requested_at.saturating_add(cfg.timeout_secs) {
        remove_request(env, staker);
        return Err(StakeVaultError::EmergencyRequestExpired);
    }

    // Prevent duplicate approvals from the same signer.
    for i in 0..req.approvals.len() {
        if req.approvals.get(i).unwrap() == *signer {
            return Err(StakeVaultError::AlreadyApproved);
        }
    }

    req.approvals.push_back(signer.clone());

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("stake_v"), symbol_short!("emg_appr")),
        (staker.clone(), signer.clone(), req.approvals.len()),
    );

    if req.approvals.len() >= cfg.required {
        // Execute the early unstake with penalty.
        execute_early_unstake(env, staker, token_addr, cfg.penalty_bps)?;
        remove_request(env, staker);
    } else {
        save_request(env, &req);
    }

    Ok(())
}

/// Anyone may call this to clean up an expired request without executing it.
pub fn expire_request(env: &Env, staker: &Address) -> Result<(), StakeVaultError> {
    let cfg = get_config(env).ok_or(StakeVaultError::EmergencyNotConfigured)?;
    let req = get_request(env, staker).ok_or(StakeVaultError::EmergencyRequestNotFound)?;

    let now = env.ledger().timestamp();
    if cfg.timeout_secs == 0 || now <= req.requested_at.saturating_add(cfg.timeout_secs) {
        return Err(StakeVaultError::EmergencyRequestNotExpired);
    }

    remove_request(env, staker);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("stake_v"), symbol_short!("emg_exp")),
        staker.clone(),
    );
    Ok(())
}

/// Read the current pending request for a staker, if any.
pub fn get_emergency_request(env: &Env, staker: &Address) -> Option<EmergencyRequest> {
    get_request(env, staker)
}

/// Read the current multi-sig config, if set.
pub fn get_config_pub(env: &Env) -> Option<EmergencyMultiSigConfig> {
    get_config(env)
}

// ── Internal ──────────────────────────────────────────────────────────────────

fn execute_early_unstake(
    env: &Env,
    staker: &Address,
    token_addr: &Address,
    penalty_bps: u32,
) -> Result<(), StakeVaultError> {
    let mut stakes: soroban_sdk::Map<Address, StakeInfoV2> = env
        .storage()
        .persistent()
        .get(&MigrationKey::StakesV2)
        .unwrap_or_else(|| soroban_sdk::Map::new(env));

    let info = stakes.get(staker.clone()).ok_or(StakeVaultError::NoStake)?;
    if info.balance == 0 {
        return Err(StakeVaultError::NoStake);
    }

    let gross = info.balance;
    let penalty = core::cmp::min((gross * penalty_bps as i128) / 10_000, gross);
    let net = gross.saturating_sub(penalty);

    // Zero balance before transfer (checks-effects-interactions).
    stakes.set(
        staker.clone(),
        StakeInfoV2 {
            balance: 0,
            locked_until: 0,
            last_updated: env.ledger().timestamp(),
        },
    );
    env.storage()
        .persistent()
        .set(&MigrationKey::StakesV2, &stakes);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("stake_v"), symbol_short!("emg_exec")),
        (staker.clone(), gross, penalty, net),
    );

    // Burn the penalty portion, transfer the net to the staker.
    let token = token::Client::new(env, token_addr);
    if penalty > 0 {
        token.burn(&env.current_contract_address(), &penalty);
    }
    if net > 0 {
        token.transfer(&env.current_contract_address(), staker, &net);
    }

    Ok(())
}
