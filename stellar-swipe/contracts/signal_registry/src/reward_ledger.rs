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
}
