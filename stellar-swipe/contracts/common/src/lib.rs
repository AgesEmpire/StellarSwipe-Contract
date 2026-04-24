#![no_std]

pub mod assets;
pub mod commit_reveal;
pub mod constants;
pub mod emergency;
pub mod health;
pub mod oracle;
pub mod rate_limit;
pub mod replay_protection;

pub use assets::{validate_asset_pair, Asset, AssetPair, AssetPairError};
pub use commit_reveal::hash_trade_intent;
pub use constants::{
    BASIS_POINTS_DENOMINATOR, BASIS_POINTS_DENOMINATOR_I128, CAT_ALL, CAT_SIGNALS, CAT_STAKES,
    CAT_TRADING, LEDGERS_PER_30_DAY_MONTH, LEDGERS_PER_DAY, PLACEHOLDER_ADMIN_STR,
    SECONDS_PER_30_DAY_MONTH, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_WEEK,
    STELLAR_AMOUNT_SCALE,
};
pub use emergency::PauseState;
pub use health::{health_uninitialized, placeholder_admin, HealthStatus};
pub use oracle::{
    oracle_price_to_i128, validate_freshness, IOracleClient, MockOracleClient, OnChainOracleClient,
    OracleError, OraclePrice,
};
pub use rate_limit::{
    check_rate_limit, record_action, set_config as set_rate_limit_config, ActionType, RateLimitConfig,
};
pub use replay_protection::{current_nonce, verify_and_commit, ReplayError};
