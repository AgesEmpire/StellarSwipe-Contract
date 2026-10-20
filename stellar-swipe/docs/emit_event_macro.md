# `emit_event!` macro — usage guide

The `emit_event!` macro, defined in `stellar_swipe_common::emit`, provides a
single, consistent way to publish Soroban contract events across all crates in
this workspace.

## Motivation

Every contract previously hand-wrote a 2-step topic-construction + publish
pattern at each call site:

```rust
// Before (repeated at every event site)
let topics = (Symbol::new(env, "admin_transfer_completed"),);
env.events().publish(topics, (old_admin, new_admin));
```

Inconsistencies crept in (trailing commas in topic tuples, stray `let topics`
bindings, mixed `symbol_short!` / `Symbol::new` usage).  The macro enforces a
uniform 1-element topic tuple for all standard events.

## Syntax

```rust
stellar_swipe_common::emit_event!(env, "event_name", data_payload);
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `env` | `&Env` or `Env` expression | The Soroban environment reference |
| `"event_name"` | string literal | Event name; becomes a `Symbol` topic |
| `data_payload` | any `contracttype`-serialisable value | Event body (tuple, struct, scalar, …) |

## Expansion

```rust
env.events().publish(
    (soroban_sdk::Symbol::new(env, "event_name"),),
    data_payload,
)
```

## Examples

```rust
// Single-field payload
stellar_swipe_common::emit_event!(env, "guardian_set", guardian_address);

// Tuple payload
stellar_swipe_common::emit_event!(
    env,
    "admin_transfer_completed",
    (old_admin, new_admin)
);

// Struct payload (must derive #[contracttype])
stellar_swipe_common::emit_event!(env, "fee_collected", fee_event_struct);
```

## When to use

Use `emit_event!` for every new event that uses a single-Symbol topic.  For
legacy events that already use `symbol_short!` or multi-Symbol topics (e.g.
`(symbol_short!("gov"), symbol_short!("propnew"))`), leave them as-is until a
planned migration is scheduled.

## Reference implementations

- `signal_registry::events::emit_admin_transfer_completed`
- `signal_registry::events::emit_admin_transferred`
- `oracle::events::emit_oracle_removed`
- `oracle::events::emit_weight_adjusted`
