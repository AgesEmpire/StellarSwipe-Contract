use soroban_sdk::{contracttype, Address, Env};

pub const MAX_FEE_RATE_BPS: u32 = 100; // 1%
pub const MIN_FEE_RATE_BPS: u32 = 1; // 0.01%
pub const DEFAULT_FEE_RATE_BPS: u32 = 30; // 0.3%

#[contracttype]
pub enum StorageKey {
    Admin,
    Initialized,
    TreasuryBalance(Address),          // persistent, per-token
    QueuedWithdrawal,                  // instance, single-slot
    FeeRate,                           // instance, current fee rate in bps
    EpochFees(Address),                // persistent, per-token epoch accumulator
    ProviderBalance(Address, Address), // persistent, (provider, token)
}

#[contracttype]
#[derive(Clone)]
pub struct QueuedWithdrawal {
    pub recipient: Address,
    pub token: Address,
    pub amount: i128,
    pub queued_at: u64,
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

// --- Epoch Fee Accumulator ---

pub fn get_epoch_fees(env: &Env, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::EpochFees(token.clone()))
        .unwrap_or(0i128)
}

pub fn set_epoch_fees(env: &Env, token: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&StorageKey::EpochFees(token.clone()), &amount);
}

// --- Provider Balance ---

pub fn get_provider_balance(env: &Env, provider: &Address, token: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&StorageKey::ProviderBalance(
            provider.clone(),
            token.clone(),
        ))
        .unwrap_or(0i128)
}

pub fn set_provider_balance(env: &Env, provider: &Address, token: &Address, balance: i128) {
    env.storage().persistent().set(
        &StorageKey::ProviderBalance(provider.clone(), token.clone()),
        &balance,
    );
}
