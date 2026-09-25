# Contract Development Guide

This guide covers conventions and operational practices for developing and
maintaining the core Soroban contracts in this repository.

## Soroban Storage TTL Bump Strategy

Soroban contract instance and persistent storage entries have a time-to-live
(TTL) measured in ledgers. When an entry's TTL is exhausted it becomes
*expired* and can no longer be read or written until it is restored. To keep
active state available during normal use, contracts must extend ("bump") the
TTL of the entries they rely on before those entries become at-risk.

### Thresholds and extension amounts

The bump strategy is driven by two explicit, configurable values:

- **`TTL_THRESHOLD`** — the remaining-TTL watermark below which an entry is
  considered *at-risk* and should be extended. Entries with more remaining TTL
  than this threshold are left untouched so we do not spend rent
  unnecessarily.
- **`TTL_EXTEND_TO`** — the target remaining TTL applied when an entry is
  extended. This must be strictly greater than `TTL_THRESHOLD` so that a single
  bump moves the entry comfortably above the watermark.

Both values are expressed in ledgers and are configurable per contract so that
instance entries and persistent storage entries can use different budgets.

### When entries are extended

- **Active entries are extended before they become renewable at-risk.** A bump
  is only performed when the entry's remaining TTL is at or below
  `TTL_THRESHOLD`. Entries that are already comfortably above the threshold are
  skipped, avoiding unnecessary rent spend.
- **Already-extended entries are left alone.** If a previous bump already
  raised the remaining TTL above `TTL_THRESHOLD`, no further extension is
  performed until the watermark is reached again.
- **Expired entries cannot be bumped in place.** Once an entry has expired it
  must be restored before it can be extended; the bump strategy only protects
  entries that are still live.

### Operational maintenance job

A scheduled maintenance job is responsible for keeping active state alive:

1. Enumerate the contract instance and the persistent storage entries that are
   expected to remain active.
2. For each entry, read its current remaining TTL.
3. If the remaining TTL is at or below `TTL_THRESHOLD`, extend it to
   `TTL_EXTEND_TO`.
4. Skip entries whose remaining TTL is already above the threshold.

The job should run frequently enough that no active entry can fall from above
`TTL_THRESHOLD` to expired between two runs under normal operation.

### Expected budget

Rent spend is proportional to the TTL actually added. Because the job only
bumps entries at or below `TTL_THRESHOLD` and extends them to `TTL_EXTEND_TO`,
the per-bump cost is bounded by `TTL_EXTEND_TO - TTL_THRESHOLD` ledgers of
additional TTL per entry. Entries that are already extended incur no cost.
Operators should size `TTL_THRESHOLD` and `TTL_EXTEND_TO` so that the worst-case
number of entries bumped per run times the per-bump cost stays within the
contract's rent budget.

### Tests

The bump strategy is covered by tests for the following cases:

- **Near-threshold** — an entry whose remaining TTL is at or below
  `TTL_THRESHOLD` is extended to `TTL_EXTEND_TO`.
- **Already-extended** — an entry whose remaining TTL is above `TTL_THRESHOLD`
  is left unchanged and incurs no rent spend.
- **Expired-entry handling** — an entry that has already expired is not bumped
  in place and must be restored before it can be extended.
