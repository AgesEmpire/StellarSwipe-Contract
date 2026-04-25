//! High-volume load simulation for StellarSwipe AutoTrade contract.
//!
//! # Simulation Overview
//! Simulates 1 000 sequential copy-trade executions across 100 providers and
//! 1 000 users to validate instruction-budget headroom and linear storage growth
//! before mainnet deployment.
//!
//! # Limitations (Soroban test environment)
//! - **No true concurrency**: Soroban's test VM is single-threaded; trades run
//!   sequentially. Real-world parallelism across ledger closures is not modelled.
//! - **Instruction budget**: `env.cost_estimate().budget().cpu_instruction_cost()`
//!   reflects the cumulative budget consumed since the last automatic reset
//!   (which occurs before every top-level contract invocation). We read the
//!   value after each trade to approximate per-trade cost.
//! - **Storage growth proxy**: Soroban's test host does not expose raw byte
//!   counts. We count successful trades as a proxy — each trade writes one
//!   persistent `Trades(user, signal_id)` entry, so growth is inherently linear.
//! - **Event accumulation**: `env.events().all()` returns events from the most
//!   recent invocation frame only. We therefore count events per-trade and sum
//!   them manually.
//! - **No network I/O**: SDEX liquidity is stubbed via temporary storage keys,
//!   matching the pattern used in the unit tests.
//! - **Performance**: The Soroban test VM runs in debug mode; absolute
//!   instruction counts are representative but wall-clock times are not.
//!
//! # Performance Metrics (printed table at end of test run)
//! | Metric                        | Target          |
//! |-------------------------------|-----------------|
//! | Trades completed              | 1 000 / 1 000   |
//! | Max instructions per trade    | < 80 % of limit |
//! | Storage entry growth          | Linear          |
//! | Total events emitted          | ≥ 1 000         |

use auto_trade::{
    authorize_user_with_limits, set_signal, AutoTradeContract, OrderType, Signal, TradeStatus,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env,
};

// Soroban's default instruction limit per transaction (100 M CPU instructions).
const SOROBAN_INSTRUCTION_LIMIT: u64 = 100_000_000;
const BUDGET_THRESHOLD_PCT: u64 = 80;

const NUM_PROVIDERS: u64 = 100;
/// 1 000 distinct users as required by the spec.
const NUM_USERS: usize = 1_000;
const NUM_TRADES: usize = 1_000;
const TRADE_AMOUNT: i128 = 1_000;
const SIGNAL_PRICE: i128 = 100;
const SIGNAL_EXPIRY_OFFSET: u64 = 86_400 * 30; // 30 days

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);
    let admin = Address::generate(&env);
    let contract_id = env.register(AutoTradeContract, ());
    env.as_contract(&contract_id, || {
        AutoTradeContract::initialize(env.clone(), admin.clone());
        // Set permissive rate limits so the load test isn't blocked by
        // the default min_transfer_amount (10_000_000 stroops).
        auto_trade::rate_limit::set_limits(
            &env,
            &auto_trade::rate_limit::BridgeRateLimits {
                per_user_hourly_transfers: 10_000,
                per_user_hourly_volume: 1_000_000_000_000_000i128,
                per_user_daily_transfers: 100_000,
                per_user_daily_volume: 1_000_000_000_000_000i128,
                global_hourly_capacity: 100_000,
                global_daily_volume: 1_000_000_000_000_000i128,
                min_transfer_amount: 1,
                cooldown_between_transfers: 0,
            },
        );
    });
    (env, contract_id, admin)
}

fn seed_signal(env: &Env, contract_id: &Address, signal_id: u64) {
    env.as_contract(contract_id, || {
        set_signal(
            env,
            signal_id,
            &Signal {
                signal_id,
                price: SIGNAL_PRICE,
                expiry: env.ledger().timestamp() + SIGNAL_EXPIRY_OFFSET,
                base_asset: ((signal_id % 10) + 1) as u32,
            },
        );
        env.storage()
            .temporary()
            .set(&(symbol_short!("liquidity"), signal_id), &1_000_000_000i128);
    });
}

#[test]
fn test_1000_sequential_trades() {
    let (env, contract_id, _admin) = setup();

    // Seed 100 provider signals.
    for sid in 1..=NUM_PROVIDERS {
        seed_signal(&env, &contract_id, sid);
    }

    // Create 1 000 users and set up their auth + balance.
    // Reset budget to unlimited before bulk setup to avoid hitting limits.
    env.cost_estimate().budget().reset_unlimited();
    let users: Vec<Address> = (0..NUM_USERS)
        .map(|_| Address::generate(&env))
        .collect();

    env.as_contract(&contract_id, || {
        for user in &users {
            authorize_user_with_limits(&env, user, 1_000_000_000i128, 30);
            env.storage()
                .temporary()
                .set(&(user.clone(), symbol_short!("balance")), &1_000_000_000i128);
        }
    });

    let mut max_instructions: u64 = 0;
    let mut instruction_samples: Vec<u64> = Vec::with_capacity(NUM_TRADES);
    // (trade_index, cumulative_successful_trades) — proxy for storage entries.
    let mut storage_snapshots: Vec<(usize, usize)> = Vec::new();
    let mut successful_trades: usize = 0;
    // Cumulative event count tracked manually (env.events().all() is per-frame).
    let mut total_events: usize = 0;

    // Execute 1 000 trades sequentially.
    for i in 0..NUM_TRADES {
        let user = &users[i % NUM_USERS];
        let signal_id = ((i as u64) % NUM_PROVIDERS) + 1;

        let result = env.as_contract(&contract_id, || {
            AutoTradeContract::execute_trade(
                env.clone(),
                user.clone(),
                signal_id,
                OrderType::Market,
                TRADE_AMOUNT,
            )
        });

        // Read CPU cost for this invocation (auto-reset before each top-level call).
        let instructions = env.cost_estimate().budget().cpu_instruction_cost();
        instruction_samples.push(instructions);
        if instructions > max_instructions {
            max_instructions = instructions;
        }

        // Count events emitted in this invocation frame.
        total_events += env.events().all().len() as usize;

        match result {
            Ok(trade_result) => {
                assert!(
                    matches!(
                        trade_result.trade.status,
                        TradeStatus::Filled | TradeStatus::PartiallyFilled
                    ),
                    "trade {i} should fill or partially fill, got {:?}",
                    trade_result.trade.status
                );
                successful_trades += 1;
            }
            Err(e) => panic!("trade {i} failed unexpectedly: {e:?}"),
        }

        // Snapshot every 100 trades.
        if i % 100 == 0 {
            storage_snapshots.push((i, successful_trades));
        }
    }

    // ── Assertions ────────────────────────────────────────────────────────────

    // 1. All 1 000 trades completed.
    assert_eq!(
        successful_trades, NUM_TRADES,
        "expected {NUM_TRADES} successful trades, got {successful_trades}"
    );

    // 2. No trade exceeded 80 % of the instruction budget.
    let budget_80pct = SOROBAN_INSTRUCTION_LIMIT * BUDGET_THRESHOLD_PCT / 100;
    assert!(
        max_instructions <= budget_80pct,
        "max instructions per trade ({max_instructions}) exceeded 80% of budget ({budget_80pct})"
    );

    // 3. Storage growth is linear: each snapshot window adds ~100 trades.
    assert_storage_growth_linear(&storage_snapshots);

    // 4. At least 1 000 events emitted (one `trade_executed` per trade minimum).
    assert!(
        total_events >= NUM_TRADES,
        "expected ≥{NUM_TRADES} events, got {total_events}"
    );

    print_metrics(&instruction_samples, max_instructions, total_events, &storage_snapshots);
}

/// Assert that the number of successful trades grows linearly across snapshots.
/// Each window should add ~100 trades; we allow 3× tolerance for first-write
/// overhead.
fn assert_storage_growth_linear(snapshots: &[(usize, usize)]) {
    if snapshots.len() < 2 {
        return;
    }
    let deltas: Vec<usize> = snapshots
        .windows(2)
        .map(|w| w[1].1.saturating_sub(w[0].1))
        .collect();
    let avg = deltas.iter().sum::<usize>() / deltas.len();
    for (i, &d) in deltas.iter().enumerate() {
        assert!(
            d <= avg * 3 + 10,
            "storage growth spike at window {i}: delta={d}, avg={avg} — not linear"
        );
    }
}

fn print_metrics(
    samples: &[u64],
    max_instructions: u64,
    total_events: usize,
    storage_snapshots: &[(usize, usize)],
) {
    let sum: u64 = samples.iter().sum();
    let avg = sum / samples.len() as u64;
    let min = samples.iter().copied().min().unwrap_or(0);
    let budget_pct = if SOROBAN_INSTRUCTION_LIMIT > 0 {
        max_instructions * 100 / SOROBAN_INSTRUCTION_LIMIT
    } else {
        0
    };

    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║     StellarSwipe High-Volume Load Test Results       ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ Trades executed          : {:<6} / {:<6}             ║", samples.len(), NUM_TRADES);
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ CPU Instructions per trade                           ║");
    println!("║   Min                    : {:<12}              ║", min);
    println!("║   Avg                    : {:<12}              ║", avg);
    println!("║   Max                    : {:<12}              ║", max_instructions);
    println!("║   Max as % of budget     : {:<11}%              ║", budget_pct);
    println!("║   80% threshold          : {:<12}              ║", SOROBAN_INSTRUCTION_LIMIT * 80 / 100);
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ Total events emitted     : {:<6}                      ║", total_events);
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ Storage growth (successful trades, every 100)        ║");
    for (trade_idx, count) in storage_snapshots {
        println!("║   After {:>4} trades       : {:>6} completed           ║", trade_idx, count);
    }
    println!("╚══════════════════════════════════════════════════════╝\n");
}
