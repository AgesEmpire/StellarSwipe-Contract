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
/// Asset metadata registry (Issue #700).
#[cfg(any(test, feature = "testutils"))]
pub mod asset_registry;
pub mod multisig;
pub use multisig::{
    approve as multisig_approve, get_config as multisig_get_config,
    get_proposal as multisig_get_proposal, has_approved as multisig_has_approved,
    is_signer as multisig_is_signer, mark_executed as multisig_mark_executed,
    propose as multisig_propose, require_can_execute as multisig_require_can_execute,
    set_config as multisig_set_config, validate_config as multisig_validate_config, MultisigConfig,
    MultisigError, MultisigStorageKey, Proposal, ProposalStatus,
};

pub mod auth;
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
#[allow(deprecated)]
pub mod version;

pub use cross_contract::{
    CrossContractError, CrossContractMessage, CrossContractMessageReceiverClient,
    CrossContractVersionClient, MessageStatus, MAX_MESSAGE_SIZE,
    require_sensitive_caller,
};
pub use errors::{ErrorCategory, RecoveryStrategy};
pub use pausable::{is_paused, require_not_paused, set_paused, PausableKey};
pub use version::{ContractKind, VersionError};
