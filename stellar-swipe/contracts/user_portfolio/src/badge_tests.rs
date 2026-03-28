use super::oracle_ok::OracleMock;
use super::oracle_ok::OracleMockClient;
use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;

#[allow(deprecated)]
fn setup(env: &Env, early_adopter_cap: u32) -> (Address, Address, Address) {
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let oracle_id = env.register_contract(None, OracleMock);
    OracleMockClient::new(env, &oracle_id).set_price(&100_i128);
    let contract_id = env.register_contract(None, UserPortfolio);
    let client = UserPortfolioClient::new(env, &contract_id);
    env.mock_all_auths();
    client.initialize(&admin, &oracle_id, &early_adopter_cap);
    (admin, user, contract_id)
}

fn count_type(badges: &Vec<Badge>, t: BadgeType) -> u32 {
    let mut n = 0;
    for i in 0..badges.len() {
        if let Some(b) = badges.get(i) {
            if b.badge_type == t {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn badge_first_trade_on_first_close_only() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    let id = client.open_position(&user, &100, &1_000);
    assert_eq!(
        count_type(&client.get_badges(&user), BadgeType::FirstTrade),
        0
    );
    client.close_position(&user, &id, &10);
    let badges = client.get_badges(&user);
    assert_eq!(count_type(&badges, BadgeType::FirstTrade), 1);
    let id2 = client.open_position(&user, &100, &500);
    client.close_position(&user, &id2, &20);
    let badges = client.get_badges(&user);
    assert_eq!(count_type(&badges, BadgeType::FirstTrade), 1);
}

#[test]
fn badge_ten_trades_on_tenth_close() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    for _ in 0..9 {
        let id = client.open_position(&user, &100, &100);
        client.close_position(&user, &id, &0);
    }
    let before = client.get_badges(&user);
    assert_eq!(count_type(&before, BadgeType::TenTrades), 0);
    let id = client.open_position(&user, &100, &100);
    client.close_position(&user, &id, &0);
    let badges = client.get_badges(&user);
    assert_eq!(count_type(&badges, BadgeType::TenTrades), 1);
}

#[test]
fn badge_profitable_streak_five() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    for _ in 0..4 {
        let id = client.open_position(&user, &100, &100);
        client.close_position(&user, &id, &1);
    }
    assert_eq!(
        count_type(&client.get_badges(&user), BadgeType::ProfitableStreak5),
        0
    );
    let id = client.open_position(&user, &100, &100);
    client.close_position(&user, &id, &1);
    let badges = client.get_badges(&user);
    assert_eq!(count_type(&badges, BadgeType::ProfitableStreak5), 1);
}

#[test]
fn badge_profitable_streak_resets_on_loss() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    for _ in 0..4 {
        let id = client.open_position(&user, &100, &100);
        client.close_position(&user, &id, &1);
    }
    let id = client.open_position(&user, &100, &100);
    client.close_position(&user, &id, &-1);
    for _ in 0..4 {
        let id = client.open_position(&user, &100, &100);
        client.close_position(&user, &id, &1);
    }
    assert_eq!(
        count_type(&client.get_badges(&user), BadgeType::ProfitableStreak5),
        0
    );
    let id = client.open_position(&user, &100, &100);
    client.close_position(&user, &id, &1);
    assert_eq!(
        count_type(&client.get_badges(&user), BadgeType::ProfitableStreak5),
        1
    );
}

#[test]
fn badge_top10_when_rank_set() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    client.set_leaderboard_rank(&user, &7_u32);
    let id = client.open_position(&user, &100, &100);
    client.close_position(&user, &id, &0);
    let badges = client.get_badges(&user);
    assert_eq!(count_type(&badges, BadgeType::Top10Leaderboard), 1);
    let id2 = client.open_position(&user, &100, &100);
    client.close_position(&user, &id2, &0);
    assert_eq!(
        count_type(&client.get_badges(&user), BadgeType::Top10Leaderboard),
        1
    );
}

#[test]
fn badge_top10_not_awarded_when_rank_zero() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    let id = client.open_position(&user, &100, &100);
    client.close_position(&user, &id, &0);
    assert_eq!(
        count_type(
            &client.get_badges(&user),
            BadgeType::Top10Leaderboard
        ),
        0
    );
}

#[test]
fn badge_early_adopter_cap() {
    let env = Env::default();
    let (_, _, cid) = setup(&env, 2);
    let client = UserPortfolioClient::new(&env, &cid);
    let u1 = Address::generate(&env);
    let u2 = Address::generate(&env);
    let u3 = Address::generate(&env);
    env.mock_all_auths();
    client.open_position(&u1, &100, &100);
    client.open_position(&u2, &100, &100);
    client.open_position(&u3, &100, &100);
    assert_eq!(
        count_type(&client.get_badges(&u1), BadgeType::EarlyAdopter),
        1
    );
    assert_eq!(
        count_type(&client.get_badges(&u2), BadgeType::EarlyAdopter),
        1
    );
    assert_eq!(
        count_type(&client.get_badges(&u3), BadgeType::EarlyAdopter),
        0
    );
}

#[test]
fn badge_awarded_emits_event() {
    let env = Env::default();
    let (_, user, cid) = setup(&env, 1000);
    let client = UserPortfolioClient::new(&env, &cid);
    let id = client.open_position(&user, &100, &1_000);
    client.close_position(&user, &id, &1);
    assert!(
        !env.events().all().is_empty(),
        "expected contract events after badge award"
    );
}
