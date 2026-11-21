//! Issue #1038: Claimable rewards emission ledger tie-in.
//!
//! Ties reward claim eligibility to a known ledger sequence so reward windows
//! are reproducible and fair. Claim calculations remain stable across repeated
//! evaluations within the same window, and every event references the ledger
//! basis for auditability.

use soroban_sdk::{contracttype, Address, Env, Symbol};

// ── Storage keys ──────────────────────────────────────────────────────────────

/// Storage key for the active reward window snapshot.
#[contracttype]
#[derive(Clone)]
pub enum RewardLedgerKey {
    /// The current reward window definition (ledger anchor + epoch id).
    ActiveWindow,
    /// Per-provider claim record for a given epoch.
    ClaimRecord(Address, u64),
    /// Epoch id from which `provider` is suspended (Issue #1328). Absent
    /// means never suspended, or reinstated.
    SuspendedFromEpoch(Address),
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Defines a reward eligibility window anchored to a specific ledger sequence.
///
/// All claim calculations within the same `epoch_id` use `anchor_ledger` as
/// their snapshot reference, ensuring stable, reproducible results regardless
/// of when within the window a provider calls `claim_rewards`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardWindow {
    /// Monotonic epoch identifier. Incremented each time a new window opens.
    pub epoch_id: u64,
    /// Ledger sequence that anchors this window's eligibility snapshot.
    pub anchor_ledger: u32,
    /// Ledger sequence at which this window closes (inclusive).
    pub close_ledger: u32,
    /// Total reward pool available for distribution in this window.
    pub total_pool: i128,
}

/// Per-provider claim record for a single epoch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRecord {
    pub epoch_id: u64,
    pub provider: Address,
    pub amount_claimed: i128,
    /// Ledger sequence at which the claim was processed.
    pub claimed_at_ledger: u32,
}

/// Emitted when a new reward window is opened (Issue #1038).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtRewardWindowOpened {
    pub epoch_id: u64,
    pub anchor_ledger: u32,
    pub close_ledger: u32,
    pub total_pool: i128,
}

/// Emitted when a provider is suspended from future reward accrual
/// (Issue #1328). Earned `ClaimRecord`s before `from_epoch` are untouched.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtProviderSuspended {
    pub provider: Address,
    pub from_epoch: u64,
}

/// Emitted when a previously suspended provider is reinstated (Issue #1328).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtProviderReinstated {
    pub provider: Address,
}

/// Emitted when an already-earned claim is explicitly forfeited by an
/// authorized admin (Issue #1328). Suspension alone never emits this —
/// forfeiture is a separate, explicit action so it is always auditable and
/// distinguishable from ordinary suspension.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtClaimForfeited {
    pub provider: Address,
    pub epoch_id: u64,
    pub amount: i128,
}

/// Emitted when a provider claims rewards (Issue #1038).
/// References `anchor_ledger` so indexers can verify the snapshot basis.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtRewardClaimed {
    pub epoch_id: u64,
    pub provider: Address,
    pub amount: i128,
    /// The ledger sequence that anchored eligibility for this claim.
    pub anchor_ledger: u32,
    pub claimed_at_ledger: u32,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RewardLedgerError {
    /// No active reward window is open.
    NoActiveWindow,
    /// The current ledger is outside the active window's range.
    WindowClosed,
    /// Provider has already claimed rewards for this epoch.
    AlreadyClaimed,
    /// Claim amount is zero or negative.
    InvalidAmount,
    /// Provider is suspended as of the current epoch; new accrual is blocked.
    ProviderSuspended,
    /// No earned claim record exists to forfeit for this provider/epoch.
    NoClaimToForfeit,
}

// ── Core functions ────────────────────────────────────────────────────────────

/// Open a new reward window anchored to the current ledger sequence.
///
/// `window_duration_ledgers` controls how many ledgers the window stays open.
/// `total_pool` is the reward pool available for this epoch.
pub fn open_reward_window(env: &Env, window_duration_ledgers: u32, total_pool: i128) -> RewardWindow {
    let current_ledger = env.ledger().sequence();
    let epoch_id: u64 = env
        .storage()
        .instance()
        .get(&RewardLedgerKey::ActiveWindow)
        .map(|w: RewardWindow| w.epoch_id.saturating_add(1))
        .unwrap_or(1);

    let window = RewardWindow {
        epoch_id,
        anchor_ledger: current_ledger,
        close_ledger: current_ledger.saturating_add(window_duration_ledgers),
        total_pool,
    };

    env.storage()
        .instance()
        .set(&RewardLedgerKey::ActiveWindow, &window);

    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "reward_window_opened"),
        ),
        EvtRewardWindowOpened {
            epoch_id: window.epoch_id,
            anchor_ledger: window.anchor_ledger,
            close_ledger: window.close_ledger,
            total_pool: window.total_pool,
        },
    );

    window
}

/// Returns the active reward window, if any.
pub fn get_active_window(env: &Env) -> Option<RewardWindow> {
    env.storage()
        .instance()
        .get(&RewardLedgerKey::ActiveWindow)
}

/// Check whether `provider` is eligible to claim in the current window.
///
/// Eligibility requires:
/// 1. An active window exists.
/// 2. The current ledger is within `[anchor_ledger, close_ledger]`.
/// 3. The provider has not already claimed for this epoch.
pub fn check_claim_eligibility(
    env: &Env,
    provider: &Address,
) -> Result<RewardWindow, RewardLedgerError> {
    let window: RewardWindow = env
        .storage()
        .instance()
        .get(&RewardLedgerKey::ActiveWindow)
        .ok_or(RewardLedgerError::NoActiveWindow)?;

    let current_ledger = env.ledger().sequence();
    if current_ledger > window.close_ledger {
        return Err(RewardLedgerError::WindowClosed);
    }

    // Issue #1328: suspension blocks new accrual from the epoch it takes
    // effect in onward. It never looks at or touches `ClaimRecord`s from
    // epochs before `from_epoch` — those stay exactly as earned.
    if let Some(from_epoch) = is_suspended_for(env, provider) {
        if window.epoch_id >= from_epoch {
            return Err(RewardLedgerError::ProviderSuspended);
        }
    }

    if env
        .storage()
        .persistent()
        .has(&RewardLedgerKey::ClaimRecord(provider.clone(), window.epoch_id))
    {
        return Err(RewardLedgerError::AlreadyClaimed);
    }

    Ok(window)
}

/// Record a reward claim for `provider` in the active window.
///
/// Validates eligibility, persists the claim record, and emits an audit event
/// referencing the window's `anchor_ledger` for reproducibility.
pub fn record_claim(
    env: &Env,
    provider: &Address,
    amount: i128,
) -> Result<ClaimRecord, RewardLedgerError> {
    if amount <= 0 {
        return Err(RewardLedgerError::InvalidAmount);
    }

    let window = check_claim_eligibility(env, provider)?;

    let record = ClaimRecord {
        epoch_id: window.epoch_id,
        provider: provider.clone(),
        amount_claimed: amount,
        claimed_at_ledger: env.ledger().sequence(),
    };

    env.storage().persistent().set(
        &RewardLedgerKey::ClaimRecord(provider.clone(), window.epoch_id),
        &record,
    );

    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "reward_claimed"),
        ),
        EvtRewardClaimed {
            epoch_id: window.epoch_id,
            provider: provider.clone(),
            amount,
            anchor_ledger: window.anchor_ledger,
            claimed_at_ledger: record.claimed_at_ledger,
        },
    );

    Ok(record)
}

/// Returns the claim record for `provider` in `epoch_id`, if any.
pub fn get_claim_record(env: &Env, provider: &Address, epoch_id: u64) -> Option<ClaimRecord> {
    env.storage()
        .persistent()
        .get(&RewardLedgerKey::ClaimRecord(provider.clone(), epoch_id))
}

// ── Issue #1328: provider suspension ────────────────────────────────────────
//
// Suspension is deliberately separate from forfeiture:
//
// * [`suspend_provider`] blocks `record_claim`/`check_claim_eligibility` for
//   epochs at or after `from_epoch`, going forward. It does not read, touch,
//   or delete any existing `ClaimRecord` — rewards already earned in earlier
//   epochs remain claimed and intact.
// * [`reinstate_provider`] clears the suspension; eligibility for future
//   epochs is restored, past records are (again) untouched either way.
// * [`forfeit_claim`] is the *only* path that can erase an earned record, and
//   it is a separate, explicit call (authorization for who may call it is
//   enforced by the caller in `lib.rs`) that always emits
//   [`EvtClaimForfeited`] so the forfeiture itself is auditable.

/// Returns the epoch a provider is suspended from, if currently suspended.
pub fn is_suspended_for(env: &Env, provider: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&RewardLedgerKey::SuspendedFromEpoch(provider.clone()))
}

/// Suspend `provider` starting at `from_epoch` (inclusive). Only blocks
/// future accrual/claims from `from_epoch` onward; never modifies past
/// `ClaimRecord`s. Authorization is the caller's responsibility.
pub fn suspend_provider(env: &Env, provider: &Address, from_epoch: u64) {
    env.storage().persistent().set(
        &RewardLedgerKey::SuspendedFromEpoch(provider.clone()),
        &from_epoch,
    );
    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "provider_suspended"),
        ),
        EvtProviderSuspended {
            provider: provider.clone(),
            from_epoch,
        },
    );
}

/// Lift a provider's suspension so future epochs are eligible again. A no-op
/// (still emits the event) if the provider was not suspended.
pub fn reinstate_provider(env: &Env, provider: &Address) {
    env.storage()
        .persistent()
        .remove(&RewardLedgerKey::SuspendedFromEpoch(provider.clone()));
    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "provider_reinstated"),
        ),
        EvtProviderReinstated {
            provider: provider.clone(),
        },
    );
}

/// Explicitly forfeit an already-earned claim record. This is the *only*
/// function in this module that can remove a settled `ClaimRecord` — plain
/// suspension never does. Requires the record to exist; the caller enforces
/// whatever admin/governance authorization forfeiture requires before
/// calling this. Always emits [`EvtClaimForfeited`] for auditability.
pub fn forfeit_claim(
    env: &Env,
    provider: &Address,
    epoch_id: u64,
) -> Result<ClaimRecord, RewardLedgerError> {
    let key = RewardLedgerKey::ClaimRecord(provider.clone(), epoch_id);
    let record: ClaimRecord = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(RewardLedgerError::NoClaimToForfeit)?;

    env.storage().persistent().remove(&key);
    env.events().publish(
        (
            Symbol::new(env, "signal_registry"),
            Symbol::new(env, "claim_forfeited"),
        ),
        EvtClaimForfeited {
            provider: provider.clone(),
            epoch_id,
            amount: record.amount_claimed,
        },
    );
    Ok(record)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, contractimpl, Address, Env};

    // Minimal contract wrapper so storage calls run inside a contract context.
    #[contract]
    struct TestContract;
    #[contractimpl]
    impl TestContract {}

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        let id = env.register(TestContract, ());
        (env, id)
    }

    #[test]
    fn open_window_sets_anchor_to_current_ledger() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(42);
        let window = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
        assert_eq!(window.anchor_ledger, 42);
        assert_eq!(window.close_ledger, 142);
        assert_eq!(window.epoch_id, 1);
    }

    #[test]
    fn claim_eligibility_passes_within_window() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(10);
        env.as_contract(&id, || { open_reward_window(&env, 50, 500_000); });
        let provider = Address::generate(&env);
        let result = env.as_contract(&id, || check_claim_eligibility(&env, &provider));
        assert!(result.is_ok());
    }

    #[test]
    fn claim_eligibility_fails_after_window_closes() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(10);
        env.as_contract(&id, || { open_reward_window(&env, 5, 500_000); });
        env.ledger().set_sequence_number(16);
        let provider = Address::generate(&env);
        let result = env.as_contract(&id, || check_claim_eligibility(&env, &provider));
        assert_eq!(result, Err(RewardLedgerError::WindowClosed));
    }

    #[test]
    fn double_claim_rejected() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        env.as_contract(&id, || { open_reward_window(&env, 100, 1_000_000); });
        let provider = Address::generate(&env);
        env.as_contract(&id, || record_claim(&env, &provider, 100).unwrap());
        let result = env.as_contract(&id, || check_claim_eligibility(&env, &provider));
        assert_eq!(result, Err(RewardLedgerError::AlreadyClaimed));
    }

    #[test]
    fn claim_record_references_anchor_ledger() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(7);
        let window = env.as_contract(&id, || open_reward_window(&env, 50, 1_000_000));
        let provider = Address::generate(&env);
        env.ledger().set_sequence_number(20);
        env.as_contract(&id, || record_claim(&env, &provider, 250).unwrap());
        let rec = env
            .as_contract(&id, || get_claim_record(&env, &provider, window.epoch_id))
            .unwrap();
        assert_eq!(rec.amount_claimed, 250);
        assert_eq!(rec.claimed_at_ledger, 20);
        assert_eq!(window.anchor_ledger, 7);
    }

    #[test]
    fn second_epoch_increments_epoch_id() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        let w1 = env.as_contract(&id, || open_reward_window(&env, 10, 100));
        env.ledger().set_sequence_number(20);
        let w2 = env.as_contract(&id, || open_reward_window(&env, 10, 200));
        assert_eq!(w2.epoch_id, w1.epoch_id + 1);
    }

    // ── Issue #1328: provider suspension ──────────────────────────────────

    #[test]
    fn suspension_preserves_rewards_earned_before_it() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        let w1 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
        let provider = Address::generate(&env);
        // Earns a reward in epoch 1, before suspension exists.
        env.as_contract(&id, || record_claim(&env, &provider, 500).unwrap());

        // Suspend starting at the *next* epoch (2) onward.
        env.as_contract(&id, || suspend_provider(&env, &provider, w1.epoch_id + 1));

        // The earlier, already-earned record is untouched.
        let rec = env
            .as_contract(&id, || get_claim_record(&env, &provider, w1.epoch_id))
            .unwrap();
        assert_eq!(rec.amount_claimed, 500);
    }

    #[test]
    fn suspension_blocks_new_accrual_from_its_effective_epoch() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        let w1 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
        let provider = Address::generate(&env);
        env.as_contract(&id, || suspend_provider(&env, &provider, w1.epoch_id));

        let result = env.as_contract(&id, || check_claim_eligibility(&env, &provider));
        assert_eq!(result, Err(RewardLedgerError::ProviderSuspended));
        let result = env.as_contract(&id, || record_claim(&env, &provider, 100));
        assert_eq!(result, Err(RewardLedgerError::ProviderSuspended));
    }

    #[test]
    fn suspension_before_any_accrual_blocks_first_claim_but_not_record_state() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        let w1 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
        let provider = Address::generate(&env);
        // Suspended immediately, before ever earning anything.
        env.as_contract(&id, || suspend_provider(&env, &provider, w1.epoch_id));
        assert!(env
            .as_contract(&id, || get_claim_record(&env, &provider, w1.epoch_id))
            .is_none());
        assert_eq!(
            env.as_contract(&id, || record_claim(&env, &provider, 100)),
            Err(RewardLedgerError::ProviderSuspended)
        );
    }

    #[test]
    fn reinstated_provider_can_accrue_again() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        let w1 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
        let provider = Address::generate(&env);
        env.as_contract(&id, || suspend_provider(&env, &provider, w1.epoch_id));
        assert_eq!(
            env.as_contract(&id, || check_claim_eligibility(&env, &provider)),
            Err(RewardLedgerError::ProviderSuspended)
        );

        env.as_contract(&id, || reinstate_provider(&env, &provider));
        assert!(env
            .as_contract(&id, || check_claim_eligibility(&env, &provider))
            .is_ok());
        env.as_contract(&id, || record_claim(&env, &provider, 200).unwrap());
        assert_eq!(
            env.as_contract(&id, || get_claim_record(&env, &provider, w1.epoch_id))
                .unwrap()
                .amount_claimed,
            200
        );
    }

    #[test]
    fn forfeiture_is_explicit_and_distinct_from_suspension() {
        let (env, id) = setup();
        env.ledger().set_sequence_number(1);
        let w1 = env.as_contract(&id, || open_reward_window(&env, 100, 1_000_000));
        let provider = Address::generate(&env);
        env.as_contract(&id, || record_claim(&env, &provider, 500).unwrap());

        // Suspension alone never forfeits the earned record.
        env.as_contract(&id, || suspend_provider(&env, &provider, w1.epoch_id + 1));
        assert!(env
            .as_contract(&id, || get_claim_record(&env, &provider, w1.epoch_id))
            .is_some());

        // Forfeiture is a distinct, explicit, auditable action.
        let forfeited =
            env.as_contract(&id, || forfeit_claim(&env, &provider, w1.epoch_id).unwrap());
        assert_eq!(forfeited.amount_claimed, 500);
        assert!(env
            .as_contract(&id, || get_claim_record(&env, &provider, w1.epoch_id))
            .is_none());

        // Forfeiting again (nothing left) is rejected, not a silent no-op.
        assert_eq!(
            env.as_contract(&id, || forfeit_claim(&env, &provider, w1.epoch_id)),
            Err(RewardLedgerError::NoClaimToForfeit)
        );
    }
}
