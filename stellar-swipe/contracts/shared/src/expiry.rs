/// Trait for values that carry a timestamp-based expiry.
///
/// Boundary semantics: a value is expired when `current_ledger_time` is
/// **strictly greater** than `expiry_timestamp()`.  At the exact expiry
/// second the value is still considered live (exclusive upper bound).
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

    /// Returns `true` when `current_ledger_time > expiry_timestamp()`.
    ///
    /// At the exact expiry second (`current_ledger_time == expiry_timestamp()`)
    /// the value is **not** yet expired.
    fn is_expired(&self, current_ledger_time: u64) -> bool {
        current_ledger_time > self.expiry_timestamp()
    }
}
