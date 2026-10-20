# Front-running: signal submission and trade execution

This document explains ordering risk on Stellar, how it applies to StellarSwipe’s
**signal submission** and **trade / copy execution**, and optional mitigations
(including **commit–reveal** and **operational** measures).

## Stellar ordering model (relevant facts)

- **Within a single ledger**, transaction order is [not guaranteed in the same
  way as on some other chains](https://developers.stellar.org/docs/learn/fundamentals/transactions#transaction-lifecycle);
  a validator chooses how to build a candidate transaction set. Users should not
  rely on two unrelated submissions landing in a strict, observer-visible order
  in the way they might on a public Ethereum mempool.
- **Still**, any observer who sees a pending (or public) transaction before it
  is included in a **closed** ledger can sometimes **react** in the *same* or
  a *subsequent* submission—especially for actions whose parameters are
  public on-chain (event logs, or announced ahead of time).
- **MEV** as on Ethereum (searcher-bundled ordering, flashbots) is not the
  same on Stellar, but the **adverse ordering** pattern remains: a participant
  may try to move **before** a victim’s trade is finalized if the victim’s
  action is **predictable** and **lucrative to copy** at the DEX/AMM layer.

## Signal submission (SignalRegistry, etc.)

### Nature of the action

- Submitting a **signal** is, by product design, **intentionally public**:
  price, side, category, and timing are part of the feed. There is **no
  “secret” alpha** in a plain signal the chain must hide.
- An adversary might still **submit a competing** signal, **spam**, or
  try to be listed **higher in the same feed** by timing or by gaming sort
  keys—but that is **sibling competition in the public feed**, not
  “stealing” a private order.

### Front-running risk assessment: **low** (in the MEV sense)

- There is no **enclosed user trade** to sandwich at submission time: the
  value at stake is **ranking, visibility, and copy adoption**, not a
  one-sided AMM path against a hidden limit order.
- Mitigations (product / ops) if abuse appears:
  - **Rate limits** and **reputation** (already in the protocol’s direction of travel).
  - **Moderation** or **staking** to raise cost of feed spam.
  - **Off-chain** ordering rules for indexers (not consensus-enforced): first-seen, provider reputation, etc.

**Conclusion:** Front-running in the *trade* sense is **not the primary
concern** for signal publication; the main trust story is **transparency
and non-repudiation of the public signal itself**.

## Trade execution (user trades, copy trade, DEX)

### What is at risk?

When a user (or a router acting for them) executes a **trade** that is **not**
fully public until broadcast:

- A third party that **infers** or **observes** the trade early may **copy
  the same** signal-side trade, **worsen the user’s fill** (e.g. earlier
  trades move the pool), or **arbitrage** the resulting imbalance—depending
  on venue, path, and liquidity.
- The **StellarSwipe** `execute_trade` style flows route through **Soroban**
  contracts and (typically) **SDEX / router** code; **slippage limits and
  balance checks** are the first line of defense, not order secrecy.

### Front-running risk assessment: **higher than for plain signals**

- The **adverse outcome** is economic: a worse **effective price** or
  **partial fill** relative to what a single-user, isolated book would
  have produced.
- Whether an attacker *can* profit in practice depends on: liquidity, pair,
  public router behavior, and **whether** the user’s full intent is visible
  before their transaction closes.

**Conclusion:** Document **slippage**, use **oracles and risk** checks
already in the stack, and for **deeper** guarantees, consider
**commit–reveal** or **private submission** (below)—each with real tradeoffs.

## Mitigations (implemented / recommended)

### Already in line with “don’t be naive”

- **Slippage and min-received** (where the contract stack supports it) cap
  how bad a “copied in front of you” path can be.
- **User auth** (`Address::require_auth` on the trader) ensures only the
  owner (or approved delegate) executes.
- **Rate limits** and **trading pauses** reduce **automated** abuse and
  incident response surface.

### Optional: commit–reveal for trade **intent** (not fully on-chain in this repo)

A **commit–reveal** pattern for *trade* execution is:

1. **Commit (ledger T₁):** user publishes
   `H = SHA-256( domain || user || signal_id || amount || min_out || salt
   || valid_until_ledger )` (and optionally a small **deposit**).
2. **Reveal (ledger T₂, T₂ ≤ valid_until_ledger):** user submits
   `signal_id, amount, min_out, salt`; contract checks
   `hash(...) == H`, then runs the DEX leg.

**Intended effect:** the **full** trade parameters (especially `min_out` and
`salt`) are not known to third parties at commit time, so a generic
front-runner cannot trivially **clone the exact** intent before the reveal
transaction.

#### Tradeoffs of commit–reveal (must be documented in product/UX)

| Benefit | Cost / risk |
|--------|-------------|
| Binds the user to known **min output** and **window**; reduces *simple* “copy the exact frontrun” | **Two** transactions, **higher** fees and **latency**; worse UX for retail |
| **Hiding** `salt` until reveal hides the **exact** size/slippage | Commit **without** a bond may still have **griefing**; bond adds more UX complexity |
| Works with a **time bound** to expire stale commits | If `valid_until` is too long, the market may move; too short, failed reveals |
| Can pair with **priority fee** (if network adds similar mechanics later) | Does **not** by itself stop a **proposer/validator** who can see the **mempool** before inclusion—only raises the **bar** for *public* cloners |
| | **MEV** on a **concentrated-liquidity AMM** is not fully solvable on-chain; **best execution** is partly off-chain and venue-specific |

**Operational note:** on Stellar, as on many L1s, **mempool privacy** is not
guaranteed; commit–reveal is **not** a silver bullet for a *malicious* block
producer, but it **does** help against **opportunistic** copiers of **public
parameters** in common indexer/bot settings.

#### Implementation in this repository

- **`stellar_swipe_common::hash_trade_intent`** (see
  `contracts/common/src/commit_reveal.rs`) defines a **canonical** SHA-256
  over the fields above. Integrators can:
  - Use it **off-chain** to precompute `H` for a future on-chain
    `submit_commit` contract, or
  - Call it from other Soroban contracts in tests / future modules that
    store `H` before `execute`.
- This **does not** yet wire a second-phase `reveal_execute` in
  `auto_trade` or `TradeExecutor`—that would be a product decision (UX,
  fees, storage for pending commits, and event indexing).

**Tests:** unit tests in `commit_reveal.rs` assert **determinism** and
**sensitivity to amount** (see `cargo test -p stellar_swipe_common`).

### Other (non-crypto) mitigations to mention in PRs and ops

- **Private / routing RPC** to reduce *casual* mempool observation (does not
  help against the validator that builds the set).
- **Stellar Turrets / sponsored channels** and **Soroban** fee strategies may
  change *who* can submit first but not global ordering guarantees.
- **Education:** “Signals are public; your trade is competitive once you
  broadcast; use min-out and time bounds.”

## Summary

| Scenario | Relative risk | Primary mitigations in scope |
|----------|---------------|------------------------------|
| **Signal submission** | Low (public by design) | Reputation, rate limits, product rules |
| **Trade execution** | Higher (execution quality) | Slippage, auth, pauses, optional **commit–reveal** (tradeoffs), **canonical hash** helper in common |

## References (internal)

- `contracts/common/src/replay_protection.rs` — replay **nonce** / tx-hash
  dedup (complementary to, not a substitute for, MEV).
- `contracts/common/src/commit_reveal.rs` — `hash_trade_intent` and tests.
- `contracts/auto_trade` / `signal_registry` — `execute_trade`,
  `record_trade_execution` patterns.
