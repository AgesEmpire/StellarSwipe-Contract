use crate::allowlist::{require_allowed_contract, AllowlistError};
use crate::auth::{check_call_depth, verify_wasm_hash};
use crate::version::check_compatible;
use soroban_sdk::{
    contractclient, contracterror, contracttype, Address, Bytes, Env, String, Symbol,
};

pub const MAX_MESSAGE_SIZE: u32 = 2048;
pub const MAX_AUTHORIZED_CALLERS: u32 = 32;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CrossContractError {
    UnauthorizedSigner = 1,
    UnauthorizedCaller = 2,
    InvalidPayload = 3,
    InvalidMessage = 4,
    MessageNotFound = 5,
    VersionMismatch = 6,
    CallDepthExceeded = 7,
    ContractHashMismatch = 8,
    AlreadyDelivered = 9,
    CallerNotRegistered = 10,
    /// The caller contract is not in the cross-contract allowlist.
    CallerNotAllowed = 11,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageStatus {
    Pending,
    Delivered,
    Failed,
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CrossContractMessage {
    pub id: u64,
    pub source_contract: Address,
    pub target_contract: Address,
    pub operation: String,
    pub payload: Bytes,
    pub callback_required: bool,
    pub status: MessageStatus,
    pub created_at: u64,
    pub last_updated_at: u64,
}

#[contracttype]
pub enum MessagingKey {
    Message(u64),
    NextMessageId,
    AuthorizedCaller(Address, Address),
    ExpectedWasmHash(Address),
}

#[contractclient(name = "CrossContractVersionClient")]
pub trait CrossContractVersionTrait {
    fn get_contract_version(env: Env) -> u32;
}

#[contractclient(name = "CrossContractMessageReceiverClient")]
pub trait CrossContractMessageReceiverTrait {
    fn receive_message(
        env: Env,
        message: CrossContractMessage,
    ) -> Result<MessageStatus, CrossContractError>;
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

fn save_message(env: &Env, msg: &CrossContractMessage) {
    env.storage()
        .persistent()
        .set(&MessagingKey::Message(msg.id), msg);
}

fn get_message(env: &Env, id: u64) -> Result<CrossContractMessage, CrossContractError> {
    env.storage()
        .persistent()
        .get(&MessagingKey::Message(id))
        .ok_or(CrossContractError::MessageNotFound)
}

fn publish_message_event(env: &Env, event: Symbol, message: &CrossContractMessage) {
    env.events().publish(
        (event, message.id),
        (
            message.source_contract.clone(),
            message.target_contract.clone(),
            message.operation.clone(),
            message.status.clone(),
        ),
    );
}

pub fn register_authorized_caller(
    env: &Env,
    manager: &Address,
    target_contract: &Address,
    caller: &Address,
) -> Result<(), CrossContractError> {
    manager.require_auth();
    let key = MessagingKey::AuthorizedCaller(target_contract.clone(), caller.clone());
    env.storage().persistent().set(&key, &true);
    Ok(())
}

pub fn authorize_caller(
    env: &Env,
    target_contract: &Address,
    caller: &Address,
) -> Result<(), CrossContractError> {
    let key = MessagingKey::AuthorizedCaller(target_contract.clone(), caller.clone());
    if env.storage().persistent().has(&key) {
        Ok(())
    } else {
        Err(CrossContractError::CallerNotRegistered)
    }
}

pub fn verify_expected_contract_hash(
    env: &Env,
    contract_id: &Address,
) -> Result<(), CrossContractError> {
    verify_wasm_hash(env, contract_id).map_err(|_| CrossContractError::ContractHashMismatch)
}

/// Enforce the cross-contract allowlist for a sensitive entrypoint.
///
/// `entrypoint` identifies which logical entrypoint is being protected — it
/// is typically `env.current_contract_address()` or a stable identifier for
/// the specific guarded function. `caller` is the contract invoking this
/// entrypoint.
///
/// Returns `Ok(())` when the caller is on the allowlist.  Returns
/// `Err(CrossContractError::CallerNotAllowed)` for any other contract,
/// ensuring unauthorized contracts are rejected before any state is mutated.
///
/// # Usage
/// ```ignore
/// require_sensitive_caller(&env, &env.current_contract_address(), &caller)?;
/// // … proceed with privileged logic …
/// ```
pub fn require_sensitive_caller(
    env: &Env,
    entrypoint: &Address,
    caller: &Address,
) -> Result<(), CrossContractError> {
    require_allowed_contract(env, entrypoint, caller).map_err(|e| match e {
        AllowlistError::ContractNotAllowed => CrossContractError::CallerNotAllowed,
        _ => CrossContractError::UnauthorizedCaller,
    })
}
pub fn validate_callee_version(
    env: &Env,
    contract_id: &Address,
    kind: crate::version::ContractKind,
) -> Result<(), CrossContractError> {
    let version = CrossContractVersionClient::new(env, contract_id).get_contract_version();
    check_compatible(version, kind).map_err(|_| CrossContractError::VersionMismatch)
}

pub fn validate_payload(_env: &Env, payload: &Bytes) -> Result<(), CrossContractError> {
    if payload.len() > MAX_MESSAGE_SIZE {
        Err(CrossContractError::InvalidPayload)
    } else {
        Ok(())
    }
}

pub fn send_cross_contract_message(
    env: &Env,
    sender: &Address,
    target_contract: &Address,
    operation: String,
    payload: Bytes,
    callback_required: bool,
    call_depth: u32,
) -> Result<u64, CrossContractError> {
    sender.require_auth();
    validate_payload(env, &payload)?;
    check_call_depth(call_depth).map_err(|_| CrossContractError::CallDepthExceeded)?;

    let id = next_message_id(env);
    let now = env.ledger().timestamp();
    let message = CrossContractMessage {
        id,
        source_contract: sender.clone(),
        target_contract: target_contract.clone(),
        operation,
        payload,
        callback_required,
        status: MessageStatus::Pending,
        created_at: now,
        last_updated_at: now,
    };

    save_message(env, &message);
    publish_message_event(env, Symbol::new(env, "msg_sent"), &message);
    Ok(id)
}

pub fn acknowledge_message_delivery(
    env: &Env,
    message_id: u64,
    receiver: &Address,
) -> Result<MessageStatus, CrossContractError> {
    receiver.require_auth();
    let mut message = get_message(env, message_id)?;
    if message.status != MessageStatus::Pending {
        return Err(CrossContractError::AlreadyDelivered);
    }
    authorize_caller(env, &message.target_contract, receiver)?;
    message.status = MessageStatus::Delivered;
    message.last_updated_at = env.ledger().timestamp();
    save_message(env, &message);
    publish_message_event(env, Symbol::new(env, "msg_delivered"), &message);
    Ok(message.status)
}

pub fn reject_message(
    env: &Env,
    message_id: u64,
    receiver: &Address,
) -> Result<MessageStatus, CrossContractError> {
    receiver.require_auth();
    let mut message = get_message(env, message_id)?;
    if message.status != MessageStatus::Pending {
        return Err(CrossContractError::AlreadyDelivered);
    }
    authorize_caller(env, &message.target_contract, receiver)?;
    message.status = MessageStatus::Rejected;
    message.last_updated_at = env.ledger().timestamp();
    save_message(env, &message);
    publish_message_event(env, Symbol::new(env, "msg_rejected"), &message);
    Ok(message.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MAX_CALL_DEPTH;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Env};

    #[contract]
    struct VersionedContract;

    #[contractimpl]
    impl VersionedContract {
        pub fn get_contract_version(_env: Env) -> u32 {
            1u32
        }
    }

    #[contract]
    struct TestContract;

    #[contractimpl]
    impl TestContract {
        pub fn register_caller(env: Env, target: Address, caller: Address) {
            register_authorized_caller(&env, &caller, &target, &caller).unwrap();
        }

        pub fn test_send_message(
            env: Env,
            sender: Address,
            target_contract: Address,
            operation: String,
            payload: Bytes,
            callback_required: bool,
            call_depth: u32,
        ) -> Result<u64, CrossContractError> {
            send_cross_contract_message(
                &env,
                &sender,
                &target_contract,
                operation,
                payload,
                callback_required,
                call_depth,
            )
        }
    }

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let source_id = env.register(TestContract, ());
        let target = Address::generate(&env);
        let user = Address::generate(&env);
        (env, source_id, target, user)
    }

    #[test]
    fn send_message_sets_pending_status() {
        let (env, source_id, target, user) = setup();
        let source_client = TestContractClient::new(&env, &source_id);
        let payload = Bytes::from_array(&env, &[1, 2, 3, 4]);
        let id = source_client.test_send_message(
            &user,
            &target,
            &String::from_str(&env, "transfer"),
            &payload,
            &false,
            &0,
        );
        env.as_contract(&source_id, || {
            let message = get_message(&env, id).unwrap();
            assert_eq!(message.status, MessageStatus::Pending);
            assert_eq!(message.payload, payload);
        });
    }

    #[test]
    fn send_message_rejects_oversize_payload() {
        let (env, source_id, target, user) = setup();
        let source_client = TestContractClient::new(&env, &source_id);
        let payload = Bytes::from_array(&env, &[0u8; (MAX_MESSAGE_SIZE + 1) as usize]);
        let result = source_client.try_test_send_message(
            &user,
            &target,
            &String::from_str(&env, "call"),
            &payload,
            &false,
            &0,
        );
        assert_eq!(result, Err(Ok(CrossContractError::InvalidPayload)));
    }

    #[test]
    fn authorized_caller_registration_and_validation() {
        let (env, source_id, target, user) = setup();
        let source_client = TestContractClient::new(&env, &source_id);
        source_client.register_caller(&target, &user);
        env.as_contract(&source_id, || {
            assert_eq!(authorize_caller(&env, &target, &user), Ok(()));
        });
    }

    #[test]
    fn validate_callee_version_fails_when_incompatible() {
        let env = Env::default();
        let v_contract = env.register(VersionedContract, ());
        let result = validate_callee_version(
            &env,
            &v_contract,
            crate::version::ContractKind::SignalRegistry,
        );
        assert_eq!(result, Err(CrossContractError::VersionMismatch));
    }

    #[test]
    fn call_depth_limit_is_enforced() {
        assert_eq!(
            check_call_depth(MAX_CALL_DEPTH - 1).map_err(|_| CrossContractError::CallDepthExceeded),
            Ok(MAX_CALL_DEPTH)
        );
        assert_eq!(
            check_call_depth(MAX_CALL_DEPTH).map_err(|_| CrossContractError::CallDepthExceeded),
            Err(CrossContractError::CallDepthExceeded)
        );
    }
}
