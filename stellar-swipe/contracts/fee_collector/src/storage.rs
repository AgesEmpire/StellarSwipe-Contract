use soroban_sdk::{contracttype, Address, Env};

pub const MAX_FEE_RATE_BPS: u32 = 100; // 1%
pub const MIN_FEE_RATE_BPS: u32 = 1; // 0.01%
pub const DEFAULT_FEE_RATE_BPS: u32 = 30; // 0.3%
pub const DEFAULT_BURN_RATE_BPS: u32 = 1_000; // 10%
pub const MAX_BURN_RATE_BPS: u32 = 10_000; // 100%
pub const LEDGERS_PER_MONTH_APPROX: u32 = 518_400; // ~30 days at ~5 seconds per ledger
pub const SILVER_TIER_VOLUME_USD: i128 = 10_000 * 10_000_000; // $10k, 7 decimals
pub const GOLD_TIER_VOLUME_USD: i128 = 50_000 * 10_000_000; // $50k, 7 decimals
pub const SILVER_DISCOUNT_BPS: u32 = 5;
pub const GOLD_DISCOUNT_BPS: u32 = 10;

#[contracttype]
pub enum StorageKey {
    Admin,
    Initialized,
    OracleContract,
    TreasuryBalance(Address),              // persistent, per-token
    QueuedWithdrawal,                      // instance, single-slot
    FeeRate,                               // instance, current fee rate in bps
    BurnRate,                              // instance, burn rate in bps
    ProviderPendingFees(Address, Address), // persistent, per (provider, token)
    MonthlyTradeVolume(Address),           // persistent, per user
    /// Accumulated fee shares per provider per day (day = unix_timestamp / SECONDS_PER_DAY).
    ProviderDailyFeeShares(Address, u64),
    /// Day number of the provider's first recorded earnings (for ALL_TIME period_start).
    ProviderEarningsFirstDay(Address),
}

#[contracttype]
#[derive(Clone)]
pub struct QueuedWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub queued_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct MonthlyTradeVolume {
    pub month_bucket: u32,
    pub volume_usd: i128,
}

// --- Admin ---

pub fn get_admin(env: &Env) -> Address {
    env.storage().instance().get(&StorageKey::Admin).unwrap()
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&StorageKey::Admin, admin);
}

// --- Initialized ---

pub fn is_initialized(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&StorageKey::Initialized)
        .unwrap_or(false)
}

pub fn set_initialized(env: &Env) {
    env.storage()
        .instance()
        .set(&StorageKey::Initialized, &true);
}

// --- Oracle Contract ---

pub fn get_oracle_contract(env: &Env) -> Option<Address> {
    env.storage().instance().get(&StorageKey::OracleContract)
}

pub fn set_oracle_contract(env: &Env, contract: &Address) {
    env.storage()
        .instance()
        .set(&StorageKey::OracleContract, contract);
}

// --- Treasury Balance ---

pub fn get_treasury_balance(env: &Env, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::TreasuryBalance(token.clone()))
        .unwrap_or(0i128)
}

pub fn set_treasury_balance(env: &Env, token: &Address, balance: i128) {
    env.storage()
        .persistent()
        .set(&StorageKey::TreasuryBalance(token.clone()), &balance);
}

// --- Queued Withdrawal ---

pub fn get_queued_withdrawal(env: &Env) -> Option<QueuedWithdrawal> {
    env.storage().instance().get(&StorageKey::QueuedWithdrawal)
}

pub fn set_queued_withdrawal(env: &Env, withdrawal: &QueuedWithdrawal) {
    env.storage()
        .instance()
        .set(&StorageKey::QueuedWithdrawal, withdrawal);
}

pub fn remove_queued_withdrawal(env: &Env) {
    env.storage()
        .instance()
        .remove(&StorageKey::QueuedWithdrawal);
}

// --- Fee Rate ---

pub fn get_fee_rate(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::FeeRate)
        .unwrap_or(DEFAULT_FEE_RATE_BPS)
}

pub fn set_fee_rate(env: &Env, rate: u32) {
    env.storage().instance().set(&StorageKey::FeeRate, &rate);
}

// --- Burn Rate ---

pub fn get_burn_rate(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::BurnRate)
        .unwrap_or(DEFAULT_BURN_RATE_BPS)
}

pub fn set_burn_rate(env: &Env, rate: u32) {
    env.storage().instance().set(&StorageKey::BurnRate, &rate);
}

// --- Provider Pending Fees ---

pub fn get_pending_fees(env: &Env, provider: &Address, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::ProviderPendingFees(
            provider.clone(),
            token.clone(),
        ))
        .unwrap_or(0i128)
}

pub fn set_pending_fees(env: &Env, provider: &Address, token: &Address, amount: i128) {
    env.storage().persistent().set(
        &StorageKey::ProviderPendingFees(provider.clone(), token.clone()),
        &amount,
    );
}

// --- Monthly Trade Volume ---

pub fn get_monthly_trade_volume(env: &Env, user: &Address) -> Option<MonthlyTradeVolume> {
    env.storage()
        .persistent()
        .get(&StorageKey::MonthlyTradeVolume(user.clone()))
}

pub fn set_monthly_trade_volume(env: &Env, user: &Address, volume: &MonthlyTradeVolume) {
    env.storage()
        .persistent()
        .set(&StorageKey::MonthlyTradeVolume(user.clone()), volume);
}

pub fn remove_monthly_trade_volume(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&StorageKey::MonthlyTradeVolume(user.clone()));
}

// --- Provider Daily Fee Shares (Issue #366) ---

pub fn get_provider_daily_fee_shares(env: &Env, provider: &Address, day: u64) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::ProviderDailyFeeShares(provider.clone(), day))
        .unwrap_or(0i128)
}

pub fn add_provider_daily_fee_shares(env: &Env, provider: &Address, day: u64, amount: i128) {
    let key = StorageKey::ProviderDailyFeeShares(provider.clone(), day);
    let current: i128 = env.storage().persistent().get(&key).unwrap_or(0i128);
    let updated = current.saturating_add(amount);
    env.storage().persistent().set(&key, &updated);

    // Record first earnings day if not yet set
    let first_key = StorageKey::ProviderEarningsFirstDay(provider.clone());
    if !env.storage().persistent().has(&first_key) {
        env.storage().persistent().set(&first_key, &day);
    }
}

pub fn get_provider_earnings_first_day(env: &Env, provider: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&StorageKey::ProviderEarningsFirstDay(provider.clone()))
}
