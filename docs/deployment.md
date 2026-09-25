# Deployment

This document describes how contracts are deployed and how the deployment
smoke tests verify that a freshly deployed contract is in a healthy state.

## Deployment process

1. Build the WASM artifacts locally (`cargo build --release --target wasm32-unknown-unknown`).
2. Deploy the artifacts to the target network.
3. Run the deployment smoke tests against the locally deployed WASM artifacts.

## Contract invariant snapshots

Deployment smoke tests read a set of **critical invariants** immediately after
initialization and again after representative mutations. Each invariant is
identified by a stable name and is versioned together with the deployment
process, so a snapshot taken by one deployment can be compared against the
snapshot taken by the next.

### Invariant set

The invariant set is versioned with the deployment process. The current
snapshot version is recorded alongside the deployment so that a mismatch
between the expected and observed invariant set is itself a failure.

| Invariant | Category | Checked after init | Checked after mutation |
| --- | --- | --- | --- |
| `authorization.admin` | authorization | yes | yes |
| `authorization.roles` | authorization | yes | yes |
| `balances.total_supply` | balances | yes | yes |
| `balances.holder` | balances | yes | yes |
| `configuration.params` | configuration | yes | yes |
| `version.contract` | version | yes | yes |

### Snapshot format

A snapshot is a map from invariant name to the observed value, plus the
contract address and the snapshot version:

```json
{
  "snapshot_version": 1,
  "contract_address": "C...",
  "invariants": {
    "authorization.admin": "G...",
    "balances.total_supply": "1000000",
    "configuration.params": { "fee_bps": 30 },
    "version.contract": "1.0.0"
  }
}
```

### Failure reporting

When an invariant does not match its expected value, the smoke test fails with
a message that identifies **both the violated invariant and the contract
address**, for example:

```
invariant `balances.total_supply` violated for contract C...:
  expected 1000000, observed 999999
```

This makes it possible to tell, from CI output alone, which contract is
unhealthy and which invariant it broke.

### Running the smoke tests

The smoke tests run against locally deployed WASM artifacts, so they exercise
the exact bytecode that will be deployed:

```
# deploy locally, then run the invariant smoke tests
cargo test --test deployment_smoke
```

## Versioning the invariant set

Because the invariant set is versioned with the deployment process, adding,
removing, or changing an invariant requires bumping `snapshot_version` and
updating this document in the same change. Deployments that observe a
`snapshot_version` different from the one they expect must fail rather than
silently accept a drifted invariant set.
