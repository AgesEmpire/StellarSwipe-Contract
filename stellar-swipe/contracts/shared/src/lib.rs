#![no_std]

/// Shared timestamp-based expiry trait (issue #679).
pub mod expiry;
pub use expiry::Expirable;

pub mod access_control;
/// Cross-contract call allowlist (Issue: cross-contract allowlist security).
pub mod allowlist;
pub use allowlist::{
    add_allowed_contract, get_allowlist, is_contract_allowed, remove_allowed_contract,
    require_allowed_contract, AllowlistError, MAX_ALLOWLIST_SIZE,
};
/// Asset metadata registry (Issue #700). Single source of truth for which
/// assets may be traded — consumed in production by `auto_trade` and
/// `trade_executor` asset-pair validation (Issue #992).
pub mod asset_registry;
/// Capability-based authorization model (Issue #860).
pub mod capabilities;
pub mod multisig;
pub use multisig::{
    approve as multisig_approve, get_config as multisig_get_config,
    get_proposal as multisig_get_proposal, has_approved as multisig_has_approved,
    is_signer as multisig_is_signer, mark_executed as multisig_mark_executed,
    propose as multisig_propose, require_can_execute as multisig_require_can_execute,
    set_config as multisig_set_config, validate_config as multisig_validate_config, MultisigConfig,
    MultisigError, MultisigStorageKey, Proposal, ProposalStatus,
};

/// Cross-contract reentrancy guard (Issue #859).
pub mod reentrancy;

pub mod auth;
pub mod capability;
#[allow(deprecated)]
pub mod cross_contract;
pub mod errors;
/// Canonical event-topic constants (issue #585).
pub mod event_topics;
#[allow(deprecated)]
pub mod events;
/// Shared double-initialization guard (issue #584).
pub mod initializable;
/// Minimum-liquidity threshold guard for pooled-fund withdrawals (issue #591).
pub mod liquidity_pool;
/// Decimal-precision scaling helpers (Issue #562).
pub mod math;
/// Shared emergency-pause state and guard (Issue #561).
pub mod pausable;
/// Generic fixed-window rate limiter (Issue #595).
pub mod rate_limiter;
/// Safe arithmetic helpers for deterministic rounding and overflow safeguards (Issue #861).
pub mod safe_math;
/// Standardized token / cross-contract invocation failure classification (Issue #1001).
pub mod token_error;
#[allow(deprecated)]
pub mod version;

pub use cross_contract::{
    require_sensitive_caller, CrossContractError, CrossContractMessage,
    CrossContractMessageReceiverClient, CrossContractVersionClient, MessageStatus,
    MAX_MESSAGE_SIZE,
};
pub use errors::{ErrorCategory, RecoveryStrategy};
pub use pausable::{is_paused, require_not_paused, set_paused, PausableKey};
pub use token_error::TokenFailure;
pub use version::{ContractKind, VersionError};

pub use capability::{
    delegate_capability, delegation_count, empty_capability_state, get_capability_state,
    put_capability_state, require_capability, revoke_capability, CapabilityDelegation,
    CapabilityError, CapabilityScope, CapabilityState, CapabilityStorageKey,
    MAX_DELEGATIONS_PER_DELEGATOR,
};
pub use errors::{
    emit_error_metadata, make_error_metadata, ErrorMetadata, ERROR_METADATA_SCHEMA_VERSION,
};
pub use events::{emit_replay_envelope, emit_with_replay, next_envelope_id, ReplayEnvelope};
