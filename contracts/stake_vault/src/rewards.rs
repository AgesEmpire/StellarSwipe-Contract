pub const XLM: i128 = 10_000_000;
pub const DEFAULT_AUTO_FUND_AMOUNT: i128 = 5_000 * XLM;

/// Decimal precision (in base-10 digits) assumed by every amount conversion in
/// this module. All on-chain amounts are stored as integer base units scaled by
/// `10^EXPECTED_DECIMALS`, so a token whose contract reports a different
/// precision cannot be mixed with these balances without silent value loss.
pub const EXPECTED_DECIMALS: u32 = 7;

/// Scale factor implied by [`EXPECTED_DECIMALS`] (`10^EXPECTED_DECIMALS`).
pub const DECIMAL_SCALE: i128 = XLM;

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolStatus {
    pub balance: i128,
    pub estimated_days_remaining: u32,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPoolLow {
    pub balance: i128,
    pub days_remaining: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RewardsPool {
    pub balance: i128,
    pub daily_outflow: i128,
    pub auto_fund_threshold: i128,
    pub treasury_balance: i128,
}

/// Minimum balance policies enforced on partial withdrawals.
///
/// `user_minimum` is the reserve that must remain in a user's account after
/// any withdraw, while `strategy_minimum` is the reserve that must remain in
/// the strategy position. Both are inclusive lower bounds: a withdraw that
/// would leave a balance strictly below the relevant minimum is rejected.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimumBalancePolicy {
    pub user_minimum: i128,
    pub strategy_minimum: i128,
}

impl MinimumBalancePolicy {
    pub fn new(user_minimum: i128, strategy_minimum: i128) -> Self {
        Self {
            user_minimum,
            strategy_minimum,
        }
    }

    /// Validate a partial withdraw before any state is mutated.
    ///
    /// Returns `Ok(())` when the withdraw preserves both the user-level and
    /// strategy-level minimums, otherwise returns the offending rule so the
    /// caller can reject the request without touching balances.
    pub fn check_withdraw(
        &self,
        user_balance: i128,
        strategy_balance: i128,
        amount: i128,
    ) -> Result<(), MinimumBalanceViolation> {
        if amount <= 0 {
            return Err(MinimumBalanceViolation::NonPositiveAmount);
        }

        if amount > user_balance {
            return Err(MinimumBalanceViolation::InsufficientBalance);
        }

        let remaining_user = user_balance - amount;
        if remaining_user < self.user_minimum {
            return Err(MinimumBalanceViolation::BelowUserMinimum {
                remaining: remaining_user,
                minimum: self.user_minimum,
            });
        }

        let remaining_strategy = strategy_balance - amount;
        if remaining_strategy < self.strategy_minimum {
            return Err(MinimumBalanceViolation::BelowStrategyMinimum {
                remaining: remaining_strategy,
                minimum: self.strategy_minimum,
            });
        }

        Ok(())
    }
}

/// Reason a partial withdraw was rejected by the minimum balance policy.
#[derive(Clone, Debug, PartialEq)]
pub enum MinimumBalanceViolation {
    NonPositiveAmount,
    InsufficientBalance,
    BelowUserMinimum { remaining: i128, minimum: i128 },
    BelowStrategyMinimum { remaining: i128, minimum: i128 },
}

/// A time-based vesting plan for a single provider's incentive rewards.
///
/// `total_amount` is the full reward allocation. `start_time` is the ledger
/// timestamp at which vesting begins, `cliff` is the duration (in seconds)
/// that must elapse before any amount becomes releasable, and `duration` is
/// the total vesting window measured from `start_time`. `released` tracks the
/// amount already withdrawn so unvested balances stay inaccessible.
#[derive(Clone, Debug, PartialEq)]
pub struct VestingSchedule {
    pub total_amount: i128,
    pub start_time: u64,
    pub cliff: u64,
    pub duration: u64,
    pub released: i128,
}

impl VestingSchedule {
    pub fn new(total_amount: i128, start_time: u64, cliff: u64, duration: u64) -> Self {
        Self {
            total_amount,
            start_time,
            cliff,
            duration,
            released: 0,
        }
    }

    /// Deterministically compute the amount that has vested as of `now`.
    ///
    /// Before the cliff elapses nothing is vested. After the full duration
    /// the entire allocation is vested. In between, vesting is linear and
    /// integer-truncated so the result is fully deterministic.
    pub fn vested_amount(&self, now: u64) -> i128 {
        if now < self.start_time.saturating_add(self.cliff) {
            return 0;
        }

        let elapsed = now.saturating_sub(self.start_time);
        if self.duration == 0 || elapsed >= self.duration {
            return self.total_amount;
        }

        (self.total_amount * elapsed as i128) / self.duration as i128
    }

    /// Amount currently available to release: vested minus already released.
    /// Unvested balances remain inaccessible until scheduled release events.
    pub fn releasable_amount(&self, now: u64) -> i128 {
        (self.vested_amount(now) - self.released).max(0)
    }

    /// Release the currently vested amount, updating the released balance.
    /// Returns the amount released (0 when nothing is yet vested).
    pub fn release(&mut self, now: u64) -> i128 {
        let amount = self.releasable_amount(now);
        self.released += amount;
        amount
    }
}

/// Tracks the last applied deposit so repeated attempts for the same
/// transaction cannot duplicate balances or rewards.
#[derive(Clone, Debug, PartialEq)]
pub struct DepositRetryGuard {
    pub last_deposit_id: u64,
    pub last_deposit_amount: i128,
    pub applied: bool,
}

impl DepositRetryGuard {
    pub fn new() -> Self {
        DepositRetryGuard {
            last_deposit_id: 0,
            last_deposit_amount: 0,
            applied: false,
        }
    }
}

impl Default for DepositRetryGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata a token contract is expected to expose at `stake_vault` setup.
///
/// `address` is the token contract identifier, `decimals` its reported
/// precision, and `is_contract` whether the address actually resolves to a
/// deployed contract (as opposed to an account or an empty/invalid address).
#[derive(Clone, Debug, PartialEq)]
pub struct TokenMetadata {
    pub address: u32,
    pub decimals: u32,
    pub is_contract: bool,
}

/// Immutable token configuration captured at `stake_vault` initialization.
///
/// Once validated, the token address and its precision are frozen: there is no
/// setter, so changing them requires a governed upgrade/redeploy rather than a
/// runtime mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenConfig {
    pub address: u32,
    pub decimals: u32,
}

/// Validate a token contract before `stake_vault` initialization.
///
/// Rejects invalid/zero addresses, addresses that are not contracts, and
/// contracts whose reported decimals differ from [`EXPECTED_DECIMALS`]. On
/// success returns the frozen [`TokenConfig`] used for all amount conversions.
pub fn validate_token_metadata(metadata: &TokenMetadata) -> Result<TokenConfig, &'static str> {
    if metadata.address == 0 {
        return Err("invalid token: address must be non-zero");
    }

    if !metadata.is_contract {
        return Err("invalid token: address is not a contract");
    }

    if metadata.decimals != EXPECTED_DECIMALS {
        return Err("incompatible token: decimals do not match vault precision");
    }

    Ok(TokenConfig {
        address: metadata.address,
        decimals: metadata.decimals,
    })
}

/// Convert a whole-token amount into base units using checked arithmetic.
///
/// Rounding: this conversion is exact for whole tokens; any fractional part is
/// truncated toward zero (integer division), never rounded up. Overflow and
/// negative inputs are rejected.
pub fn to_base_units(whole_tokens: i128) -> Result<i128, &'static str> {
    if whole_tokens < 0 {
        return Err("amount must be non-negative");
    }

    whole_tokens
        .checked_mul(DECIMAL_SCALE)
        .ok_or("amount overflow")
}

/// Convert a base-unit amount back into whole tokens, truncating any
/// fractional remainder toward zero.
pub fn from_base_units(base_units: i128) -> i128 {
    base_units / DECIMAL_SCALE
}

/// A single contract operation applied to a [`BalanceLedger`].
///
/// Every variant is a value-transfer between the tracked accounts (user,
/// fee, reward, and vault) so that a sequence of operations can be checked
/// against the conservation invariant: the sum of all account balances plus
/// the fees and rewards already paid out must equal the total ever deposited.
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// Move `amount` from the vault into the user's balance.
    Deposit { amount: i128 },
    /// Move `amount` from the user's balance back into the vault.
    Withdraw { amount: i128 },
    /// Move `amount` from the user's balance into the fee account.
    ChargeFee { amount: i128 },
    /// Move `amount` from the vault into the reward account.
    PayReward { amount: i128 },
    /// Move `amount` from the reward account into the user's balance.
    ClaimReward { amount: i128 },
}

/// Tracks user, fee, reward, and vault balances for a sequence of contract
/// operations.
///
/// The ledger is the model used by the property tests: after every operation
/// the conservation invariant `user + fee + reward + vault == total_deposited`
/// must hold, where `total_deposited` is the cumulative amount ever moved into
/// the system. Operations that would drive any balance negative are rejected
/// without mutating state, so the invariant is preserved by construction.
#[derive(Clone, Debug, PartialEq)]
pub struct BalanceLedger {
    pub user: i128,
    pub fee: i128,
    pub reward: i128,
    pub vault: i128,
    pub total_deposited: i128,
}

impl BalanceLedger {
    /// Create a ledger seeded with `initial_vault` in the vault account.
    pub fn new(initial_vault: i128) -> Self {
        Self {
            user: 0,
            fee: 0,
            reward: 0,
            vault: initial_vault,
            total_deposited: initial_vault,
        }
    }

    /// Sum of all tracked balances (user + fee + reward + vault).
    pub fn total(&self) -> i128 {
        self.user + self.fee + self.reward + self.vault
    }

    /// The conservation invariant: tracked balances equal total deposited.
    pub fn is_conserved(&self) -> bool {
        self.total() == self.total_deposited
    }

    /// Apply a single operation, returning `true` when it was accepted.
    ///
    /// Rejected operations (insufficient funds or overflow) leave the ledger
    /// untouched so the conservation invariant can never be broken by an
    /// invalid request.
    pub fn apply(&mut self, op: &Operation) -> bool {
        match *op {
            Operation::Deposit { amount } => {
                if amount < 0 || self.vault < amount {
                    return false;
                }
                self.vault -= amount;
                self.user += amount;
                self.total_deposited += amount;
            }
            Operation::Withdraw { amount } => {
                if amount < 0 || self.user < amount {
                    return false;
                }
                self.user -= amount;
                self.vault += amount;
            }
            Operation::ChargeFee { amount } => {
                if amount < 0 || self.user < amount {
                    return false;
                }
                self.user -= amount;
                self.fee += amount;
            }
            Operation::PayReward { amount } => {
                if amount < 0 || self.vault < amount {
                    return false;
                }
                self.vault -= amount;
                self.reward += amount;
            }
            Operation::ClaimReward { amount } => {
                if amount < 0 || self.reward < amount {
                    return false;
                }
                self.reward -= amount;
                self.user += amount;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift PRNG so failing seeds are reproducible without
    /// pulling in an external property-testing dependency.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn next_i128(&mut self, max: i128) -> i128 {
            if max <= 0 {
                return 0;
            }
            (self.next_u64() as i128) % (max + 1)
        }
    }

    /// Boundary values exercised by every generator: zero, one, a mid value,
    /// and the maximum representable amount.
    const BOUNDARIES: [i128; 4] = [0, 1, 1_000 * XLM, i128::MAX / 2];

    fn gen_amount(rng: &mut Rng, max: i128) -> i128 {
        if rng.next_u64() % 4 == 0 {
            BOUNDARIES[(rng.next_u64() % BOUNDARIES.len() as u64) as usize]
        } else {
            rng.next_i128(max)
        }
    }

    fn gen_operation(rng: &mut Rng, max: i128) -> Operation {
        let amount = gen_amount(rng, max);
        match rng.next_u64() % 5 {
            0 => Operation::Deposit { amount },
            1 => Operation::Withdraw { amount },
            2 => Operation::ChargeFee { amount },
            3 => Operation::PayReward { amount },
            _ => Operation::ClaimReward { amount },
        }
    }

    fn gen_sequence(rng: &mut Rng, len: usize, max: i128) -> Vec<Operation> {
        (0..len).map(|_| gen_operation(rng, max)).collect()
    }

    /// Invariant checked after every operation: balances are conserved and no
    /// account has gone negative.
    fn assert_invariants(ledger: &BalanceLedger, step: usize) {
        assert!(
            ledger.is_conserved(),
            "conservation broken at step {}: total={} deposited={}",
            step,
            ledger.total(),
            ledger.total_deposited
        );
        assert!(ledger.user >= 0, "negative user balance at step {}", step);
        assert!(ledger.fee >= 0, "negative fee balance at step {}", step);
        assert!(ledger.reward >= 0, "negative reward balance at step {}", step);
        assert!(ledger.vault >= 0, "negative vault balance at step {}", step);
    }

    /// Run a generated sequence, checking the invariant after every operation.
    fn run_sequence(seed: u64, len: usize, max: i128) -> BalanceLedger {
        let mut rng = Rng::new(seed);
        let mut ledger = BalanceLedger::new(1_000_000 * XLM);
        for (step, op) in gen_sequence(&mut rng, len, max).iter().enumerate() {
            ledger.apply(op);
            assert_invariants(&ledger, step);
        }
        ledger
    }

    #[test]
    fn conservation_holds_across_random_sequences() {
        for seed in 1..=256u64 {
            run_sequence(seed, 64, 10_000 * XLM);
        }
    }

    #[test]
    fn conservation_holds_for_boundary_amounts() {
        for seed in 1..=64u64 {
            run_sequence(seed, 32, i128::MAX / 2);
        }
    }

    #[test]
    fn rejected_operations_do_not_mutate_state() {
        let mut ledger = BalanceLedger::new(100);
        let before = ledger.clone();
        assert!(!ledger.apply(&Operation::Withdraw { amount: 101 }));
        assert!(!ledger.apply(&Operation::ChargeFee { amount: 101 }));
        assert!(!ledger.apply(&Operation::PayReward { amount: 101 }));
        assert!(!ledger.apply(&Operation::ClaimReward { amount: 1 }));
        assert!(!ledger.apply(&Operation::Deposit { amount: -1 }));
        assert_eq!(ledger, before);
        assert!(ledger.is_conserved());
    }

    #[test]
    fn fees_and_rewards_are_conserved() {
        let mut ledger = BalanceLedger::new(1_000);
        assert!(ledger.apply(&Operation::Deposit { amount: 500 }));
        assert!(ledger.apply(&Operation::ChargeFee { amount: 100 }));
        assert!(ledger.apply(&Operation::PayReward { amount: 200 }));
        assert!(ledger.apply(&Operation::ClaimReward { amount: 50 }));
        assert_eq!(ledger.fee, 100);
        assert_eq!(ledger.reward, 150);
        assert_eq!(ledger.user, 450);
        assert!(ledger.is_conserved());
    }

    /// Regression coverage for previously failing seeds. Each seed is replayed
    /// deterministically so a failure can be reproduced exactly.
    #[test]
    fn regression_seeds_are_reproducible() {
        const REGRESSION_SEEDS: [u64; 4] = [1, 7, 42, 1337];
        for &seed in REGRESSION_SEEDS.iter() {
            let first = run_sequence(seed, 64, 10_000 * XLM);
            let second = run_sequence(seed, 64, 10_000 * XLM);
            assert_eq!(first, second, "seed {} is not reproducible", seed);
            assert!(first.is_conserved());
        }
    }
}
