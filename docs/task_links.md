# Task → Source File Cross-Reference

This doc maps the security/resilience tasks we discussed to the concrete source files in the repo.

- Oracle: safer fallback & safe price exposure
  - Core: [stellar-swipe/contracts/oracle/src/lib.rs](stellar-swipe/contracts/oracle/src/lib.rs)
  - Staleness/heartbeat: [stellar-swipe/contracts/oracle/src/staleness.rs](stellar-swipe/contracts/oracle/src/staleness.rs)
  - Storage: [stellar-swipe/contracts/oracle/src/storage.rs](stellar-swipe/contracts/oracle/src/storage.rs)

- Oracle: external adapter signature & reporter validation
  - Adapter: [stellar-swipe/contracts/oracle/src/external_adapter.rs](stellar-swipe/contracts/oracle/src/external_adapter.rs)
  - Types: [stellar-swipe/contracts/oracle/src/types.rs](stellar-swipe/contracts/oracle/src/types.rs)
  - Governance (oracle registry / weights): [stellar-swipe/contracts/oracle/src/governance.rs](stellar-swipe/contracts/oracle/src/governance.rs)

- Fee collector: configurable alternate fee-payment assets
  - Main: [stellar-swipe/contracts/fee_collector/src/lib.rs](stellar-swipe/contracts/fee_collector/src/lib.rs)
  - Storage: [stellar-swipe/contracts/fee_collector/src/storage.rs](stellar-swipe/contracts/fee_collector/src/storage.rs)
  - Rebates/conversion: [stellar-swipe/contracts/fee_collector/src/rebates.rs](stellar-swipe/contracts/fee_collector/src/rebates.rs)

- Portfolio insurance: expose solvency/health metric
  - Insurance logic: [stellar-swipe/contracts/auto_trade/src/portfolio_insurance.rs](stellar-swipe/contracts/auto_trade/src/portfolio_insurance.rs)
  - Public wrapper: [stellar-swipe/contracts/auto_trade/src/lib.rs](stellar-swipe/contracts/auto_trade/src/lib.rs)

Next steps
- I can implement the safe signature checking in `external_adapter.rs` next (verify ed25519 signatures and require registered oracle addresses). Want me to start that change now?

Created by automation: concise mapping to speed implementation and reviews.
