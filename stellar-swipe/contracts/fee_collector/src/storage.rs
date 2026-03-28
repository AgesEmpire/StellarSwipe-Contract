use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
pub enum StorageKey {
    Admin,
    Initialized,
    TreasuryBalance(Address), // persistent, per-token
    QueuedWithdrawal,         // instance, single-slot
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
    env.storage()
        .instance()
        .get(&StorageKey::Admin)
        .unwrap()
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage()
        .instance()
        .set(&StorageKey::Admin, admin);
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
    env.storage()
        .instance()
        .get(&StorageKey::QueuedWithdrawal)
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
