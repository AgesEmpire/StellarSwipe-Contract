//! User achievement system (Issue #432).
//!
//! Tracks long-term engagement milestones. Progress updates automatically on
//! relevant events. Emits `AchievementCompleted` when a target is reached.

use crate::storage::DataKey;
use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

/// Achievement type identifiers.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AchievementType {
    Trades100 = 0,
    Profit1000Xlm = 1,
    Streak10Wins = 2,
    Followed10Providers = 3,
    EarlyAdopter = 4,
}

/// A single achievement record for a user.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Achievement {
    pub achievement_type: AchievementType,
    pub progress: u32,
    pub target: u32,
    pub completed: bool,
    pub completed_at: Option<u64>,
}

impl Achievement {
    fn new(achievement_type: AchievementType) -> Self {
        Achievement {
            achievement_type,
            progress: 0,
            target: target_for(achievement_type),
            completed: false,
            completed_at: None,
        }
    }
}

fn target_for(t: AchievementType) -> u32 {
    match t {
        AchievementType::Trades100 => 100,
        AchievementType::Profit1000Xlm => 1000,
        AchievementType::Streak10Wins => 10,
        AchievementType::Followed10Providers => 10,
        AchievementType::EarlyAdopter => 1,
    }
}

fn achievement_type_for_quest(quest_id: u32) -> Option<AchievementType> {
    match quest_id {
        0 => Some(AchievementType::Trades100),
        1 => Some(AchievementType::Profit1000Xlm),
        2 => Some(AchievementType::Streak10Wins),
        3 => Some(AchievementType::Followed10Providers),
        4 => Some(AchievementType::EarlyAdopter),
        _ => None,
    }
}

fn get_achievement(env: &Env, user: &Address, achievement_type: AchievementType) -> Achievement {
    let list = get_achievements(env, user);
    for i in 0..list.len() {
        let a = list.get_unchecked(i);
        if a.achievement_type == achievement_type {
            return a;
        }
    }
    panic!("achievement not found");
}

pub fn verify_quest_completion(env: &Env, user: &Address, quest_id: u32) -> bool {
    if let Some(achievement_type) = achievement_type_for_quest(quest_id) {
        get_achievement(env, user, achievement_type).completed
    } else {
        false
    }
}

const ALL_TYPES: [AchievementType; 5] = [
    AchievementType::Trades100,
    AchievementType::Profit1000Xlm,
    AchievementType::Streak10Wins,
    AchievementType::Followed10Providers,
    AchievementType::EarlyAdopter,
];

/// Load all achievements for a user, initialising missing ones with zero progress.
pub fn get_achievements(env: &Env, user: &Address) -> Vec<Achievement> {
    env.storage()
        .persistent()
        .get(&DataKey::UserAchievements(user.clone()))
        .unwrap_or_else(|| {
            let mut list = Vec::new(env);
            for t in ALL_TYPES {
                list.push_back(Achievement::new(t));
            }
            list
        })
}

fn save_achievements(env: &Env, user: &Address, list: &Vec<Achievement>) {
    env.storage()
        .persistent()
        .set(&DataKey::UserAchievements(user.clone()), list);
}

/// Increment progress for a specific achievement type by `delta`.
/// Emits `AchievementCompleted` the first time the target is reached.
pub fn increment_progress(
    env: &Env,
    user: &Address,
    achievement_type: AchievementType,
    delta: u32,
) {
    let mut list = get_achievements(env, user);
    for i in 0..list.len() {
        let mut a = list.get_unchecked(i);
        if a.achievement_type == achievement_type {
            if a.completed {
                // Already completed — do not re-complete.
                return;
            }
            a.progress = a.progress.saturating_add(delta).min(a.target);
            if a.progress >= a.target {
                a.completed = true;
                a.completed_at = Some(env.ledger().timestamp());
                list.set(i, a);
                save_achievements(env, user, &list);
                emit_achievement_completed(env, user, achievement_type);
                return;
            }
            list.set(i, a);
            save_achievements(env, user, &list);
            return;
        }
    }
}

fn emit_achievement_completed(env: &Env, user: &Address, achievement_type: AchievementType) {
    env.events().publish(
        (
            symbol_short!("ach_done"),
            user.clone(),
            achievement_type as u32,
        ),
        (),
    );
}

/// Called after a trade is closed to update trade-count and profit achievements.
pub fn on_trade_closed(env: &Env, user: &Address, realized_pnl: i128) {
    increment_progress(env, user, AchievementType::Trades100, 1);
    if realized_pnl > 0 {
        // profit in stroops (1 XLM = 10_000_000 stroops); target is 1000 XLM = 10_000_000_000 stroops.
        // We track progress in whole XLM units (stroops / 10_000_000).
        let xlm_profit = (realized_pnl / 10_000_000) as u32;
        if xlm_profit > 0 {
            increment_progress(env, user, AchievementType::Profit1000Xlm, xlm_profit);
        }
    }
}

/// Called after a winning streak update to track Streak10Wins.
pub fn on_streak_updated(env: &Env, user: &Address, current_streak: u32) {
    // Set progress to current streak value (not additive — streak is a high-water mark).
    let mut list = get_achievements(env, user);
    for i in 0..list.len() {
        let mut a = list.get_unchecked(i);
        if a.achievement_type == AchievementType::Streak10Wins {
            if a.completed {
                return;
            }
            if current_streak > a.progress {
                a.progress = current_streak.min(a.target);
                if a.progress >= a.target {
                    a.completed = true;
                    a.completed_at = Some(env.ledger().timestamp());
                    list.set(i, a);
                    save_achievements(env, user, &list);
                    emit_achievement_completed(env, user, AchievementType::Streak10Wins);
                    return;
                }
                list.set(i, a);
                save_achievements(env, user, &list);
            }
            return;
        }
    }
}

/// Called when a user follows a new provider.
pub fn on_provider_followed(env: &Env, user: &Address) {
    increment_progress(env, user, AchievementType::Followed10Providers, 1);
}

/// Called when a user qualifies as an early adopter.
pub fn on_early_adopter(env: &Env, user: &Address) {
    increment_progress(env, user, AchievementType::EarlyAdopter, 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserPortfolio, UserPortfolioClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    fn setup() -> (Env, Address, UserPortfolioClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        #[allow(deprecated)]
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle);
        (env, contract_id, client)
    }

    fn find_achievement(list: &Vec<Achievement>, t: AchievementType) -> Achievement {
        for i in 0..list.len() {
            let a = list.get_unchecked(i);
            if a.achievement_type == t {
                return a;
            }
        }
        panic!("achievement not found");
    }

    #[test]
    fn trades100_progress_tracking() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            for _ in 0..99 {
                increment_progress(&env, &user, AchievementType::Trades100, 1);
            }
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Trades100);
            assert_eq!(a.progress, 99);
            assert!(!a.completed);

            increment_progress(&env, &user, AchievementType::Trades100, 1);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Trades100);
            assert_eq!(a.progress, 100);
            assert!(a.completed);
            assert!(a.completed_at.is_some());
        });
    }

    #[test]
    fn completed_achievement_not_re_completed() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            increment_progress(&env, &user, AchievementType::EarlyAdopter, 1);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::EarlyAdopter);
            assert!(a.completed);
            let completed_at = a.completed_at;

            // Calling again should not change completed_at
            increment_progress(&env, &user, AchievementType::EarlyAdopter, 1);
            let list2 = get_achievements(&env, &user);
            let a2 = find_achievement(&list2, AchievementType::EarlyAdopter);
            assert_eq!(a2.completed_at, completed_at);
        });
    }

    #[test]
    fn streak10_wins_progress() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            on_streak_updated(&env, &user, 9);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Streak10Wins);
            assert_eq!(a.progress, 9);
            assert!(!a.completed);

            on_streak_updated(&env, &user, 10);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Streak10Wins);
            assert!(a.completed);
        });
    }

    #[test]
    fn followed10_providers_progress() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            for _ in 0..9 {
                on_provider_followed(&env, &user);
            }
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Followed10Providers);
            assert_eq!(a.progress, 9);
            assert!(!a.completed);

            on_provider_followed(&env, &user);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Followed10Providers);
            assert!(a.completed);
        });
    }

    #[test]
    fn profit1000_xlm_progress() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            // 999 XLM profit (in stroops)
            on_trade_closed(&env, &user, 999 * 10_000_000);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Profit1000Xlm);
            assert_eq!(a.progress, 999);
            assert!(!a.completed);

            // 1 more XLM
            on_trade_closed(&env, &user, 1 * 10_000_000);
            let list = get_achievements(&env, &user);
            let a = find_achievement(&list, AchievementType::Profit1000Xlm);
            assert!(a.completed);
        });
    }

    #[test]
    fn achievement_completed_event_emitted() {
        use soroban_sdk::testutils::Events as _;
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            increment_progress(&env, &user, AchievementType::EarlyAdopter, 1);
        });
        assert!(!env.events().all().is_empty());
    }

    #[test]
    fn verify_quest_completion_returns_true_when_achievement_completed() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            increment_progress(&env, &user, AchievementType::EarlyAdopter, 1);
            assert!(verify_quest_completion(&env, &user, 4));
        });
    }

    #[test]
    fn verify_quest_completion_returns_false_for_unknown_quest() {
        let (env, contract_id, _) = setup();
        let user = Address::generate(&env);
        env.as_contract(&contract_id, || {
            assert!(!verify_quest_completion(&env, &user, 999));
        });
    }
}
