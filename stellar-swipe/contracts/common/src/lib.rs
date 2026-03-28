#![no_std]

pub mod assets;
pub mod emergency;
pub mod health;

pub use assets::{validate_asset_pair, Asset, AssetPair, AssetPairError};
pub use emergency::{PauseState, CAT_TRADING, CAT_SIGNALS, CAT_STAKES, CAT_ALL};
pub use health::{health_uninitialized, placeholder_admin, HealthStatus, PLACEHOLDER_ADMIN_STR};
