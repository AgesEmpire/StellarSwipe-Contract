//! User notification preferences (Issue #430).
//!
//! Stores per-user on-chain notification preferences so the frontend can
//! filter event subscriptions accordingly.

use crate::storage::DataKey;
use soroban_sdk::{contracttype, Address, Env, IntoVal, String, Symbol, Val, Vec};

/// Notification preferences for a user.
/// Default: all alerts enabled.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationPrefs {
    pub stop_loss_alerts: bool,
    pub take_profit_alerts: bool,
    pub signal_expiry_alerts: bool,
    /// Alert when a followed provider posts a new signal.
    pub new_signal_alert: bool,
    pub leaderboard_rank_change: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RiskRating {
    Low,
    Medium,
    High,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HoldDuration {
    Short,
    Medium,
    Long,
    Any,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignalCategory {
    SCALP,
    SWING,
    LONG_TERM,
    ARBITRAGE,
    PREMIUM,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignalAction {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum SignalStatus {
    Pending,
    Active,
    Executed,
    Expired,
    Successful,
    Failed,
    ProviderDeleted,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradingStyle {
    pub preferred_categories: Vec<SignalCategory>,
    pub risk_tolerance: RiskRating,
    pub max_hold_duration: HoldDuration,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Signal {
    pub id: u64,
    pub provider: Address,
    pub asset_pair: String,
    pub action: SignalAction,
    pub price: i128,
    pub rationale: String,
    pub timestamp: u64,
    pub expiry: u64,
    pub status: SignalStatus,
    pub executions: u32,
    pub successful_executions: u32,
    pub total_volume: i128,
    pub total_roi: i128,
    pub category: SignalCategory,
    pub tags: Vec<String>,
    pub risk_level: RiskLevel,
    pub is_collaborative: bool,
    pub submitted_at: u64,
    pub rationale_hash: String,
    pub confidence: u32,
    pub adoption_count: u32,
    pub ai_validation_score: Option<u32>,
    pub avg_copier_roi_bps: i32,
    pub copier_closed_count: u32,
    pub warning_emitted: bool,
    pub benchmark_return_bps: Option<i64>,
    pub alpha_bps: Option<i64>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SignalSummary {
    pub id: u64,
    pub provider: Address,
    pub asset_pair: String,
    pub action: SignalAction,
    pub price: i128,
    pub success_rate: u32,
    pub total_copies: u32,
    pub timestamp: u64,
}

impl NotificationPrefs {
    /// Returns the default preferences with all alerts enabled.
    pub fn default_prefs() -> Self {
        NotificationPrefs {
            stop_loss_alerts: true,
            take_profit_alerts: true,
            signal_expiry_alerts: true,
            new_signal_alert: true,
            leaderboard_rank_change: true,
        }
    }
}

/// Store notification preferences for `user`. Caller must be `user`.
pub fn set_notification_preferences(env: &Env, user: &Address, prefs: NotificationPrefs) {
    user.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::NotificationPrefs(user.clone()), &prefs);
}

/// Retrieve notification preferences for `user`.
/// Returns default (all enabled) if never set.
pub fn get_notification_preferences(env: &Env, user: &Address) -> NotificationPrefs {
    env.storage()
        .persistent()
        .get(&DataKey::NotificationPrefs(user.clone()))
        .unwrap_or_else(NotificationPrefs::default_prefs)
}

/// Store trading style profile for `user`. Caller must be `user`.
pub fn set_trading_style(env: &Env, user: &Address, style: TradingStyle) {
    user.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::TradingStyle(user.clone()), &style);
}

/// Retrieve trading style profile for `user`.
pub fn get_trading_style(env: &Env, user: &Address) -> Option<TradingStyle> {
    env.storage()
        .persistent()
        .get(&DataKey::TradingStyle(user.clone()))
}

fn is_risk_allowed(signal_risk: &RiskLevel, tolerance: &RiskRating) -> bool {
    matches!(tolerance, RiskRating::High)
        || (matches!(tolerance, RiskRating::Medium) && !matches!(signal_risk, RiskLevel::High))
        || (matches!(tolerance, RiskRating::Low) && matches!(signal_risk, RiskLevel::Low))
}

fn category_matches_duration(category: &SignalCategory, max_hold_duration: &HoldDuration) -> bool {
    match max_hold_duration {
        HoldDuration::Any => true,
        HoldDuration::Short => {
            matches!(category, SignalCategory::SCALP | SignalCategory::ARBITRAGE)
        }
        HoldDuration::Medium => matches!(
            category,
            SignalCategory::SCALP
                | SignalCategory::SWING
                | SignalCategory::ARBITRAGE
                | SignalCategory::PREMIUM
        ),
        HoldDuration::Long => true,
    }
}

fn category_in_preferences(category: &SignalCategory, preferred: &Vec<SignalCategory>) -> bool {
    if preferred.is_empty() {
        return true;
    }
    for i in 0..preferred.len() {
        if preferred.get(i) == Some(category.clone()) {
            return true;
        }
    }
    false
}

fn style_matches_signal(style: &TradingStyle, signal: &Signal) -> bool {
    category_in_preferences(&signal.category, &style.preferred_categories)
        && is_risk_allowed(&signal.risk_level, &style.risk_tolerance)
        && category_matches_duration(&signal.category, &style.max_hold_duration)
}

fn to_signal_summary(signal: &Signal) -> SignalSummary {
    let success_rate = if signal.executions > 0 {
        (signal.successful_executions * 10_000) / signal.executions
    } else {
        0
    };
    SignalSummary {
        id: signal.id,
        provider: signal.provider.clone(),
        asset_pair: signal.asset_pair.clone(),
        action: signal.action.clone(),
        price: signal.price,
        success_rate,
        total_copies: signal.executions,
        timestamp: signal.timestamp,
    }
}

/// Returns recommended active signals for `user` based on their trading style.
/// If the user has no style set, returns all active signals.
pub fn get_recommended_signals(
    env: &Env,
    user: &Address,
    signal_registry: &Address,
) -> Vec<SignalSummary> {
    let sym = Symbol::new(env, "get_active_signals_archived");
    let mut args = Vec::new(env);
    args.push_back(user.clone().into_val(env));
    args.push_back(false.into_val(env));
    let active_signals: Vec<Signal> = env.invoke_contract(signal_registry, &sym, args);

    let style = get_trading_style(env, user);
    let mut recommendations: Vec<SignalSummary> = Vec::new(env);

    for i in 0..active_signals.len() {
        let signal = active_signals.get(i).unwrap();
        let include = if let Some(ref style) = style {
            style_matches_signal(style, &signal)
        } else {
            true
        };
        if include {
            recommendations.push_back(to_signal_summary(&signal));
        }
    }

    recommendations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserPortfolio, UserPortfolioClient};
    use signal_registry::{
        RiskLevel as RegistryRiskLevel, SignalAction as RegistrySignalAction,
        SignalCategory as RegistrySignalCategory, SignalRegistry, SignalRegistryClient,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, String, Vec};

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

    fn setup_with_registry() -> (
        Env,
        Address,
        UserPortfolioClient<'static>,
        SignalRegistryClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        #[allow(deprecated)]
        let contract_id = env.register_contract(None, UserPortfolio);
        let client = UserPortfolioClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle);

        #[allow(deprecated)]
        let registry_id = env.register_contract(None, SignalRegistry);
        let registry = SignalRegistryClient::new(&env, &registry_id);
        registry.initialize(&admin);
        client.set_signal_registry(&admin, &registry_id);

        (env, contract_id, client, registry)
    }

    #[test]
    fn default_preferences_all_enabled() {
        let (env, _, client) = setup();
        let user = Address::generate(&env);
        let prefs = client.get_notification_preferences(&user);
        assert!(prefs.stop_loss_alerts);
        assert!(prefs.take_profit_alerts);
        assert!(prefs.signal_expiry_alerts);
        assert!(prefs.new_signal_alert);
        assert!(prefs.leaderboard_rank_change);
    }

    #[test]
    fn set_and_get_preferences() {
        let (env, _, client) = setup();
        let user = Address::generate(&env);
        let prefs = NotificationPrefs {
            stop_loss_alerts: true,
            take_profit_alerts: false,
            signal_expiry_alerts: true,
            new_signal_alert: false,
            leaderboard_rank_change: true,
        };
        client.set_notification_preferences(&user, &prefs);
        let stored = client.get_notification_preferences(&user);
        assert_eq!(stored.stop_loss_alerts, true);
        assert_eq!(stored.take_profit_alerts, false);
        assert_eq!(stored.signal_expiry_alerts, true);
        assert_eq!(stored.new_signal_alert, false);
        assert_eq!(stored.leaderboard_rank_change, true);
    }

    #[test]
    fn update_preferences() {
        let (env, _, client) = setup();
        let user = Address::generate(&env);
        // First set
        let prefs1 = NotificationPrefs {
            stop_loss_alerts: false,
            take_profit_alerts: false,
            signal_expiry_alerts: false,
            new_signal_alert: false,
            leaderboard_rank_change: false,
        };
        client.set_notification_preferences(&user, &prefs1);
        // Update
        let prefs2 = NotificationPrefs {
            stop_loss_alerts: true,
            take_profit_alerts: true,
            signal_expiry_alerts: false,
            new_signal_alert: true,
            leaderboard_rank_change: false,
        };
        client.set_notification_preferences(&user, &prefs2);
        let stored = client.get_notification_preferences(&user);
        assert_eq!(stored.stop_loss_alerts, true);
        assert_eq!(stored.take_profit_alerts, true);
        assert_eq!(stored.signal_expiry_alerts, false);
        assert_eq!(stored.new_signal_alert, true);
        assert_eq!(stored.leaderboard_rank_change, false);
    }

    #[test]
    fn set_and_get_trading_style() {
        let (env, _, client) = setup();
        let user = Address::generate(&env);
        let mut categories = Vec::new(&env);
        categories.push_back(SignalCategory::SWING);

        let style = TradingStyle {
            preferred_categories: categories,
            risk_tolerance: RiskRating::Medium,
            max_hold_duration: HoldDuration::Medium,
        };

        client.set_trading_style(&user, &style);
        let stored = client
            .get_trading_style(&user)
            .expect("style should be stored");
        assert_eq!(stored, style);
    }

    #[test]
    fn get_recommended_signals_no_style_returns_all_active_signals() {
        let (env, _, client, registry) = setup_with_registry();
        let user = Address::generate(&env);
        let provider = Address::generate(&env);
        let asset_pair = String::from_str(&env, "XLM/USDC");
        let rationale = String::from_str(&env, "strong momentum");
        let tags = Vec::new(&env);
        let expiry = env.ledger().timestamp() + 10_000;

        registry.create_signal(
            &provider,
            &asset_pair,
            &RegistrySignalAction::Buy,
            &100,
            &rationale,
            &expiry,
            &RegistrySignalCategory::SWING,
            &tags,
            &RegistryRiskLevel::Low,
        );

        registry.create_signal(
            &provider,
            &asset_pair,
            &RegistrySignalAction::Sell,
            &50,
            &rationale,
            &expiry,
            &RegistrySignalCategory::SCALP,
            &tags,
            &RegistryRiskLevel::High,
        );

        let recommendations = client.get_recommended_signals(&user);
        assert_eq!(recommendations.len(), 2);
    }

    #[test]
    fn get_recommended_signals_filters_by_trading_style() {
        let (env, _, client, registry) = setup_with_registry();
        let user = Address::generate(&env);
        let provider = Address::generate(&env);
        let asset_pair = String::from_str(&env, "XLM/USDC");
        let rationale = String::from_str(&env, "strong momentum");
        let tags = Vec::new(&env);
        let expiry = env.ledger().timestamp() + 10_000;

        let id1 = registry.create_signal(
            &provider,
            &asset_pair,
            &RegistrySignalAction::Buy,
            &100,
            &rationale,
            &expiry,
            &RegistrySignalCategory::SWING,
            &tags,
            &RegistryRiskLevel::Low,
        );

        registry.create_signal(
            &provider,
            &asset_pair,
            &RegistrySignalAction::Sell,
            &50,
            &rationale,
            &expiry,
            &RegistrySignalCategory::SCALP,
            &tags,
            &RegistryRiskLevel::High,
        );

        let mut preferred = Vec::new(&env);
        preferred.push_back(SignalCategory::SWING);
        let style = TradingStyle {
            preferred_categories: preferred,
            risk_tolerance: RiskRating::Low,
            max_hold_duration: HoldDuration::Medium,
        };
        client.set_trading_style(&user, &style);

        let recommendations = client.get_recommended_signals(&user);
        assert_eq!(recommendations.len(), 1);
        assert_eq!(recommendations.get(0).unwrap().id, id1);
    }
}
