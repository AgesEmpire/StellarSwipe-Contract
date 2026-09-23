use soroban_sdk::{contracterror, contracttype, Env, String, Symbol};

/// Canonical failure taxonomy shared by every contract (Issue #1033).
///
/// Integrators map an on-chain error code to exactly one category so frontend
/// clients and scripts can branch on the *kind* of failure without hard-coding
/// per-contract error numbers:
///
/// | Category             | Meaning                              | Typical client response          |
/// |----------------------|--------------------------------------|----------------------------------|
/// | `Validation`         | Malformed / out-of-range input       | Fix the request and resubmit     |
/// | `Authorization`      | Caller lacks permission              | Re-authenticate / switch signer  |
/// | `ExternalDependency` | A dependency (oracle, token) failed  | Retry later / check dependency   |
/// | `Arithmetic`         | Overflow / division by zero          | Reduce amounts; report if unexpected |
/// | `Upgrade`            | Version / migration mismatch         | Upgrade client or contract       |
/// | `Network`            | Transient transport / gateway issue  | Backoff and retry                |
/// | `Recovery`           | Guardian / recovery-flow failure     | Escalate to manual review        |
/// | `CapacityLimit`      | A quota / rate / size cap was hit    | Wait for the window to reset     |
/// | `InvariantViolation` | A protocol invariant would break     | Do not retry; report a bug       |
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCategory {
    Validation = 1,
    Authorization = 2,
    ExternalDependency = 3,
    Arithmetic = 4,
    Upgrade = 5,
    Network = 6,
    Recovery = 7,
    /// A quota, rate limit, batch size, or other capacity threshold was reached.
    CapacityLimit = 8,
    /// An operation was rejected because it would break a protocol invariant
    /// (e.g. conservation of funds, monotonic version, terminal state).
    InvariantViolation = 9,
}

impl ErrorCategory {
    /// Stable lowercase slug for logs, metrics, and SDK switch statements.
    pub fn slug(&self) -> &'static str {
        match self {
            ErrorCategory::Validation => "validation",
            ErrorCategory::Authorization => "authorization",
            ErrorCategory::ExternalDependency => "external_dependency",
            ErrorCategory::Arithmetic => "arithmetic",
            ErrorCategory::Upgrade => "upgrade",
            ErrorCategory::Network => "network",
            ErrorCategory::Recovery => "recovery",
            ErrorCategory::CapacityLimit => "capacity_limit",
            ErrorCategory::InvariantViolation => "invariant_violation",
        }
    }

    /// Whether retrying the *same* request unchanged could plausibly succeed
    /// later. `Validation`, `Authorization`, and `InvariantViolation` failures
    /// never clear on their own.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ErrorCategory::ExternalDependency
                | ErrorCategory::Network
                | ErrorCategory::CapacityLimit
        )
    }

    /// The recovery strategy an integrator should default to for this category.
    pub fn default_strategy(&self) -> RecoveryStrategy {
        match self {
            ErrorCategory::Network | ErrorCategory::ExternalDependency => RecoveryStrategy::Retry,
            ErrorCategory::CapacityLimit => RecoveryStrategy::Defer,
            ErrorCategory::Recovery | ErrorCategory::InvariantViolation => {
                RecoveryStrategy::ManualReview
            }
            ErrorCategory::Validation
            | ErrorCategory::Authorization
            | ErrorCategory::Arithmetic
            | ErrorCategory::Upgrade => RecoveryStrategy::Escalate,
        }
    }
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RecoveryStrategy {
    Retry = 1,
    Defer = 2,
    Escalate = 3,
    ManualReview = 4,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorReport {
    pub category: ErrorCategory,
    pub strategy: RecoveryStrategy,
    pub message: String,
    pub timestamp: u64,
}

// ── Machine-readable error metadata for SDKs and off-chain clients ─────────────
//
// Structured schema so clients can programmatically interpret contract failures
// without resorting to ad-hoc string parsing.  Each `ErrorMetadata` record
// carries a stable `code` plus category, strategy, and actionable hints.

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMetadata {
    pub schema_version: u32,
    pub code: u32,
    pub category: ErrorCategory,
    pub strategy: RecoveryStrategy,
    pub message: String,
    pub recovery_hint: String,
    pub is_retryable: bool,
    pub client_action: String,
    pub timestamp: u64,
}

pub const ERROR_METADATA_SCHEMA_VERSION: u32 = 1;

/// Build an [`ErrorMetadata`] record on the current ledger timestamp.
pub fn make_error_metadata(
    env: &Env,
    code: u32,
    category: ErrorCategory,
    strategy: RecoveryStrategy,
    message: String,
    recovery_hint: String,
    is_retryable: bool,
    client_action: String,
) -> ErrorMetadata {
    ErrorMetadata {
        schema_version: ERROR_METADATA_SCHEMA_VERSION,
        code,
        category,
        strategy,
        message,
        recovery_hint,
        is_retryable,
        client_action,
        timestamp: env.ledger().timestamp(),
    }
}

/// Publish `metadata` on the `("error", "metadata")` topic so off-chain
/// indexers can route failures without parsing contract-specific error strings.
pub fn emit_error_metadata(env: &Env, metadata: &ErrorMetadata) {
    #[allow(deprecated)]
    env.events().publish(
        (Symbol::new(env, "error"), Symbol::new(env, "metadata")),
        metadata,
    );
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, testutils::Address as _, Env};

    #[contract]
    struct TestContract;

    fn setup() -> Env {
        let env = Env::default();
        let _id = env.register(TestContract, ());
        env
    }

    #[test]
    fn error_report_creation() {
        let env = setup();
        let report = ErrorReport {
            category: ErrorCategory::Authorization,
            strategy: RecoveryStrategy::Escalate,
            message: String::from_str(&env, "unauthorized"),
            timestamp: 123,
        };
        assert_eq!(report.category, ErrorCategory::Authorization);
        assert_eq!(report.strategy, RecoveryStrategy::Escalate);
    }

    #[test]
    fn error_metadata_has_schema_version() {
        let env = setup();
        let meta = make_error_metadata(
            &env,
            1,
            ErrorCategory::Validation,
            RecoveryStrategy::Retry,
            String::from_str(&env, "bad input"),
            String::from_str(&env, "check input and retry"),
            true,
            String::from_str(&env, "retry"),
        );
        assert_eq!(meta.schema_version, 1);
        assert_eq!(meta.code, 1);
        assert!(meta.is_retryable);
    }

    #[test]
    fn taxonomy_slugs_are_unique_and_stable() {
        let cats = [
            ErrorCategory::Validation,
            ErrorCategory::Authorization,
            ErrorCategory::ExternalDependency,
            ErrorCategory::Arithmetic,
            ErrorCategory::Upgrade,
            ErrorCategory::Network,
            ErrorCategory::Recovery,
            ErrorCategory::CapacityLimit,
            ErrorCategory::InvariantViolation,
        ];
        let mut seen: [&str; 9] = [""; 9];
        for (i, c) in cats.iter().enumerate() {
            let s = c.slug();
            assert!(!s.is_empty());
            assert!(!seen.contains(&s), "duplicate slug {s}");
            seen[i] = s;
        }
        assert_eq!(ErrorCategory::CapacityLimit.slug(), "capacity_limit");
        assert_eq!(
            ErrorCategory::InvariantViolation.slug(),
            "invariant_violation"
        );
    }

    #[test]
    fn taxonomy_transience_and_strategy() {
        assert!(ErrorCategory::CapacityLimit.is_transient());
        assert!(ErrorCategory::Network.is_transient());
        assert!(!ErrorCategory::Validation.is_transient());
        assert!(!ErrorCategory::Authorization.is_transient());
        assert!(!ErrorCategory::InvariantViolation.is_transient());

        assert_eq!(
            ErrorCategory::CapacityLimit.default_strategy(),
            RecoveryStrategy::Defer
        );
        assert_eq!(
            ErrorCategory::InvariantViolation.default_strategy(),
            RecoveryStrategy::ManualReview
        );
        assert_eq!(
            ErrorCategory::Network.default_strategy(),
            RecoveryStrategy::Retry
        );
    }

    #[test]
    fn emit_error_metadata_publishes_event() {
        use soroban_sdk::testutils::Events;
        let env = setup();
        let env_id = env.register(TestContract, ());

        env.as_contract(&env_id, || {
            let meta = make_error_metadata(
                &env,
                2,
                ErrorCategory::Network,
                RecoveryStrategy::Defer,
                String::from_str(&env, "gateway timeout"),
                String::from_str(&env, "retry later"),
                true,
                String::from_str(&env, "backoff"),
            );
            emit_error_metadata(&env, &meta);
            assert!(!env.events().all().is_empty());
        });
    }
}
