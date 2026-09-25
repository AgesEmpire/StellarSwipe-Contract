# Event Replay and Topic Version Negotiation

This document describes how indexers and off-chain consumers replay contract
events and how they negotiate event topic schema versions.

## Topic Versioning Convention

Event topics are versioned so that consumers can identify schema changes
without guessing from payload shape. Each versioned event family emits a
topic symbol suffixed with a version identifier:

```
<event_name>_v<major>
```

Examples:

- `signal_registered_v1`
- `signal_updated_v1`
- `signal_removed_v1`

A new topic symbol is introduced whenever the payload schema changes in a
backwards-incompatible way. Additive, backwards-compatible changes keep the
same version.

## Consumer Negotiation

Consumers should:

1. Subscribe to the topic symbols they understand (e.g. `*_v1`).
2. Ignore topic symbols for versions they do not recognize rather than
   attempting to decode them.
3. Log unknown versions so operators can upgrade indexers before the old
   version is retired.

This lets old and new consumers run side by side: an old consumer keeps
processing `_v1` events while a new consumer can begin handling `_v2`
events as soon as they are emitted.

## Migration Guidance

When introducing a new event version:

1. Define the new topic symbol (`<event_name>_v<major+1>`).
2. Emit the new version from the contract for the affected event family.
3. Keep emitting the previous version during a transition window if
   backwards compatibility is required.
4. Update this document and the event schema documentation with the new
   topic symbol and payload shape.
5. Announce the deprecation timeline for the old version so indexers can
   migrate before it is removed.

## Replay

To replay events, query the ledger range of interest and filter by the
versioned topic symbols you support. Because versions are encoded in the
topic, replay tooling can select a specific schema version deterministically
without inspecting payload contents.
