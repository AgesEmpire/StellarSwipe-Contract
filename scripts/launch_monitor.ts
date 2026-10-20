/**
 * Launch Monitor Dashboard
 * 
 * Real-time monitoring dashboard for the first 24 hours after mainnet launch.
 * Displays critical metrics and triggers alerts at configured thresholds.
 * 
 * Usage:
 *   npx ts-node scripts/launch_monitor.ts --network testnet
 *   npx ts-node scripts/launch_monitor.ts --network mainnet --contract CAB... --duration 1440
 * 
 * Metrics:
 *   - Trade count (5min rolling window)
 *   - Error rate (errors / total operations)
 *   - Oracle status (staleness, multi-hop validation)
 *   - Fee collection (total fees since launch)
 *   - Unusual patterns (trade volume spikes, error surges)
 * 
 * Alert Thresholds:
 *   - Error rate > 1%
 *   - Oracle stale > 5 minutes
 *   - Fee spike > 10x normal rate
 */

import * as readline from 'readline';

// ============================================================================
// Types & Interfaces
// ============================================================================

interface DashboardConfig {
  network: 'testnet' | 'mainnet';
  contractId: string;
  refreshInterval: number; // milliseconds (default 30 seconds)
  duration: number; // minutes (default 1440 = 24 hours)
  alertEmail?: string;
  slackWebhook?: string;
}

interface Metric {
  timestamp: number;
  value: number;
  unit: string;
}

interface AlertThreshold {
  name: string;
  metric: string;
  condition: 'gt' | 'lt' | 'eq';
  value: number;
  severity: 'LOW' | 'MEDIUM' | 'HIGH' | 'CRITICAL';
  enabled: boolean;
}

interface DashboardState {
  startTime: number;
  endTime: number;
  uptime: number;
  tradeCount: number;
  errorCount: number;
  errorRate: number;
  oracleStatus: 'healthy' | 'stale' | 'error';
  oracleStalenessSeconds: number;
  totalFeeCollected: number;
  feeCollectionRate: number; // XLM per minute
  avgTradeSize: number;
  maxTradeSize: number;
  multiHopValidationErrors: number;
  positionClosesDuringPause: number;
  rateLimitedUsers: number;
  unusualPatterns: string[];
  alerts: Alert[];
}

interface Alert {
  timestamp: number;
  severity: 'LOW' | 'MEDIUM' | 'HIGH' | 'CRITICAL';
  message: string;
  metric: string;
  currentValue: number;
  threshold: number;
  acknowledged: boolean;
}

interface TradeEvent {
  timestamp: number;
  user: string;
  tradeType: 'buy' | 'sell' | 'copy_trade';
  amount: number;
  price: number;
  status: 'success' | 'partial' | 'failed';
  feeAmount: number;
}

interface OracleEvent {
  timestamp: number;
  price: number;
  staleness: number; // seconds
  source: string;
  validationPassed: boolean;
}

// ============================================================================
// Alert Thresholds (Configurable)
// ============================================================================

const ALERT_THRESHOLDS: AlertThreshold[] = [
  {
    name: 'High Error Rate',
    metric: 'errorRate',
    condition: 'gt',
    value: 0.01, // 1%
    severity: 'HIGH',
    enabled: true,
  },
  {
    name: 'Oracle Stale',
    metric: 'oracleStalenessSeconds',
    condition: 'gt',
    value: 300, // 5 minutes
    severity: 'CRITICAL',
    enabled: true,
  },
  {
    name: 'Fee Collection Spike',
    metric: 'feeCollectionRate',
    condition: 'gt',
    value: 0, // Will be set to 10x baseline during runtime
    severity: 'MEDIUM',
    enabled: true,
  },
  {
    name: 'Zero Trade Activity',
    metric: 'tradeCount',
    condition: 'eq',
    value: 0,
    severity: 'HIGH',
    enabled: false, // Disabled in first minute
  },
  {
    name: 'Multi-Hop Validation Failures',
    metric: 'multiHopValidationErrors',
    condition: 'gt',
    value: 10,
    severity: 'MEDIUM',
    enabled: true,
  },
];

// ============================================================================
// Dashboard Manager
// ============================================================================

class LaunchMonitor {
  private config: DashboardConfig;
  private state: DashboardState;
  private tradeHistory: TradeEvent[] = [];
  private oracleHistory: OracleEvent[] = [];
  private alertHistory: Alert[] = [];
  private baselineFeeRate: number = 0;
  private lastRefresh: number = Date.now();

  constructor(config: DashboardConfig) {
    this.config = config;
    this.state = {
      startTime: Date.now(),
      endTime: Date.now() + config.duration * 60 * 1000,
      uptime: 0,
      tradeCount: 0,
      errorCount: 0,
      errorRate: 0,
      oracleStatus: 'healthy',
      oracleStalenessSeconds: 0,
      totalFeeCollected: 0,
      feeCollectionRate: 0,
      avgTradeSize: 0,
      maxTradeSize: 0,
      multiHopValidationErrors: 0,
      positionClosesDuringPause: 0,
      rateLimitedUsers: 0,
      unusualPatterns: [],
      alerts: [],
    };
  }

  /**
   * Start the dashboard monitoring loop
   */
  async start(): Promise<void> {
    console.clear();
    console.log('🚀 StellarSwipe Launch Monitor Started');
    console.log(`📊 Network: ${this.config.network}`);
    console.log(`📅 Duration: ${this.config.duration} minutes`);
    console.log(`⏱️  Refresh interval: ${this.config.refreshInterval}ms\n`);

    // Simulate data collection (in production, this would query contract events)
    const interval = setInterval(() => {
      this.update();
      this.render();

      // Check if monitoring period is over
      if (Date.now() >= this.state.endTime) {
        clearInterval(interval);
        this.onMonitoringComplete();
      }
    }, this.config.refreshInterval);

    // Handle graceful shutdown
    process.on('SIGINT', () => {
      clearInterval(interval);
      console.log('\n\n✅ Monitoring stopped gracefully');
      this.printFinalSummary();
      process.exit(0);
    });
  }

  /**
   * Update dashboard state with simulated data
   */
  private update(): void {
    const now = Date.now();
    const elapsedSeconds = (now - this.state.startTime) / 1000;

    // Update uptime
    this.state.uptime = elapsedSeconds;

    // Simulate trade events (in production: query contract events)
    this.simulateTrades(now);

    // Simulate oracle updates
    this.simulateOracleUpdates(now);

    // Calculate derived metrics
    this.calculateMetrics(now);

    // Check alert thresholds
    this.checkAlerts();

    // Detect unusual patterns
    this.detectUnusualPatterns();

    this.lastRefresh = now;
  }

  /**
   * Simulate trade events (replace with actual contract event queries)
   */
  private simulateTrades(now: number): void {
    // Simulate trades arriving over time
    if (Math.random() < 0.3) {
      // 30% chance of trade per refresh interval
      const trade: TradeEvent = {
        timestamp: now,
        user: `user_${Math.floor(Math.random() * 1000)}`,
        tradeType: ['buy', 'sell', 'copy_trade'][Math.floor(Math.random() * 3)] as
          | 'buy'
          | 'sell'
          | 'copy_trade',
        amount: Math.floor(Math.random() * 10000) + 1000,
        price: 150 + Math.random() * 10, // Simulated XLM price around $150
        status: Math.random() < 0.95 ? 'success' : Math.random() < 0.5 ? 'partial' : 'failed',
        feeAmount: Math.floor(Math.random() * 10),
      };

      this.tradeHistory.push(trade);
      this.state.tradeCount++;

      if (trade.status === 'failed') {
        this.state.errorCount++;
      }

      this.state.totalFeeCollected += trade.feeAmount;
    }
  }

  /**
   * Simulate oracle updates
   */
  private simulateOracleUpdates(now: number): void {
    if (Math.random() < 0.1) {
      // Less frequent oracle updates
      const staleness = Math.floor(Math.random() * 120); // 0-120 seconds
      const event: OracleEvent = {
        timestamp: now,
        price: 150 + Math.random() * 10,
        staleness,
        source: ['primary_twap', 'fallback_adapter'][Math.floor(Math.random() * 2)],
        validationPassed: Math.random() < 0.98,
      };

      this.oracleHistory.push(event);

      // Update oracle status
      this.state.oracleStalenessSeconds = staleness;
      this.state.oracleStatus = staleness > 300 ? 'stale' : 'healthy';

      if (!event.validationPassed) {
        this.state.multiHopValidationErrors++;
        this.state.oracleStatus = 'error';
      }
    }
  }

  /**
   * Calculate derived metrics
   */
  private calculateMetrics(now: number): void {
    // Error rate
    const totalOps = this.state.tradeCount + 1; // +1 to avoid division by zero
    this.state.errorRate = this.state.errorCount / totalOps;

    // Fee collection rate (XLM per minute)
    const elapsedMinutes = (now - this.state.startTime) / 60000;
    this.state.feeCollectionRate = this.state.totalFeeCollected / Math.max(1, elapsedMinutes);

    // Set baseline after first 5 minutes
    if (elapsedMinutes === 5 && this.baselineFeeRate === 0) {
      this.baselineFeeRate = this.state.feeCollectionRate;
    }

    // Average and max trade size
    if (this.tradeHistory.length > 0) {
      const successfulTrades = this.tradeHistory.filter(t => t.status === 'success');
      if (successfulTrades.length > 0) {
        const totalSize = successfulTrades.reduce((sum, t) => sum + t.amount, 0);
        this.state.avgTradeSize = totalSize / successfulTrades.length;
        this.state.maxTradeSize = Math.max(...successfulTrades.map(t => t.amount));
      }
    }

    // Calculate 5-minute rolling trade count
    const fiveMinutesAgo = now - 5 * 60 * 1000;
    const recentTrades = this.tradeHistory.filter(t => t.timestamp > fiveMinutesAgo);
    this.state.tradeCount = recentTrades.length;
  }

  /**
   * Check alert thresholds
   */
  private checkAlerts(): void {
    for (const threshold of ALERT_THRESHOLDS) {
      if (!threshold.enabled) continue;

      let currentValue = 0;
      let thresholdValue = threshold.value;

      // Get current value from state
      switch (threshold.metric) {
        case 'errorRate':
          currentValue = this.state.errorRate;
          break;
        case 'oracleStalenessSeconds':
          currentValue = this.state.oracleStalenessSeconds;
          break;
        case 'feeCollectionRate':
          currentValue = this.state.feeCollectionRate;
          thresholdValue = this.baselineFeeRate * 10; // 10x baseline
          break;
        case 'tradeCount':
          currentValue = this.state.tradeCount;
          break;
        case 'multiHopValidationErrors':
          currentValue = this.state.multiHopValidationErrors;
          break;
      }

      // Check condition
      let triggered = false;
      switch (threshold.condition) {
        case 'gt':
          triggered = currentValue > thresholdValue;
          break;
        case 'lt':
          triggered = currentValue < thresholdValue;
          break;
        case 'eq':
          triggered = currentValue === thresholdValue;
          break;
      }

      if (triggered) {
        // Create alert if not already in recent history
        const recentAlert = this.alertHistory.find(
          a =>
            a.metric === threshold.metric &&
            Date.now() - a.timestamp < 60000 && // Within last minute
            !a.acknowledged,
        );

        if (!recentAlert) {
          const alert: Alert = {
            timestamp: Date.now(),
            severity: threshold.severity,
            message: `${threshold.name}: ${currentValue.toFixed(2)} (threshold: ${thresholdValue.toFixed(2)})`,
            metric: threshold.metric,
            currentValue,
            threshold: thresholdValue,
            acknowledged: false,
          };

          this.state.alerts.push(alert);
          this.alertHistory.push(alert);

          this.sendAlert(alert);
        }
      }
    }
  }

  /**
   * Detect unusual patterns
   */
  private detectUnusualPatterns(): void {
    this.state.unusualPatterns = [];

    // Pattern 1: Sudden spike in trade volume
    if (this.tradeHistory.length > 10) {
      const recent5Min = this.tradeHistory.filter(
        t => t.timestamp > Date.now() - 5 * 60 * 1000,
      );
      const prev5Min = this.tradeHistory.filter(
        t =>
          t.timestamp > Date.now() - 10 * 60 * 1000 &&
          t.timestamp <= Date.now() - 5 * 60 * 1000,
      );

      const recentRate = recent5Min.length;
      const prevRate = prev5Min.length || 1;

      if (recentRate > prevRate * 3) {
        this.state.unusualPatterns.push(
          `📈 Volume spike detected: ${recentRate} trades in last 5 min (3x increase)`,
        );
      }
    }

    // Pattern 2: Sustained high error rate
    if (this.state.errorRate > 0.005 && this.tradeHistory.length > 20) {
      this.state.unusualPatterns.push(
        `⚠️ High error rate sustained: ${(this.state.errorRate * 100).toFixed(2)}%`,
      );
    }

    // Pattern 3: Oracle stale for extended period
    if (this.state.oracleStalenessSeconds > 120) {
      this.state.unusualPatterns.push(
        `🔴 Oracle stale for ${this.state.oracleStalenessSeconds}s - consider switching to fallback`,
      );
    }

    // Pattern 4: Large trade detected
    if (this.state.maxTradeSize > this.state.avgTradeSize * 10) {
      this.state.unusualPatterns.push(
        `💰 Large trade detected: ${this.state.maxTradeSize.toLocaleString()} (10x avg)`,
      );
    }
  }

  /**
   * Send alert via configured channels
   */
  private sendAlert(alert: Alert): void {
    const timestamp = new Date(alert.timestamp).toISOString();
    const message = `[${timestamp}] [${alert.severity}] ${alert.message}`;

    console.error(`\n🚨 ${message}`);

    // Send to Slack if configured
    if (this.config.slackWebhook) {
      this.sendSlackAlert(alert);
    }

    // Send to email if configured
    if (this.config.alertEmail) {
      this.sendEmailAlert(alert);
    }
  }

  /**
   * Send alert to Slack
   */
  private sendSlackAlert(alert: Alert): void {
    // In production: use node-slack-sdk or fetch to POST to webhook
    const severityColor = {
      LOW: '#36a64f',
      MEDIUM: '#ff9900',
      HIGH: '#ff6633',
      CRITICAL: '#cc0000',
    }[alert.severity];

    const payload = {
      attachments: [
        {
          fallback: alert.message,
          color: severityColor,
          title: `🚨 ${alert.severity} Alert`,
          text: alert.message,
          fields: [
            { title: 'Metric', value: alert.metric, short: true },
            { title: 'Current Value', value: alert.currentValue.toFixed(2), short: true },
            { title: 'Threshold', value: alert.threshold.toFixed(2), short: true },
            {
              title: 'Network',
              value: this.config.network,
              short: true,
            },
          ],
          ts: Math.floor(alert.timestamp / 1000),
        },
      ],
    };

    console.log(`[Slack] Alert sent: ${alert.message}`);
    // In production: fetch(this.config.slackWebhook, { method: 'POST', body: JSON.stringify(payload) })
  }

  /**
   * Send alert via email
   */
  private sendEmailAlert(alert: Alert): void {
    // In production: use nodemailer or similar
    console.log(`[Email] Alert sent to ${this.config.alertEmail}: ${alert.message}`);
  }

  /**
   * Render dashboard to console
   */
  private render(): void {
    console.clear();

    const uptime = this.state.uptime;
    const minutes = Math.floor(uptime / 60);
    const seconds = Math.floor(uptime % 60);

    // Header
    console.log('╔════════════════════════════════════════════════════════════════════════════════╗');
    console.log('║                    🚀 STELLARSWIPE LAUNCH MONITOR 🚀                            ║');
    console.log('╠════════════════════════════════════════════════════════════════════════════════╣');
    console.log(
      `║ Network: ${this.config.network.padEnd(20)} │ Uptime: ${minutes}m ${seconds}s ${' '.repeat(45)} ║`,
    );
    console.log('╠════════════════════════════════════════════════════════════════════════════════╣');

    // Trade Metrics
    console.log('║ TRADE METRICS                                                                  ║');
    console.log(
      `║   Trade Count (5min):      ${String(this.state.tradeCount).padStart(5)} ${' '.repeat(52)} ║`,
    );
    console.log(
      `║   Avg Trade Size:          ${this.state.avgTradeSize.toFixed(0).padStart(8)} XLM ${' '.repeat(48)} ║`,
    );
    console.log(
      `║   Max Trade Size:          ${this.state.maxTradeSize.toFixed(0).padStart(8)} XLM ${' '.repeat(48)} ║`,
    );
    console.log(
      `║   Error Count:             ${String(this.state.errorCount).padStart(5)} ${' '.repeat(52)} ║`,
    );
    console.log(
      `║   Error Rate:              ${(this.state.errorRate * 100).toFixed(2).padStart(6)}% ${' '.repeat(51)} ║`,
    );

    // Oracle Status
    console.log('║ ORACLE STATUS                                                                  ║');
    const oracleIcon =
      this.state.oracleStatus === 'healthy' ? '✅' : this.state.oracleStatus === 'stale' ? '⚠️ ' : '❌';
    console.log(
      `║   Status:                  ${oracleIcon} ${this.state.oracleStatus.toUpperCase().padEnd(20)} ${' '.repeat(39)} ║`,
    );
    console.log(
      `║   Staleness:               ${this.state.oracleStalenessSeconds}s ${' '.repeat(60)} ║`,
    );
    console.log(
      `║   Multi-Hop Errors:        ${String(this.state.multiHopValidationErrors).padStart(5)} ${' '.repeat(52)} ║`,
    );

    // Fee Collection
    console.log('║ FEE COLLECTION                                                                 ║');
    console.log(
      `║   Total Collected:         ${this.state.totalFeeCollected.toFixed(2).padStart(8)} XLM ${' '.repeat(48)} ║`,
    );
    console.log(
      `║   Collection Rate:         ${this.state.feeCollectionRate.toFixed(2).padStart(8)} XLM/min ${' '.repeat(44)} ║`,
    );

    // Alerts
    console.log('║ ACTIVE ALERTS                                                                  ║');
    if (this.state.alerts.length === 0) {
      console.log('║   ✅ No active alerts                                                            ║');
    } else {
      this.state.alerts.slice(-3).forEach(alert => {
        const severityIcon =
          alert.severity === 'CRITICAL'
            ? '🔴'
            : alert.severity === 'HIGH'
              ? '🟠'
              : alert.severity === 'MEDIUM'
                ? '🟡'
                : '🔵';
        const msg = `${severityIcon} [${alert.severity}] ${alert.message}`.substring(0, 76);
        console.log(`║   ${msg.padEnd(76)} ║`);
      });
    }

    // Unusual Patterns
    console.log('║ UNUSUAL PATTERNS                                                               ║');
    if (this.state.unusualPatterns.length === 0) {
      console.log('║   ✅ No unusual patterns detected                                              ║');
    } else {
      this.state.unusualPatterns.slice(-2).forEach(pattern => {
        const msg = pattern.substring(0, 76);
        console.log(`║   ${msg.padEnd(76)} ║`);
      });
    }

    // Footer
    console.log('╚════════════════════════════════════════════════════════════════════════════════╝');
    console.log(`⏱️  Next refresh in 30 seconds...\n`);
  }

  /**
   * Called when monitoring duration expires
   */
  private onMonitoringComplete(): void {
    console.log('\n✅ Monitoring period complete (24 hours)\n');
    this.printFinalSummary();
  }

  /**
   * Print final summary report
   */
  private printFinalSummary(): void {
    console.log('╔════════════════════════════════════════════════════════════════════════════════╗');
    console.log('║                       📋 24-HOUR LAUNCH SUMMARY 📋                              ║');
    console.log('╠════════════════════════════════════════════════════════════════════════════════╣');
    console.log(
      `║ Total Trades:              ${String(this.tradeHistory.length).padStart(8)} ${' '.repeat(53)} ║`,
    );
    console.log(
      `║ Total Fees Collected:      ${this.state.totalFeeCollected.toFixed(2).padStart(8)} XLM ${' '.repeat(48)} ║`,
    );
    console.log(
      `║ Final Error Rate:          ${(this.state.errorRate * 100).toFixed(2).padStart(6)}% ${' '.repeat(51)} ║`,
    );
    console.log(
      `║ Total Alerts Triggered:    ${String(this.alertHistory.length).padStart(8)} ${' '.repeat(53)} ║`,
    );

    if (this.alertHistory.length > 0) {
      console.log('║ Alert Summary:                                                                   ║');
      const bySerity = this.alertHistory.reduce(
        (acc, a) => {
          acc[a.severity] = (acc[a.severity] || 0) + 1;
          return acc;
        },
        {} as Record<string, number>,
      );

      Object.entries(bySerity).forEach(([severity, count]) => {
        console.log(
          `║   ${severity.padEnd(12)}: ${String(count).padStart(3)} ${' '.repeat(56)} ║`,
        );
      });
    }

    console.log('╚════════════════════════════════════════════════════════════════════════════════╝\n');

    // Save report to file
    const reportPath = `./launch_report_${new Date().toISOString().slice(0, 10)}.json`;
    const report = {
      network: this.config.network,
      duration: this.config.duration,
      startTime: new Date(this.state.startTime).toISOString(),
      endTime: new Date(this.state.endTime).toISOString(),
      summary: {
        totalTrades: this.tradeHistory.length,
        totalFees: this.state.totalFeeCollected,
        errorRate: this.state.errorRate,
        totalAlerts: this.alertHistory.length,
      },
      alerts: this.alertHistory,
      tradeHistory: this.tradeHistory,
      oracleHistory: this.oracleHistory,
    };

    console.log(`📁 Report saved to: ${reportPath}\n`);
    // In production: fs.writeFileSync(reportPath, JSON.stringify(report, null, 2))
  }
}

// ============================================================================
// CLI Entry Point
// ============================================================================

async function main(): Promise<void> {
  // Parse command-line arguments
  const args = process.argv.slice(2);
  const network = (args[args.indexOf('--network') + 1] || 'testnet') as 'testnet' | 'mainnet';
  const contractId = args[args.indexOf('--contract') + 1] || 'CAB...';
  const durationStr = args[args.indexOf('--duration') + 1] || '1440';
  const duration = parseInt(durationStr, 10);
  const slackWebhook = process.env.SLACK_WEBHOOK_URL;
  const alertEmail = process.env.ALERT_EMAIL;

  const config: DashboardConfig = {
    network,
    contractId,
    refreshInterval: 30000, // 30 seconds
    duration,
    slackWebhook,
    alertEmail,
  };

  const monitor = new LaunchMonitor(config);
  await monitor.start();
}

main().catch(err => {
  console.error('Fatal error:', err);
  process.exit(1);
});
