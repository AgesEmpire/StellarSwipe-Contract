use core::convert::TryFrom;

use soroban_sdk::{contracttype, Address, Env, Map, String, Vec};
use stellar_swipe_common::Asset;

use crate::errors::GovernanceError;
use crate::{checked_add, checked_div, checked_mul, checked_sub};

pub const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Treasury {
    pub assets: Map<Asset, i128>,
    pub tracked_assets: Vec<Asset>,
    pub total_value_usd: i128,
    pub budgets: Map<String, Budget>,
    pub budget_categories: Vec<String>,
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

pub fn empty_treasury(env: &Env) -> Treasury {
    Treasury {
        assets: Map::new(env),
        tracked_assets: Vec::new(env),
        total_value_usd: 0,
        budgets: Map::new(env),
        budget_categories: Vec::new(env),
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

#[allow(clippy::too_many_arguments)]
pub fn execute_spend(
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

    let mut budget = treasury
        .budgets
        .get(category.clone())
        .ok_or(GovernanceError::BudgetNotFound)?;
    renew_budget_if_needed(&mut budget, executed_at)?;

    if amount > budget.remaining || amount > budget.spend_limit {
        return Err(GovernanceError::BudgetExceeded);
    }

    let current_balance = treasury.assets.get(asset.clone()).unwrap_or(0);
    if current_balance < amount {
        return Err(GovernanceError::InsufficientBalance);
    }

    budget.spent = checked_add(budget.spent, amount)?;
    budget.remaining = checked_sub(budget.remaining, amount)?;
    treasury.budgets.set(category.clone(), budget);
    treasury
        .assets
        .set(asset.clone(), checked_sub(current_balance, amount)?);

    let spend = TreasurySpend {
        id: treasury.next_spend_id,
        recipient,
        amount,
        asset,
        category,
        purpose,
        approved_by_proposal,
        executed_at,
    };
    treasury.next_spend_id = treasury.next_spend_id.saturating_add(1);
    treasury.spending_history.push_back(spend.clone());
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
                    | GovernanceError::InsufficientBalance,
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

    #[test]
    fn spend_updates_budget_and_history() {
        let env = Env::default();
        env.ledger().set_timestamp(10);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "XLM");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        upsert_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "ops"),
            500,
            250,
            0,
            100,
            false,
        )
        .unwrap();

        let spend = execute_spend(
            &mut treasury,
            Address::generate(&env),
            200,
            asset.clone(),
            String::from_str(&env, "ops"),
            String::from_str(&env, "infra"),
            Some(114),
            10,
        )
        .unwrap();

        assert_eq!(spend.id, 1);
        assert_eq!(treasury.assets.get(asset).unwrap(), 800);
        let budget = treasury.budgets.get(String::from_str(&env, "ops")).unwrap();
        assert_eq!(budget.spent, 200);
        assert_eq!(budget.remaining, 300);
        assert_eq!(treasury.spending_history.len(), 1);
    }

    #[test]
    fn recurring_payment_executes_when_due() {
        let env = Env::default();
        env.ledger().set_timestamp(0);
        let mut treasury = empty_treasury(&env);
        let asset = sample_asset(&env, "USDC");
        set_asset_balance(&env, &mut treasury, asset.clone(), 1_000).unwrap();
        upsert_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "grants"),
            400,
            200,
            0,
            30,
            true,
        )
        .unwrap();
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

        let processed_early = process_recurring_payments(&mut treasury, 9).unwrap();
        assert_eq!(processed_early, 0);

        let processed_due = process_recurring_payments(&mut treasury, 10).unwrap();
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
        upsert_budget(
            &env,
            &mut treasury,
            String::from_str(&env, "ops"),
            500,
            250,
            0,
            30 * 86_400,
            true,
        )
        .unwrap();
        execute_spend(
            &mut treasury,
            Address::generate(&env),
            100,
            asset,
            String::from_str(&env, "ops"),
            String::from_str(&env, "hosting"),
            Some(114),
            30 * 86_400,
        )
        .unwrap();

        let report = build_report(&env, &treasury).unwrap();
        assert_eq!(report.total_spent, 100);
        assert_eq!(report.monthly_burn_rate, 100);
        assert_eq!(report.runway_months, 3);
    }
}
