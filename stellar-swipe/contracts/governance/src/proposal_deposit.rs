//! Refundable proposal spam-deposit requirement.
//!
//! # Behaviour
//! - Creating a proposal locks `deposit_config.amount` tokens from the proposer.
//! - On finalization:
//!   - If the proposal met the participation/support threshold → deposit is
//!     refunded to the proposer.
//!   - If the proposal failed to meet the threshold → deposit is forfeited to
//!     the treasury.
//! - Deposit amount and the minimum participation threshold are configurable by
//!   admin/governance (`set_deposit_config`).
//!
//! # Integration
//! Call `lock_proposal_deposit` inside `create_proposal` (after auth checks).
//! Call `settle_proposal_deposit` inside `finalize_proposal` once the outcome
//! is known.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

use crate::{add_balance, subtract_balance, GovernanceError, StorageKey};

// ── Config ────────────────────────────────────────────────────────────────────

/// Default spam-deposit amount (in token units, same denomination as balances).
pub const DEFAULT_DEPOSIT_AMOUNT: i128 = 1_000;

/// Default minimum participation threshold in basis-points of total supply
/// (e.g. 500 = 5 %).  Must be met for a deposit refund.
pub const DEFAULT_MIN_PARTICIPATION_BPS: u32 = 500;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepositConfig {
    /// Tokens locked from proposer at proposal creation.
    pub amount: i128,
    /// Minimum participation (total votes / total supply, in bps) required to
    /// trigger a refund rather than a forfeiture.
    pub min_participation_bps: u32,
}

impl Default for DepositConfig {
    fn default() -> Self {
        DepositConfig {
            amount: DEFAULT_DEPOSIT_AMOUNT,
            min_participation_bps: DEFAULT_MIN_PARTICIPATION_BPS,
        }
    }
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DepositKey {
    /// Global deposit configuration.
    Config,
    /// Locked deposit per proposal: (proposal_id) → proposer address.
    LockedDeposit(u64),
}

// ── Config helpers ────────────────────────────────────────────────────────────

pub fn get_deposit_config(env: &Env) -> DepositConfig {
    env.storage()
        .instance()
        .get(&StorageKey::DepositConfig)
        .unwrap_or_default()
}

pub fn set_deposit_config(
    env: &Env,
    admin: &Address,
    config: DepositConfig,
) -> Result<(), GovernanceError> {
    crate::require_admin_pub(env, admin)?;
    if config.amount <= 0 || config.min_participation_bps > 10_000 {
        return Err(GovernanceError::InvalidGovernanceConfig);
    }
    env.storage()
        .instance()
        .set(&StorageKey::DepositConfig, &config);
    Ok(())
}

// ── Deposit lifecycle ─────────────────────────────────────────────────────────

/// Lock the spam-deposit from `proposer` at proposal creation.
///
/// Subtracts `config.amount` from `proposer`'s balance and records that this
/// deposit is held for `proposal_id`.  Returns `InsufficientBalance` if the
/// proposer cannot cover the deposit.
pub fn lock_proposal_deposit(
    env: &Env,
    proposal_id: u64,
    proposer: &Address,
) -> Result<(), GovernanceError> {
    let config = get_deposit_config(env);
    if config.amount == 0 {
        return Ok(()); // deposits disabled
    }
    subtract_balance(env, proposer, config.amount)?;
    env.storage()
        .persistent()
        .set(&DepositKey::LockedDeposit(proposal_id), proposer);
    env.events().publish(
        (symbol_short!("deposit"), symbol_short!("locked")),
        (proposal_id, proposer.clone(), config.amount),
    );
    Ok(())
}

/// Settle the deposit for a finalised proposal.
///
/// `total_votes`  — sum of all votes cast (for + against + abstain).
/// `total_supply` — token total supply at finalization.
/// `treasury`     — treasury address to receive forfeited deposits.
///
/// Refunds if `total_votes / total_supply >= config.min_participation_bps`,
/// otherwise forfeits to treasury.
pub fn settle_proposal_deposit(
    env: &Env,
    proposal_id: u64,
    total_votes: i128,
    total_supply: i128,
    treasury: &Address,
) -> Result<(), GovernanceError> {
    let proposer: Address = match env
        .storage()
        .persistent()
        .get(&DepositKey::LockedDeposit(proposal_id))
    {
        Some(p) => p,
        None => return Ok(()), // no deposit recorded (e.g. amount was 0)
    };

    let config = get_deposit_config(env);

    // Participation = total_votes * 10_000 / total_supply >= min_participation_bps
    let threshold_met = total_supply > 0
        && total_votes
            .saturating_mul(10_000)
            .saturating_div(total_supply)
            >= config.min_participation_bps as i128;

    if threshold_met {
        // Refund to proposer
        add_balance(env, &proposer, config.amount)?;
        env.events().publish(
            (symbol_short!("deposit"), symbol_short!("refund")),
            (proposal_id, proposer, config.amount),
        );
    } else {
        // Forfeit to treasury
        add_balance(env, treasury, config.amount)?;
        env.events().publish(
            (symbol_short!("deposit"), symbol_short!("forfeit")),
            (proposal_id, treasury.clone(), config.amount),
        );
    }

    // Remove the lock record
    env.storage()
        .persistent()
        .remove(&DepositKey::LockedDeposit(proposal_id));

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn make_env() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        // Persistent-storage access (used by lock/settle_proposal_deposit)
        // requires an active contract context in soroban-sdk 23.x, so tests
        // register the real contract and run through `env.as_contract`.
        let contract_id = env.register(crate::GovernanceContract, ());
        env.as_contract(&contract_id, || {
            // Bootstrap minimal storage so governance helpers work.
            env.storage().instance().set(
                &StorageKey::Balances,
                &soroban_sdk::Map::<Address, i128>::new(&env),
            );
            env.storage().instance().set(
                &StorageKey::Holders,
                &soroban_sdk::Vec::<Address>::new(&env),
            );
        });
        let treasury = Address::generate(&env);
        (env, contract_id, treasury)
    }

    #[test]
    fn test_deposit_refunded_when_threshold_met() {
        let (env, contract_id, treasury) = make_env();
        let proposer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            // Give proposer enough balance
            add_balance(&env, &proposer, 10_000).unwrap();

            // Lock deposit
            lock_proposal_deposit(&env, 1, &proposer).unwrap();

            // Balance reduced by deposit amount
            let after_lock = crate::get_balance(&env, &proposer);
            assert_eq!(after_lock, 10_000 - DEFAULT_DEPOSIT_AMOUNT);

            // Finalize: threshold met (100% participation)
            settle_proposal_deposit(&env, 1, 500_000, 500_000, &treasury).unwrap();

            // Deposit refunded
            assert_eq!(crate::get_balance(&env, &proposer), 10_000);
            assert_eq!(crate::get_balance(&env, &treasury), 0);
        });
    }

    #[test]
    fn test_deposit_forfeited_when_threshold_not_met() {
        let (env, contract_id, treasury) = make_env();
        let proposer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            add_balance(&env, &proposer, 10_000).unwrap();
            lock_proposal_deposit(&env, 2, &proposer).unwrap();

            // Finalize: low participation (0.1% < 5% threshold)
            settle_proposal_deposit(&env, 2, 5, 500_000, &treasury).unwrap();

            // Deposit forfeited to treasury
            assert_eq!(
                crate::get_balance(&env, &proposer),
                10_000 - DEFAULT_DEPOSIT_AMOUNT
            );
            assert_eq!(crate::get_balance(&env, &treasury), DEFAULT_DEPOSIT_AMOUNT);
        });
    }
}
