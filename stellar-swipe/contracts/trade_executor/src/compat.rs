//! Upgrade compatibility shims for deprecated `trade_executor` entry points.
//!
//! # Overview
//! Each shim:
//!
//! 1. Emits a `deprecated` diagnostic event so off-chain monitors can alert
//!    when callers still use the old name.
//! 2. Delegates immediately to the current canonical implementation — no extra
//!    business logic lives here.
//!
//! ## Adding a new shim
//! When a function is renamed or its public signature changes:
//!
//! a) Keep the old name here, call the new canonical function from it.
//! b) Add a `Replaces: <old_name>` note to the new function's doc-comment.
//! c) Add tests in `compat_tests` covering both legacy and current paths.
//!
//! ## Removal policy
//! Shims must be retained for at least **two major contract versions** after
//! they were deprecated.  Record `since_version` when adding; remove on
//! the appropriate milestone.

#![allow(deprecated)]

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Symbol};

use crate::errors::ContractError;

// ---------------------------------------------------------------------------
// Deprecation metadata
// ---------------------------------------------------------------------------

/// A structured payload attached to every `deprecated` diagnostic event.
#[contracttype]
#[derive(Clone, Debug)]
pub struct DeprecationNotice {
    /// The old entry-point name that was invoked.
    pub old_name: Symbol,
    /// The replacement entry-point name callers should switch to.
    pub new_name: Symbol,
    /// The contract version at which the old name was deprecated.
    pub since_version: u32,
}

/// Emit a standardised deprecation-warning event.
///
/// Topic: `("depr", old_name)` — indexers can filter by topic
/// without deserialising the data body.
pub fn emit_deprecation_warning(
    env: &Env,
    old_name: Symbol,
    new_name: Symbol,
    since_version: u32,
) {
    let notice = DeprecationNotice {
        old_name: old_name.clone(),
        new_name,
        since_version,
    };
    env.events()
        .publish((symbol_short!("depr"), old_name), notice);
}

// ---------------------------------------------------------------------------
// Shims – v1 entry points → v2 canonical names
// ---------------------------------------------------------------------------

/// **Deprecated** — use `execute_copy_trade` instead.
///
/// `copy_trade` was the original flat-parameter variant of `execute_copy_trade`
/// before replay-protection parameters and order-type selection were added.
/// The new entry point covers the same market-order path and additionally
/// supports limit orders and nonce-based replay protection.
///
/// This shim sets `order_type = Market` and uses stub replay values that will
/// pass the nonce check in a fresh environment; real callers should migrate to
/// `execute_copy_trade` with explicit replay parameters.
///
/// Deprecated since contract version 2.
///
/// # Migration
/// Replace `copy_trade(user, token, amount, portfolio_pct_bps)` with
/// `execute_copy_trade(user, token, amount, portfolio_pct_bps, Market, None, nonce, tx_hash, expiry_ts)`.
#[deprecated(
    note = "Use `execute_copy_trade` instead. This shim will be removed after v4."
)]
pub fn copy_trade(
    env: &Env,
    user: Address,
    token: Address,
    amount: i128,
    portfolio_pct_bps: Option<u32>,
    nonce: u64,
    tx_hash: Bytes,
    expiry_ts: u64,
) -> Result<(), ContractError> {
    emit_deprecation_warning(
        env,
        symbol_short!("copy_trd"),
        symbol_short!("exec_cpy"),
        2,
    );
    crate::TradeExecutorContract::execute_copy_trade(
        env.clone(),
        user,
        token,
        amount,
        portfolio_pct_bps,
        crate::OrderType::Market,
        None,
        nonce,
        tx_hash,
        expiry_ts,
    )
}

/// **Deprecated** — use `get_trade_receipt` instead.
///
/// `fetch_receipt` was the original name for the trade-receipt getter before
/// it was renamed for consistency.  The return value is unchanged.
///
/// Deprecated since contract version 2.
///
/// # Migration
/// Replace `fetch_receipt(receipt_id)` with `get_trade_receipt(receipt_id)`.
#[deprecated(note = "Use `get_trade_receipt` instead. This shim will be removed after v4.")]
pub fn fetch_receipt(
    env: &Env,
    receipt_id: u64,
) -> Option<soroban_sdk::BytesN<32>> {
    emit_deprecation_warning(
        env,
        symbol_short!("fetch_rc"),
        symbol_short!("get_rcpt"),
        2,
    );
    crate::TradeExecutorContract::get_trade_receipt(env.clone(), receipt_id)
}

/// **Deprecated** — use `simulate_copy_trade` instead.
///
/// `dry_run` was an early name for the trade-simulation entry point before it
/// was renamed to `simulate_copy_trade`.  The semantics are identical.
///
/// Deprecated since contract version 2.
///
/// # Migration
/// Replace `dry_run(user, token, amount, portfolio_pct_bps)` with
/// `simulate_copy_trade(user, token, amount, portfolio_pct_bps)`.
#[deprecated(note = "Use `simulate_copy_trade` instead. This shim will be removed after v4.")]
pub fn dry_run(
    env: &Env,
    user: Address,
    token: Address,
    amount: i128,
    portfolio_pct_bps: Option<u32>,
) -> crate::SimulationResult {
    emit_deprecation_warning(
        env,
        symbol_short!("dry_run"),
        symbol_short!("sim_cpy"),
        2,
    );
    crate::TradeExecutorContract::simulate_copy_trade(
        env.clone(),
        user,
        token,
        amount,
        portfolio_pct_bps,
    )
}

/// **Deprecated** — use `health_check` instead.
///
/// `ping` was an early liveness probe that returned `true` to indicate the
/// contract was deployed.  `health_check` supersedes it with a structured
/// `HealthStatus` response (version, pause state, admin address).
///
/// Deprecated since contract version 2.
///
/// # Migration
/// Replace `ping()` boolean checks with `health_check().is_initialized`.
#[deprecated(note = "Use `health_check` instead. This shim will be removed after v4.")]
pub fn ping(env: &Env) -> bool {
    emit_deprecation_warning(
        env,
        symbol_short!("ping"),
        symbol_short!("health"),
        2,
    );
    crate::TradeExecutorContract::health_check(env.clone()).is_initialized
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod compat_tests {
    use super::*;
    use soroban_sdk::{testutils::Events, Env};

    // ── emit_deprecation_warning ────────────────────────────────────────────

    #[test]
    fn deprecation_event_is_published() {
        let env = Env::default();
        let old = symbol_short!("old_fn");
        let new = symbol_short!("new_fn");

        emit_deprecation_warning(&env, old.clone(), new.clone(), 2);

        let events = env.events().all();
        assert_eq!(events.len(), 1, "expected exactly one deprecation event");
    }

    // ── DeprecationNotice ───────────────────────────────────────────────────

    #[test]
    fn deprecation_notice_fields_match_input() {
        let env = Env::default();
        let old_name = symbol_short!("old_fn");
        let new_name = symbol_short!("new_fn");

        let notice = DeprecationNotice {
            old_name: old_name.clone(),
            new_name: new_name.clone(),
            since_version: 2,
        };

        assert_eq!(notice.since_version, 2);
        assert_eq!(notice.old_name, old_name);
        assert_eq!(notice.new_name, new_name);
    }

    // ── fetch_receipt shim ──────────────────────────────────────────────────

    #[test]
    fn fetch_receipt_returns_none_for_unknown_id_and_emits_warning() {
        let env = Env::default();

        #[allow(deprecated)]
        let result = fetch_receipt(&env, 99_999);

        assert!(result.is_none());

        let events = env.events().all();
        assert!(!events.is_empty(), "deprecation event expected");
    }

    // ── ping shim ───────────────────────────────────────────────────────────

    #[test]
    fn ping_returns_false_when_contract_not_initialized_and_emits_warning() {
        let env = Env::default();

        #[allow(deprecated)]
        let alive = ping(&env);

        // Contract not initialized — health_check returns is_initialized = false.
        assert!(!alive);

        let events = env.events().all();
        assert!(!events.is_empty(), "deprecation event expected");
    }

    // ── dry_run shim ────────────────────────────────────────────────────────

    #[test]
    fn dry_run_shim_emits_deprecation_and_returns_result() {
        let env = Env::default();
        env.mock_all_auths();

        let user = soroban_sdk::Address::generate(&env);
        let token = soroban_sdk::Address::generate(&env);

        #[allow(deprecated)]
        let result = dry_run(&env, user, token, 0, None);

        // Amount is 0 → would_succeed = false (invalid amount gate).
        assert!(!result.would_succeed);

        let events = env.events().all();
        assert!(!events.is_empty(), "deprecation event expected");
    }
}
