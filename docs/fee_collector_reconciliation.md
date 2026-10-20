# Fee Collector Balance Reconciliation Audit

## Overview

The fee collector contract stores a per-token treasury balance in persistent state (`StorageKey::TreasuryBalance`). Under normal operation this accounting stays in sync with the contract's actual on-chain token holdings. The `audit_balances` method lets operators verify that invariant at any time.

## Method signature

```rust
pub fn audit_balances(
    env: Env,
    tokens: Vec<Address>,
) -> Result<Vec<BalanceMismatch>, ContractError>
```

### Parameters

| Name | Type | Description |
|------|------|-------------|
| `tokens` | `Vec<Address>` | List of SEP-41 token addresses to audit |

### Return value

A `Vec<BalanceMismatch>` — one entry per token whose stored balance differs from the on-chain balance. Returns an empty vector when all balances reconcile.

```rust
pub struct BalanceMismatch {
    pub token: Address, // The token that has a discrepancy
    pub expected: i128, // Balance recorded in contract storage
    pub actual: i128,   // Actual on-chain balance held by this contract
    pub delta: i128,    // actual - expected (positive = surplus, negative = deficit)
}
```

### Errors

- `ContractError::NotInitialized` — contract has not been initialized yet.

## When to run reconciliation checks

| Trigger | Recommended action |
|---------|-------------------|
| After any treasury withdrawal | Confirm the stored balance decreased by the exact withdrawal amount |
| After a large fee collection period | Verify accumulated fees match expectations |
| Incident response / suspected exploit | Identify which tokens and amounts are affected |
| Routine monitoring | Run as part of a weekly cron job via simulation |
| Before a contract upgrade | Capture baseline balances to compare post-upgrade |

## How to use

### Off-chain simulation (no fees, no auth)

Use `simulateTransaction` so the read-only call costs nothing and requires no auth:

```typescript
import { Contract, TransactionBuilder, Networks } from "@stellar/stellar-sdk";

const contract = new Contract(FEE_COLLECTOR_ADDRESS);
const tokens = [TOKEN_A_ADDRESS, TOKEN_B_ADDRESS];

const tx = new TransactionBuilder(sourceAccount, { fee: "100", networkPassphrase: Networks.TESTNET })
  .addOperation(contract.call("audit_balances", xdr.ScVal.scvVec(tokens.map(toAddress))))
  .setTimeout(30)
  .build();

const result = await server.simulateTransaction(tx);
// Decode result: Vec<BalanceMismatch>
```

### Monitoring script pattern

```typescript
const mismatches = await auditBalances(tokens);

if (mismatches.length > 0) {
  for (const m of mismatches) {
    const direction = m.delta > 0 ? "SURPLUS" : "DEFICIT";
    console.error(`[ALERT] ${direction} on ${m.token}: expected=${m.expected}, actual=${m.actual}, delta=${m.delta}`);
    // Send alert to PagerDuty / Slack / etc.
  }
}
```

## Interpreting results

| Scenario | `delta` sign | Likely cause |
|----------|-------------|--------------|
| Balanced | — (no entry) | Contract accounting is correct |
| Surplus (`delta > 0`) | Positive | Tokens were sent directly to the contract address bypassing `collect_fee`, or a burn was undercounted |
| Deficit (`delta < 0`) | Negative | Tokens left the contract without updating stored balance — indicates a potential accounting bug or exploit |

A **surplus** is generally harmless but should be investigated. A **deficit** is a critical anomaly that may indicate loss of funds and should trigger immediate incident response.

## Notes

- `audit_balances` is read-only; it does not modify any state.
- It can be called without admin auth and is safe to call in a `simulateTransaction` RPC call.
- Pass only the tokens you expect the contract to hold to keep simulation costs low.
- Revenue share pool balances (`StorageKey::RevenueSharePool`) are **not** included in `expected` — only `TreasuryBalance` is audited. If you need to audit revenue share pools, compare them separately.
