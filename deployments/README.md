# Deployments

This directory documents deployment procedures and required configuration for the
Soroban contracts in this repository.

## Contract Constructor Initialization Checklist

Every contract must be initialized exactly once. Before deploying, confirm that
all required constructor arguments are supplied and that the initialization
guard is in place. The checklist below records the required constructor
arguments per contract.

### General rules

- [ ] Every required storage item (config, roles, parameters, version marker)
      is written during initialization.
- [ ] Initialization is guarded so a second call is rejected and does **not**
      mutate state (already-initialized error).
- [ ] Partial initialization cannot leave the contract in a half-configured
      state; either all fields are set or the call fails.
- [ ] Interrupted initialization (e.g. a failed call) can be safely retried.
- [ ] The version marker is written exactly once and matches the deployed
      contract version.

### Required constructor arguments

| Contract | Required arguments | Notes |
| --- | --- | --- |
| `signal_registry` | admin, version | Roles and version marker set on init. |

> Update this table whenever a contract's constructor signature changes. A
> deployment is only valid when every required argument above is provided and
> the initialization guard rejects repeated calls.
