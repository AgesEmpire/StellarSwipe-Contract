use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Map, String, Vec};

use crate::proposals::{get_governance_config, get_proposal, ProposalStatus, ProposalType};
use crate::{get_treasury, require_admin, GovernanceError, StorageKey};

/// State persisted during a shadow-mode upgrade trial period.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowModeState {
    /// Timestamp at which the shadow period ends.
    pub trial_ends_at: u64,
    /// WASM hash of the new logic being evaluated.
    pub new_wasm_hash: Bytes,
    /// Whether the new logic has been promoted to handle all paths.
    pub promoted: bool,
}

fn get_shadow_state(env: &Env) -> Option<ShadowModeState> {
    env.storage().instance().get(&StorageKey::ShadowMode)
}

fn put_shadow_state(env: &Env, state: &ShadowModeState) {
    env.storage().instance().set(&StorageKey::ShadowMode, state);
}

fn clear_shadow_state(env: &Env) {
    env.storage().instance().remove(&StorageKey::ShadowMode);
}

/// Admin-only: begin a shadow-mode trial for a new WASM hash.
///
/// During the trial period, designated read-only entrypoints should invoke
/// `compare_shadow_results` to emit a discrepancy event when old and new logic
/// disagree. Mutating entrypoints are unaffected and continue using the
/// already-promoted logic.
pub fn enter_shadow_mode(
    env: &Env,
    admin: &Address,
    new_wasm_hash: Bytes,
    trial_duration_seconds: u64,
) -> Result<ShadowModeState, GovernanceError> {
    require_admin(env, admin)?;
    if new_wasm_hash.len() != 32 {
        return Err(GovernanceError::InvalidProposal);
    }
    if trial_duration_seconds == 0 {
        return Err(GovernanceError::InvalidDuration);
    }
    let state = ShadowModeState {
        trial_ends_at: env
            .ledger()
            .timestamp()
            .saturating_add(trial_duration_seconds),
        new_wasm_hash,
        promoted: false,
    };
    put_shadow_state(env, &state);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("enter")),
        (admin.clone(), state.trial_ends_at),
    );
    Ok(state)
}

/// Compare two hashed outputs (e.g. a deterministic digest of a query result).
///
/// Both `old_output_hash` and `new_output_hash` should be 32-byte digests of
/// the outputs produced by old and new logic respectively for the same input.
///
/// When the hashes differ a `shadow/discrepancy` event is emitted for
/// monitoring. The discrepancy does **not** affect the value returned to the
/// caller — the caller should always use the result from the promoted logic.
///
/// Returns `true` if the outputs match, `false` if a discrepancy was detected.
pub fn compare_shadow_results(
    env: &Env,
    entrypoint_id: u32,
    old_output_hash: Bytes,
    new_output_hash: Bytes,
) -> bool {
    let state = match get_shadow_state(env) {
        Some(s) => s,
        None => return true,
    };
    // Outside the trial window — shadow comparison is a no-op.
    if state.promoted || env.ledger().timestamp() > state.trial_ends_at {
        return true;
    }
    let matched = old_output_hash == new_output_hash;
    if !matched {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("shadow"), symbol_short!("discrep")),
            (entrypoint_id, old_output_hash, new_output_hash),
        );
    }
    matched
}

/// Return whether the contract is currently in shadow mode (trial active).
pub fn is_in_shadow_mode(env: &Env) -> bool {
    match get_shadow_state(env) {
        Some(s) => !s.promoted && env.ledger().timestamp() <= s.trial_ends_at,
        None => false,
    }
}

/// Admin-only: end the shadow trial and promote the new logic for all paths.
pub fn promote_from_shadow_mode(env: &Env, admin: &Address) -> Result<(), GovernanceError> {
    require_admin(env, admin)?;
    let mut state = get_shadow_state(env).ok_or(GovernanceError::NotInitialized)?;
    state.promoted = true;
    put_shadow_state(env, &state);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("promote")),
        admin.clone(),
    );
    Ok(())
}

/// Admin-only: cancel the shadow trial without promoting.
pub fn cancel_shadow_mode(env: &Env, admin: &Address) -> Result<(), GovernanceError> {
    require_admin(env, admin)?;
    if get_shadow_state(env).is_none() {
        return Err(GovernanceError::NotInitialized);
    }
    clear_shadow_state(env);
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("cancel")),
        admin.clone(),
    );
    Ok(())
}

// ── Issue #797: Structured events from shadow-mode proposal dry-run ─────────

/// Structured record of a `simulate_proposal` dry-run, emitted as an event so
/// off-chain indexers and governance dashboards can build "what-if" analytics
/// for DAO proposals without needing to invoke the contract directly.
///
/// Note on `proposal_id`: this contract's proposal ids are `u64`
/// (`Proposal::id`, `get_proposal`, `execute_proposal`, ...). This struct
/// deliberately keeps `u64` rather than narrowing to `u32` so that two
/// distinct proposals can never collide in an indexer's records — exactly
/// the class of bug this hardening effort exists to remove.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowModeResult {
    pub proposal_id: u64,
    pub success: bool,
    /// Human-readable summary of the state change(s) the proposal would make
    /// if executed right now, as a flat list of `key=value`-style entries
    /// (e.g. `["type=parameter_change", "<param>", "<old>", "<new>"]`).
    /// May be non-empty even when `success` is `false`, to show what the
    /// attempted change would have been.
    pub simulated_state_changes: Vec<String>,
    /// Populated with a human-readable reason whenever `success` is `false`.
    /// Always `None` when `success` is `true`.
    pub failure_reason: Option<String>,
}

/// Read-only dry-run of a proposal's execution outcome.
///
/// Re-checks the same preconditions `execute_proposal` enforces — proposal
/// status, execution delay, execution deadline, and (for treasury spends)
/// sufficient balance — and computes the state diff the proposal would apply
/// if executed right now. Every governance parameter, treasury and feature
/// flag lookup below is a plain `env.storage().instance().get(..)` read;
/// this function never calls `.set(..)` or `.remove(..)` on any storage key,
/// so a shadow-mode invocation can never mutate persistent contract state —
/// callers only ever observe the returned `ShadowModeResult` and the emitted
/// event.
///
/// A `ShadowModeResult` event is emitted unconditionally — on both the
/// success and the failure path — so off-chain subscribers can detect
/// proposals that would fail on execution before the vote even closes.
pub fn simulate_proposal(env: &Env, proposal_id: u64) -> Result<ShadowModeResult, GovernanceError> {
    let proposal = get_proposal(env, proposal_id)?;

    let mut success = true;
    let mut failure_reason: Option<String> = None;

    if proposal.status != ProposalStatus::Succeeded {
        success = false;
        failure_reason = Some(String::from_str(
            env,
            "proposal is not in Succeeded status",
        ));
    }

    let cfg = get_governance_config(env);
    let now = env.ledger().timestamp();
    let ready_at = proposal.voting_ends.saturating_add(cfg.execution_delay);

    if success && now < ready_at {
        success = false;
        failure_reason = Some(String::from_str(env, "execution delay has not elapsed"));
    }

    if success && now > proposal.execution_deadline {
        success = false;
        failure_reason = Some(String::from_str(
            env,
            "proposal execution window has expired",
        ));
    }

    let mut simulated_state_changes: Vec<String> = Vec::new(env);

    match &proposal.proposal_type {
        ProposalType::ParameterChange(key, _expected_current, proposed) => {
            let params: Map<String, i128> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceParameters)
                .unwrap_or_else(|| Map::new(env));
            let current = params.get(key.clone()).unwrap_or(0);
            simulated_state_changes.push_back(String::from_str(env, "type=parameter_change"));
            simulated_state_changes.push_back(key.clone());
            simulated_state_changes.push_back(i128_to_string(env, current));
            simulated_state_changes.push_back(i128_to_string(env, *proposed));
        }
        ProposalType::TreasurySpend(_recipient, amount, spend_asset, _purpose) => {
            let treasury = get_treasury(env);
            let balance = treasury.assets.get(spend_asset.clone()).unwrap_or(0);
            if balance < *amount {
                if success {
                    success = false;
                    failure_reason = Some(String::from_str(
                        env,
                        "insufficient treasury balance for spend",
                    ));
                }
                simulated_state_changes.push_back(String::from_str(
                    env,
                    "type=treasury_spend_insufficient_balance",
                ));
            } else {
                simulated_state_changes.push_back(String::from_str(env, "type=treasury_spend"));
            }
            simulated_state_changes.push_back(i128_to_string(env, balance));
            simulated_state_changes.push_back(i128_to_string(env, balance.saturating_sub(*amount)));
        }
        ProposalType::FeatureToggle(name, enabled) => {
            let features: Map<String, bool> = env
                .storage()
                .instance()
                .get(&StorageKey::GovernanceFeatures)
                .unwrap_or_else(|| Map::new(env));
            let current = features.get(name.clone()).unwrap_or(false);
            simulated_state_changes.push_back(String::from_str(env, "type=feature_toggle"));
            simulated_state_changes.push_back(name.clone());
            simulated_state_changes.push_back(String::from_str(
                env,
                if current { "true" } else { "false" },
            ));
            simulated_state_changes.push_back(String::from_str(
                env,
                if *enabled { "true" } else { "false" },
            ));
        }
        ProposalType::ContractUpgrade(name, _wasm_hash) => {
            simulated_state_changes.push_back(String::from_str(env, "type=contract_upgrade"));
            simulated_state_changes.push_back(name.clone());
        }
        ProposalType::SignalProposal(_) => {
            simulated_state_changes.push_back(String::from_str(env, "type=signal_no_state_change"));
        }
        ProposalType::Custom(_) => {
            simulated_state_changes.push_back(String::from_str(env, "type=custom_no_state_change"));
        }
    }

    let result = ShadowModeResult {
        proposal_id,
        success,
        simulated_state_changes,
        failure_reason,
    };

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("shadow"), symbol_short!("sim_done")),
        result.clone(),
    );

    Ok(result)
}

/// Render an `i128` as a `soroban_sdk::String` without requiring a global
/// allocator (this crate is `#![no_std]` and does not link one). Uses
/// `unsigned_abs` so `i128::MIN` renders correctly instead of overflowing on
/// negation.
fn i128_to_string(env: &Env, value: i128) -> String {
    // "-170141183460469231731687303715884105728" (i128::MIN) is 40 bytes;
    // 48 leaves comfortable headroom.
    let mut buf = [0u8; 48];
    let mut idx = buf.len();
    let negative = value < 0;
    let mut magnitude = value.unsigned_abs();

    if magnitude == 0 {
        idx -= 1;
        buf[idx] = b'0';
    }
    while magnitude > 0 {
        idx -= 1;
        buf[idx] = b'0' + (magnitude % 10) as u8;
        magnitude /= 10;
    }
    if negative {
        idx -= 1;
        buf[idx] = b'-';
    }

    let digits = core::str::from_utf8(&buf[idx..]).expect("ascii digits are always valid utf8");
    String::from_str(env, digits)
}
