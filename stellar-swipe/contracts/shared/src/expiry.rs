/// Trait for values that carry a timestamp-based expiry.
///
/// Boundary semantics (issue #1400): deadlines are **inclusive** at the
/// deadline second. A value is expired when `current_ledger_time` is
/// **greater than or equal to** `expiry_timestamp()`. At `now == expiry` the
/// value is already expired; at `expiry - 1` it is still live.
///
/// The same rule applies across the contracts:
/// - Reward claims and campaign expiry (`stake_vault::reward_campaign`):
///   claimable while `now < expires_at`.
/// - Queued withdrawal claims (`stake_vault::withdrawal_claims`): expired
///   when `now >= expires_at`.
/// - Subscriptions (`user_portfolio::subscriptions`): active while
///   `now < expires_at`.
/// - Emergency unstake request timeout (`stake_vault::emergency_unstake`):
///   expired when `now >= requested_at + timeout_secs`.
///
/// "Ready" deadlines (unbonding `unlock_at`, per-account emergency cooldown
/// `last + cooldown`) use the same inclusive rule: the action is allowed when
/// `now >= ready_at`.
///
/// # Example
///
/// ```ignore
/// impl Expirable for MyStruct {
///     fn expiry_timestamp(&self) -> u64 {
///         self.expires_at
///     }
/// }
///
/// if record.is_expired(env.ledger().timestamp()) {
///     // handle expiry
/// }
/// ```
pub trait Expirable {
    /// The ledger timestamp (Unix seconds) at which this value expires.
    fn expiry_timestamp(&self) -> u64;

    /// Returns `true` when `current_ledger_time >= expiry_timestamp()`.
    ///
    /// At the exact expiry second (`current_ledger_time == expiry_timestamp()`)
    /// the value is already expired.
    fn is_expired(&self, current_ledger_time: u64) -> bool {
        current_ledger_time >= self.expiry_timestamp()
    }
}
