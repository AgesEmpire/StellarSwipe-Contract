# Integration Tutorials

## Introduction

This guide provides step-by-step tutorials for integrating with the StellarSwipe protocol. Whether you're building a frontend application, creating a trading bot, or integrating signal data into your platform, these tutorials will help you get started.

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [Frontend Integration](#frontend-integration)
3. [Backend Integration](#backend-integration)
4. [Trading Bot Integration](#trading-bot-integration)
5. [Analytics Integration](#analytics-integration)
6. [Webhook Integration](#webhook-integration)

---

## Quick Start

### Prerequisites

- Node.js 16+ or Python 3.8+
- Stellar account with testnet XLM
- Basic understanding of blockchain concepts

### 5-Minute Integration

```typescript
import { SorobanRpc, Contract, Networks } from '@stellar/stellar-sdk';

// 1. Connect to network
const server = new SorobanRpc.Server('https://soroban-testnet.stellar.org');

// 2. Initialize contract
const contractId = 'CXXX...'; // Signal Registry contract
const contract = new Contract(contractId);

// 3. Read signal data
async function getSignal(signalId: number) {
    const result = await contract.call('get_signal', {
        signal_id: signalId
    });
    return result;
}

// 4. Use the data
const signal = await getSignal(123);
console.log('Signal:', signal);
```

---

## Frontend Integration

### Tutorial 1: React Application

#### Step 1: Setup Project

```bash
npx create-react-app stellarswipe-app
cd stellarswipe-app
npm install @stellar/stellar-sdk
```

#### Step 2: Create Stellar Service

**`src/services/stellar.ts`**:
```typescript
import { 
    SorobanRpc, 
    Contract, 
    TransactionBuilder,
    Networks,
    BASE_FEE
} from '@stellar/stellar-sdk';

export class StellarService {
    private server: SorobanRpc.Server;
    private contractId: string;
    
    constructor(rpcUrl: string, contractId: string) {
        this.server = new SorobanRpc.Server(rpcUrl);
        this.contractId = contractId;
    }
    
    async getSignals(providerId: string) {
        const contract = new Contract(this.contractId);
        
        try {
            const result = await contract.call('get_provider_signals', {
                provider: providerId
            });
            return result;
        } catch (error) {
            console.error('Error fetching signals:', error);
            throw error;
        }
    }
    
    async registerSignal(
        provider: string,
        signalData: SignalData
    ) {
        const contract = new Contract(this.contractId);
        
        // Build transaction
        const account = await this.server.getAccount(provider);
        const transaction = new TransactionBuilder(account, {
            fee: BASE_FEE,
            networkPassphrase: Networks.TESTNET
        })
        .addOperation(contract.call('register_signal', signalData))
        .setTimeout(30)
        .build();
        
        // Sign and submit
        // ... signing logic
        
        return transaction;
    }
}
```

#### Step 3: Create React Hook

**`src/hooks/useStellarSwipe.ts`**:
```typescript
import { useState, useEffect } from 'react';
import { StellarService } from '../services/stellar';

export function useStellarSwipe() {
    const [signals, setSignals] = useState([]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(null);
    
    const service = new StellarService(
        process.env.REACT_APP_RPC_URL,
        process.env.REACT_APP_CONTRACT_ID
    );
    
    const fetchSignals = async (providerId: string) => {
        setLoading(true);
        try {
            const data = await service.getSignals(providerId);
            setSignals(data);
        } catch (err) {
            setError(err.message);
        } finally {
            setLoading(false);
        }
    };
    
    return { signals, loading, error, fetchSignals };
}
```

#### Step 4: Create Component

**`src/components/SignalList.tsx`**:
```typescript
import React, { useEffect } from 'react';
import { useStellarSwipe } from '../hooks/useStellarSwipe';

export function SignalList({ providerId }: { providerId: string }) {
    const { signals, loading, error, fetchSignals } = useStellarSwipe();
    
    useEffect(() => {
        fetchSignals(providerId);
    }, [providerId]);
    
    if (loading) return <div>Loading...</div>;
    if (error) return <div>Error: {error}</div>;
    
    return (
        <div className="signal-list">
            {signals.map(signal => (
                <div key={signal.id} className="signal-card">
                    <h3>{signal.asset_pair}</h3>
                    <p>Entry: {signal.entry_price}</p>
                    <p>Target: {signal.target_price}</p>
                    <p>Stop Loss: {signal.stop_loss}</p>
                    <span className={`status ${signal.status}`}>
                        {signal.status}
                    </span>
                </div>
            ))}
        </div>
    );
}
```

#### Step 5: Wallet Integration

**`src/services/wallet.ts`**:
```typescript
import { Keypair } from '@stellar/stellar-sdk';

export class WalletService {
    async connectFreighter() {
        if (!window.freighter) {
            throw new Error('Freighter wallet not installed');
        }
        
        const publicKey = await window.freighter.getPublicKey();
        return publicKey;
    }
    
    async signTransaction(xdr: string) {
        if (!window.freighter) {
            throw new Error('Freighter wallet not installed');
        }
        
        const signedXdr = await window.freighter.signTransaction(xdr);
        return signedXdr;
    }
}
```

---

## Backend Integration

### Tutorial 2: Node.js API Server

#### Step 1: Setup Express Server

```bash
mkdir stellarswipe-api
cd stellarswipe-api
npm init -y
npm install express @stellar/stellar-sdk dotenv
```

#### Step 2: Create API Server

**`src/server.js`**:
```javascript
const express = require('express');
const { SorobanRpc, Contract } = require('@stellar/stellar-sdk');
require('dotenv').config();

const app = express();
app.use(express.json());

const server = new SorobanRpc.Server(process.env.RPC_URL);
const contractId = process.env.CONTRACT_ID;

// Get all signals
app.get('/api/signals', async (req, res) => {
    try {
        const contract = new Contract(contractId);
        const result = await contract.call('get_all_signals');
        res.json(result);
    } catch (error) {
        res.status(500).json({ error: error.message });
    }
});

// Get signal by ID
app.get('/api/signals/:id', async (req, res) => {
    try {
        const contract = new Contract(contractId);
        const result = await contract.call('get_signal', {
            signal_id: parseInt(req.params.id)
        });
        res.json(result);
    } catch (error) {
        res.status(404).json({ error: 'Signal not found' });
    }
});

// Get provider stats
app.get('/api/providers/:address', async (req, res) => {
    try {
        const contract = new Contract(contractId);
        const result = await contract.call('get_provider_stats', {
            provider: req.params.address
        });
        res.json(result);
    } catch (error) {
        res.status(500).json({ error: error.message });
    }
});

app.listen(3000, () => {
    console.log('API server running on port 3000');
});
```

#### Step 3: Add Caching Layer

**`src/cache.js`**:
```javascript
const NodeCache = require('node-cache');
const cache = new NodeCache({ stdTTL: 60 }); // 60 second TTL

async function getCachedSignals(fetchFunction) {
    const cacheKey = 'all_signals';
    
    // Check cache
    const cached = cache.get(cacheKey);
    if (cached) {
        return cached;
    }
    
    // Fetch and cache
    const data = await fetchFunction();
    cache.set(cacheKey, data);
    return data;
}

module.exports = { getCachedSignals };
```

#### Step 4: Add WebSocket Support

**`src/websocket.js`**:
```javascript
const WebSocket = require('ws');

function setupWebSocket(server) {
    const wss = new WebSocket.Server({ server });
    
    wss.on('connection', (ws) => {
        console.log('Client connected');
        
        ws.on('message', (message) => {
            const data = JSON.parse(message);
            
            if (data.action === 'subscribe') {
                // Subscribe to signal updates
                subscribeToSignals(ws, data.filter);
            }
        });
        
        ws.on('close', () => {
            console.log('Client disconnected');
        });
    });
}

function subscribeToSignals(ws, filter) {
    // Poll for updates and send to client
    setInterval(async () => {
        const signals = await fetchSignals(filter);
        ws.send(JSON.stringify({
            type: 'signal_update',
            data: signals
        }));
    }, 5000); // Every 5 seconds
}

module.exports = { setupWebSocket };
```

---

## Trading Bot Integration

### Tutorial 3: Automated Trading Bot

#### Step 1: Bot Structure

**`bot/index.py`**:
```python
import asyncio
from stellar_sdk import Server, Keypair, TransactionBuilder, Network
from stellar_sdk.soroban_rpc import SorobanServer

class TradingBot:
    def __init__(self, secret_key, contract_id):
        self.keypair = Keypair.from_secret(secret_key)
        self.server = SorobanServer("https://soroban-testnet.stellar.org")
        self.contract_id = contract_id
        
    async def monitor_signals(self):
        """Monitor for new signals"""
        while True:
            signals = await self.fetch_signals()
            
            for signal in signals:
                if self.should_execute(signal):
                    await self.execute_trade(signal)
            
            await asyncio.sleep(10)  # Check every 10 seconds
    
    async def fetch_signals(self):
        """Fetch active signals from contract"""
        # Call contract to get signals
        result = await self.server.invoke_contract_function(
            contract_id=self.contract_id,
            function_name="get_active_signals",
            parameters=[]
        )
        return result
    
    def should_execute(self, signal):
        """Determine if signal meets criteria"""
        # Check signal quality
        if signal['provider_reputation'] < 70:
            return False
        
        # Check risk parameters
        risk_reward = (signal['target_price'] - signal['entry_price']) / \
                     (signal['entry_price'] - signal['stop_loss'])
        
        if risk_reward < 2.0:  # Minimum 2:1 risk/reward
            return False
        
        return True
    
    async def execute_trade(self, signal):
        """Execute trade based on signal"""
        print(f"Executing trade for signal {signal['id']}")
        
        # Place order on exchange
        # This would integrate with your exchange API
        order = {
            'symbol': signal['asset_pair'],
            'side': signal['signal_type'],
            'entry': signal['entry_price'],
            'target': signal['target_price'],
            'stop_loss': signal['stop_loss']
        }
        
        # Log trade
        await self.log_trade(signal['id'], order)
    
    async def log_trade(self, signal_id, order):
        """Log trade execution"""
        print(f"Trade logged: Signal {signal_id}, Order {order}")

# Run bot
if __name__ == "__main__":
    bot = TradingBot(
        secret_key="SXXX...",
        contract_id="CXXX..."
    )
    asyncio.run(bot.monitor_signals())
```

#### Step 2: Risk Management

**`bot/risk_manager.py`**:
```python
class RiskManager:
    def __init__(self, max_position_size, max_daily_loss):
        self.max_position_size = max_position_size
        self.max_daily_loss = max_daily_loss
        self.daily_pnl = 0
        
    def can_open_position(self, position_size):
        """Check if position can be opened"""
        if position_size > self.max_position_size:
            return False
        
        if abs(self.daily_pnl) >= self.max_daily_loss:
            return False
        
        return True
    
    def calculate_position_size(self, signal, account_balance):
        """Calculate appropriate position size"""
        risk_per_trade = 0.02  # 2% risk per trade
        
        entry = signal['entry_price']
        stop_loss = signal['stop_loss']
        risk_amount = abs(entry - stop_loss)
        
        position_size = (account_balance * risk_per_trade) / risk_amount
        
        return min(position_size, self.max_position_size)
```

---

## Analytics Integration

### Tutorial 4: Analytics Dashboard

#### Step 1: Data Fetching Service

**`analytics/data_service.ts`**:
```typescript
export class AnalyticsDataService {
    private contractId: string;
    private server: SorobanRpc.Server;
    
    async getProviderPerformance(providerId: string, period: string) {
        // Fetch historical data
        const signals = await this.fetchProviderSignals(providerId);
        
        // Calculate metrics
        const metrics = this.calculateMetrics(signals, period);
        
        return {
            win_rate: metrics.winRate,
            total_pnl: metrics.totalPnl,
            sharpe_ratio: metrics.sharpeRatio,
            max_drawdown: metrics.maxDrawdown,
            signals_count: signals.length
        };
    }
    
    private calculateMetrics(signals: Signal[], period: string) {
        const filtered = this.filterByPeriod(signals, period);
        
        const successful = filtered.filter(s => s.pnl > 0).length;
        const winRate = (successful / filtered.length) * 100;
        
        const totalPnl = filtered.reduce((sum, s) => sum + s.pnl, 0);
        
        // Calculate Sharpe ratio
        const returns = filtered.map(s => s.pnl);
        const avgReturn = returns.reduce((a, b) => a + b, 0) / returns.length;
        const stdDev = this.calculateStdDev(returns, avgReturn);
        const sharpeRatio = avgReturn / stdDev;
        
        // Calculate max drawdown
        const maxDrawdown = this.calculateMaxDrawdown(filtered);
        
        return {
            winRate,
            totalPnl,
            sharpeRatio,
            maxDrawdown
        };
    }
    
    private calculateStdDev(values: number[], mean: number): number {
        const squaredDiffs = values.map(v => Math.pow(v - mean, 2));
        const variance = squaredDiffs.reduce((a, b) => a + b, 0) / values.length;
        return Math.sqrt(variance);
    }
    
    private calculateMaxDrawdown(signals: Signal[]): number {
        let peak = 0;
        let maxDrawdown = 0;
        let cumulative = 0;
        
        for (const signal of signals) {
            cumulative += signal.pnl;
            if (cumulative > peak) {
                peak = cumulative;
            }
            const drawdown = (peak - cumulative) / peak;
            if (drawdown > maxDrawdown) {
                maxDrawdown = drawdown;
            }
        }
        
        return maxDrawdown * 100; // Return as percentage
    }
}
```

#### Step 2: Chart Component

**`components/PerformanceChart.tsx`**:
```typescript
import React from 'react';
import { Line } from 'react-chartjs-2';

export function PerformanceChart({ data }: { data: PerformanceData }) {
    const chartData = {
        labels: data.timestamps,
        datasets: [
            {
                label: 'Cumulative PnL',
                data: data.cumulativePnl,
                borderColor: 'rgb(75, 192, 192)',
                tension: 0.1
            }
        ]
    };
    
    const options = {
        responsive: true,
        plugins: {
            legend: {
                position: 'top' as const,
            },
            title: {
                display: true,
                text: 'Performance Over Time'
            }
        },
        scales: {
            y: {
                beginAtZero: true
            }
        }
    };
    
    return <Line data={chartData} options={options} />;
}
```

---

## Webhook Integration

### Tutorial 5: Event Notifications

#### Step 1: Webhook Server

**`webhooks/server.js`**:
```javascript
const express = require('express');
const axios = require('axios');

const app = express();
app.use(express.json());

// Store webhook subscriptions
const subscriptions = new Map();

// Register webhook
app.post('/webhooks/register', (req, res) => {
    const { url, events, filter } = req.body;
    
    const id = generateId();
    subscriptions.set(id, { url, events, filter });
    
    res.json({ subscription_id: id });
});

// Unregister webhook
app.delete('/webhooks/:id', (req, res) => {
    subscriptions.delete(req.params.id);
    res.json({ success: true });
});

// Event listener (called by indexer)
async function notifyWebhooks(event) {
    for (const [id, sub] of subscriptions) {
        if (sub.events.includes(event.type)) {
            if (matchesFilter(event, sub.filter)) {
                await sendWebhook(sub.url, event);
            }
        }
    }
}

async function sendWebhook(url, event) {
    try {
        await axios.post(url, {
            event_type: event.type,
            data: event.data,
            timestamp: Date.now()
        });
    } catch (error) {
        console.error(`Webhook failed: ${url}`, error);
    }
}

function matchesFilter(event, filter) {
    if (!filter) return true;
    
    // Check if event matches filter criteria
    for (const [key, value] of Object.entries(filter)) {
        if (event.data[key] !== value) {
            return false;
        }
    }
    
    return true;
}

app.listen(4000, () => {
    console.log('Webhook server running on port 4000');
});
```

#### Step 2: Client Usage

**`client_example.js`**:
```javascript
const axios = require('axios');

// Register webhook
async function registerWebhook() {
    const response = await axios.post('http://localhost:4000/webhooks/register', {
        url: 'https://myapp.com/webhook',
        events: ['signal_created', 'signal_completed'],
        filter: {
            provider: 'GXXX...'
        }
    });
    
    console.log('Webhook registered:', response.data.subscription_id);
    return response.data.subscription_id;
}

// Handle webhook
app.post('/webhook', (req, res) => {
    const { event_type, data, timestamp } = req.body;
    
    console.log('Received event:', event_type);
    
    switch (event_type) {
        case 'signal_created':
            handleSignalCreated(data);
            break;
        case 'signal_completed':
            handleSignalCompleted(data);
            break;
    }
    
    res.json({ received: true });
});

function handleSignalCreated(data) {
    console.log('New signal:', data.signal_id);
    // Process new signal
}

function handleSignalCompleted(data) {
    console.log('Signal completed:', data.signal_id, 'PnL:', data.pnl);
    // Update analytics
}
```

---

## Best Practices

### Error Handling

```typescript
async function safeContractCall<T>(
    fn: () => Promise<T>
): Promise<T | null> {
    try {
        return await fn();
    } catch (error) {
        if (error.code === 'TIMEOUT') {
            // Retry logic
            return await retryWithBackoff(fn);
        }
        console.error('Contract call failed:', error);
        return null;
    }
}
```

### Rate Limiting

```typescript
class RateLimiter {
    private requests: number[] = [];
    private limit: number;
    private window: number;
    
    constructor(limit: number, windowMs: number) {
        this.limit = limit;
        this.window = windowMs;
    }
    
    async acquire(): Promise<void> {
        const now = Date.now();
        this.requests = this.requests.filter(t => t > now - this.window);
        
        if (this.requests.length >= this.limit) {
            const oldestRequest = this.requests[0];
            const waitTime = this.window - (now - oldestRequest);
            await new Promise(resolve => setTimeout(resolve, waitTime));
        }
        
        this.requests.push(now);
    }
}
```

### Connection Management

```typescript
class ConnectionManager {
    private server: SorobanRpc.Server;
    private reconnectAttempts = 0;
    private maxReconnectAttempts = 5;
    
    async connect() {
        try {
            this.server = new SorobanRpc.Server(RPC_URL);
            await this.server.getHealth();
            this.reconnectAttempts = 0;
        } catch (error) {
            if (this.reconnectAttempts < this.maxReconnectAttempts) {
                this.reconnectAttempts++;
                await this.reconnect();
            } else {
                throw new Error('Max reconnection attempts reached');
            }
        }
    }
    
    private async reconnect() {
        const delay = Math.pow(2, this.reconnectAttempts) * 1000;
        await new Promise(resolve => setTimeout(resolve, delay));
        await this.connect();
    }
}
```

---

## Next Steps

1. **Explore Examples**: Check `/examples` directory for more code samples
2. **Read API Docs**: Review complete API reference
3. **Join Community**: Connect with other developers
4. **Build & Share**: Create your integration and share with community

---

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01  
**Maintained By**: StellarSwipe Core Team
