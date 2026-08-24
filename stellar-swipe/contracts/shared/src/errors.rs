use soroban_sdk::{contracterror, contracttype, Env, String, Symbol};

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
