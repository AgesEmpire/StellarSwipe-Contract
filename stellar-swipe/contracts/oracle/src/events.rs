// All oracle event structs are defined in the shared common crate.
// This module provides thin wrappers that match the original function signatures
// so existing call sites require minimal changes.

use soroban_sdk::{Address, Env};
use stellar_swipe_common::{
    EvtConsensusReached, EvtOracleRemoved, EvtOracleSlashed, EvtPriceSubmitted, EvtWeightAdjusted,
};

pub fn emit_oracle_removed(env: &Env, oracle: Address, _reason: &str) {
    EvtOracleRemoved { oracle }.publish(env);
}

pub fn emit_weight_adjusted(
    env: &Env,
    oracle: Address,
    old_weight: u32,
    new_weight: u32,
    reputation: u32,
) {
    EvtWeightAdjusted {
        oracle,
        old_weight,
        new_weight,
        reputation,
    }
    .publish(env);
}

pub fn emit_oracle_slashed(env: &Env, oracle: Address, _reason: &str, penalty: u32) {
    EvtOracleSlashed { oracle, penalty }.publish(env);
}

pub fn emit_price_submitted(env: &Env, oracle: Address, price: i128) {
    EvtPriceSubmitted { oracle, price }.publish(env);
}

pub fn emit_consensus_reached(env: &Env, price: i128, num_oracles: u32) {
    EvtConsensusReached { price, num_oracles }.publish(env);
}
