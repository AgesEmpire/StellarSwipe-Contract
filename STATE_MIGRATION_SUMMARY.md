# State Migration Summary

This document summarizes the contract state migration surface and defines the
archival export format used to snapshot selected contract state before upgrades.

## Contract State Archival Export Format

The archival export captures selected contract state so it can be archived before
upgrades, inspected offline, and used to verify migration completeness.

### Record shape

Each export record is a single JSON object with the following fields:

| Field              | Type   | Description                                                        |
| ------------------ | ------ | ------------------------------------------------------------------ |
| `contract_id`      | string | Contract identity (address or canonical contract identifier).      |
| `schema_version`   | uint32 | Version of the state schema the record was serialized against.     |
| `ledger_sequence`  | uint32 | Ledger sequence at which the state was read.                       |
| `ledger_close_time`| uint64 | Close time of the ledger context, in seconds since epoch.          |
| `network_passphrase`| string | Network passphrase identifying the ledger context.                |
| `key`              | string | Deterministic, encoded storage key.                                |
| `value`            | string | Encoded value, or a redacted marker for sensitive entries.         |

### Determinism

- Records are ordered by `(contract_id, key)` using byte-wise ascending order.
- Keys and values use a canonical encoding (stable field order, no insignificant
  whitespace) so the same state always produces the same bytes.
- The export is written as newline-delimited JSON (NDJSON), one record per line,
  with a trailing newline.

### Sensitive values

Values classified as sensitive are not written in plaintext. They are replaced
with a redaction marker and a stable digest so that presence and identity can
still be verified without exposing the underlying value, in line with the
repository security requirements.

### Verification

Import/verification compares the exported record set against the expected set:

- Missing records (present in expected, absent in export) are reported.
- Extra records (present in export, absent in expected) are reported.
- A mismatch in `schema_version` or ledger context is reported.

This allows migration completeness to be checked deterministically before and
after an upgrade.

## Migration Dry-Run Command

The migration dry-run command reports the planned effect of a contract storage
migration without committing any state. It is read-only: it never writes to
ledger state, and it can be run safely against production snapshots.

### Input: captured ledger snapshot

The dry-run operates against a captured ledger snapshot rather than live state.
The snapshot is the archival export described above (NDJSON, one record per
line), so a dry-run can be reproduced offline from any captured snapshot.

### Output: machine-readable report

The command emits a single deterministic JSON object. The same snapshot and the
same migration always produce byte-identical output.

| Field                | Type   | Description                                                       |
| -------------------- | ------ | ----------------------------------------------------------------- |
| `dry_run`            | bool   | Always `true`; asserts that no state was committed.               |
| `schema_version`     | uint32 | Schema version the snapshot was read against.                     |
| `ledger_sequence`    | uint32 | Ledger sequence of the captured snapshot.                         |
| `network_passphrase` | string | Network passphrase of the captured snapshot.                      |
| `planned_changes`    | array  | Planned key changes, ordered by `(contract_id, key)`.             |
| `counts`             | object | Aggregate counts (see below).                                     |
| `invariant_failures` | array  | Invariant violations detected while planning.                     |

Each entry in `planned_changes` is a JSON object:

| Field         | Type   | Description                                                          |
| ------------- | ------ | -------------------------------------------------------------------- |
| `contract_id` | string | Contract identity the key belongs to.                                |
| `key`         | string | Deterministic, encoded storage key.                                  |
| `operation`   | string | One of `create`, `update`, or `delete`.                              |
| `old_value`   | string | Encoded prior value, or `null` for `create`.                         |
| `new_value`   | string | Encoded planned value, or `null` for `delete`.                       |

The `counts` object reports aggregate totals:

| Field      | Type   | Description                                    |
| ---------- | ------ | ---------------------------------------------- |
| `create`   | uint32 | Number of planned `create` operations.         |
| `update`   | uint32 | Number of planned `update` operations.         |
| `delete`   | uint32 | Number of planned `delete` operations.         |
| `unchanged`| uint32 | Number of keys left unchanged by the migration.|
| `total`    | uint32 | Total keys considered.                         |

Each entry in `invariant_failures` is a JSON object:

| Field         | Type   | Description                                                       |
| ------------- | ------ | ----------------------------------------------------------------- |
| `contract_id` | string | Contract identity the failure applies to.                         |
| `key`         | string | Storage key the failure applies to, or `null` if contract-wide.   |
| `invariant`   | string | Stable identifier of the violated invariant.                      |
| `message`     | string | Human-readable description of the failure.                        |

### Determinism

- `planned_changes` is ordered by `(contract_id, key)` using byte-wise
  ascending order.
- `invariant_failures` is ordered by `(contract_id, key, invariant)` using
  byte-wise ascending order.
- All fields use a canonical encoding (stable field order, no insignificant
  whitespace) so the same snapshot and migration always produce the same bytes.
- The report is emitted as a single JSON object with a trailing newline.

### No writes in dry-run mode

Dry-run mode performs no writes to ledger state. It reads only from the captured
snapshot, computes the planned changes and invariant checks in memory, and emits
the report. The `dry_run` field is always `true` so consumers can assert that a
report came from a read-only run.

### Fixtures

Representative migrations have expected-output fixtures: for each fixture
snapshot, the expected dry-run report is stored alongside it and compared
byte-for-byte. This keeps the dry-run output stable and machine-readable across
changes.
