//! Structured panic message convention for intentional contract panics (issue #596).
//!
//! When a contract panics intentionally — as opposed to via a `Result`/
//! `ContractError` return, or an `unwrap()`/`expect()` on a condition that
//! should never happen — the panic message should use the `structured_panic!`
//! macro below rather than a bare `panic!("...")`.
//!
//! Format: `SSW-<code>: <context>`
//! - `SSW-` is a fixed prefix, making structured panics grep-able and
//!   visually distinct from a generic Rust panic (e.g. an `unwrap()` on a
//!   truly-unexpected condition has no `SSW-` prefix or code at all).
//! - `<code>` is a stable numeric code, unique per intentional-panic call
//!   site. Codes are allocated in fixed per-contract ranges (documented in
//!   `CONTRIBUTING.md`) so a code alone identifies which contract and
//!   call site produced it, even from an off-chain log with no source access.
//! - `<context>` is a short human-readable description of what went wrong.
//!
//! Convention/rationale for panic vs. `Result`: an intentional panic is for
//! conditions that represent a programming/configuration error the caller
//! cannot meaningfully recover from within the same call (e.g. "governance
//! double-initialized", "entry price of zero passed to ROI math") — as
//! opposed to expected, recoverable failure modes, which should return a
//! `ContractError`/`AdminError`/etc. `Result` instead.

/// Panic with a structured `SSW-<code>: <context>` message (issue #596).
///
/// ```ignore
/// stellar_swipe_common::structured_panic!(9100, "entry price cannot be zero");
/// stellar_swipe_common::structured_panic!(9100, "invalid amount: {}", amount);
/// ```
#[macro_export]
macro_rules! structured_panic {
    ($code:expr, $msg:expr) => {
        panic!("SSW-{}: {}", $code, $msg)
    };
    ($code:expr, $fmt:expr, $($arg:tt)*) => {
        panic!(concat!("SSW-{}: ", $fmt), $code, $($arg)*)
    };
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic(expected = "SSW-9999: deliberately broken for lint/format verification")]
    fn structured_panic_produces_expected_format() {
        crate::structured_panic!(9999, "deliberately broken for lint/format verification");
    }

    #[test]
    #[should_panic(expected = "SSW-9998: amount 42 out of range")]
    fn structured_panic_supports_format_args() {
        let amount = 42;
        crate::structured_panic!(9998, "amount {} out of range", amount);
    }
}
