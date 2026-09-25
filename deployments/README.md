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

- `admin`, or any contract's `address`, is not a syntactically valid Stellar
  StrKey (wrong length, bad checksum, or wrong address type).
- Any contract is missing a `package` name or a positive integer `version`.
- A `depends_on` entry names a contract not present in the manifest, or
  requires a `min_version` higher than that contract's declared `version`.
- The `depends_on` graph has a cycle.

See `testnet.manifest.json` for a filled-in example and
`mainnet.manifest.example.json` for a template — copy the latter to
`mainnet.manifest.json` and replace the `REPLACE_WITH_*` placeholders before
a mainnet release. It is named `*.example.json` (not `*.manifest.json`) so
an unfilled copy is never auto-discovered or auto-validated as if it were
ready to deploy.

Run it directly with:

```sh
python3 stellar-swipe/scripts/validate_deployment_manifest.py deployments/testnet.manifest.json
```

## Contract registry (Issue #881)

`deployments/registry.json` is the canonical, versioned answer to "which
contract is deployed at which address, on which network". It complements the
manifests above: a manifest describes what a release *should* contain, the
registry records what is actually live.

Each network entry carries its passphrase, RPC URL, the manifest it was
validated against, a `deployed_at` timestamp, and one `{ address, version }`
entry per contract. `address` is `null` until that contract has been deployed.

Read it through `scripts/deployment_registry.ts` rather than parsing it by hand,
so a missing deployment fails loudly instead of yielding `null` downstream:

```sh
cd scripts
npx tsx deployment_registry.ts list testnet          # all contracts on a network
npx tsx deployment_registry.ts get testnet stake_vault   # one address, exits non-zero if undeployed
```

From TypeScript:

```ts
import { loadRegistry, getContractId } from "./deployment_registry.ts";

const stakeVault = getContractId(loadRegistry(), "testnet", "stake_vault");
```

`recordDeployment(network, contract, address)` writes a freshly deployed address
back into the registry and stamps `deployed_at`, so deploy scripts keep the file
current instead of leaving addresses scattered across per-run state files.

## Contract invariant snapshots (Issue #1086)

`deployments/invariants.json` is the **versioned** set of critical invariants
that deployment smoke tests assert against. It is checked in alongside the
manifests and registry so the invariant set evolves with the deployment
process rather than living only inside test code.

Each entry names the contract, the invariant `kind`, and the expected value:

```json
{
  "version": 1,
  "invariants": [
    { "contract": "governance", "kind": "authorization", "name": "admin", "expected": "G..." },
    { "contract": "fee_collector", "kind": "balance", "name": "fee_token", "expected": "0" },
    { "contract": "oracle", "kind": "configuration", "name": "max_staleness", "expected": "300" },
    { "contract": "stake_vault", "kind": "version", "name": "version", "expected": "1" }
  ]
}
```

Supported `kind` values are `authorization`, `balance`, `configuration`, and
`version`. Bump `version` whenever the invariant set changes so a smoke-test
run can be tied back to the deployment it validated.

The smoke tests under `tests/smoke/` deploy the locally built WASM artifacts,
read each invariant immediately after initialization and again after a
representative mutation, and fail with a message that names both the violated
invariant and the contract address, e.g.:

```
invariant violated: governance.authorization.admin (expected G..., got G...)
  contract: C...
```

Run them against a local deployment with:

```sh
cargo test --test smoke -- --nocapture
```
