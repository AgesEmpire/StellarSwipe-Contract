#![allow(dead_code)]

use crate::admin::require_admin;
use crate::errors::AutoTradeError;
use shared::events::{emit_kyc_status_updated, EvtKycStatusUpdated, SCHEMA_VERSION};
use soroban_sdk::{contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KYCLevel {
    None,
    Basic,
    Enhanced,
    Full,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct KYCData {
    pub kyc_id: String,
    pub level: KYCLevel,
    pub verified: bool,
    pub submitted_at: u64,
    pub verified_at: u64,
}

#[contracttype]
pub enum KYCStorageKey {
    Data(Address),
}

pub fn submit_kyc_verification(
    env: &Env,
    user: &Address,
    kyc_id: String,
    level: KYCLevel,
) -> Result<(), AutoTradeError> {
    user.require_auth();
    let now = env.ledger().timestamp();
    let data = KYCData {
        kyc_id,
        level,
        verified: false,
        submitted_at: now,
        verified_at: 0,
    };
    env.storage()
        .persistent()
        .set(&KYCStorageKey::Data(user.clone()), &data);

    env.events().publish(
        (Symbol::new(env, "kyc_submission"), user.clone()),
        (data.kyc_id.clone(), data.level.clone(), data.submitted_at),
    );
    Ok(())
}

pub fn verify_kyc(
    env: &Env,
    caller: &Address,
    user: &Address,
    verified: bool,
) -> Result<(), AutoTradeError> {
    require_admin(env, caller)?;

    let mut data: KYCData = env
        .storage()
        .persistent()
        .get(&KYCStorageKey::Data(user.clone()))
        .unwrap_or(KYCData {
            kyc_id: String::from_str(env, "unknown"),
            level: KYCLevel::None,
            verified: false,
            submitted_at: 0,
            verified_at: 0,
        });

    data.verified = verified;
    data.verified_at = env.ledger().timestamp();

    env.storage()
        .persistent()
        .set(&KYCStorageKey::Data(user.clone()), &data);

    emit_kyc_status_updated(
        env,
        EvtKycStatusUpdated {
            schema_version: SCHEMA_VERSION,
            user: user.clone(),
            verified,
        },
    );

    env.events()
        .publish((Symbol::new(env, "kyc_verified"), user.clone()), verified);
    Ok(())
}

pub fn get_kyc_data(env: &Env, user: &Address) -> Option<KYCData> {
    env.storage()
        .persistent()
        .get(&KYCStorageKey::Data(user.clone()))
}

pub fn is_kyc_verified(env: &Env, user: &Address) -> bool {
    get_kyc_data(env, user)
        .map(|data| data.verified)
        .unwrap_or(false)
}

pub fn get_user_tier(env: &Env, user: &Address) -> String {
    if let Some(data) = get_kyc_data(env, user) {
        if data.verified {
            match data.level {
                KYCLevel::Full => String::from_str(env, "Platinum"),
                KYCLevel::Enhanced => String::from_str(env, "Gold"),
                KYCLevel::Basic => String::from_str(env, "Silver"),
                KYCLevel::None => String::from_str(env, "Bronze"),
            }
        } else {
            String::from_str(env, "OnboardingPending")
        }
    } else {
        String::from_str(env, "None")
    }
}
