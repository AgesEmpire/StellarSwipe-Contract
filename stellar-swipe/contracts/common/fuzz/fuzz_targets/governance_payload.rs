//! Fuzz target: governance proposal payload edge cases (Issue #701).
//!
//! Generates randomised proposal payloads within and beyond expected structural
//! bounds and asserts that malformed payloads are rejected with a proper
//! `ContractError` (or `GovernanceError`) rather than panicking.
//!
//! ## Running locally
//! ```bash
//! cargo +nightly fuzz run governance_payload -- -max_total_time=300
//! ```
//!
//! ## Reproducing a crash
//! ```bash
//! cargo +nightly fuzz run governance_payload artifacts/governance_payload/<crash-file>
//! ```
//!
//! ## CI integration
//! ```bash
//! cargo +nightly fuzz run governance_payload -- -max_total_time=30 -timeout=10 -seed=42
//! ```
//!
//! ## Regression tests
//! See the `regression_tests` module at the bottom of this file.

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Bytes, Env, String, IntoVal, Val, Vec,
};

// ── Minimal governance error mirror ─────────────────────────────────────────

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
enum FuzzGovernanceError {
    InvalidProposal = 40,
}

// ── Fuzz harness contract ──────────────────────────────────────────────────

#[contract]
struct GovPayloadFuzz;

#[contractimpl]
impl GovPayloadFuzz {}


// ── Payload validation logic under test ─────────────────────────────────────

/// Mirrors `validate_execution_payload` from the governance contract.
fn validate_execution_payload(
    proposal_type: u8,
    payload: &[u8],
) -> Result<(), FuzzGovernanceError> {
    match proposal_type {
        2 => {
            // ContractUpgrade: payload must be exactly 32 bytes.
            if payload.len() != 32 {
                return Err(FuzzGovernanceError::InvalidProposal);
            }
        }
        0 | 1 => {
            // ParameterChange / TreasurySpend: optional payload, but if present
            // must start with 0x01 version byte.
            if !payload.is_empty() && payload[0] != 0x01 {
                return Err(FuzzGovernanceError::InvalidProposal);
            }
        }
        5 => {
            // Custom: payload must be non-empty.
            if payload.is_empty() {
                return Err(FuzzGovernanceError::InvalidProposal);
            }
        }
        // FeatureToggle(3), SignalProposal(4), unmapped(6+): no constraints.
        _ => {}
    }
    Ok(())
}

fn proposal_type_name(ptype: u8) -> &'static str {
    match ptype {
        0 => "ParameterChange",
        1 => "TreasurySpend",
        2 => "ContractUpgrade",
// ── Fuzz target ─────────────────────────────────────────────────────────────

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let env = Env::default();
    let id = env.register(GovPayloadFuzz, ());

    env.as_contract(&id, || {
        let ptype = data[0] % 7;
        let payload_bytes = if data.len() > 1 { &data[1..] } else { &[] };

        let result = validate_execution_payload(ptype, payload_bytes);

        match ptype {
            0 | 1 => {
                if !payload_bytes.is_empty() && payload_bytes[0] != 0x01 {
                    assert_eq!(
                        result,
                        Err(FuzzGovernanceError::InvalidProposal),
                        "proposal_type={} payload_len={} first_byte={} should be rejected",
                        proposal_type_name(ptype),
                        payload_bytes.len(),
                        payload_bytes[0],
                    );
                } else {
                    assert!(result.is_ok(),
                        "proposal_type={} payload_len={} should be accepted",
                        proposal_type_name(ptype), payload_bytes.len());
                }
            }
            2 => {
                if payload_bytes.len() != 32 {
                    assert_eq!(
                        result,
                        Err(FuzzGovernanceError::InvalidProposal),
                        "proposal_type=ContractUpgrade payload_len={} should be rejected",
                        payload_bytes.len(),
                    );
                } else {
                    assert!(result.is_ok(),
                        "proposal_type=ContractUpgrade payload_len=32 should be accepted");
                }
            }
            5 => {
                if payload_bytes.is_empty() {
                    assert_eq!(
// ── Deterministic regression tests ──────────────────────────────────────────

#[cfg(test)]
mod regression_tests {
    use super::*;

    #[test]
    fn contract_upgrade_exactly_32_bytes_ok() {
        let payload = [0x01u8; 32];
        assert_eq!(validate_execution_payload(2, &payload), Ok(()));
    }

    #[test]
    fn contract_upgrade_31_bytes_fails() {
        let payload = [0x01u8; 31];
        assert_eq!(
            validate_execution_payload(2, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn contract_upgrade_33_bytes_fails() {
        let payload = [0x01u8; 33];
        assert_eq!(
            validate_execution_payload(2, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn contract_upgrade_empty_payload_fails() {
        let payload = [];
        assert_eq!(
            validate_execution_payload(2, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn parameter_change_valid_prefix_ok() {
        let payload = [0x01, 0x02, 0x03];
        assert_eq!(validate_execution_payload(0, &payload), Ok(()));
    }

    #[test]
    fn parameter_change_invalid_prefix_fails() {
        let payload = [0x00, 0x02, 0x03];
        assert_eq!(
            validate_execution_payload(0, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn parameter_change_empty_payload_ok() {
        let payload = [];
        assert_eq!(validate_execution_payload(0, &payload), Ok(()));
    }

    #[test]
    fn treasury_spend_invalid_prefix_fails() {
        let payload = [0xFF, 0x00];
        assert_eq!(
            validate_execution_payload(1, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn custom_proposal_empty_payload_fails() {
        let payload = [];
        assert_eq!(
            validate_execution_payload(5, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn custom_proposal_non_empty_payload_ok() {
        let payload = [0x01, 0x02];
        assert_eq!(validate_execution_payload(5, &payload), Ok(()));
    }

    #[test]
    fn feature_toggle_any_payload_accepted() {
        assert!(validate_execution_payload(3, &[]).is_ok());
        assert!(validate_execution_payload(3, &[0xFF; 100]).is_ok());
    }

    #[test]
    fn signal_proposal_any_payload_accepted() {
        assert!(validate_execution_payload(4, &[]).is_ok());
        assert!(validate_execution_payload(4, &[0x00; 256]).is_ok());
    }

    #[test]
    fn unknown_proposal_type_always_accepted() {
        let oversized = [0xABu8; 4096];
        assert!(validate_execution_payload(6, &oversized).is_ok());
        assert!(validate_execution_payload(6, &[]).is_ok());
    }

    #[test]
    fn oversized_payload_for_contract_upgrade_fails() {
        let payload = [0x01u8; 4096];
        assert_eq!(
            validate_execution_payload(2, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }

    #[test]
    fn single_byte_invalid_prefix_fails() {
        let payload = [0x02];
        assert_eq!(
            validate_execution_payload(0, &payload),
            Err(FuzzGovernanceError::InvalidProposal),
        );
    }
}

                        result,
                        Err(FuzzGovernanceError::InvalidProposal),
                        "proposal_type=Custom empty payload should be rejected",
                    );
                } else {
                    assert!(result.is_ok(),
                        "proposal_type=Custom non-empty payload should be accepted");
                }
            }
            _ => {
                assert!(result.is_ok(),
                    "proposal_type={} should always be accepted",
                    proposal_type_name(ptype));
            }
        }

        // Stress-test XDR round-trip through storage with arbitrary payload bytes.
        let key = ("payload", ptype);
        let bytes_val = Bytes::from_slice(&env, payload_bytes);
        env.storage().persistent().set(&key, &bytes_val);
        let _: Option<Bytes> = env.storage().persistent().get(&key);
    });
});

        3 => "FeatureToggle",
        4 => "SignalProposal",
        5 => "Custom",
        _ => "Unknown",
    }
}
