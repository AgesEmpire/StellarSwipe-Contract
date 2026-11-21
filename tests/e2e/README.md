# E2E Tests

`test_full_flow.ts` is a live-network smoke test that reads `deployments/testnet.json`
and invokes deployed contracts on Stellar testnet using `@stellar/stellar-sdk`.

Required environment:

- `STELLAR_E2E_ADMIN_SECRET`
- `STELLAR_E2E_TRADER_SECRET`
- `STELLAR_E2E_PROVIDER_SECRET`
- `STELLAR_E2E_FEE_TOKEN`

Optional overrides:

- `STELLAR_E2E_DEPLOYMENT_STATE`
- `STELLAR_E2E_RPC_URL`
- `STELLAR_E2E_NETWORK_PASSPHRASE`
- `STELLAR_E2E_GOVERNANCE_SUPPLY`
- `STELLAR_E2E_TRADE_ASSET_CODE`
- `STELLAR_E2E_TRADE_ASSET_ISSUER`

The nightly workflow runs this script from `scripts/` so it can reuse the existing
`tsx` toolchain dependency.
