#![no_std]

pub mod assets;
pub mod constants;
pub mod emergency;
pub mod events;
pub mod health;
 refactor/157-shared-constants

pub mod rate_limit;
 feat/replay-protection
pub mod replay_protection;

pub use assets::{validate_asset_pair, Asset, AssetPair, AssetPairError};
pub use emergency::{PauseState, CAT_TRADING, CAT_SIGNALS, CAT_STAKES, CAT_ALL};
pub use rate_limit::{ActionType, RateLimitConfig, check_rate_limit, record_action, set_config as set_rate_limit_config};
pub use replay_protection::{ReplayError, current_nonce, verify_and_commit};

 main
pub mod oracle;
pub mod rate_limit;

pub use assets::{validate_asset_pair, Asset, AssetPair, AssetPairError};
pub use constants::{
    BASIS_POINTS_DENOMINATOR, BASIS_POINTS_DENOMINATOR_I128, CAT_ALL, CAT_SIGNALS, CAT_STAKES,
    CAT_TRADING, LEDGERS_PER_30_DAY_MONTH, LEDGERS_PER_DAY, PLACEHOLDER_ADMIN_STR,
    SECONDS_PER_30_DAY_MONTH, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_WEEK,
    STELLAR_AMOUNT_SCALE,
};
pub use emergency::PauseState;
pub use events::*;
pub use health::{health_uninitialized, placeholder_admin, HealthStatus};
pub use oracle::{IOracleClient, MockOracleClient, OnChainOracleClient, OracleError, OraclePrice};
pub use rate_limit::{
    check_rate_limit, record_action, set_config as set_rate_limit_config, ActionType,
    RateLimitConfig,
};
 refactor/157-shared-constants

pub use oracle::{IOracleClient, MockOracleClient, OnChainOracleClient, OracleError, OraclePrice};
 main
 main
