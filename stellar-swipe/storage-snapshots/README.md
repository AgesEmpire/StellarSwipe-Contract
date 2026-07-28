# Storage Layout Snapshots

Committed XDR byte baselines for key `#[contracttype]` types used in persistent
storage. These baselines catch silent breaking changes (field reordering, type
changes) that would corrupt already-stored data or break external clients.

## How it works

The Rust test module `signal_registry::storage_layout_tests` serialises a
representative instance of each type into the Soroban host's XDR encoding,
hex-encodes the result, and compares it against the baseline files here.

## Updating a snapshot intentionally

A breaking storage change **must** be accompanied by a migration. Once the
migration is in place:

1. Delete or truncate the relevant `.hex` file.
2. Re-run `cargo test -- storage_layout_tests` — the test will generate the
   new baseline and print it to stdout (look for `SNAPSHOT_UPDATE`).
3. Paste the printed hex into the `.hex` file and commit it alongside the
   migration code.

CI will fail if a layout changes without an updated baseline.

## Files

| File                   | Type            | Contract          |
|------------------------|-----------------|-------------------|
| `signal_data_v1.hex`   | `SignalDataV1`  | signal_registry   |
| `signal_data_v2.hex`   | `SignalDataV2`  | signal_registry   |
| `signal.hex`           | `Signal`        | signal_registry   |
| `scheduled_signal.hex` | `ScheduledSignal` | signal_registry |

---

## Replay and State Snapshot Workflow

The `scripts/` directory contains two TypeScript tools for capturing and
comparing live contract state, useful for investigating production-like issues
and reproducing edge cases deterministically.

### Tools

| Script | Purpose |
|--------|---------|
| `scripts/snapshot_state.ts` | Fetch all ledger entries for a contract from an RPC node and write them to a JSON file |
| `scripts/compare_snapshots.ts` | Diff two snapshot JSON files and report added, removed, and changed entries |
| `scripts/replay_events.ts` | Fetch contract events over a ledger range and write them to JSON |

### Capturing a snapshot

```bash
# Capture current state of a contract on testnet
npx tsx scripts/snapshot_state.ts <CONTRACT_ID>

# Use a custom RPC endpoint
npx tsx scripts/snapshot_state.ts <CONTRACT_ID> https://soroban-testnet.stellar.org

# Capture before a deployment
npx tsx scripts/snapshot_state.ts <CONTRACT_ID> > /dev/null
# (file written to snapshots/<contract_id>_<ledger>.json)
```

The output file is written to `snapshots/<contract_id>_<ledger>.json` relative
to the current working directory.

### Comparing two snapshots

After capturing a before snapshot, run your transaction or upgrade, then
capture an after snapshot:

```bash
# Before (e.g., at ledger 1000)
npx tsx scripts/snapshot_state.ts CAABC... https://soroban-testnet.stellar.org
# → snapshots/CAABC..._1000.json

# (deploy or invoke)

# After (e.g., at ledger 1010)
npx tsx scripts/snapshot_state.ts CAABC... https://soroban-testnet.stellar.org
# → snapshots/CAABC..._1010.json

# Diff
npx tsx scripts/compare_snapshots.ts \
  snapshots/CAABC..._1000.json \
  snapshots/CAABC..._1010.json
```

**Exit codes:**
- `0` — snapshots are identical; no state was mutated.
- `1` — differences found (added/removed/changed entries); the diff is printed
  to stdout so you can review every changed storage key and its before/after
  decoded value.

### Replaying events

`replay_events.ts` fetches contract events from an RPC node over a ledger range
and writes them as decoded JSON. Combined with `snapshot_state.ts`, this lets
you reconstruct what happened between two known states:

```bash
npx tsx scripts/replay_events.ts \
  --contract CAABC... \
  --start 1000 \
  --end 1010 \
  --output events_1000_1010.json

# Filter by event name
npx tsx scripts/replay_events.ts \
  --contract CAABC... \
  --start 1000 \
  --end 1010 \
  --event trade_executed \
  --output trade_events.json
```

### Reproducing an edge case deterministically

1. **Identify the approximate ledger range** where the issue occurred (from
   monitoring alerts or transaction history).
2. **Snapshot the before state** at the ledger just before the first anomalous
   transaction.
3. **Replay the events** over the affected range to see exactly what was
   emitted.
4. **Snapshot the after state** to see what storage entries changed.
5. **Compare** with `compare_snapshots.ts` to isolate the exact mutation.
6. **Write a regression test** using the minimized inputs from the replay.

### Expected output format

`compare_snapshots.ts` prints a structured diff:

```
Comparing snapshots:
  before: snapshots/CAABC..._1000.json (ledger 1000)
  after:  snapshots/CAABC..._1010.json (ledger 1010)

State diff summary:
  added  : 1
  changed: 2

── ADDED entries ─────────────────────────────────────────────
  key:   {"type":"contract_data","durability":"persistent","key":"user_123"}
  value: {"type":"contract_data","value":{"balance":500000}}

── CHANGED entries ───────────────────────────────────────────
  key:    {"type":"contract_data","durability":"persistent","key":"total_staked"}
  before: {"type":"contract_data","value":{"amount":1000000}}
  after:  {"type":"contract_data","value":{"amount":1500000}}
```
