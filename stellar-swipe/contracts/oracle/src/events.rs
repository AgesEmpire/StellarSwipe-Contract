use soroban_sdk::{symbol_short, Address, Env, String, Symbol};

use crate::staleness::OracleStatus;

pub fn emit_oracle_removed(env: &Env, oracle: Address, reason: &str) {
    stellar_swipe_common::emit_event!(
        env,
        "oracle_removed",
        (oracle, String::from_str(env, reason))
    );
}

pub fn emit_weight_adjusted(
    env: &Env,
    oracle: Address,
    old_weight: u32,
    new_weight: u32,
    reputation: u32,
) {
    stellar_swipe_common::emit_event!(
        env,
        "oracle_weight_adjusted",
        (oracle, old_weight, new_weight, reputation)
    );
}

pub fn emit_oracle_slashed(env: &Env, oracle: Address, reason: &str, penalty: u32) {
    env.events().publish(
        (Symbol::new(env, "oracle_slashed"),),
        (oracle, String::from_str(env, reason), penalty),
    );
}

pub fn emit_price_submitted(env: &Env, oracle: Address, price: i128) {
    env.events().publish(
        (Symbol::new(env, "oracle_price_submitted"),),
        (oracle, price),
    );
}

pub fn emit_consensus_reached(env: &Env, price: i128, num_oracles: u32) {
    env.events().publish(
        (Symbol::new(env, "oracle_consensus_reached"),),
        (price, num_oracles),
    );
}

pub fn emit_oracle_heartbeat_missed(
    env: &Env,
    status: OracleStatus,
    last_update_ledger: u32,
    ledgers_since_update: u32,
) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("hb_missed")),
        (status, last_update_ledger, ledgers_since_update),
    );
}

pub fn emit_min_source_count_updated(env: &Env, old_count: u32, new_count: u32) {
    env.events().publish(
        (Symbol::new(env, "min_src_count_updated"),),
        (old_count, new_count),
    );
}

pub fn emit_min_confidence_updated(env: &Env, old_min_confidence: u32, new_min_confidence: u32) {
    env.events().publish(
        (Symbol::new(env, "min_confidence_updated"),),
        (old_min_confidence, new_min_confidence),
    );
}

pub fn emit_guardian_set(env: &Env, guardian: Address) {
    env.events()
        .publish((Symbol::new(env, "guardian_set"),), guardian);
}

pub fn emit_guardian_revoked(env: &Env, guardian: Address) {
    env.events()
        .publish((Symbol::new(env, "guardian_revoked"),), guardian);
}

pub fn emit_deviation_breaker_tripped(
    env: &Env,
    pair_code: String,
    previous_price: i128,
    new_price: i128,
    deviation_bps: i128,
) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("dev_trip")),
        (pair_code, previous_price, new_price, deviation_bps),
    );
}

pub fn emit_deviation_breaker_reset(env: &Env, pair_code: String, admin: Address) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("dev_reset")),
        (pair_code, admin),
    );
}
