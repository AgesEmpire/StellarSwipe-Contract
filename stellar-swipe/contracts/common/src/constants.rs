//! Shared protocol constants used across StellarSwipe contracts.

/// One basis-point denominator: 10_000 bps = 100%.
pub const BASIS_POINTS_DENOMINATOR: u32 = 10_000;

/// `BASIS_POINTS_DENOMINATOR` as `i128` for arithmetic on token amounts and prices.
pub const BASIS_POINTS_DENOMINATOR_I128: i128 = BASIS_POINTS_DENOMINATOR as i128;

/// Stellar assets conventionally use 7 decimal places.
pub const STELLAR_AMOUNT_SCALE: i128 = 10_000_000;

/// Number of seconds in one hour.
pub const SECONDS_PER_HOUR: u64 = 3_600;

/// Number of seconds in one day.
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Number of seconds in one week.
pub const SECONDS_PER_WEEK: u64 = 7 * SECONDS_PER_DAY;

/// Number of seconds in a 30-day protocol month.
pub const SECONDS_PER_30_DAY_MONTH: u64 = 30 * SECONDS_PER_DAY;

/// Approximate ledger close time used when converting time windows to ledgers.
pub const LEDGER_CLOSE_TIME_SECONDS: u32 = 5;

/// Approximate number of ledgers in one day.
pub const LEDGERS_PER_DAY: u32 = (SECONDS_PER_DAY as u32) / LEDGER_CLOSE_TIME_SECONDS;

/// Approximate number of ledgers in a 30-day month.
pub const LEDGERS_PER_30_DAY_MONTH: u32 = 30 * LEDGERS_PER_DAY;

/// Shared pause category for trading operations.
pub const CAT_TRADING: &str = "trading";

/// Shared pause category for signal-management operations.
pub const CAT_SIGNALS: &str = "signals";

/// Shared pause category for staking operations.
pub const CAT_STAKES: &str = "stakes";

/// Shared umbrella pause category that gates every subsystem.
pub const CAT_ALL: &str = "all";

/// Stellar protocol "dead" account used as a safe placeholder admin value.
pub const PLACEHOLDER_ADMIN_STR: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
