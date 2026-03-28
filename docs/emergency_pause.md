# Emergency Pause Runbook

## Pause All Contracts
```bash
# Pause signal submission
soroban contract invoke \\
  --source stellar-swipe-contract \\
  --network testnet \\
  --wasm stellar-swipe-signal-registry.wasm \\
  --id CONTRACT_ID_SIGNAL \\
  pause --arg admin

# Pause auto-trading
soroban contract invoke \\
  --source stellar-swipe-contract \\
  --network testnet \\
  --wasm stellar-swipe-auto-trade.wasm \\
  --id CONTRACT_ID_AUTO_TRADE \\
  pause --arg admin

# Repeat for oracle, bridge, governance
```

## Verify Pause
Getters work, mutators fail with ContractPaused.

## Unpause
Replace `pause` with `unpause`.

## When to Use
- Oracle failure
- Exploit discovered
- Circuit breaker triggered

Updated: SignalRegistry supports granular pause_category("CAT_SIGNALS").
