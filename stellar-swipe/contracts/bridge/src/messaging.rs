//! Cross-chain message passing protocol.
//!
//! Enables arbitrary message passing between Stellar and other chains,
//! supporting callbacks, retries, and validator-based relay tracking.

#![allow(dead_code)]

use crate::governance::get_bridge_validators;
use crate::monitoring::ChainId;
use soroban_sdk::{contracttype, Address, Bytes, Env, String, Symbol};
use stellar_swipe_common::SECONDS_PER_DAY;

// ── Constants ────────────────────────────────────────────────────────────────

pub const MAX_MESSAGE_SIZE: u32 = 4096;
pub const MESSAGE_TIMEOUT: u64 = SECONDS_PER_DAY; // 24 h

// ── Types ────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageStatus {
    Pending,
    Relayed,
    Delivered,
    Failed,
    CallbackReceived,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossChainMessage {
    pub id: u64,
    pub source_chain: ChainId,
    pub target_chain: ChainId,
    pub sender: Address,
    pub target_contract: String,
    pub payload: Bytes,
    pub gas_limit: u64,
    pub callback_required: bool,
    pub status: MessageStatus,
    pub sent_at: u64,
    pub delivered_at: Option<u64>,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
pub enum MessagingKey {
    Message(u64),
    NextMessageId,
    BridgeForChain(u32),
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn get_message(env: &Env, id: u64) -> Result<CrossChainMessage, String> {
    env.storage()
        .persistent()
        .get(&MessagingKey::Message(id))
        .ok_or_else(|| String::from_str(env, "Message not found"))
}

fn save_message(env: &Env, msg: &CrossChainMessage) {
    env.storage()
        .persistent()
        .set(&MessagingKey::Message(msg.id), msg);
}

fn remove_message(env: &Env, id: u64) {
    env.storage()
        .persistent()
        .remove(&MessagingKey::Message(id));
}

fn next_message_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&MessagingKey::NextMessageId)
        .unwrap_or(1u64);
    env.storage()
        .persistent()
        .set(&MessagingKey::NextMessageId, &(id + 1));
    id
}

/// Register which bridge_id handles a given chain so relay can look it up.
pub fn register_bridge_for_chain(env: &Env, chain_id: ChainId, bridge_id: u64) {
    env.storage()
        .persistent()
        .set(&MessagingKey::BridgeForChain(chain_id as u32), &bridge_id);
}

fn bridge_id_for_chain(env: &Env, chain_id: ChainId) -> Result<u64, String> {
    env.storage()
        .persistent()
        .get(&MessagingKey::BridgeForChain(chain_id as u32))
        .ok_or_else(|| String::from_str(env, "No bridge for chain"))
}

// ── Proof verification stubs ──────────────────────────────────────────────────
// In production these verify Merkle / ZK proofs from the target chain.
// Here any non-empty Bytes is accepted so the logic can be exercised in tests.

fn verify_delivery_proof(
    env: &Env,
    _chain: ChainId,
    _id: u64,
    proof: &Bytes,
) -> Result<(), String> {
    if proof.is_empty() {
        return Err(String::from_str(env, "Empty delivery proof"));
    }
    Ok(())
}

fn verify_callback_proof(
    env: &Env,
    _chain: ChainId,
    _id: u64,
    _payload: &Bytes,
    proof: &Bytes,
) -> Result<(), String> {
    if proof.is_empty() {
        return Err(String::from_str(env, "Empty callback proof"));
    }
    Ok(())
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Send a cross-chain message from Stellar to `target_chain`.
/// Returns the assigned message id.
pub fn send_cross_chain_message(
    env: &Env,
    sender: Address,
    target_chain: ChainId,
    target_contract: String,
    payload: Bytes,
    gas_limit: u64,
    callback_required: bool,
) -> Result<u64, String> {
    sender.require_auth();

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(String::from_str(env, "Payload too large"));
    }

    let id = next_message_id(env);

    let msg = CrossChainMessage {
        id,
        source_chain: ChainId::Stellar,
        target_chain,
        sender: sender.clone(),
        target_contract,
        payload,
        gas_limit,
        callback_required,
        status: MessageStatus::Pending,
        sent_at: env.ledger().timestamp(),
        delivered_at: None,
    };

    save_message(env, &msg);

    env.events().publish(
        (Symbol::new(env, "msg_sent"), id),
        (target_chain as u32, sender),
    );

    Ok(id)
}

/// Validator records that a message has been relayed to the target chain.
pub fn relay_message_to_target_chain(
    env: &Env,
    message_id: u64,
    validator: Address,
    _relay_proof: Bytes,
) -> Result<(), String> {
    validator.require_auth();

    let mut msg = get_message(env, message_id)?;

    if msg.status != MessageStatus::Pending {
        return Err(String::from_str(env, "Already relayed"));
    }

    let bridge_id = bridge_id_for_chain(env, msg.target_chain)?;
    let validators = get_bridge_validators(env, bridge_id)?;
    if !validators.contains(&validator) {
        return Err(String::from_str(env, "Not authorized validator"));
    }

    msg.status = MessageStatus::Relayed;
    save_message(env, &msg);

    env.events()
        .publish((Symbol::new(env, "msg_relayed"), message_id), validator);

    Ok(())
}

/// Confirm that the message was executed on the target chain.
pub fn confirm_message_delivery(
    env: &Env,
    message_id: u64,
    delivery_proof: Bytes,
) -> Result<(), String> {
    let mut msg = get_message(env, message_id)?;

    if msg.status != MessageStatus::Relayed {
        return Err(String::from_str(env, "Message not relayed"));
    }

    verify_delivery_proof(env, msg.target_chain, message_id, &delivery_proof)?;

    let now = env.ledger().timestamp();
    msg.status = MessageStatus::Delivered;
    msg.delivered_at = Some(now);
    save_message(env, &msg);

    env.events()
        .publish((Symbol::new(env, "msg_delivered"), message_id), now);

    if !msg.callback_required {
        remove_message(env, message_id);
    }

    Ok(())
}

/// Receive a callback from the target chain after message execution.
pub fn receive_message_callback(
    env: &Env,
    original_message_id: u64,
    callback_payload: Bytes,
    callback_proof: Bytes,
) -> Result<(), String> {
    let mut msg = get_message(env, original_message_id)?;

    if !msg.callback_required {
        return Err(String::from_str(env, "Callback not expected"));
    }
    if msg.status != MessageStatus::Delivered {
        return Err(String::from_str(env, "Message not delivered"));
    }

    verify_callback_proof(
        env,
        msg.target_chain,
        original_message_id,
        &callback_payload,
        &callback_proof,
    )?;

    env.events().publish(
        (Symbol::new(env, "callback_received"), original_message_id),
        (msg.sender.clone(), callback_payload),
    );

    msg.status = MessageStatus::CallbackReceived;
    save_message(env, &msg);

    remove_message(env, original_message_id);

    Ok(())
}

/// Mark a pending/relayed message as Failed so it can be retried.
pub fn mark_message_failed(env: &Env, message_id: u64) -> Result<(), String> {
    let mut msg = get_message(env, message_id)?;

    if msg.status == MessageStatus::Delivered || msg.status == MessageStatus::CallbackReceived {
        return Err(String::from_str(env, "Message already completed"));
    }

    msg.status = MessageStatus::Failed;
    save_message(env, &msg);

    env.events().publish(
        (Symbol::new(env, "msg_failed"), message_id),
        env.ledger().timestamp(),
    );

    Ok(())
}

/// Reset a Failed message back to Pending so validators can relay it again.
pub fn retry_failed_message(env: &Env, message_id: u64) -> Result<(), String> {
    let mut msg = get_message(env, message_id)?;

    if msg.status != MessageStatus::Failed {
        return Err(String::from_str(env, "Message not failed"));
    }

    msg.status = MessageStatus::Pending;
    save_message(env, &msg);

    env.events().publish(
        (Symbol::new(env, "msg_retry"), message_id),
        env.ledger().timestamp(),
    );

    Ok(())
}

/// Expire a message that has been pending/relayed past `MESSAGE_TIMEOUT`.
pub fn expire_timed_out_message(env: &Env, message_id: u64) -> Result<(), String> {
    let mut msg = get_message(env, message_id)?;

    let elapsed = env.ledger().timestamp().saturating_sub(msg.sent_at);
    if elapsed < MESSAGE_TIMEOUT {
        return Err(String::from_str(env, "Message not timed out yet"));
    }

    if msg.status == MessageStatus::Delivered || msg.status == MessageStatus::CallbackReceived {
        return Err(String::from_str(env, "Message already completed"));
    }

    msg.status = MessageStatus::Failed;
    save_message(env, &msg);

    env.events().publish(
        (Symbol::new(env, "msg_expired"), message_id),
        env.ledger().timestamp(),
    );

    Ok(())
}

/// Read a stored message (for off-chain queries / tests).
pub fn get_cross_chain_message(env: &Env, message_id: u64) -> Option<CrossChainMessage> {
    env.storage()
        .persistent()
        .get(&MessagingKey::Message(message_id))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::governance::{initialize_bridge, BridgeSecurityConfig};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup() -> (Env, Address, soroban_sdk::Vec<Address>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1000);

        let mut validators = soroban_sdk::Vec::new(&env);
        validators.push_back(Address::generate(&env));
        validators.push_back(Address::generate(&env));

        let security_config = BridgeSecurityConfig {
            max_transfer_amount: 1_000_000_000,
            daily_transfer_limit: 10_000_000_000,
            min_validator_signatures: 1,
            transfer_delay_seconds: 0,
        };
        initialize_bridge(&env, 1u64, validators.clone(), 1, security_config).unwrap();
        register_bridge_for_chain(&env, ChainId::Ethereum, 1u64);

        let sender = Address::generate(&env);
        (env, sender, validators)
    }

    fn payload(env: &Env) -> Bytes {
        Bytes::from_slice(env, &[0x01, 0x02])
    }

    fn proof(env: &Env) -> Bytes {
        Bytes::from_slice(env, &[0xFF])
    }

    fn empty(env: &Env) -> Bytes {
        Bytes::new(env)
    }

    fn target(env: &Env) -> String {
        String::from_str(env, "0xContract")
    }

    // ── send ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_send_message_ok() {
        let (env, sender, _) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        assert_eq!(id, 1);
        let msg = get_cross_chain_message(&env, id).unwrap();
        assert_eq!(msg.status, MessageStatus::Pending);
        assert_eq!(msg.source_chain, ChainId::Stellar);
        assert_eq!(msg.target_chain, ChainId::Ethereum);
    }

    #[test]
    fn test_send_increments_id() {
        let (env, sender, _) = setup();
        let id1 = send_cross_chain_message(
            &env,
            sender.clone(),
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();
        let id2 = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn test_send_payload_too_large() {
        let (env, sender, _) = setup();
        let big = Bytes::from_slice(&env, &[0u8; 4097]);
        let result = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            big,
            100_000,
            false,
        );
        assert!(result.is_err());
    }

    // ── relay ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_relay_ok() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Relayed
        );
    }

    #[test]
    fn test_relay_already_relayed_fails() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        let result =
            relay_message_to_target_chain(&env, id, validators.get(1).unwrap(), proof(&env));
        assert!(result.is_err());
    }

    #[test]
    fn test_relay_unauthorized_validator_fails() {
        let (env, sender, _) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        let rogue = Address::generate(&env);
        assert!(relay_message_to_target_chain(&env, id, rogue, proof(&env)).is_err());
    }

    // ── delivery ──────────────────────────────────────────────────────────────

    #[test]
    fn test_confirm_delivery_no_callback_cleans_up() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        confirm_message_delivery(&env, id, proof(&env)).unwrap();
        assert!(get_cross_chain_message(&env, id).is_none());
    }

    #[test]
    fn test_confirm_delivery_with_callback_keeps_message() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            true,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        confirm_message_delivery(&env, id, proof(&env)).unwrap();

        let msg = get_cross_chain_message(&env, id).unwrap();
        assert_eq!(msg.status, MessageStatus::Delivered);
        assert!(msg.delivered_at.is_some());
    }

    #[test]
    fn test_confirm_delivery_empty_proof_fails() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        assert!(confirm_message_delivery(&env, id, empty(&env)).is_err());
    }

    #[test]
    fn test_confirm_delivery_without_relay_fails() {
        let (env, sender, _) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();
        assert!(confirm_message_delivery(&env, id, proof(&env)).is_err());
    }

    // ── callback ──────────────────────────────────────────────────────────────

    #[test]
    fn test_receive_callback_ok() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            true,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        confirm_message_delivery(&env, id, proof(&env)).unwrap();
        receive_message_callback(&env, id, payload(&env), proof(&env)).unwrap();
        assert!(get_cross_chain_message(&env, id).is_none());
    }

    #[test]
    fn test_callback_not_expected_fails() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();

        // Manually force Delivered state without callback_required
        let mut msg = get_cross_chain_message(&env, id).unwrap();
        msg.status = MessageStatus::Delivered;
        env.storage()
            .persistent()
            .set(&MessagingKey::Message(id), &msg);

        assert!(receive_message_callback(&env, id, payload(&env), proof(&env)).is_err());
    }

    #[test]
    fn test_callback_before_delivery_fails() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            true,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        // Skip confirm_message_delivery
        assert!(receive_message_callback(&env, id, payload(&env), proof(&env)).is_err());
    }

    #[test]
    fn test_callback_empty_proof_fails() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            true,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        confirm_message_delivery(&env, id, proof(&env)).unwrap();
        assert!(receive_message_callback(&env, id, payload(&env), empty(&env)).is_err());
    }

    // ── retry ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_retry_failed_message_ok() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        mark_message_failed(&env, id).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Failed
        );

        retry_failed_message(&env, id).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Pending
        );
    }

    #[test]
    fn test_retry_non_failed_message_fails() {
        let (env, sender, _) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();
        assert!(retry_failed_message(&env, id).is_err());
    }

    #[test]
    fn test_mark_completed_message_failed_fails() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        confirm_message_delivery(&env, id, proof(&env)).unwrap();
        // Message was cleaned up (no callback), so get returns None — mark_failed should error
        assert!(mark_message_failed(&env, id).is_err());
    }

    // ── timeout ───────────────────────────────────────────────────────────────

    #[test]
    fn test_expire_timed_out_message() {
        let (env, sender, _) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        env.ledger().set_timestamp(1000 + MESSAGE_TIMEOUT + 1);
        expire_timed_out_message(&env, id).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Failed
        );
    }

    #[test]
    fn test_expire_not_timed_out_fails() {
        let (env, sender, _) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();
        assert!(expire_timed_out_message(&env, id).is_err());
    }

    // ── full flows ────────────────────────────────────────────────────────────

    #[test]
    fn test_full_flow_with_callback() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            true,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Relayed
        );

        confirm_message_delivery(&env, id, proof(&env)).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Delivered
        );

        receive_message_callback(&env, id, payload(&env), proof(&env)).unwrap();
        assert!(get_cross_chain_message(&env, id).is_none());
    }

    #[test]
    fn test_full_flow_fail_then_retry_then_deliver() {
        let (env, sender, validators) = setup();
        let id = send_cross_chain_message(
            &env,
            sender,
            ChainId::Ethereum,
            target(&env),
            payload(&env),
            100_000,
            false,
        )
        .unwrap();

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        mark_message_failed(&env, id).unwrap();
        retry_failed_message(&env, id).unwrap();
        assert_eq!(
            get_cross_chain_message(&env, id).unwrap().status,
            MessageStatus::Pending
        );

        relay_message_to_target_chain(&env, id, validators.get(0).unwrap(), proof(&env)).unwrap();
        confirm_message_delivery(&env, id, proof(&env)).unwrap();
        assert!(get_cross_chain_message(&env, id).is_none());
    }

    #[test]
    fn test_multiple_concurrent_messages() {
        let (env, sender, validators) = setup();
        let validator = validators.get(0).unwrap();

        let mut ids = soroban_sdk::Vec::new(&env);
        for _ in 0..5u32 {
            let id = send_cross_chain_message(
                &env,
                sender.clone(),
                ChainId::Ethereum,
                target(&env),
                payload(&env),
                100_000,
                false,
            )
            .unwrap();
            ids.push_back(id);
        }

        for id in ids.iter() {
            relay_message_to_target_chain(&env, id, validator.clone(), proof(&env)).unwrap();
            confirm_message_delivery(&env, id, proof(&env)).unwrap();
            assert!(get_cross_chain_message(&env, id).is_none());
        }
    }
}
