//! Oracle observation identity and admission rules.
//!
//! # Deduplication (Issue #1212)
//! Every observation is identified by `(source, sequence)`, where `sequence`
//! is the ledger sequence it was recorded at (or the reporter's `round_id`
//! for external reports). Within one aggregation set a source holds at most
//! one observation:
//! - same source, same sequence → replay; it never counts twice;
//! - same source, lower sequence → out-of-order replay; rejected likewise;
//! - same source, higher sequence → supersedes the earlier observation, so the
//!   source still contributes exactly one sample;
//! - distinct sources → independent samples.
//!
//! # Future timestamps (Issue #1213)
//! An observation timestamped after the current ledger timestamp is rejected
//! with [`OracleError::FutureTimestamp`] before it is stored or aggregated.
//! Precision is whole seconds: `timestamp == now` is accepted and
//! `timestamp == now + 1` is rejected.

use crate::errors::OracleError;
use soroban_sdk::{Address, Env};

/// Where an admitted observation goes in the pending set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    /// First observation from this source; append it.
    Append,
    /// Supersedes this source's earlier observation at the given index.
    Replace(u32),
}

/// Decide how an observation `(source, sequence)` enters a pending set whose
/// entries are yielded (in storage order) as `(source, sequence)` pairs.
///
/// # Errors
/// [`OracleError::DuplicateObservation`] when the source already has an
/// observation at the same or a later sequence.
pub fn admit<I>(pending: I, source: &Address, sequence: u64) -> Result<Admission, OracleError>
where
    I: IntoIterator<Item = (Address, u64)>,
{
    for (i, (existing_source, existing_sequence)) in pending.into_iter().enumerate() {
        if existing_source == *source {
            if sequence <= existing_sequence {
                return Err(OracleError::DuplicateObservation);
            }
            return Ok(Admission::Replace(i as u32));
        }
    }
    Ok(Admission::Append)
}

/// Reject timestamps later than the current ledger timestamp.
pub fn ensure_not_future(env: &Env, timestamp: u64) -> Result<(), OracleError> {
    if timestamp > env.ledger().timestamp() {
        return Err(OracleError::FutureTimestamp);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, vec, Env, Vec};

    fn pending(env: &Env, entries: &[(&Address, u64)]) -> Vec<(Address, u64)> {
        let mut v = vec![env];
        for (a, s) in entries {
            v.push_back(((*a).clone(), *s));
        }
        v
    }

    #[test]
    fn first_observation_from_source_is_appended() {
        let env = Env::default();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let p = pending(&env, &[(&a, 10)]);
        assert_eq!(admit(p.iter(), &b, 10), Ok(Admission::Append));
    }

    #[test]
    fn replay_at_same_sequence_is_rejected() {
        let env = Env::default();
        let a = Address::generate(&env);
        let p = pending(&env, &[(&a, 10)]);
        assert_eq!(
            admit(p.iter(), &a, 10),
            Err(OracleError::DuplicateObservation)
        );
    }

    #[test]
    fn older_sequence_is_rejected() {
        let env = Env::default();
        let a = Address::generate(&env);
        let p = pending(&env, &[(&a, 10)]);
        assert_eq!(
            admit(p.iter(), &a, 9),
            Err(OracleError::DuplicateObservation)
        );
    }

    #[test]
    fn advanced_sequence_replaces_in_place() {
        let env = Env::default();
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let p = pending(&env, &[(&b, 10), (&a, 10)]);
        assert_eq!(admit(p.iter(), &a, 11), Ok(Admission::Replace(1)));
    }

    #[test]
    fn future_timestamp_boundary_is_second_precise() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        assert_eq!(ensure_not_future(&env, 999), Ok(()));
        assert_eq!(ensure_not_future(&env, 1_000), Ok(()));
        assert_eq!(
            ensure_not_future(&env, 1_001),
            Err(OracleError::FutureTimestamp)
        );
        assert_eq!(
            ensure_not_future(&env, u64::MAX),
            Err(OracleError::FutureTimestamp)
        );
    }
}
