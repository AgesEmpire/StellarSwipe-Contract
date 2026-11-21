use core::convert::TryFrom;

use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};
use stellar_swipe_common::Asset;

use crate::errors::GovernanceError;
use crate::{checked_add, checked_div, checked_mul, checked_sub};

pub const BPS_DENOMINATOR: i128 = 10_000;

/// Governance-approved spending authorisation for a budget category.
///
/// Before any spend can be executed against a category the admin must call
/// `approve_treasury_budget`, which records the proposal ID that approved the
/// cap, the maximum cumulative amount that may be spent under that approval,
/// and the total already drawn down.  A single category can be re-approved
/// (e.g. each fiscal period) by calling `approve_treasury_budget` again with a
/// new proposal ID and cap; the `total_drawn` counter resets to zero.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetApproval {
    /// The on-chain proposal ID that authorised this cap.
    pub proposal_id: u64,
    /// Human-readable budget category this approval applies to.
    pub category: String,
    /// Maximum cumulative amount that may be spent under this approval.
    pub approved_cap: i128,
    /// Amount already drawn down against this approval.
    pub total_drawn: i128,
    /// Ledger timestamp when this approval was recorded.
    pub approved_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Treasury {
    pub assets: Map<Asset, i128>,
    pub tracked_assets: Vec<Asset>,
    pub total_value_usd: i128,
    pub budgets: Map<String, Budget>,
    pub budget_categories: Vec<String>,
    /// Governance-approved budget caps, keyed by category string.
    pub approved_budgets: Map<String, BudgetApproval>,
    pub recurring_payments: Vec<RecurringPayment>,
    pub spending_history: Vec<TreasurySpend>,
    pub rebalance_targets: Map<Asset, i128>,
    pub last_rebalance: u64,
    pub next_recurring_payment_id: u64,
    pub next_spend_id: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Budget {
    pub category: String,
    pub allocated: i128,
    pub spent: i128,
    pub remaining: i128,
    pub spend_limit: i128,
    pub period_start: u64,
    pub period_end: u64,
    pub auto_renew: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecurringPayment {
    pub id: u64,
    pub recipient: Address,
    pub amount: i128,
    pub asset: Asset,
    pub frequency: u64,
    pub category: String,
    pub purpose: String,
    pub approved_by_proposal: Option<u64>,
    pub last_payment: u64,
    pub end_date: Option<u64>,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasurySpend {
    pub id: u64,
    pub recipient: Address,
    pub amount: i128,
    pub asset: Asset,
    pub category: String,
    pub purpose: String,
    pub approved_by_proposal: Option<u64>,
    pub executed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetReport {
    pub category: String,
    pub allocated: i128,
    pub spent: i128,
    pub remaining: i128,
    pub spend_limit: i128,
    pub utilization_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryReport {
    pub total_assets: u32,
    pub total_value_usd: i128,
    pub active_budgets: u32,
    pub active_recurring_payments: u32,
    pub total_spends: u32,
    pub total_spent: i128,
    pub monthly_burn_rate: i128,
    pub runway_months: u32,
    pub last_rebalance: u64,
    pub budgets: Vec<BudgetReport>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebalanceAction {
    pub asset: Asset,
    pub current_value_usd: i128,
    pub target_value_usd: i128,
    pub delta_value_usd: i128,
    pub target_bps: i128,
}

/// Per-asset holding in the diversification report.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetAllocation {
    pub asset: Asset,
    /// Raw token amount held.
    pub amount: i128,
    /// USD-equivalent value (amount × oracle price).
    pub value_usd: i128,
    /// Share of total treasury value in basis points (0–10 000).
    pub concentration_bps: i128,
}

/// Treasury diversification snapshot returned by `get_treasury_diversification`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryDiversification {
    /// Holdings broken down by asset (zero-balance assets excluded).
    pub allocations: Vec<AssetAllocation>,
    /// Concentration of the single largest holding, in basis points.
    pub largest_holding_bps: i128,
    /// Asset code of the largest holding (empty string when treasury is empty).
    pub largest_asset: Asset,
    /// Total treasury value in USD-equivalent units.
    pub total_value_usd: i128,
}

/// Compute the treasury's balance breakdown and concentration metrics.
///
/// `prices` must supply a USD-equivalent price for every asset with a non-zero
/// balance; assets without a price entry are silently skipped (their value is
/// treated as zero and they are excluded from the output).
///
/// # Errors
/// - [`GovernanceError::ArithmeticOverflow`] — intermediate multiplication overflowed.
pub fn get_diversification(
    env: &Env,
    treasury: &Treasury,
    prices: &soroban_sdk::Map<Asset, i128>,
) -> Result<TreasuryDiversification, GovernanceError> {
    let mut allocations: Vec<AssetAllocation> = Vec::new(env);
    let mut total_value_usd = 0i128;

    // First pass: collect non-zero balances and compute total USD value.
    let mut index = 0;
    while index < treasury.tracked_assets.len() {
        let asset = treasury.tracked_assets.get(index).unwrap();
        let amount = treasury.assets.get(asset.clone()).unwrap_or(0);
        index += 1;

        if amount <= 0 {
            continue;
        }
        let price = match prices.get(asset.clone()) {
            Some(p) if p > 0 => p,
            _ => continue,
        };
        let value_usd = checked_mul(amount, price)?;
        total_value_usd = checked_add(total_value_usd, value_usd)?;
        allocations.push_back(AssetAllocation {
            asset,
            amount,
            value_usd,
            concentration_bps: 0, // filled in second pass
        });
    }

    // Second pass: compute per-asset concentration and find largest holding.
    let mut largest_holding_bps = 0i128;
    let mut largest_asset = Asset {
        code: soroban_sdk::String::from_str(env, ""),
        issuer: None,
    };

    let mut idx = 0;
    while idx < allocations.len() {
        let mut alloc = allocations.get(idx).unwrap();
        if total_value_usd > 0 {
            alloc.concentration_bps = checked_div(
                checked_mul(alloc.value_usd, BPS_DENOMINATOR)?,
                total_value_usd,
            )?;
        }
        if alloc.concentration_bps > largest_holding_bps {
            largest_holding_bps = alloc.concentration_bps;
            largest_asset = alloc.asset.clone();
        }
        allocations.set(idx, alloc);
        idx += 1;
    }

    Ok(TreasuryDiversification {
        allocations,
        largest_holding_bps,
        largest_asset,
        total_value_usd,
    })
}

pub fn empty_treasury(env: &Env) -> Treasury {
    Treasury {
        assets: Map::new(env),
        tracked_assets: Vec::new(env),
        total_value_usd: 0,
        budgets: Map::new(env),
        budget_categories: Vec::new(env),
        approved_budgets: Map::new(env),
        recurring_payments: Vec::new(env),
        spending_history: Vec::new(env),
        rebalance_targets: Map::new(env),
        last_rebalance: 0,
        next_recurring_payment_id: 1,
        next_spend_id: 1,
    }
}

pub fn set_asset_balance(
    env: &Env,
    treasury: &mut Treasury,
    asset: Asset,
    amount: i128,
) -> Result<(), GovernanceError> {
    if amount < 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    track_asset(env, treasury, &asset);
    treasury.assets.set(asset, amount);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_budget(
    env: &Env,
    treasury: &mut Treasury,
    category: String,
    allocated: i128,
    spend_limit: i128,
    period_start: u64,
    period_end: u64,
    auto_renew: bool,
) -> Result<Budget, GovernanceError> {
    if category.is_empty() || allocated <= 0 || spend_limit <= 0 || spend_limit > allocated {
        return Err(GovernanceError::InvalidTreasuryConfig);
    }
    if period_end <= period_start {
        return Err(GovernanceError::InvalidDuration);
    }

    let budget = Budget {
        category: category.clone(),
        allocated,
        spent: 0,
        remaining: allocated,
        spend_limit,
        period_start,
        period_end,
        auto_renew,
    };
    track_category(env, treasury, &category);
    treasury.budgets.set(category, budget.clone());
    Ok(budget)
}

/// Record a governance-approved spending cap for a budget category.
///
/// This must be called (by the admin, referencing the passing proposal) before
/// any `execute_spend` call for that category.  Re-calling for an existing
/// category **replaces** the previous approval and resets `total_drawn` to
/// zero — use this each time a new governance proposal approves a fresh cap.
///
/// # Parameters
/// - `treasury`: Mutable treasury state.
/// - `category`: Budget category string (must already exist as a budget).
/// - `proposal_id`: The governance proposal ID that authorised this cap.
/// - `approved_cap`: Maximum cumulative spend allowed under this approval.
/// - `now`: Current ledger timestamp.
///
/// # Errors
/// - [`GovernanceError::BudgetNotFound`] — `category` has no associated budget.
/// - [`GovernanceError::InvalidAmount`] — `approved_cap` ≤ 0.
/// - [`GovernanceError::BudgetExceeded`] — cap exceeds the budget's `allocated`.
pub fn approve_budget(
    env: &Env,
    treasury: &mut Treasury,
    category: String,
    proposal_id: u64,
    approved_cap: i128,
    now: u64,
) -> Result<BudgetApproval, GovernanceError> {
    if approved_cap <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    let budget = treasury
        .budgets
        .get(category.clone())
        .ok_or(GovernanceError::BudgetNotFound)?;
    if approved_cap > budget.allocated {
        return Err(GovernanceError::BudgetExceeded);
    }

    let approval = BudgetApproval {
        proposal_id,
        category: category.clone(),
        approved_cap,
        total_drawn: 0,
        approved_at: now,
    };
    treasury
        .approved_budgets
        .set(category.clone(), approval.clone());

    #[allow(deprecated)]
    env.events().publish(
        (
            soroban_sdk::symbol_short!("treasury"),
            soroban_sdk::symbol_short!("budgapprv"),
        ),
        (category, proposal_id, approved_cap, now),
    );

    Ok(approval)
}

#[allow(clippy::too_many_arguments)]
pub fn execute_spend(
    env: &Env,
    treasury: &mut Treasury,
    recipient: Address,
    amount: i128,
    asset: Asset,
    category: String,
    purpose: String,
    approved_by_proposal: Option<u64>,
    executed_at: u64,
) -> Result<TreasurySpend, GovernanceError> {
    if purpose.is_empty() {
        return Err(GovernanceError::InvalidTreasuryConfig);
    }
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }

    // ── Budget period + limit check ──────────────────────────────────────────
    let mut budget = treasury
        .budgets
        .get(category.clone())
        .ok_or(GovernanceError::BudgetNotFound)?;
    renew_budget_if_needed(&mut budget, executed_at)?;

    if amount > budget.remaining || amount > budget.spend_limit {
        return Err(GovernanceError::BudgetExceeded);
    }

    // ── Governance approval cap check ────────────────────────────────────────
    let mut approval = treasury
        .approved_budgets
        .get(category.clone())
        .ok_or(GovernanceError::BudgetApprovalRequired)?;

    let new_drawn = checked_add(approval.total_drawn, amount)?;
    if new_drawn > approval.approved_cap {
        return Err(GovernanceError::ApprovedCapExceeded);
    }

    // ── Asset balance check ──────────────────────────────────────────────────
    let current_balance = treasury.assets.get(asset.clone()).unwrap_or(0);
    if current_balance < amount {
        return Err(GovernanceError::InsufficientBalance);
    }

    // ── Commit all state changes ──────────────────────────────────────────────
    approval.total_drawn = new_drawn;
    treasury
        .approved_budgets
        .set(category.clone(), approval.clone());

    budget.spent = checked_add(budget.spent, amount)?;
    budget.remaining = checked_sub(budget.remaining, amount)?;
    treasury.budgets.set(category.clone(), budget);
    treasury
        .assets
        .set(asset.clone(), checked_sub(current_balance, amount)?);

    let spend = TreasurySpend {
        id: treasury.next_spend_id,
        recipient: recipient.clone(),
        amount,
        asset: asset.clone(),
        category: category.clone(),
        purpose: purpose.clone(),
        approved_by_proposal,
        executed_at,
    };
    treasury.next_spend_id = treasury.next_spend_id.saturating_add(1);
    treasury.spending_history.push_back(spend.clone());

    // ── Emit spend event ─────────────────────────────────────────────────────
    #[allow(deprecated)]
    env.events().publish(
        (
            soroban_sdk::symbol_short!("treasury"),
            soroban_sdk::symbol_short!("spend"),
        ),
        (
            spend.id,
            recipient,
            amount,
            category,
            approved_by_proposal,
            approval.proposal_id,
            approval.approved_cap,
            new_drawn,
            executed_at,
        ),
    );

    Ok(spend)
}

#[allow(clippy::too_many_arguments)]
pub fn schedule_recurring_payment(
    env: &Env,
    treasury: &mut Treasury,
    recipient: Address,
    amount: i128,
    asset: Asset,
    frequency: u64,
    category: String,
    purpose: String,
    approved_by_proposal: Option<u64>,
    end_date: Option<u64>,
) -> Result<RecurringPayment, GovernanceError> {
    if amount <= 0 {
        return Err(GovernanceError::InvalidAmount);
    }
    if frequency == 0 {
        return Err(GovernanceError::InvalidDuration);
    }
    if end_date.is_some() && end_date.unwrap() <= env.ledger().timestamp() {
        return Err(GovernanceError::InvalidDuration);
    }
    if !treasury.budgets.contains_key(category.clone()) {
        return Err(GovernanceError::BudgetNotFound);
    }
    // Require a governance-approved cap before recurring payments can be scheduled.
    if !treasury.approved_budgets.contains_key(category.clone()) {
        return Err(GovernanceError::BudgetApprovalRequired);
    }

    track_asset(env, treasury, &asset);

    let payment = RecurringPayment {
        id: treasury.next_recurring_payment_id,
        recipient,
        amount,
        asset,
        frequency,
        category,
        purpose,
        approved_by_proposal,
        last_payment: env.ledger().timestamp(),
        end_date,
        active: true,
    };
    treasury.next_recurring_payment_id = treasury.next_recurring_payment_id.saturating_add(1);
    treasury.recurring_payments.push_back(payment.clone());
    Ok(payment)
}

pub fn process_recurring_payments(
    env: &Env,
    treasury: &mut Treasury,
    now: u64,
) -> Result<u32, GovernanceError> {
    let mut processed = 0u32;
    let mut index = 0;

    while index < treasury.recurring_payments.len() {
        let mut payment = treasury.recurring_payments.get(index).unwrap();

        if !payment.active {
            index += 1;
            continue;
        }

        if let Some(end_date) = payment.end_date {
            if now > end_date {
                payment.active = false;
                treasury.recurring_payments.set(index, payment);
                index += 1;
                continue;
            }
        }

        if now >= payment.last_payment.saturating_add(payment.frequency) {
            match execute_spend(
                env,
                treasury,
                payment.recipient.clone(),
                payment.amount,
                payment.asset.clone(),
                payment.category.clone(),
                payment.purpose.clone(),
                payment.approved_by_proposal,
                now,
            ) {
                Ok(_) => {
                    payment.last_payment = now;
                    if let Some(end_date) = payment.end_date {
                        if now >= end_date {
                            payment.active = false;
                        }
                    }
                    treasury.recurring_payments.set(index, payment);
                    processed = processed.saturating_add(1);
                }
                Err(
                    GovernanceError::BudgetExceeded
                    | GovernanceError::BudgetNotFound
                    | GovernanceError::BudgetPeriodEnded
                    | GovernanceError::InsufficientBalance
                    | GovernanceError::BudgetApprovalRequired
                    | GovernanceError::ApprovedCapExceeded,
                ) => {
                    payment.active = false;
                    treasury.recurring_payments.set(index, payment);
                }
                Err(error) => return Err(error),
            }
        }

        index += 1;
    }

    Ok(processed)
}

pub fn set_rebalance_target(
    env: &Env,
    treasury: &mut Treasury,
    asset: Asset,
    target_bps: i128,
) -> Result<(), GovernanceError> {
    if !(0..=BPS_DENOMINATOR).contains(&target_bps) {
        return Err(GovernanceError::InvalidTreasuryConfig);
    }

    track_asset(env, treasury, &asset);
    treasury.rebalance_targets.set(asset, target_bps);

    if total_target_bps(treasury)? > BPS_DENOMINATOR {
        return Err(GovernanceError::InvalidTreasuryConfig);
    }

    Ok(())
}

pub fn rebalance(
    treasury: &mut Treasury,
    prices: Map<Asset, i128>,
    now: u64,
    env: &Env,
) -> Result<Vec<RebalanceAction>, GovernanceError> {
    let mut actions = Vec::new(env);
    let total_target = total_target_bps(treasury)?;
    if total_target > BPS_DENOMINATOR {
        return Err(GovernanceError::InvalidTreasuryConfig);
    }

    let mut total_value_usd = 0i128;
    let mut index = 0;
    while index < treasury.tracked_assets.len() {
        let asset = treasury.tracked_assets.get(index).unwrap();
        let amount = treasury.assets.get(asset.clone()).unwrap_or(0);
        if amount > 0 {
            let price = prices
                .get(asset.clone())
                .ok_or(GovernanceError::MissingAssetPrice)?;
            if price < 0 {
                return Err(GovernanceError::InvalidTreasuryConfig);
            }
            total_value_usd = checked_add(total_value_usd, checked_mul(amount, price)?)?;
        }
        index += 1;
    }

    let mut action_index = 0;
    while action_index < treasury.tracked_assets.len() {
        let asset = treasury.tracked_assets.get(action_index).unwrap();
        let amount = treasury.assets.get(asset.clone()).unwrap_or(0);
        let current_value_usd = if amount > 0 {
            let price = prices
                .get(asset.clone())
                .ok_or(GovernanceError::MissingAssetPrice)?;
            checked_mul(amount, price)?
        } else {
            0
        };
        let target_bps = treasury.rebalance_targets.get(asset.clone()).unwrap_or(0);
        let target_value_usd = if total_value_usd > 0 && target_bps > 0 {
            checked_div(checked_mul(total_value_usd, target_bps)?, BPS_DENOMINATOR)?
        } else {
            0
        };
        let delta_value_usd = checked_sub(target_value_usd, current_value_usd)?;

        if current_value_usd > 0 || target_bps > 0 {
            actions.push_back(RebalanceAction {
                asset,
                current_value_usd,
                target_value_usd,
                delta_value_usd,
                target_bps,
            });
        }
        action_index += 1;
    }

    treasury.total_value_usd = total_value_usd;
    treasury.last_rebalance = now;
    Ok(actions)
}

pub fn build_report(env: &Env, treasury: &Treasury) -> Result<TreasuryReport, GovernanceError> {
    let mut budgets = Vec::new(env);
    let mut total_spent = 0i128;
    let mut active_recurring_payments = 0u32;
    let mut monthly_burn_rate = 0i128;
    let thirty_days_ago = env.ledger().timestamp().saturating_sub(30 * 86_400);

    let mut recurring_index = 0;
    while recurring_index < treasury.recurring_payments.len() {
        if treasury
            .recurring_payments
            .get(recurring_index)
            .unwrap()
            .active
        {
            active_recurring_payments = active_recurring_payments.saturating_add(1);
        }
        recurring_index += 1;
    }

    let mut spend_index = 0;
    while spend_index < treasury.spending_history.len() {
        let spend = treasury.spending_history.get(spend_index).unwrap();
        total_spent = checked_add(total_spent, spend.amount)?;
        if spend.executed_at >= thirty_days_ago {
            monthly_burn_rate = checked_add(monthly_burn_rate, spend.amount)?;
        }
        spend_index += 1;
    }

    let mut budget_index = 0;
    while budget_index < treasury.budget_categories.len() {
        let category = treasury.budget_categories.get(budget_index).unwrap();
        if let Some(budget) = treasury.budgets.get(category.clone()) {
            let utilization_bps = if budget.allocated <= 0 {
                0
            } else {
                checked_div(
                    checked_mul(budget.spent, BPS_DENOMINATOR)?,
                    budget.allocated,
                )?
            };
            budgets.push_back(BudgetReport {
                category,
                allocated: budget.allocated,
                spent: budget.spent,
                remaining: budget.remaining,
                spend_limit: budget.spend_limit,
                utilization_bps,
            });
        }
        budget_index += 1;
    }

    let runway_months = if monthly_burn_rate > 0 {
        u32::try_from(checked_div(treasury.total_value_usd, monthly_burn_rate)?)
            .map_err(|_| GovernanceError::InvalidTreasuryConfig)?
    } else {
        999
    };

    Ok(TreasuryReport {
        total_assets: treasury.tracked_assets.len(),
        total_value_usd: treasury.total_value_usd,
        active_budgets: treasury.budget_categories.len(),
        active_recurring_payments,
        total_spends: treasury.spending_history.len(),
        total_spent,
        monthly_burn_rate,
        runway_months,
        last_rebalance: treasury.last_rebalance,
        budgets,
    })
}

fn renew_budget_if_needed(budget: &mut Budget, now: u64) -> Result<(), GovernanceError> {
    if now < budget.period_end {
        return Ok(());
    }
    if !budget.auto_renew {
        return Err(GovernanceError::BudgetPeriodEnded);
    }

    let duration = budget.period_end.saturating_sub(budget.period_start);
    if duration == 0 {
        return Err(GovernanceError::InvalidDuration);
    }

    let mut next_start = budget.period_start;
    let mut next_end = budget.period_end;
    while now >= next_end {
        next_start = next_end;
        next_end = next_end.saturating_add(duration);
    }

    budget.period_start = next_start;
    budget.period_end = next_end;
    budget.spent = 0;
    budget.remaining = budget.allocated;
    Ok(())
}

fn total_target_bps(treasury: &Treasury) -> Result<i128, GovernanceError> {
    let mut total = 0i128;
    let mut index = 0;

    while index < treasury.tracked_assets.len() {
        let asset = treasury.tracked_assets.get(index).unwrap();
        let target = treasury.rebalance_targets.get(asset).unwrap_or(0);
        total = checked_add(total, target)?;
        index += 1;
    }

    Ok(total)
}

fn track_asset(env: &Env, treasury: &mut Treasury, asset: &Asset) {
    let mut index = 0;
    while index < treasury.tracked_assets.len() {
        if treasury.tracked_assets.get(index).unwrap() == *asset {
            return;
        }
        index += 1;
    }
    treasury.tracked_assets.push_back(asset.clone());
    if !treasury.assets.contains_key(asset.clone()) {
        treasury.assets.set(asset.clone(), 0);
    }
    if !treasury.rebalance_targets.contains_key(asset.clone()) {
        treasury.rebalance_targets.set(asset.clone(), 0);
    }
    let _ = env;
}

fn track_category(_env: &Env, treasury: &mut Treasury, category: &String) {
    let mut index = 0;
    while index < treasury.budget_categories.len() {
        if treasury.budget_categories.get(index).unwrap() == *category {
            return;
        }
        index += 1;
    }
    treasury.budget_categories.push_back(category.clone());
}

#[cfg(test)]
mod tests {
    extern crate std;

    use soroban_sdk::testutils::{Address as _, Ledger};

    use super::*;

    fn sample_asset(env: &Env, code: &str) -> Asset {
        Asset {
            code: String::from_str(env, code),
            issuer: None,
        }
    }

    /// Helper: create a budget **and** attach a governance approval for it.
    fn setup_budget_with_approval(
        env: &Env,
        treasury: &mut Treasury,
        category: &str,
        allocated: i128,
        spend_limit: i128,
        period_end: u64,
        approved_cap: i128,
        proposal_id: u64,
    ) {
        upsert_budget(
            env,
            treasury,
            String::from_str(env, category),
            allocated,
            spend_limit,
            0,
            period_end,
            false,
        )
        .unwrap();
        approve_budget(
            env,
            treasury,
            String::from_str(env, category),
            proposal_id,
            approved_cap,
            env.ledger().timestamp(),
        )
        .unwrap();
    }

    // ─── existing tests (updated for new signature + approval requirement) ───

    #[test]
    fn spend_updates_budget_and_history() {
        let env = Env::default();
        env.ledger().set_timestamp(10);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        setup_budget_with_approval(&env, &mut treasury, "ops", 500, 250, 100, 500, 1);

        let spend = execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            200,
            asset.clone(),
            String::from_str(&env, "ops"),
            String::from_str(&env, "infra"),
            Some(1),
            10,
        )
        .unwrap();

        assert_eq!(spend.id, 1);
        assert_eq!(treasury.assets.get(asset).unwrap(), 800);
        let budget = treasury.budgets.get(String::from_str(&env, "ops")).unwrap();
        assert_eq!(budget.spent, 200);
        assert_eq!(budget.remaining, 300);
        assert_eq!(treasury.spending_history.len(), 1);

        // approval drawn counter advances
        let approval = treasury
            .approved_budgets
            .get(String::from_str(&env, "ops"))
            .unwrap();
        assert_eq!(approval.total_drawn, 200);
    }

    #[test]
    fn recurring_payment_executes_when_due() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "USDC");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        setup_budget_with_approval(&env, &mut treasury, "grants", 400, 200, 30, 400, 2);
        schedule_recurring_payment(
            &env,
            &mut treasury,
            Address::generate(&env),
            100,
            asset.clone(),
            10,
            String::from_str(&env, "grants"),
            String::from_str(&env, "builder"),
            None,
            Some(40),
        )
        .unwrap();

        let processed_early = process_recurring_payments(&env, &mut treasury, 9).unwrap();
        assert_eq!(processed_early, 0);

        let processed_due = process_recurring_payments(&env, &mut treasury, 10).unwrap();
        assert_eq!(processed_due, 1);
        assert_eq!(treasury.assets.get(asset).unwrap(), 900);
        assert_eq!(treasury.spending_history.len(), 1);
    }

    #[test]
    fn rebalance_builds_actions_from_targets() {
        let env = Env::default();
        let mut treasury = empty_treasury(&env);
        let xlm = sample_asset(&env, "XLM");
        let usdc = sample_asset(&env, "USDC");
        set_asset_balance(&env, &mut treasury, xlm.clone(), 100).unwrap();
        set_asset_balance(&env, &mut treasury, usdc.clone(), 100).unwrap();
        set_rebalance_target(&env, &mut treasury, xlm.clone(), 6_000).unwrap();
        set_rebalance_target(&env, &mut treasury, usdc.clone(), 4_000).unwrap();

        let mut prices = Map::new(&env);
        prices.set(xlm.clone(), 2);
        prices.set(usdc.clone(), 1);

        let actions = rebalance(&mut treasury, prices, 77, &env).unwrap();

        assert_eq!(treasury.total_value_usd, 300);
        assert_eq!(treasury.last_rebalance, 77);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions.get(0).unwrap().delta_value_usd, -20);
        assert_eq!(actions.get(1).unwrap().delta_value_usd, 20);
    }

    #[test]
    fn report_includes_burn_rate_and_runway() {
        let env = Env::default();
        env.ledger().set_timestamp(30 * 86_400);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "USDC");
        treasury.total_value_usd = 300;
        set_asset_balance(&env, &mut treasury, asset.clone(), 300).unwrap();
        setup_budget_with_approval(&env, &mut treasury, "ops", 500, 250, 30 * 86_400, 500, 3);
        // manually extend period so the auto-renew logic doesn't trip
        let mut b = treasury.budgets.get(String::from_str(&env, "ops")).unwrap();
        b.period_end = 60 * 86_400;
        treasury.budgets.set(String::from_str(&env, "ops"), b);

        execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            100,
            asset,
            String::from_str(&env, "ops"),
            String::from_str(&env, "hosting"),
            Some(3),
            30 * 86_400,
        )
        .unwrap();

        let report = build_report(&env, &treasury).unwrap();
        assert_eq!(report.total_spent, 100);
        assert_eq!(report.monthly_burn_rate, 100);
        assert_eq!(report.runway_months, 3);
    }

    // ─── new budget-cap tests ────────────────────────────────────────────────

    /// Spending without any prior `approve_budget` call must fail.
    #[test]
    fn spend_without_approval_is_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        upsert_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "ops"),
            500,
            500,
            0,
            100,
            false,
        )
        .unwrap();
        // NOTE: no approve_budget call

        let result = execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            100,
            asset,
            String::from_str(&env, "ops"),
            String::from_str(&env, "test"),
            None,
            0,
        );
        assert_eq!(result, Err(GovernanceError::BudgetApprovalRequired));
    }

    /// A spend that would push total_drawn past approved_cap must fail.
    #[test]
    fn spend_exceeding_approved_cap_is_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        // Budget allows up to 500, but governance only approved 200
        setup_budget_with_approval(&env, &mut treasury, "ops", 500, 500, 100, 200, 10);

        // First spend of 150 should succeed
        execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            150,
            asset.clone(),
            String::from_str(&env, "ops"),
            String::from_str(&env, "hosting"),
            Some(10),
            0,
        )
        .unwrap();

        // Second spend of 100 would push drawn to 250 > cap 200
        let result = execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            100,
            asset,
            String::from_str(&env, "ops"),
            String::from_str(&env, "extra"),
            Some(10),
            0,
        );
        assert_eq!(result, Err(GovernanceError::ApprovedCapExceeded));
    }

    /// A spend exactly equal to the remaining approved cap must succeed.
    #[test]
    fn spend_exactly_at_cap_succeeds() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        setup_budget_with_approval(&env, &mut treasury, "ops", 500, 300, 100, 300, 11);

        let spend = execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            300,
            asset,
            String::from_str(&env, "ops"),
            String::from_str(&env, "full draw"),
            Some(11),
            0,
        )
        .unwrap();

        assert_eq!(spend.amount, 300);
        let approval = treasury
            .approved_budgets
            .get(String::from_str(&env, "ops"))
            .unwrap();
        assert_eq!(approval.total_drawn, 300);
        assert_eq!(approval.total_drawn, approval.approved_cap);
    }

    /// Re-approving a category (new proposal, new cap) resets the drawn counter.
    #[test]
    fn re_approval_resets_drawn_counter() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        setup_budget_with_approval(&env, &mut treasury, "ops", 500, 300, 100, 300, 20);

        // Draw down to cap
        execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            300,
            asset.clone(),
            String::from_str(&env, "ops"),
            String::from_str(&env, "draw1"),
            Some(20),
            0,
        )
        .unwrap();

        // Next spend is rejected
        let rejected = execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            1,
            asset.clone(),
            String::from_str(&env, "ops"),
            String::from_str(&env, "over cap"),
            Some(20),
            0,
        );
        assert_eq!(rejected, Err(GovernanceError::ApprovedCapExceeded));

        // Re-approve with a fresh proposal
        approve_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "ops"),
            21,
            200,
            0,
        )
        .unwrap();

        // Now spending is allowed again under the new cap
        execute_spend(
            &env,
            &mut treasury,
            Address::generate(&env),
            150,
            asset,
            String::from_str(&env, "ops"),
            String::from_str(&env, "draw2"),
            Some(21),
            0,
        )
        .unwrap();

        let approval = treasury
            .approved_budgets
            .get(String::from_str(&env, "ops"))
            .unwrap();
        assert_eq!(approval.proposal_id, 21);
        assert_eq!(approval.total_drawn, 150);
    }

    /// `approve_budget` on a non-existent category returns `BudgetNotFound`.
    #[test]
    fn approve_budget_for_unknown_category_fails() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);

        let result = approve_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "nonexistent"),
            99,
            100,
            0,
        );
        assert_eq!(result, Err(GovernanceError::BudgetNotFound));
    }

    // ─── Issue #604: treasury diversification tests ──────────────────────────

    #[test]
    fn diversification_single_asset_treasury() {
        let env = Env::default();
        let mut treasury = empty_treasury(&env);
        let xlm = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, xlm.clone(), 1_000).unwrap();

        let mut prices = Map::new(&env);
        prices.set(xlm.clone(), 2);

        let report = get_diversification(&env, &treasury, &prices).unwrap();

        assert_eq!(report.total_value_usd, 2_000);
        assert_eq!(report.allocations.len(), 1);
        let alloc = report.allocations.get(0).unwrap();
        assert_eq!(alloc.asset, xlm);
        assert_eq!(alloc.amount, 1_000);
        assert_eq!(alloc.value_usd, 2_000);
        // single asset = 100% = 10 000 bps
        assert_eq!(alloc.concentration_bps, BPS_DENOMINATOR);
        assert_eq!(report.largest_holding_bps, BPS_DENOMINATOR);
    }

    #[test]
    fn diversification_multi_asset_correct_concentration() {
        let env = Env::default();
        let mut treasury = empty_treasury(&env);
        let xlm = sample_asset(&env, "XLM");
        let usdc = sample_asset(&env, "USDC");
        // XLM: 600 units @ $2 = $1200 (60 %)
        // USDC: 800 units @ $1 = $800 (40 %)
        set_asset_balance(&env, &mut treasury, xlm.clone(), 600).unwrap();
        set_asset_balance(&env, &mut treasury, usdc.clone(), 800).unwrap();

        let mut prices = Map::new(&env);
        prices.set(xlm.clone(), 2);
        prices.set(usdc.clone(), 1);

        let report = get_diversification(&env, &treasury, &prices).unwrap();

        assert_eq!(report.total_value_usd, 2_000);
        assert_eq!(report.allocations.len(), 2);

        // Find each allocation by asset.
        let mut xlm_bps = 0i128;
        let mut usdc_bps = 0i128;
        for i in 0..report.allocations.len() {
            let a = report.allocations.get(i).unwrap();
            if a.asset == xlm {
                xlm_bps = a.concentration_bps;
            } else {
                usdc_bps = a.concentration_bps;
            }
        }
        assert_eq!(xlm_bps, 6_000);
        assert_eq!(usdc_bps, 4_000);
        assert_eq!(report.largest_holding_bps, 6_000);
        assert_eq!(report.largest_asset, xlm);
    }

    #[test]
    fn diversification_excludes_zero_balance_assets() {
        let env = Env::default();
        let mut treasury = empty_treasury(&env);
        let xlm = sample_asset(&env, "XLM");
        let usdc = sample_asset(&env, "USDC");
        set_asset_balance(&env, &mut treasury, xlm.clone(), 500).unwrap();
        // USDC tracked but zero balance.
        set_asset_balance(&env, &mut treasury, usdc.clone(), 0).unwrap();

        let mut prices = Map::new(&env);
        prices.set(xlm.clone(), 1);
        prices.set(usdc.clone(), 1);

        let report = get_diversification(&env, &treasury, &prices).unwrap();

        // Only XLM should appear.
        assert_eq!(report.allocations.len(), 1);
        assert_eq!(report.allocations.get(0).unwrap().asset, xlm);
    }

    /// `approve_budget` with a cap larger than the budget's `allocated` fails.
    #[test]
    fn approve_budget_cap_exceeding_allocated_fails() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        upsert_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "ops"),
            200,
            200,
            0,
            100,
            false,
        )
        .unwrap();

        let result = approve_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "ops"),
            5,
            500, // 500 > allocated 200
            0,
        );
        assert_eq!(result, Err(GovernanceError::BudgetExceeded));
    }

    /// Recurring payment is deactivated when the approved cap is exhausted.
    #[test]
    fn recurring_payment_deactivated_when_cap_exhausted() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "USDC");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        // Budget allows 400, but governance only approved 150 — covers one 100-unit payment
        setup_budget_with_approval(&env, &mut treasury, "grants", 400, 200, 100, 150, 30);
        schedule_recurring_payment(
            &env,
            &mut treasury,
            Address::generate(&env),
            100,
            asset.clone(),
            10,
            String::from_str(&env, "grants"),
            String::from_str(&env, "stipend"),
            None,
            Some(200),
        )
        .unwrap();

        // First tick at t=10: cap still has headroom (drawn 0 → 100 ≤ 150)
        let processed = process_recurring_payments(&env, &mut treasury, 10).unwrap();
        assert_eq!(processed, 1);

        // Second tick at t=20: would draw 200 > cap 150 → payment deactivated
        let processed2 = process_recurring_payments(&env, &mut treasury, 20).unwrap();
        assert_eq!(processed2, 0);
        assert!(!treasury.recurring_payments.get(0).unwrap().active);
    }
}
