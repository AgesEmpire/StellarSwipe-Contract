# Local Simulator

This simulator provides an in-memory mock of StellarSwipe contract behavior for frontend development.

## Supported contracts

- `TradeExecutor`
- `UserPortfolio`
- `FeeCollector`
- `SignalRegistry`
- `Oracle`

## Usage

The simulator exposes the contract classes in `scripts/simulator/index.ts`.

Example:

```ts
import { Simulator } from "./scripts/simulator/index.ts";

const sim = new Simulator();
const admin = sim.createAddress("admin");
const user = sim.createAddress("user");

sim.oracle.initialize(admin);
sim.oracle.setPrice(1, 1000n);

sim.userPortfolio.initialize(admin, sim.oracle.address);
```

## Notes

- The simulator uses in-memory storage only.
- It is intended for local frontend integration and regression testing.
- Behavior is simplified but covers happy paths and common contract error cases.
