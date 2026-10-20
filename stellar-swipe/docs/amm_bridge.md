# AMM Bridge Interface

The AMM bridge integrates Stellar-based automated market makers into `auto_trade` for improved swap execution, multi-source price discovery, and resilient fallback routing.

## Architecture

```
Market order
    │
    ▼
execute_swap_with_fallback
    ├── 1. Smart routing (stored venue quotes)
    ├── 2. AMM bridge multi-source plan
    ├── 3. Per-source router fallback chain
    └── 4. SDEX stub (temporary liquidity store)
```

Shared logic lives in `stellar_swipe_common::amm_bridge`. Contract wiring is in `auto_trade::amm_bridge`.

## AMM source kinds

| Kind | Description |
|------|-------------|
| `SdexRouter` | Soroban SDEX / aggregator router (`get_best_ask` + `swap`) |
| `StellarAmm` | Native Stellar AMM pool router |
| `BridgePool` | Bridge constant-product pool |
| `PathPayment` | Path-payment strict-send router |

## Price discovery

`discover_quotes(signal_id, probe_amount)` merges:

1. **Stored venue liquidity** — quotes registered via `upsert_routing_venue`
2. **On-chain router quotes** — `get_best_ask(from_token, to_token)` for each enabled source when a token pair is configured with `set_signal_token_pair`

Quotes are ranked by effective output and used by `plan_multi_source_route` to build a greedy split route with slippage caps.

## Slippage protection

- Each segment carries `min_amount_out` derived from `min_amount_out_with_slippage`
- Route-level slippage is checked against the signal reference price
- Router swaps revert when actual output is below `min_amount_out`

## Fallback chain

When a primary venue or router fails:

1. Failed sources are marked in temporary storage
2. `build_fallback_chain` excludes failed `(kind, source_id)` pairs
3. Remaining sources are tried in priority order
4. Final fallback: `sdex::execute_market_order` (stub liquidity)

Events: `amm_quote_discovered`, `amm_route_planned`, `amm_fallback_used`.

## Public API (`auto_trade`)

| Function | Purpose |
|----------|---------|
| `register_amm_source` | Register or update an AMM router/pool source |
| `get_amm_sources` | List registered sources |
| `set_signal_token_pair` | Bind SAC addresses for router discovery |
| `discover_amm_quotes` | Run price discovery for a signal |
| `preview_amm_route` | Plan a multi-source route without executing |
| `upsert_routing_venue` | Register venue liquidity (existing smart routing) |
| `preview_smart_route` | Preview smart-routing plan |

## Router ABI

Compatible with `trade_executor::sdex`:

- `get_best_ask(from_token, to_token) -> (price, available_qty)`
- `swap(pull_from, from_token, to_token, amount_in, min_out, recipient) -> amount_out`

## Testing

- **Common unit tests**: `cargo test -p stellar_swipe_common amm_bridge`
- **Integration tests with mock routers**: `cargo test -p auto_trade --features testutils --test test_amm_bridge` (requires `auto_trade` crate to compile)

Mock router: `auto_trade::amm_bridge::mock_router::MockAmmRouter` — configurable ask depth, swap output, and failure modes.
