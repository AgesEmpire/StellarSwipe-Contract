#![no_std]

soroban_sdk::contractmeta!(key = "source_hash", val = env!("SOURCE_HASH"));

mod errors;
pub use errors::ContractError;

mod events;
mod fee_cache;
use events::{
    emit_error_reported, emit_fee_collected, emit_fee_rate_updated, emit_fees_claimed,
    emit_first_trade_fee_waived, emit_network_condition_updated, emit_retry_attempted,
    emit_treasury_withdrawal, emit_withdrawal_queued, EvtErrorReported, EvtFeeCollected,
    EvtFeeRateUpdated, EvtFeesClaimed, EvtNetworkConditionUpdated, EvtRetryAttempted,
    EvtTreasuryWithdrawal, EvtWithdrawalQueued,
};
pub use events::{
    FeeRateUpdated, FeesBurned, FeesClaimed, FirstTradeFeeWaived, TreasuryWithdrawal,
    WithdrawalQueued,
};

mod rebates;

mod reports;
pub use reports::{EarningsLeaderboardEntry, EarningsReport, ReportPeriod};

mod storage;
pub use storage::BalanceMismatch;
use storage::{
    get_admin, get_burn_rate, get_failed_fee_collection, get_fee_optimization_config, get_fee_rate,
    get_last_error_report, get_monthly_trade_volume, get_network_condition_score,
    get_oracle_contract, get_pending_fees, get_queued_withdrawal, get_treasury_balance, has_traded,
    is_initialized, remove_failed_fee_collection, remove_monthly_trade_volume,
    remove_queued_withdrawal, set_admin, set_burn_rate as set_burn_rate_storage,
    set_failed_fee_collection, set_fee_optimization_config, set_fee_rate as set_fee_rate_storage,
    set_has_traded, set_initialized, set_last_error_report, set_monthly_trade_volume,
    set_network_condition_score, set_oracle_contract as set_oracle_contract_storage,
    set_pending_fees, set_queued_withdrawal, set_treasury_balance, ErrorReport,
    FailedFeeCollection, FeeOptimizationConfig, MonthlyTradeVolume, QueuedWithdrawal, StorageKey,
    MAX_BURN_RATE_BPS, MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS,
};

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, String, Vec};

use shared::errors::{ErrorCategory, RecoveryStrategy};
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
        Ok(())
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
        let admin = get_admin(&env);
        admin.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
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
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

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

    /// Returns the current fee rate in basis points.
    pub fn fee_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_fee_rate(&env))
    }

    /// Admin-only: update the fee rate (in basis points).
    /// Validates: MIN_FEE_RATE_BPS <= new_rate_bps <= MAX_FEE_RATE_BPS.
    /// Change takes effect on the next trade — no retroactive application.
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
        Ok(())
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

        if failed.retry_count >= get_fee_optimization_config(&env).max_retry_attempts {
            return Err(ContractError::RetryLimitExceeded);
        }

        let mut retry_record = failed.clone();
        retry_record.retry_count = retry_record.retry_count.saturating_add(1);
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
        let fee_amount =
            fee_amount_floor(trade_amount, fee_rate).ok_or(ContractError::ArithmeticOverflow)?;

        if fee_amount <= 0 {
            return Err(ContractError::FeeRoundedToZero);
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&trader, &env.current_contract_address(), &fee_amount);

        let burn_rate = fee_cache.burn_rate;
        let burn_amount = fee_amount
            .checked_mul(burn_rate as i128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(ContractError::ArithmeticOverflow)?;
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

        let revenue_share_rate = storage::get_revenue_share_rate_bps(&env);
        let revenue_share_amount = distributable
            .checked_mul(revenue_share_rate as i128)
            .and_then(|v| v.checked_div(10_000))
            .unwrap_or(0);
        let treasury_credit = distributable.saturating_sub(revenue_share_amount);

        if revenue_share_amount > 0 {
            storage::add_revenue_share_pool(&env, &token, revenue_share_amount);
        }

        let updated_treasury_balance = get_treasury_balance(&env, &token)
            .checked_add(treasury_credit)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_treasury_balance(&env, &token, updated_treasury_balance);

        rebates::record_trade_volume(&env, &trader, &trade_asset, trade_amount)?;

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
    /// Returns the amount claimed (0 if no pending balance).
    pub fn claim_fees(env: Env, provider: Address, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        provider.require_auth();

        let amount = get_pending_fees(&env, &provider, &token);

        if amount > 0 {
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &provider,
                &amount,
            );
            set_pending_fees(&env, &provider, &token, 0);
        }

        emit_fees_claimed(
            &env,
            EvtFeesClaimed {
                provider: provider.clone(),
                token: token.clone(),
                amount,
            },
        );

        Ok(amount)
    }

    // ── Issue #366: Provider Earnings Report ─────────────────────────────────

    /// Record fee shares distributed to a provider for the current day.
    ///
    /// Called by the fee distribution system when allocating fee shares to a
    /// signal provider. Updates the per-day earnings bucket used by
    /// `get_provider_earnings_report`.
    pub fn record_provider_fee_share(
        env: Env,
        provider: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
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
}
