//! Batch settlement and partial-fill accounting (Issue: batch settlement).
//!
//! Implements deterministic settlement semantics for auto-trade flows where a
//! single operation is partially filled or settled in multiple batches.  Each
//! settlement record tracks progress accurately so downstream reconciliation
//! never encounters accounting gaps.
//!
//! # Concepts
//! - **Settlement order**: the top-level request for a specific amount.
//! - **Fill**: a partial or full execution against that order.
//! - **Settled amount**: cumulative amount filled so far.
//! - **Remaining amount**: `requested - settled`.
//! - **Status**: `Open` → `PartiallyFilled` → `FullySettled` | `Failed`.
//!
//! # Slippage bounds (Issue #991)
//! Every order carries caller-supplied [`SlippageBounds`]: a `min_output` floor
//! and a `max_input` cap. [`settle_batch`] enforces both against the *actual*
//! filled amounts reported after external execution (the output actually
//! received and the input actually consumed) **before** any state is mutated,
//! so an adverse price move cannot silently complete a settlement outside the
//! caller's intent — a violation returns [`SettlementError::SlippageBoundsViolated`]
//! and leaves the order, its fill records, and its events untouched.
//!
//! The module is intentionally free of cross-contract calls so it can be unit-
//! tested in isolation and reused by any contract that needs settlement logic.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Lifecycle status of a settlement order.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SettlementStatus {
    /// No fills yet; full amount is still outstanding.
    Open = 0,
    /// One or more fills recorded; some amount remains.
    PartiallyFilled = 1,
    /// All requested amount has been filled.
    FullySettled = 2,
    /// The order was closed with a failure before full settlement.
    Failed = 3,
}

/// A single fill event recorded against a settlement order.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FillRecord {
    /// Sequence number of this fill within the order (1-based).
    pub fill_index: u32,
    /// Amount filled in this individual fill.
    pub filled_amount: i128,
    /// Execution price for this fill (7-decimal fixed-point).
    pub execution_price: i128,
    /// Ledger sequence when this fill was recorded.
    pub ledger: u32,
    /// Whether this fill completed the order.
    pub is_final: bool,
}

/// Caller-supplied slippage bounds enforced on every fill of a settlement order
/// (Issue #991).
///
/// Both fields are non-negative:
/// - `min_output`: a fill is rejected unless the actual output filled is
///   `>= min_output`. `0` imposes no floor (a zero-output fill is accepted).
/// - `max_input`: a fill is rejected unless the actual input consumed is
///   `<= max_input`. Pass `i128::MAX` for no practical cap.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlippageBounds {
    /// Minimum acceptable output amount for a fill (0 = no floor).
    pub min_output: i128,
    /// Maximum acceptable input amount a fill may consume (`i128::MAX` = no cap).
    pub max_input: i128,
}

/// The full settlement state for one order.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementOrder {
    /// Unique monotonic order ID.
    pub order_id: u64,
    /// Owner of the order.
    pub user: Address,
    /// Total amount originally requested.
    pub requested_amount: i128,
    /// Cumulative amount filled across all fills so far.
    pub settled_amount: i128,
    /// Current status.
    pub status: SettlementStatus,
    /// Number of fills recorded so far.
    pub fill_count: u32,
    /// Ledger when this order was created.
    pub created_at_ledger: u32,
    /// Ledger of the most recent fill (0 if no fills yet).
    pub last_fill_ledger: u32,
    /// Minimum output a single fill must produce (0 = no floor). Issue #991.
    pub min_output: i128,
    /// Maximum input a single fill may consume (`i128::MAX` = no cap). Issue #991.
    pub max_input: i128,
}

impl SettlementOrder {
    /// Remaining amount that still needs to be filled.
    pub fn remaining(&self) -> i128 {
        self.requested_amount
            .saturating_sub(self.settled_amount)
            .max(0)
    }

    /// True when the order has been fully settled or failed.
    pub fn is_closed(&self) -> bool {
        matches!(
            self.status,
            SettlementStatus::FullySettled | SettlementStatus::Failed
        )
    }
}

/// Per-batch summary returned by [`settle_batch`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchSettlementResult {
    /// Order ID being settled.
    pub order_id: u64,
    /// Amount filled in this batch.
    pub batch_filled: i128,
    /// Cumulative amount filled after this batch.
    pub total_settled: i128,
    /// Amount still remaining after this batch.
    pub remaining: i128,
    /// Status after this batch.
    pub status: SettlementStatus,
    /// Fill index assigned to this batch's fill record.
    pub fill_index: u32,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Eq, PartialEq)]
pub enum SettlementError {
    /// Order not found in storage.
    OrderNotFound,
    /// Order is already closed (fully settled or failed).
    OrderAlreadyClosed,
    /// Fill amount or input amount is negative.
    InvalidFillAmount,
    /// Fill amount would exceed the remaining requested amount.
    FillExceedsRemaining,
    /// Next order ID counter would overflow u64.
    OrderIdOverflow,
    /// The caller-supplied slippage bounds contain negative values.
    InvalidSlippageBounds,
    /// The actual fill fell outside the order's caller-supplied slippage bounds
    /// (output below `min_output`, or input above `max_input`). No accounting
    /// update is made — the order and its fills are left untouched.
    SlippageBoundsViolated,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum SettlementKey {
    /// The settlement order for `order_id`.
    Order(u64),
    /// Individual fill record keyed by `(order_id, fill_index)`.
    Fill(u64, u32),
    /// Monotonically increasing next order ID.
    NextOrderId,
    /// All order IDs for a specific user (for listing).
    UserOrders(Address),
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn next_order_id(env: &Env) -> Result<u64, SettlementError> {
    let id: u64 = env
        .storage()
        .persistent()
        .get(&SettlementKey::NextOrderId)
        .unwrap_or(1u64);
    let next = id.checked_add(1).ok_or(SettlementError::OrderIdOverflow)?;
    env.storage()
        .persistent()
        .set(&SettlementKey::NextOrderId, &next);
    Ok(id)
}

fn save_order(env: &Env, order: &SettlementOrder) {
    env.storage()
        .persistent()
        .set(&SettlementKey::Order(order.order_id), order);
}

fn load_order(env: &Env, order_id: u64) -> Result<SettlementOrder, SettlementError> {
    env.storage()
        .persistent()
        .get(&SettlementKey::Order(order_id))
        .ok_or(SettlementError::OrderNotFound)
}

fn save_fill(env: &Env, order_id: u64, fill: &FillRecord) {
    env.storage()
        .persistent()
        .set(&SettlementKey::Fill(order_id, fill.fill_index), fill);
}

fn load_fill(env: &Env, order_id: u64, fill_index: u32) -> Option<FillRecord> {
    env.storage()
        .persistent()
        .get(&SettlementKey::Fill(order_id, fill_index))
}

fn append_user_order(env: &Env, user: &Address, order_id: u64) {
    let key = SettlementKey::UserOrders(user.clone());
    let mut ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    ids.push_back(order_id);
    env.storage().persistent().set(&key, &ids);
}

fn emit_order_created(env: &Env, order_id: u64, user: &Address, requested_amount: i128) {
    env.events().publish(
        (symbol_short!("settle"), symbol_short!("created")),
        (order_id, user.clone(), requested_amount),
    );
}

/// Emit a `settle/fill` event. The body includes both the order's `requested_amount`
/// and the `filled_amount` actually settled, so off-chain trackers can compare
/// requested vs. actual settlement amounts (Issue #991).
fn emit_fill_recorded(
    env: &Env,
    order_id: u64,
    fill_index: u32,
    requested_amount: i128,
    filled_amount: i128,
    total_settled: i128,
    remaining: i128,
    status: SettlementStatus,
) {
    env.events().publish(
        (symbol_short!("settle"), symbol_short!("fill")),
        (
            order_id,
            fill_index,
            requested_amount,
            filled_amount,
            total_settled,
            remaining,
            status,
        ),
    );
}

fn emit_order_closed(env: &Env, order_id: u64, status: SettlementStatus, total_settled: i128) {
    env.events().publish(
        (symbol_short!("settle"), symbol_short!("closed")),
        (order_id, status, total_settled),
    );
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Create a new settlement order for `user` requesting `amount`, bound by the
/// caller-supplied [`SlippageBounds`].
///
/// Returns the assigned `order_id`. The order starts in `Open` status with
/// zero fills. Call [`settle_batch`] one or more times to record fills.
///
/// # Errors
/// - [`SettlementError::InvalidFillAmount`] — `requested_amount <= 0`.
/// - [`SettlementError::InvalidSlippageBounds`] — `slippage` contains a negative value.
pub fn create_settlement_order(
    env: &Env,
    user: Address,
    requested_amount: i128,
    slippage: SlippageBounds,
) -> Result<u64, SettlementError> {
    if requested_amount <= 0 {
        return Err(SettlementError::InvalidFillAmount);
    }
    if slippage.min_output < 0 || slippage.max_input < 0 {
        return Err(SettlementError::InvalidSlippageBounds);
    }

    let order_id = next_order_id(env)?;
    let order = SettlementOrder {
        order_id,
        user: user.clone(),
        requested_amount,
        settled_amount: 0,
        status: SettlementStatus::Open,
        fill_count: 0,
        created_at_ledger: env.ledger().sequence(),
        last_fill_ledger: 0,
        min_output: slippage.min_output,
        max_input: slippage.max_input,
    };

    save_order(env, &order);
    append_user_order(env, &user, order_id);
    emit_order_created(env, order_id, &user, requested_amount);
    Ok(order_id)
}

/// Record a batch fill against an existing settlement order.
///
/// `fill_amount` is the actual output executed in this batch, `input_amount` is
/// the actual input consumed by the external execution, and `execution_price` is
/// the price at which it was filled (7-decimal fixed-point).
///
/// The order's caller-supplied slippage bounds (Issue #991) are enforced
/// against these actual filled amounts *before* any state is mutated:
/// - `fill_amount < min_output` → [`SettlementError::SlippageBoundsViolated`].
/// - `input_amount > max_input` → [`SettlementError::SlippageBoundsViolated`].
///
/// On violation no fill is recorded, no order state changes, and no event is
/// emitted — the settlement is left exactly as it was.
///
/// - If `settled + fill_amount == requested`, the order becomes `FullySettled`.
/// - If `settled + fill_amount < requested`, the order stays `PartiallyFilled`.
/// - `fill_amount > remaining` returns `FillExceedsRemaining`.
///
/// Returns a [`BatchSettlementResult`] reflecting the updated state.
pub fn settle_batch(
    env: &Env,
    order_id: u64,
    fill_amount: i128,
    input_amount: i128,
    execution_price: i128,
) -> Result<BatchSettlementResult, SettlementError> {
    if fill_amount < 0 || input_amount < 0 {
        return Err(SettlementError::InvalidFillAmount);
    }

    let mut order = load_order(env, order_id)?;

    if order.is_closed() {
        return Err(SettlementError::OrderAlreadyClosed);
    }

    // ── Slippage bounds (Issue #991) ───────────────────────────────────────
    // Enforced against the actual filled amounts after external execution,
    // before any mutation, so adverse price movement cannot silently complete
    // a settlement outside the caller's intent.
    if fill_amount < order.min_output || input_amount > order.max_input {
        return Err(SettlementError::SlippageBoundsViolated);
    }

    let remaining_before = order.remaining();
    if fill_amount > remaining_before {
        return Err(SettlementError::FillExceedsRemaining);
    }

    // Update running totals
    order.settled_amount = order
        .settled_amount
        .checked_add(fill_amount)
        .unwrap_or(i128::MAX);
    order.fill_count = order.fill_count.saturating_add(1);
    order.last_fill_ledger = env.ledger().sequence();

    let remaining_after = order.remaining();
    let is_final = remaining_after == 0;

    order.status = if is_final {
        SettlementStatus::FullySettled
    } else {
        SettlementStatus::PartiallyFilled
    };

    let fill = FillRecord {
        fill_index: order.fill_count,
        filled_amount: fill_amount,
        execution_price,
        ledger: env.ledger().sequence(),
        is_final,
    };

    save_fill(env, order_id, &fill);
    save_order(env, &order);

    emit_fill_recorded(
        env,
        order_id,
        fill.fill_index,
        order.requested_amount,
        fill_amount,
        order.settled_amount,
        remaining_after,
        order.status,
    );

    if is_final {
        emit_order_closed(env, order_id, order.status, order.settled_amount);
    }

    Ok(BatchSettlementResult {
        order_id,
        batch_filled: fill_amount,
        total_settled: order.settled_amount,
        remaining: remaining_after,
        status: order.status,
        fill_index: fill.fill_index,
    })
}

/// Mark an open or partially-filled order as failed.
///
/// Records the failure status and emits a `settle/closed` event.
/// Once failed, no further fills can be applied.
pub fn fail_settlement_order(env: &Env, order_id: u64) -> Result<(), SettlementError> {
    let mut order = load_order(env, order_id)?;

    if order.is_closed() {
        return Err(SettlementError::OrderAlreadyClosed);
    }

    order.status = SettlementStatus::Failed;
    save_order(env, &order);
    emit_order_closed(
        env,
        order_id,
        SettlementStatus::Failed,
        order.settled_amount,
    );
    Ok(())
}

/// Read the current state of a settlement order.
pub fn get_settlement_order(env: &Env, order_id: u64) -> Result<SettlementOrder, SettlementError> {
    load_order(env, order_id)
}

/// Read a specific fill record.
pub fn get_fill_record(env: &Env, order_id: u64, fill_index: u32) -> Option<FillRecord> {
    load_fill(env, order_id, fill_index)
}

/// List all order IDs for a user.
pub fn get_user_order_ids(env: &Env, user: &Address) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&SettlementKey::UserOrders(user.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, testutils::Address as _, Env};

    #[contract]
    struct TestSettlementContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        let cid = env.register(TestSettlementContract, ());
        (env, cid)
    }

    /// Bounds that never reject: no output floor, effectively unlimited input.
    fn permissive_bounds() -> SlippageBounds {
        SlippageBounds {
            min_output: 0,
            max_input: i128::MAX,
        }
    }

    // ── Order creation ────────────────────────────────────────────────────────

    #[test]
    fn create_order_assigns_sequential_ids() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id1 = create_settlement_order(&env, user.clone(), 1_000_000, permissive_bounds())
                .unwrap();
            let id2 = create_settlement_order(&env, user.clone(), 2_000_000, permissive_bounds())
                .unwrap();
            assert_ne!(id1, id2);
            assert_eq!(id2, id1 + 1);
        });
    }

    #[test]
    fn create_order_with_zero_amount_fails() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            assert_eq!(
                create_settlement_order(&env, user.clone(), 0, permissive_bounds()),
                Err(SettlementError::InvalidFillAmount)
            );
        });
    }

    #[test]
    fn create_order_initial_state_is_open() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id =
                create_settlement_order(&env, user.clone(), 500_000, permissive_bounds()).unwrap();
            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.status, SettlementStatus::Open);
            assert_eq!(order.settled_amount, 0);
            assert_eq!(order.fill_count, 0);
            assert_eq!(order.remaining(), 500_000);
        });
    }

    // ── Single full fill ──────────────────────────────────────────────────────

    #[test]
    fn full_fill_in_one_batch_marks_fully_settled() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id = create_settlement_order(&env, user.clone(), 1_000_000, permissive_bounds())
                .unwrap();
            let result = settle_batch(&env, id, 1_000_000, 1_000_000, 10_000_000).unwrap();

            assert_eq!(result.batch_filled, 1_000_000);
            assert_eq!(result.total_settled, 1_000_000);
            assert_eq!(result.remaining, 0);
            assert_eq!(result.status, SettlementStatus::FullySettled);
            assert_eq!(result.fill_index, 1);

            let order = get_settlement_order(&env, id).unwrap();
            assert!(order.is_closed());
        });
    }

    // ── Partial fills → full settlement ───────────────────────────────────────

    #[test]
    fn three_partial_fills_accumulate_to_full_settlement() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id =
                create_settlement_order(&env, user.clone(), 900_000, permissive_bounds()).unwrap();

            let r1 = settle_batch(&env, id, 300_000, 300_000, 10_000_000).unwrap();
            assert_eq!(r1.status, SettlementStatus::PartiallyFilled);
            assert_eq!(r1.remaining, 600_000);
            assert_eq!(r1.fill_index, 1);

            let r2 = settle_batch(&env, id, 300_000, 300_000, 10_100_000).unwrap();
            assert_eq!(r2.status, SettlementStatus::PartiallyFilled);
            assert_eq!(r2.remaining, 300_000);
            assert_eq!(r2.fill_index, 2);

            let r3 = settle_batch(&env, id, 300_000, 300_000, 10_200_000).unwrap();
            assert_eq!(r3.status, SettlementStatus::FullySettled);
            assert_eq!(r3.remaining, 0);
            assert_eq!(r3.fill_index, 3);

            // Verify fill records are all persisted
            for idx in 1u32..=3 {
                let fill = get_fill_record(&env, id, idx);
                assert!(fill.is_some(), "fill {idx} should be persisted");
                assert_eq!(fill.unwrap().filled_amount, 300_000);
            }
        });
    }

    // ── Fill exceeds remaining ────────────────────────────────────────────────

    #[test]
    fn fill_exceeding_remaining_is_rejected() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id =
                create_settlement_order(&env, user.clone(), 500_000, permissive_bounds()).unwrap();
            settle_batch(&env, id, 300_000, 300_000, 10_000_000).unwrap();

            // 250_000 > 200_000 remaining
            assert_eq!(
                settle_batch(&env, id, 250_000, 250_000, 10_000_000),
                Err(SettlementError::FillExceedsRemaining)
            );
        });
    }

    // ── Fill on closed order is rejected ─────────────────────────────────────

    #[test]
    fn fill_on_fully_settled_order_is_rejected() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id =
                create_settlement_order(&env, user.clone(), 100_000, permissive_bounds()).unwrap();
            settle_batch(&env, id, 100_000, 100_000, 10_000_000).unwrap();

            assert_eq!(
                settle_batch(&env, id, 1, 1, 10_000_000),
                Err(SettlementError::OrderAlreadyClosed)
            );
        });
    }

    #[test]
    fn fill_on_failed_order_is_rejected() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id =
                create_settlement_order(&env, user.clone(), 100_000, permissive_bounds()).unwrap();
            fail_settlement_order(&env, id).unwrap();

            assert_eq!(
                settle_batch(&env, id, 50_000, 50_000, 10_000_000),
                Err(SettlementError::OrderAlreadyClosed)
            );
        });
    }

    // ── Fail order ────────────────────────────────────────────────────────────

    #[test]
    fn fail_order_after_partial_fill_records_failed_status() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id = create_settlement_order(&env, user.clone(), 1_000_000, permissive_bounds())
                .unwrap();
            settle_batch(&env, id, 400_000, 400_000, 10_000_000).unwrap();
            fail_settlement_order(&env, id).unwrap();

            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.status, SettlementStatus::Failed);
            assert_eq!(
                order.settled_amount, 400_000,
                "partial fill must be preserved"
            );
            assert!(order.is_closed());
        });
    }

    #[test]
    fn fail_already_closed_order_is_idempotent_error() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id =
                create_settlement_order(&env, user.clone(), 100_000, permissive_bounds()).unwrap();
            fail_settlement_order(&env, id).unwrap();
            assert_eq!(
                fail_settlement_order(&env, id),
                Err(SettlementError::OrderAlreadyClosed)
            );
        });
    }

    // ── User order index ──────────────────────────────────────────────────────

    #[test]
    fn user_order_ids_lists_all_created_orders() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id1 =
                create_settlement_order(&env, user.clone(), 100_000, permissive_bounds()).unwrap();
            let id2 =
                create_settlement_order(&env, user.clone(), 200_000, permissive_bounds()).unwrap();
            let id3 =
                create_settlement_order(&env, user.clone(), 300_000, permissive_bounds()).unwrap();

            let ids = get_user_order_ids(&env, &user);
            assert_eq!(ids.len(), 3);
            assert_eq!(ids.get(0).unwrap(), id1);
            assert_eq!(ids.get(1).unwrap(), id2);
            assert_eq!(ids.get(2).unwrap(), id3);
        });
    }

    // ── Mixed success/failure settlement paths ────────────────────────────────

    #[test]
    fn mixed_path_partial_fill_then_fail_then_reject_new_fill() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id = create_settlement_order(&env, user.clone(), 1_000_000, permissive_bounds())
                .unwrap();

            // Partial success
            let r1 = settle_batch(&env, id, 600_000, 600_000, 10_000_000).unwrap();
            assert_eq!(r1.status, SettlementStatus::PartiallyFilled);
            assert_eq!(r1.remaining, 400_000);

            // Failure occurs mid-execution
            fail_settlement_order(&env, id).unwrap();

            // No further fills accepted
            assert_eq!(
                settle_batch(&env, id, 400_000, 400_000, 10_000_000),
                Err(SettlementError::OrderAlreadyClosed)
            );

            // But the partial fill is preserved in storage
            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.settled_amount, 600_000);
            assert_eq!(order.status, SettlementStatus::Failed);
        });
    }

    // ── Nonexistent order ─────────────────────────────────────────────────────

    #[test]
    fn settle_nonexistent_order_returns_not_found() {
        let (env, cid) = setup();
        env.as_contract(&cid, || {
            assert_eq!(
                settle_batch(&env, 9999, 100, 100, 10_000_000),
                Err(SettlementError::OrderNotFound)
            );
        });
    }

    // ── Slippage bounds (Issue #991) ──────────────────────────────────────────

    /// A fill exactly at `min_output` is accepted (exact-boundary case).
    #[test]
    fn exact_boundary_fill_meets_min_output() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 500_000,
                max_input: 1_000_000,
            };
            let id = create_settlement_order(&env, user.clone(), 500_000, bounds).unwrap();
            let result = settle_batch(&env, id, 500_000, 600_000, 10_000_000).unwrap();

            assert_eq!(result.batch_filled, 500_000);
            assert_eq!(result.status, SettlementStatus::FullySettled);
        });
    }

    /// A fill whose input is exactly at `max_input` is accepted (exact-boundary case).
    #[test]
    fn exact_boundary_input_at_max_input() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 100_000,
                max_input: 1_000_000,
            };
            let id = create_settlement_order(&env, user.clone(), 100_000, bounds).unwrap();
            let result = settle_batch(&env, id, 100_000, 1_000_000, 10_000_000).unwrap();

            assert_eq!(result.status, SettlementStatus::FullySettled);
        });
    }

    /// A fill below `min_output` is rejected with no state change.
    #[test]
    fn fill_below_min_output_violates_bounds_without_mutation() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 500_000,
                max_input: 1_000_000,
            };
            let id = create_settlement_order(&env, user.clone(), 1_000_000, bounds).unwrap();

            assert_eq!(
                settle_batch(&env, id, 400_000, 400_000, 10_000_000),
                Err(SettlementError::SlippageBoundsViolated)
            );

            // No fill recorded, no accounting update.
            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.status, SettlementStatus::Open);
            assert_eq!(order.settled_amount, 0);
            assert_eq!(order.fill_count, 0);
            assert!(get_fill_record(&env, id, 1).is_none());
        });
    }

    /// A fill whose input exceeds `max_input` is rejected with no state change.
    #[test]
    fn input_above_max_input_violates_bounds_without_mutation() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 100_000,
                max_input: 900_000,
            };
            let id = create_settlement_order(&env, user.clone(), 1_000_000, bounds).unwrap();

            assert_eq!(
                settle_batch(&env, id, 400_000, 1_000_000, 10_000_000),
                Err(SettlementError::SlippageBoundsViolated)
            );

            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.status, SettlementStatus::Open);
            assert_eq!(order.settled_amount, 0);
            assert_eq!(order.fill_count, 0);
        });
    }

    /// A zero-output fill is accepted when the caller set `min_output == 0`.
    #[test]
    fn zero_output_accepted_when_min_output_is_zero() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 0,
                max_input: 1_000_000,
            };
            let id = create_settlement_order(&env, user.clone(), 1_000_000, bounds).unwrap();
            let result = settle_batch(&env, id, 0, 1_000_000, 10_000_000).unwrap();

            assert_eq!(result.batch_filled, 0);
            assert_eq!(result.status, SettlementStatus::PartiallyFilled);
            assert_eq!(result.remaining, 1_000_000);

            // The zero fill is persisted as a record.
            let fill = get_fill_record(&env, id, 1).expect("zero fill must be recorded");
            assert_eq!(fill.filled_amount, 0);
        });
    }

    /// A zero-output fill violates `min_output` when a positive floor is set.
    #[test]
    fn zero_output_violates_when_min_output_is_positive() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 100,
                max_input: 1_000_000,
            };
            let id = create_settlement_order(&env, user.clone(), 1_000_000, bounds).unwrap();

            assert_eq!(
                settle_batch(&env, id, 0, 1_000_000, 10_000_000),
                Err(SettlementError::SlippageBoundsViolated)
            );

            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.status, SettlementStatus::Open);
            assert_eq!(order.settled_amount, 0);
        });
    }

    /// Bounds are enforced on every partial fill, not just the final one.
    #[test]
    fn partial_fills_respect_bounds_across_batches() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let bounds = SlippageBounds {
                min_output: 200_000,
                max_input: 1_000_000,
            };
            let id = create_settlement_order(&env, user.clone(), 900_000, bounds).unwrap();

            let r1 = settle_batch(&env, id, 300_000, 300_000, 10_000_000).unwrap();
            assert_eq!(r1.status, SettlementStatus::PartiallyFilled);

            let r2 = settle_batch(&env, id, 300_000, 300_000, 10_100_000).unwrap();
            assert_eq!(r2.status, SettlementStatus::PartiallyFilled);

            // Third batch slides below the floor: rejected, previous fills intact.
            assert_eq!(
                settle_batch(&env, id, 150_000, 150_000, 10_200_000),
                Err(SettlementError::SlippageBoundsViolated)
            );

            let order = get_settlement_order(&env, id).unwrap();
            assert_eq!(order.settled_amount, 600_000);
            assert_eq!(order.fill_count, 2);
        });
    }

    /// Negative bounds are rejected at order creation.
    #[test]
    fn negative_slippage_bounds_are_rejected() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            assert_eq!(
                create_settlement_order(
                    &env,
                    user.clone(),
                    1_000_000,
                    SlippageBounds {
                        min_output: -1,
                        max_input: 1_000_000,
                    },
                ),
                Err(SettlementError::InvalidSlippageBounds)
            );
            assert_eq!(
                create_settlement_order(
                    &env,
                    user.clone(),
                    1_000_000,
                    SlippageBounds {
                        min_output: 0,
                        max_input: -1,
                    },
                ),
                Err(SettlementError::InvalidSlippageBounds)
            );
        });
    }

    /// Negative input amounts are rejected as invalid fills.
    #[test]
    fn negative_input_amount_is_rejected() {
        let (env, cid) = setup();
        let user = Address::generate(&env);
        env.as_contract(&cid, || {
            let id = create_settlement_order(&env, user.clone(), 1_000_000, permissive_bounds())
                .unwrap();
            assert_eq!(
                settle_batch(&env, id, 500_000, -1, 10_000_000),
                Err(SettlementError::InvalidFillAmount)
            );
        });
    }
}
