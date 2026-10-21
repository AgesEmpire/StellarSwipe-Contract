# Event payload serialization fixtures

Committed XDR hex fixtures for representative public events defined in
`contracts/shared/src/events.rs`. Generated and checked by
`contracts/shared/src/event_fixtures_tests.rs` (issue #1339).

Each `<name>.hex` file pins the deterministic Soroban host XDR encoding of a
representative instance of an event payload struct. A test failure means the
struct's field order, field types, or field names changed in a way that would
silently break indexers consuming the deployed contract's events.

## Updating a fixture

Only update a fixture when the schema change is intentional:

1. Bump `schema_version` on the affected event struct and add a dated note to
   `docs/events.md` describing the change.
2. Run `UPDATE_EVENT_FIXTURES=1 cargo test -p shared event_fixtures_tests`
   to regenerate the `.hex` file.
3. Commit the updated `.hex` file alongside the schema-version bump.
