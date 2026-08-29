//! Standardized mapping from SEP-41 token / cross-contract invocation
//! failures to a small, stable set of categories (Issue #1001).
//!
//! Soroban's generated `try_*` client methods (both the SEP-41
//! [`soroban_sdk::token::TokenClient`] and any other contract client
//! produced by `#[contractclient]`, as well as raw [`soroban_sdk::Env::try_invoke_contract`]
//! calls) surface a callee failure as either:
//! - `Err(Ok(error))` — a structured [`soroban_sdk::Error`] the callee returned or
//!   trapped with (e.g. a Stellar Asset Contract error code), or
//! - `Err(Err(invoke_error))` — a host-level invocation abort that carried no
//!   recoverable error code (budget exhaustion, a bare `panic!`, a missing
//!   contract, etc).
//!
//! Every StellarSwipe contract that moves tokens or calls into another
//! contract (stake vault deposits/withdrawals/slashing, fee collection and
//! payout, and the SDEX/AMM router bridge used by the trade flow) previously
//! used the *panicking* client methods (`transfer`, `approve`, `burn`,
//! `env.invoke_contract`), which abort the whole transaction with an opaque
//! host trap on failure, or handled `try_*` results ad hoc and collapsed
//! every failure reason into one generic error. This module gives every
//! call site a single, documented place to classify the failure instead.
//!
//! **Policy: a failed token or cross-contract operation must never be
//! reported to a caller as `Ok`.** [`classify`] and [`map_result`] always
//! return `Err` for every non-success outcome; there is no code path in this
//! module that turns a failure into a success.

use soroban_sdk::{xdr::ScErrorType, Error, InvokeError};

/// Stable, contract-agnostic classification of a failed token or
/// cross-contract invocation.
///
/// Each StellarSwipe contract keeps its own `#[contracterror]` enum (error
/// codes are public ABI and must not be renumbered), so this type is not
/// itself a `contracterror` — it is the shared vocabulary that every
/// contract's local error maps onto via `impl From<TokenFailure> for
/// <LocalError>`. That keeps the numeric *codes* private and stable per
/// contract while centralizing the *classification policy* here.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TokenFailure {
    /// The call was rejected because required authorization was missing or
    /// invalid — `require_auth` failed inside the callee, or the host
    /// reports an `Auth`-type error. Covers the Stellar Asset Contract's
    /// `UnauthorizedError` / `AuthenticationError`.
    Unauthorized,
    /// The `from` address did not have enough balance to cover the
    /// transfer/burn. Covers the Stellar Asset Contract's `BalanceError`
    /// and `BalanceDeauthorizedError` (a frozen/deauthorized trustline is
    /// surfaced the same way to callers — in both cases the funds are not
    /// movable).
    InsufficientBalance,
    /// The `spender` did not hold a sufficient (or unexpired) allowance
    /// from `from`. Covers the Stellar Asset Contract's `AllowanceError`.
    InsufficientAllowance,
    /// The callee rejected the request as invalid input: a negative
    /// amount, a missing trustline, or a non-classic/missing account.
    /// Covers `NegativeAmountError`, `TrustlineMissingError`,
    /// `AccountMissingError`, `AccountIsNotClassic`.
    InvalidRequest,
    /// Arithmetic overflowed inside the callee. Covers `OverflowError`.
    Overflow,
    /// A structured contract error was returned that does not match any of
    /// the well-known Stellar Asset Contract codes above. Custom SEP-41
    /// tokens and routers are free to define their own error codes; this
    /// is the documented passthrough for those rather than silently
    /// misclassifying them as one of the categories above. The `u32` is
    /// the raw contract error code for diagnostics.
    OtherContractError(u32),
    /// A host-level failure that never reached callee logic with a
    /// recoverable error code: budget/resource exhaustion, a missing or
    /// uninstalled contract, a bare `panic!`, or any other invocation
    /// abort.
    HostError,
}

/// Classifies the `Err` arm of a `try_*` client call —
/// `Result<soroban_sdk::Error, soroban_sdk::InvokeError>`, exactly the type
/// every `#[contractclient]`-generated `try_*` method and
/// `Env::try_invoke_contract` produce on failure — into a [`TokenFailure`].
pub fn classify(failure: Result<Error, InvokeError>) -> TokenFailure {
    match failure {
        Ok(error) => classify_error(error),
        Err(InvokeError::Contract(code)) => classify_code(code),
        Err(InvokeError::Abort) => TokenFailure::HostError,
    }
}

fn classify_error(error: Error) -> TokenFailure {
    if error.is_type(ScErrorType::Contract) {
        classify_code(error.get_code())
    } else if error.is_type(ScErrorType::Auth) {
        TokenFailure::Unauthorized
    } else {
        TokenFailure::HostError
    }
}

/// Stellar Asset Contract / built-in token error codes, as defined by the
/// protocol's native token contract implementation. These codes are stable
/// public ABI. Custom SEP-41 tokens may use different codes for their own
/// errors; unrecognized codes fall through to `OtherContractError` as a
/// documented passthrough rather than being silently miscategorized.
fn classify_code(code: u32) -> TokenFailure {
    match code {
        4 /* UnauthorizedError */ | 5 /* AuthenticationError */ => TokenFailure::Unauthorized,
        9 /* AllowanceError */ => TokenFailure::InsufficientAllowance,
        10 /* BalanceError */ | 11 /* BalanceDeauthorizedError */ => {
            TokenFailure::InsufficientBalance
        }
        8 /* NegativeAmountError */
        | 6 /* AccountMissingError */
        | 7 /* AccountIsNotClassic */
        | 13 /* TrustlineMissingError */ => TokenFailure::InvalidRequest,
        12 /* OverflowError */ => TokenFailure::Overflow,
        other => TokenFailure::OtherContractError(other),
    }
}

/// Runs the full result of a `try_*` client call (or `try_invoke_contract`)
/// through the mapping policy in one step: `Ok(Ok(value))` passes through
/// unchanged, and every other outcome — including the value-conversion
/// error arm, which practically never fires for well-formed callees but
/// must still never be read as success — becomes a classified `Err`.
///
/// Use this at every token/cross-contract call site instead of the
/// panicking client methods, so a failed operation can never be silently
/// treated as successful.
pub fn map_result<T, C>(
    result: Result<Result<T, C>, Result<Error, InvokeError>>,
) -> Result<T, TokenFailure> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_conversion_error)) => Err(TokenFailure::HostError),
        Err(failure) => Err(classify(failure)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::xdr::ScErrorCode;

    #[test]
    fn classifies_unauthorized_from_contract_code() {
        assert_eq!(
            classify(Ok(Error::from_contract_error(4))),
            TokenFailure::Unauthorized
        );
        assert_eq!(
            classify(Ok(Error::from_contract_error(5))),
            TokenFailure::Unauthorized
        );
    }

    #[test]
    fn classifies_unauthorized_from_host_auth_error() {
        // A host-level Auth error (e.g. `require_auth` failing before the
        // callee even runs its own logic) must also be classified as
        // Unauthorized, not lumped in with generic host errors.
        let auth_error = Error::from_type_and_code(ScErrorType::Auth, ScErrorCode::InvalidAction);
        assert_eq!(classify(Ok(auth_error)), TokenFailure::Unauthorized);
    }

    #[test]
    fn classifies_insufficient_balance() {
        assert_eq!(
            classify(Ok(Error::from_contract_error(10))),
            TokenFailure::InsufficientBalance
        );
        assert_eq!(
            classify(Ok(Error::from_contract_error(11))),
            TokenFailure::InsufficientBalance
        );
    }

    #[test]
    fn classifies_insufficient_allowance() {
        assert_eq!(
            classify(Ok(Error::from_contract_error(9))),
            TokenFailure::InsufficientAllowance
        );
    }

    #[test]
    fn classifies_host_error_from_abort() {
        assert_eq!(classify(Err(InvokeError::Abort)), TokenFailure::HostError);
    }

    #[test]
    fn classifies_host_error_from_non_contract_non_auth_type() {
        let budget_error =
            Error::from_type_and_code(ScErrorType::Budget, ScErrorCode::ExceededLimit);
        assert_eq!(classify(Ok(budget_error)), TokenFailure::HostError);
    }

    #[test]
    fn classifies_unknown_contract_code_as_other_passthrough() {
        assert_eq!(
            classify(Ok(Error::from_contract_error(999))),
            TokenFailure::OtherContractError(999)
        );
    }

    #[test]
    fn map_result_passes_through_success() {
        let ok: Result<Result<i128, ()>, Result<Error, InvokeError>> = Ok(Ok(42));
        assert_eq!(map_result(ok), Ok(42));
    }

    #[test]
    fn map_result_never_reports_success_on_failure() {
        let failed: Result<Result<i128, ()>, Result<Error, InvokeError>> =
            Err(Ok(Error::from_contract_error(10)));
        assert_eq!(map_result(failed), Err(TokenFailure::InsufficientBalance));

        let aborted: Result<Result<i128, ()>, Result<Error, InvokeError>> =
            Err(Err(InvokeError::Abort));
        assert_eq!(map_result(aborted), Err(TokenFailure::HostError));
    }
}
