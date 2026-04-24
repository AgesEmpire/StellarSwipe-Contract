use soroban_sdk::{contracttype, Address, Env, String, Vec};

use crate::{BridgeError, DataKey};

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
