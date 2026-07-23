#![no_std]

/// Shared event-emission macro (issue #677).
pub mod emit;
/// Shared input-sanitization helper for string-based metadata fields (issue #676).
pub mod sanitize;
pub use sanitize::{sanitize_string, SanitizeError};

#[allow(deprecated)]
pub mod amm_bridge;
pub mod retry_backoff;
pub mod assets;
/// Checked-arithmetic wrapper for financial amounts (issue #599).
pub mod checked_amount;
pub mod commit_reveal;
pub mod constants;
pub mod emergency;
pub mod health;
#[allow(deprecated)]
pub mod multisig;
pub mod oracle;
/// Structured panic message convention for intentional panics (issue #596).
pub mod panic_codes;
pub mod perf;
#[allow(deprecated)]
pub mod rate_limit;
#[allow(deprecated)]
pub mod replay_protection;
pub mod ttl_manager;
pub mod budget_regression;
/// Generic CRUD helpers to replace per-contract storage boilerplate (Issue #579).
pub mod storage_crud;

pub use amm_bridge::{
    build_fallback_chain, emit_fallback_used, emit_quote_discovered, emit_route_planned,
    min_amount_out_with_slippage, plan_multi_source_route, quote_constant_product,
    quote_from_pool_reserves, rank_quotes_by_price, AmmBridgeError, AmmQuote, AmmRoutePlan,
    AmmRouteSegment, AmmSourceConfig, AmmSourceKind, BPS_DENOMINATOR, FN_GET_BEST_ASK, FN_SWAP,
};
pub use assets::{validate_asset_pair, Asset, AssetPair, AssetPairError};
pub use commit_reveal::{
    constant_time_eq, forfeit_expired, hash_trade_intent, reveal_and_clear, store_commitment,
    verify_commitment, CommitKey, CommitRecord, CommitRevealError,
};
pub use constants::{
    BASIS_POINTS_DENOMINATOR, BASIS_POINTS_DENOMINATOR_I128, CAT_ALL, CAT_SIGNALS, CAT_STAKES,
    CAT_TRADING, LEDGERS_PER_30_DAY_MONTH, LEDGERS_PER_DAY, PLACEHOLDER_ADMIN_STR,
    SECONDS_PER_30_DAY_MONTH, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_WEEK,
    STELLAR_AMOUNT_SCALE,
};
pub use emergency::PauseState;
pub use health::{health_uninitialized, placeholder_admin, HealthStatus};
pub use multisig::{
    approve, cancel, emit_approval_recorded, emit_proposal_approved, emit_proposal_cancelled,
    emit_proposal_created, emit_proposal_executed, emit_timelock_config_updated,
    get_multisig_stats, get_proposal, get_timelock_config, prepare_execution, propose,
    set_timelock_config, store_proposal, validate_signer_config, ApprovalProposal,
    CriticalActionType, MultisigError, MultisigStorageKey, MultisigTimelockConfig, ProposalStatus,
    DEFAULT_ADMIN_TRANSFER_DELAY, DEFAULT_CONFIG_DELAY, DEFAULT_FEE_CHANGE_DELAY,
    DEFAULT_GUARDIAN_DELAY, DEFAULT_PARAMETER_DELAY, DEFAULT_PAUSE_DELAY, DEFAULT_UNPAUSE_DELAY,
    MAX_ACTIVE_PROPOSALS, MAX_SIGNERS,
};
pub use oracle::{
    oracle_price_to_i128, validate_freshness, validate_oracle_price, validate_price_bounds,
    IOracleClient, MockOracleClient, OnChainOracleClient, OracleError, OraclePrice,
    MAX_ORACLE_PRICE, MIN_ORACLE_PRICE,
};
pub use perf::{
    mark_operation, op_batch_execute, op_collect_fee, op_create_signal, op_execute_trade,
    regression_budget_limit, tx_cache_or_compute, BASELINE_AUTO_TRADE_INSTRUCTIONS,
    BASELINE_COPY_TRADE_INSTRUCTIONS, BASELINE_FEE_COLLECT_INSTRUCTIONS,
    BASELINE_SIGNAL_SUBMIT_INSTRUCTIONS, DEFAULT_INSTRUCTION_BUDGET, REGRESSION_BUDGET_PCT,
};
pub use rate_limit::{
    check_rate_limit, record_action, set_config as set_rate_limit_config, ActionType,
    RateLimitConfig, RateLimitError,
};
pub use replay_protection::{
    current_nonce, purge_expired_nonces, verify_and_commit, PendingNonceRecord, ReplayError,
    ReplayKey,
};
pub use retry_backoff::{
    has_remaining_attempts, next_retry_state, should_retry, RetryConfig, RetryState,
};
pub use storage_crud::{
    crud_get, crud_get_or, crud_get_or_default, crud_has, crud_remove, crud_set, StorageTier,
};

#[cfg(test)]
mod storage_key_tests;

/// Shared test harness for simulating ledger time advancement.
/// Gated to test/testutils builds only — zero production overhead.
#[cfg(any(test, feature = "testutils"))]
pub mod test_time;
