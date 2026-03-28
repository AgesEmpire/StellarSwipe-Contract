#![no_std]

pub mod assets;
pub mod emergency;
pub mod oracle;

pub use assets::{validate_asset_pair, Asset, AssetPair, AssetPairError};
pub use emergency::{PauseState, CAT_TRADING, CAT_SIGNALS, CAT_STAKES, CAT_ALL};
pub use oracle::{IOracleClient, MockOracleClient, OnChainOracleClient, OracleError, OraclePrice};
