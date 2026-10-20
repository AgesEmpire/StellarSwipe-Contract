//! Per-trade escrow sub-account pattern.
//!
//! Each copy-trade has its funds held in an isolated, trade-specific escrow
//! record from the moment it is initiated until settlement or cancellation.
//! No shared pool is used; only the matching trade_id can release funds.

#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol};

use crate::errors::AutoTradeError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EscrowStatus {
    Active,
    Settled,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowRecord {
    pub trade_id: BytesN<32>,
    pub originator: Address,
    pub asset: u32,
    pub amount: i128,
    pub status: EscrowStatus,
    pub created_at: u64,
}

#[contracttype]
pub enum EscrowKey {
    Escrow(BytesN<32>),
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

pub fn get_escrow(env: &Env, trade_id: &BytesN<32>) -> Option<EscrowRecord> {
    env.storage()
        .persistent()
        .get(&EscrowKey::Escrow(trade_id.clone()))
}

fn save_escrow(env: &Env, record: &EscrowRecord) {
    env.storage()
        .persistent()
        .set(&EscrowKey::Escrow(record.trade_id.clone()), record);
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// Move `amount` of `asset` into an isolated escrow record for `trade_id`.
///
/// The funds are not transferred via a token contract here — this function
/// records the escrowed amount in trade-specific storage, isolating it from
/// every other trade's funds. Call sites must have already verified the user
/// has sufficient balance before initiating (see `sdex::has_sufficient_balance`).
///
/// Errors:
/// - `InvalidAmount` — amount ≤ 0.
/// - `EscrowAlreadyActive` (alias `PositionAlreadyExists`) — an active escrow
///   already exists for this trade_id, preventing double-initiation.
pub fn initiate_escrow(
    env: &Env,
    trade_id: BytesN<32>,
    originator: Address,
    asset: u32,
    amount: i128,
) -> Result<(), AutoTradeError> {
    if amount <= 0 {
        return Err(AutoTradeError::InvalidAmount);
    }

    // Guard: reject if an active escrow already exists (prevents double-init).
    if let Some(existing) = get_escrow(env, &trade_id) {
        if existing.status == EscrowStatus::Active {
            return Err(AutoTradeError::EscrowAlreadyActive);
        }
    }

    let record = EscrowRecord {
        trade_id: trade_id.clone(),
        originator,
        asset,
        amount,
        status: EscrowStatus::Active,
        created_at: env.ledger().timestamp(),
    };
    save_escrow(env, &record);

    #[allow(deprecated)]
    env.events()
        .publish((Symbol::new(env, "escrow_initiated"), trade_id), amount);

    Ok(())
}

/// Release escrowed funds to `destination` and mark the escrow as Settled.
///
/// Errors:
/// - `EscrowNotFound` (alias `StrategyNotFound`) — no escrow for `trade_id`.
/// - `EscrowAlreadyClosed` (alias `SystemError`) — already Settled or Cancelled;
///   prevents double-release.
pub fn settle_escrow(
    env: &Env,
    trade_id: &BytesN<32>,
    destination: Address,
) -> Result<EscrowRecord, AutoTradeError> {
    let mut record = get_escrow(env, trade_id).ok_or(AutoTradeError::EscrowNotFound)?;

    match record.status {
        EscrowStatus::Active => {}
        // Settled or Cancelled — reject to prevent double-release.
        EscrowStatus::Settled | EscrowStatus::Cancelled => {
            return Err(AutoTradeError::EscrowAlreadyClosed)
        }
    }

    record.status = EscrowStatus::Settled;
    save_escrow(env, &record);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "escrow_settled"), trade_id.clone()),
        (record.amount, destination),
    );

    Ok(record)
}

/// Return escrowed funds to the originator and mark the escrow as Cancelled.
///
/// Errors:
/// - `EscrowNotFound` (alias `StrategyNotFound`) — no escrow for `trade_id`.
/// - `EscrowAlreadyClosed` (alias `SystemError`) — already Settled or Cancelled.
pub fn cancel_escrow(env: &Env, trade_id: &BytesN<32>) -> Result<EscrowRecord, AutoTradeError> {
    let mut record = get_escrow(env, trade_id).ok_or(AutoTradeError::EscrowNotFound)?;

    match record.status {
        EscrowStatus::Active => {}
        EscrowStatus::Settled | EscrowStatus::Cancelled => {
            return Err(AutoTradeError::EscrowAlreadyClosed)
        }
    }

    record.status = EscrowStatus::Cancelled;
    save_escrow(env, &record);

    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "escrow_cancelled"), trade_id.clone()),
        (record.amount, record.originator.clone()),
    );

    Ok(record)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn make_trade_id(env: &Env, seed: u8) -> BytesN<32> {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        BytesN::from_array(env, &bytes)
    }

    // ── initiate_escrow ──────────────────────────────────────────────────────

    #[test]
    fn test_initiate_escrow_stores_record() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let trade_id = make_trade_id(&env, 1);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_id.clone(), user.clone(), 42u32, 1_000).unwrap();
            let record = get_escrow(&env, &trade_id).expect("record must exist");
            assert_eq!(record.amount, 1_000);
            assert_eq!(record.asset, 42u32);
            assert_eq!(record.originator, user);
            assert_eq!(record.status, EscrowStatus::Active);
        });
    }

    #[test]
    fn test_initiate_escrow_rejects_zero_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let trade_id = make_trade_id(&env, 2);

        env.as_contract(&contract, || {
            let result = initiate_escrow(&env, trade_id, user, 1u32, 0);
            assert_eq!(result, Err(AutoTradeError::InvalidAmount));
        });
    }

    #[test]
    fn test_double_initiation_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let trade_id = make_trade_id(&env, 3);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_id.clone(), user.clone(), 1u32, 500).unwrap();
            let result = initiate_escrow(&env, trade_id, user, 1u32, 500);
            assert_eq!(result, Err(AutoTradeError::EscrowAlreadyActive));
        });
    }

    // ── settle_escrow ────────────────────────────────────────────────────────

    #[test]
    fn test_settle_escrow_marks_settled() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let dest = Address::generate(&env);
        let trade_id = make_trade_id(&env, 4);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_id.clone(), user, 1u32, 2_000).unwrap();
            let record = settle_escrow(&env, &trade_id, dest).unwrap();
            assert_eq!(record.status, EscrowStatus::Settled);
            assert_eq!(record.amount, 2_000);
        });
    }

    #[test]
    fn test_settle_nonexistent_escrow_errors() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let dest = Address::generate(&env);
        let trade_id = make_trade_id(&env, 5);

        env.as_contract(&contract, || {
            let result = settle_escrow(&env, &trade_id, dest);
            assert_eq!(result, Err(AutoTradeError::EscrowNotFound));
        });
    }

    // ── cancel_escrow ────────────────────────────────────────────────────────

    #[test]
    fn test_cancel_escrow_returns_funds_to_originator() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let trade_id = make_trade_id(&env, 6);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_id.clone(), user.clone(), 1u32, 3_000).unwrap();
            let record = cancel_escrow(&env, &trade_id).unwrap();
            assert_eq!(record.status, EscrowStatus::Cancelled);
            assert_eq!(record.originator, user);
            assert_eq!(record.amount, 3_000);
        });
    }

    #[test]
    fn test_cancel_nonexistent_escrow_errors() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let trade_id = make_trade_id(&env, 7);

        env.as_contract(&contract, || {
            let result = cancel_escrow(&env, &trade_id);
            assert_eq!(result, Err(AutoTradeError::EscrowNotFound));
        });
    }

    // ── double-release guard ─────────────────────────────────────────────────

    #[test]
    fn test_double_settle_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let dest = Address::generate(&env);
        let trade_id = make_trade_id(&env, 8);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_id.clone(), user, 1u32, 1_000).unwrap();
            settle_escrow(&env, &trade_id, dest.clone()).unwrap();
            // Second settle must fail.
            let result = settle_escrow(&env, &trade_id, dest);
            assert_eq!(result, Err(AutoTradeError::EscrowAlreadyClosed));
        });
    }

    #[test]
    fn test_settle_after_cancel_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user = Address::generate(&env);
        let dest = Address::generate(&env);
        let trade_id = make_trade_id(&env, 9);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_id.clone(), user, 1u32, 1_000).unwrap();
            cancel_escrow(&env, &trade_id).unwrap();
            let result = settle_escrow(&env, &trade_id, dest);
            assert_eq!(result, Err(AutoTradeError::EscrowAlreadyClosed));
        });
    }

    #[test]
    fn test_unrelated_trade_cannot_affect_another_escrow() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(crate::AutoTradeContract, ());
        let user_a = Address::generate(&env);
        let user_b = Address::generate(&env);
        let trade_a = make_trade_id(&env, 10);
        let trade_b = make_trade_id(&env, 11);

        env.as_contract(&contract, || {
            initiate_escrow(&env, trade_a.clone(), user_a.clone(), 1u32, 500).unwrap();
            initiate_escrow(&env, trade_b.clone(), user_b.clone(), 1u32, 700).unwrap();

            // Cancelling trade_b must not affect trade_a's record.
            cancel_escrow(&env, &trade_b).unwrap();

            let record_a = get_escrow(&env, &trade_a).unwrap();
            assert_eq!(record_a.status, EscrowStatus::Active);
            assert_eq!(record_a.amount, 500);
        });
    }
}
