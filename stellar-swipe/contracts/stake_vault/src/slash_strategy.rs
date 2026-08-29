//! Slash strategy configuration (#1021).
//!
//! Each named strategy stores its own slashing thresholds (basis points),
//! penalty window, and risk window.  The contract enforces:
//! - All bps values in `[0, 10_000]`.
//! - Non-decreasing severity order: `minor_bps <= major_bps <= critical_bps`.
//! - `risk_window_secs > 0`.
//!
//! Slashing logic reads the active strategy's thresholds instead of the
//! global `SlashTierConfig`, giving the protocol per-strategy risk control.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

use shared::event_topics as topics;

// ── Storage key ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum SlashStrategyKey {
    /// All registered strategy names (Vec<Symbol>).
    StrategyNames,
    /// Per-strategy config: strategy_name → SlashStrategyConfig.
    Strategy(Symbol),
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Per-strategy slashing policy thresholds.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashStrategyConfig {
    /// Basis points slashed for a Minor violation (0–10_000).
    pub minor_bps: u32,
    /// Basis points slashed for a Major violation (0–10_000).
    pub major_bps: u32,
    /// Basis points slashed for a Critical violation (0–10_000).
    pub critical_bps: u32,
    /// Seconds within which repeated violations incur escalated penalties.
    pub risk_window_secs: u64,
    /// Seconds a provider is penalised (e.g. barred from rewards) after a slash.
    pub penalty_window_secs: u64,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum SlashStrategyError {
    /// A bps value exceeds 10_000 (100 %).
    BpsOutOfRange,
    /// Severity order violated: minor > major or major > critical.
    InvalidSeverityOrder,
    /// risk_window_secs must be > 0.
    InvalidRiskWindow,
    /// Strategy name not found.
    StrategyNotFound,
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate(cfg: &SlashStrategyConfig) -> Result<(), SlashStrategyError> {
    if cfg.minor_bps > 10_000 || cfg.major_bps > 10_000 || cfg.critical_bps > 10_000 {
        return Err(SlashStrategyError::BpsOutOfRange);
    }
    if cfg.minor_bps > cfg.major_bps || cfg.major_bps > cfg.critical_bps {
        return Err(SlashStrategyError::InvalidSeverityOrder);
    }
    if cfg.risk_window_secs == 0 {
        return Err(SlashStrategyError::InvalidRiskWindow);
    }
    Ok(())
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Admin: set (or update) the slash strategy config for `strategy_name`.
///
/// Validates thresholds before writing.  Emits `slhstrat` on success.
pub fn set_slash_strategy(
    env: &Env,
    strategy_name: Symbol,
    cfg: SlashStrategyConfig,
) -> Result<(), SlashStrategyError> {
    validate(&cfg)?;

    // Track strategy names for enumeration.
    let mut names: Vec<Symbol> = env
        .storage()
        .instance()
        .get(&SlashStrategyKey::StrategyNames)
        .unwrap_or_else(|| Vec::new(env));

    let mut found = false;
    for i in 0..names.len() {
        if names.get(i).unwrap() == strategy_name {
            found = true;
            break;
        }
    }
    if !found {
        names.push_back(strategy_name.clone());
        env.storage()
            .instance()
            .set(&SlashStrategyKey::StrategyNames, &names);
    }

    env.storage()
        .instance()
        .set(&SlashStrategyKey::Strategy(strategy_name.clone()), &cfg);

    env.events().publish(
        (
            Symbol::new(env, "stake_vault"),
            topics::TOPIC_SLASH_STRATEGY_SET(),
        ),
        (strategy_name, cfg.minor_bps, cfg.major_bps, cfg.critical_bps),
    );

    Ok(())
}

/// Returns the config for `strategy_name`, or `None` if not registered.
pub fn get_slash_strategy(env: &Env, strategy_name: Symbol) -> Option<SlashStrategyConfig> {
    env.storage()
        .instance()
        .get(&SlashStrategyKey::Strategy(strategy_name))
}

/// Returns all registered strategy names.
pub fn list_slash_strategies(env: &Env) -> Vec<Symbol> {
    env.storage()
        .instance()
        .get(&SlashStrategyKey::StrategyNames)
        .unwrap_or_else(|| Vec::new(env))
}

/// Compute the slash amount for `balance` under `strategy_name` at `severity`.
///
/// `severity`: 0 = Minor, 1 = Major, 2 = Critical.
/// Returns `Err(StrategyNotFound)` if the strategy is not registered.
pub fn compute_slash_amount(
    env: &Env,
    strategy_name: Symbol,
    balance: i128,
    severity: u32,
) -> Result<i128, SlashStrategyError> {
    let cfg = get_slash_strategy(env, strategy_name)
        .ok_or(SlashStrategyError::StrategyNotFound)?;

    let bps = match severity {
        0 => cfg.minor_bps as i128,
        1 => cfg.major_bps as i128,
        _ => cfg.critical_bps as i128,
    };

    let amount = (balance * bps) / 10_000;
    Ok(amount.max(if balance > 0 { 1 } else { 0 }).min(balance))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, symbol_short, Env};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, soroban_sdk::Address) {
        let env = Env::default();
        env.mock_all_auths();
        let addr = env.register(TestContract, ());
        (env, addr)
    }

    fn default_cfg() -> SlashStrategyConfig {
        SlashStrategyConfig {
            minor_bps: 500,
            major_bps: 3_000,
            critical_bps: 10_000,
            risk_window_secs: 86_400,
            penalty_window_secs: 3_600,
        }
    }

    #[test]
    fn set_and_get_strategy() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let name = symbol_short!("default");
            set_slash_strategy(&env, name.clone(), default_cfg()).unwrap();
            let got = get_slash_strategy(&env, name).unwrap();
            assert_eq!(got, default_cfg());
        });
    }

    #[test]
    fn invalid_bps_rejected() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let mut cfg = default_cfg();
            cfg.critical_bps = 10_001;
            assert_eq!(
                set_slash_strategy(&env, symbol_short!("bad"), cfg),
                Err(SlashStrategyError::BpsOutOfRange)
            );
        });
    }

    #[test]
    fn invalid_severity_order_rejected() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let cfg = SlashStrategyConfig {
                minor_bps: 5_000,
                major_bps: 3_000, // minor > major → invalid
                critical_bps: 10_000,
                risk_window_secs: 3_600,
                penalty_window_secs: 3_600,
            };
            assert_eq!(
                set_slash_strategy(&env, symbol_short!("bad"), cfg),
                Err(SlashStrategyError::InvalidSeverityOrder)
            );
        });
    }

    #[test]
    fn zero_risk_window_rejected() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let mut cfg = default_cfg();
            cfg.risk_window_secs = 0;
            assert_eq!(
                set_slash_strategy(&env, symbol_short!("bad"), cfg),
                Err(SlashStrategyError::InvalidRiskWindow)
            );
        });
    }

    #[test]
    fn compute_slash_uses_strategy_thresholds() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            let name = symbol_short!("strat1");
            set_slash_strategy(&env, name.clone(), default_cfg()).unwrap();

            // Minor: 5% of 1_000_000 = 50_000
            assert_eq!(
                compute_slash_amount(&env, name.clone(), 1_000_000, 0).unwrap(),
                50_000
            );
            // Major: 30% of 1_000_000 = 300_000
            assert_eq!(
                compute_slash_amount(&env, name.clone(), 1_000_000, 1).unwrap(),
                300_000
            );
            // Critical: 100% of 1_000_000 = 1_000_000
            assert_eq!(
                compute_slash_amount(&env, name.clone(), 1_000_000, 2).unwrap(),
                1_000_000
            );
        });
    }

    #[test]
    fn unknown_strategy_returns_not_found() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            assert_eq!(
                compute_slash_amount(&env, symbol_short!("none"), 1_000, 0),
                Err(SlashStrategyError::StrategyNotFound)
            );
        });
    }

    #[test]
    fn list_strategies_returns_all_registered() {
        let (env, addr) = setup();
        env.as_contract(&addr, || {
            set_slash_strategy(&env, symbol_short!("s1"), default_cfg()).unwrap();
            set_slash_strategy(&env, symbol_short!("s2"), default_cfg()).unwrap();
            assert_eq!(list_slash_strategies(&env).len(), 2);
        });
    }
}
