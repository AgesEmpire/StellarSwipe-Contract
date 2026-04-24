use soroban_sdk::{Address, Bytes, Env, String};
use crate::types::{CrossChainSignal, SyncStatus, AddressMapping};
use crate::StorageKey;

pub fn register_address(
    env: &Env,
    stellar_address: Address,
    source_chain: String,
    source_address: String,
    _proof: Bytes,
) {
    let mapping = AddressMapping {
        source_chain: source_chain.clone(),
        source_address: source_address.clone(),
        stellar_address: stellar_address.clone(),
        is_verified: true,
    };
    
    env.storage().persistent().set(
        &StorageKey::AddressMappings(source_chain, source_address),
        &mapping
    );
}

pub fn get_address_mapping(
    env: &Env,
    source_chain: &String,
    source_address: &String,
) -> Option<AddressMapping> {
    env.storage().persistent().get(&StorageKey::AddressMappings(source_chain.clone(), source_address.clone()))
}

pub fn store_cross_chain_signal(
    env: &Env,
    source_chain: String,
    source_id: String,
    signal: CrossChainSignal,
) {
    env.storage().persistent().set(
        &StorageKey::CrossChainSignals(source_chain, source_id),
        &signal
    );
}

pub fn get_cross_chain_signal(
    env: &Env,
    source_chain: &String,
    source_id: &String,
) -> Option<CrossChainSignal> {
    env.storage().persistent().get(&StorageKey::CrossChainSignals(source_chain.clone(), source_id.clone()))
}

pub fn verify_proof(_env: &Env, _proof: &Bytes) -> bool {
    // Placeholder for actual proof verification (e.g., Merkle proof, signature)
    true
}
