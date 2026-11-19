use soroban_sdk::{contracttype, Address, Env, String, Vec};

use crate::{BridgeError, DataKey};

/// Maximum validators in a set. Bounded before any duplicate scan or
/// membership check iterates the vector (Issue #1298).
pub const MAX_VALIDATOR_SET_SIZE: u32 = 64;
/// Maximum approvals/signatures recorded for one transfer (Issue #1298).
pub const MAX_APPROVALS_PER_TRANSFER: u32 = 64;
/// Maximum encoded length of a single validator signature (Issue #1298).
pub const MAX_SIGNATURE_LENGTH: u32 = 256;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatorApprovalKind {
    LockMint,
    BurnUnlock,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorApproval {
    pub validator: Address,
    pub signature: String,
    pub approved_at: u64,
    pub kind: ValidatorApprovalKind,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorSet {
    pub validators: Vec<Address>,
    pub required_signatures: u32,
}

pub fn build_validator_set(
    _env: &Env,
    validators: Vec<Address>,
    required_signatures: u32,
) -> Result<ValidatorSet, BridgeError> {
    // Size is checked before has_duplicates() (O(n^2)) or any consumer
    // iterates the set, so untrusted input cannot grow verification work.
    if validators.len() > MAX_VALIDATOR_SET_SIZE {
        return Err(BridgeError::ProofLimitExceeded);
    }
    if validators.is_empty()
        || required_signatures == 0
        || required_signatures > validators.len()
        || has_duplicates(&validators)
    {
        return Err(BridgeError::InvalidValidatorSet);
    }

    Ok(ValidatorSet {
        validators,
        required_signatures,
    })
}

pub fn verify_and_record_approval(
    env: &Env,
    validator_set: &ValidatorSet,
    approvals: &mut Vec<ValidatorApproval>,
    validator: Address,
    transfer_id: u64,
    signature: String,
    kind: ValidatorApprovalKind,
) -> Result<(), BridgeError> {
    // All bounds are checked before contains()/the approval loop iterate.
    if validator_set.validators.len() > MAX_VALIDATOR_SET_SIZE
        || approvals.len() >= MAX_APPROVALS_PER_TRANSFER
        || signature.len() > MAX_SIGNATURE_LENGTH
    {
        return Err(BridgeError::ProofLimitExceeded);
    }
    if !validator_set.validators.contains(&validator) {
        return Err(BridgeError::UnauthorizedValidator);
    }

    if env.storage().persistent().has(&DataKey::UsedSignature(
        validator.clone(),
        transfer_id,
        kind,
        signature.clone(),
    )) {
        return Err(BridgeError::SignatureAlreadyUsed);
    }

    for approval in approvals.iter() {
        if approval.validator == validator {
            return Err(BridgeError::SignatureAlreadyUsed);
        }
    }

    approvals.push_back(ValidatorApproval {
        validator: validator.clone(),
        signature: signature.clone(),
        approved_at: env.ledger().timestamp(),
        kind,
    });

    env.storage().persistent().set(
        &DataKey::UsedSignature(validator, transfer_id, kind, signature),
        &true,
    );

    Ok(())
}

pub fn has_quorum(approvals: &Vec<ValidatorApproval>, required_signatures: u32) -> bool {
    approvals.len() >= required_signatures
}

fn has_duplicates(validators: &Vec<Address>) -> bool {
    for i in 0..validators.len() {
        let current = validators.get(i).unwrap();
        for j in (i + 1)..validators.len() {
            if validators.get(j).unwrap() == current {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    #[test]
    fn proof_bounds_cover_empty_maximum_and_over_limit() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);

        // Empty and maximum-size sets are accepted; over-limit is a stable error.
        assert_eq!(
            build_validator_set(&env, Vec::new(&env), 1),
            Err(BridgeError::InvalidValidatorSet)
        );
        let mut max_validators = Vec::new(&env);
        for _ in 0..MAX_VALIDATOR_SET_SIZE {
            max_validators.push_back(Address::generate(&env));
        }
        let set = build_validator_set(&env, max_validators.clone(), 1).unwrap();
        assert_eq!(set.validators.len(), MAX_VALIDATOR_SET_SIZE);

        let mut oversized = max_validators.clone();
        oversized.push_back(Address::generate(&env));
        assert_eq!(
            build_validator_set(&env, oversized, 1),
            Err(BridgeError::ProofLimitExceeded)
        );

        // Maximum approvals (64) are accepted; the 65th is rejected before
        // membership iteration or storage writes.
        let mut approvals = Vec::new(&env);
        for i in 0..MAX_APPROVALS_PER_TRANSFER {
            approvals.push_back(ValidatorApproval {
                validator: set.validators.get(i).unwrap(),
                signature: String::from_str(&env, "sig"),
                approved_at: 1_000,
                kind: ValidatorApprovalKind::LockMint,
            });
        }
        assert!(verify_and_record_approval(
            &env,
            &set,
            &mut approvals,
            set.validators.get(0).unwrap(),
            1,
            String::from_str(&env, "new-sig"),
            ValidatorApprovalKind::LockMint,
        )
        .is_err());
        assert_eq!(approvals.len(), MAX_APPROVALS_PER_TRANSFER);
    }

    #[test]
    fn proof_bound_check_keeps_budget_flat_for_oversized_sets() {
        let env = Env::default();
        let mut small = Vec::new(&env);
        for _ in 0..2 {
            small.push_back(Address::generate(&env));
        }
        env.budget().reset_tracker();
        let _ = build_validator_set(&env, small, 1);
        let small_cost = env.budget().cpu_instruction_cost();

        let mut huge = Vec::new(&env);
        for _ in 0..(MAX_VALIDATOR_SET_SIZE + 64) {
            huge.push_back(Address::generate(&env));
        }
        env.budget().reset_tracker();
        let result = build_validator_set(&env, huge, 1);
        let over_limit_cost = env.budget().cpu_instruction_cost();
        assert_eq!(result, Err(BridgeError::ProofLimitExceeded));
        // The oversized set costs no more than ~2x the small set because the
        // O(n^2) duplicate scan is never entered.
        assert!(over_limit_cost <= small_cost.saturating_mul(2));
    }

    #[test]
    fn duplicate_validators_are_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let validator = Address::generate(&env);
        let mut validators = Vec::new(&env);
        validators.push_back(validator.clone());
        validators.push_back(validator);

        let result = build_validator_set(&env, validators, 2);
        assert_eq!(result, Err(BridgeError::InvalidValidatorSet));
    }
}
