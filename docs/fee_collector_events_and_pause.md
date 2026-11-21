# Fee Collector: Collection Events and Emergency Pause (issues #1247, #1248)

This page covers the `FeeCollector` contract in `contracts/fee_collector`. It
is for indexer authors and for operators who run incident response.

## Interface changes

| Entrypoint | Signature | Notes |
|------------|-----------|-------|
| `initialize` | `(admin: Address, fee_asset: Address) -> Result<(), FeeCollectorError>` | Now records the fee asset. Fails with `AlreadyInitialized` on a second call. |
| `collect` | `(source: Address, amount: i128) -> Result<i128, FeeCollectorError>` | `source` must authorize. Returns the resulting balance. |
| `fee_asset` | `() -> Result<Address, FeeCollectorError>` | New read-only query. |
| `set_collection_paused` | `(caller: Address, paused: bool) -> Result<(), FeeCollectorError>` | New. Admin only. |
| `is_collection_paused` | `() -> bool` | New read-only query. |

New error codes: `AlreadyInitialized = 7`, `CollectionPaused = 8`,
`PauseStateUnchanged = 9`, `ArithmeticOverflow = 10`, and `InvalidAddress = 100`
(see `docs/address_validation_audit.md`). Codes 1–6 are unchanged.

## `fees_collected` event

Every successful `collect` publishes exactly one event:

| Part | Value |
|------|-------|
| Topic 0 | `Symbol("fees_collected")` |
| Topic 1 | `asset: Address`, the collector's fee asset |
| Topic 2 | `source: Address`, the account or contract that authorized the collection |
| Data | `FeesCollected { asset: Address, source: Address, amount: i128, balance: i128 }` |

* `amount` is strictly positive.
* `balance` is the collector balance **after** `amount` was added.
* `source` must authorize the call (`require_auth`), so an event's attribution
  cannot be forged by a third party.
* The asset and source are repeated in the topics so consumers can filter
  without decoding the data.

### Replaying the balance

The collector balance changes in exactly two ways:

| Event | Effect on balance |
|-------|-------------------|
| `fees_collected` | `+ amount` |
| `dust_swept` (`DustSwept { caller, destination, amount }`) | `- amount` |

Start from `0` at initialization and apply both event types in ledger order.
The running total matches each `fees_collected.balance`, which works as a
checkpoint: a mismatch means the indexer missed an event.

### Failures

A `collect` call that fails returns an error, writes nothing and publishes no
event. Possible failures:

| Condition | Error |
|-----------|-------|
| `source` is a sentinel address | `InvalidAddress` |
| `source` did not authorize | host auth error |
| Collector not initialized | `NotInitialized` |
| Collection is paused | `CollectionPaused` |
| `amount <= 0` | `InvalidAmount` |
| `balance + amount` overflows `i128` | `ArithmeticOverflow` |

## Emergency collection pause

### Who can pause and resume

Only the current admin can pause or resume, by calling
`set_collection_paused(admin, true | false)` with the admin's authorization.
Any other caller fails with `Unauthorized`, and naming the admin without the
admin's signature fails the auth check. If the admin is rotated with
`set_admin`, pause control moves to the new admin.

`initialize` is now one-time only. Without that, anyone could re-initialize
the contract, make themselves admin and lift the pause.

### Transitions

| Current | Requested | Result |
|---------|-----------|--------|
| running | `true` | Paused; emits `pause_changed` |
| paused | `false` | Resumed; emits `pause_changed` |
| running | `false` | `PauseStateUnchanged`; no event |
| paused | `true` | `PauseStateUnchanged`; no event |

Redundant transitions are rejected, so each `pause_changed` event marks a real
change of state.

`pause_changed` event:

| Part | Value |
|------|-------|
| Topic 0 | `Symbol("pause_changed")` |
| Data | `PauseChanged { paused: bool, actor: Address }`, where `actor` is the admin that authorized the change |

### Behavior while paused

| Entrypoint | While paused |
|------------|--------------|
| `collect` | Fails closed with `CollectionPaused`; no state change, no event |
| `balance`, `fee_asset`, `is_collection_paused` | Work normally |
| `sweep_dust` | **Available.** This is the approved recovery path. It is admin-only, limited to amounts below `DUST_THRESHOLD`, cannot send to the caller or to a sentinel address, and still emits `dust_swept`. |
| `set_admin` | Available, so a compromised admin key can be rotated during an incident |
| `set_collection_paused` | Admin can resume |

### Tests

`contracts/fee_collector/src/lib.rs`:

* Events: `collect_emits_structured_event_with_resulting_balance`,
  `collection_events_replay_to_the_stored_balance`,
  `failed_collections_emit_no_event_and_keep_balance`,
  `collect_requires_source_authorization`, `collect_before_initialize_is_rejected`.
* Pause: `pause_blocks_collection_but_keeps_queries_and_dust_recovery`,
  `pause_transitions_emit_auditable_events`,
  `redundant_pause_transition_is_rejected_without_event`,
  `non_admin_cannot_pause_or_resume`, `pause_requires_admin_signature`,
  `rotated_admin_controls_pause`, `rejects_reinitialization`.
