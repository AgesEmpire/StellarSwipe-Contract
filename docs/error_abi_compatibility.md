# Public error ABI compatibility

Clients decode a failed contract call by its numeric error code. The code of every `#[contracterror]` variant is therefore public ABI: renumbering it, removing it, or giving an old code to a new variant silently changes what clients report.

## Fixture

`stellar-swipe/error-abi/public-error-abi.json` pins every public error code:

```json
{
  "abi_version": "1.0.0",
  "crates": { "signal_registry": { "AdminError": { "Unauthorized": 1, "...": 0 } } },
  "migrations": []
}
```

- The fixture is generated from the Rust source by `stellar-swipe/scripts/check_error_abi.py`. Do not edit it by hand.
- Test-only crates (`integration_tests`, `stake_vault_kani`) are excluded.
- If two modules in a crate define an enum with the same name, each key is prefixed with its module (for example `capability::CapabilityError`).

## What CI checks

The **Check public error ABI compatibility** step runs:

```bash
python3 stellar-swipe/scripts/check_error_abi.py --base-ref FETCH_HEAD
```

| Change | Result |
|---|---|
| Variant removed, enum removed, code changed, or an old code reused by another variant | **Fails**, unless recorded as an intentional change (below) |
| Duplicate code within one enum, or a variant without an explicit `= N` | **Fails** |
| New variant or enum | **Fails until the fixture is regenerated**, so new codes are always pinned in review |
| Fixture hand-edited to hide a break (compared with the PR base) | **Fails**, unless the break is declared in `migrations` |

The Rust test `stellar-swipe/contracts/signal_registry/tests/error_abi.rs` checks the same fixture from the compiled crate. It also checks the code a client actually receives from a failed call.

## Adding error codes

1. Add the variant with a new, unused `= N`.
2. Run `python3 stellar-swipe/scripts/check_error_abi.py --update`.
3. For a new `AdminError` variant, also add it to `ADMIN_ERRORS` in `tests/error_abi.rs`.
4. Commit the updated fixture.

## Intentional breaking changes

A break needs a new **major** `abi_version` and a migration note for clients:

```bash
python3 stellar-swipe/scripts/check_error_abi.py --update \
  --version 2.0.0 \
  --note "AdminError::PauseExpired (6) removed; clients should treat 6 as TradingPaused"
```

This appends `{version, note, changes}` to `migrations`, where `changes` lists each break. Without `--version` (with a higher major version) and `--note`, `--update` refuses to write the fixture.

## Related: event schema

Event identifiers and topics follow the same policy through `scripts/validate_event_schema.py` and `docs/event_schema.lock.json`:

- Within one contract, no two events may publish topic tuples that an indexer could confuse. Two events collide when they have the same number of topics and, at every position, either the literals are equal or at least one side is a data field.
- Topic literals must be valid Soroban symbols.
- Removing an event, changing its topics, or removing, renaming, retyping or reordering body fields requires a new major `schema_version` and a `migration_notes` entry for the event.
- Appending body fields and adding events are allowed. After either, run `python3 scripts/validate_event_schema.py --update-lock` to refresh the lock.
