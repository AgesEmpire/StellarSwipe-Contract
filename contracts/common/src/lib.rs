//! Shared types and helpers for the protocol contracts.

use serde::{Deserialize, Serialize};

/// Numeric type used by a protocol parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    /// Unsigned integer value.
    Uint,
    /// Signed integer value.
    Int,
    /// Fixed-point decimal value.
    Decimal,
}

/// Who is allowed to update a protocol parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePermission {
    /// Only governance may update the parameter.
    Governance,
    /// Only the protocol admin may update the parameter.
    Admin,
    /// The parameter is immutable after initialization.
    Immutable,
}

/// Error returned when a parameter update violates its schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamSchemaError {
    /// The supplied value is below the schema minimum.
    BelowMinimum,
    /// The supplied value is above the schema maximum.
    AboveMaximum,
    /// The supplied value does not match the schema precision.
    PrecisionMismatch,
    /// The supplied unit does not match the schema unit.
    UnitMismatch,
    /// The caller is not permitted to update the parameter.
    Unauthorized,
}

/// Shared schema describing a configurable protocol parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamSchema {
    /// Stable identifier of the parameter.
    pub key: &'static str,
    /// Numeric type of the parameter value.
    pub param_type: ParamType,
    /// Unit the value is expressed in (e.g. "bps", "seconds").
    pub unit: &'static str,
    /// Number of decimal places the value is expressed with.
    pub precision: u32,
    /// Inclusive minimum value, expressed in `unit`.
    pub min: i128,
    /// Inclusive maximum value, expressed in `unit`.
    pub max: i128,
    /// Who may update the parameter.
    pub permission: UpdatePermission,
}

impl ParamSchema {
    /// Validate a candidate value against this schema.
    ///
    /// `value` is expressed in the schema's smallest unit (i.e. already
    /// scaled by `10^precision`), and `unit` is the unit the caller supplied.
    pub fn validate(&self, value: i128, unit: &str) -> Result<(), ParamSchemaError> {
        if unit != self.unit {
            return Err(ParamSchemaError::UnitMismatch);
        }
        if value < self.min {
            return Err(ParamSchemaError::BelowMinimum);
        }
        if value > self.max {
            return Err(ParamSchemaError::AboveMaximum);
        }
        Ok(())
    }

    /// Validate a candidate value together with its declared precision.
    pub fn validate_with_precision(
        &self,
        value: i128,
        unit: &str,
        precision: u32,
    ) -> Result<(), ParamSchemaError> {
        if precision != self.precision {
            return Err(ParamSchemaError::PrecisionMismatch);
        }
        self.validate(value, unit)
    }
}

/// Schema entry for every configurable protocol parameter.
///
/// Values are expressed in the schema's smallest unit, so a parameter with
/// `precision = 2` and `unit = "percent"` stores `50` for `0.50%`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolParamSchema {
    /// All configurable parameters, in a stable order.
    pub params: Vec<ParamSchema>,
}

impl ProtocolParamSchema {
    /// Build the canonical schema for all configurable protocol parameters.
    pub fn canonical() -> Self {
        Self {
            params: vec![
                ParamSchema {
                    key: "min_collateral_ratio",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 2,
                    min: 10_000,
                    max: 100_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "liquidation_penalty",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 2,
                    min: 0,
                    max: 5_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "interest_rate",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 4,
                    min: 0,
                    max: 10_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "max_oracle_staleness",
                    param_type: ParamType::Uint,
                    unit: "seconds",
                    precision: 0,
                    min: 1,
                    max: 86_400,
                    permission: UpdatePermission::Admin,
                },
                ParamSchema {
                    key: "protocol_fee",
                    param_type: ParamType::Decimal,
                    unit: "percent",
                    precision: 2,
                    min: 0,
                    max: 1_000,
                    permission: UpdatePermission::Governance,
                },
                ParamSchema {
                    key: "chain_id",
                    param_type: ParamType::Uint,
                    unit: "id",
                    precision: 0,
                    min: 1,
                    max: i128::MAX,
                    permission: UpdatePermission::Immutable,
                },
            ],
        }
    }

    /// Look up the schema entry for a parameter key.
    pub fn get(&self, key: &str) -> Option<&ParamSchema> {
        self.params.iter().find(|p| p.key == key)
    }

    /// Validate an update against the schema entry for `key`.
    pub fn validate_update(
        &self,
        key: &str,
        value: i128,
        unit: &str,
        precision: u32,
    ) -> Result<(), ParamSchemaError> {
        let schema = self.get(key).ok_or(ParamSchemaError::UnitMismatch)?;
        schema.validate_with_precision(value, unit, precision)
    }

    /// Render the schema as documentation for clients.
    pub fn to_documentation(&self) -> String {
        let mut out = String::from("# Protocol Parameter Schema\n\n");
        out.push_str("| key | type | unit | precision | min | max | permission |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        for p in &self.params {
            out.push_str(&format!(
                "| {} | {:?} | {} | {} | {} | {} | {:?} |\n",
                p.key, p.param_type, p.unit, p.precision, p.min, p.max, p.permission
            ));
        }
        out
    }
}

/// Version of the normalized TTL renewal outcome event schema.
///
/// Bump this whenever the fields of [`TtlRenewalOutcome`] change so that
/// off-chain consumers can detect incompatible event payloads.
pub const TTL_RENEWAL_OUTCOME_EVENT_VERSION: u32 = 1;

/// Normalized outcome of a single storage TTL renewal attempt.
///
/// Every shared TTL caller emits this event instead of ad-hoc per-caller
/// emissions, so off-chain indexers observe one deterministic shape for
/// skipped, renewed, and failed renewals.
///
/// Fields:
/// - `version`: schema version, see [`TTL_RENEWAL_OUTCOME_EVENT_VERSION`].
/// - `key`: storage key whose TTL was considered for renewal.
/// - `result`: normalized outcome of the attempt.
/// - `current_ttl`: TTL (in ledgers) observed before the attempt.
/// - `threshold`: TTL (in ledgers) below which renewal is attempted.
/// - `extend_to`: TTL (in ledgers) requested when a renewal is performed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtlRenewalOutcome {
    /// Schema version of this event payload.
    pub version: u32,
    /// Storage key whose TTL was considered for renewal.
    pub key: String,
    /// Normalized outcome of the attempt.
    pub result: TtlRenewalResult,
    /// TTL (in ledgers) observed before the attempt.
    pub current_ttl: u32,
    /// TTL (in ledgers) below which renewal is attempted.
    pub threshold: u32,
    /// TTL (in ledgers) requested when a renewal is performed.
    pub extend_to: u32,
}

/// Normalized result of a storage TTL renewal attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtlRenewalResult {
    /// The TTL was above the threshold, so no renewal was needed.
    Skipped,
    /// The TTL was renewed successfully.
    Renewed,
    /// The renewal attempt failed.
    Failed,
}

/// Version of the guardian recovery event schema.
///
/// Bump this whenever the fields of [`GuardianRecoveryEvent`] change so that
/// off-chain consumers can detect incompatible event payloads.
pub const GUARDIAN_RECOVERY_EVENT_VERSION: u32 = 1;

/// Configuration governing delayed guardian recovery of administrator access.
///
/// Recovery may only be executed once at least `threshold` distinct guardians
/// have approved the pending request and at least `delay` seconds have elapsed
/// since the request was initiated. The active administrator may cancel a
/// pending request at any point during the delay window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianRecoveryConfig {
    /// Minimum number of distinct guardian approvals required to execute.
    pub threshold: u32,
    /// Delay (in seconds) that must elapse between initiation and execution.
    pub delay: u64,
}

/// Lifecycle state of a pending guardian recovery request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardianRecoveryStatus {
    /// The request is collecting approvals and awaiting the delay window.
    Pending,
    /// The request was executed and administrator access was restored.
    Executed,
    /// The active administrator cancelled the request during the delay.
    Cancelled,
}

/// A pending guardian recovery request for lost administrator access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianRecoveryRequest {
    /// Address of the administrator whose access is being recovered.
    pub admin: String,
    /// Ledger timestamp (in seconds) at which the request was initiated.
    pub initiated_at: u64,
    /// Distinct guardians that have approved the request.
    pub approvals: Vec<String>,
    /// Current lifecycle state of the request.
    pub status: GuardianRecoveryStatus,
}

/// Errors returned by the guardian recovery flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardianRecoveryError {
    /// No recovery request is currently pending.
    NoPendingRequest,
    /// The caller is not a configured guardian.
    NotAGuardian,
    /// The guardian has already approved this request.
    AlreadyApproved,
    /// Fewer than the configured threshold of approvals have been collected.
    ThresholdNotMet,
    /// The configured delay has not yet elapsed.
    DelayNotElapsed,
    /// The caller is not the active administrator.
    NotActiveAdmin,
}

impl GuardianRecoveryRequest {
    /// Initiate a new pending recovery request for `admin` at `now`.
    pub fn initiate(admin: String, now: u64) -> Self {
        Self {
            admin,
            initiated_at: now,
            approvals: Vec::new(),
            status: GuardianRecoveryStatus::Pending,
        }
    }

    /// Record a guardian approval, enforcing guardian membership and
    /// preventing duplicate approvals.
    pub fn approve(
        &mut self,
        guardian: &str,
        guardians: &[String],
    ) -> Result<(), GuardianRecoveryError> {
        if self.status != GuardianRecoveryStatus::Pending {
            return Err(GuardianRecoveryError::NoPendingRequest);
        }
        if !guardians.iter().any(|g| g == guardian) {
            return Err(GuardianRecoveryError::NotAGuardian);
        }
        if self.approvals.iter().any(|g| g == guardian) {
            return Err(GuardianRecoveryError::AlreadyApproved);
        }
        self.approvals.push(guardian.to_string());
        Ok(())
    }

    /// Whether the configured approval threshold has been reached.
    pub fn threshold_met(&self, config: &GuardianRecoveryConfig) -> bool {
        self.approvals.len() as u32 >= config.threshold
    }

    /// Whether the configured delay has elapsed at `now`.
    pub fn delay_elapsed(&self, config: &GuardianRecoveryConfig, now: u64) -> bool {
        now >= self.initiated_at.saturating_add(config.delay)
    }

    /// Execute the recovery, enforcing threshold and delay boundaries.
    pub fn execute(
        &mut self,
        config: &GuardianRecoveryConfig,
        now: u64,
    ) -> Result<(), GuardianRecoveryError> {
        if self.status != GuardianRecoveryStatus::Pending {
            return Err(GuardianRecoveryError::NoPendingRequest);
        }
        if !self.threshold_met(config) {
            return Err(GuardianRecoveryError::ThresholdNotMet);
        }
        if !self.delay_elapsed(config, now) {
            return Err(GuardianRecoveryError::DelayNotElapsed);
        }
        self.status = GuardianRecoveryStatus::Executed;
        Ok(())
    }

    /// Cancel the pending recovery. Only the active administrator may cancel,
    /// and only while the request is still pending.
    pub fn cancel(&mut self, caller: &str) -> Result<(), GuardianRecoveryError> {
        if self.status != GuardianRecoveryStatus::Pending {
            return Err(GuardianRecoveryError::NoPendingRequest);
        }
        if caller != self.admin {
            return Err(GuardianRecoveryError::NotActiveAdmin);
        }
        self.status = GuardianRecoveryStatus::Cancelled;
        Ok(())
    }
}

/// Auditable event emitted for guardian recovery lifecycle transitions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianRecoveryEvent {
    /// Schema version of this event payload.
    pub version: u32,
    /// Address of the administrator whose access is being recovered.
    pub admin: String,
    /// Lifecycle transition that occurred.
    pub status: GuardianRecoveryStatus,
    /// Number of distinct approvals collected at the time of the event.
    pub approvals: u32,
    /// Ledger timestamp (in seconds) at which the event occurred.
    pub timestamp: u64,
}

#[cfg(test)]
mod guardian_recovery_tests {
    use super::*;

    fn guardians() -> Vec<String> {
        vec!["g1".to_string(), "g2".to_string(), "g3".to_string()]
    }

    fn config() -> GuardianRecoveryConfig {
        GuardianRecoveryConfig {
            threshold: 2,
            delay: 3_600,
        }
    }

    #[test]
    fn rejects_execution_below_threshold() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        req.approve("g1", &guardians()).unwrap();
        assert_eq!(
            req.execute(&config(), 3_600),
            Err(GuardianRecoveryError::ThresholdNotMet)
        );
    }

    #[test]
    fn rejects_execution_before_delay_elapses() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        req.approve("g1", &guardians()).unwrap();
        req.approve("g2", &guardians()).unwrap();
        assert_eq!(
            req.execute(&config(), 3_599),
            Err(GuardianRecoveryError::DelayNotElapsed)
        );
    }

    #[test]
    fn executes_at_delay_boundary() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        req.approve("g1", &guardians()).unwrap();
        req.approve("g2", &guardians()).unwrap();
        assert_eq!(req.execute(&config(), 3_600), Ok(()));
        assert_eq!(req.status, GuardianRecoveryStatus::Executed);
    }

    #[test]
    fn executes_after_delay_elapses() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        req.approve("g1", &guardians()).unwrap();
        req.approve("g2", &guardians()).unwrap();
        assert_eq!(req.execute(&config(), 7_200), Ok(()));
        assert_eq!(req.status, GuardianRecoveryStatus::Executed);
    }

    #[test]
    fn active_admin_can_cancel_during_delay() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        req.approve("g1", &guardians()).unwrap();
        assert_eq!(req.cancel("admin"), Ok(()));
        assert_eq!(req.status, GuardianRecoveryStatus::Cancelled);
        assert_eq!(
            req.execute(&config(), 7_200),
            Err(GuardianRecoveryError::NoPendingRequest)
        );
    }

    #[test]
    fn non_admin_cannot_cancel() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        assert_eq!(
            req.cancel("g1"),
            Err(GuardianRecoveryError::NotActiveAdmin)
        );
    }

    #[test]
    fn rejects_duplicate_and_non_guardian_approvals() {
        let mut req = GuardianRecoveryRequest::initiate("admin".to_string(), 0);
        req.approve("g1", &guardians()).unwrap();
        assert_eq!(
            req.approve("g1", &guardians()),
            Err(GuardianRecoveryError::AlreadyApproved)
        );
        assert_eq!(
            req.approve("stranger", &guardians()),
            Err(GuardianRecoveryError::NotAGuardian)
        );
    }
}
