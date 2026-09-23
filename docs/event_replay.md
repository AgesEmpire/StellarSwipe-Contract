# Event replay (Issue #882)

Reconstructing protocol state from historical on-chain events, for debugging
and for validating upgrades against what actually happened on chain.

## Pipeline

1. `scripts/replay_events.ts` — fetch raw Soroban events from an RPC endpoint.
2. `scripts/test_event_parsing.ts` — parse them into the native JSON shape
   (see `scripts/sample_parsed_events_testnet.json`).
3. `scripts/replay_state.ts` — fold that parsed stream into a state snapshot.

```sh
cd scripts
npx tsx replay_state.ts sample_parsed_events_testnet.json
```

## What the snapshot contains

| Field | Reconstructed from |
| --- | --- |
| `stakes` | `staked` / `unstaked` |
| `signals` | `trade_executed` (count and cumulative volume per signal) |
| `feesCollected` | `fee_collected` |
| `lastOraclePrice` | `oracle_price_submitted` |
| `unhandledEvents` | any event name with no reducer yet |

Amounts are `bigint`; `serialize()` renders them as decimal strings so a
snapshot can be diffed or committed as JSON.

## Determinism

Events are sorted by ledger (then event id) before folding, so a page returned
out of order by the RPC replays to the same snapshot. `unhandledEvents` makes
coverage gaps visible instead of silently dropping events — when a contract
gains a new event, add a reducer case and it disappears from that list.

## Fixture test

`scripts/fixtures/replay_events_fixture.json` is a known-good event stream with
a deliberately out-of-order ledger and one unhandled event name. The replay is
asserted against its expected snapshot:

```sh
cd scripts
npx tsx --test replay_state.test.ts
```
