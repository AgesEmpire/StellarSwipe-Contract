//! Kani formal-verification harnesses for StakeVault balance conservation.
//!
//! # Invariant proved
//! For any sequence of `deposit`, `withdraw`, and `slash` operations bounded
//! to [`BOUND`] steps and [`N_STAKERS`] participants, the arithmetic invariant
//! must hold after every step:
//!
//! ```text
//! sum_of_individual_balances == vault_total_tokens
//! ```
//!
//! This mirrors the real contract where `StakesV2` map entries are the source
//! of truth for individual balances and the contract's actual token account
//! balance must equal their sum.
//!
//! # Running
//! Install the Kani toolchain once:
//! ```sh
//! cargo install --locked kani-verifier
//! cargo kani setup
//! ```
//!
//! Then verify all harnesses:
//! ```sh
//! cd stellar-swipe/contracts/stake_vault_kani
//! cargo kani
//! ```
//!
//! Or a single harness:
//! ```sh
//! cargo kani --harness bounded_sequence_preserves_invariant
//! ```
//!
//! # Bounds and limitations
//! - [`N_STAKERS`] = 3 keeps the state space tractable.
//! - [`BOUND`] = 4 operations per bounded-sequence harness.
//! - Symbolic amounts are constrained to `(0, MAX_AMOUNT]` to stay well below
//!   `i128::MAX`, avoiding the `checked_add(..).unwrap_or(i128::MAX)` saturation
//!   path present in the real `deposit_stake`.  That saturation edge case would
//!   break the invariant and is flagged as a known issue (STAKE_VAULT_OVERFLOW).
//! - Slash `tier_bps` is constrained to `[0, 10_000]`, matching the on-chain
//!   `configure_slash_tiers` validation.

/// Number of distinct staker slots modelled.
pub const N_STAKERS: usize = 3;
/// Number of symbolic operations in the bounded-sequence harness.
pub const BOUND: usize = 4;
/// Basis-point denominator for slash computations (100 % = 10 000).
pub const BPS_DENOMINATOR: i128 = 10_000;

// ── Pure arithmetic model ───────────────────────────────────────────────────

/// Pure arithmetic model of StakeVault's balance accounting.
///
/// `vault_total` tracks cumulative net token inflows (deposits minus
/// withdrawals minus slashes) — it mirrors the contract account's actual
/// token balance.  `balances[i]` is the stake balance for staker `i`.
///
/// # Invariant
/// `balances.iter().sum::<i128>() == vault_total`
///
/// The model uses `checked` arithmetic throughout: any operation that would
/// overflow returns `false` and leaves state unchanged, so the invariant is
/// maintained trivially on overflow.  (In the real contract, `deposit_stake`
/// uses `checked_add(..).unwrap_or(i128::MAX)`, which would violate the
/// invariant on extreme values — that is a documented limitation.)
#[derive(Clone, Debug, Default)]
pub struct VaultModel {
    pub balances: [i128; N_STAKERS],
    pub vault_total: i128,
}

impl VaultModel {
    /// Fresh vault: all balances and vault total are zero.
    pub const fn new() -> Self {
        Self {
            balances: [0; N_STAKERS],
            vault_total: 0,
        }
    }

    /// Invariant check: sum of individual balances == vault total.
    pub fn invariant_holds(&self) -> bool {
        let mut sum: i128 = 0;
        for &b in &self.balances {
            sum = match sum.checked_add(b) {
                Some(s) => s,
                None => return false, // overflow → can't verify
            };
        }
        sum == self.vault_total
    }

    /// Deposit `amount` of tokens for staker `idx`.
    ///
    /// Models `deposit_stake`: both the balance entry and the vault total are
    /// updated atomically.  Returns `false` on any overflow or if `amount <= 0`
    /// (real contract returns `NoStake`).
    pub fn deposit(&mut self, idx: usize, amount: i128) -> bool {
        if amount <= 0 || idx >= N_STAKERS {
            return false;
        }
        let new_bal = match self.balances[idx].checked_add(amount) {
            Some(v) => v,
            None => return false,
        };
        let new_total = match self.vault_total.checked_add(amount) {
            Some(v) => v,
            None => return false,
        };
        self.balances[idx] = new_bal;
        self.vault_total = new_total;
        true
    }

    /// Withdraw all stake for staker `idx`.
    ///
    /// Models `do_withdraw`: balance zeroed, vault total decremented.
    /// Returns `false` if balance is zero (real contract returns `NoStake`).
    pub fn withdraw(&mut self, idx: usize) -> bool {
        if idx >= N_STAKERS {
            return false;
        }
        let balance = self.balances[idx];
        if balance == 0 {
            return false;
        }
        self.balances[idx] = 0;
        self.vault_total -= balance; // safe: vault_total >= balance by invariant
        true
    }

    /// Slash staker `idx` by `tier_bps` basis points.
    ///
    /// `slash_amount = max(1, balance * tier_bps / BPS_DENOMINATOR).min(balance)`
    ///
    /// Models `slash_stake`: slash amount burned from vault and deducted from
    /// balance atomically.  Returns `false` if balance is zero.
    pub fn slash(&mut self, idx: usize, tier_bps: i128) -> bool {
        if idx >= N_STAKERS {
            return false;
        }
        let balance = self.balances[idx];
        if balance == 0 {
            return false;
        }
        // Real contract clamps bps to [0, 10_000] via configure_slash_tiers.
        let bps = tier_bps.clamp(0, BPS_DENOMINATOR);
        let slash_amount = ((balance * bps) / BPS_DENOMINATOR).max(1).min(balance);
        self.balances[idx] -= slash_amount;
        self.vault_total -= slash_amount; // safe: slash_amount <= balance <= vault_total
        true
    }
}

// ── Unit tests (run with `cargo test`) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_vault_invariant_holds() {
        assert!(VaultModel::new().invariant_holds());
    }

    #[test]
    fn deposit_then_withdraw_invariant() {
        let mut v = VaultModel::new();
        assert!(v.deposit(0, 1_000_000));
        assert!(v.invariant_holds());
        assert!(v.withdraw(0));
        assert!(v.invariant_holds());
        assert_eq!(v.vault_total, 0);
    }

    #[test]
    fn slash_100_percent_clears_balance() {
        let mut v = VaultModel::new();
        v.deposit(0, 5_000_000);
        v.slash(0, BPS_DENOMINATOR); // 100 %
        assert!(v.invariant_holds());
        assert_eq!(v.balances[0], 0);
        assert_eq!(v.vault_total, 0);
    }

    #[test]
    fn multi_staker_sequence() {
        let mut v = VaultModel::new();
        v.deposit(0, 1_000_000_000);
        v.deposit(1, 500_000_000);
        v.deposit(2, 250_000_000);
        assert!(v.invariant_holds());
        v.slash(1, 3_000); // 30 %
        assert!(v.invariant_holds());
        v.withdraw(0);
        assert!(v.invariant_holds());
        v.slash(2, 500); // 5 %
        assert!(v.invariant_holds());
    }

    #[test]
    fn withdraw_zero_balance_is_noop() {
        let mut v = VaultModel::new();
        assert!(!v.withdraw(0)); // no stake → false
        assert!(v.invariant_holds());
    }

    #[test]
    fn zero_amount_deposit_rejected() {
        let mut v = VaultModel::new();
        assert!(!v.deposit(0, 0));
        assert!(!v.deposit(0, -1));
        assert!(v.invariant_holds());
    }
}

// ── Kani harnesses (run with `cargo kani`) ─────────────────────────────────

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Upper bound for symbolic amounts — well below i128::MAX so no checked_add
    /// saturation can occur within a BOUND-step sequence of N_STAKERS deposits.
    const MAX_AMOUNT: i128 = 1_000_000_000_000_000; // 10^15

    /// Single deposit: invariant holds after one symbolic deposit.
    #[kani::proof]
    fn single_deposit_preserves_invariant() {
        let mut vault = VaultModel::new();
        let amount: i128 = kani::any();
        kani::assume!(amount > 0 && amount <= MAX_AMOUNT);

        vault.deposit(0, amount);
        assert!(
            vault.invariant_holds(),
            "deposit broke the balance invariant"
        );
    }

    /// Deposit then withdraw: invariant holds at both checkpoints.
    #[kani::proof]
    fn withdraw_preserves_invariant() {
        let mut vault = VaultModel::new();
        let amount: i128 = kani::any();
        kani::assume!(amount > 0 && amount <= MAX_AMOUNT);

        assert!(vault.deposit(0, amount));
        assert!(vault.invariant_holds(), "after deposit");

        vault.withdraw(0);
        assert!(vault.invariant_holds(), "after withdraw");
        assert_eq!(vault.vault_total, 0);
    }

    /// Deposit then slash with symbolic tier_bps: invariant holds.
    #[kani::proof]
    fn slash_preserves_invariant() {
        let mut vault = VaultModel::new();
        let amount: i128 = kani::any();
        let tier_bps: i128 = kani::any();
        kani::assume!(amount > 0 && amount <= MAX_AMOUNT);
        kani::assume!(tier_bps >= 0 && tier_bps <= BPS_DENOMINATOR);

        vault.deposit(0, amount);
        vault.slash(0, tier_bps);
        assert!(vault.invariant_holds(), "slash broke the balance invariant");
    }

    /// Two stakers: deposit, slash one, withdraw the other.
    #[kani::proof]
    fn two_staker_sequence_preserves_invariant() {
        let mut vault = VaultModel::new();
        let a0: i128 = kani::any();
        let a1: i128 = kani::any();
        let tier_bps: i128 = kani::any();
        kani::assume!(a0 > 0 && a0 <= MAX_AMOUNT);
        kani::assume!(a1 > 0 && a1 <= MAX_AMOUNT);
        kani::assume!(tier_bps >= 0 && tier_bps <= BPS_DENOMINATOR);

        vault.deposit(0, a0);
        assert!(vault.invariant_holds());
        vault.deposit(1, a1);
        assert!(vault.invariant_holds());
        vault.slash(0, tier_bps);
        assert!(vault.invariant_holds());
        vault.withdraw(1);
        assert!(vault.invariant_holds());
    }

    /// Bounded symbolic sequence: [`BOUND`] operations with symbolic staker
    /// index and operation type.  Invariant asserted after every step.
    ///
    /// The `#[kani::unwind]` bound is BOUND + 1 (loop limit + 1 unroll
    /// required by Kani's bounded model checker).
    #[kani::proof]
    #[kani::unwind(5)]
    fn bounded_sequence_preserves_invariant() {
        let mut vault = VaultModel::new();

        // Seed one staker so withdraw/slash operations have something to act on.
        let seed: i128 = kani::any();
        kani::assume!(seed > 0 && seed <= MAX_AMOUNT);
        vault.deposit(0, seed);
        assert!(vault.invariant_holds(), "after seed deposit");

        for _ in 0..BOUND {
            let op: u8 = kani::any();
            let idx: usize = kani::any();
            let amount: i128 = kani::any();
            let tier_bps: i128 = kani::any();

            kani::assume!(idx < N_STAKERS);
            kani::assume!(amount > 0 && amount <= MAX_AMOUNT);
            kani::assume!(tier_bps >= 0 && tier_bps <= BPS_DENOMINATOR);
            // Prevent vault_total from overflowing during the deposit path.
            kani::assume!(vault.vault_total.checked_add(amount).is_some());

            match op % 3 {
                0 => {
                    vault.deposit(idx, amount);
                }
                1 => {
                    vault.withdraw(idx);
                }
                _ => {
                    vault.slash(idx, tier_bps);
                }
            }

            assert!(
                vault.invariant_holds(),
                "balance conservation invariant violated"
            );
        }
    }
}
