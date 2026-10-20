# Flash Loan Protection — stake_vault

**Issue:** #268 (extension)  
**Status:** Implemented

---

## Protection Strategy

The `stake_vault` contract defends against flash loan exploits with four layered controls:

### 1. Same-Ledger Detection
`deposit_stake` writes `LastStakeLedger(staker) = current_ledger` to temporary storage.  
`withdraw_stake` reads this value and rejects the call with `FlashLoanDetected` if the ledger sequence matches.  
On Soroban a "transaction" maps to a single ledger close — so deposit + withdraw in the same atomic operation is the flash loan attack surface. This check eliminates it.

### 2. Time-Lock for Large Withdrawals
Withdrawals of `>= 500_000_000` stroops (Silver tier or above) require a prior `request_withdrawal` call at least **1 hour** (3 600 s) before the withdrawal executes.  
- Eliminates the ability to borrow large stakes, manipulate valuations, and repay within seconds.  
- The time-lock request is consumed on use; a second large withdrawal requires a fresh request.

### 3. Emergency Pause
Admin can call `pause()` to immediately halt all `deposit_stake` and `withdraw_stake` operations.  
Emits `stake_vault/paused` and `stake_vault/unpaused` events for monitoring hooks.  
Intended for use when the monitoring system detects anomalous patterns.

### 4. Reentrancy Guard (pre-existing)
`withdraw_stake` uses a temporary-storage boolean lock (`WithdrawLock`) to prevent re-entrant calls from a malicious token contract.

---

## Event-Based Monitoring Hooks

| Event | Topic | Payload | Trigger |
|---|---|---|---|
| Flash loan attempt | `stake_vault / flash_loan_attempt` | `(attacker, balance, ledger)` | Same-ledger deposit+withdraw |
| Withdrawal requested | `stake_vault / withdrawal_requested` | `(staker, balance, unlock_at)` | Large withdrawal request |
| Contract paused | `stake_vault / paused` | — | Admin calls `pause()` |
| Contract unpaused | `stake_vault / unpaused` | — | Admin calls `unpause()` |

The monitoring script (`scripts/monitor.ts`) should subscribe to `flash_loan_attempt` events and alert operators.

---

## Security Guarantees

| Attack Vector | Mitigation | Guarantee |
|---|---|---|
| Borrow → stake → withdraw in one tx | Same-ledger detection | Impossible — `FlashLoanDetected` returned |
| Large stake manipulation via borrowed funds | 1-hour time-lock | Attacker cannot withdraw before loan repayment deadline |
| Reentrancy via malicious token | `WithdrawLock` guard | `ReentrancyDetected` on any re-entrant call |
| Ongoing attack during incident response | Emergency pause | All stake/unstake halted instantly |

---

## Constants

```
LARGE_WITHDRAWAL_THRESHOLD     = 500_000_000 stroops  (Silver tier)
LARGE_WITHDRAWAL_TIMELOCK_SECS = 3_600 s               (1 hour)
```
