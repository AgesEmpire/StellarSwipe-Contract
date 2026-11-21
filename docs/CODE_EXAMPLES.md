# Code Examples Library

## Introduction

This library provides ready-to-use code examples for common integration scenarios with the StellarSwipe protocol.

---

## Table of Contents

1. [Basic Operations](#basic-operations)
2. [Signal Management](#signal-management)
3. [Staking Operations](#staking-operations)
4. [Analytics Queries](#analytics-queries)
5. [Event Handling](#event-handling)
6. [Advanced Patterns](#advanced-patterns)

---

## Basic Operations

### Example 1: Connect to Network

```typescript
import { SorobanRpc, Networks } from '@stellar/stellar-sdk';

// Testnet connection
const testnetServer = new SorobanRpc.Server(
    'https://soroban-testnet.stellar.org'
);

// Mainnet connection
const mainnetServer = new SorobanRpc.Server(
    'https://soroban-mainnet.stellar.org'
);

// Check connection
async function checkConnection() {
    try {
        const health = await testnetServer.getHealth();
        console.log('Connected:', health.status === 'healthy');
    } catch (error) {
        console.error('Connection failed:', error);
    }
}
```

### Example 2: Initialize Contract Client

```typescript
import { Contract, Address } from '@stellar/stellar-sdk';

const CONTRACT_ID = 'CXXX...';

class StellarSwipeClient {
    private contract: Contract;
    
    constructor(contractId: string) {
        this.contract = new Contract(contractId);
    }
    
    async callMethod(methodName: string, params: any) {
        try {
            const result = await this.contract.call(methodName, params);
            return result;
        } catch (error) {
            console.error(`Error calling ${methodName}:`, error);
            throw error;
        }
    }
}

// Usage
const client = new StellarSwipeClient(CONTRACT_ID);
```

### Example 3: Read Contract Data

```typescript
async function getContractInfo() {
    const contract = new Contract(CONTRACT_ID);
    
    // Get contract version
    const version = await contract.call('get_version');
    console.log('Contract version:', version);
    
    // Get contract config
    const config = await contract.call('get_config');
    console.log('Config:', config);
    
    return { version, config };
}
```

---

## Signal Management

### Example 4: Fetch All Signals

```typescript
async function getAllSignals() {
    const contract = new Contract(CONTRACT_ID);
    
    const signals = await contract.call('get_all_signals');
    
    return signals.map(signal => ({
        id: signal.id,
        provider: signal.provider,
        assetPair: signal.asset_pair,
        entryPrice: signal.entry_price,
        targetPrice: signal.target_price,
        stopLoss: signal.stop_loss,
        status: signal.status,
        timestamp: new Date(signal.timestamp * 1000)
    }));
}

// Usage
const signals = await getAllSignals();
console.log(`Found ${signals.length} signals`);
```

### Example 5: Get Signal by ID

```typescript
async function getSignal(signalId: number) {
    const contract = new Contract(CONTRACT_ID);
    
    try {
        const signal = await contract.call('get_signal', {
            signal_id: signalId
        });
        
        return {
            id: signal.id,
            provider: signal.provider,
            assetPair: signal.asset_pair,
            type: signal.signal_type,
            entryPrice: signal.entry_price,
            targetPrice: signal.target_price,
            stopLoss: signal.stop_loss,
            status: signal.status,
            createdAt: new Date(signal.timestamp * 1000),
            pnl: signal.pnl || 0
        };
    } catch (error) {
        console.error('Signal not found:', signalId);
        return null;
    }
}
```

### Example 6: Filter Signals by Provider

```typescript
async function getProviderSignals(providerAddress: string) {
    const contract = new Contract(CONTRACT_ID);
    
    const signals = await contract.call('get_provider_signals', {
        provider: providerAddress
    });
    
    // Filter by status
    const activeSignals = signals.filter(s => s.status === 'Active');
    const completedSignals = signals.filter(s => s.status === 'Completed');
    
    return {
        all: signals,
        active: activeSignals,
        completed: completedSignals,
        stats: {
            total: signals.length,
            active: activeSignals.length,
            completed: completedSignals.length
        }
    };
}
```

### Example 7: Register New Signal

```typescript
import { Keypair, TransactionBuilder, BASE_FEE } from '@stellar/stellar-sdk';

async function registerSignal(
    providerKeypair: Keypair,
    signalData: {
        assetPair: string;
        signalType: 'Long' | 'Short';
        entryPrice: number;
        targetPrice: number;
        stopLoss: number;
    }
) {
    const contract = new Contract(CONTRACT_ID);
    const server = new SorobanRpc.Server(RPC_URL);
    
    // Get account
    const account = await server.getAccount(providerKeypair.publicKey());
    
    // Build transaction
    const transaction = new TransactionBuilder(account, {
        fee: BASE_FEE,
        networkPassphrase: Networks.TESTNET
    })
    .addOperation(
        contract.call('register_signal', {
            provider: providerKeypair.publicKey(),
            asset_pair: signalData.assetPair,
            signal_type: signalData.signalType,
            entry_price: signalData.entryPrice,
            target_price: signalData.targetPrice,
            stop_loss: signalData.stopLoss
        })
    )
    .setTimeout(30)
    .build();
    
    // Sign transaction
    transaction.sign(providerKeypair);
    
    // Submit transaction
    const result = await server.sendTransaction(transaction);
    
    console.log('Signal registered:', result.hash);
    return result;
}
```

### Example 8: Update Signal Status

```typescript
async function updateSignalStatus(
    providerKeypair: Keypair,
    signalId: number,
    newStatus: 'Active' | 'Completed' | 'Cancelled',
    pnl?: number
) {
    const contract = new Contract(CONTRACT_ID);
    const server = new SorobanRpc.Server(RPC_URL);
    
    const account = await server.getAccount(providerKeypair.publicKey());
    
    const transaction = new TransactionBuilder(account, {
        fee: BASE_FEE,
        networkPassphrase: Networks.TESTNET
    })
    .addOperation(
        contract.call('update_signal', {
            signal_id: signalId,
            status: newStatus,
            pnl: pnl || 0
        })
    )
    .setTimeout(30)
    .build();
    
    transaction.sign(providerKeypair);
    
    const result = await server.sendTransaction(transaction);
    return result;
}
```

---

## Staking Operations

### Example 9: Stake Tokens

```typescript
async function stakeTokens(
    stakerKeypair: Keypair,
    amount: number,
    lockPeriod: number // in seconds
) {
    const contract = new Contract(STAKE_VAULT_CONTRACT_ID);
    const server = new SorobanRpc.Server(RPC_URL);
    
    const account = await server.getAccount(stakerKeypair.publicKey());
    
    const transaction = new TransactionBuilder(account, {
        fee: BASE_FEE,
        networkPassphrase: Networks.TESTNET
    })
    .addOperation(
        contract.call('stake', {
            staker: stakerKeypair.publicKey(),
            amount: amount,
            lock_period: lockPeriod
        })
    )
    .setTimeout(30)
    .build();
    
    transaction.sign(stakerKeypair);
    
    const result = await server.sendTransaction(transaction);
    console.log('Staked:', amount, 'for', lockPeriod, 'seconds');
    
    return result;
}
```

### Example 10: Check Stake Balance

```typescript
async function getStakeBalance(stakerAddress: string) {
    const contract = new Contract(STAKE_VAULT_CONTRACT_ID);
    
    const stakeInfo = await contract.call('get_stake', {
        staker: stakerAddress
    });
    
    return {
        amount: stakeInfo.amount,
        startTime: new Date(stakeInfo.start_time * 1000),
        lockPeriod: stakeInfo.lock_period,
        rewardsEarned: stakeInfo.rewards_earned,
        canUnstake: Date.now() / 1000 > stakeInfo.start_time + stakeInfo.lock_period
    };
}
```

### Example 11: Claim Rewards

```typescript
async function claimRewards(stakerKeypair: Keypair) {
    const contract = new Contract(STAKE_VAULT_CONTRACT_ID);
    const server = new SorobanRpc.Server(RPC_URL);
    
    // Check pending rewards first
    const rewards = await contract.call('get_pending_rewards', {
        staker: stakerKeypair.publicKey()
    });
    
    if (rewards === 0) {
        console.log('No rewards to claim');
        return null;
    }
    
    const account = await server.getAccount(stakerKeypair.publicKey());
    
    const transaction = new TransactionBuilder(account, {
        fee: BASE_FEE,
        networkPassphrase: Networks.TESTNET
    })
    .addOperation(
        contract.call('claim_rewards', {
            staker: stakerKeypair.publicKey()
        })
    )
    .setTimeout(30)
    .build();
    
    transaction.sign(stakerKeypair);
    
    const result = await server.sendTransaction(transaction);
    console.log('Claimed rewards:', rewards);
    
    return result;
}
```

### Example 12: Unstake Tokens

```typescript
async function unstakeTokens(stakerKeypair: Keypair) {
    const contract = new Contract(STAKE_VAULT_CONTRACT_ID);
    const server = new SorobanRpc.Server(RPC_URL);
    
    // Check if can unstake
    const stakeInfo = await getStakeBalance(stakerKeypair.publicKey());
    
    if (!stakeInfo.canUnstake) {
        throw new Error('Lock period not expired');
    }
    
    const account = await server.getAccount(stakerKeypair.publicKey());
    
    const transaction = new TransactionBuilder(account, {
        fee: BASE_FEE,
        networkPassphrase: Networks.TESTNET
    })
    .addOperation(
        contract.call('unstake', {
            staker: stakerKeypair.publicKey()
        })
    )
    .setTimeout(30)
    .build();
    
    transaction.sign(stakerKeypair);
    
    const result = await server.sendTransaction(transaction);
    console.log('Unstaked:', stakeInfo.amount);
    
    return result;
}
```

---

## Analytics Queries

### Example 13: Get Provider Statistics

```typescript
async function getProviderStats(providerAddress: string) {
    const contract = new Contract(CONTRACT_ID);
    
    const stats = await contract.call('get_provider_stats', {
        provider: providerAddress
    });
    
    return {
        totalSignals: stats.total_signals,
        successfulSignals: stats.successful_signals,
        failedSignals: stats.failed_signals,
        winRate: (stats.successful_signals / stats.total_signals * 100).toFixed(2) + '%',
        totalProfit: stats.total_profit,
        totalLoss: stats.total_loss,
        netPnL: stats.total_profit + stats.total_loss,
        reputationScore: stats.reputation_score,
        avgProfitPerSignal: stats.avg_profit_per_signal
    };
}

// Usage
const stats = await getProviderStats('GXXX...');
console.log('Provider Stats:', stats);
```

### Example 14: Calculate Performance Metrics

```typescript
async function calculatePerformanceMetrics(providerAddress: string) {
    const signals = await getProviderSignals(providerAddress);
    const completed = signals.completed;
    
    if (completed.length === 0) {
        return null;
    }
    
    // Calculate metrics
    const totalPnL = completed.reduce((sum, s) => sum + s.pnl, 0);
    const avgPnL = totalPnL / completed.length;
    
    const profitable = completed.filter(s => s.pnl > 0);
    const unprofitable = completed.filter(s => s.pnl < 0);
    
    const totalProfit = profitable.reduce((sum, s) => sum + s.pnl, 0);
    const totalLoss = Math.abs(unprofitable.reduce((sum, s) => sum + s.pnl, 0));
    
    const profitFactor = totalLoss > 0 ? totalProfit / totalLoss : 0;
    
    // Calculate max drawdown
    let peak = 0;
    let maxDrawdown = 0;
    let cumulative = 0;
    
    for (const signal of completed) {
        cumulative += signal.pnl;
        if (cumulative > peak) peak = cumulative;
        const drawdown = peak - cumulative;
        if (drawdown > maxDrawdown) maxDrawdown = drawdown;
    }
    
    return {
        totalSignals: completed.length,
        winRate: (profitable.length / completed.length * 100).toFixed(2) + '%',
        totalPnL,
        avgPnL,
        profitFactor: profitFactor.toFixed(2),
        maxDrawdown,
        bestTrade: Math.max(...completed.map(s => s.pnl)),
        worstTrade: Math.min(...completed.map(s => s.pnl))
    };
}
```

### Example 15: Get Historical Performance

```typescript
async function getHistoricalPerformance(
    providerAddress: string,
    startDate: Date,
    endDate: Date
) {
    const signals = await getProviderSignals(providerAddress);
    
    // Filter by date range
    const filtered = signals.all.filter(s => {
        const signalDate = new Date(s.timestamp * 1000);
        return signalDate >= startDate && signalDate <= endDate;
    });
    
    // Group by day
    const dailyPerformance = new Map();
    
    for (const signal of filtered) {
        const date = new Date(signal.timestamp * 1000).toDateString();
        
        if (!dailyPerformance.has(date)) {
            dailyPerformance.set(date, {
                date,
                signals: [],
                totalPnL: 0
            });
        }
        
        const day = dailyPerformance.get(date);
        day.signals.push(signal);
        day.totalPnL += signal.pnl || 0;
    }
    
    return Array.from(dailyPerformance.values());
}
```

---

## Event Handling

### Example 16: Listen for Events

```typescript
async function listenForEvents(contractId: string) {
    const server = new SorobanRpc.Server(RPC_URL);
    
    // Get latest ledger
    let lastLedger = await server.getLatestLedger();
    
    // Poll for new events
    setInterval(async () => {
        const currentLedger = await server.getLatestLedger();
        
        if (currentLedger.sequence > lastLedger.sequence) {
            // Fetch events for new ledgers
            const events = await server.getEvents({
                startLedger: lastLedger.sequence + 1,
                filters: [
                    {
                        type: 'contract',
                        contractIds: [contractId]
                    }
                ]
            });
            
            // Process events
            for (const event of events.events) {
                handleEvent(event);
            }
            
            lastLedger = currentLedger;
        }
    }, 5000); // Check every 5 seconds
}

function handleEvent(event: any) {
    const topic = event.topic[0];
    
    switch (topic) {
        case 'signal_created':
            console.log('New signal created:', event.value);
            break;
        case 'signal_updated':
            console.log('Signal updated:', event.value);
            break;
        case 'stake_deposited':
            console.log('Stake deposited:', event.value);
            break;
        default:
            console.log('Unknown event:', topic);
    }
}
```

### Example 17: Event Filtering

```typescript
async function getSignalEvents(signalId: number) {
    const server = new SorobanRpc.Server(RPC_URL);
    
    const events = await server.getEvents({
        startLedger: 0,
        filters: [
            {
                type: 'contract',
                contractIds: [CONTRACT_ID],
                topics: [['signal_created', 'signal_updated']]
            }
        ]
    });
    
    // Filter by signal ID
    return events.events.filter(e => {
        return e.value.signal_id === signalId;
    });
}
```

---

## Advanced Patterns

### Example 18: Batch Operations

```typescript
async function batchGetSignals(signalIds: number[]) {
    const contract = new Contract(CONTRACT_ID);
    
    // Fetch all signals in parallel
    const promises = signalIds.map(id =>
        contract.call('get_signal', { signal_id: id })
            .catch(err => null) // Handle individual failures
    );
    
    const results = await Promise.all(promises);
    
    // Filter out failed requests
    return results.filter(r => r !== null);
}

// Usage
const signals = await batchGetSignals([1, 2, 3, 4, 5]);
```

### Example 19: Retry Logic

```typescript
async function retryOperation<T>(
    operation: () => Promise<T>,
    maxRetries: number = 3,
    delayMs: number = 1000
): Promise<T> {
    for (let i = 0; i < maxRetries; i++) {
        try {
            return await operation();
        } catch (error) {
            if (i === maxRetries - 1) throw error;
            
            console.log(`Retry ${i + 1}/${maxRetries} after ${delayMs}ms`);
            await new Promise(resolve => setTimeout(resolve, delayMs));
            delayMs *= 2; // Exponential backoff
        }
    }
    
    throw new Error('Max retries exceeded');
}

// Usage
const signal = await retryOperation(() => getSignal(123));
```

### Example 20: Transaction Simulation

```typescript
async function simulateTransaction(transaction: Transaction) {
    const server = new SorobanRpc.Server(RPC_URL);
    
    try {
        const simulation = await server.simulateTransaction(transaction);
        
        if (simulation.error) {
            console.error('Simulation failed:', simulation.error);
            return null;
        }
        
        return {
            success: true,
            cost: simulation.cost,
            result: simulation.result,
            events: simulation.events
        };
    } catch (error) {
        console.error('Simulation error:', error);
        return null;
    }
}
```

---

## Helper Functions

### Example 21: Format Values

```typescript
function formatPrice(price: number): string {
    return new Intl.NumberFormat('en-US', {
        style: 'currency',
        currency: 'USD',
        minimumFractionDigits: 2,
        maximumFractionDigits: 6
    }).format(price);
}

function formatPercentage(value: number): string {
    return (value * 100).toFixed(2) + '%';
}

function formatTimestamp(timestamp: number): string {
    return new Date(timestamp * 1000).toLocaleString();
}

// Usage
console.log(formatPrice(1234.56789)); // $1,234.567890
console.log(formatPercentage(0.7523)); // 75.23%
console.log(formatTimestamp(1234567890)); // 2/13/2009, 6:31:30 PM
```

### Example 22: Validation Helpers

```typescript
function validateSignalData(data: any): boolean {
    if (!data.assetPair || typeof data.assetPair !== 'string') {
        return false;
    }
    
    if (!['Long', 'Short'].includes(data.signalType)) {
        return false;
    }
    
    if (data.entryPrice <= 0 || data.targetPrice <= 0 || data.stopLoss <= 0) {
        return false;
    }
    
    // Validate risk/reward ratio
    const riskReward = Math.abs(data.targetPrice - data.entryPrice) /
                       Math.abs(data.entryPrice - data.stopLoss);
    
    if (riskReward < 1) {
        console.warn('Risk/reward ratio less than 1:1');
    }
    
    return true;
}
```

---

## Complete Example: Trading Application

```typescript
class TradingApp {
    private client: StellarSwipeClient;
    private userKeypair: Keypair;
    
    constructor(secretKey: string, contractId: string) {
        this.userKeypair = Keypair.fromSecret(secretKey);
        this.client = new StellarSwipeClient(contractId);
    }
    
    async initialize() {
        console.log('Initializing trading app...');
        await this.client.connect();
        console.log('Connected to network');
    }
    
    async getActiveSignals() {
        const signals = await getAllSignals();
        return signals.filter(s => s.status === 'Active');
    }
    
    async analyzeSignal(signalId: number) {
        const signal = await getSignal(signalId);
        if (!signal) return null;
        
        const providerStats = await getProviderStats(signal.provider);
        
        return {
            signal,
            providerStats,
            recommendation: this.generateRecommendation(signal, providerStats)
        };
    }
    
    private generateRecommendation(signal: any, stats: any) {
        const winRate = parseFloat(stats.winRate);
        const riskReward = Math.abs(signal.targetPrice - signal.entryPrice) /
                          Math.abs(signal.entryPrice - signal.stopLoss);
        
        if (winRate > 70 && riskReward > 2) {
            return 'STRONG_BUY';
        } else if (winRate > 60 && riskReward > 1.5) {
            return 'BUY';
        } else if (winRate > 50) {
            return 'HOLD';
        } else {
            return 'AVOID';
        }
    }
    
    async executeSignal(signalId: number) {
        const analysis = await this.analyzeSignal(signalId);
        
        if (analysis.recommendation === 'STRONG_BUY' || 
            analysis.recommendation === 'BUY') {
            console.log('Executing signal:', signalId);
            // Execute trade logic here
            return true;
        }
        
        console.log('Signal not recommended:', analysis.recommendation);
        return false;
    }
}

// Usage
const app = new TradingApp('SXXX...', 'CXXX...');
await app.initialize();

const activeSignals = await app.getActiveSignals();
for (const signal of activeSignals) {
    await app.executeSignal(signal.id);
}
```

---

## Testing Examples

### Example 23: Unit Test

```typescript
import { describe, it, expect } from '@jest/globals';

describe('Signal Operations', () => {
    it('should fetch signal by ID', async () => {
        const signal = await getSignal(1);
        
        expect(signal).toBeDefined();
        expect(signal.id).toBe(1);
        expect(signal.provider).toBeDefined();
    });
    
    it('should calculate correct win rate', async () => {
        const stats = await getProviderStats('GXXX...');
        
        const expectedWinRate = (stats.successfulSignals / stats.totalSignals * 100).toFixed(2);
        expect(stats.winRate).toBe(expectedWinRate + '%');
    });
});
```

---

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01  
**Maintained By**: StellarSwipe Core Team
