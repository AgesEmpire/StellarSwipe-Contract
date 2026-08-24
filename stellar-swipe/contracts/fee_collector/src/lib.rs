#![no_std]

mod errors;
pub use errors::ContractError;

mod events;
mod fee_cache;
use events::{
    emit_effective_multiplier_changed, emit_error_reported, emit_fee_collected, emit_fee_forecast,
    emit_fee_rate_updated, emit_fees_claimed, emit_fees_claimed_converted,
    emit_first_trade_fee_waived, emit_insurance_payout, emit_insurance_payout_cap_updated,
    emit_network_condition_updated, emit_payout_currency_set, emit_rebate_cap_applied,
    emit_referral_fee_paid, emit_referral_fee_share_updated, emit_referral_registered,
    emit_retry_attempted, emit_snapshot_recorded, emit_treasury_withdrawal,
    emit_volume_discount_config_updated, emit_waterfall_distribution, emit_withdrawal_queued,
    EvtEffectiveMultiplierChanged, EvtErrorReported, EvtFeeCollected, EvtFeeRateUpdated,
    EvtFeesClaimed, EvtInsurancePayout, EvtNetworkConditionUpdated, EvtRebateCapApplied,
    EvtRetryAttempted, EvtSnapshotRecorded, EvtTreasuryWithdrawal, EvtWithdrawalQueued,
};
pub use events::{
    EffectiveMultiplierChanged, FeeRateUpdated, FeesBurned, FeesClaimed, FirstTradeFeeWaived,
    InsurancePayout, InsurancePayoutCapUpdated, SnapshotRecorded, TreasuryWithdrawal,
    WithdrawalQueued,
};

mod rebates;

mod reports;
pub use reports::{EarningsLeaderboardEntry, EarningsReport, ReportPeriod};

mod storage;
pub use storage::BalanceMismatch;
use storage::{
    add_daily_fee_total, add_epoch_rebate_distributed, add_to_revenue_share_pool_index, get_admin,
    get_burn_rate, get_congestion_config, get_congestion_signal, get_daily_fee_total,
    get_epoch_rebate_distributed, get_failed_fee_collection, get_fee_optimization_config,
    get_fee_rate, get_fee_snapshot, get_forecast_config, get_last_error_report,
    get_last_forecast_day, get_max_rebate_bps, get_monthly_trade_volume,
    get_network_condition_score, get_oracle_contract, get_pending_fees,
    get_provider_payout_currency, get_queued_withdrawal, get_referral_fee_share_bps, get_referrer,
    get_revenue_share_pool_index, get_treasury_balance, get_volume_discount_config,
    get_waterfall_config, has_traded, is_authorized_caller, is_initialized,
    remove_authorized_caller, remove_failed_fee_collection, remove_from_revenue_share_pool_index,
    remove_monthly_trade_volume, remove_provider_payout_currency, remove_queued_withdrawal,
    set_admin, set_authorized_caller, set_burn_rate as set_burn_rate_storage,
    set_congestion_config, set_congestion_signal, set_failed_fee_collection,
    set_fee_optimization_config, set_fee_rate as set_fee_rate_storage, set_fee_snapshot,
    set_forecast_config_storage, set_has_traded, set_initialized, set_last_error_report,
    set_last_forecast_day, set_max_rebate_bps as set_max_rebate_bps_storage,
    set_monthly_trade_volume, set_network_condition_score,
    set_oracle_contract as set_oracle_contract_storage, set_pending_fees,
    set_provider_payout_currency, set_queued_withdrawal, set_referral_fee_share_bps, set_referrer,
    set_treasury_balance, set_volume_discount_config_storage,
    set_waterfall_config as set_waterfall_config_storage, CongestionConfig, CongestionSignal,
    ErrorReport, FailedFeeCollection, FeeOptimizationConfig, FeeSnapshot, ForecastConfigData,
    MonthlyTradeVolume, QueuedWithdrawal, SnapshotEntry, StorageKey, VolumeDiscountConfig,
    VolumeTier, WaterfallConfig, WaterfallTier, WaterfallTierResult, MAX_BURN_RATE_BPS,
    MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS, SECONDS_PER_DAY_FC,
};
pub use storage::{
    get_insurance_balance, get_insurance_payout_cap, is_insurance_claim_processed,
    set_insurance_balance, set_insurance_claim_processed, set_insurance_payout_cap,
};

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, BytesN, Env, IntoVal, String, Symbol,
    Val, Vec,
};

use shared::errors::{ErrorCategory, RecoveryStrategy};
use shared::pausable;
use shared::reentrancy::{self, ReentrancyError};
use stellar_swipe_common::health::{health_uninitialized, HealthStatus};
use stellar_swipe_common::token_metadata::{validate as validate_token_metadata, TokenMetadata};
use stellar_swipe_common::Asset;
use stellar_swipe_common::SECONDS_PER_DAY;

#[cfg(test)]
mod tests;

/// Compute the fee charged to a trader using **floor (truncating) division**.
///
/// `fee = floor(trade_amount * fee_rate_bps / 10_000)`
///
/// This is **user-favorable**: the trader is never charged more than their exact
/// pro-rata fee.  The sub-unit remainder stays with the trader and is not
/// retained by the contract, so no unwithdrawable dust accumulates.
///
/// Returns `None` on arithmetic overflow.
pub fn fee_amount_floor(trade_amount: i128, fee_rate_bps: u32) -> Option<i128> {
    trade_amount
        .checked_mul(fee_rate_bps as i128)?
        .checked_div(10_000)
}

/// Compute fee and burn amounts in a single pass, avoiding a second multiply
/// (issue #633 — reduce computation steps on the hot path).
///
/// Returns `(fee_amount, burn_amount)` or `None` on overflow.
/// `burn_amount = floor(fee_amount * burn_rate_bps / 10_000)`
pub fn fee_and_burn_amounts(
    trade_amount: i128,
    fee_rate_bps: u32,
    burn_rate_bps: u32,
) -> Option<(i128, i128)> {
    let fee = fee_amount_floor(trade_amount, fee_rate_bps)?;
    let burn = fee
        .checked_mul(burn_rate_bps as i128)?
        .checked_div(10_000)?;
    Some((fee, burn))
}

/// Maximum fee collections per `batch_collect_fees` call.
pub const MAX_BATCH_FEE_SIZE: u32 = 20;

/// Maximum tokens allowed to be audited in a single call.
pub const MAX_AUDIT_TOKENS: u32 = 20;

#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchFeeInput {
    pub trader: Address,
    pub token: Address,
    pub trade_amount: i128,
    pub trade_asset: Asset,
}

soroban_sdk::contractmeta!(key = "SourceHash", val = env!("STELLAR_SOURCE_HASH"));
soroban_sdk::contractmeta!(key = "GitCommit", val = env!("STELLAR_GIT_COMMIT"));

#[contract]
pub struct FeeCollector;

#[contractimpl]
impl FeeCollector {
    /// # Summary
    /// One-time contract initialization. Sets the admin address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    ///
    /// # Returns
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

    /// `Ok(())` on success.
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] if the contract has already been initialized.
    pub fn initialize(env: Env, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();
        if is_initialized(&env) {
            return Err(ContractError::AlreadyInitialized);
        }
        set_admin(&env, &admin);
        set_initialized(&env);
        shared::version::set_contract_version(&env, shared::version::FEE_COLLECTOR_VERSION);
        Ok(())
    }

    // ── Issue #811: upgrade-safe contract versioning ─────────────────────────

    /// Returns this contract's stored version. Callers that depend on this
    /// contract cross-contract can use this to enforce a minimum compatible
    /// version before invoking privileged entry points (see
    /// `shared::version::validate_callee_version`).
    pub fn get_contract_version(env: Env) -> u32 {
        shared::version::get_contract_version(&env)
    }

    /// Admin-only: replace this contract's executable with `new_wasm_hash`
    /// (previously uploaded via `Deployer::upload_contract_wasm`) and record
    /// `new_version` as the contract's version.
    ///
    /// `new_version` must be strictly greater than the currently stored
    /// version — this rejects accidental (or malicious) downgrades that
    /// could reintroduce a fixed bug or reopen a storage-layout mismatch.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::IncompatibleContractVersion`] — `new_version` is
    ///   not strictly greater than the currently stored version.
    pub fn upgrade(
        env: Env,
        new_wasm_hash: BytesN<32>,
        new_version: u32,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        let current_version = shared::version::get_contract_version(&env);
        shared::version::guard_upgrade(current_version, new_version)
            .map_err(|_| ContractError::IncompatibleContractVersion)?;

        env.deployer().update_current_contract_wasm(new_wasm_hash);
        shared::version::set_contract_version(&env, new_version);
        shared::version::emit_contract_upgraded(&env, current_version, new_version);
        Ok(())
    }

    /// Admin-only: pause fund-moving operations.
    pub fn pause(env: Env) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        pausable::set_paused(&env, true);
        Ok(())
    }

    /// Resume fund-moving operations. Admin auth required.
    pub fn unpause(env: Env) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        pausable::set_paused(&env, false);
        Ok(())
    }

    pub fn is_paused(env: Env) -> bool {
        pausable::is_paused(&env)
    }

    pub fn require_not_paused(env: &Env) -> Result<(), ContractError> {
        pausable::require_not_paused(env).map_err(|_| ContractError::ContractPaused)
    }

    // ── Issue #813: cross-contract authentication hardening ─────────────────
    //
    // A small admin-curated allowlist of non-admin addresses (typically other
    // contracts, such as a trusted trade-settlement keeper) permitted to
    // invoke a narrow set of privileged entry points that are not naturally
    // scoped to a single end-user's own `require_auth()`. The admin is
    // always implicitly authorized and is never required to be on this list.

    /// Admin-only: add `caller` to the authorized-caller allowlist.
    pub fn authorize_caller(env: Env, caller: Address) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        set_authorized_caller(&env, &caller);
        events::emit_caller_authorized(&env, &caller);
        Ok(())
    }

    /// Admin-only: remove `caller` from the authorized-caller allowlist.
    pub fn revoke_caller(env: Env, caller: Address) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        remove_authorized_caller(&env, &caller);
        events::emit_caller_revoked(&env, &caller);
        Ok(())
    }

    /// Returns `true` if `caller` is on the authorized-caller allowlist.
    /// The admin is always implicitly authorized even if not listed here.
    pub fn is_caller_authorized(env: Env, caller: Address) -> bool {
        is_authorized_caller(&env, &caller)
    }

    /// # Summary
    /// Set the oracle contract address used for price-based fee calculations.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `oracle_contract`: Address of the oracle contract.
    ///
    /// # Returns
    /// `Ok(())` on success.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn set_oracle_contract(env: Env, oracle_contract: Address) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        set_oracle_contract_storage(&env, &oracle_contract);
        Ok(())
    }

    /// # Summary
    /// Returns the effective fee rate in basis points for a specific user,
    /// accounting for any volume-based rebates.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `user`: Address of the trader.
    ///
    /// # Returns
    /// Fee rate in basis points (e.g. `30` = 0.30%).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn fee_rate_for_user(env: Env, user: Address) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(rebates::get_fee_rate_for_user(&env, &user))
    }

    /// # Summary
    /// Returns the 30-day rolling trade volume in USD for a user.
    /// Used to determine rebate tier eligibility.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `user`: Address of the trader.
    ///
    /// # Returns
    /// Volume in USD (scaled by asset decimals).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn monthly_trade_volume(env: Env, user: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(rebates::get_active_volume_usd(&env, &user))
    }

    /// # Summary
    /// Returns the current treasury balance for a given token.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `token`: SEP-41 token contract address.
    ///
    /// # Returns
    /// Balance in the token's native units.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn treasury_balance(env: Env, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_treasury_balance(&env, &token))
    }

    /// # Summary
    /// Read-only audit that compares the stored treasury balance against the
    /// contract's actual on-chain token balance for each supplied token.
    ///
    /// Call this off-chain (via `simulateTransaction`) or from monitoring
    /// scripts after unusual activity to detect accounting discrepancies.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `tokens`: List of SEP-41 token addresses to audit.
    ///
    /// # Returns
    /// A `Vec<BalanceMismatch>` containing one entry per token where
    /// `actual != expected`. Returns an empty vec when all balances reconcile.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn audit_balances(
        env: Env,
        tokens: Vec<Address>,
    ) -> Result<Vec<BalanceMismatch>, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        if tokens.len() > MAX_AUDIT_TOKENS {
            return Err(ContractError::IterationLimitExceeded);
        }
        let mut mismatches = Vec::new(&env);
        for token in tokens.iter() {
            let expected = get_treasury_balance(&env, &token);
            let actual = token::Client::new(&env, &token).balance(&env.current_contract_address());
            if actual != expected {
                mismatches.push_back(BalanceMismatch {
                    token: token.clone(),
                    expected,
                    actual,
                    delta: actual.saturating_sub(expected),
                });
            }
        }
        Ok(mismatches)
    }

    /// # Summary
    /// Queue a treasury withdrawal. The withdrawal becomes executable after a
    /// 24-hour timelock. Admin auth required.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `recipient`: Address that will receive the tokens.
    /// - `token`: SEP-41 token contract address.
    /// - `amount`: Amount to withdraw (must be > 0 and <= treasury balance).
    ///
    /// # Returns
    /// `Ok(())` on success. Emits a [`WithdrawalQueued`] event.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::InvalidAmount`] — amount <= 0.
    /// - [`ContractError::InsufficientTreasuryBalance`] — amount exceeds balance.
    pub fn queue_withdrawal(
        env: Env,
        recipient: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        // Validate token metadata before allowing withdrawal.
        Self::validate_token_metadata_for_token(&env, &token)?;
        if amount > get_treasury_balance(&env, &token) {
            return Err(ContractError::InsufficientTreasuryBalance);
        }
        let queued_at = env.ledger().timestamp();
        let available_at = queued_at
            .checked_add(SECONDS_PER_DAY)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_queued_withdrawal(
            &env,
            &QueuedWithdrawal {
                recipient: recipient.clone(),
                token: token.clone(),
                amount,
                queued_at,
            },
        );
        emit_withdrawal_queued(
            &env,
            EvtWithdrawalQueued {
                recipient: recipient.clone(),
                token: token.clone(),
                amount,
                available_at,
            },
        );
        Ok(())
    }

    /// # Summary
    /// Execute a previously queued treasury withdrawal after the 24-hour timelock.
    /// Admin auth required. Parameters must exactly match the queued withdrawal.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `recipient`: Must match the queued recipient.
    /// - `token`: Must match the queued token.
    /// - `amount`: Must match the queued amount.
    ///
    /// # Returns
    /// `Ok(())` on success. Transfers tokens and emits [`TreasuryWithdrawal`].
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::WithdrawalNotQueued`] — no matching queued withdrawal.
    /// - [`ContractError::TimelockNotElapsed`] — 24-hour timelock has not passed.
    /// - [`ContractError::InsufficientTreasuryBalance`] — balance changed since queuing.
    pub fn withdraw_treasury_fees(
        env: Env,
        recipient: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        // Issue #859: reentrancy guard for token transfer.
        reentrancy::require_not_locked(&env).map_err(|_| ContractError::Unauthorized)?;
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();

        // Validate registered token metadata before allowing withdrawal.
        Self::validate_token_metadata_for_token(&env, &token)?;

        let queued = match get_queued_withdrawal(&env) {
            Some(q) if q.recipient == recipient && q.token == token && q.amount == amount => q,
            _ => return Err(ContractError::WithdrawalNotQueued),
        };

        if env.ledger().timestamp()
            < queued
                .queued_at
                .checked_add(SECONDS_PER_DAY)
                .ok_or(ContractError::ArithmeticOverflow)?
        {
            return Err(ContractError::TimelockNotElapsed);
        }

        if amount > get_treasury_balance(&env, &token) {
            return Err(ContractError::InsufficientTreasuryBalance);
        }

        let new_balance = get_treasury_balance(&env, &token)
            .checked_sub(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;

        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        set_treasury_balance(&env, &token, new_balance);
        remove_queued_withdrawal(&env);

        emit_treasury_withdrawal(
            &env,
            EvtTreasuryWithdrawal {
                recipient: recipient.clone(),
                token: token.clone(),
                amount,
                remaining_balance: new_balance,
            },
        );

        Ok(())
    }

    // ── Issue #960: Insurance Payout Function with Cap and Audit Log ─────────

    /// Admin-only: configure the maximum insurance payout cap per claim for a token.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `caller`: Admin address (must match contract admin).
    /// - `token`: Asset token address.
    /// - `max_cap`: Maximum amount allowed per insurance claim (must be >= 0).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::UnauthorizedCaller`] — caller is not the admin.
    /// - [`ContractError::InvalidAmount`] — `max_cap` < 0.
    pub fn set_insurance_payout_cap(
        env: Env,
        caller: Address,
        token: Address,
        max_cap: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        caller.require_auth();
        let admin = get_admin(&env);
        if caller != admin {
            return Err(ContractError::UnauthorizedCaller);
        }
        if max_cap < 0 {
            return Err(ContractError::InvalidAmount);
        }

        let old_cap = get_insurance_payout_cap(&env, &token);
        set_insurance_payout_cap(&env, &token, max_cap);
        emit_insurance_payout_cap_updated(&env, &token, old_cap, max_cap, &caller);
        Ok(())
    }

    /// Query the configured maximum insurance payout cap per claim for a token.
    pub fn get_insurance_payout_cap(env: Env, token: Address) -> i128 {
        get_insurance_payout_cap(&env, &token)
    }

    /// Query the current insurance fund balance for a token.
    pub fn get_insurance_balance(env: Env, token: Address) -> i128 {
        get_insurance_balance(&env, &token)
    }

    /// Deposit funds directly into the insurance fund.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `from`: Funding account authorizing the deposit.
    /// - `token`: Asset token address.
    /// - `amount`: Amount to deposit (must be > 0).
    pub fn deposit_insurance_fund(
        env: Env,
        from: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;
        from.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        token::Client::new(&env, &token).transfer(&from, &env.current_contract_address(), &amount);

        let current_bal = get_insurance_balance(&env, &token);
        let new_bal = current_bal
            .checked_add(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_insurance_balance(&env, &token, new_bal);
        Ok(())
    }

    /// Checks whether an insurance claim ID has already been paid out.
    pub fn is_insurance_claim_processed(env: Env, claim_id: String) -> bool {
        is_insurance_claim_processed(&env, &claim_id)
    }

    /// Pays out insurance funds to a provider or treasury while enforcing
    /// the maximum payout cap per claim and emitting an audit event.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `caller`: Admin or allowlisted keeper address.
    /// - `recipient`: Recipient provider or treasury address.
    /// - `token`: Asset token address.
    /// - `amount`: Payout amount (must be > 0 and <= max_cap if cap configured).
    /// - `claim_id`: Unique identifier for the insurance claim (prevents double payout).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::ContractPaused`] — contract is paused.
    /// - [`ContractError::UnauthorizedCaller`] — caller is neither admin nor allowlisted caller.
    /// - [`ContractError::InvalidAmount`] — amount <= 0.
    /// - [`ContractError::ClaimAlreadyProcessed`] — claim_id already paid out.
    /// - [`ContractError::PayoutExceedsCap`] — amount exceeds configured cap per claim.
    /// - [`ContractError::InsufficientInsuranceBalance`] — insurance balance insufficient.
    pub fn payout_insurance(
        env: Env,
        caller: Address,
        recipient: Address,
        token: Address,
        amount: i128,
        claim_id: String,
    ) -> Result<(), ContractError> {
        reentrancy::require_not_locked(&env).map_err(|_| ContractError::Unauthorized)?;
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;

        caller.require_auth();
        let admin = get_admin(&env);
        if caller != admin && !is_authorized_caller(&env, &caller) {
            return Err(ContractError::UnauthorizedCaller);
        }

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        if is_insurance_claim_processed(&env, &claim_id) {
            return Err(ContractError::ClaimAlreadyProcessed);
        }

        let cap = get_insurance_payout_cap(&env, &token);
        if cap > 0 && amount > cap {
            return Err(ContractError::PayoutExceedsCap);
        }

        let current_balance = get_insurance_balance(&env, &token);
        if amount > current_balance {
            return Err(ContractError::InsufficientInsuranceBalance);
        }

        let new_balance = current_balance
            .checked_sub(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_insurance_balance(&env, &token, new_balance);
        set_insurance_claim_processed(&env, &claim_id, true);

        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        emit_insurance_payout(
            &env,
            EvtInsurancePayout {
                recipient,
                token,
                amount,
                claim_id,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Returns the current fee rate in basis points.
    pub fn fee_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_fee_rate(&env))
    }

    /// Admin-only: update the fee rate (in basis points).
    pub fn set_fee_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        if new_rate_bps > MAX_FEE_RATE_BPS {
            return Err(ContractError::FeeRateTooHigh);
        }
        if new_rate_bps < MIN_FEE_RATE_BPS {
            return Err(ContractError::FeeRateTooLow);
        }

        let old_rate = get_fee_rate(&env);
        set_fee_rate_storage(&env, new_rate_bps);
        storage::bump_config_version(&env);

        emit_fee_rate_updated(
            &env,
            EvtFeeRateUpdated {
                old_rate,
                new_rate: new_rate_bps,
                updated_by: admin,
            },
        );

        Ok(())
    }

    /// Returns the current burn rate in basis points (default: 1000 = 10%).
    pub fn burn_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_burn_rate(&env))
    }

    /// Admin-only: set the percentage of collected fees to burn (in basis points).
    /// Max is 10_000 (100%). Change takes effect on the next fee collection.
    pub fn set_burn_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if new_rate_bps > MAX_BURN_RATE_BPS {
            return Err(ContractError::BurnRateTooHigh);
        }
        set_burn_rate_storage(&env, new_rate_bps);
        storage::bump_config_version(&env);
        Ok(())
    }

    /// Admin: update the congestion configuration parameters.
    pub fn set_congestion_config(env: Env, config: CongestionConfig) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        if config.min_multiplier_bps == 0
            || config.min_multiplier_bps > config.max_multiplier_bps
            || config.default_multiplier_bps < config.min_multiplier_bps
            || config.default_multiplier_bps > config.max_multiplier_bps
        {
            return Err(ContractError::InvalidMultiplierBounds);
        }

        set_congestion_config(&env, &config);
        Ok(())
    }

    /// Admin, or an allowlisted keeper (Issue #813 — see [`Self::authorize_caller`]):
    /// push the current network congestion signal.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::UnauthorizedCaller`] — `caller` is neither the
    ///   admin nor on the authorized-caller allowlist.
    pub fn set_congestion_signal(
        env: Env,
        caller: Address,
        multiplier_bps: u32,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        caller.require_auth();
        if caller != get_admin(&env) && !is_authorized_caller(&env, &caller) {
            return Err(ContractError::UnauthorizedCaller);
        }

        let config = get_congestion_config(&env);
        if multiplier_bps < config.min_multiplier_bps || multiplier_bps > config.max_multiplier_bps
        {
            return Err(ContractError::InvalidMultiplierBounds);
        }

        // Get effective multiplier BEFORE setting the new signal (to check if it actually changed)
        let old_effective = Self::current_effective_multiplier(&env);

        let new_signal = CongestionSignal {
            multiplier_bps,
            updated_at: env.ledger().timestamp(),
        };
        set_congestion_signal(&env, &new_signal);

        // Get effective multiplier AFTER setting the new signal
        let new_effective = Self::current_effective_multiplier(&env);

        if old_effective != new_effective {
            emit_effective_multiplier_changed(
                &env,
                EvtEffectiveMultiplierChanged {
                    old_multiplier_bps: old_effective,
                    new_multiplier_bps: new_effective,
                },
            );
        }

        Ok(())
    }

    pub(crate) fn current_effective_multiplier(env: &Env) -> u32 {
        let config = get_congestion_config(env);
        if let Some(signal) = get_congestion_signal(env) {
            let now = env.ledger().timestamp();
            // Check staleness (if updated_at + threshold < now)
            if now.saturating_sub(signal.updated_at) > config.staleness_threshold_secs {
                config.default_multiplier_bps
            } else {
                signal
                    .multiplier_bps
                    .clamp(config.min_multiplier_bps, config.max_multiplier_bps)
            }
        } else {
            config.default_multiplier_bps
        }
    }

    /// Admin: update fee optimization settings for dynamic fee adjustments.
    pub fn set_fee_optimization_config(
        env: Env,
        admin: Address,
        config: FeeOptimizationConfig,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        admin.require_auth();
        if config.max_dynamic_rate_bps < MIN_FEE_RATE_BPS
            || config.max_dynamic_rate_bps > MAX_FEE_RATE_BPS
            || config.congestion_sensitivity_bps > 10_000
            || config.min_effective_rate_bps < MIN_FEE_RATE_BPS
            || config.max_retry_attempts > 10
        {
            return Err(ContractError::InvalidFeeConfiguration);
        }
        set_fee_optimization_config(&env, &config);
        storage::bump_config_version(&env);
        Ok(())
    }

    /// Admin: update the network condition score used for dynamic fee pricing.
    pub fn update_network_conditions(
        env: Env,
        admin: Address,
        score_bps: u32,
        note: String,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        admin.require_auth();
        if score_bps > 10_000 {
            return Err(ContractError::NetworkConditionInvalid);
        }

        set_network_condition_score(&env, score_bps);
        storage::bump_config_version(&env);
        emit_network_condition_updated(
            &env,
            EvtNetworkConditionUpdated {
                score_bps,
                note,
                updated_at: env.ledger().timestamp(),
            },
        );
        Ok(())
    }

    /// Admin: queue a failed fee collection for retry.
    pub fn queue_failed_fee_collection(
        env: Env,
        admin: Address,
        failed: FailedFeeCollection,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        admin.require_auth();
        set_failed_fee_collection(&env, &failed);
        Ok(())
    }

    /// Retry a previously queued fee collection request.
    ///
    /// Uses the shared `stellar_swipe_common::retry_backoff` helper for
    /// exponential backoff and attempt counting (Issue #699).
    pub fn retry_failed_fee_collection(
        env: Env,
        admin: Address,
        id: String,
    ) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        admin.require_auth();

        let failed =
            get_failed_fee_collection(&env, &id).ok_or(ContractError::FailedCollectionNotFound)?;

        let config = get_fee_optimization_config(&env);
        let retry_config = stellar_swipe_common::retry_backoff::RetryConfig {
            max_attempts: config.max_retry_attempts,
            base_delay_ledgers: 5,        // ~25 seconds at 5s/ledger
            max_delay_ledgers: Some(200), // ~16 minutes max
        };
        let retry_state = stellar_swipe_common::retry_backoff::RetryState {
            attempt: failed.retry_count,
        };

        // Use shared helper to check if retry is allowed
        if stellar_swipe_common::retry_backoff::should_retry(&retry_state, &retry_config).is_none()
        {
            return Err(ContractError::RetryLimitExceeded);
        }

        let mut retry_record = failed.clone();
        let next = stellar_swipe_common::retry_backoff::next_retry_state(&retry_state);
        retry_record.retry_count = next.attempt;
        set_failed_fee_collection(&env, &retry_record);

        let result = Self::collect_fee_with_recovery(
            env.clone(),
            retry_record.trader.clone(),
            retry_record.token.clone(),
            retry_record.trade_amount,
            retry_record.trade_asset.clone(),
        );

        emit_retry_attempted(
            &env,
            EvtRetryAttempted {
                id: id.clone(),
                retry_count: retry_record.retry_count,
                successful: result.is_ok(),
                timestamp: env.ledger().timestamp(),
            },
        );

        if result.is_ok() {
            remove_failed_fee_collection(&env, &id);
        }

        result
    }

    /// # Summary
    /// Collect a fee from a trader for a completed trade. Transfers the fee
    /// from the trader to this contract, burns the configured burn slice,
    /// and credits the remainder to the treasury.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `trader`: Address of the trader (must authorize).
    /// - `token`: SEP-41 token used to pay the fee.
    /// - `trade_amount`: Gross trade amount (fee is calculated as a percentage).
    /// - `trade_asset`: Asset pair traded (used for volume tracking).
    ///
    /// # Returns
    /// The total fee amount collected (before burn).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::InvalidAmount`] — trade_amount <= 0.
    /// - [`ContractError::FeeRoundedToZero`] — fee rounds to zero at current rate.
    /// - [`ContractError::ArithmeticOverflow`] — overflow in fee calculation.
    pub fn collect_fee(
        env: Env,
        trader: Address,
        token: Address,
        trade_amount: i128,
        trade_asset: Asset,
    ) -> Result<i128, ContractError> {
        let result = Self::collect_fee_with_recovery(
            env.clone(),
            trader.clone(),
            token.clone(),
            trade_amount,
            trade_asset.clone(),
        );
        if let Err(err) = &result {
            let report = ErrorReport {
                category: ErrorCategory::ExternalDependency,
                strategy: RecoveryStrategy::Retry,
                message: String::from_str(&env, "Fee collection failed, queueing recovery."),
                timestamp: env.ledger().timestamp(),
            };
            set_last_error_report(&env, &report);
            emit_error_reported(
                &env,
                EvtErrorReported {
                    category: report.category,
                    strategy: report.strategy,
                    message: report.message.clone(),
                    timestamp: report.timestamp,
                },
            );
        }
        result
    }

    /// Collect fees for multiple trades in one call. Config is loaded once and reused
    /// across all items via the transaction-scoped fee cache.
    pub fn batch_collect_fees(
        env: Env,
        items: Vec<BatchFeeInput>,
    ) -> Result<Vec<i128>, ContractError> {
        let len = items.len();
        if len == 0 || len > MAX_BATCH_FEE_SIZE {
            return Err(ContractError::InvalidAmount);
        }

        let _cache = fee_cache::load_tx_fee_config(&env);
        let mut results: Vec<i128> = Vec::new(&env);
        for i in 0..len {
            let item = items.get(i).unwrap();
            let fee = Self::collect_fee_with_recovery(
                env.clone(),
                item.trader.clone(),
                item.token.clone(),
                item.trade_amount,
                item.trade_asset.clone(),
            )?;
            results.push_back(fee);
        }
        Ok(results)
    }

    fn collect_fee_with_recovery(
        env: Env,
        trader: Address,
        token: Address,
        trade_amount: i128,
        trade_asset: Asset,
    ) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;
        trader.require_auth();

        if trade_amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        if !has_traded(&env, &trader) {
            set_has_traded(&env, &trader);
            emit_first_trade_fee_waived(&env, &trader);
            let _ = rebates::record_trade_volume(&env, &trader, &trade_asset, trade_amount);
            return Ok(0);
        }

        let fee_cache = fee_cache::load_tx_fee_config(&env);
        let base_rate = rebates::get_fee_rate_for_user(&env, &trader);
        let fee_rate = fee_cache::effective_fee_rate_cached(&env, base_rate, &token, &fee_cache);

        // Compute fee and burn in one pass (issue #633 — fewer multiply operations)
        let burn_rate = fee_cache.burn_rate;
        let (fee_amount, burn_amount) = fee_and_burn_amounts(trade_amount, fee_rate, burn_rate)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if fee_amount <= 0 {
            return Err(ContractError::FeeRoundedToZero);
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&trader, env.current_contract_address(), &fee_amount);

        let distributable = fee_amount
            .checked_sub(burn_amount)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if burn_amount > 0 {
            token_client.burn(&env.current_contract_address(), &burn_amount);
            FeesBurned {
                amount: burn_amount,
                token: token.clone(),
            }
            .publish(&env);
        }

        let mut remaining_distributable = distributable;

        // 1. Referral Share
        if let Some(referrer) = get_referrer(&env, &trader) {
            let referral_bps = get_referral_fee_share_bps(&env);
            let mut referral_amount = fee_amount
                .checked_mul(referral_bps as i128)
                .and_then(|v| v.checked_div(10_000))
                .unwrap_or(0);

            if referral_amount > remaining_distributable {
                referral_amount = remaining_distributable;
            }

            if referral_amount > 0 {
                token_client.transfer(&env.current_contract_address(), &referrer, &referral_amount);
                emit_referral_fee_paid(&env, &referrer, &trader, &token, referral_amount);
                remaining_distributable = remaining_distributable.saturating_sub(referral_amount);
            }
        }

        // 2. Revenue Share
        let revenue_share_rate = storage::get_revenue_share_rate_bps(&env);
        let mut revenue_share_amount = distributable
            .checked_mul(revenue_share_rate as i128)
            .and_then(|v| v.checked_div(10_000))
            .unwrap_or(0);

        if revenue_share_amount > remaining_distributable {
            revenue_share_amount = remaining_distributable;
        }

        let treasury_credit = remaining_distributable.saturating_sub(revenue_share_amount);

        if revenue_share_amount > 0 {
            storage::add_revenue_share_pool(&env, &token, revenue_share_amount);
        }

        let updated_treasury_balance = get_treasury_balance(&env, &token)
            .checked_add(treasury_credit)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_treasury_balance(&env, &token, updated_treasury_balance);

        rebates::record_trade_volume(&env, &trader, &trade_asset, trade_amount)?;

        // #665: Record daily fee total and auto-emit forecast on epoch boundary.
        let current_day = env.ledger().timestamp() / SECONDS_PER_DAY_FC;
        add_daily_fee_total(&env, &token, current_day, fee_amount);
        let forecast_cfg = get_forecast_config(&env);
        let last_forecast_day = get_last_forecast_day(&env, &token);
        if current_day >= last_forecast_day.saturating_add(forecast_cfg.epoch_cadence_days) {
            Self::compute_and_emit_forecast(&env, &token, current_day, &forecast_cfg);
            set_last_forecast_day(&env, &token, current_day);
        }

        emit_fee_collected(
            &env,
            EvtFeeCollected {
                trader: trader.clone(),
                token: token.clone(),
                trade_amount,
                fee_amount,
                fee_rate_bps: fee_rate,
            },
        );

        Ok(fee_amount)
    }

    fn compute_and_emit_forecast(
        env: &Env,
        token: &Address,
        current_day: u64,
        cfg: &ForecastConfigData,
    ) {
        let window = cfg.window_days;
        if window == 0 {
            return;
        }
        let mut total: i128 = 0;
        let mut days_with_data: u64 = 0;
        let mut i: u64 = 0;
        while i < window {
            let day = current_day.saturating_sub(i);
            let daily_total = get_daily_fee_total(env, token, day);
            if daily_total > 0 {
                total = total.saturating_add(daily_total);
                days_with_data = days_with_data.saturating_add(1);
            }
            i = i.saturating_add(1);
        }
        if days_with_data == 0 {
            return;
        }
        let projected = total
            .checked_div(days_with_data as i128)
            .and_then(|avg| avg.checked_mul(cfg.epoch_cadence_days as i128))
            .unwrap_or(0);
        emit_fee_forecast(env, token, projected, window, current_day);
    }

    fn effective_fee_rate_for_trade(
        env: &Env,
        trader: &Address,
        token: &Address,
        _trade_asset: &Asset,
    ) -> Result<u32, ContractError> {
        let cache = fee_cache::load_tx_fee_config(env);
        let base_rate = rebates::get_fee_rate_for_user(env, trader);
        Ok(fee_cache::effective_fee_rate_cached(
            env, base_rate, token, &cache,
        ))
    }

    /// Returns the current dynamic fee rate for a trade after tiered rebates and
    /// network condition adjustments are applied.
    pub fn current_dynamic_fee_rate(
        env: Env,
        trader: Address,
        token: Address,
        trade_asset: Asset,
    ) -> Result<u32, ContractError> {
        Self::effective_fee_rate_for_trade(&env, &trader, &token, &trade_asset)
    }

    /// Claim all pending fee earnings for a provider and token.
    ///
    /// If the provider has set a preferred payout currency via
    /// [`set_payout_currency`] and the preferred token differs from `token`,
    /// this function attempts to convert the accrued fees at claim time using
    /// the configured oracle price feed.  On any conversion failure (stale
    /// oracle, no oracle configured, insufficient treasury balance of the
    /// preferred token) it silently falls back to settling in `token`.
    ///
    /// Authorization is scoped to `(provider, token)` via `require_auth_for_args`
    /// so a valid signature cannot be replayed against a different token or
    /// redirected to a different provider (Issue #563).
    pub fn claim_fees(env: Env, provider: Address, token: Address) -> Result<i128, ContractError> {
        // Issue #859: reentrancy guard for token transfer.
        reentrancy::require_not_locked(&env).map_err(|_| ContractError::Unauthorized)?;
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;
        // Validate registered token metadata before allowing fee claims.
        Self::validate_token_metadata_for_token(&env, &token)?;
        let mut auth_args: Vec<Val> = Vec::new(&env);
        auth_args.push_back(provider.clone().into_val(&env));
        auth_args.push_back(token.clone().into_val(&env));
        provider.require_auth_for_args(auth_args);

        let requested = get_pending_fees(&env, &provider, &token);

        if requested > 0 {
            // ── Issue #940: Per-epoch rebate cap (duplicate: also closes #947) ──
            // Compute capped amount: cap = epoch_fees * max_rebate_bps / 10_000.
            // If the remaining headroom for this epoch is less than the requested
            // amount, scale the claim down proportionally and emit RebateCapApplied.
            // When epoch_fees is zero (epoch just started or no fees yet), bypass
            // the cap to avoid blocking legitimate claims.
            let epoch_day = env.ledger().timestamp() / SECONDS_PER_DAY_FC;
            let epoch_fees = get_daily_fee_total(&env, &token, epoch_day);
            let max_rebate_bps = get_max_rebate_bps(&env);

            let amount = if epoch_fees > 0 {
                let epoch_cap = epoch_fees
                    .checked_mul(max_rebate_bps as i128)
                    .and_then(|v| v.checked_div(10_000))
                    .unwrap_or(i128::MAX);
                let already_distributed = get_epoch_rebate_distributed(&env, &token, epoch_day);
                let remaining_headroom = epoch_cap.saturating_sub(already_distributed);

                if remaining_headroom < requested {
                    // Cap triggered: scale down and emit event.
                    emit_rebate_cap_applied(
                        &env,
                        EvtRebateCapApplied {
                            epoch: epoch_day,
                            requested,
                            distributed: remaining_headroom,
                        },
                    );
                    remaining_headroom
                } else {
                    requested
                }
            } else {
                requested
            };

            // Record how much was distributed this epoch before transfer.
            if amount > 0 && epoch_fees > 0 {
                add_epoch_rebate_distributed(&env, &token, epoch_day, amount);
            }

            // #691 – honour preferred payout currency when set and conversion succeeds.
            let converted = if amount > 0 {
                if let Some(pref_token) = get_provider_payout_currency(&env, &provider) {
                    if pref_token != token {
                        Self::try_claim_in_preferred_currency(
                            &env,
                            &provider,
                            &token,
                            amount,
                            &pref_token,
                        )
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if converted.is_none() {
                // Default: settle in source token.
                if amount > 0 {
                    token::Client::new(&env, &token).transfer(
                        &env.current_contract_address(),
                        &provider,
                        &amount,
                    );
                }
                set_pending_fees(&env, &provider, &token, 0);
                emit_fees_claimed(
                    &env,
                    EvtFeesClaimed {
                        provider: provider.clone(),
                        token: token.clone(),
                        amount,
                    },
                );
            }

            Ok(amount)
        } else {
            emit_fees_claimed(
                &env,
                EvtFeesClaimed {
                    provider: provider.clone(),
                    token: token.clone(),
                    amount: 0,
                },
            );
            Ok(0)
        }
    }

    /// Attempt to convert `source_amount` of `source_token` into `pref_token`
    /// using the oracle, then transfer `pref_token` from the treasury.
    ///
    /// Returns `Some(preferred_amount)` on success, `None` on any failure
    /// (including stale oracle or insufficient preferred-token treasury balance).
    fn try_claim_in_preferred_currency(
        env: &Env,
        provider: &Address,
        source_token: &Address,
        source_amount: i128,
        pref_token: &Address,
    ) -> Option<i128> {
        let oracle_addr = get_oracle_contract(env)?;

        // Ask the oracle for an exchange rate from source_token to pref_token.
        // Method signature expected: get_rate(from: Address, to: Address) -> i128
        // where the rate is scaled by ORACLE_RATE_PRECISION (1_000_000).
        // If the oracle does not support this method or the feed is stale,
        // try_invoke_contract returns an error and we fall back.
        const ORACLE_RATE_PRECISION: i128 = 1_000_000;
        let rate_result = env.try_invoke_contract::<i128, soroban_sdk::Error>(
            &oracle_addr,
            &Symbol::new(env, "get_rate"),
            (source_token.clone(), pref_token.clone()).into_val(env),
        );

        let rate = match rate_result {
            Ok(Ok(r)) if r > 0 => r,
            _ => return None, // oracle stale or unsupported – fall back
        };

        let pref_amount = source_amount
            .checked_mul(rate)?
            .checked_div(ORACLE_RATE_PRECISION)?;

        if pref_amount <= 0 {
            return None;
        }

        // Ensure the contract holds enough of the preferred token.
        let pref_client = token::Client::new(env, pref_token);
        let pref_balance = pref_client.balance(&env.current_contract_address());
        if pref_balance < pref_amount {
            return None;
        }

        // Execute the conversion: zero out source pending fees and pay preferred.
        set_pending_fees(env, provider, source_token, 0);

        pref_client.transfer(&env.current_contract_address(), provider, &pref_amount);

        emit_fees_claimed_converted(
            env,
            provider,
            source_token,
            pref_token,
            source_amount,
            pref_amount,
        );

        Some(pref_amount)
    }

    // ── Issue #366: Provider Earnings Report ─────────────────────────────────

    /// Record fee shares distributed to a provider for the current day.
    ///
    /// Called by the fee distribution system (the admin, or an allowlisted
    /// settlement contract — see [`Self::authorize_caller`]) when allocating
    /// fee shares to a signal provider. Updates the per-day earnings bucket
    /// used by `get_provider_earnings_report`.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::UnauthorizedCaller`] — `caller` is neither the
    ///   admin nor on the authorized-caller allowlist (Issue #813).
    /// - [`ContractError::InvalidAmount`] — `amount` <= 0.
    pub fn record_provider_fee_share(
        env: Env,
        caller: Address,
        provider: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        caller.require_auth();
        if caller != get_admin(&env) && !is_authorized_caller(&env, &caller) {
            return Err(ContractError::UnauthorizedCaller);
        }
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        let day = env.ledger().timestamp() / SECONDS_PER_DAY;
        storage::add_provider_daily_fee_shares(&env, &provider, day, amount);
        Ok(())
    }

    // ── Issue #438: Protocol Token Integration ─────────────────────

    /// Returns the currently configured protocol token address, if any.
    /// When set, token-based fee payments are accepted with a 50% discount.
    /// When not set, only XLM/USDC payments are accepted (current behavior).
    pub fn get_protocol_token(env: Env) -> Option<Address> {
        storage::get_protocol_token(&env)
    }

    /// Admin: set or clear the protocol token address for token-based fee payment.
    /// Pass `None` to clear (revert to XLM/USDC-only mode).
    pub fn set_protocol_token(env: Env, token: Option<Address>) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        if let Some(token_addr) = token {
            storage::set_protocol_token(&env, &token_addr);
        } else {
            // Clear by setting to a zero-address sentinel
            let zero = Address::from_str(
                &env,
                "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
            );
            storage::set_protocol_token(&env, &zero);
        }
        Ok(())
    }

    /// Admin: register a token's metadata for validation in bridge and
    /// fee flows. Rejects tokens with invalid or ambiguous metadata.
    pub fn register_token(
        env: Env,
        admin: Address,
        token: Address,
        symbol: String,
        name: String,
        decimals: u32,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        admin.require_auth();

        let metadata = TokenMetadata {
            symbol: symbol.clone(),
            name: name.clone(),
            decimals,
        };
        validate_token_metadata(&metadata).map_err(|_| ContractError::InvalidTokenMetadata)?;

        storage::set_registered_token_metadata(&env, &token, &metadata);
        Ok(())
    }

    /// Calculate fee with optional protocol token discount.
    /// If the token being used matches the configured protocol token,
    /// a 50% discount is applied (fee_rate is halved).
    fn effective_fee_rate_for_payment(env: &Env, token: &Address) -> u32 {
        let base_rate = storage::get_fee_rate(env);
        if let Some(protocol_token) = storage::get_protocol_token(env) {
            if *token == protocol_token {
                return base_rate / 2; // 50% discount
            }
        }
        base_rate
    }

    /// Validate that a token has registered metadata and that it passes
    /// the protocol's metadata sanity checks (decimals ≤ 18, non-empty
    /// symbol ≤ 12 chars, non-empty name ≤ 64 chars).
    fn validate_token_metadata_for_token(env: &Env, token: &Address) -> Result<(), ContractError> {
        if let Some(metadata) = storage::get_registered_token_metadata(env, token) {
            validate_token_metadata(&metadata).map_err(|_| ContractError::InvalidTokenMetadata)?;
        }
        Ok(())
    }

    // ── Issue #442: Revenue Sharing with Token Holders ──────────────

    /// Get the current revenue share rate in basis points (default: 2000 = 20%).
    pub fn get_revenue_share_rate_bps(env: Env) -> u32 {
        storage::get_revenue_share_rate_bps(&env)
    }

    /// Admin: set the revenue share rate (in basis points).
    pub fn set_revenue_share_rate_bps(env: Env, rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if rate_bps > 10_000 {
            return Err(ContractError::InvalidAmount);
        }
        storage::set_revenue_share_rate_bps(&env, rate_bps);
        Ok(())
    }

    /// Get the accumulated revenue share pool for a given token.
    pub fn get_revenue_share_pool(env: Env, token: Address) -> i128 {
        storage::get_revenue_share_pool(&env, &token)
    }

    /// Admin: trigger a revenue share distribution snapshot.
    /// The accumulated pool for each token is recorded and the pool is reset.
    /// This should be called weekly.
    pub fn trigger_revenue_share_snapshot(
        env: Env,
        caller: Address,
        token: Address,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        caller.require_auth();

        let pool_amount = storage::get_revenue_share_pool(&env, &token);
        if pool_amount > 0 {
            let ledger = env.ledger().sequence() as u64;
            storage::set_last_revenue_share_snapshot(&env, ledger);

            // Emit RevenueShareDistributed event
            events::emit_revenue_share_distributed(&env, &token, pool_amount, ledger);

            // Clear the pool for the next cycle
            storage::clear_revenue_share_pool(&env, &token);
        }

        Ok(())
    }

    /// Returns an earnings report for the provider over the requested period.
    ///
    /// Categories:
    /// - `fee_shares_earned`: from on-chain daily buckets (this contract)
    /// - `stake_rewards_earned`: 0 (StakeVault cross-contract aggregation)
    /// - `subscription_fees_earned`: 0 (UserPortfolio cross-contract aggregation)
    pub fn get_provider_earnings_report(
        env: Env,
        provider: Address,
        period: ReportPeriod,
    ) -> Result<EarningsReport, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(reports::get_provider_earnings_report(
            &env, &provider, period,
        ))
    }

    // ── Issue #690: Fee Distribution Waterfall ───────────────────────────────

    /// Admin: store an ordered waterfall configuration for fee distribution.
    ///
    /// Tiers are processed in ascending `priority` order; tiers with the same
    /// priority value are processed in the order they appear in the Vec.
    pub fn set_waterfall_config(env: Env, config: WaterfallConfig) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        set_waterfall_config_storage(&env, &config);
        Ok(())
    }

    /// Returns the currently configured waterfall, or an error if none has been
    /// configured yet.
    pub fn get_waterfall_config_fn(env: Env) -> Result<WaterfallConfig, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        get_waterfall_config(&env).ok_or(ContractError::WaterfallNotConfigured)
    }

    /// Distribute `total_amount` of `token` from the treasury according to the
    /// configured waterfall, funding higher-priority tiers first.
    ///
    /// Each tier receives up to `target_amount`. When funds are insufficient:
    /// - If remaining >= tier.minimum_amount the tier receives the remaining
    ///   funds and lower tiers receive nothing.
    /// - If remaining < tier.minimum_amount the tier is skipped entirely
    ///   (minimum not met).
    ///
    /// Admin auth required.  Emits [`waterfall_distribution`].
    pub fn distribute_waterfall(
        env: Env,
        token: Address,
        total_amount: i128,
    ) -> Result<Vec<WaterfallTierResult>, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Self::require_not_paused(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();

        if total_amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let config = get_waterfall_config(&env).ok_or(ContractError::WaterfallNotConfigured)?;

        let treasury_bal = get_treasury_balance(&env, &token);
        if treasury_bal < total_amount {
            return Err(ContractError::InsufficientTreasuryBalance);
        }

        // Sort a local index by priority (insertion sort – tier count is small).
        let n = config.tiers.len();
        let mut indices: Vec<u32> = Vec::new(&env);
        let mut i = 0u32;
        while i < n {
            indices.push_back(i);
            i += 1;
        }
        // Bubble sort ascending by priority value (lower = funded first).
        loop {
            let mut swapped = false;
            let m = indices.len();
            let mut j = 0u32;
            while j + 1 < m {
                let ia = indices.get(j).unwrap();
                let ib = indices.get(j + 1).unwrap();
                let pa = config.tiers.get(ia).unwrap().priority;
                let pb = config.tiers.get(ib).unwrap().priority;
                if pa > pb {
                    indices.set(j, ib);
                    indices.set(j + 1, ia);
                    swapped = true;
                }
                j += 1;
            }
            if !swapped {
                break;
            }
        }

        let token_client = token::Client::new(&env, &token);
        let mut remaining = total_amount;
        let mut total_distributed: i128 = 0;
        let mut tier_results: Vec<WaterfallTierResult> = Vec::new(&env);

        let mut k = 0u32;
        while k < indices.len() {
            let tier_idx = indices.get(k).unwrap();
            let tier: WaterfallTier = config.tiers.get(tier_idx).unwrap();

            let allocated = if remaining >= tier.target_amount {
                tier.target_amount
            } else if remaining >= tier.minimum_amount {
                remaining
            } else {
                // Minimum not met – skip this tier.
                tier_results.push_back(WaterfallTierResult {
                    name: tier.name,
                    recipient: tier.recipient,
                    priority: tier.priority,
                    allocated: 0,
                });
                k += 1;
                continue;
            };

            token_client.transfer(&env.current_contract_address(), &tier.recipient, &allocated);
            remaining = remaining
                .checked_sub(allocated)
                .ok_or(ContractError::ArithmeticOverflow)?;
            total_distributed = total_distributed
                .checked_add(allocated)
                .ok_or(ContractError::ArithmeticOverflow)?;

            tier_results.push_back(WaterfallTierResult {
                name: tier.name,
                recipient: tier.recipient,
                priority: tier.priority,
                allocated,
            });

            k += 1;
        }

        // Deduct from treasury balance.
        let new_bal = treasury_bal
            .checked_sub(total_distributed)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_treasury_balance(&env, &token, new_bal);

        emit_waterfall_distribution(&env, &token, total_distributed, &tier_results);

        Ok(tier_results)
    }

    // ── Issue #691: Provider Settlement Currency ─────────────────────────────

    /// Set or update the preferred payout currency for `provider`.
    ///
    /// At [`claim_fees`] time the contract will attempt to convert accrued fees
    /// into `preferred_token` using the configured oracle price feed.  If the
    /// oracle feed is stale or unavailable the claim falls back to the default
    /// settlement token.
    ///
    /// Only the provider themselves may call this.
    pub fn set_payout_currency(
        env: Env,
        provider: Address,
        preferred_token: Address,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        provider.require_auth();
        set_provider_payout_currency(&env, &provider, &preferred_token);
        emit_payout_currency_set(&env, &provider, &preferred_token);
        Ok(())
    }

    /// Clear the payout currency preference for `provider`, reverting to the
    /// default settlement asset.
    ///
    /// Only the provider themselves may call this.
    pub fn clear_payout_currency(env: Env, provider: Address) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        provider.require_auth();
        remove_provider_payout_currency(&env, &provider);
        Ok(())
    }

    /// Returns the preferred payout token for `provider`, or `None` if the
    /// provider has not set a preference.
    pub fn get_payout_currency(env: Env, provider: Address) -> Option<Address> {
        get_provider_payout_currency(&env, &provider)
    }

    // ── Issue #664: Volume-based discount tiers ──────────────────────────────

    /// Admin: set the volume-based discount tier configuration.
    ///
    /// `config.tiers` must contain at least 3 entries sorted (or unsorted) by
    /// `volume_threshold_usd`.  The highest discount for which the user qualifies
    /// is applied at fee-collection time.
    pub fn set_volume_discount_config(
        env: Env,
        admin: Address,
        config: VolumeDiscountConfig,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let stored_admin = get_admin(&env);
        stored_admin.require_auth();
        if stored_admin != admin {
            return Err(ContractError::Unauthorized);
        }
        if config.tiers.len() < 3 {
            return Err(ContractError::InvalidFeeConfiguration);
        }
        // Every tier's eligibility condition and payout must be explicit and
        // bounded (#819). Without this check a tier with a negative
        // `volume_threshold_usd` would vacuously match every trader
        // (`volume_usd >= negative` is always true for `volume_usd >= 0`),
        // silently overriding the intended tiering, and a `discount_bps`
        // above `MAX_FEE_RATE_BPS` or equal to zero is not a meaningful,
        // auditable discount. Rejecting these up front keeps the discount
        // actually applied at fee-collection time deterministic and
        // traceable back to an explicit admin-configured tier.
        for i in 0..config.tiers.len() {
            let tier = config.tiers.get(i).unwrap();
            if tier.volume_threshold_usd < 0 {
                return Err(ContractError::InvalidFeeConfiguration);
            }
            if tier.discount_bps == 0 || tier.discount_bps > MAX_FEE_RATE_BPS {
                return Err(ContractError::InvalidFeeConfiguration);
            }
        }
        let tier_count = config.tiers.len();
        set_volume_discount_config_storage(&env, &config);
        emit_volume_discount_config_updated(&env, tier_count);
        Ok(())
    }

    /// Returns the current admin-configured volume discount tiers, if any.
    pub fn get_volume_discount_config_fn(env: Env) -> Option<VolumeDiscountConfig> {
        get_volume_discount_config(&env)
    }

    // ── Issue #665: Fee revenue forecasting ─────────────────────────────────

    /// Admin: configure the forecast epoch cadence and trailing window.
    pub fn set_forecast_config(
        env: Env,
        admin: Address,
        config: ForecastConfigData,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let stored_admin = get_admin(&env);
        stored_admin.require_auth();
        if stored_admin != admin {
            return Err(ContractError::Unauthorized);
        }
        if config.epoch_cadence_days == 0 || config.window_days == 0 {
            return Err(ContractError::InvalidFeeConfiguration);
        }
        set_forecast_config_storage(&env, &config);
        Ok(())
    }

    /// Returns the current forecast configuration.
    pub fn get_forecast_config_fn(env: Env) -> ForecastConfigData {
        get_forecast_config(&env)
    }

    /// Admin: manually trigger a fee revenue forecast for `token`.
    ///
    /// Computes a trailing-average projection and emits a `fee_forecast` event.
    /// Returns the projected amount for the next epoch.
    pub fn trigger_fee_forecast(
        env: Env,
        admin: Address,
        token: Address,
    ) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let stored_admin = get_admin(&env);
        stored_admin.require_auth();
        if stored_admin != admin {
            return Err(ContractError::Unauthorized);
        }
        let cfg = get_forecast_config(&env);
        let current_day = env.ledger().timestamp() / SECONDS_PER_DAY_FC;
        let window = cfg.window_days;
        let mut total: i128 = 0;
        let mut days_with_data: u64 = 0;
        let mut i: u64 = 0;
        while i < window {
            let day = current_day.saturating_sub(i);
            let daily_total = get_daily_fee_total(&env, &token, day);
            if daily_total > 0 {
                total = total.saturating_add(daily_total);
                days_with_data = days_with_data.saturating_add(1);
            }
            i = i.saturating_add(1);
        }
        let projected = if days_with_data > 0 {
            total
                .checked_div(days_with_data as i128)
                .and_then(|avg| avg.checked_mul(cfg.epoch_cadence_days as i128))
                .unwrap_or(0)
        } else {
            0
        };
        emit_fee_forecast(&env, &token, projected, window, current_day);
        set_last_forecast_day(&env, &token, current_day);
        Ok(projected)
    }

    // ── Referral System ─────────────────────────────────────────────────────────

    /// Register a referral mapping for a trader (referee) to a referrer.
    pub fn register_referral(
        env: Env,
        referrer: Address,
        referee: Address,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        referee.require_auth();

        if referrer == referee {
            return Err(ContractError::SelfReferralNotAllowed);
        }

        if get_referrer(&env, &referee).is_some() {
            return Err(ContractError::ReferralAlreadyRegistered);
        }

        set_referrer(&env, &referee, &referrer);
        emit_referral_registered(&env, &referrer, &referee);
        Ok(())
    }

    /// Admin override to forcibly change a referral mapping.
    pub fn admin_override_referral(
        env: Env,
        admin: Address,
        referrer: Address,
        referee: Address,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let stored_admin = get_admin(&env);
        stored_admin.require_auth();
        if stored_admin != admin {
            return Err(ContractError::Unauthorized);
        }

        if referrer == referee {
            return Err(ContractError::SelfReferralNotAllowed);
        }

        set_referrer(&env, &referee, &referrer);
        emit_referral_registered(&env, &referrer, &referee);
        Ok(())
    }

    /// Admin configuration to set the referral fee share percentage in basis points.
    pub fn set_referral_fee_share(
        env: Env,
        admin: Address,
        share_bps: u32,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let stored_admin = get_admin(&env);
        stored_admin.require_auth();
        if stored_admin != admin {
            return Err(ContractError::Unauthorized);
        }

        if share_bps > 10_000 {
            return Err(ContractError::InvalidFeeConfiguration);
        }

        let old_bps = get_referral_fee_share_bps(&env);
        set_referral_fee_share_bps(&env, share_bps);
        emit_referral_fee_share_updated(&env, old_bps, share_bps, &admin);
        Ok(())
    }

    // ── Issue #940: Fee Rebate Cap (duplicate: also closes #947) ────────────────

    /// Returns the current maximum rebate bps setting.
    /// Defaults to 8000 (80% of epoch fees).
    pub fn get_max_rebate_bps(env: Env) -> u32 {
        get_max_rebate_bps(&env)
    }

    /// Admin: configure the maximum fraction of epoch fees that may be rebated
    /// to providers in a single epoch.
    ///
    /// - `bps` must be in `[0, 10_000]`.
    /// - Default: 8000 (80%).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not yet initialized.
    /// - [`ContractError::Unauthorized`] — caller is not the admin.
    /// - [`ContractError::InvalidFeeConfiguration`] — `bps` exceeds 10_000.
    pub fn set_max_rebate_bps(env: Env, bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if bps > 10_000 {
            return Err(ContractError::InvalidFeeConfiguration);
        }
        set_max_rebate_bps_storage(&env, bps);
        Ok(())
    }

    // ── Issue #862: Health / Readiness ─────────────────────────────────────────

    /// Read-only health probe for monitoring and front-ends (no auth).
    pub fn health_check(env: Env) -> HealthStatus {
        let version = String::from_str(&env, env!("CARGO_PKG_VERSION"));
        if !is_initialized(&env) {
            return health_uninitialized(&env, version);
        }
        let admin = get_admin(&env);
        HealthStatus {
            is_initialized: true,
            is_paused: pausable::is_paused(&env),
            version,
            admin,
            initialized_at: 0,
        }
    }
}
