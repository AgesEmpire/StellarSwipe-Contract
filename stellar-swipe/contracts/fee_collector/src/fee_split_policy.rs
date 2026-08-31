//! Configurable protocol / provider fee split policy (Issue #1032).
//!
//! Protocol and provider fee shares were previously implied by scattered
//! constants. This module makes the split a single policy object stored in
//! protocol state, validated on write, and read by every distribution path so
//! routine policy changes need no redeploy.
//!
//! Invariant: `protocol_bps + provider_bps == 10_000` (exactly 100%).
//! The split is dust-free — the protocol receives the floored share and the
//! provider receives the exact remainder.

use crate::errors::ContractError;
use soroban_sdk::contracttype;

/// Basis-point total representing 100%.
pub const BPS_TOTAL: u32 = 10_000;

/// Default policy: 30% protocol treasury, 70% signal provider.
pub const DEFAULT_PROTOCOL_BPS: u32 = 3_000;
pub const DEFAULT_PROVIDER_BPS: u32 = 7_000;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeSplitPolicy {
    /// Share routed to the protocol treasury, in basis points.
    pub protocol_bps: u32,
    /// Share routed to the signal provider, in basis points.
    pub provider_bps: u32,
}

impl FeeSplitPolicy {
    /// Reject any policy whose shares do not sum to exactly `BPS_TOTAL`.
    pub fn validate(&self) -> Result<(), ContractError> {
        let total = self
            .protocol_bps
            .checked_add(self.provider_bps)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if total != BPS_TOTAL {
            return Err(ContractError::InvalidFeeConfiguration);
        }
        Ok(())
    }

    /// Split `gross` into `(protocol_share, provider_share)` from this policy.
    /// The protocol share is floored; the provider share is the remainder, so
    /// `protocol_share + provider_share == gross` for every non-negative input.
    pub fn split(&self, gross: i128) -> Result<(i128, i128), ContractError> {
        if gross < 0 {
            return Err(ContractError::InvalidAmount);
        }
        let protocol_share = gross
            .checked_mul(self.protocol_bps as i128)
            .and_then(|v| v.checked_div(BPS_TOTAL as i128))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let provider_share = gross
            .checked_sub(protocol_share)
            .ok_or(ContractError::ArithmeticOverflow)?;
        Ok((protocol_share, provider_share))
    }
}

/// The policy applied when none has been configured.
pub fn default_policy() -> FeeSplitPolicy {
    FeeSplitPolicy {
        protocol_bps: DEFAULT_PROTOCOL_BPS,
        provider_bps: DEFAULT_PROVIDER_BPS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_valid_and_sums_to_100pct() {
        let p = default_policy();
        assert!(p.validate().is_ok());
        assert_eq!(p.protocol_bps + p.provider_bps, BPS_TOTAL);
    }

    #[test]
    fn rejects_shares_that_do_not_sum_to_total() {
        assert_eq!(
            FeeSplitPolicy {
                protocol_bps: 4_000,
                provider_bps: 5_000,
            }
            .validate(),
            Err(ContractError::InvalidFeeConfiguration)
        );
        assert_eq!(
            FeeSplitPolicy {
                protocol_bps: 6_000,
                provider_bps: 5_000,
            }
            .validate(),
            Err(ContractError::InvalidFeeConfiguration)
        );
    }

    #[test]
    fn rejects_overflowing_shares() {
        assert_eq!(
            FeeSplitPolicy {
                protocol_bps: u32::MAX,
                provider_bps: 1,
            }
            .validate(),
            Err(ContractError::ArithmeticOverflow)
        );
    }

    #[test]
    fn split_is_dust_free() {
        let p = FeeSplitPolicy {
            protocol_bps: 3_333,
            provider_bps: 6_667,
        };
        p.validate().unwrap();
        for gross in [0i128, 1, 7, 100, 999, 1_000_000, i128::from(u64::MAX)] {
            let (protocol, provider) = p.split(gross).unwrap();
            assert_eq!(protocol + provider, gross);
            assert!(protocol >= 0 && provider >= 0);
        }
    }

    #[test]
    fn split_matches_expected_shares() {
        let p = default_policy();
        let (protocol, provider) = p.split(1_000).unwrap();
        assert_eq!(protocol, 300);
        assert_eq!(provider, 700);
    }

    #[test]
    fn split_rejects_negative_amount() {
        assert_eq!(
            default_policy().split(-1),
            Err(ContractError::InvalidAmount)
        );
    }

    #[test]
    fn all_to_provider_or_all_to_protocol() {
        let all_provider = FeeSplitPolicy {
            protocol_bps: 0,
            provider_bps: 10_000,
        };
        all_provider.validate().unwrap();
        assert_eq!(all_provider.split(500).unwrap(), (0, 500));

        let all_protocol = FeeSplitPolicy {
            protocol_bps: 10_000,
            provider_bps: 0,
        };
        all_protocol.validate().unwrap();
        assert_eq!(all_protocol.split(500).unwrap(), (500, 0));
    }
}
