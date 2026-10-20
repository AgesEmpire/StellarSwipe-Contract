# Cross-Contract Communication Patterns

This document describes the standardized cross-contract communication patterns implemented in `contracts/shared/src/cross_contract.rs`.

## Goals

- Define interface specifications for generic cross-contract routing.
- Provide a reusable message passing protocol.
- Enforce authentication and authorization for callers.
- Include version and call validation checks.
- Support deterministic error propagation and delivery state management.

## Standard interface specification

The shared library exposes:

- `CrossContractMessage` — a routed message payload containing source/target contracts, operation metadata, and status.
- `MessageStatus` — message lifecycle states: `Pending`, `Delivered`, `Failed`, `Rejected`.
- `CrossContractVersionClient` — a version client used by callers to validate compatibility before invoking a target contract.
- `CrossContractMessageReceiverClient` — a receiver trait that contracts can implement to receive generic routed messages.

## Message passing protocol

The pattern is:

1. A caller constructs a `CrossContractMessage` via `send_cross_contract_message`.
2. The shared library validates payload size and cross-contract call depth.
3. The message is persisted in a shared message queue.
4. The message status is emitted as events such as `msg_sent`.
5. Authorized receivers acknowledge delivery via `acknowledge_message_delivery` or reject invalid requests.

## Authentication and authorization

Authentication is enforced through explicit signer checks:

- `send_cross_contract_message` requires the message sender to authorize the request.
- `register_authorized_caller` binds contract call relationships and requires the manager to sign.
- `authorize_caller` checks whether the receiver is allowed to process messages sent to a specific target contract.

## Call validation logic

The shared library validates:

- Payload size against `MAX_MESSAGE_SIZE`.
- Cross-contract call depth using `check_call_depth`.
- Target contract compatiblity using `CrossContractVersionClient` and `check_compatible`.
- Optional contract hash expectations through shared auth checks.

## Error propagation mechanisms

All shared routing operations return `CrossContractError` variants. This enables standard handling of:

- unauthorized signers
- unauthorized callers
- invalid payloads
- missing messages
- incompatible target versions
- call depth exhaustion
- contract hash mismatches

Events also carry the message lifecycle state so off-chain listeners can trace routing and delivery outcomes.

## Integration guidance

Contracts can consume the shared module with:

```rust
use shared::cross_contract::{self, CrossContractError, CrossContractMessage, MessageStatus};
```

Example call flow:

1. `sender.require_auth()`
2. `cross_contract::send_cross_contract_message(...)`
3. On the target contract, check authorized caller and call depth.
4. `cross_contract::acknowledge_message_delivery(...)`

Contracts can also expose a receiver interface with `CrossContractMessageReceiverClient` and implement `receive_message` for reusable semantics.
