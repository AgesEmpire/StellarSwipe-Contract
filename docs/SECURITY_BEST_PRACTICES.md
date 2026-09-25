# Security Best Practices

This document records the security assumptions and required practices for the
Soroban contracts in this repository. It is a living document: any change to an
authorization-sensitive entrypoint must be reflected here.

## Authorization

### Bind authorization to the caller, not to metadata

Sensitive entrypoints MUST call `require_auth` on the authenticated address
that is expected to authorize the operation. Authorization decisions MUST NOT
rely on caller-controlled metadata (arguments, event payloads, or any other
value supplied by the invoker) as the sole basis for granting access.

```rust
// Good: the authenticated address is the one we expect.
caller.require_auth();

// Bad: trusting an address passed in as an argument.
let claimed = args.caller;
claimed.require_auth(); // only valid if `claimed` is independently bound
```

When an entrypoint accepts an address argument, the contract MUST verify that
the argument matches the address that actually authorized the invocation, or
that the argument is otherwise bound to the authenticated context.

### Authorization tree drift

Soroban authorization is evaluated over a tree of nested invocations. When a
contract calls another contract, the authorization tree can change shape
because of intermediary contracts. Contracts MUST NOT assume that the
immediate caller is the ultimate authorizer.

- Verify the intended caller and the intended operation at each sensitive
  boundary.
- Do not cache or reuse an authorization decision across invocations; the tree
  for a nested call may differ from the tree for a direct call.
- Treat intermediary contracts as untrusted: an intermediary may forward a
  call, but it cannot grant authorization that the root authorizer did not
  provide.

### Nested and intermediary calls

For any entrypoint that can be reached through a nested invocation:

1. Re-derive the expected authorizer from contract state, not from the call
   arguments.
2. Call `require_auth` on that expected authorizer.
3. Validate the operation parameters against the expected operation before
   mutating state.

This keeps the check valid whether the entrypoint is called directly, through
another contract in this repository, or through an unauthorized intermediary.

## Testing requirements

Authorization changes MUST be covered by tests for at least:

- **Direct calls** — the expected authorizer invokes the entrypoint directly.
- **Nested calls** — the entrypoint is reached through another contract, and
  the authorization tree reflects the nested invocation.
- **Unauthorized intermediaries** — an intermediary contract attempts to
  invoke the entrypoint without the required authorization; the call MUST fail.

Tests MUST assert failure for unauthorized callers and success only for the
bound authorizer.

## Storage and TTL

Storage entries that back authorization-sensitive state MUST be kept live for
the lifetime of the authorization assumptions they support. TTL bumping is a
correctness concern, not only a cost concern: an expired entry can change the
observable state that an authorization check depends on.

## Documentation

Security assumptions for each contract MUST be recorded alongside the
contract. When an authorization check changes, update this document and the
relevant contract documentation in the same change.
