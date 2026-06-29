//! Real-time TVL tracking across all vaults (Issue #673).
//!
//! Aggregates balances from stake_vault, liquidity_pool, and escrow/holding
//! into a single TVL figure updated incrementally on each deposit/withdrawal.

use soroban_sdk::{contracttype, Env, Symbol};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Source categories contributing to TVL.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TvlSource {
    StakeVault,
    LiquidityPool,
    Escrow,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TvlSnapshot {
    pub stake_vault: i128,
    pub liquidity_pool: i128,
    pub escrow: i128,
    pub total: i128,
    pub last_updated: u64,
}

#[contracttype]
#[derive(Clone)]
enum TvlKey {
    Snapshot,
}

// ── Storage ───────────────────────────────────────────────────────────────────

fn load(env: &Env) -> TvlSnapshot {
    env.storage()
        .instance()
        .get(&TvlKey::Snapshot)
        .unwrap_or(TvlSnapshot {
            stake_vault: 0,
            liquidity_pool: 0,
            escrow: 0,
            total: 0,
            last_updated: 0,
        })
}

fn save(env: &Env, snapshot: &TvlSnapshot) {
    env.storage()
        .instance()
        .set(&TvlKey::Snapshot, snapshot);
}

// ── Incremental updates ───────────────────────────────────────────────────────

/// Apply an incremental delta (positive = deposit, negative = withdrawal)
/// to the specified source and update the aggregate total.
pub fn apply_delta(env: &Env, source: TvlSource, delta: i128) {
    let mut snap = load(env);

    match source {
        TvlSource::StakeVault => {
            snap.stake_vault = snap.stake_vault.saturating_add(delta);
        }
        TvlSource::LiquidityPool => {
            snap.liquidity_pool = snap.liquidity_pool.saturating_add(delta);
        }
        TvlSource::Escrow => {
            snap.escrow = snap.escrow.saturating_add(delta);
        }
    }

    // Recompute total from components to avoid accumulated drift
    snap.total = snap
        .stake_vault
        .saturating_add(snap.liquidity_pool)
        .saturating_add(snap.escrow);
    snap.last_updated = env.ledger().timestamp();

    save(env, &snap);
}

// ── Query ─────────────────────────────────────────────────────────────────────

/// Return the current aggregated TVL snapshot.
pub fn get_protocol_tvl(env: &Env) -> TvlSnapshot {
    load(env)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn setup_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env
    }

    #[test]
    fn test_initial_tvl_is_zero() {
        let env = setup_env();
        let snap = get_protocol_tvl(&env);
        assert_eq!(snap.total, 0);
        assert_eq!(snap.stake_vault, 0);
        assert_eq!(snap.liquidity_pool, 0);
        assert_eq!(snap.escrow, 0);
    }

    #[test]
    fn test_deposit_increases_tvl() {
        let env = setup_env();
        env.ledger().with_mut(|l| l.timestamp = 1000);
        apply_delta(&env, TvlSource::StakeVault, 500);
        let snap = get_protocol_tvl(&env);
        assert_eq!(snap.stake_vault, 500);
        assert_eq!(snap.total, 500);
        assert_eq!(snap.last_updated, 1000);
    }

    #[test]
    fn test_multi_source_aggregation() {
        let env = setup_env();
        apply_delta(&env, TvlSource::StakeVault, 1000);
        apply_delta(&env, TvlSource::LiquidityPool, 2000);
        apply_delta(&env, TvlSource::Escrow, 300);
        let snap = get_protocol_tvl(&env);
        assert_eq!(snap.stake_vault, 1000);
        assert_eq!(snap.liquidity_pool, 2000);
        assert_eq!(snap.escrow, 300);
        assert_eq!(snap.total, 3300);
    }

    #[test]
    fn test_withdrawal_decreases_tvl() {
        let env = setup_env();
        apply_delta(&env, TvlSource::StakeVault, 1000);
        apply_delta(&env, TvlSource::StakeVault, -400);
        let snap = get_protocol_tvl(&env);
        assert_eq!(snap.stake_vault, 600);
        assert_eq!(snap.total, 600);
    }

    #[test]
    fn test_tvl_consistent_across_deposit_withdraw_sequences() {
        let env = setup_env();
        // Series of deposits and withdrawals
        apply_delta(&env, TvlSource::StakeVault, 1000);
        apply_delta(&env, TvlSource::LiquidityPool, 500);
        apply_delta(&env, TvlSource::StakeVault, -200);
        apply_delta(&env, TvlSource::Escrow, 100);
        apply_delta(&env, TvlSource::LiquidityPool, -50);
        apply_delta(&env, TvlSource::Escrow, -30);

        let snap = get_protocol_tvl(&env);
        // stake_vault: 1000 - 200 = 800
        // liquidity_pool: 500 - 50 = 450
        // escrow: 100 - 30 = 70
        // total: 1320
        assert_eq!(snap.stake_vault, 800);
        assert_eq!(snap.liquidity_pool, 450);
        assert_eq!(snap.escrow, 70);
        assert_eq!(snap.total, snap.stake_vault + snap.liquidity_pool + snap.escrow);
        assert_eq!(snap.total, 1320);
    }

    #[test]
    fn test_full_withdrawal_returns_zero() {
        let env = setup_env();
        apply_delta(&env, TvlSource::StakeVault, 500);
        apply_delta(&env, TvlSource::StakeVault, -500);
        let snap = get_protocol_tvl(&env);
        assert_eq!(snap.stake_vault, 0);
        assert_eq!(snap.total, 0);
    }
}
