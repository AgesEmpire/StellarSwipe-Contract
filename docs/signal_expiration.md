# Signal expiration and archive behavior (signal_registry)

This page specifies how `signal_registry` represents expired signals, which queries can see them, and how they are cleaned up and removed. The implementation is in `stellar-swipe/contracts/signal_registry/src/expiry.rs`. The tests are in `stellar-swipe/contracts/signal_registry/tests/expiry_archive.rs`.

## Expiry boundary

Each signal has an `expiry` ledger timestamp (seconds).

- A signal is **live** while `now <= expiry`, including the exact expiry second.
- It is **expired** once `now > expiry`.

Every expiry decision uses this one rule: queries, cleanup, archival, pruning and `health_check`.

## Lifecycle states

`get_signal_expiry_state(signal_id)` is read-only and reports where a signal id is in its lifecycle. The state is derived when you call it and is never stored.

| State | Condition | In active queries? |
|---|---|---|
| `Live` | Not expired, and status is not `Expired` or `Executed` | Yes |
| `ExpiredPendingCleanup` | Past expiry, but cleanup has not yet set status to `Expired` | No |
| `Expired` | Status is `Expired` | No |
| `Closed` | Status is `Executed` | No |
| `Removed` | The id was allocated, but its record was archived or pruned | No (`get_signal` returns `None`) |
| `Unknown` | The id was never allocated (`0` or greater than the signal counter) | No |

## Query visibility

`get_active_signals`, `get_active_signals_personalized` and `get_active_signals_archived` all filter with the same predicate, `is_visible_active`. A signal is visible exactly when its state is `Live`.

The predicate checks the ledger clock, not just the stored status. An expired signal is therefore hidden **as soon as it expires**, whether or not cleanup has run. Cleanup and archival only ever move a signal away from `Active`, or delete it, so no cleanup run (partial, failed or repeated) can make an expired signal visible again.

## Cleanup: `cleanup_expired_signals(limit)`

Cleanup sets the stored status of expired `Active` signals to `Expired`, releases the provider's active-signal slot, and emits one `signal_expired` event for each signal it changes. Anyone can call it.

- **Bounded:** each call examines at most `limit` entries. `limit` is capped at 100, and `0` means 100. Every entry examined counts toward the limit, whatever its status.
- **Resumable:** the id of the last entry examined is stored in `ExpiryCleanupCursor`. The next call starts after it and wraps around to the lowest id. Repeated calls reach every signal, even when long-lived signals hold the lowest ids.
- **Idempotent:** a signal that is already `Expired` is skipped. Retrying after a failed transaction, or running a sweep twice, changes nothing and emits no duplicate events.
- **Returns** `(examined, newly_expired)`.

`get_signal` also expires an `Active` signal lazily when it is read after its expiry.

## Archival: `archive_old_signals(limit)`

Archival removes a signal from the signals map once **both** of these hold:

1. It is expired: its status is `Expired`, or its status is `Active` and it is past expiry.
2. More than 30 days have passed since its expiry (`now - expiry > 2_592_000`).

Other terminal statuses (`Executed`, `Successful`, `Failed`, `Cancelled`, `ProviderDeleted`) are kept as history.

Archival uses the same limit, cursor (`ArchiveCursor`) and retry guarantees as cleanup. For each signal it archives, it also:

- removes the id from the category index;
- if the signal was still `Active`, releases the provider's slot and emits `signal_expired`, because cleanup never announced it;
- emits one `signals_archived(count, archived_at)` event per call that removes at least one signal.

After archival the id reports `Removed`. Ids are never reused, so `Removed` is permanent.

`admin_cleanup_storage` runs the same archival sweep, restricted to the admin. `prune_expired_signals` (admin or keeper) is a separate, more aggressive tool: it deletes any signal past expiry without waiting 30 days, and emits `signals_pruned`.

## Operating cleanup

Keepers should call `cleanup_expired_signals(100)` and `archive_old_signals(100)` periodically. Each call does a fixed amount of work, so a keeper cannot exceed per-transaction resource limits however many signals are stored. `get_pending_expiry_count` and `health_check.expired_signal_count` show how much work remains. Delays in cleanup affect storage rent and provider slot accounting, but never what active queries return.
