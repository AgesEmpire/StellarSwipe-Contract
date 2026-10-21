//! Bridge fee escrow with timeout release (#1127).
//!
//! Each escrow records the payer, the claimant (relayer) and an expiry.
//! Successful execution releases the fee to the claimant; once the expiry has
//! passed without execution, the payer may reclaim it. Both paths are
//! idempotent-safe: an escrow reaches exactly one final disposition.

use soroban_sdk::{contracterror, contracttype, symbol_short, Address, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum EscrowError {
    EscrowExists = 1,
    EscrowNotFound = 2,
    InvalidAmount = 3,
    InvalidExpiry = 4,
    Unauthorized = 5,
    NotExpired = 6,
    Expired = 7,
    AlreadySettled = 8,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscrowDisposition {
    Pending,
    ReleasedToClaimant,
    RefundedToPayer,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeEscrow {
    pub message_id: u64,
    pub payer: Address,
    pub claimant: Address,
    pub amount: i128,
    pub expiry: u64,
    pub disposition: EscrowDisposition,
}

#[contracttype]
enum EscrowKey {
    Escrow(u64),
}

fn load(env: &Env, id: u64) -> Result<FeeEscrow, EscrowError> {
    env.storage()
        .persistent()
        .get(&EscrowKey::Escrow(id))
        .ok_or(EscrowError::EscrowNotFound)
}

fn save(env: &Env, e: &FeeEscrow) {
    env.storage()
        .persistent()
        .set(&EscrowKey::Escrow(e.message_id), e);
}

pub fn get_escrow(env: &Env, id: u64) -> Option<FeeEscrow> {
    env.storage().persistent().get(&EscrowKey::Escrow(id))
}

pub fn create_escrow(
    env: &Env,
    message_id: u64,
    payer: Address,
    claimant: Address,
    amount: i128,
    expiry: u64,
) -> Result<FeeEscrow, EscrowError> {
    payer.require_auth();
    if amount <= 0 {
        return Err(EscrowError::InvalidAmount);
    }
    if expiry <= env.ledger().timestamp() {
        return Err(EscrowError::InvalidExpiry);
    }
    if env.storage().persistent().has(&EscrowKey::Escrow(message_id)) {
        return Err(EscrowError::EscrowExists);
    }
    let e = FeeEscrow {
        message_id,
        payer,
        claimant,
        amount,
        expiry,
        disposition: EscrowDisposition::Pending,
    };
    save(env, &e);
    env.events()
        .publish((symbol_short!("esc_new"), message_id), (amount, expiry));
    Ok(e)
}

/// Called on successful message execution; pays the claimant and blocks any
/// later refund.
pub fn release_on_execution(env: &Env, message_id: u64) -> Result<FeeEscrow, EscrowError> {
    let mut e = load(env, message_id)?;
    e.claimant.require_auth();
    if e.disposition != EscrowDisposition::Pending {
        return Err(EscrowError::AlreadySettled);
    }
    if env.ledger().timestamp() >= e.expiry {
        return Err(EscrowError::Expired);
    }
    e.disposition = EscrowDisposition::ReleasedToClaimant;
    save(env, &e);
    env.events().publish(
        (symbol_short!("esc_fin"), message_id),
        (e.disposition, e.claimant.clone(), e.amount),
    );
    Ok(e)
}

/// Timeout path: after expiry the payer reclaims the fee. Repeat calls return
/// `AlreadySettled` without side effects.
pub fn refund_on_timeout(
    env: &Env,
    caller: &Address,
    message_id: u64,
) -> Result<FeeEscrow, EscrowError> {
    caller.require_auth();
    let mut e = load(env, message_id)?;
    if *caller != e.payer {
        return Err(EscrowError::Unauthorized);
    }
    if e.disposition != EscrowDisposition::Pending {
        return Err(EscrowError::AlreadySettled);
    }
    if env.ledger().timestamp() < e.expiry {
        return Err(EscrowError::NotExpired);
    }
    e.disposition = EscrowDisposition::RefundedToPayer;
    save(env, &e);
    env.events().publish(
        (symbol_short!("esc_fin"), message_id),
        (e.disposition, e.payer.clone(), e.amount),
    );
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, Env};

    #[contract]
    struct Dummy;

    #[test]
    fn timeout_refund_and_execution_exclusive() {
        let env = Env::default();
        env.mock_all_auths();
        let cid = env.register(Dummy, ());
        env.as_contract(&cid, || {
            let payer = Address::generate(&env);
            let relayer = Address::generate(&env);
            create_escrow(&env, 1, payer.clone(), relayer.clone(), 10, 100).unwrap();
            create_escrow(&env, 2, payer.clone(), relayer.clone(), 10, 100).unwrap();
            assert_eq!(refund_on_timeout(&env, &payer, 1), Err(EscrowError::NotExpired));
            release_on_execution(&env, 2).unwrap();
            env.ledger().with_mut(|l| l.timestamp = 100);
            assert_eq!(refund_on_timeout(&env, &relayer, 1), Err(EscrowError::Unauthorized));
            refund_on_timeout(&env, &payer, 1).unwrap();
            assert_eq!(refund_on_timeout(&env, &payer, 1), Err(EscrowError::AlreadySettled));
            assert_eq!(refund_on_timeout(&env, &payer, 2), Err(EscrowError::AlreadySettled));
            assert_eq!(release_on_execution(&env, 1), Err(EscrowError::AlreadySettled));
        });
    }
}
