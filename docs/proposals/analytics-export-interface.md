# Standardized Analytics Export/Query Interface

## Problem
Contract-derived analytics data exists but there's no standardized way for
off-chain dashboards and reporting systems to query it reliably.

## Proposed Solution
- Define a stable, versioned read-only query surface on the `analytics`
  contract: `get_metrics_snapshot(scope, from_ledger, to_ledger)` returning a
  typed `AnalyticsSnapshot` struct rather than ad-hoc individual getters.
- Version the response shape (`schema_version` field) so downstream consumers
  can detect breaking changes without guessing from field presence.
- Emit a periodic `AnalyticsSnapshotPublished { schema_version, ledger, hash }`
  event so off-chain indexers know when a new queryable snapshot is available
  without polling every ledger.
- Document the export schema in `docs/` (e.g. `docs/analytics-schema.md`) so
  dashboard/reporting teams can integrate against a stable contract rather
  than reverse-engineering storage layout.

## Benefits
- Off-chain systems get a documented, versioned contract instead of relying on
  raw storage reads that break silently on upgrades.
- Event-driven notification avoids wasteful polling for new data.

## Next Steps
- Define `AnalyticsSnapshot` and `schema_version` in `contracts/analytics`.
- Add the snapshot query entrypoint and publish event.
- Write `docs/analytics-schema.md` describing the query interface and schema
  versioning policy for external integrators.
