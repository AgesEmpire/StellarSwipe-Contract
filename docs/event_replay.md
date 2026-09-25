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

## Nonce domain separation (Issue #1081)

Nonces are scoped by an explicit **domain identifier** so a nonce that is valid
in one flow cannot be replayed in another. The domain is derived from three
components and is included in both the nonce storage key and the signed hash:

| Component | Source | Purpose |
| --- | --- | --- |
| `user` | signer address | binds the nonce to the account |
| `operation` | operation type tag (e.g. `stake`, `trade`, `withdraw`) | prevents cross-operation replay |
| `contract` | contract domain id (registry / trading / oracle) | prevents cross-contract replay |

### Domain identifier

```
domain = sha256(user || operation || contract)
```

The domain is stored alongside the nonce and is part of the message that is
hashed for signature verification. A signature produced for
`(user, stake, registry)` therefore fails verification when presented as
`(user, trade, registry)` or `(user, stake, trading)`.

### Storage layout

Nonces are keyed by the full domain rather than by user alone:

```
nonce_key = ("nonce", domain)
```

This keeps independent counters per `(user, operation, contract)` tuple, so
consuming a nonce in one flow does not advance or invalidate another flow.

### Replay safety

- **Cross-operation replay** — a nonce signed for one operation is rejected
  when submitted under a different operation tag, because the recomputed
  domain differs and the stored nonce does not match.
- **Cross-contract replay** — a nonce signed for one contract domain is
  rejected by another contract, because the contract component of the domain
  differs.

Both cases fail closed: verification returns an error and the nonce is not
consumed.

### Migration strategy for existing nonce state

Existing deployments stored nonces keyed by user only. Migration is handled
lazily and is backward compatible:

1. On first use of a `(user, operation, contract)` domain, if no domain-scoped
   nonce exists, the contract reads the legacy user-scoped nonce.
2. The legacy value is adopted as the initial nonce for the new domain and the
   domain-scoped entry is written.
3. The legacy entry is left in place (read-only) so in-flight signatures
   created before the upgrade still verify once; it is never advanced again.
4. After the migration window, the legacy entry can be pruned by an admin
   operation; no on-chain state rewrite is required.

This means no bulk migration transaction is needed and existing clients keep
working during the transition.

### Client guidance

Clients must determine the required nonce domain before signing. The domain is
computed from the same three components the contract uses:

```ts
const domain = sha256(concat(user, operation, contract));
const nonce = await contract.get_nonce(domain);
```

`get_nonce(domain)` is exposed so clients can query the current nonce for the
flow they intend to use. Clients should compute the domain from the operation
they are about to submit and the contract they are targeting, then include the
nonce in the signed payload. Reusing a nonce across operations or contracts
will be rejected on-chain.
