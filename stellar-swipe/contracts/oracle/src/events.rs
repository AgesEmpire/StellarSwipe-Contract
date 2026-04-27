use soroban_sdk::{Address, Env, String, Symbol};

use crate::staleness::OracleStatus;

pub fn emit_oracle_removed(env: &Env, oracle: Address, reason: &str) {
    env.events().publish(
        (Symbol::new(env, "oracle_removed"),),
        (oracle, String::from_str(env, reason)),
    );
}

pub fn emit_weight_adjusted(
    env: &Env,
    oracle: Address,
    old_weight: u32,
    new_weight: u32,
    reputation: u32,
) {
    env.events().publish(
        (Symbol::new(env, "oracle_weight_adjusted"),),
        (oracle, old_weight, new_weight, reputation),
    );
}

pub fn emit_oracle_slashed(env: &Env, oracle: Address, reason: &str, penalty: u32) {
    env.events().publish(
        (Symbol::new(env, "oracle_slashed"),),
        (oracle, String::from_str(env, reason), penalty),
    );
}

pub fn emit_price_submitted(env: &Env, oracle: Address, price: i128) {
    env.events()
        .publish((Symbol::new(env, "oracle_price_submitted"),), (oracle, price));
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
    env.events().publish(
        (symbol_short!("oracle"), symbol_short!("hb_missed")),
        (status, last_update_ledger, ledgers_since_update),
    );
}
