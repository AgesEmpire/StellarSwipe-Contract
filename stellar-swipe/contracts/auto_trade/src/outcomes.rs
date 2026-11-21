//! Bounded execution receipts linking auto-trade outcomes to signal and
//! strategy identifiers, looked up by intent id (issue #1366).

use soroban_sdk::{contracttype, Address, Env, String, Vec};

use crate::errors::AutoTradeError;

/// Maximum number of receipts retained; the oldest is evicted beyond this.
pub const MAX_RECEIPTS: u32 = 500;
/// Maximum length of a failure reason stored in a receipt.
pub const MAX_REASON_LEN: u32 = 64;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum IntentOutcome {
    Succeeded = 0,
    Failed = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReceipt {
    pub intent_id: u64,
    pub user: Address,
    pub signal_id: u64,
    pub strategy_id: u64,
    pub outcome: IntentOutcome,
    pub executed_amount: i128,
    pub executed_price: i128,
    pub reason: Option<String>,
    pub recorded_at: u64,
}

#[contracttype]
enum ReceiptKey {
    Receipt(u64),
    Index,
}

pub fn record(env: &Env, receipt: ExecutionReceipt) -> Result<(), AutoTradeError> {
    if receipt.executed_amount < 0 || receipt.executed_price < 0 {
        return Err(AutoTradeError::InvalidAmount);
    }
    if let Some(r) = &receipt.reason {
        if r.len() > MAX_REASON_LEN {
            return Err(AutoTradeError::InvalidAmount);
        }
    }
    let key = ReceiptKey::Receipt(receipt.intent_id);
    let store = env.storage().persistent();
    // Receipts are terminal and immutable once written.
    if store.has(&key) {
        return Err(AutoTradeError::IntentReceiptExists);
    }
    let mut index: Vec<u64> = store.get(&ReceiptKey::Index).unwrap_or(Vec::new(env));
    if index.len() >= MAX_RECEIPTS {
        if let Some(oldest) = index.pop_front() {
            store.remove(&ReceiptKey::Receipt(oldest));
        }
    }
    index.push_back(receipt.intent_id);
    store.set(&key, &receipt);
    store.set(&ReceiptKey::Index, &index);
    Ok(())
}

pub fn get(env: &Env, intent_id: u64) -> Option<ExecutionReceipt> {
    env.storage().persistent().get(&ReceiptKey::Receipt(intent_id))
}
