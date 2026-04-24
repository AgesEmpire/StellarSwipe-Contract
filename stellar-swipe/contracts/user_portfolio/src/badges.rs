//! Verifiable on-chain badges: milestones evaluated from `open_position` / `close_position`.

use crate::storage::DataKey;
use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BadgeType {
    FirstTrade = 0,
    TenTrades = 1,
    ProfitableStreak5 = 2,
    Top10Leaderboard = 3,
    EarlyAdopter = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Badge {
    pub badge_type: BadgeType,
    pub awarded_at: u64,
    pub metadata: Symbol,
}

pub fn get_badges(env: &Env, user: Address) -> Vec<Badge> {
    let key = DataKey::UserBadges(user);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

pub(crate) fn after_open_position(env: &Env, user: &Address, is_first_ever_open: bool) {
    if is_first_ever_open {
        let cap: u32 = env
            .storage()
            .instance()
            .get(&DataKey::EarlyAdopterCap)
            .unwrap_or(1000);
        let n: u32 = env
            .storage()
            .instance()
            .get(&DataKey::TotalUsersFirstOpen)
            .unwrap_or(0);
        if n < cap {
            try_grant(
                env,
                user,
                BadgeType::EarlyAdopter,
                Symbol::new(env, "early"),
            );
        }
        env.storage()
            .instance()
            .set(&DataKey::TotalUsersFirstOpen, &n.saturating_add(1));
    }
    maybe_top10_leaderboard(env, user);
}

pub(crate) fn after_close_position(env: &Env, user: &Address, realized_pnl: i128) {
    let ckey = DataKey::UserClosedTradeCount(user.clone());
    let closed: u32 = env
        .storage()
        .persistent()
        .get(&ckey)
        .unwrap_or(0);
    let closed = closed.saturating_add(1);
    env.storage().persistent().set(&ckey, &closed);

    if closed == 1 {
        try_grant(
            env,
            user,
            BadgeType::FirstTrade,
            Symbol::new(env, "first"),
        );
    }
    if closed == 10 {
        try_grant(
            env,
            user,
            BadgeType::TenTrades,
            Symbol::new(env, "ten"),
        );
    }

    let skey = DataKey::UserProfitStreak(user.clone());
    let mut streak: u32 = env.storage().persistent().get(&skey).unwrap_or(0);
    if realized_pnl > 0 {
        streak = streak.saturating_add(1);
    } else {
        streak = 0;
    }
    env.storage().persistent().set(&skey, &streak);

    if streak == 5 {
        try_grant(
            env,
            user,
            BadgeType::ProfitableStreak5,
            Symbol::new(env, "strk5"),
        );
    }

    maybe_top10_leaderboard(env, user);
}

fn maybe_top10_leaderboard(env: &Env, user: &Address) {
    let rank: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::LeaderboardRank(user.clone()))
        .unwrap_or(0);
    if rank >= 1 && rank <= 10 {
        try_grant(
            env,
            user,
            BadgeType::Top10Leaderboard,
            Symbol::new(env, "top10"),
        );
    }
}

fn has_badge(env: &Env, user: &Address, badge_type: BadgeType) -> bool {
    let key = DataKey::UserBadges(user.clone());
    let Some(list) = env.storage().persistent().get::<DataKey, Vec<Badge>>(&key) else {
        return false;
    };
    for i in 0..list.len() {
        if let Some(b) = list.get(i) {
            if b.badge_type == badge_type {
                return true;
            }
        }
    }
    false
}

fn try_grant(env: &Env, user: &Address, badge_type: BadgeType, metadata: Symbol) {
    if has_badge(env, user, badge_type) {
        return;
    }
    let awarded_at = env.ledger().timestamp();
    let badge = Badge {
        badge_type,
        awarded_at,
        metadata,
    };
    let key = DataKey::UserBadges(user.clone());
    let mut list: Vec<Badge> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    list.push_back(badge);
    env.storage().persistent().set(&key, &list);
    emit_badge_awarded(env, user, badge_type);
}

#[allow(deprecated)]
fn emit_badge_awarded(env: &Env, user: &Address, badge_type: BadgeType) {
    let topics = (
        Symbol::new(env, "BadgeAwarded"),
        user.clone(),
        badge_type as u32,
    );
    env.events().publish(topics, ());
}
