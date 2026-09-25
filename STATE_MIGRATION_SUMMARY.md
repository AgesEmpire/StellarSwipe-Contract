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
