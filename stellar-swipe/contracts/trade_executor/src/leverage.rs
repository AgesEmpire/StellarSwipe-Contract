use soroban_sdk::{contracttype, Env, Symbol};

pub fn execute_copy_trade_with_leverage(amount: i128, leverage_multiplier: u32) -> (i128, i128) {
    assert!(
        leverage_multiplier >= 1 && leverage_multiplier <= 3,
        "invalid leverage"
    );

    let borrowed = amount * (leverage_multiplier as i128 - 1);
    let total_position = amount + borrowed;

    (total_position, borrowed)
}

pub fn should_liquidate(position_value: i128, borrowed: i128) -> bool {
    position_value < borrowed * 11 / 10
}

// ── Issue #1037: Debt settlement for undercollateralized positions ─────────────

/// Minimum collateral ratio in basis points (110% = 11_000 bps).
/// A position is considered undercollateralized when its ratio falls below this.
pub const MIN_COLLATERAL_RATIO_BPS: u32 = 11_000;

/// Basis-point denominator.
const BPS_DENOM: i128 = 10_000;

/// Outcome of a debt settlement operation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementResult {
    /// Debt amount cleared by this settlement.
    pub debt_cleared: i128,
    /// Reserve balance consumed to cover the shortfall.
    pub reserve_consumed: i128,
    /// Remaining collateral after settlement (may be 0 if fully wiped).
    pub collateral_remaining: i128,
    /// Ledger sequence at which settlement was executed.
    pub settled_at_ledger: u32,
}

/// Emitted when a debt settlement completes (Issue #1037).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvtDebtSettled {
    pub debt_cleared: i128,
    pub reserve_consumed: i128,
    pub collateral_remaining: i128,
    pub settled_at_ledger: u32,
}

/// Compute the collateral ratio in basis points: `(collateral * 10_000) / debt`.
/// Returns `None` when `debt == 0` (no debt ⟹ fully collateralized).
pub fn collateral_ratio_bps(collateral: i128, debt: i128) -> Option<u32> {
    if debt == 0 {
        return None;
    }
    collateral
        .checked_mul(BPS_DENOM)
        .and_then(|v| v.checked_div(debt))
        .map(|r| r as u32)
}

/// Returns `true` when the position is undercollateralized and eligible for settlement.
pub fn is_undercollateralized(collateral: i128, debt: i128) -> bool {
    match collateral_ratio_bps(collateral, debt) {
        None => false, // no debt — healthy
        Some(ratio) => ratio < MIN_COLLATERAL_RATIO_BPS,
    }
}

/// Settle an undercollateralized position.
///
/// # Invariants
/// - `collateral >= 0`, `debt > 0`, `reserve >= 0`.
/// - Validates collateral health before proceeding; returns `None` when the
///   position is not undercollateralized (settlement not needed).
/// - Clears as much debt as possible using available collateral first, then
///   draws from `reserve` for any remaining shortfall, capped at `reserve`.
/// - Reserve draw is bounded: `reserve_consumed <= reserve`.
/// - `debt_cleared + collateral_remaining + reserve_consumed` is deterministic
///   and leaves no mismatched totals.
///
/// Returns `None` when the position is healthy (no settlement needed).
pub fn settle_debt(
    env: &Env,
    collateral: i128,
    debt: i128,
    reserve: i128,
) -> Option<SettlementResult> {
    if !is_undercollateralized(collateral, debt) {
        return None;
    }

    // Apply collateral first.
    let collateral_applied = collateral.min(debt);
    let remaining_debt = debt.saturating_sub(collateral_applied);
    let collateral_remaining = collateral.saturating_sub(collateral_applied);

    // Draw from reserve for any shortfall, capped at available reserve.
    let reserve_consumed = remaining_debt.min(reserve);
    let debt_cleared = collateral_applied.saturating_add(reserve_consumed);

    let result = SettlementResult {
        debt_cleared,
        reserve_consumed,
        collateral_remaining,
        settled_at_ledger: env.ledger().sequence(),
    };

    env.events().publish(
        (
            Symbol::new(env, "trade_executor"),
            Symbol::new(env, "debt_settled"),
        ),
        EvtDebtSettled {
            debt_cleared: result.debt_cleared,
            reserve_consumed: result.reserve_consumed,
            collateral_remaining: result.collateral_remaining,
            settled_at_ledger: result.settled_at_ledger,
        },
    );

    Some(result)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn healthy_position_returns_none() {
        let env = Env::default();
        // collateral = 120, debt = 100 → ratio = 12_000 bps (120%) — healthy
        assert!(settle_debt(&env, 120, 100, 1_000).is_none());
    }

    #[test]
    fn undercollateralized_clears_debt_from_collateral() {
        let env = Env::default();
        // collateral = 100, debt = 100 → ratio = 10_000 bps (100%) — undercollateralized
        let result = settle_debt(&env, 100, 100, 500).unwrap();
        assert_eq!(result.debt_cleared, 100);
        assert_eq!(result.reserve_consumed, 0);
        assert_eq!(result.collateral_remaining, 0);
    }

    #[test]
    fn shortfall_draws_from_reserve() {
        let env = Env::default();
        // collateral = 50, debt = 100 → ratio = 5_000 bps (50%) — undercollateralized
        let result = settle_debt(&env, 50, 100, 500).unwrap();
        assert_eq!(result.debt_cleared, 100); // 50 collateral + 50 reserve
        assert_eq!(result.reserve_consumed, 50);
        assert_eq!(result.collateral_remaining, 0);
    }

    #[test]
    fn reserve_draw_capped_at_available() {
        let env = Env::default();
        // collateral = 0, debt = 100, reserve = 30 — can only clear 30
        let result = settle_debt(&env, 0, 100, 30).unwrap();
        assert_eq!(result.debt_cleared, 30);
        assert_eq!(result.reserve_consumed, 30);
        assert_eq!(result.collateral_remaining, 0);
    }

    #[test]
    fn no_debt_returns_none() {
        let env = Env::default();
        assert!(settle_debt(&env, 100, 0, 500).is_none());
    }
}
